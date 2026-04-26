use std::sync::Arc;

use nous_core::channel::{ReadPool, WriteChannel};
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
    pub config: Config,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl NousServer {
    pub fn new(config: Config, embedding: Box<dyn EmbeddingBackend>) -> nous_shared::Result<Self> {
        let db = MemoryDb::open(":memory:", None)?;
        let classifier = CategoryClassifier::new(&db, embedding.as_ref())?;
        let (write_channel, write_handle) = WriteChannel::new(db);
        let read_pool = ReadPool::new(":memory:", None, 4)?;
        let embedding = Arc::from(embedding);

        Ok(Self {
            write_channel,
            write_handle,
            read_pool,
            embedding,
            classifier,
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
    async fn memory_store(&self, _params: Parameters<MemoryStoreParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_recall", description = "Recall a memory by ID")]
    async fn memory_recall(&self, _params: Parameters<MemoryRecallParams>) -> CallToolResult {
        not_implemented()
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
    async fn memory_forget(&self, _params: Parameters<MemoryForgetParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_unarchive", description = "Restore an archived memory")]
    async fn memory_unarchive(&self, _params: Parameters<MemoryUnarchiveParams>) -> CallToolResult {
        not_implemented()
    }

    #[tool(name = "memory_update", description = "Update fields on a memory")]
    async fn memory_update(&self, _params: Parameters<MemoryUpdateParams>) -> CallToolResult {
        not_implemented()
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
