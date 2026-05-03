use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::tools::{
    AgentTool, ExecutionPolicy, ToolCategory, ToolContent, ToolContext, ToolError, ToolMetadata,
    ToolOutput, ToolPermissions,
};

// --- MemorySaveTool ---

#[derive(Default)]
pub struct MemorySaveTool {
    meta: OnceLock<ToolMetadata>,
}

impl MemorySaveTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "memory_save".into(),
            description: "Save a memory (decision, convention, fact, etc.)".into(),
            category: ToolCategory::Memory,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string", "description": "Memory content to save" },
                    "memory_type": { "type": "string", "description": "Type: decision, convention, fact, observation" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for categorization" }
                },
                "required": ["content"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["memory".into(), "save".into()],
        })
    }
}

impl AgentTool for MemorySaveTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'content' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "memory_save: would save '{}' for agent {}",
                    content, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- MemorySearchTool ---

#[derive(Default)]
pub struct MemorySearchTool {
    meta: OnceLock<ToolMetadata>,
}

impl MemorySearchTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "memory_search".into(),
            description: "Search memories via FTS5 full-text search".into(),
            category: ToolCategory::Memory,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" }
                },
                "required": ["query"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["memory".into(), "search".into()],
        })
    }
}

impl AgentTool for MemorySearchTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'query' required".into()))?;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "memory_search: would search '{}' (limit {}) for agent {}",
                    query, limit, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- MemorySearchHybridTool ---

#[derive(Default)]
pub struct MemorySearchHybridTool {
    meta: OnceLock<ToolMetadata>,
}

impl MemorySearchHybridTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "memory_search_hybrid".into(),
            description: "Hybrid FTS + vector search across memories".into(),
            category: ToolCategory::Memory,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" },
                    "fts_weight": { "type": "number", "description": "FTS weight (0.0-1.0, default: 0.5)" }
                },
                "required": ["query"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 15,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["memory".into(), "search".into(), "hybrid".into()],
        })
    }
}

impl AgentTool for MemorySearchHybridTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'query' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "memory_search_hybrid: would hybrid-search '{}' for agent {}",
                    query, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- MemoryGetContextTool ---

#[derive(Default)]
pub struct MemoryGetContextTool {
    meta: OnceLock<ToolMetadata>,
}

impl MemoryGetContextTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "memory_get_context".into(),
            description: "Get recent context memories for the current session".into(),
            category: ToolCategory::Memory,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max memories to return (default: 20)" }
                }
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                idempotent: true,
                ..Default::default()
            },
            tags: vec!["memory".into(), "context".into()],
        })
    }
}

impl AgentTool for MemoryGetContextTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "memory_get_context: would return {} recent memories for agent {}",
                    limit, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- MemoryRelateTool ---

#[derive(Default)]
pub struct MemoryRelateTool {
    meta: OnceLock<ToolMetadata>,
}

impl MemoryRelateTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "memory_relate".into(),
            description: "Create a relationship between two memories".into(),
            category: ToolCategory::Memory,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string", "description": "Source memory ID" },
                    "target_id": { "type": "string", "description": "Target memory ID" },
                    "relation": { "type": "string", "description": "Relationship type (e.g. supports, contradicts, extends)" }
                },
                "required": ["source_id", "target_id", "relation"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["memory".into(), "relate".into()],
        })
    }
}

impl AgentTool for MemoryRelateTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let source_id = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'source_id' required".into()))?;
        let target_id = args
            .get("target_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'target_id' required".into()))?;
        let relation = args
            .get("relation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'relation' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "memory_relate: would create '{}' relationship from {} to {} for agent {}",
                    relation, source_id, target_id, ctx.agent_id
                ),
            }],
            metadata: None,
        })
    }
}

// --- MemoryUpdateTool ---

#[derive(Default)]
pub struct MemoryUpdateTool {
    meta: OnceLock<ToolMetadata>,
}

impl MemoryUpdateTool {
    pub fn new() -> Self {
        Self {
            meta: OnceLock::new(),
        }
    }

    fn init_meta(&self) -> &ToolMetadata {
        self.meta.get_or_init(|| ToolMetadata {
            name: "memory_update".into(),
            description: "Update an existing memory's content or metadata".into(),
            category: ToolCategory::Memory,
            version: "0.1.0".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "ID of memory to update" },
                    "content": { "type": "string", "description": "New content" },
                    "importance": { "type": "string", "description": "New importance level" }
                },
                "required": ["memory_id"]
            }),
            output_schema: None,
            permissions: ToolPermissions::default(),
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["memory".into(), "update".into()],
        })
    }
}

impl AgentTool for MemoryUpdateTool {
    fn metadata(&self) -> &ToolMetadata {
        self.init_meta()
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let memory_id = args
            .get("memory_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'memory_id' required".into()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "memory_update: would update {} for agent {}",
                    memory_id, ctx.agent_id
                ),
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
            },
        }
    }

    #[tokio::test]
    async fn memory_save_stub() {
        let tool = MemorySaveTool::new();
        let output = tool
            .call(json!({"content": "test memory"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("memory_save"));
            assert!(text.contains("test memory"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn memory_search_stub() {
        let tool = MemorySearchTool::new();
        let output = tool
            .call(json!({"query": "test query"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("memory_search"));
            assert!(text.contains("test query"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn memory_relate_validates_args() {
        let tool = MemoryRelateTool::new();
        let result = tool.call(json!({"source_id": "a"}), &test_ctx()).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn memory_get_context_stub() {
        let tool = MemoryGetContextTool::new();
        let output = tool.call(json!({}), &test_ctx()).await.unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("memory_get_context"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn memory_search_hybrid_stub() {
        let tool = MemorySearchHybridTool::new();
        let output = tool
            .call(json!({"query": "hybrid test"}), &test_ctx())
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("memory_search_hybrid"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn memory_update_stub() {
        let tool = MemoryUpdateTool::new();
        let output = tool
            .call(
                json!({"memory_id": "mem-123", "content": "updated content"}),
                &test_ctx(),
            )
            .await
            .unwrap();

        if let ToolContent::Text { text } = &output.content[0] {
            assert!(text.contains("memory_update"));
            assert!(text.contains("mem-123"));
        } else {
            panic!("expected text content");
        }
    }

    #[tokio::test]
    async fn memory_update_validates_args() {
        let tool = MemoryUpdateTool::new();
        let result = tool.call(json!({"content": "no id"}), &test_ctx()).await;

        assert!(result.is_err());
    }
}
