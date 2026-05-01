use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use nous_core::agents;
use nous_core::inventory;
use nous_core::memory;
use nous_core::messages::{
    post_message, read_messages, search_messages, PostMessageRequest, ReadMessagesRequest,
    SearchMessagesRequest,
};
use nous_core::notifications::{room_wait, subscribe_to_room, unsubscribe_from_room};
use nous_core::rooms::{create_room, delete_room, get_room, list_rooms};
use nous_core::schedules;
use nous_core::tasks;
use nous_core::worktrees;

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
        ToolSchema {
            name: "task_create",
            description: "Create a new task",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Task title" },
                    "description": { "type": "string", "description": "Task description" },
                    "priority": { "type": "string", "description": "Priority: low, medium, high, critical" },
                    "assignee_id": { "type": "string", "description": "Assignee agent ID" },
                    "labels": { "type": "array", "items": { "type": "string" }, "description": "Labels" },
                    "room_id": { "type": "string", "description": "Existing room ID for discussion" },
                    "create_room": { "type": "boolean", "description": "Create a new room for this task" }
                },
                "required": ["title"]
            }),
        },
        ToolSchema {
            name: "task_list",
            description: "List tasks with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status" },
                    "assignee_id": { "type": "string", "description": "Filter by assignee" },
                    "label": { "type": "string", "description": "Filter by label" },
                    "limit": { "type": "integer", "description": "Max results" },
                    "offset": { "type": "integer", "description": "Offset for pagination" }
                }
            }),
        },
        ToolSchema {
            name: "task_get",
            description: "Get a task by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Task ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "task_update",
            description: "Update a task's fields",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Task ID" },
                    "status": { "type": "string", "description": "New status" },
                    "priority": { "type": "string", "description": "New priority" },
                    "assignee_id": { "type": "string", "description": "New assignee" },
                    "description": { "type": "string", "description": "New description" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "task_close",
            description: "Close a task",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Task ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "task_link",
            description: "Create a link between two tasks",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string", "description": "Source task ID" },
                    "target_id": { "type": "string", "description": "Target task ID" },
                    "link_type": { "type": "string", "description": "Link type: blocked_by, parent, related_to" }
                },
                "required": ["source_id", "target_id", "link_type"]
            }),
        },
        ToolSchema {
            name: "task_unlink",
            description: "Remove a link between two tasks",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string", "description": "Source task ID" },
                    "target_id": { "type": "string", "description": "Target task ID" },
                    "link_type": { "type": "string", "description": "Link type: blocked_by, parent, related_to" }
                },
                "required": ["source_id", "target_id", "link_type"]
            }),
        },
        ToolSchema {
            name: "task_list_links",
            description: "List all links for a task",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Task ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "task_add_note",
            description: "Add a note to a task's discussion room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Task ID" },
                    "sender_id": { "type": "string", "description": "Sender agent ID" },
                    "content": { "type": "string", "description": "Note content" }
                },
                "required": ["id", "sender_id", "content"]
            }),
        },
        ToolSchema {
            name: "worktree_create",
            description: "Create a new git worktree",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "branch": { "type": "string", "description": "Branch name for the worktree" },
                    "slug": { "type": "string", "description": "Optional slug identifier" },
                    "repo_root": { "type": "string", "description": "Repository root path (defaults to '.')" },
                    "agent_id": { "type": "string", "description": "Agent ID to associate" },
                    "task_id": { "type": "string", "description": "Task ID to associate" }
                },
                "required": ["branch"]
            }),
        },
        ToolSchema {
            name: "worktree_list",
            description: "List worktrees with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "description": "Filter by status: active, stale, archived, deleted" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "task_id": { "type": "string", "description": "Filter by task ID" },
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "worktree_get",
            description: "Get a worktree by ID or slug",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Worktree ID or slug" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "worktree_archive",
            description: "Archive a worktree (removes git worktree, sets status to archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Worktree ID or slug" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "worktree_delete",
            description: "Delete a worktree (removes directory, sets status to deleted)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Worktree ID or slug" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_register",
            description: "Register a new agent in the org hierarchy",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Agent name (unique within namespace)" },
                    "type": { "type": "string", "description": "Agent type: engineer, manager, director, senior-manager" },
                    "parent_id": { "type": "string", "description": "Parent agent ID" },
                    "namespace": { "type": "string", "description": "Namespace (defaults to 'default')" },
                    "room": { "type": "string", "description": "Room name for this agent" },
                    "metadata": { "type": "string", "description": "JSON metadata string" },
                    "status": { "type": "string", "description": "Initial status" }
                },
                "required": ["name", "type"]
            }),
        },
        ToolSchema {
            name: "agent_deregister",
            description: "Deregister an agent (remove from registry)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" },
                    "cascade": { "type": "boolean", "description": "Cascade delete children" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_lookup",
            description: "Look up an agent by name",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Agent name" },
                    "namespace": { "type": "string", "description": "Namespace" }
                },
                "required": ["name"]
            }),
        },
        ToolSchema {
            name: "agent_list",
            description: "List agents with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "status": { "type": "string", "description": "Filter by status" },
                    "type": { "type": "string", "description": "Filter by agent type" },
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "agent_list_children",
            description: "List direct children of an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Parent agent ID" },
                    "namespace": { "type": "string", "description": "Namespace" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_list_ancestors",
            description: "List ancestors of an agent (root to parent)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" },
                    "namespace": { "type": "string", "description": "Namespace" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_tree",
            description: "Get the agent tree (hierarchy)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "root_id": { "type": "string", "description": "Root agent ID (omit for full tree)" },
                    "namespace": { "type": "string", "description": "Namespace" }
                }
            }),
        },
        ToolSchema {
            name: "agent_heartbeat",
            description: "Send a heartbeat for an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" },
                    "status": { "type": "string", "description": "New status: running, idle, blocked, done" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_search",
            description: "Search agents by name/metadata using FTS5",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "namespace": { "type": "string", "description": "Namespace" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
            name: "agent_stale",
            description: "List stale agents (past heartbeat threshold)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "threshold": { "type": "integer", "description": "Threshold in seconds (default 900)" },
                    "namespace": { "type": "string", "description": "Namespace" }
                }
            }),
        },
        ToolSchema {
            name: "agent_inspect",
            description: "Inspect an agent: full details with current version and template info",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_versions",
            description: "List version history for an agent (newest first)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "limit": { "type": "integer", "description": "Max results (default 20)" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_record_version",
            description: "Record a new version for an agent (skill hashes + config hash). Clears upgrade_available flag.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "skill_hash": { "type": "string", "description": "SHA-256 of concatenated skill file contents" },
                    "config_hash": { "type": "string", "description": "SHA-256 of effective config" },
                    "skills_json": { "type": "string", "description": "JSON array: [{name, path, hash}]" }
                },
                "required": ["agent_id", "skill_hash", "config_hash"]
            }),
        },
        ToolSchema {
            name: "agent_rollback",
            description: "Rollback an agent to a previous version (advisory — agent must restart with old skills)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "version_id": { "type": "string", "description": "Target version ID to rollback to" }
                },
                "required": ["agent_id", "version_id"]
            }),
        },
        ToolSchema {
            name: "agent_notify_upgrade",
            description: "Set upgrade_available flag for an agent (advisory notification)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_outdated",
            description: "List agents with upgrade_available flag set",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "agent_template_create",
            description: "Create an immutable agent template (blueprint for spawning agents)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Template name (unique)" },
                    "type": { "type": "string", "description": "Template type (e.g. engineer, reviewer, monitor)" },
                    "default_config": { "type": "string", "description": "Default config JSON" },
                    "skill_refs": { "type": "string", "description": "JSON array of skill file paths or names" }
                },
                "required": ["name", "type"]
            }),
        },
        ToolSchema {
            name: "agent_template_list",
            description: "List agent templates",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Filter by template type" },
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "agent_template_get",
            description: "Get an agent template by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Template ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "agent_instantiate",
            description: "Create a new agent from a template with optional config overrides",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "template_id": { "type": "string", "description": "Template ID" },
                    "name": { "type": "string", "description": "Agent name override" },
                    "namespace": { "type": "string", "description": "Namespace" },
                    "parent_id": { "type": "string", "description": "Parent agent ID" },
                    "config_overrides": { "type": "string", "description": "Config overrides JSON (merged with template defaults)" }
                },
                "required": ["template_id"]
            }),
        },
        ToolSchema {
            name: "artifact_register",
            description: "Register a new artifact owned by an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Owning agent ID" },
                    "type": { "type": "string", "description": "Artifact type: worktree, room, schedule, branch" },
                    "name": { "type": "string", "description": "Artifact name" },
                    "path": { "type": "string", "description": "Optional path" },
                    "namespace": { "type": "string", "description": "Namespace" }
                },
                "required": ["agent_id", "type", "name"]
            }),
        },
        ToolSchema {
            name: "artifact_deregister",
            description: "Deregister (delete) an artifact",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Artifact ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "artifact_list",
            description: "List artifacts with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Filter by owning agent ID" },
                    "type": { "type": "string", "description": "Filter by artifact type" },
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "inventory_register",
            description: "Register a new inventory item (P5 artifact registry)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Item name" },
                    "type": { "type": "string", "description": "Artifact type: worktree, room, schedule, branch, file, docker-image, binary" },
                    "owner_agent_id": { "type": "string", "description": "Owning agent ID (optional)" },
                    "path": { "type": "string", "description": "Filesystem or logical path" },
                    "namespace": { "type": "string", "description": "Namespace (default: 'default')" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for discovery" },
                    "metadata": { "type": "string", "description": "JSON metadata (type-specific fields)" }
                },
                "required": ["name", "type"]
            }),
        },
        ToolSchema {
            name: "inventory_list",
            description: "List inventory items with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Filter by artifact type" },
                    "status": { "type": "string", "description": "Filter by status: active, archived, deleted" },
                    "owner_agent_id": { "type": "string", "description": "Filter by owner agent ID" },
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "orphaned": { "type": "boolean", "description": "Show only orphaned (unowned) items" },
                    "limit": { "type": "integer", "description": "Max results (default: 50)" }
                }
            }),
        },
        ToolSchema {
            name: "inventory_get",
            description: "Get an inventory item by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Item ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "inventory_update",
            description: "Update an inventory item (name, path, tags, metadata, status)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Item ID" },
                    "name": { "type": "string", "description": "New name" },
                    "path": { "type": "string", "description": "New path" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "New tags (replaces existing)" },
                    "metadata": { "type": "string", "description": "New JSON metadata (replaces existing)" },
                    "status": { "type": "string", "description": "New status: active, archived, deleted" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "inventory_search",
            description: "Search inventory by tags (AND semantics: item must have ALL specified tags)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to search (AND semantics)" },
                    "type": { "type": "string", "description": "Filter by artifact type" },
                    "status": { "type": "string", "description": "Filter by status" },
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["tags"]
            }),
        },
        ToolSchema {
            name: "inventory_archive",
            description: "Archive an active inventory item (status: active → archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Item ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "inventory_deregister",
            description: "Deregister an inventory item (soft-delete by default, hard=true removes row)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Item ID" },
                    "hard": { "type": "boolean", "description": "Hard delete (remove from DB entirely)" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "memory_save",
            description: "Save a new memory (persistent structured observation). If topic_key matches an existing active memory, it updates instead.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short searchable title" },
                    "content": { "type": "string", "description": "Structured content (use **What**, **Why**, **Where**, **Learned** format)" },
                    "type": { "type": "string", "description": "Memory type: decision, convention, bugfix, architecture, fact, observation" },
                    "importance": { "type": "string", "description": "Importance: low, moderate, high (default: moderate)" },
                    "agent_id": { "type": "string", "description": "Agent ID that created this memory" },
                    "workspace_id": { "type": "string", "description": "Workspace scope (default: 'default')" },
                    "topic_key": { "type": "string", "description": "Topic key for upsert (e.g. 'architecture/auth-model')" },
                    "valid_from": { "type": "string", "description": "ISO-8601 start of validity" },
                    "valid_until": { "type": "string", "description": "ISO-8601 end of validity" }
                },
                "required": ["title", "content", "type"]
            }),
        },
        ToolSchema {
            name: "memory_search",
            description: "Search memories using full-text search (FTS5)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "workspace_id": { "type": "string", "description": "Filter by workspace" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "type": { "type": "string", "description": "Filter by memory type" },
                    "importance": { "type": "string", "description": "Filter by importance" },
                    "include_archived": { "type": "boolean", "description": "Include archived memories (default: false)" },
                    "limit": { "type": "integer", "description": "Max results (default: 20)" }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
            name: "memory_get",
            description: "Get a memory by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "memory_update",
            description: "Update a memory (title, content, importance, topic_key, archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory ID" },
                    "title": { "type": "string", "description": "New title" },
                    "content": { "type": "string", "description": "New content" },
                    "importance": { "type": "string", "description": "New importance" },
                    "topic_key": { "type": "string", "description": "New topic key" },
                    "valid_from": { "type": "string", "description": "New valid_from" },
                    "valid_until": { "type": "string", "description": "New valid_until" },
                    "archived": { "type": "boolean", "description": "Set archived state" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "memory_relate",
            description: "Create a relation between two memories (supersedes, conflicts_with, related, compatible, scoped, not_conflict)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string", "description": "Source memory ID" },
                    "target_id": { "type": "string", "description": "Target memory ID" },
                    "relation_type": { "type": "string", "description": "Relation type: supersedes, conflicts_with, related, compatible, scoped, not_conflict" }
                },
                "required": ["source_id", "target_id", "relation_type"]
            }),
        },
        ToolSchema {
            name: "memory_context",
            description: "Get recent memories as context (ordered by recency, non-archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "workspace_id": { "type": "string", "description": "Filter by workspace" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "topic_key": { "type": "string", "description": "Filter by topic key" },
                    "limit": { "type": "integer", "description": "Max results (default: 20)" }
                }
            }),
        },
        ToolSchema {
            name: "memory_relations",
            description: "List all relations for a memory",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Memory ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "schedule_create",
            description: "Create a new schedule",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Schedule name" },
                    "cron_expr": { "type": "string", "description": "Cron expression (5-field or @hourly/@daily/@weekly/@monthly/@yearly)" },
                    "trigger_at": { "type": "integer", "description": "One-shot trigger timestamp (overrides cron_expr)" },
                    "timezone": { "type": "string", "description": "Timezone (default: UTC)" },
                    "action_type": { "type": "string", "description": "Action type: mcp_tool, shell, http" },
                    "action_payload": { "type": "string", "description": "JSON action payload" },
                    "desired_outcome": { "type": "string", "description": "Expected output substring or /regex/" },
                    "max_retries": { "type": "integer", "description": "Max retry attempts (default: 3)" },
                    "timeout_secs": { "type": "integer", "description": "Per-run timeout in seconds" },
                    "max_runs": { "type": "integer", "description": "Max run history (default: 100, use 1 for one-shot)" }
                },
                "required": ["name", "cron_expr", "action_type", "action_payload"]
            }),
        },
        ToolSchema {
            name: "schedule_get",
            description: "Get a schedule by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Schedule ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "schedule_list",
            description: "List schedules with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean", "description": "Filter by enabled state" },
                    "action_type": { "type": "string", "description": "Filter by action type" },
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "schedule_update",
            description: "Update a schedule",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Schedule ID" },
                    "name": { "type": "string", "description": "New name" },
                    "cron_expr": { "type": "string", "description": "New cron expression" },
                    "trigger_at": { "type": ["integer", "null"], "description": "One-shot trigger timestamp (null to clear)" },
                    "enabled": { "type": "boolean", "description": "Enable/disable" },
                    "action_type": { "type": "string", "description": "New action type" },
                    "action_payload": { "type": "string", "description": "New action payload" },
                    "desired_outcome": { "type": ["string", "null"], "description": "Expected output (null to clear)" },
                    "max_retries": { "type": "integer", "description": "New max retries" },
                    "timeout_secs": { "type": "integer", "description": "New timeout" },
                    "max_runs": { "type": "integer", "description": "New max runs" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "schedule_delete",
            description: "Delete a schedule",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Schedule ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "schedule_runs_list",
            description: "List runs for a schedule",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "schedule_id": { "type": "string", "description": "Schedule ID" },
                    "status": { "type": "string", "description": "Filter by status" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["schedule_id"]
            }),
        },
        ToolSchema {
            name: "schedule_health",
            description: "Get schedule health overview",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolSchema {
            name: "task_depends_add",
            description: "Add a dependency between tasks (execution-order constraint)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID that has the dependency" },
                    "depends_on_task_id": { "type": "string", "description": "Task ID it depends on" },
                    "dep_type": { "type": "string", "description": "Dependency type: blocked_by, blocks, waiting_on (default: blocked_by)" }
                },
                "required": ["task_id", "depends_on_task_id"]
            }),
        },
        ToolSchema {
            name: "task_depends_remove",
            description: "Remove a dependency between tasks",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID" },
                    "depends_on_task_id": { "type": "string", "description": "Depends-on task ID" },
                    "dep_type": { "type": "string", "description": "Dependency type (default: blocked_by)" }
                },
                "required": ["task_id", "depends_on_task_id"]
            }),
        },
        ToolSchema {
            name: "task_depends_list",
            description: "List all dependencies for a task",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task ID" }
                },
                "required": ["task_id"]
            }),
        },
        ToolSchema {
            name: "task_template_create",
            description: "Create a reusable task template",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Template name (unique)" },
                    "title_pattern": { "type": "string", "description": "Title pattern (use {{var}} for variables)" },
                    "description_template": { "type": "string", "description": "Description template" },
                    "default_priority": { "type": "string", "description": "Default priority: critical, high, medium, low" },
                    "default_labels": { "type": "array", "items": { "type": "string" }, "description": "Default labels" },
                    "checklist": { "type": "array", "items": { "type": "string" }, "description": "Checklist items" }
                },
                "required": ["name", "title_pattern"]
            }),
        },
        ToolSchema {
            name: "task_template_list",
            description: "List task templates",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max results" }
                }
            }),
        },
        ToolSchema {
            name: "task_template_get",
            description: "Get a task template by ID or name",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Template ID or name" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "task_template_use",
            description: "Create a task from a template",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "template_id": { "type": "string", "description": "Template ID or name" },
                    "title_vars": { "type": "object", "description": "Variables to substitute in title pattern (key: value)" },
                    "description": { "type": "string", "description": "Override description" },
                    "assignee_id": { "type": "string", "description": "Override assignee" },
                    "labels": { "type": "array", "items": { "type": "string" }, "description": "Override labels" }
                },
                "required": ["template_id"]
            }),
        },
        ToolSchema {
            name: "task_batch_close",
            description: "Batch close multiple tasks",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_ids": { "type": "array", "items": { "type": "string" }, "description": "List of task IDs to close" }
                },
                "required": ["task_ids"]
            }),
        },
        ToolSchema {
            name: "task_batch_update_status",
            description: "Batch update status of multiple tasks",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_ids": { "type": "array", "items": { "type": "string" }, "description": "List of task IDs" },
                    "status": { "type": "string", "description": "New status: open, in_progress, done, closed" }
                },
                "required": ["task_ids", "status"]
            }),
        },
        ToolSchema {
            name: "task_batch_assign",
            description: "Batch assign multiple tasks to an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_ids": { "type": "array", "items": { "type": "string" }, "description": "List of task IDs" },
                    "assignee_id": { "type": "string", "description": "Assignee agent ID" }
                },
                "required": ["task_ids", "assignee_id"]
            }),
        },
        ToolSchema {
            name: "memory_store_embedding",
            description: "Store a pre-computed embedding vector for a memory",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Embedding vector (array of f32)" }
                },
                "required": ["memory_id", "embedding"]
            }),
        },
        ToolSchema {
            name: "memory_search_similar",
            description: "Search memories by cosine similarity to a query embedding vector",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Query embedding vector (array of f32)" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" },
                    "workspace_id": { "type": "string", "description": "Filter by workspace" },
                    "threshold": { "type": "number", "description": "Minimum similarity threshold (default: 0.0)" }
                },
                "required": ["embedding"]
            }),
        },
        ToolSchema {
            name: "memory_chunk",
            description: "Chunk text for a given memory_id and store chunks (no embedding)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID to chunk" },
                    "chunk_size": { "type": "integer", "description": "Tokens per chunk (default: 256)" },
                    "overlap": { "type": "integer", "description": "Overlap tokens (default: 64)" }
                },
                "required": ["memory_id"]
            }),
        },
        ToolSchema {
            name: "memory_embed",
            description: "Generate embeddings for all chunks of a memory_id using local ONNX model",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID whose chunks to embed" }
                },
                "required": ["memory_id"]
            }),
        },
        ToolSchema {
            name: "memory_search_hybrid",
            description: "Hybrid search: FTS + vector similarity + RRF reranking. Auto-embeds query text internally.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query text" },
                    "limit": { "type": "integer", "description": "Max results (default: 10)" },
                    "workspace_id": { "type": "string", "description": "Filter by workspace ID" },
                    "agent_id": { "type": "string", "description": "Filter by agent ID" },
                    "memory_type": { "type": "string", "description": "Filter by memory type (decision, convention, bugfix, architecture, fact, observation)" }
                },
                "required": ["query"]
            }),
        },
        ToolSchema {
            name: "memory_store_with_embedding",
            description: "Full pipeline: chunk text, generate embeddings, and store (all-in-one)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "memory_id": { "type": "string", "description": "Memory ID to process" },
                    "chunk_size": { "type": "integer", "description": "Tokens per chunk (default: 256)" },
                    "overlap": { "type": "integer", "description": "Overlap tokens (default: 64)" }
                },
                "required": ["memory_id"]
            }),
        },
        ToolSchema {
            name: "memory_search_stats",
            description: "Get search analytics: total searches, type breakdown, zero-result rate, avg latency, top queries",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "since": { "type": "string", "description": "ISO datetime string to filter events (created_at >= since)" }
                }
            }),
        },
        ToolSchema {
            name: "memory_session_start",
            description: "Start a new memory session (creates a session record for grouping memories)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "project": { "type": "string", "description": "Project name" }
                }
            }),
        },
        ToolSchema {
            name: "memory_session_end",
            description: "End an active memory session (sets ended_at)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session ID to end" }
                },
                "required": ["session_id"]
            }),
        },
        ToolSchema {
            name: "memory_session_summary",
            description: "Save a summary to a session record and create a session_summary memory",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session ID" },
                    "summary": { "type": "string", "description": "Session summary text" },
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "workspace_id": { "type": "string", "description": "Workspace ID" }
                },
                "required": ["session_id", "summary"]
            }),
        },
        ToolSchema {
            name: "memory_save_prompt",
            description: "Save a user prompt as a memory linked to the active session",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string", "description": "The user prompt text" },
                    "session_id": { "type": "string", "description": "Active session ID to link to" },
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "workspace_id": { "type": "string", "description": "Workspace ID" }
                },
                "required": ["prompt"]
            }),
        },
        ToolSchema {
            name: "memory_current_project",
            description: "Detect the current project from a directory by looking for Cargo.toml, package.json, go.mod, etc.",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "cwd": { "type": "string", "description": "Directory path to detect project from (defaults to '.')" }
                }
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
        ToolSchema {
            name: "agent_bulk_deregister",
            description: "Deregister multiple agents by ID list",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "List of agent IDs to deregister" },
                    "cascade": { "type": "boolean", "description": "Cascade delete children (default: false)" }
                },
                "required": ["ids"]
            }),
        },
        ToolSchema {
            name: "agent_update_status",
            description: "Update an agent's status field directly",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" },
                    "status": { "type": "string", "description": "New status: active, inactive, archived, running, idle, blocked, done" }
                },
                "required": ["id", "status"]
            }),
        },
        ToolSchema {
            name: "artifact_update",
            description: "Update artifact fields (name, path)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Artifact ID" },
                    "name": { "type": "string", "description": "New name" },
                    "path": { "type": "string", "description": "New path" }
                },
                "required": ["id"]
            }),
        },
        // --- NOUS-026: Agent lifecycle management tools ---
        ToolSchema {
            name: "agent_spawn",
            description: "Spawn an agent process (claude, shell, or http)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID to spawn process for" },
                    "command": { "type": "string", "description": "Command to run (overrides agent config)" },
                    "process_type": { "type": "string", "description": "Process type: claude, shell, http (default: shell)" },
                    "working_dir": { "type": "string", "description": "Working directory" },
                    "env": { "type": "object", "description": "Environment variables as key-value pairs" },
                    "timeout_secs": { "type": "integer", "description": "Process timeout in seconds" },
                    "restart_policy": { "type": "string", "description": "Restart policy: never, on-failure, always (default: never)" },
                    "max_restarts": { "type": "integer", "description": "Max restart attempts (default: 3)" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_stop",
            description: "Stop a running agent process (SIGTERM then SIGKILL after grace period)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "force": { "type": "boolean", "description": "Force kill immediately (SIGKILL)" },
                    "grace_secs": { "type": "integer", "description": "Grace period before SIGKILL (default: 10)" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_restart",
            description: "Stop and re-spawn an agent process",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "command": { "type": "string", "description": "New command (overrides previous)" },
                    "working_dir": { "type": "string", "description": "New working directory" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_invoke",
            description: "Send work to an agent (executes prompt synchronously or async)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "prompt": { "type": "string", "description": "Work prompt/command to execute" },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default: 300)" },
                    "metadata": { "type": "object", "description": "Arbitrary metadata for the invocation" },
                    "async": { "type": "boolean", "description": "If true, return immediately with invocation ID" }
                },
                "required": ["agent_id", "prompt"]
            }),
        },
        ToolSchema {
            name: "agent_invoke_result",
            description: "Get the result of an async invocation",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "invocation_id": { "type": "string", "description": "Invocation ID" }
                },
                "required": ["invocation_id"]
            }),
        },
        ToolSchema {
            name: "agent_invocations",
            description: "List invocation history for an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "status": { "type": "string", "description": "Filter by status: pending, running, completed, failed, timeout, cancelled" },
                    "limit": { "type": "integer", "description": "Max results (default: 20)" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_process_status",
            description: "Get current process info (PID, uptime, DB record) for an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_logs",
            description: "Get recent stdout/stderr and process history for an agent",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string", "description": "Agent ID" },
                    "limit": { "type": "integer", "description": "Max process records (default: 5)" }
                },
                "required": ["agent_id"]
            }),
        },
        ToolSchema {
            name: "agent_update",
            description: "Update agent config (process_type, spawn_command, working_dir, auto_restart, metadata)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" },
                    "process_type": { "type": "string", "description": "Process type: claude, shell, http" },
                    "spawn_command": { "type": "string", "description": "Default spawn command" },
                    "working_dir": { "type": "string", "description": "Default working directory" },
                    "auto_restart": { "type": "boolean", "description": "Auto-restart on crash" },
                    "metadata": { "type": "string", "description": "JSON metadata string" }
                },
                "required": ["id"]
            }),
        },
    ]
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

pub async fn dispatch(
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
        "task_create" => {
            let title = require_str(args, "title")?;
            let description = args.get("description").and_then(|v| v.as_str());
            let priority = args.get("priority").and_then(|v| v.as_str());
            let assignee_id = args.get("assignee_id").and_then(|v| v.as_str());
            let labels: Option<Vec<String>> =
                args.get("labels").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
            let room_id = args.get("room_id").and_then(|v| v.as_str());
            let create_room_flag = args
                .get("create_room")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let task = tasks::create_task(
                &state.pool,
                title,
                description,
                priority,
                assignee_id,
                labels.as_deref(),
                room_id,
                create_room_flag,
                None,
            )
            .await?;
            Ok(serde_json::to_value(task).unwrap())
        }
        "task_list" => {
            let status = args.get("status").and_then(|v| v.as_str());
            let assignee_id = args.get("assignee_id").and_then(|v| v.as_str());
            let label = args.get("label").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let offset = args
                .get("offset")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let result = tasks::list_tasks(
                &state.pool,
                status,
                assignee_id,
                label,
                limit,
                offset,
                None,
                None,
            )
            .await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "task_get" => {
            let id = require_str(args, "id")?;
            let task = tasks::get_task(&state.pool, id).await?;
            Ok(serde_json::to_value(task).unwrap())
        }
        "task_update" => {
            let id = require_str(args, "id")?;
            let status = args.get("status").and_then(|v| v.as_str());
            let priority = args.get("priority").and_then(|v| v.as_str());
            let assignee_id = args.get("assignee_id").and_then(|v| v.as_str());
            let description = args.get("description").and_then(|v| v.as_str());
            let task = tasks::update_task(
                &state.pool,
                id,
                status,
                priority,
                assignee_id,
                description,
                None,
                None,
            )
            .await?;
            Ok(serde_json::to_value(task).unwrap())
        }
        "task_close" => {
            let id = require_str(args, "id")?;
            let task = tasks::close_task(&state.pool, id, None).await?;
            Ok(serde_json::to_value(task).unwrap())
        }
        "task_link" => {
            let source_id = require_str(args, "source_id")?;
            let target_id = require_str(args, "target_id")?;
            let link_type = require_str(args, "link_type")?;
            let link =
                tasks::link_tasks(&state.pool, source_id, target_id, link_type, None).await?;
            Ok(serde_json::to_value(link).unwrap())
        }
        "task_unlink" => {
            let source_id = require_str(args, "source_id")?;
            let target_id = require_str(args, "target_id")?;
            let link_type = require_str(args, "link_type")?;
            tasks::unlink_tasks(&state.pool, source_id, target_id, link_type, None).await?;
            Ok(serde_json::json!({"unlinked": true}))
        }
        "task_list_links" => {
            let id = require_str(args, "id")?;
            let links = tasks::list_links(&state.pool, id).await?;
            Ok(serde_json::to_value(links).unwrap())
        }
        "task_add_note" => {
            let id = require_str(args, "id")?;
            let sender_id = require_str(args, "sender_id")?;
            let content = require_str(args, "content")?;
            let msg = tasks::add_note(&state.pool, id, sender_id, content).await?;
            Ok(msg)
        }
        "worktree_create" => {
            let branch = require_str(args, "branch")?.to_string();
            let slug = args.get("slug").and_then(|v| v.as_str()).map(String::from);
            let repo_root = args
                .get("repo_root")
                .and_then(|v| v.as_str())
                .unwrap_or(".")
                .to_string();
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let task_id = args
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let wt = worktrees::create(
                &state.pool,
                worktrees::CreateWorktreeRequest {
                    slug,
                    branch,
                    repo_root,
                    agent_id,
                    task_id,
                },
            )
            .await?;
            Ok(serde_json::to_value(wt).unwrap())
        }
        "worktree_list" => {
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<worktrees::WorktreeStatus>())
                .transpose()?;
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let task_id = args
                .get("task_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let wts = worktrees::list(
                &state.pool,
                worktrees::ListWorktreesFilter {
                    status,
                    agent_id,
                    task_id,
                    repo_root: None,
                    limit,
                    offset: None,
                },
            )
            .await?;
            Ok(serde_json::to_value(wts).unwrap())
        }
        "worktree_get" => {
            let id = require_str(args, "id")?;
            let wt = worktrees::get(&state.pool, id).await?;
            Ok(serde_json::to_value(wt).unwrap())
        }
        "worktree_archive" => {
            let id = require_str(args, "id")?;
            let wt = worktrees::archive(&state.pool, id).await?;
            Ok(serde_json::to_value(wt).unwrap())
        }
        "worktree_delete" => {
            let id = require_str(args, "id")?;
            worktrees::delete(&state.pool, id).await?;
            Ok(serde_json::json!({"deleted": true}))
        }
        "agent_register" => {
            let name = require_str(args, "name")?.to_string();
            let agent_type_str = require_str(args, "type")?;
            let agent_type: agents::AgentType = agent_type_str.parse()?;
            let parent_id = args
                .get("parent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let room = args.get("room").and_then(|v| v.as_str()).map(String::from);
            let metadata = args
                .get("metadata")
                .and_then(|v| v.as_str())
                .map(String::from);
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            let agent = agents::register_agent(
                &state.pool,
                agents::RegisterAgentRequest {
                    name,
                    agent_type,
                    parent_id,
                    namespace,
                    room,
                    metadata,
                    status,
                },
            )
            .await?;
            Ok(serde_json::to_value(agent).unwrap())
        }
        "agent_deregister" => {
            let id = require_str(args, "id")?;
            let cascade = args
                .get("cascade")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let result = agents::deregister_agent(&state.pool, id, cascade).await?;
            Ok(serde_json::json!({"result": result}))
        }
        "agent_lookup" => {
            let name = require_str(args, "name")?;
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let agent = agents::lookup_agent(&state.pool, name, namespace).await?;
            Ok(serde_json::to_value(agent).unwrap())
        }
        "agent_list" => {
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            let agent_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<agents::AgentType>())
                .transpose()?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let list = agents::list_agents(
                &state.pool,
                &agents::ListAgentsFilter {
                    namespace,
                    status,
                    agent_type,
                    limit,
                    ..Default::default()
                },
            )
            .await?;
            Ok(serde_json::to_value(list).unwrap())
        }
        "agent_list_children" => {
            let id = require_str(args, "id")?;
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let children = agents::list_children(&state.pool, id, namespace).await?;
            Ok(serde_json::to_value(children).unwrap())
        }
        "agent_list_ancestors" => {
            let id = require_str(args, "id")?;
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let ancestors = agents::list_ancestors(&state.pool, id, namespace).await?;
            Ok(serde_json::to_value(ancestors).unwrap())
        }
        "agent_tree" => {
            let root_id = args.get("root_id").and_then(|v| v.as_str());
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let tree = agents::get_tree(&state.pool, root_id, namespace).await?;
            Ok(serde_json::to_value(tree).unwrap())
        }
        "agent_heartbeat" => {
            let id = require_str(args, "id")?;
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<agents::AgentStatus>())
                .transpose()?;
            agents::heartbeat(&state.pool, id, status).await?;
            Ok(serde_json::json!({"ok": true}))
        }
        "agent_search" => {
            let query = require_str(args, "query")?;
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let results = agents::search_agents(&state.pool, query, namespace, limit).await?;
            Ok(serde_json::to_value(results).unwrap())
        }
        "agent_stale" => {
            let threshold = args
                .get("threshold")
                .and_then(|v| v.as_u64())
                .unwrap_or(900);
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let stale = agents::list_stale_agents(&state.pool, threshold, namespace).await?;
            Ok(serde_json::to_value(stale).unwrap())
        }
        "agent_inspect" => {
            let id = require_str(args, "id")?;
            let inspection = agents::inspect_agent(&state.pool, id).await?;
            Ok(serde_json::to_value(inspection).unwrap())
        }
        "agent_versions" => {
            let agent_id = require_str(args, "agent_id")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let versions = agents::list_versions(&state.pool, agent_id, limit).await?;
            Ok(serde_json::to_value(versions).unwrap())
        }
        "agent_record_version" => {
            let agent_id = require_str(args, "agent_id")?.to_string();
            let skill_hash = require_str(args, "skill_hash")?.to_string();
            let config_hash = require_str(args, "config_hash")?.to_string();
            let skills_json = args
                .get("skills_json")
                .and_then(|v| v.as_str())
                .map(String::from);
            let version = agents::record_version(
                &state.pool,
                agents::RecordVersionRequest {
                    agent_id,
                    skill_hash,
                    config_hash,
                    skills_json,
                },
            )
            .await?;
            Ok(serde_json::to_value(version).unwrap())
        }
        "agent_rollback" => {
            let agent_id = require_str(args, "agent_id")?;
            let version_id = require_str(args, "version_id")?;
            let version = agents::rollback_agent(&state.pool, agent_id, version_id).await?;
            Ok(serde_json::to_value(version).unwrap())
        }
        "agent_notify_upgrade" => {
            let id = require_str(args, "id")?;
            agents::set_upgrade_available(&state.pool, id, true).await?;
            Ok(serde_json::json!({"notified": true}))
        }
        "agent_outdated" => {
            let namespace = args.get("namespace").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let outdated = agents::list_outdated_agents(&state.pool, namespace, limit).await?;
            Ok(serde_json::to_value(outdated).unwrap())
        }
        "agent_template_create" => {
            let name = require_str(args, "name")?.to_string();
            let template_type = require_str(args, "type")?.to_string();
            let default_config = args
                .get("default_config")
                .and_then(|v| v.as_str())
                .map(String::from);
            let skill_refs = args
                .get("skill_refs")
                .and_then(|v| v.as_str())
                .map(String::from);
            let template = agents::create_template(
                &state.pool,
                agents::CreateTemplateRequest {
                    name,
                    template_type,
                    default_config,
                    skill_refs,
                },
            )
            .await?;
            Ok(serde_json::to_value(template).unwrap())
        }
        "agent_template_list" => {
            let template_type = args.get("type").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let templates = agents::list_templates(&state.pool, template_type, limit).await?;
            Ok(serde_json::to_value(templates).unwrap())
        }
        "agent_template_get" => {
            let id = require_str(args, "id")?;
            let template = agents::get_template_by_id(&state.pool, id).await?;
            Ok(serde_json::to_value(template).unwrap())
        }
        "agent_instantiate" => {
            let template_id = require_str(args, "template_id")?.to_string();
            let name = args.get("name").and_then(|v| v.as_str()).map(String::from);
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let parent_id = args
                .get("parent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let config_overrides = args
                .get("config_overrides")
                .and_then(|v| v.as_str())
                .map(String::from);
            let agent = agents::instantiate_from_template(
                &state.pool,
                agents::InstantiateRequest {
                    template_id,
                    name,
                    namespace,
                    parent_id,
                    config_overrides,
                },
            )
            .await?;
            Ok(serde_json::to_value(agent).unwrap())
        }
        "artifact_register" => {
            let agent_id = require_str(args, "agent_id")?.to_string();
            let artifact_type_str = require_str(args, "type")?;
            let artifact_type: agents::ArtifactType = artifact_type_str.parse()?;
            let name = require_str(args, "name")?.to_string();
            let path = args.get("path").and_then(|v| v.as_str()).map(String::from);
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let artifact = agents::register_artifact(
                &state.pool,
                agents::RegisterArtifactRequest {
                    agent_id,
                    artifact_type,
                    name,
                    path,
                    namespace,
                },
            )
            .await?;
            Ok(serde_json::to_value(artifact).unwrap())
        }
        "artifact_deregister" => {
            let id = require_str(args, "id")?;
            agents::deregister_artifact(&state.pool, id).await?;
            Ok(serde_json::json!({"ok": true}))
        }
        "artifact_list" => {
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let artifact_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<agents::ArtifactType>())
                .transpose()?;
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let artifacts = agents::list_artifacts(
                &state.pool,
                &agents::ListArtifactsFilter {
                    agent_id,
                    artifact_type,
                    namespace,
                    limit,
                    ..Default::default()
                },
            )
            .await?;
            Ok(serde_json::to_value(artifacts).unwrap())
        }
        "schedule_create" => {
            let clock = schedules::SystemClock;
            let name = require_str(args, "name")?;
            let cron_expr = require_str(args, "cron_expr")?;
            let action_type = require_str(args, "action_type")?;
            let action_payload = require_str(args, "action_payload")?;
            let trigger_at = args.get("trigger_at").and_then(|v| v.as_i64());
            let timezone = args.get("timezone").and_then(|v| v.as_str());
            let desired_outcome = args.get("desired_outcome").and_then(|v| v.as_str());
            let max_retries = args
                .get("max_retries")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let timeout_secs = args
                .get("timeout_secs")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let max_runs = args
                .get("max_runs")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let schedule = schedules::create_schedule(
                &state.pool,
                name,
                cron_expr,
                trigger_at,
                timezone,
                action_type,
                action_payload,
                desired_outcome,
                max_retries,
                timeout_secs,
                None,
                max_runs,
                &clock,
            )
            .await?;
            Ok(serde_json::to_value(schedule).unwrap())
        }
        "schedule_get" => {
            let id = require_str(args, "id")?;
            let schedule = schedules::get_schedule(&state.pool, id).await?;
            Ok(serde_json::to_value(schedule).unwrap())
        }
        "schedule_list" => {
            let enabled = args.get("enabled").and_then(|v| v.as_bool());
            let action_type = args.get("action_type").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let list = schedules::list_schedules(&state.pool, enabled, action_type, limit).await?;
            Ok(serde_json::to_value(list).unwrap())
        }
        "schedule_update" => {
            let clock = schedules::SystemClock;
            let id = require_str(args, "id")?;
            let name = args.get("name").and_then(|v| v.as_str());
            let cron_expr = args.get("cron_expr").and_then(|v| v.as_str());
            let trigger_at = if args.get("trigger_at").is_some() {
                let val = args["trigger_at"].as_i64();
                Some(val)
            } else {
                None
            };
            let enabled = args.get("enabled").and_then(|v| v.as_bool());
            let action_type = args.get("action_type").and_then(|v| v.as_str());
            let action_payload = args.get("action_payload").and_then(|v| v.as_str());
            let desired_outcome = if args.get("desired_outcome").is_some() {
                Some(args["desired_outcome"].as_str())
            } else {
                None
            };
            let max_retries = args
                .get("max_retries")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let timeout_secs = args
                .get("timeout_secs")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let max_runs = args
                .get("max_runs")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32);
            let schedule = schedules::update_schedule(
                &state.pool,
                id,
                name,
                cron_expr,
                trigger_at,
                enabled,
                action_type,
                action_payload,
                desired_outcome,
                max_retries,
                timeout_secs.map(Some),
                max_runs,
                &clock,
            )
            .await?;
            Ok(serde_json::to_value(schedule).unwrap())
        }
        "schedule_delete" => {
            let id = require_str(args, "id")?;
            schedules::delete_schedule(&state.pool, id).await?;
            Ok(serde_json::json!({"deleted": true}))
        }
        "schedule_runs_list" => {
            let schedule_id = require_str(args, "schedule_id")?;
            let status = args.get("status").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let runs = schedules::list_runs(&state.pool, schedule_id, status, limit).await?;
            Ok(serde_json::to_value(runs).unwrap())
        }
        "schedule_health" => {
            let health = schedules::schedule_health(&state.pool).await?;
            Ok(health)
        }
        "inventory_register" => {
            let name = require_str(args, "name")?.to_string();
            let type_str = require_str(args, "type")?;
            let artifact_type: inventory::InventoryType = type_str.parse()?;
            let owner_agent_id = args
                .get("owner_agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let path = args.get("path").and_then(|v| v.as_str()).map(String::from);
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let metadata = args
                .get("metadata")
                .and_then(|v| v.as_str())
                .map(String::from);
            let tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });
            let item = inventory::register_item(
                &state.pool,
                inventory::RegisterItemRequest {
                    name,
                    artifact_type,
                    owner_agent_id,
                    namespace,
                    path,
                    metadata,
                    tags,
                },
            )
            .await?;
            Ok(serde_json::to_value(item).unwrap())
        }
        "inventory_list" => {
            let artifact_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<inventory::InventoryType>())
                .transpose()?;
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<inventory::InventoryStatus>())
                .transpose()?;
            let owner_agent_id = args
                .get("owner_agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let orphaned = args.get("orphaned").and_then(|v| v.as_bool());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let items = inventory::list_items(
                &state.pool,
                &inventory::ListItemsFilter {
                    artifact_type,
                    status,
                    owner_agent_id,
                    namespace,
                    orphaned,
                    limit,
                    ..Default::default()
                },
            )
            .await?;
            Ok(serde_json::to_value(items).unwrap())
        }
        "inventory_get" => {
            let id = require_str(args, "id")?;
            let item = inventory::get_item_by_id(&state.pool, id).await?;
            Ok(serde_json::to_value(item).unwrap())
        }
        "inventory_update" => {
            let id = require_str(args, "id")?.to_string();
            let name = args.get("name").and_then(|v| v.as_str()).map(String::from);
            let path = args.get("path").and_then(|v| v.as_str()).map(String::from);
            let metadata = args
                .get("metadata")
                .and_then(|v| v.as_str())
                .map(String::from);
            let tags = args.get("tags").and_then(|v| v.as_array()).map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            });
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<inventory::InventoryStatus>())
                .transpose()?;
            let item = inventory::update_item(
                &state.pool,
                inventory::UpdateItemRequest {
                    id,
                    name,
                    path,
                    metadata,
                    tags,
                    status,
                },
            )
            .await?;
            Ok(serde_json::to_value(item).unwrap())
        }
        "inventory_search" => {
            let tags: Vec<String> = args
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let artifact_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<inventory::InventoryType>())
                .transpose()?;
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<inventory::InventoryStatus>())
                .transpose()?;
            let namespace = args
                .get("namespace")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let items = inventory::search_by_tags(
                &state.pool,
                &inventory::SearchItemsRequest {
                    tags,
                    artifact_type,
                    status,
                    namespace,
                    limit,
                },
            )
            .await?;
            Ok(serde_json::to_value(items).unwrap())
        }
        "inventory_archive" => {
            let id = require_str(args, "id")?;
            let item = inventory::archive_item(&state.pool, id).await?;
            Ok(serde_json::to_value(item).unwrap())
        }
        "inventory_deregister" => {
            let id = require_str(args, "id")?;
            let hard = args.get("hard").and_then(|v| v.as_bool()).unwrap_or(false);
            inventory::deregister_item(&state.pool, id, hard).await?;
            Ok(serde_json::json!({"ok": true}))
        }
        "memory_save" => {
            let title = require_str(args, "title")?.to_string();
            let content = require_str(args, "content")?.to_string();
            let type_str = require_str(args, "type")?;
            let memory_type: memory::MemoryType = type_str.parse()?;
            let importance = args
                .get("importance")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<memory::Importance>())
                .transpose()?;
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let workspace_id = args
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let topic_key = args
                .get("topic_key")
                .and_then(|v| v.as_str())
                .map(String::from);
            let valid_from = args
                .get("valid_from")
                .and_then(|v| v.as_str())
                .map(String::from);
            let valid_until = args
                .get("valid_until")
                .and_then(|v| v.as_str())
                .map(String::from);
            let mem = memory::save_memory(
                &state.pool,
                memory::SaveMemoryRequest {
                    workspace_id,
                    agent_id,
                    title,
                    content,
                    memory_type,
                    importance,
                    topic_key,
                    valid_from,
                    valid_until,
                },
            )
            .await?;
            Ok(serde_json::to_value(mem).unwrap())
        }
        "memory_search" => {
            let query = require_str(args, "query")?.to_string();
            let workspace_id = args
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let memory_type = args
                .get("type")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<memory::MemoryType>())
                .transpose()?;
            let importance = args
                .get("importance")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<memory::Importance>())
                .transpose()?;
            let include_archived = args
                .get("include_archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let start = std::time::Instant::now();
            let results = memory::search_memories(
                &state.pool,
                &memory::SearchMemoryRequest {
                    query: query.clone(),
                    workspace_id: workspace_id.clone(),
                    agent_id: agent_id.clone(),
                    memory_type,
                    importance,
                    include_archived,
                    limit,
                },
            )
            .await?;
            let latency_ms = start.elapsed().as_millis() as i64;
            let _ = memory::analytics::record_search_event(
                &state.pool,
                &memory::analytics::SearchEvent {
                    query_text: query,
                    search_type: "fts".to_string(),
                    result_count: results.len() as i64,
                    latency_ms,
                    workspace_id,
                    agent_id,
                },
            )
            .await;
            Ok(serde_json::to_value(results).unwrap())
        }
        "memory_get" => {
            let id = require_str(args, "id")?;
            let mem = memory::get_memory_by_id(&state.pool, id).await?;
            Ok(serde_json::to_value(mem).unwrap())
        }
        "memory_update" => {
            let id = require_str(args, "id")?.to_string();
            let title = args.get("title").and_then(|v| v.as_str()).map(String::from);
            let content = args
                .get("content")
                .and_then(|v| v.as_str())
                .map(String::from);
            let importance = args
                .get("importance")
                .and_then(|v| v.as_str())
                .map(|s| s.parse::<memory::Importance>())
                .transpose()?;
            let topic_key = args
                .get("topic_key")
                .and_then(|v| v.as_str())
                .map(String::from);
            let valid_from = args
                .get("valid_from")
                .and_then(|v| v.as_str())
                .map(String::from);
            let valid_until = args
                .get("valid_until")
                .and_then(|v| v.as_str())
                .map(String::from);
            let archived = args.get("archived").and_then(|v| v.as_bool());
            let mem = memory::update_memory(
                &state.pool,
                memory::UpdateMemoryRequest {
                    id,
                    title,
                    content,
                    importance,
                    topic_key,
                    valid_from,
                    valid_until,
                    archived,
                },
            )
            .await?;
            Ok(serde_json::to_value(mem).unwrap())
        }
        "memory_relate" => {
            let source_id = require_str(args, "source_id")?.to_string();
            let target_id = require_str(args, "target_id")?.to_string();
            let relation_type_str = require_str(args, "relation_type")?;
            let relation_type: memory::RelationType = relation_type_str.parse()?;
            let rel = memory::relate_memories(
                &state.pool,
                &memory::RelateRequest {
                    source_id,
                    target_id,
                    relation_type,
                },
            )
            .await?;
            Ok(serde_json::to_value(rel).unwrap())
        }
        "memory_context" => {
            let workspace_id = args
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let topic_key = args
                .get("topic_key")
                .and_then(|v| v.as_str())
                .map(String::from);
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let results = memory::get_context(
                &state.pool,
                &memory::ContextRequest {
                    workspace_id,
                    agent_id,
                    topic_key,
                    limit,
                },
            )
            .await?;
            Ok(serde_json::to_value(results).unwrap())
        }
        "memory_relations" => {
            let id = require_str(args, "id")?;
            let relations = memory::list_relations(&state.pool, id).await?;
            Ok(serde_json::to_value(relations).unwrap())
        }
        "task_depends_add" => {
            let task_id = require_str(args, "task_id")?;
            let depends_on_task_id = require_str(args, "depends_on_task_id")?;
            let dep_type = args.get("dep_type").and_then(|v| v.as_str());
            let dep =
                tasks::add_dependency(&state.pool, task_id, depends_on_task_id, dep_type).await?;
            Ok(serde_json::to_value(dep).unwrap())
        }
        "task_depends_remove" => {
            let task_id = require_str(args, "task_id")?;
            let depends_on_task_id = require_str(args, "depends_on_task_id")?;
            let dep_type = args.get("dep_type").and_then(|v| v.as_str());
            tasks::remove_dependency(&state.pool, task_id, depends_on_task_id, dep_type).await?;
            Ok(serde_json::json!({"removed": true}))
        }
        "task_depends_list" => {
            let task_id = require_str(args, "task_id")?;
            let deps = tasks::list_dependencies(&state.pool, task_id).await?;
            Ok(serde_json::to_value(deps).unwrap())
        }
        "task_template_create" => {
            let name = require_str(args, "name")?;
            let title_pattern = require_str(args, "title_pattern")?;
            let description_template = args.get("description_template").and_then(|v| v.as_str());
            let default_priority = args.get("default_priority").and_then(|v| v.as_str());
            let default_labels: Option<Vec<String>> = args
                .get("default_labels")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
            let checklist: Option<Vec<String>> =
                args.get("checklist").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
            let tmpl = tasks::create_template(
                &state.pool,
                name,
                title_pattern,
                description_template,
                default_priority,
                default_labels.as_deref(),
                checklist.as_deref(),
            )
            .await?;
            Ok(serde_json::to_value(tmpl).unwrap())
        }
        "task_template_list" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let templates = tasks::list_templates(&state.pool, limit).await?;
            Ok(serde_json::to_value(templates).unwrap())
        }
        "task_template_get" => {
            let id = require_str(args, "id")?;
            let tmpl = tasks::get_template(&state.pool, id).await?;
            Ok(serde_json::to_value(tmpl).unwrap())
        }
        "task_template_use" => {
            let template_id = require_str(args, "template_id")?;
            let title_vars: Option<std::collections::HashMap<String, String>> = args
                .get("title_vars")
                .and_then(|v| v.as_object())
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                });
            let description = args.get("description").and_then(|v| v.as_str());
            let assignee_id = args.get("assignee_id").and_then(|v| v.as_str());
            let labels: Option<Vec<String>> =
                args.get("labels").and_then(|v| v.as_array()).map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                });
            let task = tasks::create_from_template(
                &state.pool,
                template_id,
                title_vars.as_ref(),
                description,
                assignee_id,
                labels.as_deref(),
            )
            .await?;
            Ok(serde_json::to_value(task).unwrap())
        }
        "task_batch_close" => {
            let task_ids: Vec<String> = args
                .get("task_ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let result = tasks::batch_close(&state.pool, &task_ids).await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "task_batch_update_status" => {
            let task_ids: Vec<String> = args
                .get("task_ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let status = require_str(args, "status")?;
            let result = tasks::batch_update_status(&state.pool, &task_ids, status).await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "task_batch_assign" => {
            let task_ids: Vec<String> = args
                .get("task_ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let assignee_id = require_str(args, "assignee_id")?;
            let result = tasks::batch_assign(&state.pool, &task_ids, assignee_id).await?;
            Ok(serde_json::to_value(result).unwrap())
        }
        "memory_store_embedding" => {
            let memory_id = require_str(args, "memory_id")?;
            let embedding: Vec<f32> = args
                .get("embedding")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default();
            if embedding.is_empty() {
                return Err(nous_core::error::NousError::Validation(
                    "embedding array cannot be empty".into(),
                ));
            }
            memory::store_embedding(&state.pool, &state.vec_pool, memory_id, &embedding).await?;
            Ok(serde_json::json!({"stored": true}))
        }
        "memory_search_similar" => {
            let embedding: Vec<f32> = args
                .get("embedding")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default();
            if embedding.is_empty() {
                return Err(nous_core::error::NousError::Validation(
                    "embedding array cannot be empty".into(),
                ));
            }
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32)
                .unwrap_or(10);
            let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
            let threshold = args
                .get("threshold")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32);
            let start = std::time::Instant::now();
            let results = memory::search_similar(
                &state.pool,
                &state.vec_pool,
                &embedding,
                limit,
                workspace_id,
                threshold,
            )
            .await?;
            let latency_ms = start.elapsed().as_millis() as i64;
            let _ = memory::analytics::record_search_event(
                &state.pool,
                &memory::analytics::SearchEvent {
                    query_text: String::new(),
                    search_type: "vector".to_string(),
                    result_count: results.len() as i64,
                    latency_ms,
                    workspace_id: workspace_id.map(String::from),
                    agent_id: None,
                },
            )
            .await;
            Ok(serde_json::to_value(results).unwrap())
        }
        "memory_chunk" => {
            let memory_id = require_str(args, "memory_id")?;
            let chunk_size = args
                .get("chunk_size")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(256);
            let overlap = args
                .get("overlap")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(64);

            let mem = memory::get_memory_by_id(&state.pool, memory_id).await?;
            let chunker = memory::Chunker::new(chunk_size, overlap);
            let chunks = chunker.chunk(memory_id, &mem.content);
            memory::store_chunks(&state.vec_pool, &chunks)?;

            Ok(serde_json::json!({
                "memory_id": memory_id,
                "chunk_count": chunks.len(),
                "chunks": chunks
            }))
        }
        "memory_embed" => {
            let memory_id = require_str(args, "memory_id")?;
            let chunks = memory::get_chunks_for_memory(&state.vec_pool, memory_id)?;
            if chunks.is_empty() {
                return Err(nous_core::error::NousError::Validation(
                    "no chunks found for memory_id — run memory_chunk first".into(),
                ));
            }

            let embedder = state.embedder.as_ref().ok_or_else(|| {
                nous_core::error::NousError::Internal(
                    "embedding model not available — run `nous model download` to install".into(),
                )
            })?;
            let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
            let embeddings = embedder.embed(&texts)?;

            for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
                memory::store_chunk_embedding(&state.vec_pool, &chunk.id, embedding)?;
            }

            Ok(serde_json::json!({
                "memory_id": memory_id,
                "chunks_embedded": chunks.len()
            }))
        }
        "memory_search_stats" => {
            let since = args.get("since").and_then(|v| v.as_str());
            let stats = memory::analytics::get_search_stats(&state.pool, since).await?;
            Ok(serde_json::to_value(stats).unwrap())
        }
        "memory_search_hybrid" => {
            let query = require_str(args, "query")?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(10);
            let workspace_id = args
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let agent_id = args
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(String::from);
            let memory_type = args
                .get("memory_type")
                .and_then(|v| v.as_str())
                .and_then(|s| {
                    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
                });

            let query_embedding = state
                .embedder
                .as_ref()
                .and_then(|embedder| embedder.embed(&[query]).ok())
                .and_then(|mut vecs| vecs.pop());

            let start = std::time::Instant::now();

            if let Some(embedding) = query_embedding {
                let results = memory::search_hybrid_filtered(
                    &state.pool,
                    &state.vec_pool,
                    query,
                    &embedding,
                    limit,
                    workspace_id.as_deref(),
                    agent_id.as_deref(),
                    memory_type,
                )
                .await?;
                let latency_ms = start.elapsed().as_millis() as i64;
                let _ = memory::analytics::record_search_event(
                    &state.pool,
                    &memory::analytics::SearchEvent {
                        query_text: query.to_string(),
                        search_type: "hybrid".to_string(),
                        result_count: results.len() as i64,
                        latency_ms,
                        workspace_id,
                        agent_id,
                    },
                )
                .await;
                Ok(serde_json::to_value(results).unwrap())
            } else {
                let fts_results = memory::search_memories(
                    &state.pool,
                    &memory::SearchMemoryRequest {
                        query: query.to_string(),
                        workspace_id: workspace_id.clone(),
                        agent_id: agent_id.clone(),
                        memory_type,
                        importance: None,
                        include_archived: false,
                        limit: Some(limit as u32),
                    },
                )
                .await?;
                let latency_ms = start.elapsed().as_millis() as i64;
                let _ = memory::analytics::record_search_event(
                    &state.pool,
                    &memory::analytics::SearchEvent {
                        query_text: query.to_string(),
                        search_type: "fts5_fallback".to_string(),
                        result_count: fts_results.len() as i64,
                        latency_ms,
                        workspace_id,
                        agent_id,
                    },
                )
                .await;
                Ok(serde_json::json!({
                    "results": fts_results,
                    "_warning": "embedding unavailable, fell back to FTS5-only search"
                }))
            }
        }
        "memory_store_with_embedding" => {
            let memory_id = require_str(args, "memory_id")?;
            let chunk_size = args
                .get("chunk_size")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(256);
            let overlap = args
                .get("overlap")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(64);

            let embedder = state.embedder.as_ref().ok_or_else(|| {
                nous_core::error::NousError::Internal(
                    "embedding model not available — run `nous model download` to install".into(),
                )
            })?;

            let mem = memory::get_memory_by_id(&state.pool, memory_id).await?;

            let chunker = memory::Chunker::new(chunk_size, overlap);
            let chunks = chunker.chunk(memory_id, &mem.content);
            memory::store_chunks(&state.vec_pool, &chunks)?;

            let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
            let embeddings = embedder.embed(&texts)?;

            for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
                memory::store_chunk_embedding(&state.vec_pool, &chunk.id, embedding)?;
            }

            let full_embeddings = embedder.embed(&[&mem.content])?;
            if let Some(full_emb) = full_embeddings.first() {
                memory::store_embedding(&state.pool, &state.vec_pool, memory_id, full_emb).await?;
            }

            Ok(serde_json::json!({
                "memory_id": memory_id,
                "chunk_count": chunks.len(),
                "chunks_embedded": chunks.len(),
                "full_embedding_stored": true
            }))
        }
        "memory_session_start" => {
            let agent_id = args.get("agent_id").and_then(|v| v.as_str());
            let project = args.get("project").and_then(|v| v.as_str());
            let session = memory::session_start(&state.pool, agent_id, project).await?;
            Ok(serde_json::to_value(session).unwrap())
        }
        "memory_session_end" => {
            let session_id = require_str(args, "session_id")?;
            let session = memory::session_end(&state.pool, session_id).await?;
            Ok(serde_json::to_value(session).unwrap())
        }
        "memory_session_summary" => {
            let session_id = require_str(args, "session_id")?;
            let summary = require_str(args, "summary")?;
            let agent_id = args.get("agent_id").and_then(|v| v.as_str());
            let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
            let session =
                memory::session_summary(&state.pool, session_id, summary, agent_id, workspace_id)
                    .await?;
            Ok(serde_json::to_value(session).unwrap())
        }
        "memory_save_prompt" => {
            let prompt = require_str(args, "prompt")?;
            let session_id = args.get("session_id").and_then(|v| v.as_str());
            let agent_id = args.get("agent_id").and_then(|v| v.as_str());
            let workspace_id = args.get("workspace_id").and_then(|v| v.as_str());
            let mem = memory::save_prompt(&state.pool, session_id, agent_id, workspace_id, prompt)
                .await?;
            Ok(serde_json::to_value(mem).unwrap())
        }
        "memory_current_project" => {
            let cwd = args.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
            match memory::detect_current_project(cwd) {
                Some(project) => Ok(serde_json::to_value(project).unwrap()),
                None => {
                    Ok(serde_json::json!({"detected": false, "message": "no project marker found"}))
                }
            }
        }
        "room_unarchive" => {
            let id = require_str(args, "id")?;
            let room = nous_core::rooms::unarchive_room(&state.pool, id).await?;
            Ok(serde_json::to_value(room).unwrap())
        }
        "room_mentions" => {
            let room_id = require_str(args, "room_id")?;
            let agent_id = require_str(args, "agent_id")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let messages =
                nous_core::messages::list_mentions(&state.pool, room_id, agent_id, limit).await?;
            Ok(serde_json::to_value(messages).unwrap())
        }
        "room_inspect" => {
            let id = require_str(args, "id")?;
            let stats = nous_core::rooms::inspect_room(&state.pool, id).await?;
            Ok(serde_json::to_value(stats).unwrap())
        }
        "agent_bulk_deregister" => {
            let ids: Vec<String> = args
                .get("ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let cascade = args
                .get("cascade")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut results = serde_json::Map::new();
            for id in &ids {
                match agents::deregister_agent(&state.pool, id, cascade).await {
                    Ok(result) => {
                        results.insert(id.clone(), serde_json::json!(result));
                    }
                    Err(e) => {
                        results.insert(id.clone(), serde_json::json!({"error": e.to_string()}));
                    }
                }
            }
            Ok(serde_json::Value::Object(results))
        }
        "agent_update_status" => {
            let id = require_str(args, "id")?;
            let status_str = require_str(args, "status")?;
            let status: agents::AgentStatus = status_str.parse()?;
            let agent = agents::update_agent_status(&state.pool, id, status).await?;
            Ok(serde_json::to_value(agent).unwrap())
        }
        "artifact_update" => {
            let id = require_str(args, "id")?;
            let name = args.get("name").and_then(|v| v.as_str());
            let path = args.get("path").and_then(|v| v.as_str());
            let artifact = agents::update_artifact(&state.pool, id, name, path).await?;
            Ok(serde_json::to_value(artifact).unwrap())
        }
        // --- NOUS-026: Agent lifecycle management tools ---
        "agent_spawn" => {
            let agent_id = require_str(args, "agent_id")?;
            let agent = agents::get_agent_by_id(&state.pool, agent_id).await?;
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .or(agent.spawn_command.as_deref())
                .ok_or_else(|| {
                    nous_core::error::NousError::Validation(
                        "command is required (not set on agent config either)".into(),
                    )
                })?
                .to_string();
            let process_type = args
                .get("process_type")
                .and_then(|v| v.as_str())
                .or(agent.process_type.as_deref())
                .unwrap_or("shell");
            let working_dir = args
                .get("working_dir")
                .and_then(|v| v.as_str())
                .or(agent.working_dir.as_deref());
            let env = args.get("env").filter(|v| !v.is_null()).cloned();
            let timeout_secs = args.get("timeout_secs").and_then(|v| v.as_i64());
            let restart_policy = args
                .get("restart_policy")
                .and_then(|v| v.as_str())
                .unwrap_or(if agent.auto_restart { "on-failure" } else { "never" });
            let max_restarts = args
                .get("max_restarts")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32)
                .unwrap_or(3);
            let process = state
                .process_registry
                .spawn(
                    state,
                    agent_id,
                    &command,
                    process_type,
                    working_dir,
                    env,
                    timeout_secs,
                    restart_policy,
                    max_restarts,
                )
                .await?;
            Ok(serde_json::to_value(process).unwrap())
        }
        "agent_stop" => {
            let agent_id = require_str(args, "agent_id")?;
            let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
            let grace_secs = args
                .get("grace_secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(10);
            let process = state
                .process_registry
                .stop(state, agent_id, force, grace_secs)
                .await?;
            Ok(serde_json::to_value(process).unwrap())
        }
        "agent_restart" => {
            let agent_id = require_str(args, "agent_id")?;
            let command = args.get("command").and_then(|v| v.as_str());
            let working_dir = args.get("working_dir").and_then(|v| v.as_str());
            let process = state
                .process_registry
                .restart(state, agent_id, command, working_dir)
                .await?;
            Ok(serde_json::to_value(process).unwrap())
        }
        "agent_invoke" => {
            let agent_id = require_str(args, "agent_id")?;
            let prompt = require_str(args, "prompt")?;
            let timeout_secs = args.get("timeout_secs").and_then(|v| v.as_i64());
            let metadata = args.get("metadata").filter(|v| !v.is_null()).cloned();
            let is_async = args.get("async").and_then(|v| v.as_bool()).unwrap_or(false);
            let invocation = state
                .process_registry
                .invoke(state, agent_id, prompt, timeout_secs, metadata, is_async)
                .await?;
            Ok(serde_json::to_value(invocation).unwrap())
        }
        "agent_invoke_result" => {
            let invocation_id = require_str(args, "invocation_id")?;
            let invocation =
                agents::processes::get_invocation(&state.pool, invocation_id).await?;
            Ok(serde_json::to_value(invocation).unwrap())
        }
        "agent_invocations" => {
            let agent_id = require_str(args, "agent_id")?;
            let status = args.get("status").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let invocations =
                agents::processes::list_invocations(&state.pool, agent_id, status, limit).await?;
            Ok(serde_json::to_value(invocations).unwrap())
        }
        "agent_process_status" => {
            let agent_id = require_str(args, "agent_id")?;
            let runtime_status = state.process_registry.get_status(agent_id).await;
            let db_process = match agents::processes::get_active_process(&state.pool, agent_id).await? {
                Some(p) => Some(p),
                None => agents::processes::get_latest_process(&state.pool, agent_id).await?,
            };
            Ok(serde_json::json!({
                "runtime": runtime_status,
                "process": db_process,
            }))
        }
        "agent_logs" => {
            let agent_id = require_str(args, "agent_id")?;
            let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
            let processes =
                agents::processes::list_processes(&state.pool, agent_id, limit).await?;
            Ok(serde_json::to_value(processes).unwrap())
        }
        "agent_update" => {
            let id = require_str(args, "id")?;
            let process_type = args.get("process_type").and_then(|v| v.as_str());
            let spawn_command = args.get("spawn_command").and_then(|v| v.as_str());
            let working_dir = args.get("working_dir").and_then(|v| v.as_str());
            let auto_restart = args.get("auto_restart").and_then(|v| v.as_bool());
            let metadata = args.get("metadata").and_then(|v| v.as_str());
            let agent = agents::processes::update_agent(
                &state.pool,
                id,
                process_type,
                spawn_command,
                working_dir,
                auto_restart,
                metadata,
            )
            .await?;
            Ok(serde_json::to_value(agent).unwrap())
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
