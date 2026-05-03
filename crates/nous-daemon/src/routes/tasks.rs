use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
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
    Ok((StatusCode::CREATED, Json(task)))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListTasksQuery>,
) -> Result<impl IntoResponse, AppError> {
    let tasks = nous_core::tasks::list_tasks(nous_core::tasks::ListTasksParams {
        db: &state.pool,
        status: params.status.as_deref(),
        assignee_id: params.assignee_id.as_deref(),
        label: params.label.as_deref(),
        limit: params.limit,
        offset: params.offset,
        order_by: None,
        order_dir: None,
    })
    .await?;
    Ok(Json(tasks))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let task = nous_core::tasks::get_task(&state.pool, &id).await?;
    Ok(Json(task))
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
    Ok(Json(task))
}

pub async fn close(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let task = nous_core::tasks::close_task(&state.pool, &id, None).await?;
    Ok(Json(task))
}

pub async fn link(
    State(state): State<AppState>,
    Json(body): Json<LinkBody>,
) -> Result<impl IntoResponse, AppError> {
    let task_link = nous_core::tasks::link_tasks(
        &state.pool,
        &body.source_id,
        &body.target_id,
        &body.link_type,
        None,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(task_link)))
}

pub async fn unlink(
    State(state): State<AppState>,
    Json(body): Json<UnlinkBody>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::tasks::unlink_tasks(
        &state.pool,
        &body.source_id,
        &body.target_id,
        &body.link_type,
        None,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_links(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let links = nous_core::tasks::list_links(&state.pool, &id).await?;
    Ok(Json(links))
}

pub async fn add_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AddNoteBody>,
) -> Result<impl IntoResponse, AppError> {
    let msg = nous_core::tasks::add_note(&state.pool, &id, &body.sender_id, &body.content).await?;
    Ok((StatusCode::CREATED, Json(msg)))
}
