use axum::extract::{Query, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use nous_core::messages::{search_messages, SearchMessagesRequest};

use crate::error::AppError;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub room_id: Option<String>,
    pub limit: Option<u32>,
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<impl IntoResponse, AppError> {
    let results = search_messages(
        &state.pool,
        SearchMessagesRequest {
            query: params.q,
            room_id: params.room_id,
            limit: params.limit,
        },
    )
    .await?;
    Ok(Json(results))
}
