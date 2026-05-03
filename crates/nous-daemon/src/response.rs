use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
struct DataEnvelope<T: Serialize> {
    data: T,
}

pub struct ApiResponse<T: Serialize>(StatusCode, T);

impl<T: Serialize> ApiResponse<T> {
    pub fn ok(data: T) -> Self {
        Self(StatusCode::OK, data)
    }

    pub fn created(data: T) -> Self {
        Self(StatusCode::CREATED, data)
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        let envelope = DataEnvelope { data: self.1 };
        (self.0, Json(envelope)).into_response()
    }
}

pub fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}
