use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::RwLock;

use crate::tools::{
    AgentTool, ExecutionPolicy, RiskLevel, ShellPermission, ToolCategory, ToolContent, ToolContext,
    ToolError, ToolMetadata, ToolOutput, ToolPermissions,
};

static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

struct BackgroundProcess {
    child: tokio::process::Child,
    command: String,
}

static BACKGROUND_PROCESSES: OnceLock<RwLock<HashMap<u64, BackgroundProcess>>> = OnceLock::new();

fn bg_processes() -> &'static RwLock<HashMap<u64, BackgroundProcess>> {
    BACKGROUND_PROCESSES.get_or_init(|| RwLock::new(HashMap::new()))
}

fn check_shell_permission(command: &str, ctx: &ToolContext) -> Result<(), ToolError> {
    let shell_perm = ctx.permissions.shell.as_ref().ok_or_else(|| {
        ToolError::PermissionDenied("shell execution denied: no shell permissions configured".into())
    })?;

    if shell_perm.allow_arbitrary {
        return Ok(());
    }

    let base_command = command.split_whitespace().next().unwrap_or("");

    if shell_perm
        .denied_commands
        .iter()
        .any(|d| d == base_command)
    {
        return Err(ToolError::PermissionDenied(format!(
            "shell command denied: '{base_command}' is in the denied list"
        )));
    }

    if shell_perm.allowed_commands.is_empty()
        || !shell_perm
            .allowed_commands
            .iter()
            .any(|a| a == base_command)
    {
        return Err(ToolError::PermissionDenied(format!(
            "shell command denied: '{base_command}' is not in the allowed list"
        )));
    }

    Ok(())
}

// --- ShellExecTool ---

#[derive(Default)]
pub struct ShellExecTool {
    meta: OnceLock<ToolMetadata>,
}

impl ShellExecTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "shell_exec".into(),
            description: "Execute a shell command and return its output".into(),
            category: ToolCategory::Shell,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "cwd": { "type": "string", "description": "Working directory (optional)" },
                    "timeout_ms": { "type": "integer", "description": "Timeout in milliseconds (default: 120000)" }
                },
                "required": ["command"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                shell: Some(ShellPermission {
                    allowed_commands: vec![],
                    denied_commands: vec![],
                    allow_arbitrary: true,
                }),
                requires_confirmation: true,
                risk_level: RiskLevel::High,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 120,
                ..Default::default()
            },
            tags: vec!["shell".into(), "exec".into()],
        })
    }
}

impl AgentTool for ShellExecTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'command' required".into()))?;

        check_shell_permission(command, ctx)?;

        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .or_else(|| ctx.workspace_dir.clone());

        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        if let Some(ref dir) = cwd {
            cmd.current_dir(dir);
        }

        let output = cmd
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let text = if output.status.success() {
            if stderr.is_empty() {
                stdout.into_owned()
            } else {
                format!("{stdout}\n[stderr]\n{stderr}")
            }
        } else {
            format!(
                "exit code {}\n[stdout]\n{stdout}\n[stderr]\n{stderr}",
                output.status.code().unwrap_or(-1)
            )
        };

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text }],
            metadata: Some(json!({
                "exit_code": output.status.code().unwrap_or(-1),
            })),
        })
    }
}

// --- ShellExecBackgroundTool ---

#[derive(Default)]
pub struct ShellExecBackgroundTool {
    meta: OnceLock<ToolMetadata>,
}

impl ShellExecBackgroundTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "shell_exec_background".into(),
            description: "Run a shell command in the background and return a handle".into(),
            category: ToolCategory::Shell,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to execute" },
                    "cwd": { "type": "string", "description": "Working directory (optional)" }
                },
                "required": ["command"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                shell: Some(ShellPermission {
                    allowed_commands: vec![],
                    denied_commands: vec![],
                    allow_arbitrary: true,
                }),
                requires_confirmation: true,
                risk_level: RiskLevel::High,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 5,
                ..Default::default()
            },
            tags: vec!["shell".into(), "background".into()],
        })
    }
}

impl AgentTool for ShellExecBackgroundTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'command' required".into()))?;

        check_shell_permission(command, ctx)?;

        let cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .or_else(|| ctx.workspace_dir.clone());

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        if let Some(ref dir) = cwd {
            cmd.current_dir(dir);
        }

        let child = cmd
            .spawn()
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
        bg_processes().write().await.insert(
            handle,
            BackgroundProcess {
                child,
                command: command.to_string(),
            },
        );

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!("background process started with handle {handle}"),
            }],
            metadata: Some(json!({"handle": handle})),
        })
    }
}

// --- ShellReadOutputTool ---

#[derive(Default)]
pub struct ShellReadOutputTool {
    meta: OnceLock<ToolMetadata>,
}

impl ShellReadOutputTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "shell_read_output".into(),
            description: "Read output from a background shell command".into(),
            category: ToolCategory::Shell,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "handle": { "type": "integer", "description": "Background process handle" }
                },
                "required": ["handle"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["shell".into(), "read".into()],
        })
    }
}

impl AgentTool for ShellReadOutputTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let handle = args
            .get("handle")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs("'handle' required".into()))?;

        let mut procs = bg_processes().write().await;
        let proc = procs.get_mut(&handle).ok_or_else(|| {
            ToolError::NotFound(format!("no background process with handle {handle}"))
        })?;

        match proc.child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(ref mut out) = proc.child.stdout {
                    use tokio::io::AsyncReadExt;
                    let _ = out.read_to_string(&mut stdout).await;
                }
                if let Some(ref mut err) = proc.child.stderr {
                    use tokio::io::AsyncReadExt;
                    let _ = err.read_to_string(&mut stderr).await;
                }
                procs.remove(&handle);
                Ok(ToolOutput {
                    content: vec![ToolContent::Text {
                        text: format!(
                            "process completed (exit {})\n[stdout]\n{stdout}\n[stderr]\n{stderr}",
                            status.code().unwrap_or(-1)
                        ),
                    }],
                    metadata: Some(
                        json!({"status": "completed", "exit_code": status.code().unwrap_or(-1)}),
                    ),
                })
            }
            Ok(None) => Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: format!("process {} still running ({})", handle, proc.command),
                }],
                metadata: Some(json!({"status": "running"})),
            }),
            Err(e) => Err(ToolError::ExecutionFailed(e.to_string())),
        }
    }
}

// --- ShellKillTool ---

#[derive(Default)]
pub struct ShellKillTool {
    meta: OnceLock<ToolMetadata>,
}

impl ShellKillTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "shell_kill".into(),
            description: "Kill a background process by handle".into(),
            category: ToolCategory::Shell,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "handle": { "type": "integer", "description": "Background process handle" }
                },
                "required": ["handle"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                risk_level: RiskLevel::Medium,
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 5,
                ..Default::default()
            },
            tags: vec!["shell".into(), "kill".into()],
        })
    }
}

impl AgentTool for ShellKillTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let handle = args
            .get("handle")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs("'handle' required".into()))?;

        let mut procs = bg_processes().write().await;
        let mut proc = procs.remove(&handle).ok_or_else(|| {
            ToolError::NotFound(format!("no background process with handle {handle}"))
        })?;

        proc.child
            .kill()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!("killed background process {handle}"),
            }],
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::tools::{NetworkPolicy, ResolvedPermissions, ToolContext};

    fn test_ctx() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::None,
                max_output_bytes: 1_048_576,
                shell: Some(ShellPermission {
                    allowed_commands: vec![],
                    denied_commands: vec![],
                    allow_arbitrary: true,
                }),
                network: None,
            },
            services: None,
        }
    }

    #[tokio::test]
    async fn shell_exec_echo() {
        let tool = ShellExecTool::new();
        let output = tool
            .call(json!({"command": "echo hello"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("hello"));
        } else {
            panic!("expected text content");
        }
        assert_eq!(output.metadata.unwrap()["exit_code"], 0);
    }

    #[tokio::test]
    async fn shell_exec_exit_code() {
        let tool = ShellExecTool::new();
        let output = tool
            .call(json!({"command": "exit 42"}), &test_ctx())
            .await
            .unwrap();

        assert_eq!(output.metadata.unwrap()["exit_code"], 42);
    }

    #[tokio::test]
    async fn shell_background_and_kill() {
        let bg = ShellExecBackgroundTool::new();
        let output = bg
            .call(json!({"command": "sleep 60"}), &test_ctx())
            .await
            .unwrap();

        let handle = output.metadata.unwrap()["handle"].as_u64().unwrap();

        let kill = ShellKillTool::new();
        kill.call(json!({"handle": handle}), &test_ctx())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn shell_denied_by_default_no_permissions() {
        let ctx = ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::None,
                max_output_bytes: 1_048_576,
                shell: None,
                network: None,
            },
            services: None,
        };
        let tool = ShellExecTool::new();
        let result = tool.call(json!({"command": "echo hello"}), &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }

    #[tokio::test]
    async fn shell_allowed_with_matching_permission() {
        let ctx = ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::None,
                max_output_bytes: 1_048_576,
                shell: Some(ShellPermission {
                    allowed_commands: vec!["echo".into()],
                    denied_commands: vec![],
                    allow_arbitrary: false,
                }),
                network: None,
            },
            services: None,
        };
        let tool = ShellExecTool::new();
        let result = tool.call(json!({"command": "echo hello"}), &ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn shell_denied_command_rejected() {
        let ctx = ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "test".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::None,
                max_output_bytes: 1_048_576,
                shell: Some(ShellPermission {
                    allowed_commands: vec!["echo".into(), "rm".into()],
                    denied_commands: vec!["rm".into()],
                    allow_arbitrary: false,
                }),
                network: None,
            },
            services: None,
        };
        let tool = ShellExecTool::new();
        let result = tool.call(json!({"command": "rm -rf /tmp/test"}), &ctx).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }
}
