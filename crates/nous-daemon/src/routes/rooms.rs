use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct CreateRoomBody {
    pub name: String,
    pub purpose: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ListRoomsQuery {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Deserialize)]
pub struct DeleteRoomQuery {
    #[serde(default)]
    pub force: bool,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateRoomBody>,
) -> Result<impl IntoResponse, AppError> {
    let room = nous_core::rooms::create_room(
        &state.pool,
        &body.name,
        body.purpose.as_deref(),
        body.metadata.as_ref(),
    )
    .await?;
    Ok(ApiResponse::created(room))
}

pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<ListRoomsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let rooms = nous_core::rooms::list_rooms(&state.pool, params.include_archived).await?;
    let total = rooms.len();
    Ok(crate::response::ListEnvelope {
        data: rooms,
        total,
        limit: total as u32,
        offset: 0,
        has_more: false,
    })
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let room = nous_core::rooms::get_room(&state.pool, &id).await?;
    Ok(ApiResponse::ok(room))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteRoomQuery>,
) -> Result<impl IntoResponse, AppError> {
    let room = nous_core::rooms::get_room(&state.pool, &id).await?;
    nous_core::rooms::delete_room(&state.pool, &room.id, params.force).await?;
    Ok(crate::response::no_content())
}
