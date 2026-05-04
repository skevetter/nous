use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use sea_orm::{ConnectionTrait, Statement};
use serde::{Deserialize, Serialize};

use super::count_total;
use crate::error::AppError;
use crate::response::{clamp_limit, ApiResponse};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SaveBody {
    pub title: String,
    pub content: String,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub importance: Option<String>,
    pub agent_id: Option<String>,
    pub workspace_id: Option<String>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub title: Option<String>,
    pub content: Option<String>,
    pub importance: Option<String>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    #[serde(rename = "type")]
    pub memory_type: Option<String>,
    pub importance: Option<String>,
    pub include_archived: Option<bool>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct ContextQuery {
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub topic_key: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct RelateBody {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
}

pub async fn save(
    State(state): State<AppState>,
    Json(body): Json<SaveBody>,
) -> Result<impl IntoResponse, AppError> {
    let memory_type: nous_core::memory::MemoryType = body.memory_type.parse()?;
    let importance = body
        .importance
        .as_deref()
        .map(str::parse::<nous_core::memory::Importance>)
        .transpose()?;

    let mem = nous_core::memory::save_memory(
        &state.pool,
        nous_core::memory::SaveMemoryRequest {
            workspace_id: body.workspace_id,
            agent_id: body.agent_id,
            title: body.title,
            content: body.content,
            memory_type,
            importance,
            topic_key: body.topic_key,
            valid_from: body.valid_from,
            valid_until: body.valid_until,
        },
    )
    .await?;
    Ok(ApiResponse::created(mem))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let mem = nous_core::memory::get_memory_by_id(&state.pool, &id).await?;
    Ok(ApiResponse::ok(mem))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, AppError> {
    let importance = body
        .importance
        .as_deref()
        .map(str::parse::<nous_core::memory::Importance>)
        .transpose()?;

    let mem = nous_core::memory::update_memory(
        &state.pool,
        nous_core::memory::UpdateMemoryRequest {
            id,
            title: body.title,
            content: body.content,
            importance,
            topic_key: body.topic_key,
            valid_from: body.valid_from,
            valid_until: body.valid_until,
            archived: body.archived,
        },
    )
    .await?;
    Ok(ApiResponse::ok(mem))
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<impl IntoResponse, AppError> {
    let memory_type = params
        .memory_type
        .as_deref()
        .map(str::parse::<nous_core::memory::MemoryType>)
        .transpose()?;
    let importance = params
        .importance
        .as_deref()
        .map(str::parse::<nous_core::memory::Importance>)
        .transpose()?;

    let results = nous_core::memory::search_memories(
        &state.pool,
        &nous_core::memory::SearchMemoryRequest {
            query: params.q,
            workspace_id: params.workspace_id,
            agent_id: params.agent_id,
            memory_type,
            importance,
            include_archived: params.include_archived.unwrap_or(false),
            limit: params.limit,
        },
    )
    .await?;
    Ok(ApiResponse::ok(results))
}

pub async fn context(
    State(state): State<AppState>,
    Query(params): Query<ContextQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit_val = clamp_limit(params.limit.unwrap_or(20));

    let mut count_sql = String::from("SELECT COUNT(*) as cnt FROM memories WHERE archived = 0");
    let mut count_vals: Vec<sea_orm::Value> = Vec::new();
    if let Some(ref ws) = params.workspace_id {
        count_sql.push_str(" AND workspace_id = ?");
        count_vals.push(ws.clone().into());
    }
    if let Some(ref agent) = params.agent_id {
        count_sql.push_str(" AND agent_id = ?");
        count_vals.push(agent.clone().into());
    }
    if let Some(ref topic) = params.topic_key {
        count_sql.push_str(" AND topic_key = ?");
        count_vals.push(topic.clone().into());
    }
    let total_count = count_total(&state.pool, &count_sql, count_vals).await?;

    let results = nous_core::memory::get_context(
        &state.pool,
        &nous_core::memory::ContextRequest {
            workspace_id: params.workspace_id,
            agent_id: params.agent_id,
            topic_key: params.topic_key,
            limit: Some(limit_val + 1),
        },
    )
    .await?;
    Ok(crate::response::paginated(results, limit_val, 0, total_count))
}

pub async fn relate(
    State(state): State<AppState>,
    Json(body): Json<RelateBody>,
) -> Result<impl IntoResponse, AppError> {
    let relation_type: nous_core::memory::RelationType = body.relation_type.parse()?;
    let rel = nous_core::memory::relate_memories(
        &state.pool,
        &nous_core::memory::RelateRequest {
            source_id: body.source_id,
            target_id: body.target_id,
            relation_type,
        },
    )
    .await?;
    Ok(ApiResponse::created(rel))
}

pub async fn list_relations(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let relations = nous_core::memory::list_relations(&state.pool, &id).await?;
    let total = relations.len();
    Ok(crate::response::ListEnvelope {
        data: relations,
        total,
        limit: total as u32,
        offset: 0,
        has_more: false,
    })
}

#[derive(Deserialize)]
pub struct DecayBody {
    pub high_days: Option<u32>,
    pub moderate_days: Option<u32>,
}

pub async fn decay(
    State(state): State<AppState>,
    Json(body): Json<DecayBody>,
) -> Result<impl IntoResponse, AppError> {
    let high_days = body.high_days.unwrap_or(30);
    let moderate_days = body.moderate_days.unwrap_or(60);
    let affected =
        nous_core::memory::run_importance_decay(&state.pool, high_days, moderate_days).await?;
    Ok(Json(serde_json::json!({ "decayed": affected })))
}

#[derive(Deserialize)]
pub struct ReEmbedBody {
    pub model_name: String,
}

#[derive(Serialize)]
struct ReEmbedResponse {
    status: &'static str,
    memories_processed: usize,
    old_dimension: Option<usize>,
    new_dimension: usize,
}

pub async fn re_embed(
    State(state): State<AppState>,
    Json(body): Json<ReEmbedBody>,
) -> Result<impl IntoResponse, AppError> {
    use nous_core::memory::{Embedder, OnnxEmbeddingModel};

    let model = OnnxEmbeddingModel::load(Some(&body.model_name))
        .map_err(|e| nous_core::error::NousError::Internal(format!("failed to load model: {e}")))?;
    let new_dim = model.dimension();

    let old_dim = nous_core::db::read_vec_dimension(&state.vec_pool)?;

    // Fetch all memory (id, content) from fts DB.
    let rows = state
        .pool
        .query_all(Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            "SELECT id, title, content FROM memories WHERE archived = 0",
        ))
        .await
        .map_err(nous_core::error::NousError::SeaOrm)?;

    let memories: Vec<(String, String)> = rows
        .iter()
        .filter_map(|row| {
            let id: String = row.try_get_by_index(0).ok()?;
            let title: String = row.try_get_by_index(1).ok()?;
            let content: String = row.try_get_by_index(2).ok()?;
            Some((id, format!("{title}\n{content}")))
        })
        .collect();

    // Create staging vec0 table with new dimension.
    {
        let conn = state
            .vec_pool
            .lock()
            .map_err(|e| nous_core::error::NousError::Internal(format!("vec pool lock poisoned: {e}")))?;
        conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS memory_embeddings_staging;\
             CREATE VIRTUAL TABLE memory_embeddings_staging USING vec0(\
             memory_id TEXT PRIMARY KEY, embedding float[{new_dim}]);"
        ))
        .map_err(|e| nous_core::error::NousError::Internal(format!("failed to create staging table: {e}")))?;
    }

    // Batch embed and insert into staging; drop staging on any error.
    let result = embed_and_insert_staging(&state, &model, &memories, new_dim).await;

    if let Err(e) = result {
        // Cleanup staging table on failure.
        if let Ok(conn) = state.vec_pool.lock() {
            let _ = conn.execute_batch("DROP TABLE IF EXISTS memory_embeddings_staging;");
        }
        return Err(AppError::from(e));
    }

    // Atomic swap: drop old table, rename staging.
    {
        let conn = state
            .vec_pool
            .lock()
            .map_err(|e| nous_core::error::NousError::Internal(format!("vec pool lock poisoned: {e}")))?;
        conn.execute_batch(
            "DROP TABLE IF EXISTS memory_embeddings;\
             ALTER TABLE memory_embeddings_staging RENAME TO memory_embeddings;",
        )
        .map_err(|e| {
            nous_core::error::NousError::Internal(format!("failed to swap embedding tables: {e}"))
        })?;
    }

    Ok(Json(ReEmbedResponse {
        status: "ok",
        memories_processed: memories.len(),
        old_dimension: old_dim,
        new_dimension: new_dim,
    }))
}

async fn embed_and_insert_staging(
    state: &AppState,
    model: &nous_core::memory::OnnxEmbeddingModel,
    memories: &[(String, String)],
    _new_dim: usize,
) -> Result<(), nous_core::error::NousError> {
    use nous_core::memory::Embedder;

    for chunk in memories.chunks(32) {
        let texts: Vec<&str> = chunk.iter().map(|(_, text)| text.as_str()).collect();
        let embeddings = model.embed(&texts)?;

        let conn = state
            .vec_pool
            .lock()
            .map_err(|e| nous_core::error::NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

        for ((id, _), embedding) in chunk.iter().zip(embeddings.iter()) {
            let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
            conn.execute(
                "INSERT OR REPLACE INTO memory_embeddings_staging(memory_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![id, bytes],
            )
            .map_err(|e| nous_core::error::NousError::Internal(format!("failed to insert staging embedding: {e}")))?;
        }
    }

    Ok(())
}
