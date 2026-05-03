use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::notifications::{
    list_subscriptions, subscribe_to_room, unsubscribe_from_room, Notification,
    NotificationPriority, NotificationRegistry,
};

#[derive(Subcommand)]
pub enum NotificationCommands {
    /// List notification subscriptions for an agent
    List {
        /// Agent ID
        agent_id: String,
    },
    /// Subscribe an agent to a room's notifications
    Subscribe {
        /// Agent ID
        agent_id: String,
        /// Room ID
        room_id: String,
        /// Comma-separated topic filters (optional)
        #[arg(long)]
        topics: Option<String>,
    },
    /// Unsubscribe an agent from a room's notifications
    Unsubscribe {
        /// Agent ID
        agent_id: String,
        /// Room ID
        room_id: String,
    },
    /// Send a test notification to a room
    Test {
        /// Room ID
        room_id: String,
        /// Optional test message
        #[arg(long)]
        message: Option<String>,
    },
}

pub async fn run(cmd: NotificationCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(
    cmd: NotificationCommands,
    port: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations(&config.search.tokenizer).await?;
    let pool = &pools.fts;

    match cmd {
        NotificationCommands::List { agent_id } => {
            let subs = list_subscriptions(pool, &agent_id).await?;
            println!("{}", serde_json::to_string_pretty(&subs)?);
        }
        NotificationCommands::Subscribe {
            agent_id,
            room_id,
            topics,
        } => {
            let topics_vec = topics.map(|t| t.split(',').map(|s| s.trim().to_string()).collect());
            subscribe_to_room(pool, &room_id, &agent_id, topics_vec).await?;
            println!("Subscribed {agent_id} to {room_id}");
        }
        NotificationCommands::Unsubscribe { agent_id, room_id } => {
            unsubscribe_from_room(pool, &room_id, &agent_id).await?;
            println!("Unsubscribed {agent_id} from {room_id}");
        }
        NotificationCommands::Test { room_id, message } => {
            let msg = message.unwrap_or_else(|| "Test notification from CLI".to_string());
            let registry = NotificationRegistry::new();
            let notification = Notification {
                room_id: room_id.clone(),
                message_id: format!("test-{}", uuid::Uuid::now_v7()),
                sender_id: "cli".to_string(),
                priority: NotificationPriority::Normal,
                topics: vec!["test".to_string()],
                mentions: vec![],
            };
            registry.notify(notification).await;
            println!("Sent test notification to {room_id}: {msg}");
        }
    }

    pools.close().await;
    Ok(())
}
