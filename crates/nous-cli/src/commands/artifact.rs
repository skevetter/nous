use clap::Subcommand;

use nous_core::agents;
use nous_core::config::Config;
use nous_core::db::DbPools;

#[derive(Subcommand)]
pub enum ArtifactCommands {
    /// Register a new artifact owned by an agent
    Register {
        /// Owning agent ID
        #[arg(long)]
        agent: String,
        /// Artifact type: worktree, room, schedule, branch
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
        /// Artifact name
        #[arg(long)]
        name: String,
        /// Optional path (filesystem path or repo root)
        #[arg(long)]
        path: Option<String>,
        /// Namespace
        #[arg(long)]
        namespace: Option<String>,
    },
    /// List artifacts with optional filters
    List {
        /// Filter by owning agent ID
        #[arg(long)]
        agent: Option<String>,
        /// Filter by artifact type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by namespace
        #[arg(long)]
        namespace: Option<String>,
        #[arg(long, default_value = "50")]
        limit: u32,
    },
    /// Deregister (delete) an artifact
    Deregister {
        /// Artifact ID
        id: String,
    },
}

pub async fn run(cmd: ArtifactCommands) {
    if let Err(e) = execute(cmd).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: ArtifactCommands) -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;
    config.ensure_dirs()?;
    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;

    match cmd {
        ArtifactCommands::Register {
            agent,
            r#type,
            name,
            path,
            namespace,
        } => {
            let artifact_type: agents::ArtifactType = r#type.parse()?;
            let artifact = agents::register_artifact(
                pool,
                agents::RegisterArtifactRequest {
                    agent_id: agent,
                    artifact_type,
                    name,
                    path,
                    namespace,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&artifact)?);
        }
        ArtifactCommands::List {
            agent,
            r#type,
            namespace,
            limit,
        } => {
            let type_parsed = r#type
                .as_deref()
                .map(|s| s.parse::<agents::ArtifactType>())
                .transpose()?;
            let list = agents::list_artifacts(
                pool,
                &agents::ListArtifactsFilter {
                    agent_id: agent,
                    artifact_type: type_parsed,
                    namespace,
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&list)?);
        }
        ArtifactCommands::Deregister { id } => {
            agents::deregister_artifact(pool, &id).await?;
            println!("{{\"ok\": true}}");
        }
    }

    pools.close().await;
    Ok(())
}
