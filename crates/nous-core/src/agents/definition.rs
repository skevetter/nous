use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::error::NousError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub agent: AgentSection,
    pub process: Option<ProcessSection>,
    pub skills: Option<SkillsSection>,
    pub tools: Option<ToolsSection>,
    pub memory: Option<MemorySection>,
    pub metadata: Option<MetadataSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsSection {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub custom: Option<Vec<CustomToolDef>>,
    pub permissions: Option<ToolPermissionsConfig>,
    pub execution: Option<ExecutionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolDef {
    pub name: String,
    pub script: Option<String>,
    pub description: String,
    pub input_schema: Option<Value>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissionsConfig {
    pub filesystem_read: Option<Vec<String>>,
    pub filesystem_write: Option<Vec<String>>,
    pub network_hosts: Option<Vec<String>>,
    pub shell_commands: Option<Vec<String>>,
    pub require_confirmation: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub default_timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    pub max_output_bytes: Option<usize>,
    pub sandbox_required: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySection {
    #[serde(default = "default_scope")]
    pub scope: MemoryScope,
    #[serde(default = "default_retrieval")]
    pub retrieval: RetrievalStrategy,
    #[serde(default = "default_context_size")]
    pub context_size: u32,
    #[serde(default)]
    pub auto_save: Vec<String>,
    #[serde(default = "default_importance")]
    pub importance_default: String,
    #[serde(default)]
    pub session_tracking: bool,
    pub workspace_override: Option<String>,
}

fn default_scope() -> MemoryScope {
    MemoryScope::Agent
}

fn default_retrieval() -> RetrievalStrategy {
    RetrievalStrategy::Hybrid
}

fn default_context_size() -> u32 {
    5
}

fn default_importance() -> String {
    "moderate".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    Workspace,
    Agent,
    Session,
    Shared(Vec<String>),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RetrievalStrategy {
    Fts,
    Vector,
    Hybrid,
    Recency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSection {
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub version: String,
    pub namespace: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessSection {
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub spawn_command: Option<String>,
    pub working_dir: Option<String>,
    pub auto_restart: Option<bool>,
    pub restart_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsSection {
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetadataSection {
    pub model: Option<String>,
    pub timeout: Option<u64>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ResolvedSkill {
    pub name: String,
    pub path: PathBuf,
    pub content: String,
}

pub fn load_definition(path: &Path) -> Result<AgentDefinition, NousError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| NousError::Config(format!("failed to read {}: {e}", path.display())))?;
    let def: AgentDefinition = toml::from_str(&contents)
        .map_err(|e| NousError::Config(format!("failed to parse {}: {e}", path.display())))?;
    validate_definition(&def)?;
    Ok(def)
}

const VALID_MEMORY_TYPES: &[&str] = &[
    "decision",
    "convention",
    "bugfix",
    "architecture",
    "fact",
    "observation",
];

const VALID_IMPORTANCE_LEVELS: &[&str] = &["low", "moderate", "high"];

fn validate_definition(def: &AgentDefinition) -> Result<(), NousError> {
    if let Some(memory) = &def.memory {
        for entry in &memory.auto_save {
            if !VALID_MEMORY_TYPES.contains(&entry.as_str()) {
                return Err(NousError::Validation(format!(
                    "invalid auto_save type '{}': must be one of {:?}",
                    entry, VALID_MEMORY_TYPES
                )));
            }
        }

        if !VALID_IMPORTANCE_LEVELS.contains(&memory.importance_default.as_str()) {
            return Err(NousError::Validation(format!(
                "invalid importance_default '{}': must be one of {:?}",
                memory.importance_default, VALID_IMPORTANCE_LEVELS
            )));
        }

        if let MemoryScope::Shared(ids) = &memory.scope {
            if ids.is_empty() {
                return Err(NousError::Validation(
                    "shared scope requires at least one agent ID".into(),
                ));
            }
        }
    }
    Ok(())
}

pub fn resolve_skills(refs: &[String], skills_dir: &Path) -> Result<Vec<ResolvedSkill>, NousError> {
    refs.iter().map(|r| resolve_one(r, skills_dir)).collect()
}

fn resolve_one(skill_ref: &str, skills_dir: &Path) -> Result<ResolvedSkill, NousError> {
    if skill_ref.contains('/') {
        return Err(NousError::Validation(
            "tap-qualified skill refs not yet supported".into(),
        ));
    }

    let file = skills_dir.join(format!("{skill_ref}.md"));
    let content = std::fs::read_to_string(&file).map_err(|_| {
        NousError::NotFound(format!(
            "skill '{}' not found at {}",
            skill_ref,
            file.display()
        ))
    })?;

    Ok(ResolvedSkill {
        name: skill_ref.to_string(),
        path: file,
        content,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const FULL_TOML: &str = r#"
[agent]
name       = "reviewer"
type       = "engineer"
version    = "1.2.0"
namespace  = "eng"
description = "Performs code review on feature branches"

[process]
type         = "claude"
spawn_command = "claude --model claude-sonnet-4-6"
working_dir  = "~"
auto_restart = false
restart_policy = "on-failure"

[skills]
refs = [
  "code-review",
  "git-workflow",
]

[metadata]
model   = "global.anthropic.claude-sonnet-4-6-v1"
timeout = 3600
tags    = ["review", "quality"]
"#;

    #[test]
    fn test_parse_valid_definition() {
        let def: AgentDefinition = toml::from_str(FULL_TOML).unwrap();

        assert_eq!(def.agent.name, "reviewer");
        assert_eq!(def.agent.r#type, "engineer");
        assert_eq!(def.agent.version, "1.2.0");
        assert_eq!(def.agent.namespace.as_deref(), Some("eng"));
        assert_eq!(
            def.agent.description.as_deref(),
            Some("Performs code review on feature branches")
        );

        let process = def.process.unwrap();
        assert_eq!(process.r#type.as_deref(), Some("claude"));
        assert_eq!(
            process.spawn_command.as_deref(),
            Some("claude --model claude-sonnet-4-6")
        );
        assert_eq!(process.working_dir.as_deref(), Some("~"));
        assert_eq!(process.auto_restart, Some(false));
        assert_eq!(process.restart_policy.as_deref(), Some("on-failure"));

        let skills = def.skills.unwrap();
        assert_eq!(skills.refs, vec!["code-review", "git-workflow"]);

        let meta = def.metadata.unwrap();
        assert_eq!(
            meta.model.as_deref(),
            Some("global.anthropic.claude-sonnet-4-6-v1")
        );
        assert_eq!(meta.timeout, Some(3600));
        assert_eq!(
            meta.tags.as_deref(),
            Some(vec!["review".to_string(), "quality".to_string()].as_slice())
        );
    }

    #[test]
    fn test_parse_minimal_definition() {
        let toml_str = r#"
[agent]
name    = "basic"
type    = "manager"
version = "0.1.0"
"#;
        let def: AgentDefinition = toml::from_str(toml_str).unwrap();

        assert_eq!(def.agent.name, "basic");
        assert_eq!(def.agent.r#type, "manager");
        assert_eq!(def.agent.version, "0.1.0");
        assert!(def.agent.namespace.is_none());
        assert!(def.agent.description.is_none());
        assert!(def.process.is_none());
        assert!(def.skills.is_none());
        assert!(def.tools.is_none());
        assert!(def.metadata.is_none());
    }

    #[test]
    fn test_parse_missing_required_field() {
        let toml_str = r#"
[agent]
type    = "engineer"
version = "1.0.0"
"#;
        let result: Result<AgentDefinition, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_skills_local() {
        let dir = tempfile::tempdir().unwrap();
        let skill_path = dir.path().join("code-review.md");
        let mut f = std::fs::File::create(&skill_path).unwrap();
        f.write_all(b"# Code Review\nReview guidelines here.")
            .unwrap();

        let resolved = resolve_skills(&["code-review".to_string()], dir.path()).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "code-review");
        assert_eq!(resolved[0].path, skill_path);
        assert!(resolved[0].content.contains("Review guidelines here."));
    }

    #[test]
    fn test_resolve_skills_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_skills(&["nonexistent".to_string()], dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::NotFound(msg) => assert!(msg.contains("nonexistent")),
            other => panic!("expected NotFound, got: {other:?}"),
        }
    }

    #[test]
    fn test_resolve_skills_tap_qualified_errors() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_skills(&["tap/some-skill".to_string()], dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => {
                assert!(msg.contains("tap-qualified skill refs not yet supported"))
            }
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[test]
    fn test_parse_full_tools_section() {
        let toml_str = r#"
[agent]
name       = "code-reviewer"
type       = "engineer"
version    = "1.0.0"
description = "Reviews pull requests"

[process]
type          = "claude"
spawn_command = "claude --model claude-sonnet-4-6"

[skills]
refs = ["code-review"]

[tools]
allow = ["fs_read", "fs_search", "code_grep", "shell_exec"]
deny = ["fs_delete", "shell_kill"]

[[tools.custom]]
name = "lint_check"
script = "scripts/lint.sh"
description = "Run project linter"

[[tools.custom]]
name = "test_runner"
script = "scripts/test.py"
description = "Run test suite"
timeout_secs = 120

[tools.permissions]
filesystem_read  = ["**/*.rs", "**/*.toml"]
filesystem_write = []
network_hosts    = []
shell_commands   = ["git", "cargo"]
require_confirmation = false

[tools.execution]
default_timeout_secs = 30
max_retries          = 1
max_output_bytes     = 2097152
sandbox_required     = false

[metadata]
model   = "global.anthropic.claude-sonnet-4-6-v1"
timeout = 3600
tags    = ["review", "quality"]
"#;
        let def: AgentDefinition = toml::from_str(toml_str).unwrap();

        let tools = def.tools.unwrap();
        let allow = tools.allow.unwrap();
        assert_eq!(
            allow,
            vec!["fs_read", "fs_search", "code_grep", "shell_exec"]
        );

        let deny = tools.deny.unwrap();
        assert_eq!(deny, vec!["fs_delete", "shell_kill"]);

        let custom = tools.custom.unwrap();
        assert_eq!(custom.len(), 2);
        assert_eq!(custom[0].name, "lint_check");
        assert_eq!(custom[0].script.as_deref(), Some("scripts/lint.sh"));
        assert_eq!(custom[1].name, "test_runner");
        assert_eq!(custom[1].timeout_secs, Some(120));

        let perms = tools.permissions.unwrap();
        assert_eq!(
            perms.filesystem_read.as_deref(),
            Some(vec!["**/*.rs".to_string(), "**/*.toml".to_string()].as_slice())
        );
        assert_eq!(perms.require_confirmation, Some(false));

        let exec = tools.execution.unwrap();
        assert_eq!(exec.default_timeout_secs, Some(30));
        assert_eq!(exec.max_retries, Some(1));
        assert_eq!(exec.max_output_bytes, Some(2097152));
        assert_eq!(exec.sandbox_required, Some(false));
    }

    #[test]
    fn test_parse_minimal_without_tools_backward_compat() {
        let toml_str = FULL_TOML;
        let def: AgentDefinition = toml::from_str(toml_str).unwrap();
        assert!(def.tools.is_none());
        assert_eq!(def.agent.name, "reviewer");
    }

    #[test]
    fn test_parse_memory_section() {
        let toml_str = r#"
[agent]
name    = "mem-agent"
type    = "engineer"
version = "1.0.0"

[memory]
scope = "workspace"
retrieval = "hybrid"
context_size = 10
auto_save = ["decision", "convention"]
importance_default = "high"
session_tracking = true
"#;
        let def: AgentDefinition = toml::from_str(toml_str).unwrap();
        let mem = def.memory.unwrap();
        assert_eq!(mem.scope, MemoryScope::Workspace);
        assert_eq!(mem.retrieval, RetrievalStrategy::Hybrid);
        assert_eq!(mem.context_size, 10);
        assert_eq!(mem.auto_save, vec!["decision", "convention"]);
        assert_eq!(mem.importance_default, "high");
        assert!(mem.session_tracking);
        assert!(mem.workspace_override.is_none());
    }

    #[test]
    fn test_parse_memory_defaults() {
        let toml_str = r#"
[agent]
name    = "mem-agent"
type    = "engineer"
version = "1.0.0"

[memory]
"#;
        let def: AgentDefinition = toml::from_str(toml_str).unwrap();
        let mem = def.memory.unwrap();
        assert_eq!(mem.scope, MemoryScope::Agent);
        assert_eq!(mem.retrieval, RetrievalStrategy::Hybrid);
        assert_eq!(mem.context_size, 5);
        assert!(mem.auto_save.is_empty());
        assert_eq!(mem.importance_default, "moderate");
        assert!(!mem.session_tracking);
        assert!(mem.workspace_override.is_none());
    }

    #[test]
    fn test_parse_memory_shared_scope() {
        let toml_str = r#"
[agent]
name    = "mem-agent"
type    = "engineer"
version = "1.0.0"

[memory]
scope = {shared = ["agent-1", "agent-2"]}
"#;
        let def: AgentDefinition = toml::from_str(toml_str).unwrap();
        let mem = def.memory.unwrap();
        assert_eq!(
            mem.scope,
            MemoryScope::Shared(vec!["agent-1".to_string(), "agent-2".to_string()])
        );
    }

    #[test]
    fn test_validate_invalid_auto_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(
            &path,
            r#"
[agent]
name    = "mem-agent"
type    = "engineer"
version = "1.0.0"

[memory]
auto_save = ["decision", "invalid_type"]
"#,
        )
        .unwrap();
        let result = load_definition(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => assert!(msg.contains("invalid_type")),
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_invalid_importance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(
            &path,
            r#"
[agent]
name    = "mem-agent"
type    = "engineer"
version = "1.0.0"

[memory]
importance_default = "critical"
"#,
        )
        .unwrap();
        let result = load_definition(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => assert!(msg.contains("critical")),
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[test]
    fn test_validate_empty_shared_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.toml");
        std::fs::write(
            &path,
            r#"
[agent]
name    = "mem-agent"
type    = "engineer"
version = "1.0.0"

[memory]
scope = {shared = []}
"#,
        )
        .unwrap();
        let result = load_definition(&path);
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => {
                assert!(msg.contains("shared scope requires at least one agent ID"))
            }
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[test]
    fn test_backward_compat_no_memory() {
        let def: AgentDefinition = toml::from_str(FULL_TOML).unwrap();
        assert!(def.memory.is_none());
        assert_eq!(def.agent.name, "reviewer");
        assert_eq!(def.agent.r#type, "engineer");
        assert_eq!(def.agent.version, "1.2.0");
        assert!(def.process.is_some());
        assert!(def.skills.is_some());
        assert!(def.metadata.is_some());
    }
}
