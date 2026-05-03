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
pub struct DeregisterQuery {
    pub hard: Option<bool>,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<impl IntoResponse, AppError> {
    let artifact_type: nous_core::inventory::InventoryType = body.artifact_type.parse()?;
    let item = nous_core::inventory::register_item(
        &state.pool,
        nous_core::inventory::RegisterItemRequest {
            name: body.name,
            artifact_type,
            owner_agent_id: body.owner_agent_id,
            namespace: body.namespace,
            path: body.path,
            metadata: body.metadata,
            tags: body.tags,
        },
    )
    .await?;
    Ok(ApiResponse::created(item))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let artifact_type = params
        .artifact_type
        .as_deref()
        .map(|s| s.parse::<nous_core::inventory::InventoryType>())
        .transpose()?;
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::inventory::InventoryStatus>())
        .transpose()?;

    let items = nous_core::inventory::list_items(
        &state.pool,
        &nous_core::inventory::ListItemsFilter {
            artifact_type,
            status,
            owner_agent_id: params.owner_agent_id,
            namespace: params.namespace,
            orphaned: params.orphaned,
            limit: params.limit,
            offset: params.offset,
        },
    )
    .await?;
    Ok(ApiResponse::ok(items))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let item = nous_core::inventory::get_item_by_id(&state.pool, &id).await?;
    Ok(ApiResponse::ok(item))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateBody>,
) -> Result<impl IntoResponse, AppError> {
    let item = nous_core::inventory::update_item(
        &state.pool,
        nous_core::inventory::UpdateItemRequest {
            id,
            name: body.name,
            path: body.path,
            metadata: body.metadata,
            tags: body.tags,
            status: None,
        },
    )
    .await?;
    Ok(ApiResponse::ok(item))
}

pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let item = nous_core::inventory::archive_item(&state.pool, &id).await?;
    Ok(ApiResponse::ok(item))
}

pub async fn deregister(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeregisterQuery>,
) -> Result<impl IntoResponse, AppError> {
    let hard = params.hard.unwrap_or(false);
    nous_core::inventory::deregister_item(&state.pool, &id, hard).await?;
    if hard {
        Ok(crate::response::no_content().into_response())
    } else {
        Ok(ApiResponse::ok(serde_json::json!({"ok": true})).into_response())
    }
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

    let artifact_type = params
        .artifact_type
        .as_deref()
        .map(|s| s.parse::<nous_core::inventory::InventoryType>())
        .transpose()?;
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::inventory::InventoryStatus>())
        .transpose()?;

    let items = nous_core::inventory::search_by_tags(
        &state.pool,
        &nous_core::inventory::SearchItemsRequest {
            tags,
            artifact_type,
            status,
            namespace: params.namespace,
            limit: params.limit,
        },
    )
    .await?;
    Ok(ApiResponse::ok(items))
}
