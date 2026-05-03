use std::sync::Arc;

use nous_core::db::VecPool;
use nous_core::memory::{Embedder, EmbeddingConfig, VectorStoreConfig};
use nous_core::notifications::NotificationRegistry;
use sqlx::SqlitePool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::llm_client::LlmClient;
use crate::process_manager::ProcessRegistry;
#[cfg(feature = "sandbox")]
use crate::sandbox::SandboxManager;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub vec_pool: VecPool,
    pub registry: Arc<NotificationRegistry>,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub embedding_config: EmbeddingConfig,
    pub vector_store_config: VectorStoreConfig,
    pub schedule_notify: Arc<Notify>,
    pub shutdown: CancellationToken,
    pub process_registry: Arc<ProcessRegistry>,
    pub llm_client: Option<Arc<LlmClient>>,
    pub default_model: String,
    #[cfg(feature = "sandbox")]
    pub sandbox_manager: Option<Arc<tokio::sync::Mutex<SandboxManager>>>,
}

impl AppState {
    #[cfg(feature = "sandbox")]
    pub fn sandbox_manager(&self) -> Option<&Arc<tokio::sync::Mutex<SandboxManager>>> {
        self.sandbox_manager.as_ref()
    }
}
