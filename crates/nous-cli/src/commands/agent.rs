use clap::Subcommand;

use nous_core::agents;
use nous_core::config::Config;
use nous_core::db::DbPools;

#[derive(Subcommand)]
pub enum AgentCommands {
    /// Register a new agent in the org hierarchy
    Register {
        /// Agent name (must be unique within namespace)
        #[arg(long)]
        name: String,
        /// Agent type: engineer, manager, director, senior-manager
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
        /// Parent agent ID
        #[arg(long)]
        parent: Option<String>,
        /// Namespace (defaults to config or 'default')
        #[arg(long)]
        namespace: Option<String>,
        /// Room name for this agent
        #[arg(long)]
        room: Option<String>,
        /// JSON metadata string
        #[arg(long)]
        metadata: Option<String>,
        /// Initial status
        #[arg(long)]
        status: Option<String>,
    },
    /// Deregister an agent (remove from registry)
    Deregister {
        /// Agent ID or name
        id: String,
        /// Cascade delete children
        #[arg(long)]
        cascade: bool,
    },
    /// Look up an agent by name
    Lookup {
        /// Agent name
        name: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// List agents with optional filters
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by agent type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show the agent tree (hierarchy)
    Tree {
        /// Root agent ID (omit for full tree)
        #[arg(long)]
        root: Option<String>,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Send a heartbeat for an agent
    Heartbeat {
        /// Agent ID
        id: String,
        /// New status (running, idle, blocked, done)
        #[arg(long)]
        status: Option<String>,
    },
    /// List children of an agent
    Children {
        /// Parent agent ID
        id: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// List ancestors of an agent (root to parent)
    Ancestors {
        /// Agent ID
        id: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// Search agents by name/metadata
    Search {
        /// Search query
        query: String,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// List stale agents (past heartbeat threshold)
    Stale {
        /// Threshold in seconds (default 900)
        #[arg(long, default_value = "900")]
        threshold: u64,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
}

pub async fn run(cmd: AgentCommands) {
    if let Err(e) = execute(cmd).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: AgentCommands) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;

    match cmd {
        AgentCommands::Register {
            name,
            r#type,
            parent,
            namespace,
            room,
            metadata,
            status,
        } => {
            let agent_type: agents::AgentType = r#type.parse()?;
            let agent_status = status
                .as_deref()
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            let agent = agents::register_agent(
                pool,
                agents::RegisterAgentRequest {
                    name,
                    agent_type,
                    parent_id: parent,
                    namespace,
                    room,
                    metadata,
                    status: agent_status,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&agent)?);
        }
        AgentCommands::Deregister { id, cascade } => {
            let resolved_id = if looks_like_uuid(&id) {
                id
            } else {
                let agent = agents::lookup_agent(pool, &id, None).await?;
                agent.id
            };
            let result = agents::deregister_agent(pool, &resolved_id, cascade).await?;
            println!("{{\"result\": \"{result}\"}}");
        }
        AgentCommands::Lookup { name, namespace } => {
            let agent =
                agents::lookup_agent(pool, &name, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&agent)?);
        }
        AgentCommands::List {
            status,
            r#type,
            namespace,
            limit,
        } => {
            let status_parsed = status
                .as_deref()
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<agents::AgentType>())
                .transpose()?;
            let list = agents::list_agents(
                pool,
                &agents::ListAgentsFilter {
                    namespace,
                    status: status_parsed,
                    agent_type: type_parsed,
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        AgentCommands::Tree { root, namespace } => {
            let tree =
                agents::get_tree(pool, root.as_deref(), namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&tree)?);
        }
        AgentCommands::Heartbeat { id, status } => {
            let agent_status = status
                .as_deref()
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            agents::heartbeat(pool, &id, agent_status).await?;
            println!("{{\"ok\": true}}");
        }
        AgentCommands::Children { id, namespace } => {
            let children =
                agents::list_children(pool, &id, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&children)?);
        }
        AgentCommands::Ancestors { id, namespace } => {
            let ancestors =
                agents::list_ancestors(pool, &id, namespace.as_deref()).await?;
            println!("{}", serde_json::to_string_pretty(&ancestors)?);
        }
        AgentCommands::Search {
            query,
            namespace,
            limit,
        } => {
            let results =
                agents::search_agents(pool, &query, namespace.as_deref(), Some(limit))
                    .await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        AgentCommands::Stale {
            threshold,
            namespace,
        } => {
            let stale =
                agents::list_stale_agents(pool, threshold, namespace.as_deref())
                    .await?;
            println!("{}", serde_json::to_string_pretty(&stale)?);
        }
    }

    pools.close().await;
    Ok(())
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4
}
