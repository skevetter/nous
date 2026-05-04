use serde_json::Value;

use nous_core::agents;
use nous_core::messages::{post_message, MessageType, PostMessageRequest};

use crate::process_manager::{InvokeParams, RestartParams, SpawnParams, StopParams};
use crate::state::AppState;

use super::{require_str, to_json, ToolSchema};

pub fn schemas() -> Vec<ToolSchema> {
    let mut all = agent_lifecycle_schemas();
    all.extend(agent_version_schemas());
    all.extend(agent_process_schemas());
    all.extend(agent_coordination_schemas());
    all
}

fn agent_lifecycle_schemas() -> Vec<ToolSchema> {
    let mut schemas = agent_lifecycle_core_schemas();
    schemas.extend(agent_lifecycle_search_schemas());
    schemas
}

fn agent_lifecycle_core_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "agent_register",
            description: "Register a new agent in the org hierarchy",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Agent name (unique within namespace)" },
                    "parent_id": { "type": "string", "description": "Parent agent ID" },
                    "namespace": { "type": "string", "description": "Namespace (defaults to 'default')" },
                    "room": { "type": "string", "description": "Room name for this agent" },
                    "metadata": { "type": "string", "description": "JSON metadata string" },
                    "status": { "type": "string", "description": "Initial status" }
                },
                "required": ["name"]
            }),
        },
        ToolSchema {
            name: "agent_deregister",
            description: "Deregister an agent (remove from registry)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Agent ID" },
                    "force": { "type": "boolean", "description": "Force delete (cascade children)" }
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
    ]
}

fn agent_lifecycle_search_schemas() -> Vec<ToolSchema> {
    vec![
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
    ]
}

fn agent_version_schemas() -> Vec<ToolSchema> {
    let mut schemas = agent_version_history_schemas();
    schemas.extend(agent_template_schemas());
    schemas
}

fn agent_version_history_schemas() -> Vec<ToolSchema> {
    vec![
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
            name: "agent_bulk_deregister",
            description: "Deregister multiple agents by ID list",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "ids": { "type": "array", "items": { "type": "string" }, "description": "List of agent IDs to deregister" },
                    "force": { "type": "boolean", "description": "Force delete (cascade children, default: false)" }
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
    ]
}

fn agent_template_schemas() -> Vec<ToolSchema> {
    vec![
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
    ]
}

fn agent_process_schemas() -> Vec<ToolSchema> {
    let mut schemas = agent_spawn_control_schemas();
    schemas.extend(agent_invocation_schemas());
    schemas
}

fn agent_spawn_control_schemas() -> Vec<ToolSchema> {
    vec![
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
                    "timeout_secs": { "type": "integer", "description": "Process timeout in seconds" }
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

fn agent_invocation_schemas() -> Vec<ToolSchema> {
    vec![
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
    ]
}

fn agent_coordination_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "agent_presence",
            description: "Post a presence status update for an agent to their registered room",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["active", "idle", "blocked", "done"] }
                },
                "required": ["agent_id", "status"]
            }),
        },
        ToolSchema {
            name: "agent_handoff",
            description: "Send a structured work handoff from one agent to another",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string" },
                    "from_agent": { "type": "string" },
                    "to_agent": { "type": "string" },
                    "task_id": { "type": "string" },
                    "branch": { "type": "string" },
                    "scope": { "type": "string" },
                    "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
                    "context": { "type": "object" },
                    "deadline": { "type": "string" }
                },
                "required": ["room_id", "from_agent", "to_agent"]
            }),
        },
    ]
}

pub async fn dispatch(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    if let Some(r) = dispatch_lifecycle(name, args, state).await {
        return Some(r);
    }
    if let Some(r) = dispatch_version_and_templates(name, args, state).await {
        return Some(r);
    }
    if let Some(r) = dispatch_process(name, args, state).await {
        return Some(r);
    }
    dispatch_coordination(name, args, state).await
}

async fn dispatch_lifecycle(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    if let Some(r) = dispatch_lifecycle_crud(name, args, state).await {
        return Some(r);
    }
    dispatch_lifecycle_query(name, args, state).await
}

async fn dispatch_lifecycle_crud(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_register" => Some(handle_register(args, state).await),
        "agent_deregister" => Some(handle_deregister(args, state).await),
        "agent_lookup" => Some(handle_lookup(args, state).await),
        "agent_list" => Some(handle_list(args, state).await),
        "agent_list_children" => Some(handle_list_children(args, state).await),
        "agent_list_ancestors" => Some(handle_list_ancestors(args, state).await),
        _ => None,
    }
}

async fn dispatch_lifecycle_query(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_tree" => Some(handle_tree(args, state).await),
        "agent_heartbeat" => Some(handle_heartbeat(args, state).await),
        "agent_search" => Some(handle_search(args, state).await),
        "agent_stale" => Some(handle_stale(args, state).await),
        "agent_inspect" => Some(handle_inspect(args, state).await),
        _ => None,
    }
}

async fn dispatch_version_and_templates(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    if let Some(r) = dispatch_versions(name, args, state).await {
        return Some(r);
    }
    dispatch_templates(name, args, state).await
}

async fn dispatch_versions(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_versions" => Some(handle_versions(args, state).await),
        "agent_record_version" => Some(handle_record_version(args, state).await),
        "agent_rollback" => Some(handle_rollback(args, state).await),
        "agent_notify_upgrade" => Some(handle_notify_upgrade(args, state).await),
        "agent_outdated" => Some(handle_outdated(args, state).await),
        "agent_bulk_deregister" => Some(handle_bulk_deregister(args, state).await),
        "agent_update_status" => Some(handle_update_status(args, state).await),
        _ => None,
    }
}

async fn dispatch_templates(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_template_create" => Some(handle_template_create(args, state).await),
        "agent_template_list" => Some(handle_template_list(args, state).await),
        "agent_template_get" => Some(handle_template_get(args, state).await),
        "agent_instantiate" => Some(handle_instantiate(args, state).await),
        _ => None,
    }
}

async fn dispatch_process(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    if let Some(r) = dispatch_spawn_control(name, args, state).await {
        return Some(r);
    }
    dispatch_invocations(name, args, state).await
}

async fn dispatch_spawn_control(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_spawn" => Some(handle_spawn(args, state).await),
        "agent_stop" => Some(handle_stop(args, state).await),
        "agent_restart" => Some(handle_restart(args, state).await),
        "agent_update" => Some(handle_update(args, state).await),
        _ => None,
    }
}

async fn dispatch_invocations(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_invoke" => Some(handle_invoke(args, state).await),
        "agent_invoke_result" => Some(handle_invoke_result(args, state).await),
        "agent_invocations" => Some(handle_invocations(args, state).await),
        "agent_process_status" => Some(handle_process_status(args, state).await),
        "agent_logs" => Some(handle_logs(args, state).await),
        _ => None,
    }
}

async fn dispatch_coordination(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "agent_presence" => Some(handle_presence(args, state).await),
        "agent_handoff" => Some(handle_handoff(args, state).await),
        _ => None,
    }
}

async fn handle_register(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let name = require_str(args, "name")?.to_string();
    let parent_id = args
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let agent_type = args
        .get("agent_type")
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
        .map(str::parse::<agents::AgentStatus>)
        .transpose()?;
    let agent = agents::register_agent(
        &state.pool,
        agents::RegisterAgentRequest {
            name,
            parent_id,
            agent_type,
            namespace,
            room,
            metadata,
            status,
        },
    )
    .await?;
    to_json(agent)
}

async fn handle_deregister(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let force = args
        .get("force")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    agents::deregister_agent(&state.pool, id, force).await?;
    Ok(serde_json::json!({"deleted": true}))
}

async fn handle_lookup(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let name = require_str(args, "name")?;
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let agent = agents::lookup_agent(&state.pool, name, namespace).await?;
    to_json(agent)
}

async fn handle_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let namespace = args
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(String::from);
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::parse::<agents::AgentStatus>)
        .transpose()?;
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let list = agents::list_agents(
        &state.pool,
        &agents::ListAgentsFilter {
            namespace,
            status,
            limit,
            ..Default::default()
        },
    )
    .await?;
    to_json(list)
}

async fn handle_list_children(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let children = agents::list_children(&state.pool, id, namespace).await?;
    to_json(children)
}

async fn handle_list_ancestors(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let ancestors = agents::list_ancestors(&state.pool, id, namespace).await?;
    to_json(ancestors)
}

async fn handle_tree(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let root_id = args.get("root_id").and_then(|v| v.as_str());
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let tree = agents::get_tree(&state.pool, root_id, namespace).await?;
    to_json(tree)
}

async fn handle_heartbeat(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::parse::<agents::AgentStatus>)
        .transpose()?;
    agents::heartbeat(&state.pool, id, status).await?;
    Ok(serde_json::json!({"ok": true}))
}

async fn handle_search(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let query = require_str(args, "query")?;
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let results = agents::search_agents(&state.pool, query, namespace, limit).await?;
    to_json(results)
}

async fn handle_stale(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let threshold = args
        .get("threshold")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(900);
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let stale_agents = agents::list_stale_agents(&state.pool, threshold, namespace).await?;
    to_json(stale_agents)
}

async fn handle_inspect(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let inspection = agents::inspect_agent(&state.pool, id).await?;
    to_json(inspection)
}

async fn handle_versions(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let versions = agents::list_versions(&state.pool, agent_id, limit).await?;
    to_json(versions)
}

async fn handle_record_version(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    to_json(version)
}

async fn handle_rollback(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let version_id = require_str(args, "version_id")?;
    let version = agents::rollback_agent(&state.pool, agent_id, version_id).await?;
    to_json(version)
}

async fn handle_notify_upgrade(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    agents::set_upgrade_available(&state.pool, id, true).await?;
    Ok(serde_json::json!({"notified": true}))
}

async fn handle_outdated(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let namespace = args.get("namespace").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let outdated = agents::list_outdated_agents(&state.pool, namespace, limit).await?;
    to_json(outdated)
}

async fn handle_template_create(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    to_json(template)
}

async fn handle_template_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let template_type = args.get("type").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let templates = agents::list_templates(&state.pool, template_type, limit).await?;
    to_json(templates)
}

async fn handle_template_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let template = agents::get_template_by_id(&state.pool, id).await?;
    to_json(template)
}

async fn handle_instantiate(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    to_json(agent)
}

async fn handle_bulk_deregister(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let ids: Vec<String> = args
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let force = args
        .get("force")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let mut results = serde_json::Map::new();
    for id in &ids {
        match agents::deregister_agent(&state.pool, id, force).await {
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

async fn handle_update_status(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let status_str = require_str(args, "status")?;
    let status: agents::AgentStatus = status_str.parse()?;
    let agent = agents::update_agent_status(&state.pool, id, status).await?;
    to_json(agent)
}

async fn handle_spawn(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    let timeout_secs = args.get("timeout_secs").and_then(serde_json::Value::as_i64);
    let process = state
        .process_registry
        .spawn(SpawnParams {
            state,
            agent_id,
            command: &command,
            process_type,
            working_dir,
            env,
            timeout_secs,
        })
        .await?;
    to_json(process)
}

async fn handle_stop(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let force = args.get("force").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let grace_secs = args
        .get("grace_secs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(10);
    let process = state
        .process_registry
        .stop(StopParams { state, agent_id, force, grace_secs })
        .await?;
    to_json(process)
}

async fn handle_restart(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let command = args.get("command").and_then(|v| v.as_str());
    let working_dir = args.get("working_dir").and_then(|v| v.as_str());
    let process = state
        .process_registry
        .restart(RestartParams { state, agent_id, command, working_dir })
        .await?;
    to_json(process)
}

async fn handle_invoke(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let prompt = require_str(args, "prompt")?;
    let timeout_secs = args.get("timeout_secs").and_then(serde_json::Value::as_i64);
    let metadata = args.get("metadata").filter(|v| !v.is_null()).cloned();
    let is_async = args.get("async").and_then(serde_json::Value::as_bool).unwrap_or(false);
    let invocation = state
        .process_registry
        .invoke(InvokeParams { state, agent_id, prompt, timeout_secs, metadata, is_async })
        .await?;
    to_json(invocation)
}

async fn handle_invoke_result(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let invocation_id = require_str(args, "invocation_id")?;
    let invocation = agents::processes::get_invocation(&state.pool, invocation_id).await?;
    to_json(invocation)
}

async fn handle_invocations(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let status = args.get("status").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let invocations =
        agents::processes::list_invocations(&state.pool, agent_id, status, limit).await?;
    to_json(invocations)
}

async fn handle_process_status(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_logs(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let limit = args.get("limit").and_then(serde_json::Value::as_u64).map(|v| v as u32);
    let processes = agents::processes::list_processes(&state.pool, agent_id, limit).await?;
    to_json(processes)
}

async fn handle_update(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let process_type = args.get("process_type").and_then(|v| v.as_str());
    let spawn_command = args.get("spawn_command").and_then(|v| v.as_str());
    let working_dir = args.get("working_dir").and_then(|v| v.as_str());
    let auto_restart = args.get("auto_restart").and_then(serde_json::Value::as_bool);
    let metadata = args.get("metadata").and_then(|v| v.as_str());
    let agent = agents::processes::update_agent(
        &state.pool,
        agents::processes::UpdateAgentRequest {
            id,
            process_type,
            spawn_command,
            working_dir,
            auto_restart,
            metadata_json: metadata,
        },
    )
    .await?;
    to_json(agent)
}

async fn handle_presence(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let agent_id = require_str(args, "agent_id")?;
    let status = require_str(args, "status")?;
    if !matches!(status, "active" | "idle" | "blocked" | "done") {
        return Err(nous_core::error::NousError::Validation(format!(
            "invalid presence status: {status} (expected active, idle, blocked, or done)"
        )));
    }
    let agent = agents::get_agent_by_id(&state.pool, agent_id).await?;
    let room_id = agent.room.ok_or_else(|| {
        nous_core::error::NousError::Validation(format!(
            "agent '{agent_id}' has no registered room"
        ))
    })?;
    let metadata = serde_json::json!({
        "topics": ["presence"],
        "presence": { "agent_id": agent_id, "status": status }
    });
    let msg = post_message(
        &state.pool,
        PostMessageRequest {
            room_id,
            sender_id: agent_id.to_string(),
            content: format!("Agent {agent_id} is now {status}"),
            reply_to: None,
            metadata: Some(metadata),
            message_type: Some(MessageType::System),
        },
        Some(&state.registry),
    )
    .await?;
    to_json(msg)
}

async fn handle_handoff(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let room_id = require_str(args, "room_id")?.to_string();
    let from_agent = require_str(args, "from_agent")?.to_string();
    let to_agent = require_str(args, "to_agent")?.to_string();
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str().map(String::from));
    let branch = args
        .get("branch")
        .and_then(|v| v.as_str().map(String::from));
    let scope = args.get("scope").and_then(|v| v.as_str().map(String::from));
    let acceptance_criteria: Vec<String> = args
        .get("acceptance_criteria")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let context = args
        .get("context")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let deadline = args
        .get("deadline")
        .and_then(|v| v.as_str().map(String::from));

    let payload = agents::coordination::HandoffPayload {
        task_id,
        branch,
        scope,
        acceptance_criteria,
        context,
        deadline,
    };

    let msg = agents::coordination::post_handoff(
        &state.pool,
        Some(&state.registry),
        agents::coordination::PostHandoffRequest {
            room_id: &room_id,
            from_agent: &from_agent,
            to_agent: &to_agent,
            payload,
        },
    )
    .await?;
    to_json(msg)
}
