use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::agents::definition::CustomToolDef;
use crate::error::NousError;
use crate::tools::{
    AgentTool, ExecutionPolicy, RiskLevel, ShellPermission, ToolCategory, ToolContent, ToolContext,
    ToolError, ToolMetadata, ToolOutput, ToolPermissions,
};

#[derive(Debug)]
pub struct ScriptTool {
    meta: ToolMetadata,
    script_path: PathBuf,
}

impl ScriptTool {
    pub fn from_def(def: &CustomToolDef, base_dir: &Path) -> Result<Self, NousError> {
        let script_path = base_dir.join(
            def.script
                .as_deref()
                .ok_or_else(|| NousError::Validation("script path required".into()))?,
        );

        if !script_path.exists() {
            return Err(NousError::NotFound(format!(
                "script not found: {}",
                script_path.display()
            )));
        }

        let canonical_script = script_path
            .canonicalize()
            .map_err(|e| NousError::Validation(format!("cannot resolve script path: {e}")))?;
        let canonical_base = base_dir
            .canonicalize()
            .map_err(|e| NousError::Validation(format!("cannot resolve base dir: {e}")))?;
        if !canonical_script.starts_with(&canonical_base) {
            return Err(NousError::Validation(format!(
                "script path escapes base directory: {}",
                def.script.as_deref().unwrap_or("<unknown>")
            )));
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&canonical_script)
                .map_err(|e| NousError::Validation(format!("cannot read script metadata: {e}")))?
                .permissions()
                .mode();
            if mode & 0o111 == 0 {
                return Err(NousError::Validation(format!(
                    "script is not executable: {}",
                    canonical_script.display()
                )));
            }
        }

        Ok(Self {
            meta: ToolMetadata {
                name: def.name.clone(),
                description: def.description.clone(),
                category: ToolCategory::Custom,
                version: "0.1.0".into(),
                input_schema: def.input_schema.clone().unwrap_or_else(|| {
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "args": {
                                "type": "string",
                                "description": "Arguments to pass to the script"
                            }
                        }
                    })
                }),
                output_schema: None,
                permissions: ToolPermissions {
                    shell: Some(ShellPermission {
                        allowed_commands: vec![script_path.display().to_string()],
                        denied_commands: vec![],
                        allow_arbitrary: false,
                    }),
                    risk_level: RiskLevel::Medium,
                    ..Default::default()
                },
                execution_policy: ExecutionPolicy {
                    timeout_secs: def.timeout_secs.unwrap_or(30),
                    ..Default::default()
                },
                tags: vec!["custom".into(), "script".into()],
            },
            script_path,
        })
    }
}

impl AgentTool for ScriptTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.meta
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let script_args = args.get("args").and_then(|v| v.as_str()).unwrap_or("");

        let output = tokio::process::Command::new(&self.script_path)
            .arg(script_args)
            .current_dir(ctx.workspace_dir.as_deref().unwrap_or(Path::new(".")))
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: stdout.into_owned(),
                }],
                metadata: None,
            })
        } else {
            Err(ToolError::ExecutionFailed(format!(
                "exit code {}: {}",
                output.status, stderr
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn create_executable_script(path: &Path, content: &[u8]) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn from_def_with_valid_script() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("test.sh");
        create_executable_script(&script_path, b"#!/bin/sh\necho hello");

        let def = CustomToolDef {
            name: "test_script".into(),
            script: Some("test.sh".into()),
            description: "A test script".into(),
            input_schema: None,
            timeout_secs: Some(60),
        };

        let tool = ScriptTool::from_def(&def, dir.path()).unwrap();
        assert_eq!(tool.meta.name, "test_script");
        assert_eq!(tool.meta.category, ToolCategory::Custom);
        assert_eq!(tool.meta.execution_policy.timeout_secs, 60);
    }

    #[test]
    fn from_def_missing_script_path() {
        let dir = tempfile::tempdir().unwrap();
        let def = CustomToolDef {
            name: "bad".into(),
            script: None,
            description: "No script".into(),
            input_schema: None,
            timeout_secs: None,
        };

        let result = ScriptTool::from_def(&def, dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => assert!(msg.contains("script path required")),
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[test]
    fn from_def_script_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let def = CustomToolDef {
            name: "missing".into(),
            script: Some("nonexistent.sh".into()),
            description: "Missing script".into(),
            input_schema: None,
            timeout_secs: None,
        };

        let result = ScriptTool::from_def(&def, dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::NotFound(msg) => assert!(msg.contains("nonexistent.sh")),
            other => panic!("expected NotFound, got: {other:?}"),
        }
    }

    #[test]
    fn from_def_rejects_path_traversal() {
        let parent = tempfile::tempdir().unwrap();
        let base_dir = parent.path().join("workspace");
        std::fs::create_dir_all(&base_dir).unwrap();
        let outside_script = parent.path().join("evil.sh");
        create_executable_script(&outside_script, b"#!/bin/sh\necho pwned");

        let def = CustomToolDef {
            name: "traversal".into(),
            script: Some("../evil.sh".into()),
            description: "Path traversal attempt".into(),
            input_schema: None,
            timeout_secs: None,
        };

        let result = ScriptTool::from_def(&def, &base_dir);
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => assert!(msg.contains("escapes base directory")),
            other => panic!("expected Validation, got: {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn from_def_rejects_non_executable_script() {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("noexec.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        f.write_all(b"#!/bin/sh\necho hello").unwrap();

        let def = CustomToolDef {
            name: "noexec".into(),
            script: Some("noexec.sh".into()),
            description: "Non-executable script".into(),
            input_schema: None,
            timeout_secs: None,
        };

        let result = ScriptTool::from_def(&def, dir.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            NousError::Validation(msg) => assert!(msg.contains("not executable")),
            other => panic!("expected Validation, got: {other:?}"),
        }
    }
}
