use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::tasks;

use super::output::{OutputFormat, print_list};

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
        /// Output format: json (default), table, csv
        #[arg(short, long, default_value = "json")]
        output: OutputFormat,
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
    /// Manage task dependencies
    Depends {
        #[command(subcommand)]
        command: DependsCommands,
    },
    /// Manage task templates
    Template {
        #[command(subcommand)]
        command: TemplateCommands,
    },
}

#[derive(Subcommand)]
pub enum DependsCommands {
    /// Add a dependency
    Add {
        /// Task ID
        task_id: String,
        /// Depends on task ID
        depends_on: String,
        /// Dependency type: `blocked_by`, blocks, `waiting_on`
        #[arg(long, default_value = "blocked_by")]
        dep_type: String,
    },
    /// Remove a dependency
    Remove {
        /// Task ID
        task_id: String,
        /// Depends on task ID
        depends_on: String,
        /// Dependency type
        #[arg(long, default_value = "blocked_by")]
        dep_type: String,
    },
    /// List dependencies for a task
    List {
        /// Task ID
        task_id: String,
    },
}

#[derive(Subcommand)]
pub enum TemplateCommands {
    /// Create a task template
    Create {
        /// Template name (unique)
        name: String,
        /// Title pattern (use {{var}} for variables)
        title_pattern: String,
        /// Description template
        #[arg(long)]
        description: Option<String>,
        /// Default priority
        #[arg(long, default_value = "medium")]
        priority: String,
    },
    /// List templates
    List,
    /// Get a template
    Get {
        /// Template ID or name
        id: String,
    },
    /// Create a task from a template
    Use {
        /// Template ID or name
        template: String,
        /// Assignee agent ID
        #[arg(long)]
        assignee: Option<String>,
    },
}

pub async fn run(cmd: TaskCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: TaskCommands, port: Option<u16>) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
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
            let task = tasks::create_task(tasks::CreateTaskParams {
                db: pool,
                title: &title,
                description: description.as_deref(),
                priority: Some(priority.as_str()),
                assignee_id: assignee.as_deref(),
                labels: label_refs,
                room_id: room_id.as_deref(),
                create_room,
                actor_id: None,
                registry: None,
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        TaskCommands::List {
            status,
            assignee,
            label,
            limit,
            offset,
            output,
        } => {
            let tasks = tasks::list_tasks(tasks::ListTasksParams {
                db: pool,
                status: status.as_deref(),
                assignee_id: assignee.as_deref(),
                label: label.as_deref(),
                limit: Some(limit),
                offset: Some(offset),
                order_by: None,
                order_dir: None,
            })
            .await?;
            let val = serde_json::to_value(&tasks)?;
            print_list(&val, &output, &["id", "title", "status", "priority", "assignee_id", "created_at"]);
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
            let task = tasks::update_task(tasks::UpdateTaskParams {
                db: pool,
                id: &id,
                status: status.as_deref(),
                priority: priority.as_deref(),
                assignee_id: assignee.as_deref(),
                description: description.as_deref(),
                labels: None,
                actor_id: None,
                registry: None,
            })
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
        TaskCommands::Depends { command } => match command {
            DependsCommands::Add {
                task_id,
                depends_on,
                dep_type,
            } => {
                let dep =
                    tasks::add_dependency(pool, &task_id, &depends_on, Some(&dep_type)).await?;
                println!("{}", serde_json::to_string_pretty(&dep)?);
            }
            DependsCommands::Remove {
                task_id,
                depends_on,
                dep_type,
            } => {
                tasks::remove_dependency(pool, &task_id, &depends_on, Some(&dep_type)).await?;
                println!("Dependency removed");
            }
            DependsCommands::List { task_id } => {
                let deps = tasks::list_dependencies(pool, &task_id).await?;
                println!("{}", serde_json::to_string_pretty(&deps)?);
            }
        },
        TaskCommands::Template { command } => match command {
            TemplateCommands::Create {
                name,
                title_pattern,
                description,
                priority,
            } => {
                let tmpl = tasks::create_template(
                    pool,
                    &name,
                    &title_pattern,
                    description.as_deref(),
                    Some(&priority),
                    None,
                    None,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&tmpl)?);
            }
            TemplateCommands::List => {
                let templates = tasks::list_templates(pool, None).await?;
                println!("{}", serde_json::to_string_pretty(&templates)?);
            }
            TemplateCommands::Get { id } => {
                let tmpl = tasks::get_template(pool, &id).await?;
                println!("{}", serde_json::to_string_pretty(&tmpl)?);
            }
            TemplateCommands::Use { template, assignee } => {
                let task = tasks::create_from_template::<std::collections::hash_map::RandomState>(
                    pool,
                    &template,
                    None,
                    None,
                    assignee.as_deref(),
                    None,
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&task)?);
            }
        },
    }

    pools.close().await;
    Ok(())
}
