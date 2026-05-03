use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::NousError;

pub mod builtin;
pub mod custom;
pub mod execution;
pub mod permissions;
pub mod registry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub category: ToolCategory,
    pub version: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub permissions: ToolPermissions,
    pub execution_policy: ExecutionPolicy,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    FileSystem,
    Shell,
    Http,
    Memory,
    AgentComms,
    Database,
    CodeAnalysis,
    Custom,
}

pub trait AgentTool: Send + Sync + 'static {
    fn metadata(&self) -> &ToolMetadata;

    fn call(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send;
}

pub trait AgentToolDyn: Send + Sync + 'static {
    fn metadata_dyn(&self) -> &ToolMetadata;
    fn call_dyn<'a>(
        &'a self,
        args: Value,
        ctx: &'a ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>>;
}

impl<T: AgentTool> AgentToolDyn for T {
    fn metadata_dyn(&self) -> &ToolMetadata {
        self.metadata()
    }

    fn call_dyn<'a>(
        &'a self,
        args: Value,
        ctx: &'a ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(self.call(args, ctx))
    }
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub agent_id: String,
    pub agent_name: String,
    pub namespace: String,
    pub workspace_dir: Option<PathBuf>,
    pub session_id: Option<String>,
    pub timeout: Duration,
    pub permissions: ResolvedPermissions,
}

#[derive(Debug, Clone)]
pub struct ResolvedPermissions {
    pub allowed_tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub allowed_paths: Option<Vec<PathBuf>>,
    pub network_access: NetworkPolicy,
    pub max_output_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    None,
    Isolated,
    AllowList,
    Unrestricted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: Vec<ToolContent>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Text { text: String },
    Json { data: Value },
    Binary { mime_type: String, data: Vec<u8> },
    Error { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("timeout after {0:?}")]
    Timeout(Duration),

    #[error("tool not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Internal(#[from] NousError),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissions {
    pub filesystem: Option<FileSystemPermission>,
    pub network: Option<NetworkPermission>,
    pub shell: Option<ShellPermission>,
    pub requires_confirmation: bool,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSystemPermission {
    pub read_paths: Vec<String>,
    pub write_paths: Vec<String>,
    pub deny_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPermission {
    pub allowed_hosts: Vec<String>,
    pub denied_hosts: Vec<String>,
    pub max_request_size_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellPermission {
    pub allowed_commands: Vec<String>,
    pub denied_commands: Vec<String>,
    pub allow_arbitrary: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub max_output_bytes: usize,
    pub sandbox_required: bool,
    pub idempotent: bool,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_retries: 0,
            retry_delay_ms: 1000,
            max_output_bytes: 1_048_576,
            sandbox_required: false,
            idempotent: false,
        }
    }
}

use std::collections::HashSet;

use crate::agents::definition::AgentDefinition;
use registry::DynTool;

pub async fn resolve_agent_tools(
    registry: &registry::ToolRegistry,
    agent_def: &AgentDefinition,
    agent_type: &str,
) -> Vec<DynTool> {
    let allowed_names: HashSet<String> = if let Some(ref tools_section) = agent_def.tools {
        if let Some(ref allow) = tools_section.allow {
            allow.iter().cloned().collect()
        } else {
            default_tools_for_type(agent_type)
        }
    } else {
        default_tools_for_type(agent_type)
    };

    let denied_names: HashSet<String> = agent_def
        .tools
        .as_ref()
        .and_then(|t| t.deny.as_ref())
        .map(|d| d.iter().cloned().collect())
        .unwrap_or_default();

    let final_names: HashSet<String> = allowed_names.difference(&denied_names).cloned().collect();

    let mut resolved = Vec::new();
    for name in &final_names {
        if let Some(tool) = registry.get(name).await {
            resolved.push(tool);
        }
    }
    resolved
}

pub fn default_tools_for_type(agent_type: &str) -> HashSet<String> {
    match agent_type {
        "engineer" => [
            "fs_read",
            "fs_write",
            "fs_edit",
            "fs_list",
            "fs_search",
            "fs_stat",
            "fs_mkdir",
            "shell_exec",
            "shell_exec_background",
            "shell_read_output",
            "code_grep",
            "code_glob",
            "code_symbols",
            "memory_save",
            "memory_search",
            "memory_search_hybrid",
            "memory_get_context",
            "room_post",
            "room_read",
            "room_wait",
            "task_update",
            "http_fetch",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        "manager" | "director" | "senior_manager" => [
            "memory_save",
            "memory_search",
            "memory_search_hybrid",
            "memory_get_context",
            "memory_relate",
            "room_post",
            "room_read",
            "room_create",
            "room_wait",
            "task_create",
            "task_update",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        _ => HashSet::new(),
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::agents::definition::{AgentDefinition, AgentSection, ToolsSection};
    use crate::tools::builtin::register_builtin_tools;
    use crate::tools::registry::ToolRegistry;

    fn minimal_agent_def(agent_type: &str) -> AgentDefinition {
        AgentDefinition {
            agent: AgentSection {
                name: "test".into(),
                r#type: agent_type.into(),
                version: "1.0.0".into(),
                namespace: None,
                description: None,
            },
            process: None,
            skills: None,
            tools: None,
            metadata: None,
        }
    }

    #[test]
    fn default_tools_engineer_has_filesystem_and_shell() {
        let tools = default_tools_for_type("engineer");
        assert!(tools.contains("fs_read"));
        assert!(tools.contains("fs_write"));
        assert!(tools.contains("shell_exec"));
        assert!(tools.contains("code_grep"));
        assert!(tools.contains("memory_save"));
        assert!(!tools.contains("task_create"));
    }

    #[test]
    fn default_tools_manager_has_memory_and_comms_only() {
        let tools = default_tools_for_type("manager");
        assert!(tools.contains("memory_save"));
        assert!(tools.contains("room_post"));
        assert!(tools.contains("task_create"));
        assert!(!tools.contains("fs_read"));
        assert!(!tools.contains("shell_exec"));
        assert!(!tools.contains("code_grep"));
    }

    #[test]
    fn default_tools_engineer_vs_manager_differ() {
        let eng = default_tools_for_type("engineer");
        let mgr = default_tools_for_type("manager");
        assert_ne!(eng, mgr);
        assert!(eng.len() > mgr.len());
    }

    #[test]
    fn default_tools_unknown_type_is_empty() {
        let tools = default_tools_for_type("unknown");
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn resolve_engineer_gets_filesystem_tools() {
        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;

        let def = minimal_agent_def("engineer");
        let tools = resolve_agent_tools(&registry, &def, "engineer").await;

        let names: HashSet<String> = tools.iter().map(|t| t.metadata_dyn().name.clone()).collect();
        assert!(names.contains("fs_read"));
        assert!(names.contains("shell_exec"));
        assert!(names.contains("code_grep"));
    }

    #[tokio::test]
    async fn resolve_manager_gets_only_memory_comms_task() {
        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;

        let def = minimal_agent_def("manager");
        let tools = resolve_agent_tools(&registry, &def, "manager").await;

        let names: HashSet<String> = tools.iter().map(|t| t.metadata_dyn().name.clone()).collect();
        assert!(names.contains("memory_save"));
        assert!(names.contains("room_post"));
        assert!(names.contains("task_create"));
        assert!(!names.contains("fs_read"));
        assert!(!names.contains("shell_exec"));
    }

    #[tokio::test]
    async fn resolve_allow_list_restricts_tools() {
        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;

        let mut def = minimal_agent_def("engineer");
        def.tools = Some(ToolsSection {
            allow: Some(vec!["fs_read".into(), "fs_write".into()]),
            deny: None,
            custom: None,
            permissions: None,
            execution: None,
        });

        let tools = resolve_agent_tools(&registry, &def, "engineer").await;
        let names: HashSet<String> = tools.iter().map(|t| t.metadata_dyn().name.clone()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains("fs_read"));
        assert!(names.contains("fs_write"));
    }

    #[tokio::test]
    async fn resolve_deny_list_removes_tools() {
        let registry = ToolRegistry::new();
        register_builtin_tools(&registry).await;

        let mut def = minimal_agent_def("engineer");
        def.tools = Some(ToolsSection {
            allow: Some(vec![
                "fs_read".into(),
                "fs_write".into(),
                "fs_delete".into(),
            ]),
            deny: Some(vec!["fs_delete".into()]),
            custom: None,
            permissions: None,
            execution: None,
        });

        let tools = resolve_agent_tools(&registry, &def, "engineer").await;
        let names: HashSet<String> = tools.iter().map(|t| t.metadata_dyn().name.clone()).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains("fs_read"));
        assert!(names.contains("fs_write"));
        assert!(!names.contains("fs_delete"));
    }
}
