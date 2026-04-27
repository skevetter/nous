mod commands;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use nous_mcp::config;
use nous_mcp::server::NousServer;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

#[derive(Debug, Clone, ValueEnum)]
enum Transport {
    Stdio,
    Http,
}

#[derive(Debug, Parser)]
#[command(name = "nous-mcp", about = "Nous MCP server and management CLI")]
struct Cli {
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
    Export {
        #[arg(long, default_value = "json")]
        format: String,
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
    Show {
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

fn main() {
    let cli = Cli::parse();

    let config = config::Config::load(None).unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load config: {e}");
        config::Config::default()
    });

    let _db_key = config.resolve_db_key().ok();

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
            rt.block_on(run_serve(config, transport, port, &model, &variant))
                .expect("server error");
        }
        Command::ReEmbed { model, variant } => {
            let variant = variant.unwrap_or_else(|| config.embedding.variant.clone());
            let embedding = build_embedding(&model, &variant);
            commands::run_re_embed(&config, embedding.as_ref())
                .unwrap_or_else(|e| eprintln!("re-embed failed: {e}"));
        }
        Command::ReClassify { since } => {
            let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
            commands::run_re_classify(&config, since.as_deref(), embedding.as_ref())
                .unwrap_or_else(|e| eprintln!("re-classify failed: {e}"));
        }
        Command::Category(cat) => match cat.command {
            CategorySubcommand::List { source } => {
                commands::run_category_list(&config, source.as_deref())
                    .unwrap_or_else(|e| eprintln!("category list failed: {e}"));
            }
            CategorySubcommand::Add {
                name,
                parent,
                description,
            } => {
                let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
                commands::run_category_add(
                    &config,
                    &name,
                    parent.as_deref(),
                    description.as_deref(),
                    embedding.as_ref(),
                )
                .unwrap_or_else(|e| eprintln!("category add failed: {e}"));
            }
            CategorySubcommand::Delete { name } => {
                commands::run_category_delete(&config, &name)
                    .unwrap_or_else(|e| eprintln!("category delete failed: {e}"));
            }
            CategorySubcommand::Rename { old, new } => {
                let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
                commands::run_category_rename(&config, &old, &new, embedding.as_ref())
                    .unwrap_or_else(|e| eprintln!("category rename failed: {e}"));
            }
            CategorySubcommand::Update {
                name,
                new_name,
                description,
                threshold,
            } => {
                let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
                commands::run_category_update(
                    &config,
                    &name,
                    new_name.as_deref(),
                    description.as_deref(),
                    threshold,
                    embedding.as_ref(),
                )
                .unwrap_or_else(|e| eprintln!("category update failed: {e}"));
            }
        },
        Command::Room(room) => match room.command {
            RoomSubcommand::Create { name, purpose } => {
                commands::run_room_create(&config, &name, purpose.as_deref())
                    .unwrap_or_else(|e| eprintln!("room create failed: {e}"));
            }
            RoomSubcommand::List { archived, limit } => {
                commands::run_room_list(&config, archived, limit)
                    .unwrap_or_else(|e| eprintln!("room list failed: {e}"));
            }
            RoomSubcommand::Show { id } => {
                commands::run_room_show(&config, &id)
                    .unwrap_or_else(|e| eprintln!("room show failed: {e}"));
            }
            RoomSubcommand::Post {
                room,
                content,
                sender,
                reply_to,
            } => {
                commands::run_room_post(
                    &config,
                    &room,
                    &content,
                    sender.as_deref(),
                    reply_to.as_deref(),
                )
                .unwrap_or_else(|e| eprintln!("room post failed: {e}"));
            }
            RoomSubcommand::Read { room, limit, since } => {
                commands::run_room_read(&config, &room, limit, since.as_deref())
                    .unwrap_or_else(|e| eprintln!("room read failed: {e}"));
            }
            RoomSubcommand::Search { room, query, limit } => {
                commands::run_room_search(&config, &room, &query, limit)
                    .unwrap_or_else(|e| eprintln!("room search failed: {e}"));
            }
            RoomSubcommand::Delete { id, hard } => {
                commands::run_room_delete(&config, &id, hard)
                    .unwrap_or_else(|e| eprintln!("room delete failed: {e}"));
            }
        },
        Command::Export { format: _ } => {
            commands::run_export(&config).unwrap_or_else(|e| eprintln!("export failed: {e}"));
        }
        Command::Import { file } => {
            let embedding = build_embedding(&config.embedding.model, &config.embedding.variant);
            commands::run_import(&config, &file, embedding.as_ref())
                .unwrap_or_else(|e| eprintln!("import failed: {e}"));
        }
        Command::RotateKey { new_key_file } => {
            commands::run_rotate_key(&config, new_key_file.as_deref())
                .unwrap_or_else(|e| eprintln!("rotate-key failed: {e}"));
        }
        Command::Status => {
            commands::run_status(&config).unwrap_or_else(|e| eprintln!("status failed: {e}"));
        }
        Command::Trace {
            trace_id,
            memory_id,
            session_id,
        } => {
            if let Err(e) = commands::run_trace(
                &config,
                trace_id.as_deref(),
                memory_id.as_deref(),
                session_id.as_deref(),
            ) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
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
        let cli = Cli::try_parse_from(["nous-mcp", "serve"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["nous-mcp", "serve", "--transport", "http", "--port", "9000"])
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
        let cli = Cli::try_parse_from(["nous-mcp", "re-embed", "--model", "org/repo"]).unwrap();
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
        let cli = Cli::try_parse_from(["nous-mcp", "re-classify"]).unwrap();
        match cli.command {
            Command::ReClassify { since } => assert!(since.is_none()),
            _ => panic!("expected ReClassify"),
        }
    }

    #[test]
    fn re_classify_with_since() {
        let cli =
            Cli::try_parse_from(["nous-mcp", "re-classify", "--since", "2024-01-01"]).unwrap();
        match cli.command {
            Command::ReClassify { since } => {
                assert_eq!(since.as_deref(), Some("2024-01-01"));
            }
            _ => panic!("expected ReClassify"),
        }
    }

    #[test]
    fn category_add() {
        let cli = Cli::try_parse_from(["nous-mcp", "category", "add", "testing"]).unwrap();
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
        let cli = Cli::try_parse_from(["nous-mcp", "category", "list"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["nous-mcp", "category", "list", "--source", "manual"]).unwrap();
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
        let cli = Cli::try_parse_from(["nous-mcp", "export"]).unwrap();
        match cli.command {
            Command::Export { format } => assert_eq!(format, "json"),
            _ => panic!("expected Export"),
        }
    }

    #[test]
    fn import_file() {
        let cli = Cli::try_parse_from(["nous-mcp", "import", "/tmp/data.json"]).unwrap();
        match cli.command {
            Command::Import { file } => {
                assert_eq!(file, PathBuf::from("/tmp/data.json"));
            }
            _ => panic!("expected Import"),
        }
    }

    #[test]
    fn rotate_key_no_file() {
        let cli = Cli::try_parse_from(["nous-mcp", "rotate-key"]).unwrap();
        match cli.command {
            Command::RotateKey { new_key_file } => assert!(new_key_file.is_none()),
            _ => panic!("expected RotateKey"),
        }
    }

    #[test]
    fn rotate_key_with_file() {
        let cli = Cli::try_parse_from(["nous-mcp", "rotate-key", "--new-key-file", "/tmp/key.bin"])
            .unwrap();
        match cli.command {
            Command::RotateKey { new_key_file } => {
                assert_eq!(new_key_file, Some(PathBuf::from("/tmp/key.bin")));
            }
            _ => panic!("expected RotateKey"),
        }
    }

    #[test]
    fn status_command() {
        let cli = Cli::try_parse_from(["nous-mcp", "status"]).unwrap();
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn trace_with_trace_id() {
        let cli = Cli::try_parse_from(["nous-mcp", "trace", "--trace-id", "abc123"]).unwrap();
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
        let cli = Cli::try_parse_from(["nous-mcp", "trace", "--memory-id", "mem-789"]).unwrap();
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
        let result =
            Cli::try_parse_from(["nous-mcp", "trace", "--trace-id", "a", "--memory-id", "b"]);
        assert!(result.is_err());
    }

    #[test]
    fn trace_session_id_requires_trace_id() {
        let result = Cli::try_parse_from(["nous-mcp", "trace", "--session-id", "s"]);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_subcommand_errors() {
        let result = Cli::try_parse_from(["nous-mcp", "nonexistent"]);
        assert!(result.is_err());
    }

    #[test]
    fn re_embed_missing_model_errors() {
        let result = Cli::try_parse_from(["nous-mcp", "re-embed"]);
        assert!(result.is_err());
    }

    #[test]
    fn import_missing_file_errors() {
        let result = Cli::try_parse_from(["nous-mcp", "import"]);
        assert!(result.is_err());
    }

    #[test]
    fn room_create() {
        let cli = Cli::try_parse_from(["nous-mcp", "room", "create", "test-room"]).unwrap();
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
        let cli = Cli::try_parse_from(["nous-mcp", "room", "list"]).unwrap();
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
    fn room_show() {
        let cli = Cli::try_parse_from(["nous-mcp", "room", "show", "my-room"]).unwrap();
        match cli.command {
            Command::Room(RoomCmd {
                command: RoomSubcommand::Show { id },
            }) => {
                assert_eq!(id, "my-room");
            }
            _ => panic!("expected Room Show"),
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
            Cli::try_parse_from(["nous-mcp", "room", "read", "dev-chat", "--limit", "10"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["nous-mcp", "room", "search", "dev-chat", "linter"]).unwrap();
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
        let cli =
            Cli::try_parse_from(["nous-mcp", "room", "delete", "old-room", "--hard"]).unwrap();
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
        let cfg = config::Config::default();
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
        let cfg = config::Config::default();
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
}
