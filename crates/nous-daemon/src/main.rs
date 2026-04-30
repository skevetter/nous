use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory::OnnxEmbeddingModel;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::state::AppState;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Config::load().expect("failed to load config");
    config.ensure_dirs().expect("failed to create directories");

    let pools = DbPools::connect(&config.data_dir)
        .await
        .expect("failed to connect to database");
    pools
        .run_migrations(&config.search.tokenizer)
        .await
        .expect("failed to run migrations");

    let embedder: Arc<dyn nous_core::memory::Embedder> =
        Arc::new(OnnxEmbeddingModel::load(None).expect("failed to load ONNX embedding model"));

    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder,
    };

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, nous_daemon::app(state))
        .await
        .unwrap();
}
