use axum::Json;
use serde_json::{json, Value};

pub async fn get() -> Json<Value> {
    Json(json!({"status": "ok"}))
}
