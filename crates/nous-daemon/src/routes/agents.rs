use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RegisterAgentBody {
    pub name: String,
    #[serde(rename = "type")]
    pub agent_type: String,
    pub parent_id: Option<String>,
    pub namespace: Option<String>,
    pub room: Option<String>,
    pub metadata: Option<String>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct DeregisterBody {
    pub cascade: Option<bool>,
}

#[derive(Deserialize)]
pub struct ListAgentsQuery {
    pub namespace: Option<String>,
    pub status: Option<String>,
    #[serde(rename = "type")]
    pub agent_type: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Deserialize)]
pub struct HeartbeatBody {
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct TreeQuery {
    pub root: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Deserialize)]
pub struct ChildrenQuery {
    pub namespace: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct StaleQuery {
    pub threshold: Option<u64>,
    pub namespace: Option<String>,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterAgentBody>,
) -> Result<impl IntoResponse, AppError> {
    let agent_type: nous_core::agents::AgentType = body.agent_type.parse()?;
    let agent_status = body
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::agents::AgentStatus>())
        .transpose()?;

    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: body.name,
            agent_type,
            parent_id: body.parent_id,
            namespace: body.namespace,
            room: body.room,
            metadata: body.metadata,
            status: agent_status,
        },
    )
    .await?;

    Ok((StatusCode::CREATED, Json(agent)))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let agent = nous_core::agents::get_agent_by_id(&state.pool, &id).await?;
    Ok(Json(agent))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListAgentsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::agents::AgentStatus>())
        .transpose()?;
    let agent_type = params
        .agent_type
        .as_deref()
        .map(|s| s.parse::<nous_core::agents::AgentType>())
        .transpose()?;

    let agents = nous_core::agents::list_agents(
        &state.pool,
        &nous_core::agents::ListAgentsFilter {
            namespace: params.namespace,
            status,
            agent_type,
            limit: params.limit,
            offset: params.offset,
        },
    )
    .await?;

    Ok(Json(agents))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeregisterBody>,
) -> Result<impl IntoResponse, AppError> {
    let cascade = params.cascade.unwrap_or(false);
    let result = nous_core::agents::deregister_agent(&state.pool, &id, cascade).await?;
    Ok(Json(serde_json::json!({ "result": result })))
}

pub async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<HeartbeatBody>,
) -> Result<impl IntoResponse, AppError> {
    let status = body
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::agents::AgentStatus>())
        .transpose()?;
    nous_core::agents::heartbeat(&state.pool, &id, status).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn tree(
    State(state): State<AppState>,
    Query(params): Query<TreeQuery>,
) -> Result<impl IntoResponse, AppError> {
    let tree = nous_core::agents::get_tree(
        &state.pool,
        params.root.as_deref(),
        params.namespace.as_deref(),
    )
    .await?;
    Ok(Json(tree))
}

pub async fn children(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ChildrenQuery>,
) -> Result<impl IntoResponse, AppError> {
    let children =
        nous_core::agents::list_children(&state.pool, &id, params.namespace.as_deref()).await?;
    Ok(Json(children))
}

pub async fn ancestors(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ChildrenQuery>,
) -> Result<impl IntoResponse, AppError> {
    let ancestors =
        nous_core::agents::list_ancestors(&state.pool, &id, params.namespace.as_deref()).await?;
    Ok(Json(ancestors))
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<impl IntoResponse, AppError> {
    let results = nous_core::agents::search_agents(
        &state.pool,
        &params.q,
        params.namespace.as_deref(),
        params.limit,
    )
    .await?;
    Ok(Json(results))
}

pub async fn stale(
    State(state): State<AppState>,
    Query(params): Query<StaleQuery>,
) -> Result<impl IntoResponse, AppError> {
    let threshold = params.threshold.unwrap_or(900);
    let agents = nous_core::agents::list_stale_agents(
        &state.pool,
        threshold,
        params.namespace.as_deref(),
    )
    .await?;
    Ok(Json(agents))
}

// --- Artifact routes ---

#[derive(Deserialize)]
pub struct RegisterArtifactBody {
    pub agent_id: String,
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub name: String,
    pub path: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Deserialize)]
pub struct ListArtifactsQuery {
    pub agent_id: Option<String>,
    #[serde(rename = "type")]
    pub artifact_type: Option<String>,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn register_artifact(
    State(state): State<AppState>,
    Json(body): Json<RegisterArtifactBody>,
) -> Result<impl IntoResponse, AppError> {
    let artifact_type: nous_core::agents::ArtifactType = body.artifact_type.parse()?;
    let artifact = nous_core::agents::register_artifact(
        &state.pool,
        nous_core::agents::RegisterArtifactRequest {
            agent_id: body.agent_id,
            artifact_type,
            name: body.name,
            path: body.path,
            namespace: body.namespace,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(artifact)))
}

pub async fn list_artifacts(
    State(state): State<AppState>,
    Query(params): Query<ListArtifactsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let artifact_type = params
        .artifact_type
        .as_deref()
        .map(|s| s.parse::<nous_core::agents::ArtifactType>())
        .transpose()?;

    let artifacts = nous_core::agents::list_artifacts(
        &state.pool,
        &nous_core::agents::ListArtifactsFilter {
            agent_id: params.agent_id,
            artifact_type,
            namespace: params.namespace,
            limit: params.limit,
            offset: params.offset,
            ..Default::default()
        },
    )
    .await?;
    Ok(Json(artifacts))
}

pub async fn deregister_artifact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::agents::deregister_artifact(&state.pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
