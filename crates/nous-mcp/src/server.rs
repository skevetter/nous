use std::sync::Arc;

use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::chunk::Chunker;
use nous_core::classify::CategoryClassifier;
use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};

use crate::config::Config;
use crate::tools::*;

pub struct NousServer {
    pub write_channel: WriteChannel,
    #[allow(dead_code)]
    write_handle: tokio::task::JoinHandle<()>,
    pub read_pool: ReadPool,
    pub embedding: Arc<dyn EmbeddingBackend>,
    pub classifier: CategoryClassifier,
    pub chunker: Chunker,
    pub config: Config,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl NousServer {
    pub fn new(
        config: Config,
        embedding: Box<dyn EmbeddingBackend>,
        db_path: &str,
    ) -> nous_shared::Result<Self> {
        let db = MemoryDb::open(db_path, None)?;
        let classifier = CategoryClassifier::new(&db, embedding.as_ref())?;
        let chunker = Chunker::new(config.embedding.chunk_size, config.embedding.chunk_overlap);
        let (write_channel, write_handle) = WriteChannel::new(db);
        let read_pool = ReadPool::new(db_path, None, 4)?;
        let embedding = Arc::from(embedding);

        Ok(Self {
            write_channel,
            write_handle,
            read_pool,
            embedding,
            classifier,
            chunker,
            config,
            tool_router: Self::tool_router(),
        })
    }
}

fn not_implemented() -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::Content::text("not implemented")])
}

#[tool_router]
impl NousServer {
    #[tool(name = "memory_store", description = "Store a new memory")]
    async fn memory_store(&self, params: Parameters<MemoryStoreParams>) -> CallToolResult {
        handle_store(
            params.0,
            &self.write_channel,
            &self.embedding,
            &self.classifier,
            &self.chunker,
        )
        .await
    }

    #[tool(name = "memory_recall", description = "Recall a memory by ID")]
    async fn memory_recall(&self, params: Parameters<MemoryRecallParams>) -> CallToolResult {
        handle_recall(params.0, &self.read_pool, &self.write_channel).await
    }

    #[tool(
        name = "memory_search",
        description = "Search memories using FTS, semantic, or hybrid search"
    )]
    async fn memory_search(&self, _params: Parameters<MemorySearchParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(
        name = "memory_context",
        description = "Get context-relevant memories for a workspace"
    )]
    async fn memory_context(&self, _params: Parameters<MemoryContextParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(
        name = "memory_forget",
        description = "Archive or hard-delete a memory"
    )]
    async fn memory_forget(&self, params: Parameters<MemoryForgetParams>) -> CallToolResult {
        handle_forget(params.0, &self.write_channel).await
    }

    #[tool(name = "memory_unarchive", description = "Restore an archived memory")]
    async fn memory_unarchive(&self, params: Parameters<MemoryUnarchiveParams>) -> CallToolResult {
        handle_unarchive(
            params.0,
            &self.write_channel,
            &self.embedding,
            &self.chunker,
            &self.read_pool,
        )
        .await
    }

    #[tool(name = "memory_update", description = "Update fields on a memory")]
    async fn memory_update(&self, params: Parameters<MemoryUpdateParams>) -> CallToolResult {
        handle_update(
            params.0,
            &self.write_channel,
            &self.embedding,
            &self.chunker,
            &self.read_pool,
        )
        .await
    }

    #[tool(
        name = "memory_relate",
        description = "Create a relationship between two memories"
    )]
    async fn memory_relate(&self, _params: Parameters<MemoryRelateParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(
        name = "memory_unrelate",
        description = "Remove a relationship between two memories"
    )]
    async fn memory_unrelate(&self, _params: Parameters<MemoryUnrelateParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(
        name = "memory_category_suggest",
        description = "Suggest a new category for a memory"
    )]
    async fn memory_category_suggest(
        &self,
        _params: Parameters<MemoryCategorySuggestParams>,
    ) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_workspaces", description = "List known workspaces")]
    async fn memory_workspaces(
        &self,
        _params: Parameters<MemoryWorkspacesParams>,
    ) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_tags", description = "List known tags")]
    async fn memory_tags(&self, _params: Parameters<MemoryTagsParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_stats", description = "Get memory database statistics")]
    async fn memory_stats(&self) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_schema", description = "Get the memory database schema")]
    async fn memory_schema(&self) -> CallToolResult {
        not_implemented()
    }

    #[tool(
        name = "memory_sql",
        description = "Execute a read-only SQL query against the memory database"
    )]
    async fn memory_sql(&self, _params: Parameters<MemorySqlParams>) -> CallToolResult {
        not_implemented()
    }
}

#[tool_handler(name = "nous-mcp", version = "0.1.0")]
impl ServerHandler for NousServer {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn test_db_path() -> String {
        format!(
            "/tmp/nous-test-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    fn test_server(db_path: &str) -> NousServer {
        let cfg = Config::default();
        let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
        NousServer::new(cfg, embedding, db_path).unwrap()
    }

    fn extract_json(result: &CallToolResult) -> serde_json::Value {
        let text = result.content[0].as_text().unwrap().text.as_str();
        serde_json::from_str(text).unwrap()
    }

    fn is_success(result: &CallToolResult) -> bool {
        result.is_error != Some(true)
    }

    #[tokio::test]
    async fn store_and_recall() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let store_result = handle_store(
            MemoryStoreParams {
                title: "Test memory".into(),
                content: "This is a test memory with some content for testing".into(),
                memory_type: "decision".into(),
                tags: vec!["rust".into(), "test".into()],
                source: Some("unit-test".into()),
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        assert!(is_success(&store_result), "store should succeed");
        let store_json = extract_json(&store_result);
        let id = store_json["id"].as_str().unwrap();
        assert!(!id.is_empty());

        // Small delay to let write worker flush and WAL propagate to read connections
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall_result = handle_recall(
            MemoryRecallParams { id: id.into() },
            &server.read_pool,
            &server.write_channel,
        )
        .await;

        assert!(is_success(&recall_result), "recall should succeed");
        let recall_json = extract_json(&recall_result);
        assert_eq!(recall_json["id"].as_str().unwrap(), id);
        assert_eq!(recall_json["title"], "Test memory");
        assert_eq!(
            recall_json["content"],
            "This is a test memory with some content for testing"
        );
        assert_eq!(recall_json["memory_type"], "decision");
        assert_eq!(recall_json["importance"], "moderate");
        assert_eq!(recall_json["confidence"], "moderate");
        assert_eq!(recall_json["source"], "unit-test");
        assert_eq!(recall_json["archived"], false);
        assert_eq!(
            recall_json["tags"].as_array().unwrap(),
            &vec![serde_json::json!("rust"), serde_json::json!("test")]
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn store_auto_classifies_when_no_category() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_store(
            MemoryStoreParams {
                title: "Auto classify test".into(),
                content: "Content for auto-classification".into(),
                memory_type: "fact".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert!(json["id"].as_str().is_some());
        // category_id may or may not be set depending on classifier cache,
        // but the field should exist
        assert!(json.get("category_id").is_some());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn store_uses_explicit_category() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_store(
            MemoryStoreParams {
                title: "Explicit category".into(),
                content: "Memory with explicit category".into(),
                memory_type: "convention".into(),
                tags: vec![],
                source: None,
                importance: Some("high".into()),
                confidence: Some("high".into()),
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: Some(1),
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert_eq!(json["category_id"], 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn update_changes_title() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let store_result = handle_store(
            MemoryStoreParams {
                title: "Original title".into(),
                content: "Some content".into(),
                memory_type: "fact".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        let id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let update_result = handle_update(
            MemoryUpdateParams {
                id: id.clone(),
                title: Some("Updated title".into()),
                content: None,
                tags: None,
                importance: None,
                confidence: None,
                valid_until: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.chunker,
            &server.read_pool,
        )
        .await;

        assert!(is_success(&update_result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall_result = handle_recall(
            MemoryRecallParams { id },
            &server.read_pool,
            &server.write_channel,
        )
        .await;

        let recall_json = extract_json(&recall_result);
        assert_eq!(recall_json["title"], "Updated title");
        assert_eq!(recall_json["content"], "Some content");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn forget_soft_archives() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let store_result = handle_store(
            MemoryStoreParams {
                title: "To forget".into(),
                content: "Content to forget".into(),
                memory_type: "observation".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        let id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let forget_result = handle_forget(
            MemoryForgetParams {
                id: id.clone(),
                hard: false,
            },
            &server.write_channel,
        )
        .await;

        assert!(is_success(&forget_result));
        let forget_json = extract_json(&forget_result);
        assert_eq!(forget_json["status"], "archived");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall_result = handle_recall(
            MemoryRecallParams { id },
            &server.read_pool,
            &server.write_channel,
        )
        .await;

        let recall_json = extract_json(&recall_result);
        assert_eq!(recall_json["archived"], true);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn unarchive_restores() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let store_result = handle_store(
            MemoryStoreParams {
                title: "To archive and restore".into(),
                content: "Content for archive cycle".into(),
                memory_type: "fact".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        let id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Archive
        handle_forget(
            MemoryForgetParams {
                id: id.clone(),
                hard: false,
            },
            &server.write_channel,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Unarchive
        let unarchive_result = handle_unarchive(
            MemoryUnarchiveParams { id: id.clone() },
            &server.write_channel,
            &server.embedding,
            &server.chunker,
            &server.read_pool,
        )
        .await;

        assert!(is_success(&unarchive_result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall_result = handle_recall(
            MemoryRecallParams { id },
            &server.read_pool,
            &server.write_channel,
        )
        .await;

        let recall_json = extract_json(&recall_result);
        assert_eq!(recall_json["archived"], false);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn full_lifecycle() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        // 1. Store
        let store_result = handle_store(
            MemoryStoreParams {
                title: "Lifecycle test".into(),
                content: "Full lifecycle content for testing".into(),
                memory_type: "decision".into(),
                tags: vec!["lifecycle".into()],
                source: None,
                importance: Some("high".into()),
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;
        assert!(is_success(&store_result));
        let id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // 2. Recall
        let recall_result = handle_recall(
            MemoryRecallParams { id: id.clone() },
            &server.read_pool,
            &server.write_channel,
        )
        .await;
        assert!(is_success(&recall_result));
        let r = extract_json(&recall_result);
        assert_eq!(r["title"], "Lifecycle test");
        assert_eq!(r["importance"], "high");

        // 3. Update
        let update_result = handle_update(
            MemoryUpdateParams {
                id: id.clone(),
                title: Some("Lifecycle test - updated".into()),
                content: None,
                tags: Some(vec!["lifecycle".into(), "updated".into()]),
                importance: None,
                confidence: Some("high".into()),
                valid_until: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.chunker,
            &server.read_pool,
        )
        .await;
        assert!(is_success(&update_result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let r = extract_json(
            &handle_recall(
                MemoryRecallParams { id: id.clone() },
                &server.read_pool,
                &server.write_channel,
            )
            .await,
        );
        assert_eq!(r["title"], "Lifecycle test - updated");
        assert_eq!(r["confidence"], "high");
        assert!(
            r["tags"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("updated"))
        );

        // 4. Forget (soft)
        let forget_result = handle_forget(
            MemoryForgetParams {
                id: id.clone(),
                hard: false,
            },
            &server.write_channel,
        )
        .await;
        assert!(is_success(&forget_result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let r = extract_json(
            &handle_recall(
                MemoryRecallParams { id: id.clone() },
                &server.read_pool,
                &server.write_channel,
            )
            .await,
        );
        assert_eq!(r["archived"], true);

        // 5. Unarchive
        let unarchive_result = handle_unarchive(
            MemoryUnarchiveParams { id: id.clone() },
            &server.write_channel,
            &server.embedding,
            &server.chunker,
            &server.read_pool,
        )
        .await;
        assert!(is_success(&unarchive_result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let r = extract_json(
            &handle_recall(
                MemoryRecallParams { id },
                &server.read_pool,
                &server.write_channel,
            )
            .await,
        );
        assert_eq!(r["archived"], false);
        assert_eq!(r["title"], "Lifecycle test - updated");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn store_invalid_memory_type() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_store(
            MemoryStoreParams {
                title: "Bad type".into(),
                content: "Content".into(),
                memory_type: "invalid_type".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        assert_eq!(result.is_error, Some(true));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn recall_nonexistent() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_recall(
            MemoryRecallParams {
                id: "nonexistent-id".into(),
            },
            &server.read_pool,
            &server.write_channel,
        )
        .await;

        assert_eq!(result.is_error, Some(true));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn update_with_content_change_reembeds() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let store_result = handle_store(
            MemoryStoreParams {
                title: "Content change test".into(),
                content: "Original content for embedding".into(),
                memory_type: "fact".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: None,
                trace_id: None,
                agent_id: None,
                agent_model: None,
                valid_from: None,
                category_id: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.classifier,
            &server.chunker,
        )
        .await;

        let id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let update_result = handle_update(
            MemoryUpdateParams {
                id: id.clone(),
                title: None,
                content: Some("Completely new content for re-embedding".into()),
                tags: None,
                importance: None,
                confidence: None,
                valid_until: None,
            },
            &server.write_channel,
            &server.embedding,
            &server.chunker,
            &server.read_pool,
        )
        .await;

        assert!(is_success(&update_result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall_result = handle_recall(
            MemoryRecallParams { id },
            &server.read_pool,
            &server.write_channel,
        )
        .await;

        let recall_json = extract_json(&recall_result);
        assert_eq!(
            recall_json["content"],
            "Completely new content for re-embedding"
        );

        let _ = std::fs::remove_file(&db_path);
    }
}
