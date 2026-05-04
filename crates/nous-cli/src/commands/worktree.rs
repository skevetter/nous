use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::worktrees;

#[derive(Subcommand)]
pub enum WorktreeCommands {
    /// Create a new git worktree
    Create {
        /// Branch name for the worktree
        #[arg(long)]
        branch: String,
        /// Optional slug (defaults to last 8 chars of UUID)
        #[arg(long)]
        slug: Option<String>,
        /// Repository root path (defaults to current directory)
        #[arg(long, default_value = ".")]
        repo_root: String,
        /// Task ID to associate with this worktree
        #[arg(long)]
        task: Option<String>,
        /// Agent ID to associate with this worktree
        #[arg(long)]
        agent: Option<String>,
    },
    /// List worktrees with optional filters
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
        #[arg(long, default_value = "0")]
        offset: u32,
    },
    /// Show worktree details by ID or slug
    Show {
        /// Worktree ID or slug
        id: String,
    },
    /// Archive a worktree (removes git worktree, sets status to archived)
    Archive {
        /// Worktree ID or slug
        id: String,
    },
    /// Delete a worktree (removes directory, sets status to deleted)
    Delete {
        /// Worktree ID or slug
        id: String,
    },
}

pub async fn run(cmd: WorktreeCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(
    cmd: WorktreeCommands,
    port: Option<u16>,
) -> Result<(), Box<dyn std::error::Error>> {
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
    cmd: WorktreeCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        WorktreeCommands::Create { branch, slug, repo_root, task, agent } => {
            cmd_create(pool, CreateArgs { branch, slug, repo_root, task, agent }).await?;
        }
        WorktreeCommands::List { status, agent, task, limit, offset } => {
            cmd_list(pool, ListArgs { status, agent, task, limit, offset }).await?;
        }
        WorktreeCommands::Show { id } => {
            let wt = worktrees::get(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&wt)?);
        }
        WorktreeCommands::Archive { id } => {
            let wt = worktrees::archive(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&wt)?);
        }
        WorktreeCommands::Delete { id } => {
            worktrees::delete(pool, &id).await?;
            println!("{{\"deleted\": true}}");
        }
    }
    Ok(())
}

struct CreateArgs {
    branch: String,
    slug: Option<String>,
    repo_root: String,
    task: Option<String>,
    agent: Option<String>,
}

async fn cmd_create(
    pool: &sea_orm::DatabaseConnection,
    args: CreateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let wt = worktrees::create(
        pool,
        worktrees::CreateWorktreeRequest {
            slug: args.slug,
            branch: args.branch,
            repo_root: args.repo_root,
            agent_id: args.agent,
            task_id: args.task,
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&wt)?);
    Ok(())
}

struct ListArgs {
    status: Option<String>,
    agent: Option<String>,
    task: Option<String>,
    limit: u32,
    offset: u32,
}

async fn cmd_list(
    pool: &sea_orm::DatabaseConnection,
    args: ListArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let status_parsed = args
        .status
        .as_deref()
        .map(|s| s.parse::<worktrees::WorktreeStatus>())
        .transpose()?;
    let wts = worktrees::list(
        pool,
        worktrees::ListWorktreesFilter {
            status: status_parsed,
            agent_id: args.agent,
            task_id: args.task,
            repo_root: None,
            limit: Some(args.limit),
            offset: Some(args.offset),
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&wts)?);
    Ok(())
}
