use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RegisterAgentBody {
    pub name: String,
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
    let agent_status = body
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::agents::AgentStatus>())
        .transpose()?;

    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: body.name,
            parent_id: body.parent_id,
            namespace: body.namespace,
            room: body.room,
            metadata: body.metadata,
            status: agent_status,
        },
    )
    .await?;

    Ok(ApiResponse::created(agent))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let agent = nous_core::agents::get_agent_by_id(&state.pool, &id).await?;
    Ok(ApiResponse::ok(agent))
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

    let agents = nous_core::agents::list_agents(
        &state.pool,
        &nous_core::agents::ListAgentsFilter {
            namespace: params.namespace,
            status,
            limit: params.limit,
            offset: params.offset,
        },
    )
    .await?;

    Ok(ApiResponse::ok(agents))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeregisterBody>,
) -> Result<impl IntoResponse, AppError> {
    let cascade = params.cascade.unwrap_or(false);
    let result = nous_core::agents::deregister_agent(&state.pool, &id, cascade).await?;
    Ok(ApiResponse::ok(serde_json::json!({ "result": result })))
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
    Ok(ApiResponse::ok(serde_json::json!({ "ok": true })))
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
    Ok(ApiResponse::ok(tree))
}

pub async fn children(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ChildrenQuery>,
) -> Result<impl IntoResponse, AppError> {
    let children =
        nous_core::agents::list_children(&state.pool, &id, params.namespace.as_deref()).await?;
    Ok(ApiResponse::ok(children))
}

pub async fn ancestors(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ChildrenQuery>,
) -> Result<impl IntoResponse, AppError> {
    let ancestors =
        nous_core::agents::list_ancestors(&state.pool, &id, params.namespace.as_deref()).await?;
    Ok(ApiResponse::ok(ancestors))
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
    Ok(ApiResponse::ok(results))
}

pub async fn stale(
    State(state): State<AppState>,
    Query(params): Query<StaleQuery>,
) -> Result<impl IntoResponse, AppError> {
    let threshold = params.threshold.unwrap_or(900);
    let agents =
        nous_core::agents::list_stale_agents(&state.pool, threshold, params.namespace.as_deref())
            .await?;
    Ok(ApiResponse::ok(agents))
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
    Ok(ApiResponse::created(artifact))
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
    Ok(ApiResponse::ok(artifacts))
}

pub async fn deregister_artifact(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::agents::deregister_artifact(&state.pool, &id).await?;
    Ok(crate::response::no_content())
}

// --- P7: Agent lifecycle, versioning, templates ---

#[derive(Deserialize)]
pub struct RecordVersionBody {
    pub agent_id: String,
    pub skill_hash: String,
    pub config_hash: String,
    pub skills_json: Option<String>,
}

pub async fn inspect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let inspection = nous_core::agents::inspect_agent(&state.pool, &id).await?;
    Ok(ApiResponse::ok(inspection))
}

pub async fn list_versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ListAgentsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit;
    let versions = nous_core::agents::list_versions(&state.pool, &id, limit).await?;
    Ok(ApiResponse::ok(versions))
}

pub async fn record_version(
    State(state): State<AppState>,
    Json(body): Json<RecordVersionBody>,
) -> Result<impl IntoResponse, AppError> {
    let version = nous_core::agents::record_version(
        &state.pool,
        nous_core::agents::RecordVersionRequest {
            agent_id: body.agent_id,
            skill_hash: body.skill_hash,
            config_hash: body.config_hash,
            skills_json: body.skills_json,
        },
    )
    .await?;
    Ok(ApiResponse::created(version))
}

pub async fn rollback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RollbackBody>,
) -> Result<impl IntoResponse, AppError> {
    let version = nous_core::agents::rollback_agent(&state.pool, &id, &body.version_id).await?;
    Ok(ApiResponse::ok(version))
}

#[derive(Deserialize)]
pub struct RollbackBody {
    pub version_id: String,
}

pub async fn notify_upgrade(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::agents::set_upgrade_available(&state.pool, &id, true).await?;
    Ok(ApiResponse::ok(serde_json::json!({"notified": true})))
}

pub async fn list_outdated(
    State(state): State<AppState>,
    Query(params): Query<ListAgentsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let agents = nous_core::agents::list_outdated_agents(
        &state.pool,
        params.namespace.as_deref(),
        params.limit,
    )
    .await?;
    Ok(ApiResponse::ok(agents))
}

// --- Template routes ---

#[derive(Deserialize)]
pub struct CreateTemplateBody {
    pub name: String,
    #[serde(rename = "type")]
    pub template_type: String,
    pub default_config: Option<String>,
    pub skill_refs: Option<String>,
}

#[derive(Deserialize)]
pub struct ListTemplatesQuery {
    #[serde(rename = "type")]
    pub template_type: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct InstantiateBody {
    pub template_id: String,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub parent_id: Option<String>,
    pub config_overrides: Option<String>,
}

pub async fn create_template(
    State(state): State<AppState>,
    Json(body): Json<CreateTemplateBody>,
) -> Result<impl IntoResponse, AppError> {
    let template = nous_core::agents::create_template(
        &state.pool,
        nous_core::agents::CreateTemplateRequest {
            name: body.name,
            template_type: body.template_type,
            default_config: body.default_config,
            skill_refs: body.skill_refs,
        },
    )
    .await?;
    Ok(ApiResponse::created(template))
}

pub async fn list_templates(
    State(state): State<AppState>,
    Query(params): Query<ListTemplatesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let templates = nous_core::agents::list_templates(
        &state.pool,
        params.template_type.as_deref(),
        params.limit,
    )
    .await?;
    Ok(ApiResponse::ok(templates))
}

pub async fn get_template(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let template = nous_core::agents::get_template_by_id(&state.pool, &id).await?;
    Ok(ApiResponse::ok(template))
}

pub async fn instantiate(
    State(state): State<AppState>,
    Json(body): Json<InstantiateBody>,
) -> Result<impl IntoResponse, AppError> {
    let agent = nous_core::agents::instantiate_from_template(
        &state.pool,
        nous_core::agents::InstantiateRequest {
            template_id: body.template_id,
            name: body.name,
            namespace: body.namespace,
            parent_id: body.parent_id,
            config_overrides: body.config_overrides,
        },
    )
    .await?;
    Ok(ApiResponse::created(agent))
}
