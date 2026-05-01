use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateScheduleBody {
    pub name: String,
    pub cron_expr: String,
    pub trigger_at: Option<i64>,
    pub timezone: Option<String>,
    pub action_type: String,
    pub action_payload: String,
    pub desired_outcome: Option<String>,
    pub max_retries: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_output_bytes: Option<i32>,
    pub max_runs: Option<i32>,
}

#[derive(Deserialize)]
pub struct UpdateScheduleBody {
    pub name: Option<String>,
    pub cron_expr: Option<String>,
    pub trigger_at: Option<Option<i64>>,
    pub enabled: Option<bool>,
    pub action_type: Option<String>,
    pub action_payload: Option<String>,
    pub desired_outcome: Option<String>,
    pub max_retries: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_runs: Option<i32>,
}

#[derive(Deserialize)]
pub struct ListSchedulesQuery {
    pub enabled: Option<bool>,
    pub action_type: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Deserialize)]
pub struct ListRunsQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateScheduleBody>,
) -> Result<impl IntoResponse, AppError> {
    let clock = nous_core::schedules::SystemClock;
    let schedule = nous_core::schedules::create_schedule(
        &state.pool,
        &body.name,
        &body.cron_expr,
        body.trigger_at,
        body.timezone.as_deref(),
        &body.action_type,
        &body.action_payload,
        body.desired_outcome.as_deref(),
        body.max_retries,
        body.timeout_secs,
        body.max_output_bytes,
        body.max_runs,
        &clock,
    )
    .await?;
    state.schedule_notify.notify_one();
    Ok((StatusCode::CREATED, Json(schedule)))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListSchedulesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let schedules = nous_core::schedules::list_schedules(
        &state.pool,
        params.enabled,
        params.action_type.as_deref(),
        params.limit,
    )
    .await?;
    Ok(Json(schedules))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let schedule = nous_core::schedules::get_schedule(&state.pool, &id).await?;
    Ok(Json(schedule))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateScheduleBody>,
) -> Result<impl IntoResponse, AppError> {
    let clock = nous_core::schedules::SystemClock;
    let desired_outcome_opt = body.desired_outcome.as_deref().map(Some);
    let timeout_opt = body.timeout_secs.map(Some);

    let schedule = nous_core::schedules::update_schedule(
        &state.pool,
        &id,
        body.name.as_deref(),
        body.cron_expr.as_deref(),
        body.trigger_at,
        body.enabled,
        body.action_type.as_deref(),
        body.action_payload.as_deref(),
        desired_outcome_opt,
        body.max_retries,
        timeout_opt,
        body.max_runs,
        &clock,
    )
    .await?;
    state.schedule_notify.notify_one();
    Ok(Json(schedule))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    nous_core::schedules::delete_schedule(&state.pool, &id).await?;
    state.schedule_notify.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ListRunsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let runs =
        nous_core::schedules::list_runs(&state.pool, &id, params.status.as_deref(), params.limit)
            .await?;
    Ok(Json(runs))
}

pub async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let health = nous_core::schedules::schedule_health(&state.pool).await?;
    Ok(Json(health))
}
