use nous_core::db::DbPools;
use nous_core::memory::{EmbeddingConfig, MockEmbedder, VectorStoreConfig};
use nous_core::notifications::NotificationRegistry;
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::state::AppState;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub async fn setup_test_db() -> (DbPools, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools, tmp)
}

pub async fn test_state() -> (AppState, TempDir) {
    let (pools, tmp) = setup_test_db().await;
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder: Some(Arc::new(MockEmbedder::new())),
        embedding_config: EmbeddingConfig::default(),
        vector_store_config: VectorStoreConfig::default(),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client: None,
        default_model: "test-model".to_string(),
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };
    (state, tmp)
}

pub async fn test_state_no_embedder() -> (AppState, TempDir) {
    let (pools, tmp) = setup_test_db().await;
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder: None,
        embedding_config: EmbeddingConfig::default(),
        vector_store_config: VectorStoreConfig::default(),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client: None,
        default_model: "test-model".to_string(),
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };
    (state, tmp)
}
