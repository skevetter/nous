use crate::response::ApiResponse;
use serde_json::json;

pub async fn get() -> ApiResponse<serde_json::Value> {
    ApiResponse::ok(json!({"status": "ok"}))
}
