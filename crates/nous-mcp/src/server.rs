use std::sync::Arc;

use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::chunk::Chunker;
use nous_core::classify::CategoryClassifier;
use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use nous_core::scheduler::Scheduler;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use tokio::sync::Notify;

use crate::config::Config;
use crate::tools::*;

pub struct NousServer {
    pub write_channel: WriteChannel,
    _write_handle: tokio::task::JoinHandle<()>,
    pub read_pool: ReadPool,
    pub embedding: Arc<dyn EmbeddingBackend>,
    pub classifier: CategoryClassifier,
    pub chunker: Chunker,
    pub config: Config,
    pub db_path: String,
    pub scheduler_notify: Arc<Notify>,
    _scheduler_handle: Option<tokio::task::JoinHandle<()>>,
    _tool_router: ToolRouter<Self>,
}

impl NousServer {
    pub fn new(
        config: Config,
        embedding: Box<dyn EmbeddingBackend>,
        db_path: &str,
    ) -> nous_shared::Result<Self> {
        let new_dim = embedding.dimensions();
        let db = MemoryDb::open(db_path, None, new_dim)?;

        let old_dim = db
            .active_model()
            .ok()
            .flatten()
            .map(|m| m.dimensions as usize);

        db.register_and_activate_model(
            embedding.model_id(),
            None,
            new_dim as i64,
            embedding.max_tokens() as i64,
            config.embedding.chunk_size as i64,
            config.embedding.chunk_overlap as i64,
        )?;

        if let Some(prev) = old_dim
            && prev != new_dim
        {
            eprintln!(
                "warning: embedding dimensions changed ({prev} -> {new_dim}), resetting vec0 table"
            );
            db.reset_embeddings(new_dim)?;
        }

        let classifier = CategoryClassifier::new(
            &db,
            embedding.as_ref(),
            config.classification.confidence_threshold as f32,
        )?;
        let chunker = Chunker::new(config.embedding.chunk_size, config.embedding.chunk_overlap);
        let (write_channel, write_handle) = WriteChannel::new(db);
        let read_pool = ReadPool::new(db_path, None, 4)?;
        let embedding = Arc::from(embedding);

        let (scheduler_notify, scheduler_handle) = if config.schedule.enabled {
            let (notify, handle) = Scheduler::spawn(
                write_channel.clone(),
                read_pool.clone(),
                config.schedule.clone(),
            );
            (notify, Some(handle))
        } else {
            (Arc::new(Notify::new()), None)
        };

        Ok(Self {
            write_channel,
            _write_handle: write_handle,
            read_pool,
            embedding,
            classifier,
            chunker,
            config,
            db_path: db_path.to_owned(),
            scheduler_notify,
            _scheduler_handle: scheduler_handle,
            _tool_router: Self::tool_router(),
        })
    }
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
    async fn memory_search(&self, params: Parameters<MemorySearchParams>) -> CallToolResult {
        handle_search(params.0, &self.db_path, &self.embedding).await
    }

    #[tool(
        name = "memory_context",
        description = "Get context-relevant memories for a workspace"
    )]
    async fn memory_context(&self, params: Parameters<MemoryContextParams>) -> CallToolResult {
        handle_context(params.0, &self.db_path).await
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
        description = "Create a typed relationship between two memories (related, supersedes, contradicts, depends_on). supersedes auto-sets valid_until on target."
    )]
    async fn memory_relate(&self, params: Parameters<MemoryRelateParams>) -> CallToolResult {
        handle_relate(params.0, &self.write_channel).await
    }

    #[tool(
        name = "memory_unrelate",
        description = "Remove a relationship between two memories"
    )]
    async fn memory_unrelate(&self, params: Parameters<MemoryUnrelateParams>) -> CallToolResult {
        handle_unrelate(params.0, &self.write_channel).await
    }

    #[tool(
        name = "memory_category_suggest",
        description = "Suggest a new category, compute its embedding, and assign it to a memory"
    )]
    async fn memory_category_suggest(
        &self,
        params: Parameters<MemoryCategorySuggestParams>,
    ) -> CallToolResult {
        handle_category_suggest(params.0, &self.write_channel, &self.embedding).await
    }

    #[tool(
        name = "memory_category_list",
        description = "List all categories as a tree. Optionally filter by source (system, user, agent)."
    )]
    async fn memory_category_list(
        &self,
        params: Parameters<MemoryCategoryListParams>,
    ) -> CallToolResult {
        handle_category_list(&self.db_path, params.0.source).await
    }

    #[tool(
        name = "memory_category_add",
        description = "Create a new user-sourced category with optional parent, description, and threshold"
    )]
    async fn memory_category_add(
        &self,
        params: Parameters<MemoryCategoryAddParams>,
    ) -> CallToolResult {
        handle_category_add(params.0, &self.db_path, &self.embedding).await
    }

    #[tool(
        name = "memory_category_delete",
        description = "Delete a category by name. Refuses if children exist. Orphaned memories get category_id set to NULL."
    )]
    async fn memory_category_delete(
        &self,
        params: Parameters<MemoryCategoryDeleteParams>,
    ) -> CallToolResult {
        handle_category_delete(params.0, &self.write_channel).await
    }

    #[tool(
        name = "memory_category_update",
        description = "Update a category's name, description, and/or threshold"
    )]
    async fn memory_category_update(
        &self,
        params: Parameters<MemoryCategoryUpdateParams>,
    ) -> CallToolResult {
        handle_category_update(
            params.0,
            &self.write_channel,
            &self.read_pool,
            &self.embedding,
        )
        .await
    }

    #[tool(
        name = "memory_workspaces",
        description = "List all workspaces with memory counts"
    )]
    async fn memory_workspaces(
        &self,
        params: Parameters<MemoryWorkspacesParams>,
    ) -> CallToolResult {
        handle_workspaces(&self.read_pool, params.0.source).await
    }

    #[tool(
        name = "memory_tags",
        description = "List all tags with usage counts ordered by frequency"
    )]
    async fn memory_tags(&self, params: Parameters<MemoryTagsParams>) -> CallToolResult {
        handle_tags(&self.read_pool, params.0.prefix).await
    }

    #[tool(
        name = "memory_stats",
        description = "Get memory database statistics: counts by type, category, importance, workspace, top tags, access frequency"
    )]
    async fn memory_stats(&self) -> CallToolResult {
        handle_stats(&self.read_pool).await
    }

    #[tool(
        name = "memory_schema",
        description = "Return the current database schema SQL text"
    )]
    async fn memory_schema(&self) -> CallToolResult {
        handle_schema(&self.read_pool).await
    }

    #[tool(
        name = "memory_sql",
        description = "Execute a read-only SQL query against the memory database. Only SELECT, EXPLAIN, read-only PRAGMA, and read-only WITH are allowed."
    )]
    async fn memory_sql(&self, params: Parameters<MemorySqlParams>) -> CallToolResult {
        handle_sql(params.0, &self.read_pool).await
    }

    #[tool(
        name = "otlp_trace_context",
        description = "Get correlated memories, spans, and logs for a given trace ID. Optionally provide a session_id to also fetch logs."
    )]
    async fn otlp_trace_context(
        &self,
        params: Parameters<OtlpTraceContextParams>,
    ) -> CallToolResult {
        handle_otlp_trace_context(params.0, &self.config.otlp.db_path, &self.read_pool).await
    }

    #[tool(
        name = "otlp_memory_context",
        description = "Look up a memory by ID and fetch correlated OTLP spans and logs using the memory's trace_id and session_id."
    )]
    async fn otlp_memory_context(
        &self,
        params: Parameters<OtlpMemoryContextParams>,
    ) -> CallToolResult {
        handle_otlp_memory_context(params.0, &self.config.otlp.db_path, &self.read_pool).await
    }

    #[tool(name = "room_create", description = "Create a new conversation room")]
    async fn room_create(&self, params: Parameters<RoomCreateParams>) -> CallToolResult {
        handle_room_create(params.0, &self.write_channel).await
    }

    #[tool(
        name = "room_list",
        description = "List rooms. Set archived=true to list archived rooms."
    )]
    async fn room_list(&self, params: Parameters<RoomListParams>) -> CallToolResult {
        handle_room_list(params.0, &self.read_pool).await
    }

    #[tool(name = "room_get", description = "Get a room by ID (UUIDv7) or name")]
    async fn room_get(&self, params: Parameters<RoomGetParams>) -> CallToolResult {
        handle_room_get(params.0, &self.read_pool).await
    }

    #[tool(
        name = "room_delete",
        description = "Archive or hard-delete a room. Set hard=true to permanently delete."
    )]
    async fn room_delete(&self, params: Parameters<RoomDeleteParams>) -> CallToolResult {
        handle_room_delete(params.0, &self.write_channel).await
    }

    #[tool(
        name = "room_post_message",
        description = "Post a message to a room. Room can be specified by ID or name."
    )]
    async fn room_post_message(&self, params: Parameters<RoomPostMessageParams>) -> CallToolResult {
        handle_room_post_message(params.0, &self.write_channel, &self.read_pool).await
    }

    #[tool(
        name = "room_read_messages",
        description = "Read messages from a room with optional pagination. Room can be specified by ID or name."
    )]
    async fn room_read_messages(
        &self,
        params: Parameters<RoomReadMessagesParams>,
    ) -> CallToolResult {
        handle_room_read_messages(params.0, &self.read_pool).await
    }

    #[tool(
        name = "room_search",
        description = "Search messages within a room using full-text search. Room can be specified by ID or name."
    )]
    async fn room_search(&self, params: Parameters<RoomSearchParams>) -> CallToolResult {
        handle_room_search(params.0, &self.read_pool).await
    }

    #[tool(
        name = "room_info",
        description = "Get room details including participants and message count. Room can be specified by ID or name."
    )]
    async fn room_info(&self, params: Parameters<RoomInfoParams>) -> CallToolResult {
        handle_room_info(params.0, &self.read_pool).await
    }

    #[tool(
        name = "room_join",
        description = "Add a participant to a room with a role (owner, member, observer). Room can be specified by ID or name."
    )]
    async fn room_join(&self, params: Parameters<RoomJoinParams>) -> CallToolResult {
        handle_room_join(params.0, &self.write_channel, &self.read_pool).await
    }

    #[tool(
        name = "schedule_create",
        description = "Register a new cron schedule. Validates cron_expr and returns the schedule ID."
    )]
    async fn schedule_create(&self, params: Parameters<ScheduleCreateParams>) -> CallToolResult {
        handle_schedule_create(params.0, &self.write_channel, &self.scheduler_notify).await
    }

    #[tool(
        name = "schedule_list",
        description = "List schedules ordered by next fire time. Filter by enabled status or action_type."
    )]
    async fn schedule_list(&self, params: Parameters<ScheduleListParams>) -> CallToolResult {
        handle_schedule_list(params.0, &self.read_pool).await
    }

    #[tool(
        name = "schedule_get",
        description = "Get full schedule detail including last 10 runs."
    )]
    async fn schedule_get(&self, params: Parameters<ScheduleGetParams>) -> CallToolResult {
        handle_schedule_get(params.0, &self.read_pool).await
    }

    #[tool(
        name = "schedule_update",
        description = "Modify a schedule. Recomputes next_run_at if cron_expr changes."
    )]
    async fn schedule_update(&self, params: Parameters<ScheduleUpdateParams>) -> CallToolResult {
        handle_schedule_update(
            params.0,
            &self.write_channel,
            &self.read_pool,
            &self.scheduler_notify,
        )
        .await
    }

    #[tool(
        name = "schedule_delete",
        description = "Remove a schedule and all its run history."
    )]
    async fn schedule_delete(&self, params: Parameters<ScheduleDeleteParams>) -> CallToolResult {
        handle_schedule_delete(params.0, &self.write_channel, &self.scheduler_notify).await
    }

    #[tool(
        name = "schedule_pause",
        description = "Pause a schedule (set enabled=false). Optionally auto-resume after duration_secs."
    )]
    async fn schedule_pause(&self, params: Parameters<SchedulePauseParams>) -> CallToolResult {
        handle_schedule_pause(params.0, &self.write_channel, &self.scheduler_notify).await
    }

    #[tool(
        name = "schedule_resume",
        description = "Resume a paused schedule and recompute next_run_at."
    )]
    async fn schedule_resume(&self, params: Parameters<ScheduleResumeParams>) -> CallToolResult {
        handle_schedule_resume(params.0, &self.write_channel, &self.scheduler_notify).await
    }
}

#[tool_handler(name = "nous-mcp", version = "0.1.0")]
impl ServerHandler for NousServer {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use nous_otlp::db::OtlpDb;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn test_db_path() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(
            "/tmp/nous-test-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            seq,
        )
    }

    fn test_server(db_path: &str) -> NousServer {
        let mut cfg = Config::default();
        cfg.encryption.db_key_file = format!("{db_path}.key");
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

    async fn store_test_memory_simple(
        server: &NousServer,
        title: &str,
        tags: Vec<String>,
    ) -> String {
        let result = handle_store(
            MemoryStoreParams {
                title: title.into(),
                content: format!("Content for {title}"),
                memory_type: "decision".into(),
                tags,
                source: Some("test".into()),
                importance: None,
                confidence: None,
                workspace_path: Some("/tmp/test-workspace".into()),
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
        extract_json(&result)["id"].as_str().unwrap().to_string()
    }

    async fn store_test_memory(
        server: &NousServer,
        title: &str,
        content: &str,
        memory_type: &str,
        importance: Option<&str>,
        workspace_path: Option<&str>,
    ) -> String {
        let result = handle_store(
            MemoryStoreParams {
                title: title.into(),
                content: content.into(),
                memory_type: memory_type.into(),
                tags: vec![],
                source: None,
                importance: importance.map(String::from),
                confidence: None,
                workspace_path: workspace_path.map(String::from),
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
        extract_json(&result)["id"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn relate_supersedes_sets_valid_until() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let id1 = store_test_memory_simple(&server, "Original decision", vec![]).await;
        let id2 = store_test_memory_simple(&server, "Updated decision", vec![]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_relate(
            MemoryRelateParams {
                source_id: id2.clone(),
                target_id: id1.clone(),
                relation_type: "supersedes".into(),
            },
            &server.write_channel,
        )
        .await;
        assert!(is_success(&result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall = handle_recall(
            MemoryRecallParams { id: id1.clone() },
            &server.read_pool,
            &server.write_channel,
        )
        .await;
        let json = extract_json(&recall);
        assert!(
            json["valid_until"].as_str().is_some(),
            "supersedes should set valid_until on target"
        );

        let rels = json["relationships"].as_array().unwrap();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0]["relation_type"], "supersedes");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn unrelate_removes_relationship() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let id1 = store_test_memory_simple(&server, "Memory A", vec![]).await;
        let id2 = store_test_memory_simple(&server, "Memory B", vec![]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        handle_relate(
            MemoryRelateParams {
                source_id: id1.clone(),
                target_id: id2.clone(),
                relation_type: "related".into(),
            },
            &server.write_channel,
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_unrelate(
            MemoryUnrelateParams {
                source_id: id1.clone(),
                target_id: id2.clone(),
                relation_type: "related".into(),
            },
            &server.write_channel,
        )
        .await;
        assert!(is_success(&result));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall = handle_recall(
            MemoryRecallParams { id: id1 },
            &server.read_pool,
            &server.write_channel,
        )
        .await;
        let json = extract_json(&recall);
        assert_eq!(json["relationships"].as_array().unwrap().len(), 0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn category_suggest_creates_and_assigns() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let id = store_test_memory_simple(&server, "Uncategorized memory", vec![]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_category_suggest(
            MemoryCategorySuggestParams {
                memory_id: id.clone(),
                name: "custom-category".into(),
                description: Some("A test category".into()),
                parent_id: None,
            },
            &server.write_channel,
            &server.embedding,
        )
        .await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        assert!(json["category_id"].as_i64().is_some());

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let recall = handle_recall(
            MemoryRecallParams { id },
            &server.read_pool,
            &server.write_channel,
        )
        .await;
        let recall_json = extract_json(&recall);
        assert!(recall_json["category_id"].as_i64().is_some());
        assert_eq!(recall_json["category"]["name"], "custom-category");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn workspaces_lists_with_counts() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "WS memory 1", vec![]).await;
        store_test_memory_simple(&server, "WS memory 2", vec![]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_workspaces(&server.read_pool, None).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let workspaces = json["workspaces"].as_array().unwrap();
        assert!(!workspaces.is_empty());
        let ws = &workspaces[0];
        assert_eq!(ws["path"], "/tmp/test-workspace");
        assert_eq!(ws["memory_count"], 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn tags_lists_with_usage_counts() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "Tagged 1", vec!["alpha".into(), "beta".into()]).await;
        store_test_memory_simple(&server, "Tagged 2", vec!["alpha".into(), "gamma".into()]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_tags(&server.read_pool, None).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let tags = json["tags"].as_array().unwrap();
        assert!(tags.len() >= 3);

        let alpha = tags.iter().find(|t| t["tag"] == "alpha").unwrap();
        assert_eq!(alpha["count"], 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn stats_returns_counts() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "Stats test", vec!["stat-tag".into()]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_stats(&server.read_pool).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        assert!(json["total"].as_i64().unwrap() >= 1);
        assert!(json["by_type"].is_object());
        assert!(json["by_importance"].is_object());
        assert!(json["by_workspace"].is_object());
        assert!(json["by_category"].is_object());
        assert!(json["top_tags"].is_array());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn schema_returns_sql() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_schema(&server.read_pool).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let schema = json["schema"].as_str().unwrap();
        assert!(schema.contains("CREATE TABLE"));
        assert!(schema.contains("memories"));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn sql_select_works() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "SQL test", vec![]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_sql(
            MemorySqlParams {
                query: "SELECT count(*) as cnt FROM memories".into(),
            },
            &server.read_pool,
        )
        .await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        assert_eq!(json["columns"][0], "cnt");
        assert!(json["rows"][0][0].as_i64().unwrap() >= 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn sql_rejects_write_operations() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let write_queries = vec![
            "DELETE FROM memories",
            "INSERT INTO memories (id, title, content, memory_type) VALUES ('x','x','x','fact')",
            "UPDATE memories SET title = 'hacked'",
            "DROP TABLE memories",
            "ALTER TABLE memories ADD COLUMN evil TEXT",
            "CREATE TABLE evil (id INTEGER)",
            "ATTACH DATABASE '/tmp/evil.db' AS evil",
        ];

        for query in write_queries {
            let result = handle_sql(
                MemorySqlParams {
                    query: query.into(),
                },
                &server.read_pool,
            )
            .await;
            assert_eq!(
                result.is_error,
                Some(true),
                "should reject write query: {query}"
            );
        }

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn sql_allows_explain_and_pragma() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_sql(
            MemorySqlParams {
                query: "EXPLAIN SELECT * FROM memories".into(),
            },
            &server.read_pool,
        )
        .await;
        assert!(is_success(&result));

        let result = handle_sql(
            MemorySqlParams {
                query: "PRAGMA table_info(memories)".into(),
            },
            &server.read_pool,
        )
        .await;
        assert!(is_success(&result));

        let result = handle_sql(
            MemorySqlParams {
                query: "PRAGMA journal_mode = WAL".into(),
            },
            &server.read_pool,
        )
        .await;
        assert_eq!(
            result.is_error,
            Some(true),
            "should reject PRAGMA with assignment"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_hybrid_default_ranks_matching_first() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(&server, "alpha", "the quick brown fox", "fact", None, None).await;
        store_test_memory(
            &server,
            "beta",
            "xylophone orchestra performance",
            "fact",
            None,
            None,
        )
        .await;
        store_test_memory(
            &server,
            "gamma",
            "red blue green yellow",
            "fact",
            None,
            None,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "xylophone".into(),
                mode: None,
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let results = json["results"].as_array().unwrap();
        assert!(!results.is_empty());
        assert!(
            results[0]["memory"]["title"]
                .as_str()
                .unwrap()
                .contains("beta")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn sql_allows_readonly_with() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "CTE test", vec![]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_sql(
            MemorySqlParams {
                query: "WITH recent AS (SELECT * FROM memories ORDER BY created_at DESC LIMIT 5) SELECT count(*) as cnt FROM recent".into(),
            },
            &server.read_pool,
        )
        .await;
        assert!(is_success(&result));

        let result = handle_sql(
            MemorySqlParams {
                query: "WITH t AS (SELECT 1) INSERT INTO memories (id) VALUES ('x')".into(),
            },
            &server.read_pool,
        )
        .await;
        assert_eq!(
            result.is_error,
            Some(true),
            "should reject WITH containing INSERT"
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_fts_mode() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "fts target",
            "kubernetes deployment",
            "fact",
            None,
            None,
        )
        .await;
        store_test_memory(
            &server,
            "other",
            "unrelated content here",
            "fact",
            None,
            None,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "kubernetes".into(),
                mode: Some("fts".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            results[0]["memory"]["title"]
                .as_str()
                .unwrap()
                .contains("fts target")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_semantic_mode() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "semantic target",
            "kubernetes container orchestration platform",
            "fact",
            None,
            None,
        )
        .await;
        store_test_memory(
            &server,
            "distant",
            "completely different topic about cooking recipes",
            "fact",
            None,
            None,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "kubernetes container orchestration platform".into(),
                mode: Some("semantic".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let results = json["results"].as_array().unwrap();
        assert!(!results.is_empty());
        assert!(
            results[0]["memory"]["title"]
                .as_str()
                .unwrap()
                .contains("semantic target")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_with_filters() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "decision mem",
            "database connection pooling strategy",
            "decision",
            Some("high"),
            None,
        )
        .await;
        store_test_memory(
            &server,
            "fact mem",
            "database sharding approach",
            "fact",
            None,
            None,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "database".into(),
                mode: Some("fts".into()),
                memory_type: Some("decision".into()),
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: Some("high".into()),
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            results[0]["memory"]["title"]
                .as_str()
                .unwrap()
                .contains("decision")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn context_returns_workspace_memories() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "ws-a mem",
            "content for workspace A",
            "fact",
            Some("high"),
            Some("/tmp/test-ws-a"),
        )
        .await;
        store_test_memory(
            &server,
            "ws-b mem",
            "content for workspace B",
            "fact",
            None,
            Some("/tmp/test-ws-b"),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_context(
            MemoryContextParams {
                workspace_path: "/tmp/test-ws-a".into(),
                summary: false,
            },
            &db_path,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0]["title"].as_str().unwrap().contains("ws-a"));
        assert!(entries[0]["content"].as_str().is_some());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn context_summary_omits_content() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "summary test",
            "this content should be omitted in summary",
            "fact",
            None,
            Some("/tmp/test-ws-summary"),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_context(
            MemoryContextParams {
                workspace_path: "/tmp/test-ws-summary".into(),
                summary: true,
            },
            &db_path,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0]["content"].is_null());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn context_high_importance_first() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "low importance",
            "low importance content",
            "fact",
            Some("low"),
            Some("/tmp/test-ws-order"),
        )
        .await;
        store_test_memory(
            &server,
            "high importance",
            "high importance content",
            "decision",
            Some("high"),
            Some("/tmp/test-ws-order"),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_context(
            MemoryContextParams {
                workspace_path: "/tmp/test-ws-order".into(),
                summary: false,
            },
            &db_path,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let entries = json["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(
            entries[0]["title"]
                .as_str()
                .unwrap()
                .contains("high importance")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn workspaces_filter_by_source_returns_matching() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "ws-a mem",
            "content a",
            "fact",
            None,
            Some("/tmp/test-ws-a"),
        )
        .await;
        store_test_memory(
            &server,
            "ws-b mem",
            "content b",
            "fact",
            None,
            Some("/tmp/other-workspace"),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_workspaces(&server.read_pool, Some("test-ws".into())).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let workspaces = json["workspaces"].as_array().unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0]["path"], "/tmp/test-ws-a");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn workspaces_filter_none_returns_all() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "ws-a mem",
            "content a",
            "fact",
            None,
            Some("/tmp/test-ws-a"),
        )
        .await;
        store_test_memory(
            &server,
            "ws-b mem",
            "content b",
            "fact",
            None,
            Some("/tmp/other-workspace"),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_workspaces(&server.read_pool, None).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let workspaces = json["workspaces"].as_array().unwrap();
        assert_eq!(workspaces.len(), 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn tags_filter_by_prefix_returns_matching() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "T1", vec!["bug-123".into(), "bug-456".into()]).await;
        store_test_memory_simple(&server, "T2", vec!["feature-x".into()]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_tags(&server.read_pool, Some("bug".into())).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let tags = json["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert!(
            tags.iter()
                .all(|t| t["tag"].as_str().unwrap().starts_with("bug"))
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn tags_filter_none_returns_all() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory_simple(&server, "T1", vec!["bug-123".into(), "feature-x".into()]).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_tags(&server.read_pool, None).await;
        assert!(is_success(&result));
        let json = extract_json(&result);
        let tags = json["tags"].as_array().unwrap();
        assert!(tags.len() >= 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn context_unknown_workspace_returns_empty() {
        let db_path = test_db_path();
        let _server = test_server(&db_path);

        let result = handle_context(
            MemoryContextParams {
                workspace_path: "/tmp/nonexistent-workspace".into(),
                summary: false,
            },
            &db_path,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let entries = json["entries"].as_array().unwrap();
        assert!(entries.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_empty_query_fts_returns_error() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_search(
            MemorySearchParams {
                query: "".into(),
                mode: Some("fts".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert_eq!(result.is_error, Some(true));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_invalid_mode_returns_error() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_search(
            MemorySearchParams {
                query: "test".into(),
                mode: Some("invalid".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert_eq!(result.is_error, Some(true));

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_filters_by_tags() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        handle_store(
            MemoryStoreParams {
                title: "tagged memory".into(),
                content: "kubernetes deployment strategy".into(),
                memory_type: "fact".into(),
                tags: vec!["k8s".into(), "infra".into()],
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
        handle_store(
            MemoryStoreParams {
                title: "untagged memory".into(),
                content: "kubernetes pod scheduling".into(),
                memory_type: "fact".into(),
                tags: vec!["other".into()],
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

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "kubernetes".into(),
                mode: Some("fts".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: Some(vec!["k8s".into()]),
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            results[0]["memory"]["title"]
                .as_str()
                .unwrap()
                .contains("tagged")
        );

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_respects_limit() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        for i in 0..5 {
            store_test_memory(
                &server,
                &format!("memory {i}"),
                &format!("searchable content number {i}"),
                "fact",
                None,
                None,
            )
            .await;
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "searchable".into(),
                mode: Some("fts".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: Some(2),
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        let results = json["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(json["count"], 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn search_creates_access_log_entries() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "access log test",
            "content for access log verification",
            "fact",
            None,
            None,
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_search(
            MemorySearchParams {
                query: "access".into(),
                mode: Some("fts".into()),
                memory_type: None,
                category_id: None,
                workspace_id: None,
                trace_id: None,
                session_id: None,
                importance: None,
                confidence: None,
                tags: None,
                archived: None,
                since: None,
                until: None,
                valid_only: None,
                limit: None,
            },
            &db_path,
            &server.embedding,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert!(!json["results"].as_array().unwrap().is_empty());

        let db = MemoryDb::open(&db_path, None, 384).unwrap();
        let count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM access_log WHERE access_type = 'search'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "search should create access log entries");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn context_creates_access_log_entries() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        store_test_memory(
            &server,
            "context access test",
            "content for context access log",
            "fact",
            None,
            Some("/tmp/test-ws-access-log"),
        )
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_context(
            MemoryContextParams {
                workspace_path: "/tmp/test-ws-access-log".into(),
                summary: false,
            },
            &db_path,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert!(!json["entries"].as_array().unwrap().is_empty());

        let db = MemoryDb::open(&db_path, None, 384).unwrap();
        let count: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM access_log WHERE access_type = 'context'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "context should create access log entries");

        let _ = std::fs::remove_file(&db_path);
    }

    fn test_otlp_db_path() -> String {
        let path = format!(
            "/tmp/nous-test-otlp-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        );
        OtlpDb::open(&path, None).unwrap();
        path
    }

    fn seed_otlp(
        path: &str,
        spans: &[nous_otlp::decode::Span],
        logs: &[nous_otlp::decode::LogEvent],
    ) {
        let db = OtlpDb::open(path, None).unwrap();
        db.store_spans(spans).unwrap();
        db.store_logs(logs).unwrap();
    }

    #[tokio::test]
    async fn otlp_trace_context_returns_correlated_data() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let otlp_path = test_otlp_db_path();

        let store_result = handle_store(
            MemoryStoreParams {
                title: "Traced memory".into(),
                content: "Memory correlated with a trace".into(),
                memory_type: "observation".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: Some("sess-001".into()),
                trace_id: Some("trace-abc".into()),
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

        seed_otlp(
            &otlp_path,
            &[nous_otlp::decode::Span {
                trace_id: "trace-abc".into(),
                span_id: "span-001".into(),
                parent_span_id: None,
                name: "HTTP GET /api".into(),
                kind: 1,
                start_time: 1000,
                end_time: 2000,
                status_code: 0,
                status_message: None,
                resource_attrs: "{}".into(),
                span_attrs: "{}".into(),
                events_json: "[]".into(),
            }],
            &[nous_otlp::decode::LogEvent {
                timestamp: 1500,
                severity: "INFO".into(),
                body: "Request processed".into(),
                resource_attrs: "{}".into(),
                scope_attrs: "{}".into(),
                log_attrs: "{}".into(),
                session_id: Some("sess-001".into()),
                trace_id: Some("trace-abc".into()),
                span_id: None,
            }],
        );

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let result = handle_otlp_trace_context(
            OtlpTraceContextParams {
                trace_id: "trace-abc".into(),
                session_id: Some("sess-001".into()),
            },
            &otlp_path,
            &server.read_pool,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert!(!json["memories"].as_array().unwrap().is_empty());
        assert_eq!(json["spans"].as_array().unwrap().len(), 1);
        assert_eq!(json["spans"][0]["name"], "HTTP GET /api");
        assert_eq!(json["logs"].as_array().unwrap().len(), 1);
        assert_eq!(json["logs"][0]["body"], "Request processed");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&otlp_path);
    }

    #[tokio::test]
    async fn otlp_trace_context_without_session_id_returns_empty_logs() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let otlp_path = test_otlp_db_path();

        seed_otlp(
            &otlp_path,
            &[nous_otlp::decode::Span {
                trace_id: "trace-xyz".into(),
                span_id: "span-002".into(),
                parent_span_id: None,
                name: "DB query".into(),
                kind: 2,
                start_time: 3000,
                end_time: 4000,
                status_code: 0,
                status_message: None,
                resource_attrs: "{}".into(),
                span_attrs: "{}".into(),
                events_json: "[]".into(),
            }],
            &[],
        );

        let result = handle_otlp_trace_context(
            OtlpTraceContextParams {
                trace_id: "trace-xyz".into(),
                session_id: None,
            },
            &otlp_path,
            &server.read_pool,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert_eq!(json["spans"].as_array().unwrap().len(), 1);
        assert!(json["logs"].as_array().unwrap().is_empty());

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&otlp_path);
    }

    #[tokio::test]
    async fn otlp_trace_context_no_otlp_db_returns_error() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_otlp_trace_context(
            OtlpTraceContextParams {
                trace_id: "trace-abc".into(),
                session_id: None,
            },
            "",
            &server.read_pool,
        )
        .await;

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().unwrap().text.as_str();
        assert_eq!(text, "OTLP database not configured");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn otlp_memory_context_returns_correlated_data() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let otlp_path = test_otlp_db_path();

        let store_result = handle_store(
            MemoryStoreParams {
                title: "Correlated memory".into(),
                content: "Memory with trace and session".into(),
                memory_type: "decision".into(),
                tags: vec![],
                source: None,
                importance: None,
                confidence: None,
                workspace_path: None,
                session_id: Some("sess-mem".into()),
                trace_id: Some("trace-mem".into()),
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
        let memory_id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        seed_otlp(
            &otlp_path,
            &[nous_otlp::decode::Span {
                trace_id: "trace-mem".into(),
                span_id: "span-mem".into(),
                parent_span_id: None,
                name: "memory_store".into(),
                kind: 1,
                start_time: 5000,
                end_time: 6000,
                status_code: 0,
                status_message: None,
                resource_attrs: "{}".into(),
                span_attrs: "{}".into(),
                events_json: "[]".into(),
            }],
            &[nous_otlp::decode::LogEvent {
                timestamp: 5500,
                severity: "DEBUG".into(),
                body: "Storing memory".into(),
                resource_attrs: "{}".into(),
                scope_attrs: "{}".into(),
                log_attrs: "{}".into(),
                session_id: Some("sess-mem".into()),
                trace_id: Some("trace-mem".into()),
                span_id: None,
            }],
        );

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_otlp_memory_context(
            OtlpMemoryContextParams {
                memory_id: memory_id.clone(),
            },
            &otlp_path,
            &server.read_pool,
        )
        .await;

        assert!(is_success(&result));
        let json = extract_json(&result);
        assert_eq!(json["memory"]["id"], memory_id);
        assert_eq!(json["memory"]["title"], "Correlated memory");
        assert_eq!(json["spans"].as_array().unwrap().len(), 1);
        assert_eq!(json["spans"][0]["name"], "memory_store");
        assert_eq!(json["logs"].as_array().unwrap().len(), 1);
        assert_eq!(json["logs"][0]["body"], "Storing memory");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&otlp_path);
    }

    #[tokio::test]
    async fn otlp_memory_context_no_correlation_ids_returns_error() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let otlp_path = test_otlp_db_path();

        let store_result = handle_store(
            MemoryStoreParams {
                title: "No correlation".into(),
                content: "Memory without trace or session".into(),
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
        let memory_id = extract_json(&store_result)["id"]
            .as_str()
            .unwrap()
            .to_string();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let result = handle_otlp_memory_context(
            OtlpMemoryContextParams { memory_id },
            &otlp_path,
            &server.read_pool,
        )
        .await;

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().unwrap().text.as_str();
        assert_eq!(
            text,
            "memory has no trace_id or session_id for OTLP correlation"
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&otlp_path);
    }

    #[tokio::test]
    async fn otlp_memory_context_no_otlp_db_returns_error() {
        let db_path = test_db_path();
        let server = test_server(&db_path);

        let result = handle_otlp_memory_context(
            OtlpMemoryContextParams {
                memory_id: "some-id".into(),
            },
            "",
            &server.read_pool,
        )
        .await;

        assert_eq!(result.is_error, Some(true));
        let text = result.content[0].as_text().unwrap().text.as_str();
        assert_eq!(text, "OTLP database not configured");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn otlp_memory_context_nonexistent_memory_returns_error() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let otlp_path = test_otlp_db_path();

        let result = handle_otlp_memory_context(
            OtlpMemoryContextParams {
                memory_id: "nonexistent-id".into(),
            },
            &otlp_path,
            &server.read_pool,
        )
        .await;

        assert_eq!(result.is_error, Some(true));

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&otlp_path);
    }
}
