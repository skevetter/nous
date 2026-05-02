use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::error::NousError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub agent: AgentSection,
    pub process: Option<ProcessSection>,
    pub skills: Option<SkillsSection>,
    pub metadata: Option<MetadataSection>,
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
    toml::from_str(&contents)
        .map_err(|e| NousError::Config(format!("failed to parse {}: {e}", path.display())))
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
}
