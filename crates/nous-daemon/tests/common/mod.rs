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
    use sea_orm::ConnectionTrait;
    for agent_id in ["agent-1", "agent-2", "test-agent"] {
        pools.fts.execute_unprepared(
            &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
        ).await.unwrap();
    }
    (pools, tmp)
}

pub async fn test_state() -> (AppState, TempDir) {
    let (pools, tmp) = setup_test_db().await;
    let registry = Arc::new(NotificationRegistry::new());
    let embedder: Option<Arc<dyn nous_core::memory::Embedder>> =
        Some(Arc::new(MockEmbedder::new()));
    let tool_services = AppState::build_tool_services(
        pools.fts.clone(),
        pools.vec.clone(),
        embedder.clone(),
        registry.clone(),
    );
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry,
        embedder,
        embedding_config: EmbeddingConfig::default(),
        vector_store_config: VectorStoreConfig::default(),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client: None,
        default_model: "test-model".to_string(),
        tool_services,
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };
    (state, tmp)
}

pub async fn test_state_no_embedder() -> (AppState, TempDir) {
    let (pools, tmp) = setup_test_db().await;
    let registry = Arc::new(NotificationRegistry::new());
    let tool_services = AppState::build_tool_services(
        pools.fts.clone(),
        pools.vec.clone(),
        None,
        registry.clone(),
    );
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry,
        embedder: None,
        embedding_config: EmbeddingConfig::default(),
        vector_store_config: VectorStoreConfig::default(),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client: None,
        default_model: "test-model".to_string(),
        tool_services,
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };
    (state, tmp)
}
