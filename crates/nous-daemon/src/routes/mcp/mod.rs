mod agents;
mod memory;
mod messages;
mod resources;
mod rooms;
mod tasks;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AppError;
use crate::state::AppState;

#[derive(Serialize, Clone)]
pub struct ToolSchema {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[derive(Deserialize)]
pub struct ToolCallRequest {
    pub name: String,
    pub arguments: Value,
}

#[derive(Serialize)]
pub struct ToolCallResponse {
    pub content: Vec<ToolContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Serialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: &'static str,
    pub text: String,
}

pub fn get_tool_schemas() -> Vec<ToolSchema> {
    let mut schemas = Vec::new();
    schemas.extend(rooms::schemas());
    schemas.extend(messages::schemas());
    schemas.extend(tasks::schemas());
    schemas.extend(agents::schemas());
    schemas.extend(resources::schemas());
    schemas.extend(memory::schemas());
    schemas
}

pub async fn list_tools() -> impl IntoResponse {
    let tools = get_tool_schemas();
    Json(serde_json::json!({ "tools": tools }))
}

pub async fn call_tool(
    State(state): State<AppState>,
    Json(req): Json<ToolCallRequest>,
) -> Result<impl IntoResponse, AppError> {
    let result = dispatch(&state, &req.name, &req.arguments).await;

    match result {
        Ok(value) => Ok(Json(ToolCallResponse {
            content: vec![ToolContent {
                content_type: "text",
                text: serde_json::to_string(&value).unwrap_or_default(),
            }],
            is_error: None,
        })),
        Err(e) => {
            tracing::error!(tool = %req.name, error = %e, "MCP tool call failed");
            let (_, msg) = error_to_parts(&e);
            Ok(Json(ToolCallResponse {
                content: vec![ToolContent {
                    content_type: "text",
                    text: msg,
                }],
                is_error: Some(true),
            }))
        }
    }
}

pub(crate) fn require_str<'a>(
    args: &'a Value,
    field: &str,
) -> Result<&'a str, nous_core::error::NousError> {
    args.get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            nous_core::error::NousError::Validation(format!("missing required field: {field}"))
        })
}

pub async fn dispatch(
    state: &AppState,
    name: &str,
    args: &Value,
) -> Result<Value, nous_core::error::NousError> {
    if let Some(result) = rooms::dispatch(name, args, state).await {
        return result;
    }
    if let Some(result) = messages::dispatch(name, args, state).await {
        return result;
    }
    if let Some(result) = tasks::dispatch(name, args, state).await {
        return result;
    }
    if let Some(result) = agents::dispatch(name, args, state).await {
        return result;
    }
    if let Some(result) = resources::dispatch(name, args, state).await {
        return result;
    }
    if let Some(result) = memory::dispatch(name, args, state).await {
        return result;
    }

    Err(nous_core::error::NousError::Validation(format!(
        "unknown tool: {name}"
    )))
}

fn error_to_parts(err: &nous_core::error::NousError) -> (StatusCode, String) {
    match err {
        nous_core::error::NousError::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
        nous_core::error::NousError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
        nous_core::error::NousError::Conflict(msg) => (StatusCode::CONFLICT, msg.clone()),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal error".to_string(),
        ),
    }
}
