use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::notifications::NotificationRegistry;
use nous_core::schedules::SystemClock;
use nous_daemon::embedding::{build_embedder, resolve_embedding_config};
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

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

    let embedding_config = resolve_embedding_config(None, None, None);
    let embedder: Option<Arc<dyn nous_core::memory::Embedder>> =
        match build_embedder(&embedding_config) {
            Ok(embedder) => Some(embedder),
            Err(e) => {
                tracing::warn!("embedding model not available, vector/hybrid search disabled: {e}");
                None
            }
        };

    use nous_daemon::llm_client::{LlmClient, DEFAULT_MODEL};
    use rig::client::ProviderClient;

    let (llm_client, default_model) = match LlmClient::from_env() {
        Ok(client) => {
            tracing::info!("LLM client configured for Bedrock");
            let model =
                std::env::var("NOUS_LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
            (Some(Arc::new(client)), model)
        }
        Err(e) => {
            tracing::info!("LLM client not available: {e}");
            (None, DEFAULT_MODEL.to_string())
        }
    };

    let shutdown = CancellationToken::new();

    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to register SIGTERM handler");
            tokio::select! {
                _ = sigterm.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            tracing::info!("shutdown signal received");
            shutdown.cancel();
        });
    }

    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder,
        embedding_config,
        schedule_notify: Arc::new(Notify::new()),
        shutdown: shutdown.clone(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client,
        default_model,
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };

    let scheduler_handle = Scheduler::spawn(
        state.clone(),
        SchedulerConfig::default(),
        Arc::new(SystemClock),
        shutdown.clone(),
    );

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, nous_daemon::app(state))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .unwrap();

    scheduler_handle.await.unwrap();
}
