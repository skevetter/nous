use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::inventory;

#[derive(Subcommand)]
pub enum InventoryCommands {
    /// Register a new inventory item
    Register {
        /// Item name
        #[arg(long)]
        name: String,
        /// Artifact type: worktree, room, schedule, branch, file, docker-image, binary
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
    },
    /// List inventory items
    List {
        /// Filter by artifact type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by status: active, archived, deleted
        #[arg(long)]
        status: Option<String>,
        /// Filter by owner agent ID
        #[arg(long)]
        owner: Option<String>,
        /// Show only orphaned (unowned) items
        #[arg(long)]
        orphaned: bool,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        /// Max results
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Show a single inventory item by ID
    Show {
        /// Item ID
        id: String,
    },
    /// Update an inventory item
    Update {
        /// Item ID
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
    },
    /// Search inventory by tags (AND semantics)
    Search {
        /// Tags to search for (can be specified multiple times)
        #[arg(long = "tag", required = true)]
        tags: Vec<String>,
        /// Filter by artifact type
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
    /// Archive an active inventory item
    Archive {
        /// Item ID
        id: String,
    },
    /// Deregister (soft-delete or hard-delete) an inventory item
    Deregister {
        /// Item ID
        id: String,
        /// Hard delete (remove from database entirely)
        #[arg(long)]
        hard: bool,
    },
}

pub async fn run(cmd: InventoryCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(
    cmd: InventoryCommands,
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
        InventoryCommands::Register {
            name,
            r#type,
            owner,
            path,
            namespace,
            tags,
            metadata,
        } => {
            let artifact_type: inventory::InventoryType = r#type.parse()?;
            let tags_vec = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let item = inventory::register_item(
                pool,
                inventory::RegisterItemRequest {
                    name,
                    artifact_type,
                    owner_agent_id: owner,
                    namespace,
                    path,
                    metadata,
                    tags: tags_vec,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&item)?);
        }
        InventoryCommands::List {
            r#type,
            status,
            owner,
            orphaned,
            namespace,
            limit,
        } => {
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<inventory::InventoryType>())
                .transpose()?;
            let status_parsed = status
                .as_deref()
                .map(|s| s.parse::<inventory::InventoryStatus>())
                .transpose()?;
            let items = inventory::list_items(
                pool,
                &inventory::ListItemsFilter {
                    artifact_type: type_parsed,
                    status: status_parsed,
                    owner_agent_id: owner,
                    namespace,
                    orphaned: if orphaned { Some(true) } else { None },
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        InventoryCommands::Show { id } => {
            let item = inventory::get_item_by_id(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&item)?);
        }
        InventoryCommands::Update {
            id,
            name,
            path,
            tags,
            metadata,
        } => {
            let tags_vec = tags.map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let item = inventory::update_item(
                pool,
                inventory::UpdateItemRequest {
                    id,
                    name,
                    path,
                    metadata,
                    tags: tags_vec,
                    status: None,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&item)?);
        }
        InventoryCommands::Search {
            tags,
            r#type,
            status,
            namespace,
            limit,
        } => {
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<inventory::InventoryType>())
                .transpose()?;
            let status_parsed = status
                .as_deref()
                .map(|s| s.parse::<inventory::InventoryStatus>())
                .transpose()?;
            let items = inventory::search_by_tags(
                pool,
                &inventory::SearchItemsRequest {
                    tags,
                    artifact_type: type_parsed,
                    status: status_parsed,
                    namespace,
                    limit: Some(limit),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&items)?);
        }
        InventoryCommands::Archive { id } => {
            let item = inventory::archive_item(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&item)?);
        }
        InventoryCommands::Deregister { id, hard } => {
            inventory::deregister_item(pool, &id, hard).await?;
            println!("{{\"ok\": true}}");
        }
    }

    pools.close().await;
    Ok(())
}
