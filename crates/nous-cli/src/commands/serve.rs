use std::sync::Arc;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory::OnnxEmbeddingModel;
use nous_core::notifications::NotificationRegistry;
use nous_core::schedules::SystemClock;
use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

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

pub async fn run(
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
    daemon: bool,
    foreground_daemon: bool,
) {
    if daemon && !foreground_daemon {
        if let Err(e) = daemonize(model, region, profile, port) {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = execute(model, region, profile, port, foreground_daemon).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

#[allow(clippy::zombie_processes)]
fn daemonize(
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;

    let mut args = vec!["serve".to_string(), "--foreground-daemon".to_string()];

    if let Some(ref m) = model {
        args.push("--model".to_string());
        args.push(m.clone());
    }
    if let Some(ref r) = region {
        args.push("--region".to_string());
        args.push(r.clone());
    }
    if let Some(ref p) = profile {
        args.push("--profile".to_string());
        args.push(p.clone());
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

    match child.try_wait()? {
        Some(status) => {
            let log_path = log_dir.join("nous-daemon.log");
            eprintln!("daemon exited immediately with {status}");
            eprintln!("check log: {}", log_path.display());
            std::process::exit(1);
        }
        None => {
            let pid = child.id();
            eprintln!("nous daemon started (pid: {pid})");
            eprintln!("PID file: {}", pid_file_path().display());
            eprintln!("Log file: {}", log_dir.join("nous-daemon.log").display());
        }
    }

    Ok(())
}

async fn execute(
    model: Option<String>,
    region: Option<String>,
    profile: Option<String>,
    port: Option<u16>,
    is_daemon: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if is_daemon {
        write_pid_file(std::process::id())?;
    }

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

    use nous_daemon::llm_client::{build_client, LlmConfig};

    let llm_config = LlmConfig::resolve(model, region, profile);

    let has_credentials = std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        || std::env::var("AWS_PROFILE").is_ok()
        || std::env::var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI").is_ok();

    let (llm_client, default_model) = if has_credentials {
        let client = build_client(&llm_config).await;
        tracing::info!(region = %llm_config.region, model = %llm_config.model, "LLM client configured for Bedrock");
        (Some(Arc::new(client)), llm_config.model)
    } else {
        tracing::warn!("LLM client not available (no AWS credentials found in environment)");
        (None, llm_config.model)
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
        llm_client,
        default_model,
    };

    let _scheduler_handle = Scheduler::spawn(
        state.clone(),
        SchedulerConfig::default(),
        Arc::new(SystemClock),
        shutdown.clone(),
    );

    {
        let shutdown = shutdown.clone();
        let current_port = config.port;
        tokio::spawn(async move {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to register SIGTERM handler");
            let mut sighup =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
                    .expect("failed to register SIGHUP handler");
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
                        tracing::info!("SIGHUP received, reloading config");
                        match Config::load() {
                            Ok(new_config) => {
                                if new_config.port != current_port {
                                    tracing::warn!(
                                        "config reloaded: port changed from {} to {}, restart required",
                                        current_port,
                                        new_config.port
                                    );
                                } else {
                                    tracing::info!("config reloaded successfully (no restart required)");
                                }
                            }
                            Err(e) => {
                                tracing::error!("failed to reload config: {e}");
                            }
                        }
                    }
                }
            }
        });
    }

    let addr = format!("{}:{}", config.host, config.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("listening on {}", listener.local_addr()?);

    axum::serve(listener, nous_daemon::app(state.clone()))
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;

    process_registry.shutdown(&state).await;

    pools.close().await;

    if is_daemon {
        remove_pid_file();
    }

    Ok(())
}
