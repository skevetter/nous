use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::response::ApiResponse;
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
        .map(|s| s.parse::<nous_core::memory::Importance>())
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
        .map(|s| s.parse::<nous_core::memory::Importance>())
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
        .map(|s| s.parse::<nous_core::memory::MemoryType>())
        .transpose()?;
    let importance = params
        .importance
        .as_deref()
        .map(|s| s.parse::<nous_core::memory::Importance>())
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
    let results = nous_core::memory::get_context(
        &state.pool,
        &nous_core::memory::ContextRequest {
            workspace_id: params.workspace_id,
            agent_id: params.agent_id,
            topic_key: params.topic_key,
            limit: params.limit,
        },
    )
    .await?;
    Ok(ApiResponse::ok(results))
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
    Ok(ApiResponse::ok(relations))
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
