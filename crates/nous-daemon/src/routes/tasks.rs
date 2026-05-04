use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use super::count_total;
use crate::error::AppError;
use crate::response::{clamp_limit, ApiResponse};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateTaskBody {
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub assignee_id: Option<String>,
    pub labels: Option<Vec<String>>,
    pub room_id: Option<String>,
    pub create_room: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateTaskBody {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee_id: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct ListTasksQuery {
    pub status: Option<String>,
    pub assignee_id: Option<String>,
    pub label: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Deserialize)]
pub struct LinkBody {
    pub source_id: String,
    pub target_id: String,
    pub link_type: String,
}

#[derive(Deserialize)]
pub struct UnlinkBody {
    pub source_id: String,
    pub target_id: String,
    pub link_type: String,
}

#[derive(Deserialize)]
pub struct AddNoteBody {
    pub sender_id: String,
    pub content: String,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateTaskBody>,
) -> Result<impl IntoResponse, AppError> {
    let task = nous_core::tasks::create_task(nous_core::tasks::CreateTaskParams {
        db: &state.pool,
        title: &body.title,
        description: body.description.as_deref(),
        priority: body.priority.as_deref(),
        assignee_id: body.assignee_id.as_deref(),
        labels: body.labels.as_deref(),
        room_id: body.room_id.as_deref(),
        create_room: body.create_room.unwrap_or(false),
        actor_id: None,
        registry: None,
    })
    .await?;
    Ok(ApiResponse::created(task))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListTasksQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = clamp_limit(params.limit.unwrap_or(50));
    let offset = params.offset.unwrap_or(0);

    let mut count_sql = if params.label.is_some() {
        String::from("SELECT COUNT(*) as cnt FROM tasks, json_each(tasks.labels) AS je")
    } else {
        String::from("SELECT COUNT(*) as cnt FROM tasks")
    };
    let mut count_conds: Vec<String> = Vec::new();
    let mut count_vals: Vec<sea_orm::Value> = Vec::new();
    if let Some(ref s) = params.status {
        count_conds.push("tasks.status = ?".into());
        count_vals.push(s.clone().into());
    }
    if let Some(ref a) = params.assignee_id {
        count_conds.push("tasks.assignee_id = ?".into());
        count_vals.push(a.clone().into());
    }
    if let Some(ref l) = params.label {
        count_conds.push("je.value = ?".into());
        count_vals.push(l.clone().into());
    }
    if !count_conds.is_empty() {
        count_sql.push_str(" WHERE ");
        count_sql.push_str(&count_conds.join(" AND "));
    }
    let total_count = count_total(&state.pool, &count_sql, count_vals).await?;

    let tasks = nous_core::tasks::list_tasks(nous_core::tasks::ListTasksParams {
        db: &state.pool,
        status: params.status.as_deref(),
        assignee_id: params.assignee_id.as_deref(),
        label: params.label.as_deref(),
        limit: Some(limit + 1),
        offset: Some(offset),
        order_by: None,
        order_dir: None,
    })
    .await?;
    Ok(crate::response::paginated(tasks, limit, offset, total_count))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let task = nous_core::tasks::get_task(&state.pool, &id).await?;
    Ok(ApiResponse::ok(task))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<impl IntoResponse, AppError> {
    let task = nous_core::tasks::update_task(nous_core::tasks::UpdateTaskParams {
        db: &state.pool,
        id: &id,
        status: body.status.as_deref(),
        priority: body.priority.as_deref(),
        assignee_id: body.assignee_id.as_deref(),
        description: body.description.as_deref(),
        labels: None,
        actor_id: None,
        registry: None,
    })
    .await?;
    Ok(ApiResponse::ok(task))
}

pub async fn close(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let task = nous_core::tasks::close_task(&state.pool, &id, None).await?;
    Ok(ApiResponse::ok(task))
}

pub async fn link(
    State(state): State<AppState>,
    Json(body): Json<LinkBody>,
) -> Result<impl IntoResponse, AppError> {
    let task_link = nous_core::tasks::link_tasks(nous_core::tasks::LinkTasksParams {
        db: &state.pool,
        source_id: &body.source_id,
        target_id: &body.target_id,
        link_type: &body.link_type,
        actor_id: None,
    })
    .await?;
    Ok(ApiResponse::created(task_link))
}

pub async fn unlink(
    State(state): State<AppState>,
    Json(body): Json<UnlinkBody>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::tasks::unlink_tasks(nous_core::tasks::UnlinkTasksParams {
        db: &state.pool,
        source_id: &body.source_id,
        target_id: &body.target_id,
        link_type: &body.link_type,
        actor_id: None,
    })
    .await?;
    Ok(crate::response::no_content())
}

pub async fn list_links(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let links = nous_core::tasks::list_links(&state.pool, &id).await?;
    Ok(ApiResponse::ok(links))
}

pub async fn add_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AddNoteBody>,
) -> Result<impl IntoResponse, AppError> {
    let msg = nous_core::tasks::add_note(&state.pool, &id, &body.sender_id, &body.content).await?;
    Ok(ApiResponse::created(msg))
}
