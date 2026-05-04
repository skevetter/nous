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

    match cmd {
        WorktreeCommands::Create {
            branch,
            slug,
            repo_root,
            task,
            agent,
        } => {
            let wt = worktrees::create(
                pool,
                worktrees::CreateWorktreeRequest {
                    slug,
                    branch,
                    repo_root,
                    agent_id: agent,
                    task_id: task,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&wt)?);
        }
        WorktreeCommands::List {
            status,
            agent,
            task,
            limit,
            offset,
        } => {
            let status_parsed = status
                .as_deref()
                .map(str::parse::<worktrees::WorktreeStatus>)
                .transpose()?;
            let wts = worktrees::list(
                pool,
                worktrees::ListWorktreesFilter {
                    status: status_parsed,
                    agent_id: agent,
                    task_id: task,
                    repo_root: None,
                    limit: Some(limit),
                    offset: Some(offset),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&wts)?);
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

    pools.close().await;
    Ok(())
}
