use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::notifications::NotificationRegistry;
use nous_core::schedules::SystemClock;
use nous_daemon::embedding::{build_embedder, resolve_embedding_config, validate_embedding_dimensions};
use nous_daemon::llm_client::{LlmClient, DEFAULT_MODEL};
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use nous_daemon::vector_store::resolve_vector_store_config;
use rig::client::ProviderClient;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

fn log_llm_not_configured(e: &impl std::fmt::Display) {
    let msg = format!(
        "\n╔══════════════════════════════════════════════════════╗\
         \n║  LLM CLIENT NOT CONFIGURED                          ║\
         \n║                                                      ║\
         \n║  Agent invocation endpoints will return 503.         ║\
         \n║  Set AWS_PROFILE or AWS credentials to enable LLM.   ║\
         \n║  Reason: {:<43} ║\
         \n╚══════════════════════════════════════════════════════╝",
        format!("{e}")
    );
    tracing::warn!("{msg}");
}

fn build_llm_client() -> (Option<Arc<LlmClient>>, String) {
    match LlmClient::from_env() {
        Ok(client) => {
            tracing::info!("LLM client configured for Bedrock");
            let model =
                std::env::var("NOUS_LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
            (Some(Arc::new(client)), model)
        }
        Err(e) => {
            log_llm_not_configured(&e);
            (None, DEFAULT_MODEL.to_string())
        }
    }
}

fn spawn_shutdown_listener(shutdown: CancellationToken) {
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

struct AppStateConfig {
    embedder: Option<Arc<dyn nous_core::memory::Embedder>>,
    embedding_config: nous_core::memory::EmbeddingConfig,
    vector_store_config: nous_core::memory::VectorStoreConfig,
    llm_client: Option<Arc<LlmClient>>,
    default_model: String,
    shutdown: CancellationToken,
}

fn build_app_state(pools: &DbPools, cfg: AppStateConfig) -> AppState {
    let registry = Arc::new(NotificationRegistry::new());
    let tool_services = AppState::build_tool_services(
        pools.fts.clone(),
        pools.vec.clone(),
        cfg.embedder.clone(),
        registry.clone(),
    );
    AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry,
        embedder: cfg.embedder,
        embedding_config: cfg.embedding_config,
        vector_store_config: cfg.vector_store_config,
        schedule_notify: Arc::new(Notify::new()),
        shutdown: cfg.shutdown,
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client: cfg.llm_client,
        default_model: cfg.default_model,
        tool_services,
        #[cfg(feature = "sandbox")]
        sandbox_manager: None,
    }
}

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
    validate_embedding_dimensions(&embedding_config, &pools.vec)
        .expect("embedding dimension mismatch — see error above");
    let vector_store_config = resolve_vector_store_config(None);
    let embedder: Option<Arc<dyn nous_core::memory::Embedder>> =
        match build_embedder(&embedding_config) {
            Ok(embedder) => Some(embedder),
            Err(e) => {
                tracing::warn!("embedding model not available, vector/hybrid search disabled: {e}");
                None
            }
        };

    let (llm_client, default_model) = build_llm_client();

    let shutdown = CancellationToken::new();
    spawn_shutdown_listener(shutdown.clone());

    let state = build_app_state(
        &pools,
        AppStateConfig {
            embedder,
            embedding_config,
            vector_store_config,
            llm_client,
            default_model,
            shutdown: shutdown.clone(),
        },
    );

    let api_key = config.resolve_api_key();
    if api_key.is_some() {
        tracing::info!("API key authentication enabled");
    } else {
        tracing::warn!("no API key configured — all endpoints are publicly accessible");
    }

    let scheduler_handle = Scheduler::spawn(
        state.clone(),
        SchedulerConfig {
            max_concurrent: config.scheduler.max_concurrent,
            allow_shell: config.scheduler.allow_shell,
            default_timeout_secs: config.scheduler.default_timeout_secs,
            auth_configured: api_key.is_some(),
        },
        Arc::new(SystemClock),
        shutdown.clone(),
    );

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr)
        .await
        .expect("failed to bind TCP listener");
    tracing::info!(
        "listening on {}",
        listener
            .local_addr()
            .expect("listener has no local address")
    );

    let process_registry = state.process_registry.clone();
    let shutdown_state = state.clone();

    axum::serve(
        listener,
        nous_daemon::app_with_options(state, Some(&config.rate_limit), api_key.as_deref())
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move { shutdown.cancelled().await })
    .await
    .expect("axum server exited with error");

    tracing::info!("HTTP server stopped, shutting down child processes");
    process_registry.shutdown(&shutdown_state).await;

    scheduler_handle.await.expect("scheduler task panicked");
}
