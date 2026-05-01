use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::messages::{
    post_message, read_messages, search_messages, PostMessageRequest, ReadMessagesRequest,
    SearchMessagesRequest,
};
use nous_core::notifications::{room_wait, NotificationRegistry};
use nous_core::rooms::{create_room, delete_room, get_room, list_rooms};

#[derive(Subcommand)]
pub enum ChatCommands {
    /// Create a new chat room
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        purpose: Option<String>,
    },
    /// List chat rooms
    List {
        #[arg(long, default_value_t = false)]
        include_archived: bool,
    },
    /// Inspect a room (by ID or name)
    Inspect {
        /// Room ID or name
        room: String,
    },
    /// Delete a room
    Delete {
        /// Room ID or name
        room: String,
        #[arg(long, default_value_t = false)]
        hard: bool,
    },
    /// Post a message to a room
    Post {
        /// Room ID or name
        room: String,
        /// Sender agent ID
        #[arg(long)]
        sender: String,
        /// Message content
        #[arg(long)]
        content: String,
        /// Optional reply-to message ID
        #[arg(long)]
        reply_to: Option<String>,
    },
    /// Read messages from a room
    Read {
        /// Room ID or name
        room: String,
        #[arg(long, default_value_t = 50)]
        limit: u32,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        before: Option<String>,
    },
    /// Wait for new messages in a room
    Wait {
        /// Room ID or name
        room: String,
        #[arg(long, default_value_t = 30000)]
        timeout: u64,
    },
    /// Search messages using full-text search
    Search {
        /// Search query
        query: String,
        /// Optional room filter (ID or name)
        #[arg(long)]
        room: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
}

pub async fn run(cmd: ChatCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: ChatCommands, port: Option<u16>) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations(&config.search.tokenizer).await?;
    let pool = &pools.fts;

    match cmd {
        ChatCommands::Create { name, purpose } => {
            let room = create_room(pool, &name, purpose.as_deref(), None).await?;
            println!("{}", serde_json::to_string_pretty(&room)?);
        }
        ChatCommands::List { include_archived } => {
            let rooms = list_rooms(pool, include_archived).await?;
            println!("{}", serde_json::to_string_pretty(&rooms)?);
        }
        ChatCommands::Inspect { room } => {
            let r = get_room(pool, &room).await?;
            println!("{}", serde_json::to_string_pretty(&r)?);
        }
        ChatCommands::Delete { room, hard } => {
            let r = get_room(pool, &room).await?;
            delete_room(pool, &r.id, hard).await?;
            let status = if hard { "hard-deleted" } else { "archived" };
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "id": r.id,
                    "name": r.name,
                    "status": status,
                }))?
            );
        }
        ChatCommands::Post {
            room,
            sender,
            content,
            reply_to,
        } => {
            let r = get_room(pool, &room).await?;
            let msg = post_message(
                pool,
                PostMessageRequest {
                    room_id: r.id,
                    sender_id: sender,
                    content,
                    reply_to,
                    metadata: None,
                },
                None,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&msg)?);
        }
        ChatCommands::Read {
            room,
            limit,
            since,
            before,
        } => {
            let r = get_room(pool, &room).await?;
            let messages = read_messages(
                pool,
                ReadMessagesRequest {
                    room_id: r.id,
                    since,
                    before,
                    limit: Some(limit),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&messages)?);
        }
        ChatCommands::Wait { room, timeout } => {
            let r = get_room(pool, &room).await?;
            let registry = NotificationRegistry::new();
            let result = room_wait(&registry, &r.id, Some(timeout), None).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        ChatCommands::Search { query, room, limit } => {
            let room_id = match room {
                Some(ref name) => Some(get_room(pool, name).await?.id),
                None => None,
            };
            let results = search_messages(
                pool,
                SearchMessagesRequest {
                    query,
                    room_id,
                    limit: Some(limit),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
    }

    pools.close().await;
    Ok(())
}
