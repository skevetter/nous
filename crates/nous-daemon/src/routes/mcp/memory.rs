use serde_json::Value;

use nous_core::memory;
use nous_core::memory::store::{SavePromptRequest, SessionSummaryRequest};

use crate::state::AppState;

use super::{require_str, to_json, ToolSchema};

pub fn schemas() -> Vec<ToolSchema> {
    let mut all = memory_core_schemas();
    all.extend(memory_embedding_schemas());
    all.extend(memory_session_schemas());
    all
}

fn memory_core_schemas() -> Vec<ToolSchema> {
    let mut schemas = memory_crud_schemas();
    schemas.extend(memory_relation_schemas());
    schemas
}

fn memory_crud_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "memory_save",
            description: "Save a new memory (persistent structured observation). If topic_key matches an existing active memory, it updates instead.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short searchable title" },
                    "content": { "type": "string", "description": "Structured content (use **What**, **Why**, **Where**, **Learned** format)" },
                    "type": { "type": "string", "description": "Memory type: decision, convention, bugfix, architecture, fact, observation" },
                    "importance": { "type": "string", "description": "Importance: low, moderate, high (default: moderate)" },
                    "agent_id": { "type": "string", "description": "Agent ID that created this memory" },
                    "workspace_id": { "type": "string", "description": "Workspace scope (default: 'default')" },
                    "topic_key": { "type": "string", "description": "Topic key for upsert (e.g. 'architecture/auth-model')" },
                    "valid_from": { "type": "string", "description": "ISO-8601 start of validity" },
                    "valid_until": { "type": "string", "description": "ISO-8601 end of validity" }
                },
                "required": ["title", "content", "type"]
            }),
        },
        ToolSchema {
            name: "memory_search",
            description: "Search memories using full-text search (FTS5)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "workspace_id": { "type": "string", "description": "Filter by workspace" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "type": { "type": "string", "description": "Filter by memory type" },
                    "importance": { "type": "string", "description": "Filter by importance" },
                    "include_archived": { "type": "boolean", "description": "Include archived memories (default: false)" },
                    "limit": { "type": "integer", "description": "Max results (default: 20)" }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
            name: "memory_get",
            description: "Get a memory by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "memory_update",
            description: "Update a memory (title, content, importance, topic_key, archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory ID" },
                    "title": { "type": "string", "description": "New title" },
                    "content": { "type": "string", "description": "New content" },
                    "importance": { "type": "string", "description": "New importance" },
                    "topic_key": { "type": "string", "description": "New topic key" },
                    "valid_from": { "type": "string", "description": "New valid_from" },
                    "valid_until": { "type": "string", "description": "New valid_until" },
                    "archived": { "type": "boolean", "description": "Set archived state" }
                },
                "required": ["id"]
            }),
        },
    ]
}

fn memory_relation_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "memory_relate",
            description: "Create a relation between two memories (supersedes, conflicts_with, related, compatible, scoped, not_conflict)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string", "description": "Source memory ID" },
                    "target_id": { "type": "string", "description": "Target memory ID" },
                    "relation_type": { "type": "string", "description": "Relation type: supersedes, conflicts_with, related, compatible, scoped, not_conflict" }
                },
                "required": ["source_id", "target_id", "relation_type"]
            }),
        },
        ToolSchema {
            name: "memory_context",
            description: "Get recent memories as context (ordered by recency, non-archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "workspace_id": { "type": "string", "description": "Filter by workspace" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "topic_key": { "type": "string", "description": "Filter by topic key" },
                    "limit": { "type": "integer", "description": "Max results (default: 20)" }
                }
            }),
        },
        ToolSchema {
            name: "memory_relations",
            description: "List all relations for a memory",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory ID" }
                },
                "required": ["id"]
            }),
        },
    ]
}

fn memory_embedding_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "memory_store_embedding",
            description: "Store a pre-computed embedding vector for a memory",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Embedding vector (array of f32)" }
                },
                "required": ["memory_id", "embedding"]
            }),
        },
        ToolSchema {
            name: "memory_search_similar",
            description: "Search memories by cosine similarity to a query embedding vector",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Query embedding vector (array of f32)" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" },
                    "workspace_id": { "type": "string", "description": "Filter by workspace" },
                    "threshold": { "type": "number", "description": "Minimum similarity threshold (default: 0.0)" }
                },
                "required": ["embedding"]
            }),
        },
        ToolSchema {
            name: "memory_chunk",
            description: "Chunk text for a given memory_id and store chunks (no embedding)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID to chunk" },
                    "chunk_size": { "type": "integer", "description": "Tokens per chunk (default: 256)" },
                    "overlap": { "type": "integer", "description": "Overlap tokens (default: 64)" }
                },
                "required": ["memory_id"]
            }),
        },
        ToolSchema {
            name: "memory_embed",
            description: "Generate embeddings for all chunks of a memory_id using local ONNX model",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID whose chunks to embed" }
                },
                "required": ["memory_id"]
            }),
        },
        ToolSchema {
            name: "memory_search_hybrid",
            description: "Hybrid search: FTS + vector similarity + RRF reranking. Auto-embeds query text internally.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" },
                    "workspace_id": { "type": "string", "description": "Filter by workspace ID" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "memory_type": { "type": "string", "description": "Filter by memory type (decision, convention, bugfix, architecture, fact, observation)" }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
            name: "memory_store_with_embedding",
            description: "Full pipeline: chunk text, generate embeddings, and store (all-in-one)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID to process" },
                    "chunk_size": { "type": "integer", "description": "Tokens per chunk (default: 256)" },
                    "overlap": { "type": "integer", "description": "Overlap tokens (default: 64)" }
                },
                "required": ["memory_id"]
            }),
        },
        ToolSchema {
            name: "memory_search_stats",
            description: "Get search analytics: total searches, type breakdown, zero-result rate, avg latency, top queries",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "since": { "type": "string", "description": "ISO datetime string to filter events (created_at >= since)" }
                }
            }),
        },
    ]
}

fn memory_session_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "memory_session_start",
            description: "Start a new memory session (creates a session record for grouping memories)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "project": { "type": "string", "description": "Project name" }
                }
            }),
        },
        ToolSchema {
            name: "memory_session_end",
            description: "End an active memory session (sets ended_at)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session ID to end" }
                },
                "required": ["session_id"]
            }),
        },
        ToolSchema {
            name: "memory_session_summary",
            description: "Save a summary to a session record and create a session_summary memory",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session ID" },
                    "summary": { "type": "string", "description": "Session summary text" },
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "workspace_id": { "type": "string", "description": "Workspace ID" }
                },
                "required": ["session_id", "summary"]
            }),
        },
        ToolSchema {
            name: "memory_save_prompt",
            description: "Save a user prompt as a memory linked to the active session",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "The user prompt text" },
                    "session_id": { "type": "string", "description": "Active session ID to link to" },
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "workspace_id": { "type": "string", "description": "Workspace ID" }
                },
                "required": ["prompt"]
            }),
        },
        ToolSchema {
            name: "memory_current_project",
            description: "Detect the current project from a directory by looking for Cargo.toml, package.json, go.mod, etc.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string", "description": "Directory path to detect project from (defaults to '.')" }
                }
            }),
        },
    ]
}

pub async fn dispatch(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    if let Some(r) = dispatch_core_memory(name, args, state).await {
        return Some(r);
    }
    if let Some(r) = dispatch_embedding_memory(name, args, state).await {
        return Some(r);
    }
    dispatch_session_memory(name, args, state).await
}

async fn dispatch_core_memory(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "memory_save" => Some(handle_save(args, state).await),
        "memory_search" => Some(handle_search(args, state).await),
        "memory_get" => Some(handle_get(args, state).await),
        "memory_update" => Some(handle_update(args, state).await),
        "memory_relate" => Some(handle_relate(args, state).await),
        "memory_context" => Some(handle_context(args, state).await),
        "memory_relations" => Some(handle_relations(args, state).await),
        _ => None,
    }
}

async fn dispatch_embedding_memory(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "memory_store_embedding" => Some(handle_store_embedding(args, state).await),
        "memory_search_similar" => Some(handle_search_similar(args, state).await),
        "memory_chunk" => Some(handle_chunk(args, state).await),
        "memory_embed" => Some(handle_embed(args, state)),
        "memory_search_hybrid" => Some(handle_search_hybrid(args, state).await),
        "memory_store_with_embedding" => Some(handle_store_with_embedding(args, state).await),
        "memory_search_stats" => Some(handle_search_stats(args, state).await),
        _ => None,
    }
}

async fn dispatch_session_memory(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "memory_session_start" => Some(handle_session_start(args, state).await),
        "memory_session_end" => Some(handle_session_end(args, state).await),
        "memory_session_summary" => Some(handle_session_summary(args, state).await),
        "memory_save_prompt" => Some(handle_save_prompt(args, state).await),
        "memory_current_project" => Some(handle_current_project(args)),
        _ => None,
    }
}

async fn handle_save(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let title = require_str(args, "title")?.to_string();
    let content = require_str(args, "content")?.to_string();
    let type_str = require_str(args, "type")?;
    let memory_type: memory::MemoryType = type_str.parse()?;
    let importance = args
        .get("importance")
        .and_then(|v| v.as_str())
        .map(str::parse::<memory::Importance>)
        .transpose()?;
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let topic_key = args
        .get("topic_key")
        .and_then(|v| v.as_str())
        .map(String::from);
    let valid_from = args
        .get("valid_from")
        .and_then(|v| v.as_str())
        .map(String::from);
    let valid_until = args
        .get("valid_until")
        .and_then(|v| v.as_str())
        .map(String::from);
    let mem = memory::save_memory(
        &state.pool,
        memory::SaveMemoryRequest {
            workspace_id,
            agent_id,
            title,
            content,
            memory_type,
            importance,
            topic_key,
            valid_from,
            valid_until,
        },
    )
    .await?;
    to_json(mem)
}

struct SearchAnalyticsParams {
    query_text: String,
    search_type: String,
    result_count: i64,
    latency_ms: i64,
    workspace_id: Option<String>,
    agent_id: Option<String>,
}

async fn record_search_analytics(
    pool: &nous_core::db::DatabaseConnection,
    params: SearchAnalyticsParams,
) {
    if let Err(e) = memory::analytics::record_search_event(
        pool,
        &memory::analytics::SearchEvent {
            query_text: params.query_text,
            search_type: params.search_type,
            result_count: params.result_count,
            latency_ms: params.latency_ms,
            workspace_id: params.workspace_id,
            agent_id: params.agent_id,
        },
    )
    .await
    {
        tracing::debug!(error = %e, "failed to record search analytics");
    }
}

async fn log_access_for_results(
    pool: &nous_core::db::DatabaseConnection,
    results: &[memory::Memory],
    access_type: &str,
) {
    for mem in results {
        let _ = memory::log_access(pool, &mem.id, access_type, None).await;
    }
}

async fn handle_search(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let query = require_str(args, "query")?.to_string();
    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let memory_type = args
        .get("type")
        .and_then(|v| v.as_str())
        .map(str::parse::<memory::MemoryType>)
        .transpose()?;
    let importance = args
        .get("importance")
        .and_then(|v| v.as_str())
        .map(str::parse::<memory::Importance>)
        .transpose()?;
    let include_archived = args
        .get("include_archived")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let start = std::time::Instant::now();
    let results = memory::search_memories(
        &state.pool,
        &memory::SearchMemoryRequest {
            query: query.clone(),
            workspace_id: workspace_id.clone(),
            agent_id: agent_id.clone(),
            memory_type,
            importance,
            include_archived,
            limit,
        },
    )
    .await?;
    let latency_ms = start.elapsed().as_millis() as i64;
    record_search_analytics(
        &state.pool,
        SearchAnalyticsParams {
            query_text: query,
            search_type: "fts".to_string(),
            result_count: i64::try_from(results.len()).unwrap_or(i64::MAX),
            latency_ms,
            workspace_id,
            agent_id,
        },
    )
    .await;
    log_access_for_results(&state.pool, &results, "search").await;
    to_json(results)
}

async fn handle_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let mem = memory::get_memory_by_id(&state.pool, id).await?;
    let _ = memory::log_access(&state.pool, &mem.id, "get", None).await;
    to_json(mem)
}

async fn handle_update(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?.to_string();
    let title = args.get("title").and_then(|v| v.as_str()).map(String::from);
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .map(String::from);
    let importance = args
        .get("importance")
        .and_then(|v| v.as_str())
        .map(str::parse::<memory::Importance>)
        .transpose()?;
    let topic_key = args
        .get("topic_key")
        .and_then(|v| v.as_str())
        .map(String::from);
    let valid_from = args
        .get("valid_from")
        .and_then(|v| v.as_str())
        .map(String::from);
    let valid_until = args
        .get("valid_until")
        .and_then(|v| v.as_str())
        .map(String::from);
    let archived = args.get("archived").and_then(serde_json::Value::as_bool);
    let mem = memory::update_memory(
        &state.pool,
        memory::UpdateMemoryRequest {
            id,
            title,
            content,
            importance,
            topic_key,
            valid_from,
            valid_until,
            archived,
        },
    )
    .await?;
    to_json(mem)
}

async fn handle_relate(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let source_id = require_str(args, "source_id")?.to_string();
    let target_id = require_str(args, "target_id")?.to_string();
    let relation_type_str = require_str(args, "relation_type")?;
    let relation_type: memory::RelationType = relation_type_str.parse()?;
    let rel = memory::relate_memories(
        &state.pool,
        &memory::RelateRequest {
            source_id,
            target_id,
            relation_type,
        },
    )
    .await?;
    to_json(rel)
}

async fn handle_context(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let topic_key = args
        .get("topic_key")
        .and_then(|v| v.as_str())
        .map(String::from);
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let results = memory::get_context(
        &state.pool,
        &memory::ContextRequest {
            workspace_id,
            agent_id,
            topic_key,
            limit,
        },
    )
    .await?;
    log_access_for_results(&state.pool, &results, "context").await;
    to_json(results)
}

async fn handle_relations(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let relations = memory::list_relations(&state.pool, id).await?;
    to_json(relations)
}

async fn handle_store_embedding(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let memory_id = require_str(args, "memory_id")?;
    let embedding: Vec<f32> = args
        .get("embedding")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()
        })
        .unwrap_or_default();
    if embedding.is_empty() {
        return Err(nous_core::error::NousError::Validation(
            "embedding array cannot be empty".into(),
        ));
    }
    memory::store_embedding(&state.pool, &state.vec_pool, memory_id, &embedding).await?;
    Ok(serde_json::json!({"stored": true}))
}

async fn handle_search_similar(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let embedding: Vec<f32> = args
        .get("embedding")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_f64().map(|f| f as f32))
                .collect()
        })
        .unwrap_or_default();
    if embedding.is_empty() {
        return Err(nous_core::error::NousError::Validation(
            "embedding array cannot be empty".into(),
        ));
    }
    let limit = args
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |v| v as u32);
    let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
    let threshold = args
        .get("threshold")
        .and_then(serde_json::Value::as_f64)
        .map(|f| f as f32);
    let start = std::time::Instant::now();
    let results = memory::search_similar(memory::SearchSimilarParams {
        db: &state.pool,
        vec_pool: &state.vec_pool,
        query_embedding: &embedding,
        limit,
        workspace_id,
        threshold,
    })
    .await?;
    let latency_ms = start.elapsed().as_millis() as i64;
    record_search_analytics(
        &state.pool,
        SearchAnalyticsParams {
            query_text: String::new(),
            search_type: "vector".to_string(),
            result_count: i64::try_from(results.len()).unwrap_or(i64::MAX),
            latency_ms,
            workspace_id: workspace_id.map(String::from),
            agent_id: None,
        },
    )
    .await;
    to_json(results)
}

async fn handle_chunk(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let memory_id = require_str(args, "memory_id")?;
    let chunk_size = args
        .get("chunk_size")
        .and_then(serde_json::Value::as_u64)
        .map_or(256, |v| v as usize);
    let overlap = args
        .get("overlap")
        .and_then(serde_json::Value::as_u64)
        .map_or(64, |v| v as usize);

    let mem = memory::get_memory_by_id(&state.pool, memory_id).await?;
    let chunker = memory::Chunker::new(chunk_size, overlap);
    let chunks = chunker.chunk(memory_id, &mem.content);
    memory::store_chunks(&state.vec_pool, &chunks)?;

    Ok(serde_json::json!({
        "memory_id": memory_id,
        "chunk_count": chunks.len(),
        "chunks": chunks
    }))
}

fn handle_embed(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let memory_id = require_str(args, "memory_id")?;
    let chunks = memory::get_chunks_for_memory(&state.vec_pool, memory_id)?;
    if chunks.is_empty() {
        return Err(nous_core::error::NousError::Validation(
            "no chunks found for memory_id — run memory_chunk first".into(),
        ));
    }

    let embedder = state.embedder.as_ref().ok_or_else(|| {
        nous_core::error::NousError::Internal(
            "embedding model not available — run `nous model download` to install".into(),
        )
    })?;
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embedder.embed(&texts)?;

    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        memory::store_chunk_embedding(&state.vec_pool, &chunk.id, embedding)?;
    }

    Ok(serde_json::json!({
        "memory_id": memory_id,
        "chunks_embedded": chunks.len()
    }))
}

struct HybridSearchParams {
    query: String,
    embedding: Vec<f32>,
    limit: usize,
    workspace_id: Option<String>,
    agent_id: Option<String>,
    memory_type: Option<memory::MemoryType>,
}

struct FtsFallbackParams {
    query: String,
    limit: usize,
    workspace_id: Option<String>,
    agent_id: Option<String>,
    memory_type: Option<memory::MemoryType>,
}

async fn search_hybrid_with_embedding(
    state: &AppState,
    params: HybridSearchParams,
) -> Result<Value, nous_core::error::NousError> {
    let start = std::time::Instant::now();
    let results = memory::search_hybrid_filtered(memory::SearchHybridFilteredParams {
        fts_db: &state.pool,
        vec_pool: &state.vec_pool,
        query: &params.query,
        query_embedding: &params.embedding,
        limit: params.limit,
        workspace_id: params.workspace_id.as_deref(),
        agent_id: params.agent_id.as_deref(),
        memory_type: params.memory_type,
    })
    .await?;
    let latency_ms = start.elapsed().as_millis() as i64;
    record_search_analytics(
        &state.pool,
        SearchAnalyticsParams {
            query_text: params.query,
            search_type: "hybrid".to_string(),
            result_count: results.len() as i64,
            latency_ms,
            workspace_id: params.workspace_id,
            agent_id: params.agent_id,
        },
    )
    .await;
    to_json(results)
}

async fn search_hybrid_fts_fallback(
    state: &AppState,
    params: FtsFallbackParams,
) -> Result<Value, nous_core::error::NousError> {
    let start = std::time::Instant::now();
    let fts_results = memory::search_memories(
        &state.pool,
        &memory::SearchMemoryRequest {
            query: params.query.clone(),
            workspace_id: params.workspace_id.clone(),
            agent_id: params.agent_id.clone(),
            memory_type: params.memory_type,
            importance: None,
            include_archived: false,
            limit: Some(params.limit as u32),
        },
    )
    .await?;
    let latency_ms = start.elapsed().as_millis() as i64;
    record_search_analytics(
        &state.pool,
        SearchAnalyticsParams {
            query_text: params.query,
            search_type: "fts5_fallback".to_string(),
            result_count: fts_results.len() as i64,
            latency_ms,
            workspace_id: params.workspace_id,
            agent_id: params.agent_id,
        },
    )
    .await;
    Ok(serde_json::json!({
        "results": fts_results,
        "_warning": "embedding unavailable, fell back to FTS5-only search"
    }))
}

async fn handle_search_hybrid(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let query = require_str(args, "query")?;
    let limit = args
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map_or(10, |v| v as usize);
    let workspace_id = args
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let agent_id = args
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let memory_type = args
        .get("memory_type")
        .and_then(|v| v.as_str())
        .and_then(|s| serde_json::from_value(serde_json::Value::String(s.to_string())).ok());

    let query_embedding = state
        .embedder
        .as_ref()
        .and_then(|embedder| embedder.embed(&[query]).ok())
        .and_then(|mut vecs| vecs.pop());

    if let Some(embedding) = query_embedding {
        search_hybrid_with_embedding(
            state,
            HybridSearchParams {
                query: query.to_string(),
                embedding,
                limit,
                workspace_id,
                agent_id,
                memory_type,
            },
        )
        .await
    } else {
        search_hybrid_fts_fallback(
            state,
            FtsFallbackParams {
                query: query.to_string(),
                limit,
                workspace_id,
                agent_id,
                memory_type,
            },
        )
        .await
    }
}

async fn handle_store_with_embedding(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let memory_id = require_str(args, "memory_id")?;
    let chunk_size = args
        .get("chunk_size")
        .and_then(serde_json::Value::as_u64)
        .map_or(256, |v| v as usize);
    let overlap = args
        .get("overlap")
        .and_then(serde_json::Value::as_u64)
        .map_or(64, |v| v as usize);

    let embedder = state.embedder.as_ref().ok_or_else(|| {
        nous_core::error::NousError::Internal(
            "embedding model not available — run `nous model download` to install".into(),
        )
    })?;

    let mem = memory::get_memory_by_id(&state.pool, memory_id).await?;

    let chunker = memory::Chunker::new(chunk_size, overlap);
    let chunks = chunker.chunk(memory_id, &mem.content);
    memory::store_chunks(&state.vec_pool, &chunks)?;

    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embedder.embed(&texts)?;

    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        memory::store_chunk_embedding(&state.vec_pool, &chunk.id, embedding)?;
    }

    let full_embeddings = embedder.embed(&[&mem.content])?;
    if let Some(full_emb) = full_embeddings.first() {
        memory::store_embedding(&state.pool, &state.vec_pool, memory_id, full_emb).await?;
    }

    Ok(serde_json::json!({
        "memory_id": memory_id,
        "chunk_count": chunks.len(),
        "chunks_embedded": chunks.len(),
        "full_embedding_stored": true
    }))
}

async fn handle_search_stats(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let since = args.get("since").and_then(|v| v.as_str());
    let search_stats = memory::analytics::get_search_stats(&state.pool, since).await?;
    to_json(search_stats)
}

async fn handle_session_start(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = args.get("agent_id").and_then(|v| v.as_str());
    let project = args.get("project").and_then(|v| v.as_str());
    let session = memory::session_start(&state.pool, agent_id, project).await?;
    to_json(session)
}

async fn handle_session_end(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let session_id = require_str(args, "session_id")?;
    let session = memory::session_end(&state.pool, session_id).await?;
    to_json(session)
}

async fn handle_session_summary(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let session_id = require_str(args, "session_id")?;
    let summary = require_str(args, "summary")?;
    let agent_id = args.get("agent_id").and_then(|v| v.as_str());
    let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
    let session = memory::session_summary(
        &state.pool,
        SessionSummaryRequest {
            session_id,
            summary,
            agent_id,
            workspace_id,
        },
    )
    .await?;
    to_json(session)
}

async fn handle_save_prompt(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let prompt = require_str(args, "prompt")?;
    let session_id = args.get("session_id").and_then(|v| v.as_str());
    let agent_id = args.get("agent_id").and_then(|v| v.as_str());
    let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
    let mem = memory::save_prompt(
        &state.pool,
        SavePromptRequest {
            session_id,
            agent_id,
            workspace_id,
            prompt,
        },
    )
    .await?;
    to_json(mem)
}

fn handle_current_project(args: &Value) -> Result<Value, nous_core::error::NousError> {
    let cwd = args.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
    match memory::detect_current_project(cwd) {
        Some(project) => to_json(project),
        None => Ok(serde_json::json!({"detected": false, "message": "no project marker found"})),
    }
}
