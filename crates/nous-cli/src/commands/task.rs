use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::tasks;

#[derive(Subcommand)]
pub enum TaskCommands {
    /// Create a new task
    Create {
        /// Task title
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, default_value = "medium")]
        priority: String,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long = "label", action = clap::ArgAction::Append)]
        labels: Vec<String>,
        #[arg(long)]
        room_id: Option<String>,
        #[arg(long)]
        create_room: bool,
    },
    /// List tasks with optional filters
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
        #[arg(long, default_value = "0")]
        offset: u32,
    },
    /// Get task details by ID
    Get {
        /// Task ID
        id: String,
    },
    /// Update a task
    Update {
        /// Task ID
        id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        priority: Option<String>,
        #[arg(long)]
        assignee: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Close a task
    Close {
        /// Task ID
        id: String,
    },
    /// Link two tasks
    Link {
        /// Source task ID
        id: String,
        #[arg(long)]
        blocks: Option<String>,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long = "related-to")]
        related_to: Option<String>,
    },
    /// List links for a task
    Links {
        /// Task ID
        id: String,
    },
    /// Add a note to a task (posts to linked room)
    Note {
        /// Task ID
        id: String,
        /// Note content
        content: String,
    },
    /// View task event history
    History {
        /// Task ID
        id: String,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Search tasks using full-text search
    Search {
        /// Search query
        query: String,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
}

pub async fn run(cmd: TaskCommands) {
    if let Err(e) = execute(cmd).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: TaskCommands) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;

    match cmd {
        TaskCommands::Create {
            title,
            description,
            priority,
            assignee,
            labels,
            room_id,
            create_room,
        } => {
            let label_refs = if labels.is_empty() {
                None
            } else {
                Some(labels.as_slice())
            };
            let task = tasks::create_task(
                pool,
                &title,
                description.as_deref(),
                Some(priority.as_str()),
                assignee.as_deref(),
                label_refs,
                room_id.as_deref(),
                create_room,
                None,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        TaskCommands::List {
            status,
            assignee,
            label,
            limit,
            offset,
        } => {
            let tasks = tasks::list_tasks(
                pool,
                status.as_deref(),
                assignee.as_deref(),
                label.as_deref(),
                Some(limit),
                Some(offset),
                None,
                None,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&tasks)?);
        }
        TaskCommands::Get { id } => {
            let task = tasks::get_task(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        TaskCommands::Update {
            id,
            status,
            priority,
            assignee,
            description,
        } => {
            let task = tasks::update_task(
                pool,
                &id,
                status.as_deref(),
                priority.as_deref(),
                assignee.as_deref(),
                description.as_deref(),
                None,
                None,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        TaskCommands::Close { id } => {
            let task = tasks::close_task(pool, &id, None).await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        TaskCommands::Link {
            id,
            blocks,
            parent,
            related_to,
        } => {
            let (link_type, target) = if let Some(ref t) = blocks {
                ("blocked_by", t.as_str())
            } else if let Some(ref t) = parent {
                ("parent", t.as_str())
            } else if let Some(ref t) = related_to {
                ("related_to", t.as_str())
            } else {
                return Err("must specify one of --blocks, --parent, or --related-to".into());
            };
            let link = tasks::link_tasks(pool, &id, target, link_type, None).await?;
            println!("{}", serde_json::to_string_pretty(&link)?);
        }
        TaskCommands::Links { id } => {
            let links = tasks::list_links(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&links)?);
        }
        TaskCommands::Note { id, content } => {
            let note = tasks::add_note(pool, &id, "cli", &content).await?;
            println!("{}", serde_json::to_string_pretty(&note)?);
        }
        TaskCommands::History { id, limit } => {
            let events = tasks::task_history(pool, &id, Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&events)?);
        }
        TaskCommands::Search { query, limit } => {
            let results = tasks::search_tasks(pool, &query, Some(limit)).await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
    }

    pools.close().await;
    Ok(())
}
