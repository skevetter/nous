use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::registry::ToolRegistry;
use super::{
    AgentToolDyn, ExecutionPolicy, NetworkPolicy, ToolCategory, ToolContent, ToolContext,
    ToolError, ToolMetadata, ToolOutput,
};

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
}

impl ToolExecutor {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    pub async fn invoke(
        &self,
        tool_name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .registry
            .get(tool_name)
            .await
            .ok_or_else(|| ToolError::NotFound(tool_name.into()))?;
        self.validate_args(&tool.metadata_dyn().input_schema, &args)?;

        self.authorize(tool.metadata_dyn(), ctx)?;

        let policy = &tool.metadata_dyn().execution_policy;
        let result = self
            .execute_with_policy(tool.as_ref(), args, ctx, policy)
            .await;

        let output = self.capture_output(result, policy.max_output_bytes)?;

        self.record_invocation(tool_name, ctx, &output).await;

        Ok(output)
    }

    fn validate_args(&self, schema: &Value, args: &Value) -> Result<(), ToolError> {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for field in required {
                if let Some(field_name) = field.as_str() {
                    if args.get(field_name).is_none() {
                        return Err(ToolError::InvalidArgs(format!(
                            "missing required field: '{field_name}'"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn authorize(&self, tool_meta: &ToolMetadata, ctx: &ToolContext) -> Result<(), ToolError> {
        let perms = &ctx.permissions;

        if let Some(ref allowed) = perms.allowed_tools {
            if !allowed.iter().any(|t| t == &tool_meta.name) {
                return Err(ToolError::PermissionDenied(format!(
                    "tool '{}' not in agent's allowlist",
                    tool_meta.name
                )));
            }
        }

        if let Some(ref denied) = perms.denied_tools {
            if denied.iter().any(|t| t == &tool_meta.name) {
                return Err(ToolError::PermissionDenied(format!(
                    "tool '{}' is explicitly denied",
                    tool_meta.name
                )));
            }
        }

        if tool_meta.category == ToolCategory::Http && perms.network_access == NetworkPolicy::None {
            return Err(ToolError::PermissionDenied(
                "network access not permitted for this agent".into(),
            ));
        }

        Ok(())
    }

    async fn execute_with_policy(
        &self,
        tool: &dyn AgentToolDyn,
        args: Value,
        ctx: &ToolContext,
        policy: &ExecutionPolicy,
    ) -> Result<ToolOutput, ToolError> {
        let timeout = Duration::from_secs(policy.timeout_secs);
        let mut last_err = None;

        for attempt in 0..=policy.max_retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(
                    policy.retry_delay_ms * attempt as u64,
                ))
                .await;
            }

            match tokio::time::timeout(timeout, tool.call_dyn(args.clone(), ctx)).await {
                Ok(Ok(output)) => return Ok(output),
                Ok(Err(e)) if policy.idempotent && attempt < policy.max_retries => {
                    last_err = Some(e);
                    continue;
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    last_err = Some(ToolError::Timeout(timeout));
                    if !policy.idempotent || attempt >= policy.max_retries {
                        return Err(ToolError::Timeout(timeout));
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| ToolError::ExecutionFailed("unknown".into())))
    }

    fn capture_output(
        &self,
        result: Result<ToolOutput, ToolError>,
        max_bytes: usize,
    ) -> Result<ToolOutput, ToolError> {
        let output = result?;
        let serialized = serde_json::to_string(&output.content)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        if serialized.len() > max_bytes {
            let mut end = max_bytes;
            while end > 0 && !serialized.is_char_boundary(end) {
                end -= 1;
            }
            let truncated = &serialized[..end];
            Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: format!(
                        "{}\n\n[Output truncated at {} bytes. Total: {} bytes]",
                        truncated,
                        end,
                        serialized.len()
                    ),
                }],
                metadata: output.metadata,
            })
        } else {
            Ok(output)
        }
    }

    async fn record_invocation(&self, tool_name: &str, ctx: &ToolContext, _output: &ToolOutput) {
        tracing::info!(
            tool = tool_name,
            agent_id = %ctx.agent_id,
            agent_name = %ctx.agent_name,
            namespace = %ctx.namespace,
            "tool invocation recorded"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::tools::{
        AgentTool, ExecutionPolicy, NetworkPolicy, ResolvedPermissions, ToolCategory, ToolContent,
        ToolContext, ToolMetadata, ToolOutput, ToolPermissions,
    };

    struct SlowTool {
        meta: ToolMetadata,
        delay: Duration,
    }

    impl AgentTool for SlowTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.meta
        }

        async fn call(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolOutput, crate::tools::ToolError> {
            tokio::time::sleep(self.delay).await;
            Ok(ToolOutput {
                content: vec![ToolContent::Text {
                    text: "done".into(),
                }],
                metadata: None,
            })
        }
    }

    struct BigOutputTool {
        meta: ToolMetadata,
    }

    impl AgentTool for BigOutputTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.meta
        }

        async fn call(
            &self,
            _args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolOutput, crate::tools::ToolError> {
            let big_text = "x".repeat(2_000_000);
            Ok(ToolOutput {
                content: vec![ToolContent::Text { text: big_text }],
                metadata: None,
            })
        }
    }

    fn make_meta(name: &str, category: ToolCategory, policy: ExecutionPolicy) -> ToolMetadata {
        ToolMetadata {
            name: name.into(),
            description: format!("Test {name}"),
            category,
            version: "0.1.0".into(),
            input_schema: json!({"type": "object"}),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: policy,
            tags: vec![],
        }
    }

    fn default_ctx() -> ToolContext {
        ToolContext {
            agent_id: "agent-1".into(),
            agent_name: "test-agent".into(),
            namespace: "test".into(),
            workspace_dir: None,
            session_id: None,
            timeout: Duration::from_secs(30),
            permissions: ResolvedPermissions {
                allowed_tools: None,
                denied_tools: None,
                allowed_paths: None,
                network_access: NetworkPolicy::Unrestricted,
                max_output_bytes: 1_048_576,
            },
            services: None,
        }
    }

    #[tokio::test]
    async fn timeout_fires() {
        let registry = ToolRegistry::new();
        let mut policy = ExecutionPolicy::default();
        policy.timeout_secs = 1;

        registry
            .register(SlowTool {
                meta: make_meta("slow_tool", ToolCategory::Custom, policy),
                delay: Duration::from_secs(5),
            })
            .await;

        let executor = ToolExecutor::new(Arc::new(registry));
        let result = executor
            .invoke("slow_tool", json!({}), &default_ctx())
            .await;

        assert!(matches!(result, Err(ToolError::Timeout(_))));
    }

    #[tokio::test]
    async fn output_truncation() {
        let registry = ToolRegistry::new();
        let mut policy = ExecutionPolicy::default();
        policy.max_output_bytes = 100;

        registry
            .register(BigOutputTool {
                meta: make_meta("big_tool", ToolCategory::Custom, policy),
            })
            .await;

        let executor = ToolExecutor::new(Arc::new(registry));
        let output = executor
            .invoke("big_tool", json!({}), &default_ctx())
            .await
            .unwrap();

        assert_eq!(output.content.len(), 1);
        if let ToolContent::Text { ref text } = output.content[0] {
            assert!(text.contains("[Output truncated at 100 bytes"));
        } else {
            panic!("expected Text content");
        }
    }

    #[tokio::test]
    async fn not_found_error() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(Arc::new(registry));
        let result = executor
            .invoke("nonexistent", json!({}), &default_ctx())
            .await;

        assert!(matches!(result, Err(ToolError::NotFound(_))));
    }

    fn make_output(text: &str) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: text.to_string(),
            }],
            metadata: None,
        })
    }

    #[test]
    fn capture_output_no_truncation() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(Arc::new(registry));
        let result = executor.capture_output(make_output("short"), 10_000);
        let output = result.unwrap();
        if let ToolContent::Text { ref text } = output.content[0] {
            assert!(!text.contains("[Output truncated"));
        }
    }

    #[test]
    fn capture_output_emoji_no_panic() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(Arc::new(registry));
        let emoji_text = "🦀".repeat(500);
        let result = executor.capture_output(make_output(&emoji_text), 100);
        let output = result.unwrap();
        if let ToolContent::Text { ref text } = output.content[0] {
            assert!(text.contains("[Output truncated at"));
            assert!(text.contains("bytes. Total:"));
        } else {
            panic!("expected Text content");
        }
    }

    #[test]
    fn capture_output_cjk_no_panic() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(Arc::new(registry));
        let cjk_text = "漢字".repeat(500);
        let result = executor.capture_output(make_output(&cjk_text), 200);
        let output = result.unwrap();
        if let ToolContent::Text { ref text } = output.content[0] {
            assert!(text.contains("[Output truncated at"));
            assert!(text.contains("bytes. Total:"));
        } else {
            panic!("expected Text content");
        }
    }

    #[test]
    fn capture_output_mixed_multibyte_no_panic() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(Arc::new(registry));
        let mixed = format!("{}🎉{}", "a".repeat(50), "漢".repeat(100));
        let result = executor.capture_output(make_output(&mixed), 80);
        let output = result.unwrap();
        if let ToolContent::Text { ref text } = output.content[0] {
            assert!(text.contains("[Output truncated at"));
            assert!(text.contains("bytes. Total:"));
        } else {
            panic!("expected Text content");
        }
    }

    #[test]
    fn capture_output_empty_string() {
        let registry = ToolRegistry::new();
        let executor = ToolExecutor::new(Arc::new(registry));
        let result = executor.capture_output(make_output(""), 10_000);
        let output = result.unwrap();
        if let ToolContent::Text { ref text } = output.content[0] {
            assert!(!text.contains("[Output truncated"));
        }
    }
}
