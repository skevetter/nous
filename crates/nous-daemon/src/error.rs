use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use nous_core::error::NousError;
use serde::Serialize;

pub struct AppError(NousError);

impl From<NousError> for AppError {
    fn from(err: NousError) -> Self {
        Self(err)
    }
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

impl AppError {
    fn code_and_status(&self) -> (StatusCode, &'static str, String) {
        match &self.0 {
            NousError::Validation(msg) => {
                (StatusCode::BAD_REQUEST, "validation_error", msg.clone())
            }
            NousError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            NousError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            NousError::Config(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "config_error", msg.clone())
            }
            NousError::Sqlite(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                "internal database error".to_string(),
            ),
            NousError::SeaOrm(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                "internal database error".to_string(),
            ),
            NousError::CyclicLink(msg) => (StatusCode::CONFLICT, "cyclic_link", msg.clone()),
            NousError::NoLinkedRoom(msg) => {
                (StatusCode::BAD_REQUEST, "no_linked_room", msg.clone())
            }
            NousError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg.clone())
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.code_and_status();
        let body = ErrorEnvelope {
            error: ErrorBody { code, message },
        };
        (status, Json(body)).into_response()
    }
}
