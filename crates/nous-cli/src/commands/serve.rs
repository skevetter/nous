use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory::OnnxEmbeddingModel;
use nous_core::notifications::NotificationRegistry;
use nous_core::schedules::SystemClock;
use nous_daemon::llm_client::{build_client, LlmClient, LlmConfig};
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub struct ServeParams {
    pub model: Option<String>,
    pub region: Option<String>,
    pub profile: Option<String>,
    pub port: Option<u16>,
    pub daemon: bool,
    pub foreground_daemon: bool,
}

fn pid_file_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("nous")
        .join("nous.pid")
}

fn write_pid_file(pid: u32) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&path)?;
    write!(file, "{pid}")
}

fn remove_pid_file() {
    let path = pid_file_path();
    let _ = std::fs::remove_file(path);
}

pub async fn run(params: ServeParams) {
    if params.daemon && !params.foreground_daemon {
        if let Err(e) = daemonize(
            params.model.as_deref(),
            params.region.as_deref(),
            params.profile.as_deref(),
            params.port,
        ) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
        return;
    }
    let is_daemon = params.foreground_daemon;
    if let Err(e) = execute(ExecuteParams {
        model: params.model,
        region: params.region,
        profile: params.profile,
        port: params.port,
        is_daemon,
    })
    .await
    {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

// Daemon process intentionally outlives parent
#[allow(clippy::zombie_processes)]
fn daemonize(
    model: Option<&str>,
    region: Option<&str>,
    profile: Option<&str>,
    port: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;

    let mut args = vec!["serve".to_string(), "--foreground-daemon".to_string()];

    if let Some(m) = model {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
    if let Some(r) = region {
        args.push("--region".to_string());
        args.push(r.to_string());
    }
    if let Some(p) = profile {
        args.push("--profile".to_string());
        args.push(p.to_string());
    }
    if let Some(p) = port {
        args.insert(0, p.to_string());
        args.insert(0, "--port".to_string());
    }

    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
        .join("nous");
    std::fs::create_dir_all(&log_dir)?;

    let log_file = std::fs::File::create(log_dir.join("nous-daemon.log"))?;
    let stderr_file = log_file.try_clone()?;

    let mut child = std::process::Command::new(exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(stderr_file))
        .spawn()?;

    std::thread::sleep(std::time::Duration::from_secs(2));

    if let Some(status) = child.try_wait()? {
        let log_path = log_dir.join("nous-daemon.log");
        eprintln!("daemon exited immediately with {status}");
        eprintln!("check log: {}", log_path.display());
        std::process::exit(1);
    } else {
        let pid = child.id();
        eprintln!("nous daemon started (pid: {pid})");
        eprintln!("PID file: {}", pid_file_path().display());
        eprintln!("Log file: {}", log_dir.join("nous-daemon.log").display());
    }

    Ok(())
}

fn has_aws_credentials() -> bool {
    std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        || std::env::var("AWS_PROFILE").is_ok()
        || std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok()
}

async fn build_llm_client_for_config(
    llm_config: &LlmConfig,
) -> Arc<LlmClient> {
    let client = build_client(llm_config).await;
    tracing::info!(
        region = %llm_config.region,
        model = %llm_config.model,
        "LLM client configured for Bedrock"
    );
    Arc::new(client)
}

async fn resolve_llm(
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
) -> (Option<Arc<LlmClient>>, String) {
    let llm_config = LlmConfig::resolve(model, region, profile);

    if has_aws_credentials() {
        let client = build_llm_client_for_config(&llm_config).await;
        (Some(client), llm_config.model)
    } else {
        tracing::warn!("LLM client not available (no AWS credentials found in environment)");
        (None, llm_config.model)
    }
}

fn spawn_signal_handler(shutdown: CancellationToken, initial_port: u16) {
    tokio::spawn(async move {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to register SIGTERM handler");
        let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .expect("failed to register SIGHUP handler");
        let mut current_port = initial_port;
        loop {
            tokio::select! {
                _ = sigterm.recv() => {
                    tracing::info!("shutdown signal received");
                    shutdown.cancel();
                    break;
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("ctrl-c received");
                    shutdown.cancel();
                    break;
                }
                _ = sighup.recv() => {
                    handle_sighup(&shutdown, &mut current_port);
                }
            }
        }
    });
}

fn handle_sighup(shutdown: &CancellationToken, current_port: &mut u16) {
    // Currently validates config and detects port changes only.
    // No config values are hot-reloaded at runtime — restart required for all changes.
    tracing::info!("SIGHUP received, reloading config");
    reload_config(shutdown, current_port);
}

fn log_config_reload_error(e: &nous_core::error::NousError) {
    tracing::error!("failed to reload config: {e}");
}

fn apply_reloaded_config(new_config: nous_core::config::Config, current_port: &mut u16) {
    check_port_change(new_config.port, current_port);
    tracing::info!("config reloaded successfully");
}

fn reload_config(shutdown: &CancellationToken, current_port: &mut u16) {
    match Config::load() {
        Ok(new_config) => apply_reloaded_config(new_config, current_port),
        Err(e) => log_config_reload_error(&e),
    }
    // suppress unused warning — shutdown is used in the outer select! macro
    let _ = shutdown;
}

fn check_port_change(new_port: u16, current_port: &mut u16) {
    if new_port != *current_port {
        tracing::warn!(
            "config reloaded: port changed from {} to {}, restart required",
            current_port,
            new_port
        );
    }
    *current_port = new_port;
}

struct ExecuteParams {
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
    is_daemon: bool,
}

struct BuildStateParams {
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    shutdown: CancellationToken,
    process_registry: Arc<ProcessRegistry>,
}

async fn build_state(
    pools: &DbPools,
    p: BuildStateParams,
) -> Result<AppState, Box<dyn std::error::Error>> {
    let embedder: Option<Arc<dyn nous_core::memory::Embedder>> =
        match OnnxEmbeddingModel::load(None) {
            Ok(m) => Some(Arc::new(m)),
            Err(e) => {
                tracing::warn!("embedding model not available, vector/hybrid search disabled: {e}");
                None
            }
        };

    let (llm_client, default_model) = resolve_llm(p.model, p.region, p.profile).await;


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
        embedding_config: nous_core::memory::EmbeddingConfig::default(),
        vector_store_config: nous_core::memory::VectorStoreConfig::default(),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: p.shutdown,
        process_registry: p.process_registry,
        llm_client,
        default_model,
        tool_services,
        #[cfg(feature = "sandbox")]
        sandbox_manager: Some(Arc::new(tokio::sync::Mutex::new(
            nous_daemon::sandbox::SandboxManager::new(),
        ))),
    };

    Ok(state)
}

async fn setup_config_and_db(
    port: Option<u16>,
) -> Result<(nous_core::config::Config, DbPools), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    Ok((config, pools))
}

struct RunServerParams {
    state: AppState,
    shutdown: CancellationToken,
    listener: TcpListener,
    process_registry: Arc<ProcessRegistry>,
    pools: DbPools,
    is_daemon: bool,
}

async fn run_server(p: RunServerParams) -> Result<(), Box<dyn std::error::Error>> {
    axum::serve(p.listener, nous_daemon::app(p.state.clone()))
        .with_graceful_shutdown(async move { p.shutdown.cancelled().await })
        .await?;

    p.process_registry.shutdown(&p.state).await;
    p.pools.close().await;

    if p.is_daemon {
        remove_pid_file();
    }

    Ok(())
}

async fn execute(params: ExecuteParams) -> Result<(), Box<dyn std::error::Error>> {
    let ExecuteParams { model, region, profile, port, is_daemon } = params;

    if is_daemon {
        write_pid_file(std::process::id())?;
    }

    setup_and_run(SetupParams { model, region, profile, port, is_daemon }).await
}

struct SetupParams {
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
    is_daemon: bool,
}

async fn setup_and_run(p: SetupParams) -> Result<(), Box<dyn std::error::Error>> {
    let (config, pools) = setup_config_and_db(p.port).await?;
    let shutdown = CancellationToken::new();
    let process_registry = Arc::new(ProcessRegistry::new());
    let state = build_state(
        &pools,
        BuildStateParams { model: p.model, region: p.region, profile: p.profile, shutdown: shutdown.clone(), process_registry: process_registry.clone() },
    )
    .await?;

    start_background_services(BackgroundServicesParams { pools: &pools, process_registry: &process_registry, state: &state, shutdown: &shutdown, port: config.port }).await;

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    run_server(RunServerParams { state, shutdown, listener, process_registry, pools, is_daemon: p.is_daemon }).await
}

struct BackgroundServicesParams<'a> {
    pools: &'a DbPools,
    process_registry: &'a Arc<ProcessRegistry>,
    state: &'a AppState,
    shutdown: &'a CancellationToken,
    port: u16,
}

async fn start_background_services(p: BackgroundServicesParams<'_>) {
    recover_sandboxes(p.pools, p.process_registry, p.state).await;
    Scheduler::spawn(p.state.clone(), SchedulerConfig::default(), Arc::new(SystemClock), p.shutdown.clone());
    spawn_signal_handler(p.shutdown.clone(), p.port);
}

async fn recover_sandboxes(
    _pools: &DbPools,
    _process_registry: &Arc<ProcessRegistry>,
    _state: &AppState,
) {
    // Sandbox recovery is best-effort: a failure here means some sandboxes won't be
    // reconnected, but the daemon should still start and serve new requests.
    #[cfg(feature = "sandbox")]
    {
        if let Some(ref sandbox_mgr) = _state.sandbox_manager {
            if let Err(e) = _process_registry
                .recover_sandboxes(&_pools.fts, sandbox_mgr)
                .await
            {
                tracing::error!("sandbox recovery failed: {e}");
            }
        }
    }
}
