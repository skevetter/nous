use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::resources;

use super::output::{OutputFormat, parse_fields, print_list};

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
        /// Output format: json (default), table, csv
        #[arg(short, long, default_value = "json")]
        output: OutputFormat,
        /// Comma-separated fields to include (e.g. id,name,resource_type)
        #[arg(long)]
        fields: Option<String>,
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
    /// Update `last_seen_at` timestamp (liveness heartbeat)
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

    dispatch(pool, cmd).await?;

    pools.close().await;
    Ok(())
}

async fn dispatch(
    pool: &sea_orm::DatabaseConnection,
    cmd: ResourceCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        ResourceCommands::Register {
            name, r#type, owner, path, namespace, tags, metadata, policy,
        } => cmd_register(pool, RegisterArgs { name, resource_type: r#type, owner, path, namespace, tags, metadata, policy }).await?,
        ResourceCommands::List {
            r#type, status, owner, orphaned, namespace, policy, limit, output, fields,
        } => cmd_list(pool, ListArgs { resource_type: r#type, status, owner, orphaned, namespace, policy, limit, output, fields }).await?,
        ResourceCommands::Update { id, name, path, tags, metadata, policy } => {
            cmd_update(pool, UpdateArgs { id, name, path, tags, metadata, policy }).await?;
        }
        ResourceCommands::Search { tags, r#type, status, namespace, limit } => {
            cmd_search(pool, SearchArgs { tags, resource_type: r#type, status, namespace, limit }).await?;
        }
        other => dispatch_resource_cmd(pool, other).await?,
    }
    Ok(())
}

async fn dispatch_resource_cmd(
    pool: &sea_orm::DatabaseConnection,
    cmd: ResourceCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        ResourceCommands::Show { id } => {
            let resource = resources::get_resource_by_id(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&resource)?);
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
        _ => unreachable!("handled by dispatch"),
    }
    Ok(())
}

struct RegisterArgs {
    name: String,
    resource_type: String,
    owner: Option<String>,
    path: Option<String>,
    namespace: Option<String>,
    tags: Option<String>,
    metadata: Option<String>,
    policy: Option<String>,
}

async fn cmd_register(
    pool: &sea_orm::DatabaseConnection,
    args: RegisterArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let rtype: resources::ResourceType = args.resource_type.parse()?;
    let ownership_policy = args.policy
        .as_deref()
        .map(|s| s.parse::<resources::OwnershipPolicy>())
        .transpose()?;
    let tags_vec = args.tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let resource = resources::register_resource(
        pool,
        resources::RegisterResourceRequest {
            name: args.name,
            resource_type: rtype,
            owner_agent_id: args.owner,
            namespace: args.namespace,
            path: args.path,
            metadata: args.metadata,
            tags: tags_vec,
            ownership_policy,
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&resource)?);
    Ok(())
}

struct ListArgs {
    resource_type: Option<String>,
    status: Option<String>,
    owner: Option<String>,
    orphaned: bool,
    namespace: Option<String>,
    policy: Option<String>,
    limit: u32,
    output: OutputFormat,
    fields: Option<String>,
}

async fn cmd_list(
    pool: &sea_orm::DatabaseConnection,
    args: ListArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let type_parsed = args.resource_type
        .as_deref()
        .map(|s| s.parse::<resources::ResourceType>())
        .transpose()?;
    let status_parsed = args.status
        .as_deref()
        .map(|s| s.parse::<resources::ResourceStatus>())
        .transpose()?;
    let policy_parsed = args.policy
        .as_deref()
        .map(|s| s.parse::<resources::OwnershipPolicy>())
        .transpose()?;
    let items = resources::list_resources(
        pool,
        &resources::ListResourcesFilter {
            resource_type: type_parsed,
            status: status_parsed,
            owner_agent_id: args.owner,
            namespace: args.namespace,
            orphaned: if args.orphaned { Some(true) } else { None },
            ownership_policy: policy_parsed,
            limit: Some(args.limit),
            ..Default::default()
        },
    )
    .await?;
    let val = serde_json::to_value(&items)?;
    let fields_override = args.fields.as_deref().map(parse_fields);
    print_list(&val, &args.output, &["id", "name", "resource_type", "status", "owner_agent_id", "namespace"], fields_override.as_deref());
    Ok(())
}

struct UpdateArgs {
    id: String,
    name: Option<String>,
    path: Option<String>,
    tags: Option<String>,
    metadata: Option<String>,
    policy: Option<String>,
}

async fn cmd_update(
    pool: &sea_orm::DatabaseConnection,
    args: UpdateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let tags_vec = args.tags.map(|t| {
        t.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });
    let ownership_policy = args.policy
        .as_deref()
        .map(|s| s.parse::<resources::OwnershipPolicy>())
        .transpose()?;
    let resource = resources::update_resource(
        pool,
        resources::UpdateResourceRequest {
            id: args.id,
            name: args.name,
            path: args.path,
            metadata: args.metadata,
            tags: tags_vec,
            status: None,
            ownership_policy,
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&resource)?);
    Ok(())
}

struct SearchArgs {
    tags: Vec<String>,
    resource_type: Option<String>,
    status: Option<String>,
    namespace: Option<String>,
    limit: u32,
}

async fn cmd_search(
    pool: &sea_orm::DatabaseConnection,
    args: SearchArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let type_parsed = args.resource_type
        .as_deref()
        .map(|s| s.parse::<resources::ResourceType>())
        .transpose()?;
    let status_parsed = args.status
        .as_deref()
        .map(|s| s.parse::<resources::ResourceStatus>())
        .transpose()?;
    let items = resources::search_by_tags(
        pool,
        &resources::SearchResourcesRequest {
            tags: args.tags,
            resource_type: type_parsed,
            status: status_parsed,
            namespace: args.namespace,
            limit: Some(args.limit),
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&items)?);
    Ok(())
}
