use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use nous_mcp::commands;
use nous_mcp::config;
use nous_mcp::daemon_client::DaemonClient;
use nous_mcp::server::NousServer;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

#[derive(Debug, Clone, ValueEnum)]
enum SearchMode {
    Fts,
    Semantic,
    Hybrid,
}

#[derive(Debug, Clone, ValueEnum)]
enum Transport {
    Stdio,
    Http,
}

use commands::OutputFormat;

#[derive(Debug, Parser)]
#[command(name = "nous", about = "Nous memory system CLI")]
struct Cli {
    #[arg(long, global = true, help = "Config file path")]
    config: Option<PathBuf>,

    #[arg(long, global = true, help = "Database path")]
    db: Option<PathBuf>,

    #[arg(short, long, global = true, help = "Verbose output")]
    verbose: bool,

    #[arg(short, long, global = true, help = "Quiet mode")]
    quiet: bool,

    #[arg(long, global = true, default_value = "human", help = "Output format")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "stdio")]
        transport: Transport,
        #[arg(long, default_value_t = 8377)]
        port: u16,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        variant: Option<String>,
    },
    ReEmbed {
        #[arg(long)]
        model: String,
        #[arg(long)]
        variant: Option<String>,
    },
    ReClassify {
        #[arg(long)]
        since: Option<String>,
    },
    Category(CategoryCmd),
    Room(RoomCmd),
    Daemon(DaemonCmd),
    Export {
        #[arg(long, default_value = "json")]
        export_format: String,
    },
    Import {
        file: PathBuf,
    },
    RotateKey {
        #[arg(long)]
        new_key_file: Option<PathBuf>,
    },
    Status,
    Trace {
        #[arg(long, group = "lookup")]
        trace_id: Option<String>,
        #[arg(long, group = "lookup")]
        memory_id: Option<String>,
        #[arg(long, requires = "trace_id")]
        session_id: Option<String>,
    },
    Store {
        #[arg(long)]
        title: String,
        #[arg(long, allow_hyphen_values = true)]
        content: String,
        #[arg(long, default_value = "observation")]
        r#type: String,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        importance: Option<String>,
        #[arg(long)]
        confidence: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        trace_id: Option<String>,
        #[arg(long)]
        agent_id: Option<String>,
        #[arg(long)]
        agent_model: Option<String>,
        #[arg(long)]
        valid_from: Option<String>,
        #[arg(long)]
        category_id: Option<i64>,
    },
    Recall {
        id: String,
    },
    Update {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long, allow_hyphen_values = true)]
        content: Option<String>,
        #[arg(long)]
        importance: Option<String>,
        #[arg(long)]
        confidence: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long)]
        valid_until: Option<String>,
    },
    Forget {
        id: String,
        #[arg(long)]
        hard: bool,
    },
    Unarchive {
        id: String,
    },
    Relate {
        source: String,
        target: String,
        r#type: String,
    },
    Unrelate {
        source: String,
        target: String,
        r#type: String,
    },
    Search {
        query: String,
        #[arg(long, default_value = "hybrid")]
        mode: SearchMode,
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long)]
        importance: Option<String>,
        #[arg(long)]
        confidence: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        #[arg(long)]
        archived: bool,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
        #[arg(long)]
        valid_only: bool,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Context {
        workspace: String,
        #[arg(long)]
        summary: Option<String>,
    },
    Sql {
        query: String,
    },
    Schema,
    Workspaces,
    Tags,
    Model(ModelCmd),
    Embedding(EmbeddingCmd),
}

#[derive(Debug, Parser)]
struct ModelCmd {
    #[command(subcommand)]
    command: ModelSubcommand,
}

#[derive(Debug, Subcommand)]
enum ModelSubcommand {
    List,
    Info {
        id: i64,
    },
    Register {
        #[arg(long)]
        name: String,
        #[arg(long)]
        variant: String,
        #[arg(long)]
        dimensions: i64,
        #[arg(long, default_value_t = 512)]
        chunk_size: i64,
        #[arg(long, default_value_t = 64)]
        chunk_overlap: i64,
    },
    Activate {
        id: i64,
    },
    Deactivate {
        id: i64,
    },
    Switch {
        id: i64,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Parser)]
struct EmbeddingCmd {
    #[command(subcommand)]
    command: EmbeddingSubcommand,
}

#[derive(Debug, Subcommand)]
enum EmbeddingSubcommand {
    Inspect,
    Reset {
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Parser)]
struct CategoryCmd {
    #[command(subcommand)]
    command: CategorySubcommand,
}

#[derive(Debug, Subcommand)]
enum CategorySubcommand {
    List {
        #[arg(long)]
        source: Option<String>,
    },
    Add {
        name: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    Delete {
        name: String,
    },
    Rename {
        old: String,
        new: String,
    },
    Update {
        name: String,
        #[arg(long)]
        new_name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        threshold: Option<f32>,
    },
}

#[derive(Debug, Parser)]
struct RoomCmd {
    #[command(subcommand)]
    command: RoomSubcommand,
}

#[derive(Debug, Subcommand)]
enum RoomSubcommand {
    Create {
        name: String,
        #[arg(long)]
        purpose: Option<String>,
    },
    List {
        #[arg(long)]
        archived: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
    Get {
        id: String,
    },
    Post {
        room: String,
        content: String,
        #[arg(long)]
        sender: Option<String>,
        #[arg(long)]
        reply_to: Option<String>,
    },
    Read {
        room: String,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        since: Option<String>,
    },
    Search {
        room: String,
        query: String,
        #[arg(long)]
        limit: Option<usize>,
    },
    Delete {
        id: String,
        #[arg(long)]
        hard: bool,
    },
}

#[derive(Debug, Parser)]
struct DaemonCmd {
    #[command(subcommand)]
    command: DaemonSubcommand,
}

#[derive(Debug, Subcommand)]
enum DaemonSubcommand {
    Start {
        #[arg(long)]
        foreground: bool,
    },
    Stop,
    Restart {
        #[arg(long)]
        foreground: bool,
    },
    Status,
}

fn build_embedding(model: &str, variant: &str) -> Box<dyn nous_core::embed::EmbeddingBackend> {
    match nous_core::embed::OnnxBackend::builder()
        .model(model)
        .variant(variant)
        .build()
    {
        Ok(backend) => Box::new(backend),
        Err(e) => {
            eprintln!("Warning: OnnxBackend failed ({e}), falling back to MockEmbedding");
            Box::new(nous_core::embed::MockEmbedding::new(384))
        }
    }
}

fn try_daemon_client(config: &config::Config) -> Option<DaemonClient> {
    let pid_file = commands::expand_tilde(&config.daemon.pid_file);
    let pid_path = Path::new(&pid_file);
    if !pid_path.exists() {
        return None;
    }

    let pid_str = std::fs::read_to_string(pid_path).ok()?;
    let pid: u32 = pid_str.trim().parse().ok()?;

    if !Path::new(&format!("/proc/{pid}")).exists() {
        return None;
    }

    let socket_path = commands::expand_tilde(&config.daemon.socket_path);
    if !Path::new(&socket_path).exists() {
        return None;
    }

    Some(DaemonClient::new(socket_path))
}

fn run_daemon_start(config: &config::Config, foreground: bool) {
    if foreground {
        let daemon_cfg = config::DaemonConfig {
            socket_path: commands::expand_tilde(&config.daemon.socket_path),
            pid_file: commands::expand_tilde(&config.daemon.pid_file),
            log_file: commands::expand_tilde(&config.daemon.log_file),
            ..config.daemon.clone()
        };

        let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        rt.block_on(async {
            let daemon = match nous_mcp::daemon::Daemon::new(&daemon_cfg) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("daemon start failed: {e}");
                    std::process::exit(1);
                }
            };

            let db_path = commands::expand_tilde(&config.memory.db_path);
            let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
            let server = match NousServer::new(config.clone(), embedding, &db_path) {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    eprintln!("server init failed: {e}");
                    std::process::exit(1);
                }
            };

            let router = nous_mcp::daemon_api::daemon_router(daemon.shutdown_sender(), server);

            eprintln!(
                "daemon started (PID {}, socket {})",
                std::process::id(),
                daemon_cfg.socket_path
            );

            if let Err(e) = daemon.run(router).await {
                eprintln!("daemon error: {e}");
                std::process::exit(1);
            }
        });
    } else {
        let exe = std::env::current_exe().unwrap_or_else(|e| {
            eprintln!("cannot find current executable: {e}");
            std::process::exit(1);
        });

        let log_path = commands::expand_tilde(&config.daemon.log_file);
        if let Some(parent) = Path::new(&log_path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let log_file = std::fs::File::create(&log_path).unwrap_or_else(|e| {
            eprintln!("cannot create log file {log_path}: {e}");
            std::process::exit(1);
        });
        let stderr_file = log_file.try_clone().unwrap();

        let mut child = std::process::Command::new(exe)
            .args(["daemon", "start", "--foreground"])
            .stdout(log_file)
            .stderr(stderr_file)
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap_or_else(|e| {
                eprintln!("failed to spawn daemon: {e}");
                std::process::exit(1);
            });

        println!("daemon started (PID {})", child.id());
        println!("log: {log_path}");

        // Detach: parent exits immediately, child becomes a daemon.
        // Brief wait to detect immediate startup failures.
        std::thread::sleep(std::time::Duration::from_millis(200));
        match child.try_wait() {
            Ok(Some(status)) if !status.success() => {
                eprintln!("daemon exited immediately with {status}");
                std::process::exit(1);
            }
            _ => {}
        }
    }
}

fn run_daemon_stop(config: &config::Config) {
    let pid_path = commands::expand_tilde(&config.daemon.pid_file);
    let pid_str = match std::fs::read_to_string(&pid_path) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("daemon not running (no PID file)");
            return;
        }
    };
    let pid: u32 = match pid_str.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            eprintln!("invalid PID file");
            let _ = std::fs::remove_file(&pid_path);
            return;
        }
    };

    let socket_path = commands::expand_tilde(&config.daemon.socket_path);
    let client = DaemonClient::new(&socket_path);
    let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");

    match rt.block_on(client.shutdown()) {
        Ok(_) => {
            for _ in 0..20 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if !Path::new(&format!("/proc/{pid}")).exists() {
                    println!("daemon stopped (PID {pid})");
                    return;
                }
            }
            eprintln!("daemon did not exit in time, sending SIGTERM");
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
            if Path::new(&format!("/proc/{pid}")).exists() {
                eprintln!("sending SIGKILL");
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
            let _ = std::fs::remove_file(&pid_path);
            let _ = std::fs::remove_file(&socket_path);
            println!("daemon stopped (PID {pid})");
        }
        Err(e) => {
            eprintln!("shutdown request failed: {e}");
            if !Path::new(&format!("/proc/{pid}")).exists() {
                let _ = std::fs::remove_file(&pid_path);
                let _ = std::fs::remove_file(&socket_path);
                println!("daemon was not running (stale PID file cleaned)");
            } else {
                eprintln!("sending SIGTERM to PID {pid}");
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
                let _ = std::fs::remove_file(&pid_path);
                let _ = std::fs::remove_file(&socket_path);
            }
        }
    }
}

fn run_daemon_status(config: &config::Config) {
    match try_daemon_client(config) {
        Some(client) => {
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            match rt.block_on(client.status()) {
                Ok(status) => {
                    println!("pid: {}", status.pid);
                    println!("uptime: {}s", status.uptime_secs);
                    println!("version: {}", status.version);
                }
                Err(e) => {
                    eprintln!("daemon probe failed: {e}");
                }
            }
        }
        None => {
            println!("daemon not running");
        }
    }
}

fn route_room_via_daemon(
    client: &DaemonClient,
    room_sub: &RoomSubcommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match room_sub {
            RoomSubcommand::Create { name, purpose } => {
                let body = serde_json::json!({
                    "name": name,
                    "purpose": purpose,
                });
                let resp: serde_json::Value = client
                    .post_json("/rooms", &body)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(id) = resp.get("id").and_then(|v| v.as_str()) {
                    println!("{id}");
                }
            }
            RoomSubcommand::List { archived, limit } => {
                let mut path = format!("/rooms?archived={archived}");
                if let Some(l) = limit {
                    path.push_str(&format!("&limit={l}"));
                }
                let resp: serde_json::Value =
                    client.get_json(&path).await.map_err(|e| e.to_string())?;
                if let Some(rooms) = resp.get("rooms").and_then(|v| v.as_array()) {
                    if rooms.is_empty() {
                        println!("No rooms found.");
                    } else {
                        for r in rooms {
                            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
                            let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let purpose = r.get("purpose").and_then(|v| v.as_str()).unwrap_or("");
                            let created =
                                r.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                            println!("{id}  {name}  {purpose}  {created}");
                        }
                    }
                }
            }
            RoomSubcommand::Get { id } => {
                let resp: serde_json::Value = client
                    .get_json(&format!("/rooms/{id}"))
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(room_id) = resp.get("id").and_then(|v| v.as_str()) {
                    println!("id: {room_id}");
                }
                if let Some(name) = resp.get("name").and_then(|v| v.as_str()) {
                    println!("name: {name}");
                }
                if let Some(purpose) = resp.get("purpose").and_then(|v| v.as_str()) {
                    println!("purpose: {purpose}");
                }
            }
            RoomSubcommand::Post {
                room,
                content,
                sender,
                reply_to,
            } => {
                let body = serde_json::json!({
                    "content": content,
                    "sender": sender,
                    "reply_to": reply_to,
                });
                let resp: serde_json::Value = client
                    .post_json(&format!("/rooms/{room}/messages"), &body)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Some(id) = resp.get("id").and_then(|v| v.as_str()) {
                    println!("{id}");
                }
            }
            RoomSubcommand::Read { room, limit, since } => {
                let mut path = format!("/rooms/{room}/messages?");
                if let Some(l) = limit {
                    path.push_str(&format!("limit={l}&"));
                }
                if let Some(s) = since {
                    path.push_str(&format!("since={s}&"));
                }
                let resp: serde_json::Value =
                    client.get_json(&path).await.map_err(|e| e.to_string())?;
                if let Some(messages) = resp.get("messages").and_then(|v| v.as_array()) {
                    for m in messages {
                        let sender_id = m.get("sender_id").and_then(|v| v.as_str()).unwrap_or("");
                        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        let created = m.get("created_at").and_then(|v| v.as_str()).unwrap_or("");
                        println!("[{created}] {sender_id}: {content}");
                    }
                }
            }
            RoomSubcommand::Search { .. } => {
                Err("search not available via daemon, falling back")?;
            }
            RoomSubcommand::Delete { .. } => {
                Err("delete not available via daemon, falling back")?;
            }
        }
        Ok(())
    })
}

fn main() {
    let cli = Cli::parse();

    let config = config::Config::load(cli.config.clone()).unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load config: {e}");
        config::Config::default()
    });

    let mut config = config;
    if let Some(ref db_path) = cli.db {
        config.memory.db_path = db_path.to_string_lossy().into_owned();
    }

    let _db_key = config.resolve_db_key().ok();
    let format = cli.format.clone();

    match run_command(cli, &config, &format) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("Error: {e}");
            let code = e
                .downcast_ref::<nous_shared::NousError>()
                .map(|ne| ne.exit_code())
                .unwrap_or(1);
            std::process::exit(code);
        }
    }
}

fn run_command(
    cli: Cli,
    config: &config::Config,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Serve {
            transport,
            port,
            model,
            variant,
        } => {
            let model = model.unwrap_or_else(|| config.embedding.model.clone());
            let variant = variant.unwrap_or_else(|| config.embedding.variant.clone());
            let rt = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
            rt.block_on(run_serve(config.clone(), transport, port, &model, &variant))?;
        }
        Command::ReEmbed { model, variant } => {
            let variant = variant.unwrap_or_else(|| config.embedding.variant.clone());
            let embedding = build_embedding(&model, &variant);
            commands::run_re_embed(config, embedding.as_ref())?;
        }
        Command::ReClassify { since } => {
            let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
            commands::run_re_classify(config, since.as_deref(), embedding.as_ref())?;
        }
        Command::Category(cat) => match cat.command {
            CategorySubcommand::List { source } => {
                commands::run_category_list(config, source.as_deref(), format)?;
            }
            CategorySubcommand::Add {
                name,
                parent,
                description,
            } => {
                let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
                commands::run_category_add(
                    config,
                    &name,
                    parent.as_deref(),
                    description.as_deref(),
                    embedding.as_ref(),
                )?;
            }
            CategorySubcommand::Delete { name } => {
                commands::run_category_delete(config, &name)?;
            }
            CategorySubcommand::Rename { old, new } => {
                let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
                commands::run_category_rename(config, &old, &new, embedding.as_ref())?;
            }
            CategorySubcommand::Update {
                name,
                new_name,
                description,
                threshold,
            } => {
                let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
                commands::run_category_update(
                    config,
                    &name,
                    new_name.as_deref(),
                    description.as_deref(),
                    threshold,
                    embedding.as_ref(),
                )?;
            }
        },
        Command::Room(room) => {
            if let Some(client) = try_daemon_client(config)
                && route_room_via_daemon(&client, &room.command).is_ok()
            {
                return Ok(());
            }
            match room.command {
                RoomSubcommand::Create { name, purpose } => {
                    commands::run_room_create(config, &name, purpose.as_deref())?;
                }
                RoomSubcommand::List { archived, limit } => {
                    commands::run_room_list(config, archived, limit)?;
                }
                RoomSubcommand::Get { id } => {
                    commands::run_room_get(config, &id)?;
                }
                RoomSubcommand::Post {
                    room,
                    content,
                    sender,
                    reply_to,
                } => {
                    commands::run_room_post(
                        config,
                        &room,
                        &content,
                        sender.as_deref(),
                        reply_to.as_deref(),
                    )?;
                }
                RoomSubcommand::Read { room, limit, since } => {
                    commands::run_room_read(config, &room, limit, since.as_deref())?;
                }
                RoomSubcommand::Search { room, query, limit } => {
                    commands::run_room_search(config, &room, &query, limit)?;
                }
                RoomSubcommand::Delete { id, hard } => {
                    commands::run_room_delete(config, &id, hard)?;
                }
            }
        }
        Command::Export { export_format: _ } => {
            commands::run_export(config)?;
        }
        Command::Import { file } => {
            let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
            commands::run_import(config, &file, embedding.as_ref())?;
        }
        Command::RotateKey { new_key_file } => {
            commands::run_rotate_key(config, new_key_file.as_deref())?;
        }
        Command::Status => {
            commands::run_status(config, format)?;
        }
        Command::Trace {
            trace_id,
            memory_id,
            session_id,
        } => {
            commands::run_trace(
                config,
                trace_id.as_deref(),
                memory_id.as_deref(),
                session_id.as_deref(),
            )?;
        }
        Command::Store {
            title,
            content,
            r#type,
            source,
            importance,
            confidence,
            tags,
            workspace,
            session_id,
            trace_id,
            agent_id,
            agent_model,
            valid_from,
            category_id,
        } => {
            let actual_content = if content == "-" {
                std::io::read_to_string(std::io::stdin())?
            } else {
                content
            };
            commands::run_store(
                config,
                &title,
                &actual_content,
                &r#type,
                source.as_deref(),
                importance.as_deref(),
                confidence.as_deref(),
                &tags.unwrap_or_default(),
                workspace.as_deref(),
                session_id.as_deref(),
                trace_id.as_deref(),
                agent_id.as_deref(),
                agent_model.as_deref(),
                valid_from.as_deref(),
                category_id,
                format,
            )?;
        }
        Command::Recall { id } => {
            commands::run_recall(config, &id, format)?;
        }
        Command::Update {
            id,
            title,
            content,
            importance,
            confidence,
            tags,
            valid_until,
        } => {
            let actual_content = match content.as_deref() {
                Some("-") => Some(std::io::read_to_string(std::io::stdin())?),
                other => other.map(String::from),
            };
            commands::run_update(
                config,
                &id,
                title.as_deref(),
                actual_content.as_deref(),
                importance.as_deref(),
                confidence.as_deref(),
                tags.as_deref(),
                valid_until.as_deref(),
                format,
            )?;
        }
        Command::Forget { id, hard } => {
            commands::run_forget(config, &id, hard, format)?;
        }
        Command::Unarchive { id } => {
            commands::run_unarchive(config, &id, format)?;
        }
        Command::Relate {
            source,
            target,
            r#type,
        } => {
            commands::run_relate(config, &source, &target, &r#type, format)?;
        }
        Command::Unrelate {
            source,
            target,
            r#type,
        } => {
            commands::run_unrelate(config, &source, &target, &r#type, format)?;
        }
        Command::Search {
            query,
            mode,
            r#type,
            importance,
            confidence,
            workspace,
            tags,
            archived,
            since,
            until,
            valid_only,
            limit,
        } => {
            let mode_str = match mode {
                SearchMode::Fts => "fts",
                SearchMode::Semantic => "semantic",
                SearchMode::Hybrid => "hybrid",
            };
            commands::run_search(
                config,
                &query,
                mode_str,
                r#type.as_deref(),
                importance.as_deref(),
                confidence.as_deref(),
                workspace.as_deref(),
                tags.as_deref(),
                archived,
                since.as_deref(),
                until.as_deref(),
                valid_only,
                limit,
                format,
            )?;
        }
        Command::Context { workspace, summary } => {
            commands::run_context(config, &workspace, summary.as_deref(), format)?;
        }
        Command::Sql { query } => {
            commands::run_sql(config, &query, format)?;
        }
        Command::Schema => {
            commands::run_schema(config)?;
        }
        Command::Workspaces => {
            commands::run_workspaces(config, format)?;
        }
        Command::Tags => {
            commands::run_tags(config, format)?;
        }
        Command::Model(model) => match model.command {
            ModelSubcommand::List => {
                commands::run_model_list(config, format)?;
            }
            ModelSubcommand::Info { id } => {
                commands::run_model_info(config, id, format)?;
            }
            ModelSubcommand::Register {
                name,
                variant,
                dimensions,
                chunk_size,
                chunk_overlap,
            } => {
                commands::run_model_register(
                    config,
                    &name,
                    &variant,
                    dimensions,
                    chunk_size,
                    chunk_overlap,
                    format,
                )?;
            }
            ModelSubcommand::Activate { id } => {
                commands::run_model_activate(config, id, format)?;
            }
            ModelSubcommand::Deactivate { id } => {
                commands::run_model_deactivate(config, id, format)?;
            }
            ModelSubcommand::Switch { id, force } => {
                commands::run_model_switch(config, id, force, format)?;
            }
        },
        Command::Embedding(emb) => match emb.command {
            EmbeddingSubcommand::Inspect => {
                commands::run_embedding_inspect(config, format)?;
            }
            EmbeddingSubcommand::Reset { force } => {
                commands::run_embedding_reset(config, force, format)?;
            }
        },
        Command::Daemon(daemon) => match daemon.command {
            DaemonSubcommand::Start { foreground } => {
                run_daemon_start(config, foreground);
            }
            DaemonSubcommand::Stop => {
                run_daemon_stop(config);
            }
            DaemonSubcommand::Restart { foreground } => {
                run_daemon_stop(config);
                std::thread::sleep(std::time::Duration::from_millis(500));
                run_daemon_start(config, foreground);
            }
            DaemonSubcommand::Status => {
                run_daemon_status(config);
            }
        },
    }
    Ok(())
}

async fn run_serve(
    config: config::Config,
    transport: Transport,
    port: u16,
    model: &str,
    variant: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = config.memory.db_path.clone();
    let embedding = build_embedding(model, variant);
    let server = NousServer::new(config, embedding, &db_path)?;

    match transport {
        Transport::Stdio => {
            let transport = rmcp::transport::io::stdio();
            let service = server.serve(transport).await?;
            service.waiting().await?;
        }
        Transport::Http => {
            let user_config = server.config.clone();
            let user_db_path = db_path.clone();
            let user_model = model.to_owned();
            let user_variant = variant.to_owned();
            let http_config = StreamableHttpServerConfig::default();
            let ct = http_config.cancellation_token.clone();
            let session_manager = Arc::new(LocalSessionManager::default());
            let service = StreamableHttpService::new(
                move || {
                    let embedding = build_embedding(&user_model, &user_variant);
                    let cfg = user_config.clone();
                    NousServer::new(cfg, embedding, &user_db_path)
                        .map_err(|e| std::io::Error::other(e.to_string()))
                },
                session_manager,
                http_config,
            );
            let router = axum::Router::new().fallback_service(service);
            let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
            let listener = tokio::net::TcpListener::bind(addr).await?;
            eprintln!("Nous MCP HTTP server listening on {addr}");
            axum::serve(listener, router)
                .with_graceful_shutdown(async move { ct.cancelled().await })
                .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_defaults() {
        let cli = Cli::try_parse_from(["nous", "serve"]).unwrap();
        match cli.command {
            Command::Serve {
                transport,
                port,
                model,
                variant,
            } => {
                assert!(matches!(transport, Transport::Stdio));
                assert_eq!(port, 8377);
                assert!(model.is_none());
                assert!(variant.is_none());
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn serve_explicit_http_and_port() {
        let cli = Cli::try_parse_from(["nous", "serve", "--transport", "http", "--port", "9000"])
            .unwrap();
        match cli.command {
            Command::Serve {
                transport,
                port,
                model,
                variant,
            } => {
                assert!(matches!(transport, Transport::Http));
                assert_eq!(port, 9000);
                assert!(model.is_none());
                assert!(variant.is_none());
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn serve_with_model_and_variant() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "serve",
            "--model",
            "org/repo",
            "--variant",
            "q4",
        ])
        .unwrap();
        match cli.command {
            Command::Serve { model, variant, .. } => {
                assert_eq!(model.as_deref(), Some("org/repo"));
                assert_eq!(variant.as_deref(), Some("q4"));
            }
            _ => panic!("expected Serve"),
        }
    }

    #[test]
    fn re_embed_with_model() {
        let cli = Cli::try_parse_from(["nous", "re-embed", "--model", "org/repo"]).unwrap();
        match cli.command {
            Command::ReEmbed { model, variant } => {
                assert_eq!(model, "org/repo");
                assert!(variant.is_none());
            }
            _ => panic!("expected ReEmbed"),
        }
    }

    #[test]
    fn re_embed_with_variant() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "re-embed",
            "--model",
            "org/repo",
            "--variant",
            "q4",
        ])
        .unwrap();
        match cli.command {
            Command::ReEmbed { model, variant } => {
                assert_eq!(model, "org/repo");
                assert_eq!(variant.as_deref(), Some("q4"));
            }
            _ => panic!("expected ReEmbed"),
        }
    }

    #[test]
    fn re_classify_no_args() {
        let cli = Cli::try_parse_from(["nous", "re-classify"]).unwrap();
        match cli.command {
            Command::ReClassify { since } => assert!(since.is_none()),
            _ => panic!("expected ReClassify"),
        }
    }

    #[test]
    fn re_classify_with_since() {
        let cli = Cli::try_parse_from(["nous", "re-classify", "--since", "2024-01-01"]).unwrap();
        match cli.command {
            Command::ReClassify { since } => {
                assert_eq!(since.as_deref(), Some("2024-01-01"));
            }
            _ => panic!("expected ReClassify"),
        }
    }

    #[test]
    fn category_add() {
        let cli = Cli::try_parse_from(["nous", "category", "add", "testing"]).unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command:
                    CategorySubcommand::Add {
                        name,
                        parent,
                        description,
                    },
            }) => {
                assert_eq!(name, "testing");
                assert!(parent.is_none());
                assert!(description.is_none());
            }
            _ => panic!("expected Category Add"),
        }
    }

    #[test]
    fn category_add_with_parent_and_description() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "category",
            "add",
            "unit-tests",
            "--parent",
            "testing",
            "--description",
            "Unit test category",
        ])
        .unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command:
                    CategorySubcommand::Add {
                        name,
                        parent,
                        description,
                    },
            }) => {
                assert_eq!(name, "unit-tests");
                assert_eq!(parent.as_deref(), Some("testing"));
                assert_eq!(description.as_deref(), Some("Unit test category"));
            }
            _ => panic!("expected Category Add"),
        }
    }

    #[test]
    fn category_list_no_filter() {
        let cli = Cli::try_parse_from(["nous", "category", "list"]).unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command: CategorySubcommand::List { source },
            }) => {
                assert!(source.is_none());
            }
            _ => panic!("expected Category List"),
        }
    }

    #[test]
    fn category_list_with_source() {
        let cli = Cli::try_parse_from(["nous", "category", "list", "--source", "manual"]).unwrap();
        match cli.command {
            Command::Category(CategoryCmd {
                command: CategorySubcommand::List { source },
            }) => {
                assert_eq!(source.as_deref(), Some("manual"));
            }
            _ => panic!("expected Category List"),
        }
    }

    #[test]
    fn export_default_format() {
        let cli = Cli::try_parse_from(["nous", "export"]).unwrap();
        match cli.command {
            Command::Export { export_format } => assert_eq!(export_format, "json"),
            _ => panic!("expected Export"),
        }
    }

    #[test]
    fn import_file() {
        let cli = Cli::try_parse_from(["nous", "import", "/tmp/data.json"]).unwrap();
        match cli.command {
            Command::Import { file } => {
                assert_eq!(file, PathBuf::from("/tmp/data.json"));
            }
            _ => panic!("expected Import"),
        }
    }

    #[test]
    fn rotate_key_no_file() {
        let cli = Cli::try_parse_from(["nous", "rotate-key"]).unwrap();
        match cli.command {
            Command::RotateKey { new_key_file } => assert!(new_key_file.is_none()),
            _ => panic!("expected RotateKey"),
        }
    }

    #[test]
    fn rotate_key_with_file() {
        let cli =
            Cli::try_parse_from(["nous", "rotate-key", "--new-key-file", "/tmp/key.bin"]).unwrap();
        match cli.command {
            Command::RotateKey { new_key_file } => {
                assert_eq!(new_key_file, Some(PathBuf::from("/tmp/key.bin")));
            }
            _ => panic!("expected RotateKey"),
        }
    }

    #[test]
    fn status_command() {
        let cli = Cli::try_parse_from(["nous", "status"]).unwrap();
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn trace_with_trace_id() {
        let cli = Cli::try_parse_from(["nous", "trace", "--trace-id", "abc123"]).unwrap();
        match cli.command {
            Command::Trace {
                trace_id,
                memory_id,
                session_id,
            } => {
                assert_eq!(trace_id.as_deref(), Some("abc123"));
                assert!(memory_id.is_none());
                assert!(session_id.is_none());
            }
            _ => panic!("expected Trace"),
        }
    }

    #[test]
    fn trace_with_trace_id_and_session_id() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "trace",
            "--trace-id",
            "abc123",
            "--session-id",
            "sess-456",
        ])
        .unwrap();
        match cli.command {
            Command::Trace {
                trace_id,
                memory_id,
                session_id,
            } => {
                assert_eq!(trace_id.as_deref(), Some("abc123"));
                assert!(memory_id.is_none());
                assert_eq!(session_id.as_deref(), Some("sess-456"));
            }
            _ => panic!("expected Trace"),
        }
    }

    #[test]
    fn trace_with_memory_id() {
        let cli = Cli::try_parse_from(["nous", "trace", "--memory-id", "mem-789"]).unwrap();
        match cli.command {
            Command::Trace {
                trace_id,
                memory_id,
                session_id,
            } => {
                assert!(trace_id.is_none());
                assert_eq!(memory_id.as_deref(), Some("mem-789"));
                assert!(session_id.is_none());
            }
            _ => panic!("expected Trace"),
        }
    }

    #[test]
    fn trace_both_trace_and_memory_id_errors() {
        let result = Cli::try_parse_from(["nous", "trace", "--trace-id", "a", "--memory-id", "b"]);
        assert!(result.is_err());
    }

    #[test]
    fn trace_session_id_requires_trace_id() {
        let result = Cli::try_parse_from(["nous", "trace", "--session-id", "s"]);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_subcommand_errors() {
        let result = Cli::try_parse_from(["nous", "nonexistent"]);
        assert!(result.is_err());
    }

    #[test]
    fn re_embed_missing_model_errors() {
        let result = Cli::try_parse_from(["nous", "re-embed"]);
        assert!(result.is_err());
    }

    #[test]
    fn import_missing_file_errors() {
        let result = Cli::try_parse_from(["nous", "import"]);
        assert!(result.is_err());
    }

    #[test]
    fn room_create() {
        let cli = Cli::try_parse_from(["nous", "room", "create", "test-room"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Create { name, purpose },
            }) => {
                assert_eq!(name, "test-room");
                assert!(purpose.is_none());
            }
            _ => panic!("expected Room Create"),
        }
    }

    #[test]
    fn room_create_with_purpose() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "room",
            "create",
            "dev-chat",
            "--purpose",
            "Dev discussion",
        ])
        .unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Create { name, purpose },
            }) => {
                assert_eq!(name, "dev-chat");
                assert_eq!(purpose.as_deref(), Some("Dev discussion"));
            }
            _ => panic!("expected Room Create"),
        }
    }

    #[test]
    fn room_list_defaults() {
        let cli = Cli::try_parse_from(["nous", "room", "list"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::List { archived, limit },
            }) => {
                assert!(!archived);
                assert!(limit.is_none());
            }
            _ => panic!("expected Room List"),
        }
    }

    #[test]
    fn room_get() {
        let cli = Cli::try_parse_from(["nous", "room", "get", "my-room"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Get { id },
            }) => {
                assert_eq!(id, "my-room");
            }
            _ => panic!("expected Room Get"),
        }
    }

    #[test]
    fn room_post() {
        let cli = Cli::try_parse_from([
            "nous-mcp",
            "room",
            "post",
            "my-room",
            "Hello, world!",
            "--sender",
            "agent-1",
        ])
        .unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command:
                    RoomSubcommand::Post {
                        room,
                        content,
                        sender,
                        reply_to,
                    },
            }) => {
                assert_eq!(room, "my-room");
                assert_eq!(content, "Hello, world!");
                assert_eq!(sender.as_deref(), Some("agent-1"));
                assert!(reply_to.is_none());
            }
            _ => panic!("expected Room Post"),
        }
    }

    #[test]
    fn room_read() {
        let cli =
            Cli::try_parse_from(["nous", "room", "read", "dev-chat", "--limit", "10"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Read { room, limit, since },
            }) => {
                assert_eq!(room, "dev-chat");
                assert_eq!(limit, Some(10));
                assert!(since.is_none());
            }
            _ => panic!("expected Room Read"),
        }
    }

    #[test]
    fn room_search() {
        let cli = Cli::try_parse_from(["nous", "room", "search", "dev-chat", "linter"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Search { room, query, limit },
            }) => {
                assert_eq!(room, "dev-chat");
                assert_eq!(query, "linter");
                assert!(limit.is_none());
            }
            _ => panic!("expected Room Search"),
        }
    }

    #[test]
    fn room_delete() {
        let cli = Cli::try_parse_from(["nous", "room", "delete", "old-room", "--hard"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Delete { id, hard },
            }) => {
                assert_eq!(id, "old-room");
                assert!(hard);
            }
            _ => panic!("expected Room Delete"),
        }
    }

    fn test_db_path() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(
            "/tmp/nous-test-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            seq,
        )
    }

    #[tokio::test]
    async fn server_constructs_with_mock_embedding() {
        let db_path = test_db_path();
        let mut cfg = config::Config::default();
        cfg.encryption.db_key_file = format!("{db_path}.key");
        let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
        let server = NousServer::new(cfg, embedding, &db_path).expect("server should construct");

        assert!(server.embedding.dimensions() == 384);
        assert_eq!(server.embedding.model_id(), "mock");
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn server_lists_all_30_tools() {
        use rmcp::model::CallToolRequestParams;
        use rmcp::{ClientHandler, ServiceExt};

        let (server_transport, client_transport) = tokio::io::duplex(4096);

        let db_path = test_db_path();
        let mut cfg = config::Config::default();
        cfg.encryption.db_key_file = format!("{db_path}.key");
        let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
        let server = NousServer::new(cfg, embedding, &db_path).unwrap();

        let server_handle = tokio::spawn(async move {
            server.serve(server_transport).await?.waiting().await?;
            anyhow::Ok(())
        });

        #[derive(Debug, Clone, Default)]
        struct TestClient;
        impl ClientHandler for TestClient {}

        let client = TestClient.serve(client_transport).await.unwrap();
        let tools_result = client.peer().list_tools(Default::default()).await.unwrap();
        let tool_names: Vec<&str> = tools_result.tools.iter().map(|t| t.name.as_ref()).collect();

        let expected = [
            "memory_store",
            "memory_recall",
            "memory_search",
            "memory_context",
            "memory_forget",
            "memory_unarchive",
            "memory_update",
            "memory_relate",
            "memory_unrelate",
            "memory_category_suggest",
            "memory_category_list",
            "memory_category_add",
            "memory_category_delete",
            "memory_category_update",
            "memory_workspaces",
            "memory_tags",
            "memory_stats",
            "memory_schema",
            "memory_sql",
            "otlp_trace_context",
            "otlp_memory_context",
            "room_create",
            "room_list",
            "room_get",
            "room_delete",
            "room_post_message",
            "room_read_messages",
            "room_search",
            "room_info",
            "room_join",
        ];

        assert_eq!(
            tools_result.tools.len(),
            30,
            "expected 30 tools, got {:?}",
            tool_names
        );

        for name in &expected {
            assert!(tool_names.contains(name), "missing tool: {name}");
        }

        let result = client
            .call_tool(
                CallToolRequestParams::new("memory_search").with_arguments(
                    serde_json::json!({"query": "test"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));

        client.cancel().await.unwrap();
        server_handle.await.unwrap().unwrap();
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn daemon_start_foreground() {
        let cli = Cli::try_parse_from(["nous", "daemon", "start", "--foreground"]).unwrap();
        match cli.command {
            Command::Daemon(DaemonCmd {
                command: DaemonSubcommand::Start { foreground },
            }) => {
                assert!(foreground);
            }
            _ => panic!("expected Daemon Start"),
        }
    }

    #[test]
    fn daemon_start_background() {
        let cli = Cli::try_parse_from(["nous", "daemon", "start"]).unwrap();
        match cli.command {
            Command::Daemon(DaemonCmd {
                command: DaemonSubcommand::Start { foreground },
            }) => {
                assert!(!foreground);
            }
            _ => panic!("expected Daemon Start"),
        }
    }

    #[test]
    fn daemon_stop() {
        let cli = Cli::try_parse_from(["nous", "daemon", "stop"]).unwrap();
        match cli.command {
            Command::Daemon(DaemonCmd {
                command: DaemonSubcommand::Stop,
            }) => {}
            _ => panic!("expected Daemon Stop"),
        }
    }

    #[test]
    fn daemon_restart() {
        let cli = Cli::try_parse_from(["nous", "daemon", "restart", "--foreground"]).unwrap();
        match cli.command {
            Command::Daemon(DaemonCmd {
                command: DaemonSubcommand::Restart { foreground },
            }) => {
                assert!(foreground);
            }
            _ => panic!("expected Daemon Restart"),
        }
    }

    #[test]
    fn daemon_restart_background() {
        let cli = Cli::try_parse_from(["nous", "daemon", "restart"]).unwrap();
        match cli.command {
            Command::Daemon(DaemonCmd {
                command: DaemonSubcommand::Restart { foreground },
            }) => {
                assert!(!foreground);
            }
            _ => panic!("expected Daemon Restart"),
        }
    }

    #[test]
    fn daemon_status() {
        let cli = Cli::try_parse_from(["nous", "daemon", "status"]).unwrap();
        match cli.command {
            Command::Daemon(DaemonCmd {
                command: DaemonSubcommand::Status,
            }) => {}
            _ => panic!("expected Daemon Status"),
        }
    }

    #[test]
    fn global_format_json() {
        let cli = Cli::try_parse_from(["nous", "--format", "json", "status"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Json));
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn global_format_csv() {
        let cli = Cli::try_parse_from(["nous", "--format", "csv", "status"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Csv));
    }

    #[test]
    fn global_format_default_is_human() {
        let cli = Cli::try_parse_from(["nous", "status"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Human));
    }

    #[test]
    fn global_verbose_flag() {
        let cli = Cli::try_parse_from(["nous", "--verbose", "status"]).unwrap();
        assert!(cli.verbose);
        assert!(!cli.quiet);
    }

    #[test]
    fn global_quiet_flag() {
        let cli = Cli::try_parse_from(["nous", "--quiet", "status"]).unwrap();
        assert!(cli.quiet);
        assert!(!cli.verbose);
    }

    #[test]
    fn global_config_path() {
        let cli = Cli::try_parse_from(["nous", "--config", "/tmp/nous.toml", "status"]).unwrap();
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/nous.toml")));
    }

    #[test]
    fn global_db_path() {
        let cli = Cli::try_parse_from(["nous", "--db", "/tmp/test.db", "status"]).unwrap();
        assert_eq!(cli.db, Some(PathBuf::from("/tmp/test.db")));
    }

    #[test]
    fn global_flags_after_subcommand() {
        let cli = Cli::try_parse_from(["nous", "status", "--format", "json"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Json));
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn global_short_verbose() {
        let cli = Cli::try_parse_from(["nous", "-v", "status"]).unwrap();
        assert!(cli.verbose);
    }

    #[test]
    fn global_short_quiet() {
        let cli = Cli::try_parse_from(["nous", "-q", "status"]).unwrap();
        assert!(cli.quiet);
    }

    #[test]
    fn global_invalid_format_errors() {
        let result = Cli::try_parse_from(["nous", "--format", "xml", "status"]);
        assert!(result.is_err());
    }

    #[test]
    fn try_daemon_client_returns_none_without_pid_file() {
        let mut cfg = config::Config::default();
        cfg.daemon.pid_file = "/tmp/nous-nonexistent-pid-file-test.pid".into();
        cfg.daemon.socket_path = "/tmp/nous-nonexistent-socket-test.sock".into();
        let _ = std::fs::remove_file(&cfg.daemon.pid_file);
        assert!(try_daemon_client(&cfg).is_none());
    }

    #[test]
    fn try_daemon_client_returns_none_for_dead_pid() {
        let dir = std::env::temp_dir().join(format!("nous-try-client-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let pid_file = dir.join("daemon.pid");
        std::fs::write(&pid_file, "999999999").unwrap();

        let mut cfg = config::Config::default();
        cfg.daemon.pid_file = pid_file.to_string_lossy().into_owned();
        cfg.daemon.socket_path = dir.join("daemon.sock").to_string_lossy().into_owned();

        assert!(try_daemon_client(&cfg).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Memory CRUD clap parsing tests ---

    #[test]
    fn store_minimal() {
        let cli = Cli::try_parse_from([
            "nous",
            "store",
            "--title",
            "Test",
            "--content",
            "Body",
            "--type",
            "decision",
        ])
        .unwrap();
        match cli.command {
            Command::Store {
                title,
                content,
                r#type,
                importance,
                tags,
                ..
            } => {
                assert_eq!(title, "Test");
                assert_eq!(content, "Body");
                assert_eq!(r#type, "decision");
                assert!(importance.is_none());
                assert!(tags.is_none());
            }
            _ => panic!("expected Store"),
        }
    }

    #[test]
    fn store_with_all_flags() {
        let cli = Cli::try_parse_from([
            "nous",
            "store",
            "--title",
            "Full",
            "--content",
            "All flags",
            "--type",
            "bugfix",
            "--importance",
            "high",
            "--confidence",
            "low",
            "--tags",
            "a,b,c",
            "--workspace",
            "/tmp",
            "--source",
            "test",
        ])
        .unwrap();
        match cli.command {
            Command::Store {
                title,
                content,
                r#type,
                importance,
                confidence,
                tags,
                workspace,
                source,
                ..
            } => {
                assert_eq!(title, "Full");
                assert_eq!(content, "All flags");
                assert_eq!(r#type, "bugfix");
                assert_eq!(importance.as_deref(), Some("high"));
                assert_eq!(confidence.as_deref(), Some("low"));
                assert_eq!(tags, Some(vec!["a".into(), "b".into(), "c".into()]));
                assert_eq!(workspace.as_deref(), Some("/tmp"));
                assert_eq!(source.as_deref(), Some("test"));
            }
            _ => panic!("expected Store"),
        }
    }

    #[test]
    fn store_stdin_content() {
        let cli =
            Cli::try_parse_from(["nous", "store", "--title", "Stdin", "--content", "-"]).unwrap();
        match cli.command {
            Command::Store { content, .. } => {
                assert_eq!(content, "-");
            }
            _ => panic!("expected Store"),
        }
    }

    #[test]
    fn store_default_type() {
        let cli =
            Cli::try_parse_from(["nous", "store", "--title", "Def", "--content", "Body"]).unwrap();
        match cli.command {
            Command::Store { r#type, .. } => {
                assert_eq!(r#type, "observation");
            }
            _ => panic!("expected Store"),
        }
    }

    #[test]
    fn recall_by_id() {
        let cli = Cli::try_parse_from(["nous", "recall", "mem_abc123"]).unwrap();
        match cli.command {
            Command::Recall { id } => {
                assert_eq!(id, "mem_abc123");
            }
            _ => panic!("expected Recall"),
        }
    }

    #[test]
    fn update_with_title() {
        let cli =
            Cli::try_parse_from(["nous", "update", "mem_abc123", "--title", "New title"]).unwrap();
        match cli.command {
            Command::Update {
                id,
                title,
                content,
                importance,
                ..
            } => {
                assert_eq!(id, "mem_abc123");
                assert_eq!(title.as_deref(), Some("New title"));
                assert!(content.is_none());
                assert!(importance.is_none());
            }
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn update_multiple_fields() {
        let cli = Cli::try_parse_from([
            "nous",
            "update",
            "mem_abc123",
            "--importance",
            "high",
            "--tags",
            "x,y",
        ])
        .unwrap();
        match cli.command {
            Command::Update {
                importance, tags, ..
            } => {
                assert_eq!(importance.as_deref(), Some("high"));
                assert_eq!(tags, Some(vec!["x".into(), "y".into()]));
            }
            _ => panic!("expected Update"),
        }
    }

    #[test]
    fn forget_soft() {
        let cli = Cli::try_parse_from(["nous", "forget", "mem_abc123"]).unwrap();
        match cli.command {
            Command::Forget { id, hard } => {
                assert_eq!(id, "mem_abc123");
                assert!(!hard);
            }
            _ => panic!("expected Forget"),
        }
    }

    #[test]
    fn forget_hard() {
        let cli = Cli::try_parse_from(["nous", "forget", "mem_abc123", "--hard"]).unwrap();
        match cli.command {
            Command::Forget { id, hard } => {
                assert_eq!(id, "mem_abc123");
                assert!(hard);
            }
            _ => panic!("expected Forget"),
        }
    }

    #[test]
    fn unarchive_by_id() {
        let cli = Cli::try_parse_from(["nous", "unarchive", "mem_abc123"]).unwrap();
        match cli.command {
            Command::Unarchive { id } => {
                assert_eq!(id, "mem_abc123");
            }
            _ => panic!("expected Unarchive"),
        }
    }

    #[test]
    fn relate_three_args() {
        let cli = Cli::try_parse_from(["nous", "relate", "mem_a", "mem_b", "supersedes"]).unwrap();
        match cli.command {
            Command::Relate {
                source,
                target,
                r#type,
            } => {
                assert_eq!(source, "mem_a");
                assert_eq!(target, "mem_b");
                assert_eq!(r#type, "supersedes");
            }
            _ => panic!("expected Relate"),
        }
    }

    #[test]
    fn unrelate_three_args() {
        let cli =
            Cli::try_parse_from(["nous", "unrelate", "mem_a", "mem_b", "supersedes"]).unwrap();
        match cli.command {
            Command::Unrelate {
                source,
                target,
                r#type,
            } => {
                assert_eq!(source, "mem_a");
                assert_eq!(target, "mem_b");
                assert_eq!(r#type, "supersedes");
            }
            _ => panic!("expected Unrelate"),
        }
    }

    #[test]
    fn search_minimal() {
        let cli = Cli::try_parse_from(["nous", "search", "pool size"]).unwrap();
        match cli.command {
            Command::Search {
                query,
                mode,
                r#type,
                limit,
                archived,
                ..
            } => {
                assert_eq!(query, "pool size");
                assert!(matches!(mode, SearchMode::Hybrid));
                assert!(r#type.is_none());
                assert_eq!(limit, 20);
                assert!(!archived);
            }
            _ => panic!("expected Search"),
        }
    }

    #[test]
    fn search_with_filters() {
        let cli = Cli::try_parse_from([
            "nous",
            "search",
            "db pool",
            "--mode",
            "fts",
            "--type",
            "decision",
            "--limit",
            "10",
            "--importance",
            "high",
        ])
        .unwrap();
        match cli.command {
            Command::Search {
                query,
                mode,
                r#type,
                limit,
                importance,
                ..
            } => {
                assert_eq!(query, "db pool");
                assert!(matches!(mode, SearchMode::Fts));
                assert_eq!(r#type.as_deref(), Some("decision"));
                assert_eq!(limit, 10);
                assert_eq!(importance.as_deref(), Some("high"));
            }
            _ => panic!("expected Search"),
        }
    }

    #[test]
    fn store_missing_title_errors() {
        let result = Cli::try_parse_from(["nous", "store", "--content", "Body"]);
        assert!(result.is_err());
    }

    #[test]
    fn store_missing_content_errors() {
        let result = Cli::try_parse_from(["nous", "store", "--title", "T"]);
        assert!(result.is_err());
    }

    #[test]
    fn recall_missing_id_errors() {
        let result = Cli::try_parse_from(["nous", "recall"]);
        assert!(result.is_err());
    }

    #[test]
    fn relate_missing_type_errors() {
        let result = Cli::try_parse_from(["nous", "relate", "a", "b"]);
        assert!(result.is_err());
    }

    #[test]
    fn search_missing_query_errors() {
        let result = Cli::try_parse_from(["nous", "search"]);
        assert!(result.is_err());
    }

    // --- Query/Inspection clap parsing tests ---

    #[test]
    fn context_with_workspace() {
        let cli = Cli::try_parse_from(["nous", "context", "/home/user/project"]).unwrap();
        match cli.command {
            Command::Context {
                workspace, summary, ..
            } => {
                assert_eq!(workspace, "/home/user/project");
                assert!(summary.is_none());
            }
            _ => panic!("expected Context"),
        }
    }

    #[test]
    fn context_with_summary() {
        let cli = Cli::try_parse_from([
            "nous",
            "context",
            "/home/user/project",
            "--summary",
            "auth flow",
        ])
        .unwrap();
        match cli.command {
            Command::Context {
                workspace, summary, ..
            } => {
                assert_eq!(workspace, "/home/user/project");
                assert_eq!(summary.as_deref(), Some("auth flow"));
            }
            _ => panic!("expected Context"),
        }
    }

    #[test]
    fn context_missing_workspace_errors() {
        let result = Cli::try_parse_from(["nous", "context"]);
        assert!(result.is_err());
    }

    #[test]
    fn sql_with_query() {
        let cli = Cli::try_parse_from(["nous", "sql", "SELECT COUNT(*) FROM memories"]).unwrap();
        match cli.command {
            Command::Sql { query } => {
                assert_eq!(query, "SELECT COUNT(*) FROM memories");
            }
            _ => panic!("expected Sql"),
        }
    }

    #[test]
    fn sql_missing_query_errors() {
        let result = Cli::try_parse_from(["nous", "sql"]);
        assert!(result.is_err());
    }

    #[test]
    fn schema_command() {
        let cli = Cli::try_parse_from(["nous", "schema"]).unwrap();
        assert!(matches!(cli.command, Command::Schema));
    }

    #[test]
    fn workspaces_command() {
        let cli = Cli::try_parse_from(["nous", "workspaces"]).unwrap();
        assert!(matches!(cli.command, Command::Workspaces));
    }

    #[test]
    fn tags_command() {
        let cli = Cli::try_parse_from(["nous", "tags"]).unwrap();
        assert!(matches!(cli.command, Command::Tags));
    }

    #[test]
    fn workspaces_with_format() {
        let cli = Cli::try_parse_from(["nous", "--format", "csv", "workspaces"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Csv));
        assert!(matches!(cli.command, Command::Workspaces));
    }

    #[test]
    fn tags_with_format() {
        let cli = Cli::try_parse_from(["nous", "--format", "json", "tags"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Json));
        assert!(matches!(cli.command, Command::Tags));
    }

    #[test]
    fn sql_with_format() {
        let cli = Cli::try_parse_from(["nous", "--format", "csv", "sql", "SELECT 1"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Csv));
        match cli.command {
            Command::Sql { query } => {
                assert_eq!(query, "SELECT 1");
            }
            _ => panic!("expected Sql"),
        }
    }

    // --- Integration tests ---

    fn make_test_config() -> config::Config {
        let mut cfg = config::Config::default();
        let db_path = test_db_path();
        cfg.encryption.db_key_file = format!("{db_path}.key");
        cfg.memory.db_path = db_path;
        cfg
    }

    #[test]
    fn integration_store_and_recall() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "Test memory",
            "Integration test content",
            "decision",
            Some("cli"),
            Some("high"),
            Some("moderate"),
            &["test".to_string(), "integration".to_string()],
            Some("/tmp/test"),
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let conn = db.connection();
        let id: String = conn
            .query_row(
                "SELECT id FROM memories WHERE title = 'Test memory'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        commands::run_recall(&cfg, &id, &format).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_store_update_recall() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "Original",
            "Original content",
            "fact",
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let id: String = db
            .connection()
            .query_row(
                "SELECT id FROM memories WHERE title = 'Original'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        commands::run_update(
            &cfg,
            &id,
            Some("Updated title"),
            None,
            Some("high"),
            None,
            Some(&["new-tag".to_string()]),
            None,
            &format,
        )
        .unwrap();

        let recalled = db
            .recall(&id.parse().unwrap())
            .unwrap()
            .expect("memory should exist");
        assert_eq!(recalled.memory.title, "Updated title");
        assert_eq!(
            recalled.memory.importance,
            nous_core::types::Importance::High
        );
        assert!(recalled.tags.contains(&"new-tag".to_string()));

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_forget_and_unarchive() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "To archive",
            "Will be archived",
            "observation",
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let id: String = db
            .connection()
            .query_row(
                "SELECT id FROM memories WHERE title = 'To archive'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        commands::run_forget(&cfg, &id, false, &format).unwrap();

        let archived: bool = db
            .connection()
            .query_row(
                "SELECT archived FROM memories WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, i64>(0).map(|v| v != 0),
            )
            .unwrap();
        assert!(archived);

        commands::run_unarchive(&cfg, &id, &format).unwrap();

        let archived_after: bool = db
            .connection()
            .query_row(
                "SELECT archived FROM memories WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get::<_, i64>(0).map(|v| v != 0),
            )
            .unwrap();
        assert!(!archived_after);

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_forget_hard_deletes() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "To delete",
            "Will be hard deleted",
            "observation",
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let id: String = db
            .connection()
            .query_row(
                "SELECT id FROM memories WHERE title = 'To delete'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        commands::run_forget(&cfg, &id, true, &format).unwrap();

        let exists: bool = db
            .connection()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1)",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists);

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_relate_and_unrelate() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "Source",
            "Source content",
            "decision",
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();
        commands::run_store(
            &cfg,
            "Target",
            "Target content",
            "decision",
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let src_id: String = db
            .connection()
            .query_row(
                "SELECT id FROM memories WHERE title = 'Source'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let tgt_id: String = db
            .connection()
            .query_row(
                "SELECT id FROM memories WHERE title = 'Target'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        commands::run_relate(&cfg, &src_id, &tgt_id, "supersedes", &format).unwrap();

        let rel_count: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rel_count, 1);

        commands::run_unrelate(&cfg, &src_id, &tgt_id, "supersedes", &format).unwrap();

        let rel_count_after: i64 = db
            .connection()
            .query_row("SELECT COUNT(*) FROM relationships", [], |row| row.get(0))
            .unwrap();
        assert_eq!(rel_count_after, 0);

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_search_after_store() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "Database pool increase",
            "Increased database connection pool from 10 to 50",
            "decision",
            None,
            Some("high"),
            None,
            &["database".to_string()],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        commands::run_search(
            &cfg,
            "database pool",
            "fts",
            Some("decision"),
            None,
            None,
            None,
            None,
            false,
            None,
            None,
            false,
            10,
            &format,
        )
        .unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_not_found_exits_3() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        let result = commands::run_recall(&cfg, "mem_nonexistent_id_12345", &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 3);
        } else {
            panic!("expected NousError::NotFound");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    // --- Query/Inspection integration tests ---

    #[test]
    fn integration_sql_select() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "SQL test memory",
            "Content for SQL test",
            "fact",
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        commands::run_sql(&cfg, "SELECT COUNT(*) as cnt FROM memories", &format).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_sql_insert_rejected() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        let result = commands::run_sql(
            &cfg,
            "INSERT INTO memories (id, title) VALUES ('x', 'y')",
            &format,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 2);
        } else {
            panic!("expected NousError::Validation (exit 2)");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_sql_drop_rejected() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        let result = commands::run_sql(&cfg, "DROP TABLE memories", &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 2);
        } else {
            panic!("expected NousError::Validation (exit 2)");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_sql_delete_rejected() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        let result = commands::run_sql(&cfg, "DELETE FROM memories", &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 2);
        } else {
            panic!("expected NousError::Validation (exit 2)");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_sql_update_rejected() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        let result = commands::run_sql(&cfg, "UPDATE memories SET title = 'x'", &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 2);
        } else {
            panic!("expected NousError::Validation (exit 2)");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_sql_nested_insert_in_select_rejected() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        let result = commands::run_sql(
            &cfg,
            "SELECT * FROM (INSERT INTO memories VALUES ('x','y','z') RETURNING *)",
            &format,
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 2);
        } else {
            panic!("expected NousError::Validation (exit 2)");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_schema_dumps_ddl() {
        let cfg = make_test_config();

        // Initialize DB by opening it
        let db_key = cfg.resolve_db_key().ok();
        let _db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();

        commands::run_schema(&cfg).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_workspaces_with_counts() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "WS test",
            "Content",
            "fact",
            None,
            None,
            None,
            &[],
            Some("/tmp/test-ws"),
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        commands::run_workspaces(&cfg, &format).unwrap();
        commands::run_workspaces(&cfg, &OutputFormat::Csv).unwrap();
        commands::run_workspaces(&cfg, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_tags_with_counts() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "Tag test",
            "Content",
            "fact",
            None,
            None,
            None,
            &["alpha".to_string(), "beta".to_string()],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        commands::run_tags(&cfg, &format).unwrap();
        commands::run_tags(&cfg, &OutputFormat::Csv).unwrap();
        commands::run_tags(&cfg, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_context_with_workspace() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_store(
            &cfg,
            "Context test memory",
            "OAuth2 flow implemented",
            "fact",
            None,
            Some("high"),
            None,
            &[],
            Some("/tmp/ctx-test"),
            None,
            None,
            None,
            None,
            None,
            None,
            &format,
        )
        .unwrap();

        commands::run_context(&cfg, "/tmp/ctx-test", None, &format).unwrap();
        commands::run_context(&cfg, "/tmp/ctx-test", Some("auth"), &OutputFormat::Csv).unwrap();
        commands::run_context(&cfg, "/tmp/ctx-test", None, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_context_unknown_workspace() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        // Initialize DB
        let db_key = cfg.resolve_db_key().ok();
        let _db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();

        let result = commands::run_context(&cfg, "/nonexistent/workspace", None, &format);
        assert!(result.is_err());

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    // --- Model/Embedding clap parsing tests ---

    #[test]
    fn model_list() {
        let cli = Cli::try_parse_from(["nous", "model", "list"]).unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::List,
            }) => {}
            _ => panic!("expected Model List"),
        }
    }

    #[test]
    fn model_info() {
        let cli = Cli::try_parse_from(["nous", "model", "info", "1"]).unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::Info { id },
            }) => {
                assert_eq!(id, 1);
            }
            _ => panic!("expected Model Info"),
        }
    }

    #[test]
    fn model_info_missing_id_errors() {
        let result = Cli::try_parse_from(["nous", "model", "info"]);
        assert!(result.is_err());
    }

    #[test]
    fn model_register_all_flags() {
        let cli = Cli::try_parse_from([
            "nous",
            "model",
            "register",
            "--name",
            "BAAI/bge-small-en-v1.5",
            "--variant",
            "onnx/model.onnx",
            "--dimensions",
            "384",
            "--chunk-size",
            "256",
            "--chunk-overlap",
            "32",
        ])
        .unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command:
                    ModelSubcommand::Register {
                        name,
                        variant,
                        dimensions,
                        chunk_size,
                        chunk_overlap,
                    },
            }) => {
                assert_eq!(name, "BAAI/bge-small-en-v1.5");
                assert_eq!(variant, "onnx/model.onnx");
                assert_eq!(dimensions, 384);
                assert_eq!(chunk_size, 256);
                assert_eq!(chunk_overlap, 32);
            }
            _ => panic!("expected Model Register"),
        }
    }

    #[test]
    fn model_register_defaults() {
        let cli = Cli::try_parse_from([
            "nous",
            "model",
            "register",
            "--name",
            "test-model",
            "--variant",
            "v1",
            "--dimensions",
            "768",
        ])
        .unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command:
                    ModelSubcommand::Register {
                        chunk_size,
                        chunk_overlap,
                        ..
                    },
            }) => {
                assert_eq!(chunk_size, 512);
                assert_eq!(chunk_overlap, 64);
            }
            _ => panic!("expected Model Register"),
        }
    }

    #[test]
    fn model_register_missing_name_errors() {
        let result = Cli::try_parse_from([
            "nous",
            "model",
            "register",
            "--variant",
            "v1",
            "--dimensions",
            "384",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn model_register_missing_variant_errors() {
        let result = Cli::try_parse_from([
            "nous",
            "model",
            "register",
            "--name",
            "m",
            "--dimensions",
            "384",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn model_register_missing_dimensions_errors() {
        let result = Cli::try_parse_from([
            "nous",
            "model",
            "register",
            "--name",
            "m",
            "--variant",
            "v1",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn model_activate() {
        let cli = Cli::try_parse_from(["nous", "model", "activate", "2"]).unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::Activate { id },
            }) => {
                assert_eq!(id, 2);
            }
            _ => panic!("expected Model Activate"),
        }
    }

    #[test]
    fn model_activate_missing_id_errors() {
        let result = Cli::try_parse_from(["nous", "model", "activate"]);
        assert!(result.is_err());
    }

    #[test]
    fn model_deactivate() {
        let cli = Cli::try_parse_from(["nous", "model", "deactivate", "3"]).unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::Deactivate { id },
            }) => {
                assert_eq!(id, 3);
            }
            _ => panic!("expected Model Deactivate"),
        }
    }

    #[test]
    fn model_deactivate_missing_id_errors() {
        let result = Cli::try_parse_from(["nous", "model", "deactivate"]);
        assert!(result.is_err());
    }

    #[test]
    fn model_switch_no_force() {
        let cli = Cli::try_parse_from(["nous", "model", "switch", "5"]).unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::Switch { id, force },
            }) => {
                assert_eq!(id, 5);
                assert!(!force);
            }
            _ => panic!("expected Model Switch"),
        }
    }

    #[test]
    fn model_switch_with_force() {
        let cli = Cli::try_parse_from(["nous", "model", "switch", "5", "--force"]).unwrap();
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::Switch { id, force },
            }) => {
                assert_eq!(id, 5);
                assert!(force);
            }
            _ => panic!("expected Model Switch"),
        }
    }

    #[test]
    fn model_switch_missing_id_errors() {
        let result = Cli::try_parse_from(["nous", "model", "switch"]);
        assert!(result.is_err());
    }

    #[test]
    fn embedding_inspect() {
        let cli = Cli::try_parse_from(["nous", "embedding", "inspect"]).unwrap();
        match cli.command {
            Command::Embedding(EmbeddingCmd {
                command: EmbeddingSubcommand::Inspect,
            }) => {}
            _ => panic!("expected Embedding Inspect"),
        }
    }

    #[test]
    fn embedding_reset_with_force() {
        let cli = Cli::try_parse_from(["nous", "embedding", "reset", "--force"]).unwrap();
        match cli.command {
            Command::Embedding(EmbeddingCmd {
                command: EmbeddingSubcommand::Reset { force },
            }) => {
                assert!(force);
            }
            _ => panic!("expected Embedding Reset"),
        }
    }

    #[test]
    fn embedding_reset_without_force() {
        let cli = Cli::try_parse_from(["nous", "embedding", "reset"]).unwrap();
        match cli.command {
            Command::Embedding(EmbeddingCmd {
                command: EmbeddingSubcommand::Reset { force },
            }) => {
                assert!(!force);
            }
            _ => panic!("expected Embedding Reset"),
        }
    }

    #[test]
    fn model_list_with_format() {
        let cli = Cli::try_parse_from(["nous", "--format", "json", "model", "list"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Json));
        match cli.command {
            Command::Model(ModelCmd {
                command: ModelSubcommand::List,
            }) => {}
            _ => panic!("expected Model List"),
        }
    }

    #[test]
    fn embedding_inspect_with_format() {
        let cli = Cli::try_parse_from(["nous", "--format", "csv", "embedding", "inspect"]).unwrap();
        assert!(matches!(cli.format, OutputFormat::Csv));
        match cli.command {
            Command::Embedding(EmbeddingCmd {
                command: EmbeddingSubcommand::Inspect,
            }) => {}
            _ => panic!("expected Embedding Inspect"),
        }
    }

    // --- Model/Embedding integration tests ---

    #[test]
    fn integration_model_register_and_list() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(
            &cfg,
            "BAAI/bge-small-en-v1.5",
            "onnx/model.onnx",
            384,
            512,
            64,
            &format,
        )
        .unwrap();

        commands::run_model_list(&cfg, &format).unwrap();
        commands::run_model_list(&cfg, &OutputFormat::Csv).unwrap();
        commands::run_model_list(&cfg, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_model_info() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(&cfg, "test-model", "v1", 384, 512, 64, &format).unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let model_id = models.iter().find(|m| m.name == "test-model").unwrap().id;

        commands::run_model_info(&cfg, model_id, &format).unwrap();
        commands::run_model_info(&cfg, model_id, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_model_info_not_found() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        // Initialize DB
        let db_key = cfg.resolve_db_key().ok();
        let _db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();

        let result = commands::run_model_info(&cfg, 99999, &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 3);
        } else {
            panic!("expected NousError::NotFound");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_model_activate_deactivate() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(&cfg, "test-model", "v1", 384, 512, 64, &format).unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let model_id = models.iter().find(|m| m.name == "test-model").unwrap().id;

        commands::run_model_activate(&cfg, model_id, &format).unwrap();

        let model = db.get_model(model_id).unwrap();
        assert!(model.active);

        commands::run_model_deactivate(&cfg, model_id, &format).unwrap();

        let model = db.get_model(model_id).unwrap();
        assert!(!model.active);

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_model_switch_same_dims() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(&cfg, "model-a", "v1", 384, 512, 64, &format).unwrap();
        commands::run_model_register(&cfg, "model-b", "v1", 384, 512, 64, &format).unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let id_a = models.iter().find(|m| m.name == "model-a").unwrap().id;
        let id_b = models.iter().find(|m| m.name == "model-b").unwrap().id;

        commands::run_model_activate(&cfg, id_a, &format).unwrap();
        commands::run_model_switch(&cfg, id_b, true, &format).unwrap();

        let model = db.get_model(id_b).unwrap();
        assert!(model.active);
        let model_a = db.get_model(id_a).unwrap();
        assert!(!model_a.active);

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_model_switch_diff_dims_force() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(&cfg, "small", "v1", 384, 512, 64, &format).unwrap();
        commands::run_model_register(&cfg, "large", "v1", 768, 512, 64, &format).unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let id_small = models.iter().find(|m| m.name == "small").unwrap().id;
        let id_large = models.iter().find(|m| m.name == "large").unwrap().id;

        commands::run_model_activate(&cfg, id_small, &format).unwrap();
        commands::run_model_switch(&cfg, id_large, true, &format).unwrap();

        let model = db.get_model(id_large).unwrap();
        assert!(model.active);

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_embedding_inspect() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(&cfg, "test-model", "v1", 384, 512, 64, &format).unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let model_id = models.iter().find(|m| m.name == "test-model").unwrap().id;
        commands::run_model_activate(&cfg, model_id, &format).unwrap();

        commands::run_embedding_inspect(&cfg, &format).unwrap();
        commands::run_embedding_inspect(&cfg, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_embedding_reset_requires_force() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        // Initialize DB
        let db_key = cfg.resolve_db_key().ok();
        let _db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();

        let result = commands::run_embedding_reset(&cfg, false, &format);
        assert!(result.is_err());
        let err = result.unwrap_err();
        if let Some(ne) = err.downcast_ref::<nous_shared::NousError>() {
            assert_eq!(ne.exit_code(), 2);
        } else {
            panic!("expected NousError::Validation");
        }

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_embedding_reset_force() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(&cfg, "test-model", "v1", 384, 512, 64, &format).unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let model_id = models.iter().find(|m| m.name == "test-model").unwrap().id;
        commands::run_model_activate(&cfg, model_id, &format).unwrap();

        commands::run_embedding_reset(&cfg, true, &format).unwrap();
        commands::run_embedding_reset(&cfg, true, &OutputFormat::Human).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }

    #[test]
    fn integration_full_model_lifecycle() {
        let cfg = make_test_config();
        let format = OutputFormat::Json;

        commands::run_model_register(
            &cfg,
            "BAAI/bge-small-en-v1.5",
            "onnx/model.onnx",
            384,
            512,
            64,
            &format,
        )
        .unwrap();
        commands::run_model_register(
            &cfg,
            "BAAI/bge-base-en-v1.5",
            "onnx/model.onnx",
            768,
            512,
            64,
            &format,
        )
        .unwrap();

        let db_key = cfg.resolve_db_key().ok();
        let db =
            nous_core::db::MemoryDb::open(&cfg.memory.db_path, db_key.as_deref(), 384).unwrap();
        let models = db.list_models().unwrap();
        let id_small = models
            .iter()
            .find(|m| m.name == "BAAI/bge-small-en-v1.5")
            .unwrap()
            .id;
        let id_large = models
            .iter()
            .find(|m| m.name == "BAAI/bge-base-en-v1.5")
            .unwrap()
            .id;

        commands::run_model_activate(&cfg, id_small, &format).unwrap();
        commands::run_model_list(&cfg, &format).unwrap();
        commands::run_model_info(&cfg, id_small, &format).unwrap();
        commands::run_embedding_inspect(&cfg, &format).unwrap();

        commands::run_model_switch(&cfg, id_large, true, &format).unwrap();

        let active = db.active_model().unwrap().unwrap();
        assert_eq!(active.id, id_large);
        assert_eq!(active.dimensions, 768);

        commands::run_embedding_inspect(&cfg, &format).unwrap();
        commands::run_embedding_reset(&cfg, true, &format).unwrap();

        let _ = std::fs::remove_file(&cfg.memory.db_path);
    }
}
