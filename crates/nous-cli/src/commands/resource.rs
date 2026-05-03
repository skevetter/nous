use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::resources;

#[derive(Subcommand)]
pub enum ResourceCommands {
    /// Register a new resource
    Register {
        /// Resource name
        #[arg(long)]
        name: String,
        /// Resource type: worktree, room, schedule, branch, file, docker-image, binary
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
        /// Owning agent ID (optional)
        #[arg(long)]
        owner: Option<String>,
        /// Filesystem or logical path
        #[arg(long)]
        path: Option<String>,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// JSON metadata
        #[arg(long)]
        metadata: Option<String>,
        /// Ownership policy: cascade-delete, orphan, transfer-to-parent
        #[arg(long)]
        policy: Option<String>,
    },
    /// List resources with optional filters
    List {
        /// Filter by resource type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by status: active, archived, deleted
        #[arg(long)]
        status: Option<String>,
        /// Filter by owner agent ID
        #[arg(long)]
        owner: Option<String>,
        /// Show only orphaned (unowned) resources
        #[arg(long)]
        orphaned: bool,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Filter by ownership policy
        #[arg(long)]
        policy: Option<String>,
        /// Max results
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show a single resource by ID
    Show {
        /// Resource ID
        id: String,
    },
    /// Update a resource
    Update {
        /// Resource ID
        id: String,
        /// New name
        #[arg(long)]
        name: Option<String>,
        /// New path
        #[arg(long)]
        path: Option<String>,
        /// New tags (comma-separated, replaces existing)
        #[arg(long)]
        tags: Option<String>,
        /// New metadata JSON (replaces existing)
        #[arg(long)]
        metadata: Option<String>,
        /// New ownership policy
        #[arg(long)]
        policy: Option<String>,
    },
    /// Search resources by tags (AND semantics)
    Search {
        /// Tags to search for (can be specified multiple times)
        #[arg(long = "tag", required = true)]
        tags: Vec<String>,
        /// Filter by resource type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Max results
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Archive an active resource
    Archive {
        /// Resource ID
        id: String,
    },
    /// Delete a resource
    Delete {
        /// Resource ID
        id: String,
        /// Force delete (remove from database entirely)
        #[arg(long)]
        force: bool,
    },
    /// Deregister a resource (deprecated: use 'delete')
    #[command(hide = true)]
    Deregister {
        /// Resource ID
        id: String,
        /// Force delete (remove from database entirely)
        #[arg(long)]
        force: bool,
    },
    /// Update last_seen_at timestamp (liveness heartbeat)
    Heartbeat {
        /// Resource ID
        id: String,
    },
    /// Transfer ownership of resources from one agent to another
    Transfer {
        /// Source agent ID
        #[arg(long)]
        from: String,
        /// Target agent ID (omit to orphan)
        #[arg(long)]
        to: Option<String>,
    },
}

pub async fn run(cmd: ResourceCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(
    cmd: ResourceCommands,
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
        ResourceCommands::Register {
            name,
            r#type,
            owner,
            path,
            namespace,
            tags,
            metadata,
            policy,
        } => {
            let resource_type: resources::ResourceType = r#type.parse()?;
            let ownership_policy = policy
                .as_deref()
                .map(|s| s.parse::<resources::OwnershipPolicy>())
                .transpose()?;
            let tags_vec = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let resource = resources::register_resource(
                pool,
                resources::RegisterResourceRequest {
                    name,
                    resource_type,
                    owner_agent_id: owner,
                    namespace,
                    path,
                    metadata,
                    tags: tags_vec,
                    ownership_policy,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&resource)?);
        }
        ResourceCommands::List {
            r#type,
            status,
            owner,
            orphaned,
            namespace,
            policy,
            limit,
        } => {
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<resources::ResourceType>())
                .transpose()?;
            let status_parsed = status
                .as_deref()
                .map(|s| s.parse::<resources::ResourceStatus>())
                .transpose()?;
            let policy_parsed = policy
                .as_deref()
                .map(|s| s.parse::<resources::OwnershipPolicy>())
                .transpose()?;
            let items = resources::list_resources(
                pool,
                &resources::ListResourcesFilter {
                    resource_type: type_parsed,
                    status: status_parsed,
                    owner_agent_id: owner,
                    namespace,
                    orphaned: if orphaned { Some(true) } else { None },
                    ownership_policy: policy_parsed,
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        ResourceCommands::Show { id } => {
            let resource = resources::get_resource_by_id(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&resource)?);
        }
        ResourceCommands::Update {
            id,
            name,
            path,
            tags,
            metadata,
            policy,
        } => {
            let tags_vec = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let ownership_policy = policy
                .as_deref()
                .map(|s| s.parse::<resources::OwnershipPolicy>())
                .transpose()?;
            let resource = resources::update_resource(
                pool,
                resources::UpdateResourceRequest {
                    id,
                    name,
                    path,
                    metadata,
                    tags: tags_vec,
                    status: None,
                    ownership_policy,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&resource)?);
        }
        ResourceCommands::Search {
            tags,
            r#type,
            status,
            namespace,
            limit,
        } => {
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<resources::ResourceType>())
                .transpose()?;
            let status_parsed = status
                .as_deref()
                .map(|s| s.parse::<resources::ResourceStatus>())
                .transpose()?;
            let items = resources::search_by_tags(
                pool,
                &resources::SearchResourcesRequest {
                    tags,
                    resource_type: type_parsed,
                    status: status_parsed,
                    namespace,
                    limit: Some(limit),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        ResourceCommands::Archive { id } => {
            let resource = resources::archive_resource(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&resource)?);
        }
        ResourceCommands::Delete { id, force } | ResourceCommands::Deregister { id, force } => {
            resources::deregister_resource(pool, &id, force).await?;
            println!("{{\"deleted\": true}}");
        }
        ResourceCommands::Heartbeat { id } => {
            let resource = resources::heartbeat_resource(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&resource)?);
        }
        ResourceCommands::Transfer { from, to } => {
            let count = resources::transfer_ownership(pool, &from, to.as_deref()).await?;
            println!("{{\"transferred\": {count}}}");
        }
    }

    pools.close().await;
    Ok(())
}
