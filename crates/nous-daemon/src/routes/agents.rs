use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use super::count_total;
use crate::error::AppError;
use crate::response::{clamp_limit, ApiResponse};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RegisterAgentBody {
    pub name: String,
    pub agent_type: Option<String>,
    pub parent_id: Option<String>,
    pub namespace: Option<String>,
    pub room: Option<String>,
    pub metadata: Option<String>,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    pub force: bool,
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
        .map(str::parse::<nous_core::agents::AgentStatus>)
        .transpose()?;

    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: body.name,
            agent_type: body.agent_type,
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
    let limit = clamp_limit(params.limit.unwrap_or(50));
    let offset = params.offset.unwrap_or(0);
    let status = params
        .status
        .as_deref()
        .map(str::parse::<nous_core::agents::AgentStatus>)
        .transpose()?;

    let mut count_sql = String::from("SELECT COUNT(*) as cnt FROM agents");
    let mut count_conds: Vec<String> = Vec::new();
    let mut count_vals: Vec<sea_orm::Value> = Vec::new();
    if let Some(ref ns) = params.namespace {
        count_conds.push("namespace = ?".into());
        count_vals.push(ns.clone().into());
    }
    if let Some(ref s) = status {
        count_conds.push("status = ?".into());
        count_vals.push(s.as_str().to_string().into());
    }
    if !count_conds.is_empty() {
        count_sql.push_str(" WHERE ");
        count_sql.push_str(&count_conds.join(" AND "));
    }
    let total_count = count_total(&state.pool, &count_sql, count_vals).await?;

    let agents = nous_core::agents::list_agents(
        &state.pool,
        &nous_core::agents::ListAgentsFilter {
            namespace: params.namespace,
            status,
            limit: Some(limit + 1),
            offset: Some(offset),
        },
    )
    .await?;

    Ok(crate::response::paginated(agents, limit, offset, total_count))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteQuery>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::agents::deregister_agent(&state.pool, &id, params.force).await?;
    Ok(crate::response::no_content())
}

pub async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<HeartbeatBody>,
) -> Result<impl IntoResponse, AppError> {
    let status = body
        .status
        .as_deref()
        .map(str::parse::<nous_core::agents::AgentStatus>)
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
    let limit = clamp_limit(params.limit.unwrap_or(50));
    let offset = params.offset.unwrap_or(0);
    let total_count = count_total(
        &state.pool,
        "SELECT COUNT(*) as cnt FROM agent_versions WHERE agent_id = ?",
        vec![id.clone().into()],
    )
    .await?;
    let versions = nous_core::agents::list_versions(&state.pool, &id, Some(limit + 1)).await?;
    Ok(crate::response::paginated(versions, limit, offset, total_count))
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
    let limit = clamp_limit(params.limit.unwrap_or(50));
    let offset = params.offset.unwrap_or(0);

    let mut count_sql =
        String::from("SELECT COUNT(*) as cnt FROM agents WHERE upgrade_available = 1");
    let mut count_vals: Vec<sea_orm::Value> = Vec::new();
    if let Some(ref ns) = params.namespace {
        count_sql.push_str(" AND namespace = ?");
        count_vals.push(ns.clone().into());
    }
    let total_count = count_total(&state.pool, &count_sql, count_vals).await?;

    let agents = nous_core::agents::list_outdated_agents(
        &state.pool,
        params.namespace.as_deref(),
        Some(limit + 1),
    )
    .await?;
    Ok(crate::response::paginated(agents, limit, offset, total_count))
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
    let limit = clamp_limit(params.limit.unwrap_or(50));
    let offset: u32 = 0;

    let mut count_sql = String::from("SELECT COUNT(*) as cnt FROM agent_templates");
    let mut count_vals: Vec<sea_orm::Value> = Vec::new();
    if let Some(ref t) = params.template_type {
        count_sql.push_str(" WHERE template_type = ?");
        count_vals.push(t.clone().into());
    }
    let total_count = count_total(&state.pool, &count_sql, count_vals).await?;

    let templates = nous_core::agents::list_templates(
        &state.pool,
        params.template_type.as_deref(),
        Some(limit + 1),
    )
    .await?;
    Ok(crate::response::paginated(templates, limit, offset, total_count))
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
