use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

pub const MAX_PAGE_SIZE: u32 = 1000;

pub fn clamp_limit(requested: u32) -> u32 {
    requested.min(MAX_PAGE_SIZE)
}

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

pub fn paginated<T: Serialize>(
    mut items: Vec<T>,
    limit: u32,
    offset: u32,
    total_count: usize,
) -> ListEnvelope<T> {
    let has_more = items.len() > limit as usize;
    if has_more {
        items.truncate(limit as usize);
    }
    ListEnvelope {
        data: items,
        total: total_count,
        limit,
        offset,
        has_more,
    }
}

pub fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_caps_at_max_page_size() {
        assert_eq!(clamp_limit(50), 50);
        assert_eq!(clamp_limit(1000), 1000);
        assert_eq!(clamp_limit(5000), MAX_PAGE_SIZE);
        assert_eq!(clamp_limit(u32::MAX), MAX_PAGE_SIZE);
    }

    #[test]
    fn paginated_total_reflects_actual_count() {
        let items: Vec<i32> = (0..101).collect();
        let envelope = paginated(items, 100, 0, 5000);
        assert_eq!(envelope.total, 5000);
        assert_eq!(envelope.data.len(), 100);
        assert!(envelope.has_more);
        assert_eq!(envelope.limit, 100);
        assert_eq!(envelope.offset, 0);
    }

    #[test]
    fn paginated_no_more_items() {
        let items: Vec<i32> = (0..50).collect();
        let envelope = paginated(items, 100, 0, 50);
        assert_eq!(envelope.total, 50);
        assert_eq!(envelope.data.len(), 50);
        assert!(!envelope.has_more);
    }

    #[test]
    fn paginated_with_offset() {
        let items: Vec<i32> = (0..51).collect();
        let envelope = paginated(items, 50, 200, 1000);
        assert_eq!(envelope.total, 1000);
        assert_eq!(envelope.data.len(), 50);
        assert!(envelope.has_more);
        assert_eq!(envelope.offset, 200);
    }

    #[test]
    fn paginated_exact_limit_no_has_more() {
        let items: Vec<i32> = (0..100).collect();
        let envelope = paginated(items, 100, 0, 100);
        assert_eq!(envelope.total, 100);
        assert_eq!(envelope.data.len(), 100);
        assert!(!envelope.has_more);
    }
}
