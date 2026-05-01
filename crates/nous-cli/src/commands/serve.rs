use std::sync::Arc;

use nous_core::schedules::SystemClock;
use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory::OnnxEmbeddingModel;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub async fn run(port: Option<u16>) {
    if let Err(e) = execute(port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(port: Option<u16>) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
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
    let process_registry = Arc::new(ProcessRegistry::new());

    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder,
        schedule_notify: Arc::new(Notify::new()),
        shutdown: shutdown.clone(),
        process_registry: process_registry.clone(),
        llm_client: None,
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

    // 1. HTTP server stops (graceful_shutdown)
    axum::serve(listener, nous_daemon::app(state.clone()))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;

    // 2. ProcessRegistry.shutdown() — stops all agent processes
    process_registry.shutdown(&state).await;

    // 3. Scheduler stops via CancellationToken (already cancelled above)
    // 4. DB pools close
    pools.close().await;
    Ok(())
}
