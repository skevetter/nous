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

#[derive(Serialize)]
pub struct ListEnvelope<T: Serialize> {
    pub data: Vec<T>,
    pub total: usize,
    pub limit: u32,
    pub offset: u32,
    pub has_more: bool,
}

impl<T: Serialize> IntoResponse for ListEnvelope<T> {
    fn into_response(self) -> Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

pub fn paginated<T: Serialize>(mut items: Vec<T>, limit: u32, offset: u32) -> ListEnvelope<T> {
    let has_more = items.len() > limit as usize;
    if has_more {
        items.truncate(limit as usize);
    }
    let total = items.len();
    ListEnvelope {
        data: items,
        total,
        limit,
        offset,
        has_more,
    }
}

pub fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}
