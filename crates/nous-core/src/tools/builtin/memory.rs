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
        let memory_type = args
            .get("memory_type")
            .and_then(|v| v.as_str())
            .unwrap_or("observation");
        let tags: Vec<String> = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .save_memory(crate::tools::SaveMemoryParams {
                workspace_id: None,
                agent_id: ctx.agent_id.clone(),
                content: content.to_string(),
                memory_type: memory_type.to_string(),
                importance: "moderate".to_string(),
                tags,
            })
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .search_memories(crate::tools::SearchMemoriesParams {
                query: query.to_string(),
                agent_id: Some(ctx.agent_id.clone()),
                workspace_id: None,
                memory_type: None,
                limit,
            })
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
        let fts_weight = args.get("fts_weight").and_then(|v| v.as_f64());

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .search_memories_hybrid(crate::tools::SearchMemoriesHybridParams {
                query: query.to_string(),
                agent_id: Some(ctx.agent_id.clone()),
                limit,
                fts_weight,
            })
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .get_memory_context(crate::tools::GetMemoryContextParams {
                agent_id: Some(ctx.agent_id.clone()),
                workspace_id: None,
                topic_key: None,
                limit,
            })
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .relate_memories(
                source_id.to_string(),
                target_id.to_string(),
                relation.to_string(),
            )
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
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
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .map(String::from);
        let importance = args
            .get("importance")
            .and_then(|v| v.as_str())
            .map(String::from);

        let services = ctx
            .services
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed("no services configured".into()))?;

        let result = services
            .update_memory(memory_id.to_string(), content, importance)
            .await?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: result }],
            metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::tools::{NetworkPolicy, ResolvedPermissions, ToolContext, ToolServices};

    struct MockToolServices;

    #[async_trait::async_trait]
    impl ToolServices for MockToolServices {
        async fn save_memory(&self, params: crate::tools::SaveMemoryParams) -> Result<Value, ToolError> {
            let crate::tools::SaveMemoryParams { agent_id, content, memory_type, tags, .. } = params;
            Ok(json!({
                "id": "mem-001",
                "agent_id": agent_id,
                "content": content,
                "memory_type": memory_type,
                "tags": tags,
            }))
        }

        async fn search_memories(
            &self,
            params: crate::tools::SearchMemoriesParams,
        ) -> Result<Value, ToolError> {
            let crate::tools::SearchMemoriesParams { query, limit, .. } = params;
            Ok(json!({
                "query": query,
                "limit": limit.unwrap_or(10),
                "results": [],
            }))
        }

        async fn search_memories_hybrid(
            &self,
            params: crate::tools::SearchMemoriesHybridParams,
        ) -> Result<Value, ToolError> {
            let crate::tools::SearchMemoriesHybridParams { query, limit, fts_weight, .. } = params;
            Ok(json!({
                "query": query,
                "limit": limit.unwrap_or(10),
                "fts_weight": fts_weight.unwrap_or(0.5),
                "results": [],
            }))
        }

        async fn get_memory_context(
            &self,
            params: crate::tools::GetMemoryContextParams,
        ) -> Result<Value, ToolError> {
            let limit = params.limit;
            Ok(json!({
                "limit": limit.unwrap_or(20),
                "memories": [],
            }))
        }

        async fn relate_memories(
            &self,
            source_id: String,
            target_id: String,
            relation_type: String,
        ) -> Result<Value, ToolError> {
            Ok(json!({
                "source_id": source_id,
                "target_id": target_id,
                "relation_type": relation_type,
            }))
        }

        async fn update_memory(
            &self,
            memory_id: String,
            content: Option<String>,
            importance: Option<String>,
        ) -> Result<Value, ToolError> {
            Ok(json!({
                "memory_id": memory_id,
                "content": content,
                "importance": importance,
            }))
        }

        async fn post_to_room(
            &self,
            _params: crate::tools::PostToRoomParams,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn read_room(&self, _room: String, _limit: Option<u32>) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn create_room(
            &self,
            _name: String,
            _purpose: Option<String>,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn wait_for_message(
            &self,
            _room: String,
            _timeout_secs: u64,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn create_task(
            &self,
            _params: crate::tools::ToolCreateTaskParams,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }

        async fn update_task(
            &self,
            _task_id: String,
            _status: Option<String>,
            _note: Option<String>,
        ) -> Result<Value, ToolError> {
            Ok(json!({}))
        }
    }

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
                shell: None,
                network: None,
            },
            services: None,
        }
    }

    fn test_ctx_with_services() -> ToolContext {
        ToolContext {
            services: Some(Arc::new(MockToolServices)),
            ..test_ctx()
        }
    }

    #[tokio::test]
    async fn memory_save_no_services_returns_error() {
        let tool = MemorySaveTool::new();
        let result = tool
            .call(json!({"content": "test memory"}), &test_ctx())
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no services configured"));
    }

    #[tokio::test]
    async fn memory_save_delegates_to_services() {
        let tool = MemorySaveTool::new();
        let output = tool
            .call(
                json!({"content": "test memory", "tags": ["tag1"]}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["id"], "mem-001");
            assert_eq!(data["content"], "test memory");
            assert_eq!(data["agent_id"], "test-agent");
            assert_eq!(data["tags"][0], "tag1");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn memory_search_no_services_returns_error() {
        let tool = MemorySearchTool::new();
        let result = tool.call(json!({"query": "test query"}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn memory_search_delegates_to_services() {
        let tool = MemorySearchTool::new();
        let output = tool
            .call(
                json!({"query": "test query", "limit": 5}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["query"], "test query");
            assert_eq!(data["limit"], 5);
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn memory_search_hybrid_delegates_to_services() {
        let tool = MemorySearchHybridTool::new();
        let output = tool
            .call(
                json!({"query": "hybrid test", "fts_weight": 0.7}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["query"], "hybrid test");
            assert_eq!(data["fts_weight"], 0.7);
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn memory_get_context_delegates_to_services() {
        let tool = MemoryGetContextTool::new();
        let output = tool
            .call(json!({"limit": 5}), &test_ctx_with_services())
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["limit"], 5);
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn memory_relate_validates_args() {
        let tool = MemoryRelateTool::new();
        let result = tool
            .call(json!({"source_id": "a"}), &test_ctx_with_services())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn memory_relate_delegates_to_services() {
        let tool = MemoryRelateTool::new();
        let output = tool
            .call(
                json!({"source_id": "a", "target_id": "b", "relation": "supports"}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["source_id"], "a");
            assert_eq!(data["target_id"], "b");
            assert_eq!(data["relation_type"], "supports");
        } else {
            panic!("expected json content");
        }
    }

    #[tokio::test]
    async fn memory_update_validates_args() {
        let tool = MemoryUpdateTool::new();
        let result = tool
            .call(json!({"content": "no id"}), &test_ctx_with_services())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn memory_update_delegates_to_services() {
        let tool = MemoryUpdateTool::new();
        let output = tool
            .call(
                json!({"memory_id": "mem-123", "content": "updated content"}),
                &test_ctx_with_services(),
            )
            .await
            .unwrap();

        if let ToolContent::Json { data } = &output.content[0] {
            assert_eq!(data["memory_id"], "mem-123");
            assert_eq!(data["content"], "updated content");
        } else {
            panic!("expected json content");
        }
    }
}
