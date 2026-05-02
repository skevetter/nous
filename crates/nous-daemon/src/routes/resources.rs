use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RegisterBody {
    pub name: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
    pub ownership_policy: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub name: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    pub ownership_policy: Option<String>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(rename = "type")]
    pub resource_type: Option<String>,
    pub status: Option<String>,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub orphaned: Option<bool>,
    pub ownership_policy: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub tags: String,
    #[serde(rename = "type")]
    pub resource_type: Option<String>,
    pub status: Option<String>,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct DeregisterQuery {
    pub hard: Option<bool>,
}

#[derive(Deserialize)]
pub struct TransferBody {
    pub from_agent_id: String,
    pub to_agent_id: Option<String>,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, AppError> {
    let resource_type: nous_core::resources::ResourceType = body.resource_type.parse()?;
    let ownership_policy = body
        .ownership_policy
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::OwnershipPolicy>())
        .transpose()?;
    let resource = nous_core::resources::register_resource(
        &state.pool,
        nous_core::resources::RegisterResourceRequest {
            name: body.name,
            resource_type,
            owner_agent_id: body.owner_agent_id,
            namespace: body.namespace,
            path: body.path,
            metadata: body.metadata,
            tags: body.tags,
            ownership_policy,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(resource)))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let resource_type = params
        .resource_type
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceType>())
        .transpose()?;
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceStatus>())
        .transpose()?;
    let ownership_policy = params
        .ownership_policy
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::OwnershipPolicy>())
        .transpose()?;

    let resources = nous_core::resources::list_resources(
        &state.pool,
        &nous_core::resources::ListResourcesFilter {
            resource_type,
            status,
            owner_agent_id: params.owner_agent_id,
            namespace: params.namespace,
            orphaned: params.orphaned,
            ownership_policy,
            limit: params.limit,
            offset: params.offset,
        },
    )
    .await?;
    Ok(Json(resources))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let resource = nous_core::resources::get_resource_by_id(&state.pool, &id).await?;
    Ok(Json(resource))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, AppError> {
    let status = body
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceStatus>())
        .transpose()?;
    let ownership_policy = body
        .ownership_policy
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::OwnershipPolicy>())
        .transpose()?;

    let resource = nous_core::resources::update_resource(
        &state.pool,
        nous_core::resources::UpdateResourceRequest {
            id,
            name: body.name,
            path: body.path,
            metadata: body.metadata,
            tags: body.tags,
            status,
            ownership_policy,
        },
    )
    .await?;
    Ok(Json(resource))
}

pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let resource = nous_core::resources::archive_resource(&state.pool, &id).await?;
    Ok(Json(resource))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeregisterQuery>,
) -> Result<impl IntoResponse, AppError> {
    let hard = params.hard.unwrap_or(false);
    nous_core::resources::deregister_resource(&state.pool, &id, hard).await?;
    if hard {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok(Json(serde_json::json!({"ok": true})).into_response())
    }
}

pub async fn heartbeat(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let resource = nous_core::resources::heartbeat_resource(&state.pool, &id).await?;
    Ok(Json(resource))
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<impl IntoResponse, AppError> {
    let tags: Vec<String> = params
        .tags
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let resource_type = params
        .resource_type
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceType>())
        .transpose()?;
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceStatus>())
        .transpose()?;

    let resources = nous_core::resources::search_by_tags(
        &state.pool,
        &nous_core::resources::SearchResourcesRequest {
            tags,
            resource_type,
            status,
            namespace: params.namespace,
            limit: params.limit,
        },
    )
    .await?;
    Ok(Json(resources))
}

pub async fn transfer(
    State(state): State<AppState>,
    Json(body): Json<TransferBody>,
) -> Result<impl IntoResponse, AppError> {
    let count = nous_core::resources::transfer_ownership(
        &state.pool,
        &body.from_agent_id,
        body.to_agent_id.as_deref(),
    )
    .await?;
    Ok(Json(serde_json::json!({"transferred": count})))
}
