use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use nous_core::messages::{
    post_message, read_messages, search_messages, PostMessageRequest, ReadMessagesRequest,
    SearchMessagesRequest,
};
use nous_core::notifications::{room_wait, subscribe_to_room, unsubscribe_from_room};
use nous_core::rooms::{create_room, delete_room, get_room, list_rooms};

use crate::error::AppError;
use crate::state::AppState;

#[derive(Serialize)]
struct ToolSchema {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
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

pub async fn list_tools() -> impl IntoResponse {
    let tools = vec![
        ToolSchema {
            name: "room_create",
            description: "Create a new chat room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Room name" },
                    "purpose": { "type": "string", "description": "Room purpose" },
                    "metadata": { "type": "object", "description": "Arbitrary metadata" }
                },
                "required": ["name"]
            }),
        },
        ToolSchema {
            name: "room_list",
            description: "List chat rooms",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "include_archived": { "type": "boolean", "default": false }
                }
            }),
        },
        ToolSchema {
            name: "room_get",
            description: "Get a room by ID or name",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Room ID or name" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "room_delete",
            description: "Delete a room (soft archive by default)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Room ID" },
                    "hard": { "type": "boolean", "default": false }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "room_post_message",
            description: "Post a message to a room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "sender_id": { "type": "string" },
                    "content": { "type": "string" },
                    "reply_to": { "type": "string" },
                    "metadata": { "type": "object" }
                },
                "required": ["room_id", "sender_id", "content"]
            }),
        },
        ToolSchema {
            name: "room_read_messages",
            description: "Read messages from a room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "since": { "type": "string" },
                    "before": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["room_id"]
            }),
        },
        ToolSchema {
            name: "room_search",
            description: "Search messages using full-text search",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "room_id": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
            name: "room_wait",
            description: "Wait for a new message in a room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "topics": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["room_id"]
            }),
        },
        ToolSchema {
            name: "room_subscribe",
            description: "Subscribe an agent to a room's notifications",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "agent_id": { "type": "string" },
                    "topics": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["room_id", "agent_id"]
            }),
        },
        ToolSchema {
            name: "room_unsubscribe",
            description: "Unsubscribe an agent from a room's notifications",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "agent_id": { "type": "string" }
                },
                "required": ["room_id", "agent_id"]
            }),
        },
    ];

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

fn require_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, nous_core::error::NousError> {
    args.get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            nous_core::error::NousError::Validation(format!("missing required field: {field}"))
        })
}

async fn dispatch(
    state: &AppState,
    name: &str,
    args: &Value,
) -> Result<Value, nous_core::error::NousError> {
    match name {
        "room_create" => {
            let name = require_str(args, "name")?;
            let purpose = args.get("purpose").and_then(|v| v.as_str());
            let metadata = args.get("metadata").filter(|v| !v.is_null());
            let room = create_room(&state.pool, name, purpose, metadata).await?;
            Ok(serde_json::to_value(room).unwrap())
        }
        "room_list" => {
            let include_archived = args
                .get("include_archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let rooms = list_rooms(&state.pool, include_archived).await?;
            Ok(serde_json::to_value(rooms).unwrap())
        }
        "room_get" => {
            let id = require_str(args, "id")?;
            let room = get_room(&state.pool, id).await?;
            Ok(serde_json::to_value(room).unwrap())
        }
        "room_delete" => {
            let id = require_str(args, "id")?;
            let hard = args.get("hard").and_then(|v| v.as_bool()).unwrap_or(false);
            delete_room(&state.pool, id, hard).await?;
            Ok(serde_json::json!({"deleted": true}))
        }
        "room_post_message" => {
            let room_id = require_str(args, "room_id")?.to_string();
            let sender_id = require_str(args, "sender_id")?.to_string();
            let content = require_str(args, "content")?.to_string();
            let reply_to = args
                .get("reply_to")
                .and_then(|v| v.as_str())
                .map(String::from);
            let metadata = args.get("metadata").filter(|v| !v.is_null()).cloned();
            let msg = post_message(
                &state.pool,
                PostMessageRequest {
                    room_id,
                    sender_id,
                    content,
                    reply_to,
                    metadata,
                },
                Some(&state.registry),
            )
            .await?;
            Ok(serde_json::to_value(msg).unwrap())
        }
        "room_read_messages" => {
            let room_id = require_str(args, "room_id")?.to_string();
            let since = args.get("since").and_then(|v| v.as_str()).map(String::from);
            let before = args
                .get("before")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let messages = read_messages(
                &state.pool,
                ReadMessagesRequest {
                    room_id,
                    since,
                    before,
                    limit,
                },
            )
            .await?;
            Ok(serde_json::to_value(messages).unwrap())
        }
        "room_search" => {
            let query = require_str(args, "query")?.to_string();
            let room_id = args
                .get("room_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let results = search_messages(
                &state.pool,
                SearchMessagesRequest {
                    query,
                    room_id,
                    limit,
                },
            )
            .await?;
            Ok(serde_json::to_value(results).unwrap())
        }
        "room_wait" => {
            let room_id = require_str(args, "room_id")?;
            let timeout_ms = args.get("timeout_ms").and_then(|v| v.as_u64());
            let topics: Option<Vec<String>> =
                args.get("topics").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
            let result = room_wait(&state.registry, room_id, timeout_ms, topics.as_deref()).await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "room_subscribe" => {
            let room_id = require_str(args, "room_id")?;
            let agent_id = require_str(args, "agent_id")?;
            let topics: Option<Vec<String>> =
                args.get("topics").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
            subscribe_to_room(&state.pool, room_id, agent_id, topics).await?;
            Ok(serde_json::json!({"subscribed": true}))
        }
        "room_unsubscribe" => {
            let room_id = require_str(args, "room_id")?;
            let agent_id = require_str(args, "agent_id")?;
            unsubscribe_from_room(&state.pool, room_id, agent_id).await?;
            Ok(serde_json::json!({"unsubscribed": true}))
        }
        _ => Err(nous_core::error::NousError::Validation(format!(
            "unknown tool: {name}"
        ))),
    }
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
