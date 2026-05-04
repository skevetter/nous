use clap::Subcommand;

use nous_core::config::Config;
use nous_core::db::DbPools;
use nous_core::memory;
use nous_core::memory::Embedder;

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// Save a new memory
    Save {
        /// Memory title
        #[arg(long)]
        title: String,
        /// Memory content
        #[arg(long)]
        content: String,
        /// Memory type: decision, convention, bugfix, architecture, fact, observation
        #[arg(long, rename_all = "kebab-case", default_value = "observation")]
        r#type: String,
        /// Importance: low, moderate, high
        #[arg(long, default_value = "moderate")]
        importance: String,
        /// Agent ID
        #[arg(long)]
        agent_id: Option<String>,
        /// Workspace ID
        #[arg(long)]
        workspace: Option<String>,
        /// Topic key for upsert behavior
        #[arg(long)]
        topic_key: Option<String>,
        /// Valid-from timestamp (ISO-8601)
        #[arg(long)]
        valid_from: Option<String>,
        /// Valid-until timestamp (ISO-8601)
        #[arg(long)]
        valid_until: Option<String>,
    },
    /// Search memories using full-text search
    Search {
        /// Search query
        query: String,
        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,
        /// Filter by agent ID
        #[arg(long)]
        agent_id: Option<String>,
        /// Filter by memory type
        #[arg(long, rename_all = "kebab-case")]
        r#type: Option<String>,
        /// Filter by importance
        #[arg(long)]
        importance: Option<String>,
        /// Include archived memories
        #[arg(long)]
        include_archived: bool,
        /// Max results
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Get a memory by ID
    Get {
        /// Memory ID
        id: String,
    },
    /// Update a memory
    Update {
        /// Memory ID
        id: String,
        /// New title
        #[arg(long)]
        title: Option<String>,
        /// New content
        #[arg(long)]
        content: Option<String>,
        /// New importance
        #[arg(long)]
        importance: Option<String>,
        /// New topic key
        #[arg(long)]
        topic_key: Option<String>,
        /// Archive the memory
        #[arg(long)]
        archive: bool,
    },
    /// Create a relation between two memories
    Relate {
        /// Source memory ID
        #[arg(long)]
        source: String,
        /// Target memory ID
        #[arg(long)]
        target: String,
        /// Relation type: supersedes, `conflicts_with`, related, compatible, scoped, `not_conflict`
        #[arg(long, rename_all = "kebab-case")]
        r#type: String,
    },
    /// Get recent memories as context
    Context {
        /// Filter by workspace
        #[arg(long)]
        workspace: Option<String>,
        /// Filter by agent ID
        #[arg(long)]
        agent_id: Option<String>,
        /// Filter by topic key
        #[arg(long)]
        topic_key: Option<String>,
        /// Max results
        #[arg(long, default_value = "20")]
        limit: u32,
    },
    /// Run importance decay sweep
    Decay {
        /// Days without access before high → moderate (default: 30)
        #[arg(long, default_value = "30")]
        high_days: u32,
        /// Days without access before moderate → low (default: 60)
        #[arg(long, default_value = "60")]
        moderate_days: u32,
    },
}

pub async fn run(cmd: MemoryCommands, port: Option<u16>) {
    if let Err(e) = execute(cmd, port).await {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

async fn execute(cmd: MemoryCommands, port: Option<u16>) -> Result<(), Box<dyn std::error::Error>> {
    let mut config = Config::load()?;
    if let Some(p) = port {
        config.port = p;
    }
    config.ensure_dirs()?;

    if let MemoryCommands::Decay { high_days, moderate_days } = cmd {
        return cmd_decay(&config, high_days, moderate_days).await;
    }

    let pools = DbPools::connect(&config.data_dir).await?;
    pools.run_migrations().await?;
    let pool = &pools.fts;

    dispatch(pool, &pools, cmd).await?;

    pools.close().await;
    Ok(())
}

async fn dispatch(
    pool: &sea_orm::DatabaseConnection,
    pools: &DbPools,
    cmd: MemoryCommands,
) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        MemoryCommands::Save {
            title, content, r#type, importance, agent_id, workspace, topic_key, valid_from, valid_until,
        } => cmd_save(pools, SaveArgs { title, content, memory_type: r#type, importance, agent_id, workspace, topic_key, valid_from, valid_until }).await?,
        MemoryCommands::Search {
            query, workspace, agent_id, r#type, importance, include_archived, limit,
        } => cmd_search(pool, SearchArgs { query, workspace, agent_id, memory_type: r#type, importance, include_archived, limit }).await?,
        MemoryCommands::Get { id } => {
            let mem = memory::get_memory_by_id(pool, &id).await?;
            println!("{}", serde_json::to_string_pretty(&mem)?);
        }
        MemoryCommands::Update { id, title, content, importance, topic_key, archive } => {
            cmd_update(pool, UpdateArgs { id, title, content, importance, topic_key, archive }).await?;
        }
        MemoryCommands::Relate { source, target, r#type } => {
            let relation_type: memory::RelationType = r#type.parse()?;
            let rel = memory::relate_memories(
                pool,
                &memory::RelateRequest {
                    source_id: source,
                    target_id: target,
                    relation_type,
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&rel)?);
        }
        MemoryCommands::Context { workspace, agent_id, topic_key, limit } => {
            let results = memory::get_context(
                pool,
                &memory::ContextRequest {
                    workspace_id: workspace,
                    agent_id,
                    topic_key,
                    limit: Some(limit),
                },
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&results)?);
        }
        MemoryCommands::Decay { .. } => unreachable!("decay is handled above"),
    }
    Ok(())
}

async fn cmd_decay(
    config: &nous_core::config::Config,
    high_days: u32,
    moderate_days: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("http://127.0.0.1:{}/memories/decay", config.port);
    let client = reqwest::Client::new();
    let resp = client
        .patch(&url)
        .json(&serde_json::json!({
            "high_days": high_days,
            "moderate_days": moderate_days,
        }))
        .send()
        .await?
        .error_for_status()?;
    let body: serde_json::Value = resp.json().await?;
    println!("{}", serde_json::to_string_pretty(&body)?);
    Ok(())
}

struct SaveArgs {
    title: String,
    content: String,
    memory_type: String,
    importance: String,
    agent_id: Option<String>,
    workspace: Option<String>,
    topic_key: Option<String>,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

async fn cmd_save(
    pools: &DbPools,
    args: SaveArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let pool = &pools.fts;
    let mtype: memory::MemoryType = args.memory_type.parse()?;
    let imp: memory::Importance = args.importance.parse()?;
    let mem = memory::save_memory(
        pool,
        memory::SaveMemoryRequest {
            workspace_id: args.workspace,
            agent_id: args.agent_id,
            title: args.title,
            content: args.content,
            memory_type: mtype,
            importance: Some(imp),
            topic_key: args.topic_key,
            valid_from: args.valid_from,
            valid_until: args.valid_until,
        },
    )
    .await?;

    store_embeddings(pools, &mem).await;

    println!("{}", serde_json::to_string_pretty(&mem)?);
    Ok(())
}

fn store_single_chunk_embedding(
    vec_pool: &nous_core::db::VecPool,
    chunk: &memory::Chunk,
    embedding: &[f32],
) {
    if let Err(e) = memory::store_chunk_embedding(vec_pool, &chunk.id, embedding) {
        tracing::warn!(chunk_id = %chunk.id, error = %e, "failed to store chunk embedding");
    }
}

fn embed_and_store_chunks(
    embedder: &memory::OnnxEmbeddingModel,
    vec_pool: &nous_core::db::VecPool,
    chunks: &[memory::Chunk],
) {
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let Ok(embeddings) = embedder.embed(&texts) else { return };
    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        store_single_chunk_embedding(vec_pool, chunk, embedding);
    }
}

fn store_chunk_embeddings(
    embedder: &memory::OnnxEmbeddingModel,
    pools: &DbPools,
    mem: &memory::Memory,
) {
    let vec_pool = &pools.vec;
    let chunker = memory::Chunker::default();
    let chunks = chunker.chunk(&mem.id, &mem.content);
    if memory::store_chunks(vec_pool, &chunks).is_ok() {
        embed_and_store_chunks(embedder, vec_pool, &chunks);
    }
}

async fn try_store_embedding(
    pools: &DbPools,
    mem: &memory::Memory,
    full_emb: &[f32],
) {
    if let Err(e) = memory::store_embedding(&pools.fts, &pools.vec, &mem.id, full_emb).await {
        tracing::warn!(memory_id = %mem.id, error = %e, "failed to store memory embedding");
    }
}

async fn store_first_embedding(
    pools: &DbPools,
    mem: &memory::Memory,
    full_embeddings: Vec<Vec<f32>>,
) {
    let Some(full_emb) = full_embeddings.first() else { return };
    try_store_embedding(pools, mem, full_emb).await;
}

async fn store_full_embedding(
    embedder: &memory::OnnxEmbeddingModel,
    pools: &DbPools,
    mem: &memory::Memory,
) {
    if let Ok(full_embeddings) = embedder.embed(&[&mem.content]) {
        store_first_embedding(pools, mem, full_embeddings).await;
    }
}

async fn store_embeddings(pools: &DbPools, mem: &memory::Memory) {
    let Ok(embedder) = memory::OnnxEmbeddingModel::load(None) else { return };
    store_chunk_embeddings(&embedder, pools, mem);
    store_full_embedding(&embedder, pools, mem).await;
}

struct SearchArgs {
    query: String,
    workspace: Option<String>,
    agent_id: Option<String>,
    memory_type: Option<String>,
    importance: Option<String>,
    include_archived: bool,
    limit: u32,
}

async fn cmd_search(
    pool: &sea_orm::DatabaseConnection,
    args: SearchArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let mtype = args.memory_type
        .as_deref()
        .map(|s| s.parse::<memory::MemoryType>())
        .transpose()?;
    let importance_parsed = args.importance
        .as_deref()
        .map(|s| s.parse::<memory::Importance>())
        .transpose()?;
    let results = memory::search_memories(
        pool,
        &memory::SearchMemoryRequest {
            query: args.query,
            workspace_id: args.workspace,
            agent_id: args.agent_id,
            memory_type: mtype,
            importance: importance_parsed,
            include_archived: args.include_archived,
            limit: Some(args.limit),
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

struct UpdateArgs {
    id: String,
    title: Option<String>,
    content: Option<String>,
    importance: Option<String>,
    topic_key: Option<String>,
    archive: bool,
}

async fn cmd_update(
    pool: &sea_orm::DatabaseConnection,
    args: UpdateArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let importance_parsed = args.importance
        .as_deref()
        .map(|s| s.parse::<memory::Importance>())
        .transpose()?;
    let mem = memory::update_memory(
        pool,
        memory::UpdateMemoryRequest {
            id: args.id,
            title: args.title,
            content: args.content,
            importance: importance_parsed,
            topic_key: args.topic_key,
            valid_from: None,
            valid_until: None,
            archived: if args.archive { Some(true) } else { None },
        },
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&mem)?);
    Ok(())
}
