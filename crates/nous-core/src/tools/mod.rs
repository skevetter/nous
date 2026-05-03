use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::NousError;

pub mod builtin;
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
