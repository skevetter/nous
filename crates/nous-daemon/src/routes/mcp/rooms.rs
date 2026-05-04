use serde_json::Value;

use nous_core::rooms::{create_room, delete_room, get_room, list_rooms};

use crate::state::AppState;

use super::{require_str, ToolSchema};

pub fn schemas() -> Vec<ToolSchema> {
    vec![
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
                    "force": { "type": "boolean", "default": false }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "room_unarchive",
            description: "Re-activate an archived room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Room ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "room_inspect",
            description: "Get room stats: message count, last message timestamp, subscriber count",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Room ID" }
                },
                "required": ["id"]
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
        "room_create" => Some(handle_room_create(args, state).await),
        "room_list" => Some(handle_room_list(args, state).await),
        "room_get" => Some(handle_room_get(args, state).await),
        "room_delete" => Some(handle_room_delete(args, state).await),
        "room_unarchive" => Some(handle_room_unarchive(args, state).await),
        "room_inspect" => Some(handle_room_inspect(args, state).await),
        _ => None,
    }
}

async fn handle_room_create(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let name = require_str(args, "name")?;
    let purpose = args.get("purpose").and_then(|v| v.as_str());
    let metadata = args.get("metadata").filter(|v| !v.is_null());
    let room = create_room(&state.pool, name, purpose, metadata).await?;
    Ok(serde_json::to_value(room).unwrap())
}

async fn handle_room_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let include_archived = args
        .get("include_archived")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let rooms = list_rooms(&state.pool, include_archived).await?;
    Ok(serde_json::to_value(rooms).unwrap())
}

async fn handle_room_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let room = get_room(&state.pool, id).await?;
    Ok(serde_json::to_value(room).unwrap())
}

async fn handle_room_delete(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let force = args.get("force").and_then(serde_json::Value::as_bool).unwrap_or(false);
    delete_room(&state.pool, id, force).await?;
    Ok(serde_json::json!({"deleted": true}))
}

async fn handle_room_unarchive(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let room = nous_core::rooms::unarchive_room(&state.pool, id).await?;
    Ok(serde_json::to_value(room).unwrap())
}

async fn handle_room_inspect(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let room_stats = nous_core::rooms::inspect_room(&state.pool, id).await?;
    Ok(serde_json::to_value(room_stats).unwrap())
}
