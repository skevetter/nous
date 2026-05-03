use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::notifications::NotificationRegistry;
use nous_core::schedules::SystemClock;
use nous_daemon::embedding::{build_embedder, resolve_embedding_config};
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use nous_daemon::vector_store::resolve_vector_store_config;
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
        .run_migrations()
        .await
        .expect("failed to run migrations");

    let embedding_config = resolve_embedding_config(None, None, None);
    let vector_store_config = resolve_vector_store_config(None, None, None);
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
            tracing::warn!("╔══════════════════════════════════════════════════════╗");
            tracing::warn!("║  LLM CLIENT NOT CONFIGURED                          ║");
            tracing::warn!("║                                                      ║");
            tracing::warn!("║  Agent invocation endpoints will return 503.         ║");
            tracing::warn!("║  Set AWS_PROFILE or AWS credentials to enable LLM.   ║");
            tracing::warn!("║  Reason: {:<43} ║", format!("{e}"));
            tracing::warn!("╚══════════════════════════════════════════════════════╝");
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

    let registry = Arc::new(NotificationRegistry::new());
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
        embedding_config,
        vector_store_config,
        schedule_notify: Arc::new(Notify::new()),
        shutdown: shutdown.clone(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client,
        default_model,
        tool_services,
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    };

    let scheduler_handle = Scheduler::spawn(
        state.clone(),
        SchedulerConfig::default(),
        Arc::new(SystemClock),
        shutdown.clone(),
    );

    let api_key = config.resolve_api_key();
    if api_key.is_some() {
        tracing::info!("API key authentication enabled");
    } else {
        tracing::warn!("no API key configured — all endpoints are publicly accessible");
    }

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("failed to bind TCP listener");
    tracing::info!("listening on {}", listener.local_addr().expect("listener has no local address"));
    axum::serve(
        listener,
        nous_daemon::app_with_options(state, Some(&config.rate_limit), api_key.as_deref()),
    )
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        .expect("axum server exited with error");

    scheduler_handle.await.expect("scheduler task panicked");
}
