use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};

use super::{AgentToolDyn, ToolContent, ToolContext};

pub struct NousToolAdapter {
    inner: Arc<dyn AgentToolDyn>,
    ctx: ToolContext,
}

impl NousToolAdapter {
    pub fn new(tool: Arc<dyn AgentToolDyn>, ctx: ToolContext) -> Self {
        Self { inner: tool, ctx }
    }
}

#[derive(Debug, Deserialize)]
pub struct GenericArgs {
    #[serde(flatten)]
    pub args: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct GenericOutput {
    pub content: String,
    pub is_error: bool,
}

impl std::fmt::Display for GenericOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct AdapterError {
    pub message: String,
}

impl Tool for NousToolAdapter {
    const NAME: &'static str = "nous_tool";

    type Error = AdapterError;
    type Args = GenericArgs;
    type Output = GenericOutput;

    fn name(&self) -> String {
        self.inner.metadata_dyn().name.clone()
    }

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let meta = self.inner.metadata_dyn();
        ToolDefinition {
            name: meta.name.clone(),
            description: meta.description.clone(),
            parameters: meta.input_schema.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match self.inner.call_dyn(args.args, &self.ctx).await {
            Ok(output) => {
                let text = output
                    .content
                    .iter()
                    .map(|c| match c {
                        ToolContent::Text { text } => text.clone(),
                        ToolContent::Json { data } => {
                            serde_json::to_string_pretty(data).unwrap_or_default()
                        }
                        ToolContent::Error { message } => format!("Error: {message}"),
                        ToolContent::Binary { mime_type, .. } => format!("[binary: {mime_type}]"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(GenericOutput {
                    content: text,
                    is_error: false,
                })
            }
            Err(e) => Ok(GenericOutput {
                content: e.to_string(),
                is_error: true,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::tools::{
        AgentTool, ExecutionPolicy, NetworkPolicy, ResolvedPermissions, ToolCategory, ToolError,
        ToolMetadata, ToolOutput, ToolPermissions,
    };

    fn test_context() -> ToolContext {
        ToolContext {
            agent_id: "test-agent".into(),
            agent_name: "Test Agent".into(),
            namespace: "default".into(),
            workspace_dir: Some(PathBuf::from("/tmp")),
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::None,
                max_output_bytes: 1_048_576,
            },
        }
    }

    struct MockTool {
        meta: ToolMetadata,
    }

    impl MockTool {
        fn new(name: &str, description: &str) -> Self {
            Self {
                meta: ToolMetadata {
                    name: name.into(),
                    description: description.into(),
                    category: ToolCategory::Custom,
                    version: "0.1.0".into(),
                    input_schema: json!({"type": "object", "properties": {"msg": {"type": "string"}}}),
                    output_schema: None,
                    permissions: ToolPermissions::default(),
                    execution_policy: ExecutionPolicy::default(),
                    tags: vec![],
                },
            }
        }
    }

    impl AgentTool for MockTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.meta
        }

        async fn call(
            &self,
            args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolOutput, ToolError> {
            let msg = args
                .get("msg")
                .and_then(|v| v.as_str())
                .unwrap_or("no message");
            Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: format!("echo: {msg}"),
                }],
                metadata: None,
            })
        }
    }

    struct ErrorTool {
        meta: ToolMetadata,
    }

    impl ErrorTool {
        fn new() -> Self {
            Self {
                meta: ToolMetadata {
                    name: "error_tool".into(),
                    description: "A tool that always errors".into(),
                    category: ToolCategory::Custom,
                    version: "0.1.0".into(),
                    input_schema: json!({"type": "object"}),
                    output_schema: None,
                    permissions: ToolPermissions::default(),
                    execution_policy: ExecutionPolicy::default(),
                    tags: vec![],
                },
            }
        }
    }

    impl AgentTool for ErrorTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.meta
        }

        async fn call(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolOutput, ToolError> {
            Err(ToolError::ExecutionFailed("something went wrong".into()))
        }
    }

    #[tokio::test]
    async fn adapter_definition_matches_metadata() {
        let tool = MockTool::new("my_tool", "My custom tool");
        let adapter = NousToolAdapter::new(Arc::new(tool), test_context());

        let def = adapter.definition(String::new()).await;
        assert_eq!(def.name, "my_tool");
        assert_eq!(def.description, "My custom tool");
    }

    #[tokio::test]
    async fn adapter_name_returns_dynamic_name() {
        let tool = MockTool::new("dynamic_name", "A tool");
        let adapter = NousToolAdapter::new(Arc::new(tool), test_context());

        assert_eq!(adapter.name(), "dynamic_name");
    }

    #[tokio::test]
    async fn adapter_call_returns_output() {
        let tool = MockTool::new("echo", "Echo tool");
        let adapter = NousToolAdapter::new(Arc::new(tool), test_context());

        let result = adapter
            .call(GenericArgs {
                args: json!({"msg": "hello"}),
            })
            .await
            .unwrap();

        assert_eq!(result.content, "echo: hello");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn adapter_error_returns_generic_output_with_is_error() {
        let tool = ErrorTool::new();
        let adapter = NousToolAdapter::new(Arc::new(tool), test_context());

        let result = adapter.call(GenericArgs { args: json!({}) }).await.unwrap();

        assert!(result.is_error);
        assert!(result.content.contains("something went wrong"));
    }
}
