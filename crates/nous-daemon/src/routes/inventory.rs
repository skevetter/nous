use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct RegisterBody {
    pub name: String,
    #[serde(rename = "type")]
    pub artifact_type: String,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct UpdateBody {
    pub name: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(rename = "type")]
    pub artifact_type: Option<String>,
    pub status: Option<String>,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub orphaned: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub tags: String,
    #[serde(rename = "type")]
    pub artifact_type: Option<String>,
    pub status: Option<String>,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    pub force: bool,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, AppError> {
    let resource_type: nous_core::resources::ResourceType = body.artifact_type.parse()?;
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
            ownership_policy: Some(resource_type.default_ownership_policy()),
        },
    )
    .await?;
    Ok(ApiResponse::created(resource))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let resource_type = params
        .artifact_type
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceType>())
        .transpose()?;
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::resources::ResourceStatus>())
        .transpose()?;

    let resources = nous_core::resources::list_resources(
        &state.pool,
        &nous_core::resources::ListResourcesFilter {
            resource_type,
            status,
            owner_agent_id: params.owner_agent_id,
            namespace: params.namespace,
            orphaned: params.orphaned,
            limit: params.limit,
            offset: params.offset,
            ..Default::default()
        },
    )
    .await?;
    Ok(ApiResponse::ok(resources))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let resource = nous_core::resources::get_resource_by_id(&state.pool, &id).await?;
    Ok(ApiResponse::ok(resource))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, AppError> {
    let resource = nous_core::resources::update_resource(
        &state.pool,
        nous_core::resources::UpdateResourceRequest {
            id,
            name: body.name,
            path: body.path,
            metadata: body.metadata,
            tags: body.tags,
            status: None,
            ownership_policy: None,
        },
    )
    .await?;
    Ok(ApiResponse::ok(resource))
}

pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let resource = nous_core::resources::archive_resource(&state.pool, &id).await?;
    Ok(ApiResponse::ok(resource))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteQuery>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::resources::deregister_resource(&state.pool, &id, params.force).await?;
    Ok(crate::response::no_content())
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
        .artifact_type
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
    Ok(ApiResponse::ok(resources))
}
