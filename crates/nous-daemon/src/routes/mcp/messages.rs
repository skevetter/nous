use serde_json::Value;

use nous_core::messages::{
    get_thread, list_mentions, mark_read, post_message, read_messages, search_messages,
    unread_count, GetThreadRequest, MarkReadRequest, MessageType, PostMessageRequest,
    ReadMessagesRequest, SearchMessagesRequest, UnreadCountRequest,
};
use nous_core::notifications::{
    room_wait, room_wait_persistent, subscribe_to_room, unsubscribe_from_room,
};

use crate::state::AppState;

use super::{require_str, ToolSchema};

pub fn schemas() -> Vec<ToolSchema> {
    vec![
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
                    "metadata": { "type": "object" },
                    "message_type": { "type": "string", "enum": ["user", "system", "task_event", "command", "handoff"], "default": "user" }
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
            description: "Wait for a new message in a room. When agent_id is provided, checks persistent notification queue first.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "topics": { "type": "array", "items": { "type": "string" } },
                    "agent_id": { "type": "string", "description": "Agent ID for persistent wait (checks queued notifications first)" }
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
        ToolSchema {
            name: "room_mentions",
            description: "List messages mentioning a specific agent in a room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string", "description": "Room ID" },
                    "agent_id": { "type": "string", "description": "Agent ID to search mentions for" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["room_id", "agent_id"]
            }),
        },
        ToolSchema {
            name: "room_thread_view",
            description: "View a message thread (root message and all replies)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "root_message_id": { "type": "string" }
                },
                "required": ["room_id", "root_message_id"]
            }),
        },
        ToolSchema {
            name: "room_mark_read",
            description: "Mark messages as read up to a given message ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "agent_id": { "type": "string" },
                    "message_id": { "type": "string" }
                },
                "required": ["room_id", "agent_id", "message_id"]
            }),
        },
        ToolSchema {
            name: "room_unread_count",
            description: "Get unread message count for an agent in a room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "agent_id": { "type": "string" }
                },
                "required": ["room_id", "agent_id"]
            }),
        },
    ]
}

pub async fn dispatch(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "room_post_message" => Some(handle_post_message(args, state).await),
        "room_read_messages" => Some(handle_read_messages(args, state).await),
        "room_search" => Some(handle_search(args, state).await),
        "room_wait" => Some(handle_wait(args, state).await),
        "room_subscribe" => Some(handle_subscribe(args, state).await),
        "room_unsubscribe" => Some(handle_unsubscribe(args, state).await),
        "room_mentions" => Some(handle_mentions(args, state).await),
        "room_thread_view" => Some(handle_thread_view(args, state).await),
        "room_mark_read" => Some(handle_mark_read(args, state).await),
        "room_unread_count" => Some(handle_unread_count(args, state).await),
        _ => None,
    }
}

async fn handle_post_message(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?.to_string();
    let sender_id = require_str(args, "sender_id")?.to_string();
    let content = require_str(args, "content")?.to_string();
    let reply_to = args
        .get("reply_to")
        .and_then(|v| v.as_str())
        .map(String::from);
    let metadata = args.get("metadata").filter(|v| !v.is_null()).cloned();
    let message_type = args
        .get("message_type")
        .and_then(|v| v.as_str())
        .map(str::parse::<MessageType>)
        .transpose()
        .map_err(nous_core::error::NousError::Validation)?;
    let msg = post_message(
        &state.pool,
        PostMessageRequest {
            room_id,
            sender_id,
            content,
            reply_to,
            metadata,
            message_type,
        },
        Some(&state.registry),
    )
    .await?;
    Ok(serde_json::to_value(msg).unwrap())
}

async fn handle_read_messages(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?.to_string();
    let since = args.get("since").and_then(|v| v.as_str()).map(String::from);
    let before = args
        .get("before")
        .and_then(|v| v.as_str())
        .map(String::from);
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
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

async fn handle_search(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let query = require_str(args, "query")?.to_string();
    let room_id = args
        .get("room_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
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

async fn handle_wait(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?;
    let timeout_ms = args.get("timeout_ms").and_then(serde_json::Value::as_u64);
    let topics: Option<Vec<String>> =
        args.get("topics").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
    let agent_id = args.get("agent_id").and_then(|v| v.as_str());
    let result = if let Some(aid) = agent_id {
        room_wait_persistent(
            &state.pool,
            &state.registry,
            room_id,
            aid,
            timeout_ms,
            topics.as_deref(),
        )
        .await?
    } else {
        room_wait(&state.registry, room_id, timeout_ms, topics.as_deref()).await?
    };
    Ok(serde_json::to_value(result).unwrap())
}

async fn handle_subscribe(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_unsubscribe(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?;
    let agent_id = require_str(args, "agent_id")?;
    unsubscribe_from_room(&state.pool, room_id, agent_id).await?;
    Ok(serde_json::json!({"unsubscribed": true}))
}

async fn handle_mentions(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?;
    let agent_id = require_str(args, "agent_id")?;
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let messages = list_mentions(&state.pool, room_id, agent_id, limit).await?;
    Ok(serde_json::to_value(messages).unwrap())
}

async fn handle_thread_view(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?.to_string();
    let root_message_id = require_str(args, "root_message_id")?.to_string();
    let thread = get_thread(
        &state.pool,
        GetThreadRequest {
            room_id,
            root_message_id,
        },
    )
    .await?;
    Ok(serde_json::to_value(thread).unwrap())
}

async fn handle_mark_read(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?.to_string();
    let agent_id = require_str(args, "agent_id")?.to_string();
    let message_id = require_str(args, "message_id")?.to_string();
    let cursor = mark_read(
        &state.pool,
        MarkReadRequest {
            room_id,
            agent_id,
            message_id,
        },
    )
    .await?;
    Ok(serde_json::to_value(cursor).unwrap())
}

async fn handle_unread_count(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?.to_string();
    let agent_id = require_str(args, "agent_id")?.to_string();
    let cursor = unread_count(&state.pool, UnreadCountRequest { room_id, agent_id }).await?;
    Ok(serde_json::to_value(cursor).unwrap())
}
