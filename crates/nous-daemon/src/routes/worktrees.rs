use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use super::count_total;
use crate::error::AppError;
use crate::response::{clamp_limit, ApiResponse};
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
    Ok(ApiResponse::created(wt))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListWorktreesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = clamp_limit(params.limit.unwrap_or(50));
    let offset = params.offset.unwrap_or(0);
    let status = params
        .status
        .as_deref()
        .map(str::parse::<nous_core::worktrees::WorktreeStatus>)
        .transpose()?;

    let mut count_sql = String::from("SELECT COUNT(*) as cnt FROM worktrees");
    let mut count_conds: Vec<String> = Vec::new();
    let mut count_vals: Vec<sea_orm::Value> = Vec::new();
    if let Some(ref s) = status {
        count_conds.push("status = ?".into());
        count_vals.push(s.as_str().to_string().into());
    }
    if let Some(ref a) = params.agent_id {
        count_conds.push("agent_id = ?".into());
        count_vals.push(a.clone().into());
    }
    if let Some(ref t) = params.task_id {
        count_conds.push("task_id = ?".into());
        count_vals.push(t.clone().into());
    }
    if !count_conds.is_empty() {
        count_sql.push_str(" WHERE ");
        count_sql.push_str(&count_conds.join(" AND "));
    }
    let total_count = count_total(&state.pool, &count_sql, count_vals).await?;

    let wts = nous_core::worktrees::list(
        &state.pool,
        nous_core::worktrees::ListWorktreesFilter {
            status,
            agent_id: params.agent_id,
            task_id: params.task_id,
            repo_root: None,
            limit: Some(limit + 1),
            offset: Some(offset),
        },
    )
    .await?;
    Ok(crate::response::paginated(wts, limit, offset, total_count))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let wt = nous_core::worktrees::get(&state.pool, &id).await?;
    Ok(ApiResponse::ok(wt))
}

pub async fn update_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateWorktreeStatusBody>,
) -> Result<impl IntoResponse, AppError> {
    let status: nous_core::worktrees::WorktreeStatus = body.status.parse()?;
    let wt = nous_core::worktrees::update_status(&state.pool, &id, status).await?;
    Ok(ApiResponse::ok(wt))
}

pub async fn archive(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let wt = nous_core::worktrees::archive(&state.pool, &id).await?;
    Ok(ApiResponse::ok(wt))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::worktrees::delete(&state.pool, &id).await?;
    Ok(crate::response::no_content())
}
