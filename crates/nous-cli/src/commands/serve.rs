use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::state::AppState;
use tokio::net::TcpListener;

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
    pools.run_migrations().await?;

    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
    };

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, nous_daemon::app(state)).await?;

    pools.close().await;
    Ok(())
}
