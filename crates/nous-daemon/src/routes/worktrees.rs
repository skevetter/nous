use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateWorktreeBody {
    pub branch: String,
    pub slug: Option<String>,
    pub repo_root: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateWorktreeStatusBody {
    pub status: String,
}

#[derive(Deserialize)]
pub struct ListWorktreesQuery {
    pub status: Option<String>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateWorktreeBody>,
) -> Result<impl IntoResponse, AppError> {
    let wt = nous_core::worktrees::create(
        &state.pool,
        nous_core::worktrees::CreateWorktreeRequest {
            slug: body.slug,
            branch: body.branch,
            repo_root: body.repo_root.unwrap_or_else(|| ".".to_string()),
            agent_id: body.agent_id,
            task_id: body.task_id,
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(wt)))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListWorktreesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let status = params
        .status
        .as_deref()
        .map(|s| s.parse::<nous_core::worktrees::WorktreeStatus>())
        .transpose()?;
    let wts = nous_core::worktrees::list(
        &state.pool,
        nous_core::worktrees::ListWorktreesFilter {
            status,
            agent_id: params.agent_id,
            task_id: params.task_id,
            repo_root: None,
            limit: params.limit,
            offset: params.offset,
        },
    )
    .await?;
    Ok(Json(wts))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let wt = nous_core::worktrees::get(&state.pool, &id).await?;
    Ok(Json(wt))
}

pub async fn update_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateWorktreeStatusBody>,
) -> Result<impl IntoResponse, AppError> {
    let status: nous_core::worktrees::WorktreeStatus = body.status.parse()?;
    let wt = nous_core::worktrees::update_status(&state.pool, &id, status).await?;
    Ok(Json(wt))
}

pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let wt = nous_core::worktrees::archive(&state.pool, &id).await?;
    Ok(Json(wt))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::worktrees::delete(&state.pool, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
