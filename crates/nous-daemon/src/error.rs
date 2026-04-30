use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use nous_core::error::NousError;

pub struct AppError(NousError);

impl From<NousError> for AppError {
    fn from(err: NousError) -> Self {
        Self(err)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            NousError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            NousError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            NousError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
            NousError::Config(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
            NousError::Sqlite(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal database error".to_string(),
            ),
            NousError::CyclicLink(msg) => (StatusCode::CONFLICT, msg.clone()),
            NousError::NoLinkedRoom(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            NousError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        };

        let body = serde_json::json!({ "error": message });
        (status, Json(body)).into_response()
    }
}
