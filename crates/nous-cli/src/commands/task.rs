use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::tasks;

use super::output::{OutputFormat, parse_fields, print_list};

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
        /// Comma-separated fields to include (e.g. id,title,status)
        #[arg(long)]
        fields: Option<String>,
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

    dispatch(pool, cmd).await?;

    pools.close().await;
    Ok(())
}

async fn dispatch(
    pool: &sea_orm::DatabaseConnection,
    cmd: TaskCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        TaskCommands::Create { title, description, priority, assignee, labels, room_id, create_room } => {
            cmd_create(pool, CreateArgs { title, description, priority, assignee, labels, room_id, create_room }).await?;
        }
        TaskCommands::List { status, assignee, label, limit, offset, output, fields } => {
            cmd_list(pool, ListArgs { status, assignee, label, limit, offset, output, fields }).await?;
        }
        TaskCommands::Get { id } => cmd_get(pool, &id).await?,
        TaskCommands::Update { id, status, priority, assignee, description } => {
            cmd_update(pool, UpdateArgs { id, status, priority, assignee, description }).await?;
        }
        TaskCommands::Close { id } => cmd_close(pool, &id).await?,
        TaskCommands::Link { id, blocks, parent, related_to } => {
            cmd_link(pool, LinkArgs { id, blocks, parent, related_to }).await?;
        }
        cmd => dispatch_extended(pool, cmd).await?,
    }
    Ok(())
}

async fn dispatch_extended(
    pool: &sea_orm::DatabaseConnection,
    cmd: TaskCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        TaskCommands::Links { id } => cmd_links(pool, &id).await?,
        TaskCommands::Note { id, content } => cmd_note(pool, &id, &content).await?,
        TaskCommands::History { id, limit } => cmd_history(pool, &id, limit).await?,
        TaskCommands::Search { query, limit } => cmd_search(pool, &query, limit).await?,
        TaskCommands::Depends { command } => cmd_depends(pool, command).await?,
        TaskCommands::Template { command } => cmd_template(pool, command).await?,
        _ => unreachable!("all variants handled in dispatch"),
    }
    Ok(())
}

async fn cmd_get(
    pool: &sea_orm::DatabaseConnection,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let task = tasks::get_task(pool, id).await?;
    println!("{}", serde_json::to_string_pretty(&task)?);
    Ok(())
}

async fn cmd_close(
    pool: &sea_orm::DatabaseConnection,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let task = tasks::close_task(pool, id, None).await?;
    println!("{}", serde_json::to_string_pretty(&task)?);
    Ok(())
}

async fn cmd_links(
    pool: &sea_orm::DatabaseConnection,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let links = tasks::list_links(pool, id).await?;
    println!("{}", serde_json::to_string_pretty(&links)?);
    Ok(())
}

async fn cmd_note(
    pool: &sea_orm::DatabaseConnection,
    id: &str,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let note = tasks::add_note(pool, id, "cli", content).await?;
    println!("{}", serde_json::to_string_pretty(&note)?);
    Ok(())
}

async fn cmd_history(
    pool: &sea_orm::DatabaseConnection,
    id: &str,
    limit: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let events = tasks::task_history(pool, id, Some(limit)).await?;
    println!("{}", serde_json::to_string_pretty(&events)?);
    Ok(())
}

async fn cmd_search(
    pool: &sea_orm::DatabaseConnection,
    query: &str,
    limit: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let results = tasks::search_tasks(pool, query, Some(limit)).await?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

struct CreateArgs {
    title: String,
    description: Option<String>,
    priority: String,
    assignee: Option<String>,
    labels: Vec<String>,
    room_id: Option<String>,
    create_room: bool,
}

async fn cmd_create(
    pool: &sea_orm::DatabaseConnection,
    args: CreateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let label_refs = if args.labels.is_empty() {
        None
    } else {
        Some(args.labels.as_slice())
    };
    let task = tasks::create_task(tasks::CreateTaskParams {
        db: pool,
        title: &args.title,
        description: args.description.as_deref(),
        priority: Some(args.priority.as_str()),
        assignee_id: args.assignee.as_deref(),
        labels: label_refs,
        room_id: args.room_id.as_deref(),
        create_room: args.create_room,
        actor_id: None,
        registry: None,
    })
    .await?;
    println!("{}", serde_json::to_string_pretty(&task)?);
    Ok(())
}

struct ListArgs {
    status: Option<String>,
    assignee: Option<String>,
    label: Option<String>,
    limit: u32,
    offset: u32,
    output: OutputFormat,
    fields: Option<String>,
}

async fn cmd_list(
    pool: &sea_orm::DatabaseConnection,
    args: ListArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let tasks = tasks::list_tasks(tasks::ListTasksParams {
        db: pool,
        status: args.status.as_deref(),
        assignee_id: args.assignee.as_deref(),
        label: args.label.as_deref(),
        limit: Some(args.limit),
        offset: Some(args.offset),
        order_by: None,
        order_dir: None,
    })
    .await?;
    let val = serde_json::to_value(&tasks)?;
    let fields_override = args.fields.as_deref().map(parse_fields);
    print_list(&val, &args.output, &["id", "title", "status", "priority", "assignee_id", "created_at"], fields_override.as_deref());
    Ok(())
}

struct UpdateArgs {
    id: String,
    status: Option<String>,
    priority: Option<String>,
    assignee: Option<String>,
    description: Option<String>,
}

async fn cmd_update(
    pool: &sea_orm::DatabaseConnection,
    args: UpdateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let task = tasks::update_task(tasks::UpdateTaskParams {
        db: pool,
        id: &args.id,
        status: args.status.as_deref(),
        priority: args.priority.as_deref(),
        assignee_id: args.assignee.as_deref(),
        description: args.description.as_deref(),
        labels: None,
        actor_id: None,
        registry: None,
    })
    .await?;
    println!("{}", serde_json::to_string_pretty(&task)?);
    Ok(())
}

struct LinkArgs {
    id: String,
    blocks: Option<String>,
    parent: Option<String>,
    related_to: Option<String>,
}

async fn cmd_link(
    pool: &sea_orm::DatabaseConnection,
    args: LinkArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let (link_type, target) = if let Some(ref t) = args.blocks {
        ("blocked_by", t.as_str())
    } else if let Some(ref t) = args.parent {
        ("parent", t.as_str())
    } else if let Some(ref t) = args.related_to {
        ("related_to", t.as_str())
    } else {
        return Err("must specify one of --blocks, --parent, or --related-to".into());
    };
    let link = tasks::link_tasks(tasks::LinkTasksParams {
        db: pool,
        source_id: &args.id,
        target_id: target,
        link_type,
        actor_id: None,
    })
    .await?;
    println!("{}", serde_json::to_string_pretty(&link)?);
    Ok(())
}

async fn cmd_depends(
    pool: &sea_orm::DatabaseConnection,
    command: DependsCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        DependsCommands::Add { task_id, depends_on, dep_type } => {
            let dep = tasks::add_dependency(pool, &task_id, &depends_on, Some(&dep_type)).await?;
            println!("{}", serde_json::to_string_pretty(&dep)?);
        }
        DependsCommands::Remove { task_id, depends_on, dep_type } => {
            tasks::remove_dependency(pool, &task_id, &depends_on, Some(&dep_type)).await?;
            println!("Dependency removed");
        }
        DependsCommands::List { task_id } => {
            let deps = tasks::list_dependencies(pool, &task_id).await?;
            println!("{}", serde_json::to_string_pretty(&deps)?);
        }
    }
    Ok(())
}

async fn cmd_template(
    pool: &sea_orm::DatabaseConnection,
    command: TemplateCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        TemplateCommands::Create { name, title_pattern, description, priority } => {
            let tmpl = tasks::create_template(tasks::CreateTemplateParams {
                db: pool,
                name: &name,
                title_pattern: &title_pattern,
                description_template: description.as_deref(),
                default_priority: Some(&priority),
                default_labels: None,
                checklist: None,
            })
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
            let task = tasks::create_from_template(tasks::CreateFromTemplateParams {
                db: pool,
                template_id: &template,
                title_vars: None,
                overrides_description: None,
                overrides_assignee: assignee.as_deref(),
                overrides_labels: None,
            })
            .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
    }
    Ok(())
}
