use std::sync::Arc;

use nous_core::schedules::SystemClock;
use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory::OnnxEmbeddingModel;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub async fn run() {
    if let Err(e) = execute().await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    config.ensure_dirs()?;

    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations(&config.search.tokenizer).await?;

    let embedder: Option<Arc<dyn nous_core::memory::Embedder>> =
        match OnnxEmbeddingModel::load(None) {
            Ok(model) => Some(Arc::new(model)),
            Err(e) => {
                tracing::warn!("embedding model not available, vector/hybrid search disabled: {e}");
                None
            }
        };

    let shutdown = CancellationToken::new();

    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder,
        schedule_notify: Arc::new(Notify::new()),
        shutdown: shutdown.clone(),
    };

    let _scheduler_handle = Scheduler::spawn(
        state.clone(),
        SchedulerConfig::default(),
        Arc::new(SystemClock),
        shutdown.clone(),
    );

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, nous_daemon::app(state))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;

    pools.close().await;
    Ok(())
}
