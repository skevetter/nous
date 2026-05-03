use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use nous_core::messages::{post_message, read_messages, PostMessageRequest, ReadMessagesRequest};

use crate::error::AppError;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct PostMessageBody {
    pub sender_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct ReadMessagesQuery {
    pub since: Option<String>,
    pub before: Option<String>,
    pub limit: Option<u32>,
}

pub async fn post(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Json(body): Json<PostMessageBody>,
) -> Result<impl IntoResponse, AppError> {
    let room = nous_core::rooms::get_room(&state.pool, &room_id).await?;
    let msg = post_message(
        &state.pool,
        PostMessageRequest {
            room_id: room.id,
            sender_id: body.sender_id,
            content: body.content,
            reply_to: body.reply_to,
            metadata: body.metadata,
            message_type: None,
        },
        Some(&state.registry),
    )
    .await?;
    Ok(ApiResponse::created(msg))
}

pub async fn read(
    State(state): State<AppState>,
    Path(room_id): Path<String>,
    Query(params): Query<ReadMessagesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let room = nous_core::rooms::get_room(&state.pool, &room_id).await?;
    let messages = read_messages(
        &state.pool,
        ReadMessagesRequest {
            room_id: room.id,
            since: params.since,
            before: params.before,
            limit: params.limit,
        },
    )
    .await?;
    Ok(ApiResponse::ok(messages))
}
