use serde_json::Value;

use nous_core::resources;
use nous_core::schedules;
use nous_core::worktrees;

use crate::state::AppState;

use super::{require_str, ToolSchema};

pub fn schemas() -> Vec<ToolSchema> {
    vec![
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
            name: "resource_register",
            description: "Register a new resource",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Resource name" },
                    "type": { "type": "string", "description": "Resource type: worktree, room, schedule, branch, file, docker-image, binary" },
                    "owner_agent_id": { "type": "string", "description": "Owning agent ID (optional)" },
                    "path": { "type": "string", "description": "Filesystem or logical path" },
                    "namespace": { "type": "string", "description": "Namespace (default: 'default')" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags for discovery" },
                    "metadata": { "type": "string", "description": "JSON metadata" },
                    "ownership_policy": { "type": "string", "description": "Ownership policy: cascade-delete, orphan, transfer-to-parent" }
                },
                "required": ["name", "type"]
            }),
        },
        ToolSchema {
            name: "resource_list",
            description: "List resources with optional filters",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Filter by resource type" },
                    "status": { "type": "string", "description": "Filter by status: active, archived, deleted" },
                    "owner_agent_id": { "type": "string", "description": "Filter by owner agent ID" },
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "orphaned": { "type": "boolean", "description": "Show only orphaned (unowned) resources" },
                    "ownership_policy": { "type": "string", "description": "Filter by ownership policy" },
                    "limit": { "type": "integer", "description": "Max results (default: 50)" }
                }
            }),
        },
        ToolSchema {
            name: "resource_get",
            description: "Get a resource by ID",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Resource ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "resource_update",
            description: "Update a resource (name, path, tags, metadata, status, ownership_policy)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Resource ID" },
                    "name": { "type": "string", "description": "New name" },
                    "path": { "type": "string", "description": "New path" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "New tags (replaces existing)" },
                    "metadata": { "type": "string", "description": "New JSON metadata (replaces existing)" },
                    "status": { "type": "string", "description": "New status: active, archived, deleted" },
                    "ownership_policy": { "type": "string", "description": "New ownership policy" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "resource_search",
            description: "Search resources by tags (AND semantics: resource must have ALL specified tags)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Tags to search (AND semantics)" },
                    "type": { "type": "string", "description": "Filter by resource type" },
                    "status": { "type": "string", "description": "Filter by status" },
                    "namespace": { "type": "string", "description": "Filter by namespace" },
                    "limit": { "type": "integer", "description": "Max results" }
                },
                "required": ["tags"]
            }),
        },
        ToolSchema {
            name: "resource_archive",
            description: "Archive an active resource (status: active -> archived)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Resource ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "resource_deregister",
            description: "Deregister a resource (soft-delete by default, hard=true removes row)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Resource ID" },
                    "force": { "type": "boolean", "description": "Force delete (remove from DB entirely)" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "resource_heartbeat",
            description: "Update a resource's last_seen_at timestamp (liveness tracking)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Resource ID" }
                },
                "required": ["id"]
            }),
        },
        ToolSchema {
            name: "resource_transfer",
            description: "Bulk transfer resource ownership from one agent to another (or orphan)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "from_agent_id": { "type": "string", "description": "Source agent ID" },
                    "to_agent_id": { "type": "string", "description": "Target agent ID (omit to orphan)" }
                },
                "required": ["from_agent_id"]
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
    ]
}

pub async fn dispatch(
    name: &str,
    args: &Value,
    state: &AppState,
) -> Option<Result<Value, nous_core::error::NousError>> {
    match name {
        "worktree_create" => Some(handle_worktree_create(args, state).await),
        "worktree_list" => Some(handle_worktree_list(args, state).await),
        "worktree_get" => Some(handle_worktree_get(args, state).await),
        "worktree_archive" => Some(handle_worktree_archive(args, state).await),
        "worktree_delete" => Some(handle_worktree_delete(args, state).await),
        "resource_register" => Some(handle_resource_register(args, state).await),
        "resource_list" => Some(handle_resource_list(args, state).await),
        "resource_get" => Some(handle_resource_get(args, state).await),
        "resource_update" => Some(handle_resource_update(args, state).await),
        "resource_search" => Some(handle_resource_search(args, state).await),
        "resource_archive" => Some(handle_resource_archive(args, state).await),
        "resource_deregister" => Some(handle_resource_deregister(args, state).await),
        "resource_heartbeat" => Some(handle_resource_heartbeat(args, state).await),
        "resource_transfer" => Some(handle_resource_transfer(args, state).await),
        "schedule_create" => Some(handle_schedule_create(args, state).await),
        "schedule_get" => Some(handle_schedule_get(args, state).await),
        "schedule_list" => Some(handle_schedule_list(args, state).await),
        "schedule_update" => Some(handle_schedule_update(args, state).await),
        "schedule_delete" => Some(handle_schedule_delete(args, state).await),
        "schedule_runs_list" => Some(handle_schedule_runs_list(args, state).await),
        "schedule_health" => Some(handle_schedule_health(state).await),
        _ => None,
    }
}

async fn handle_worktree_create(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_worktree_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_worktree_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let wt = worktrees::get(&state.pool, id).await?;
    Ok(serde_json::to_value(wt).unwrap())
}

async fn handle_worktree_archive(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let wt = worktrees::archive(&state.pool, id).await?;
    Ok(serde_json::to_value(wt).unwrap())
}

async fn handle_worktree_delete(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    worktrees::delete(&state.pool, id).await?;
    Ok(serde_json::json!({"deleted": true}))
}

async fn handle_resource_register(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let name = require_str(args, "name")?.to_string();
    let type_str = require_str(args, "type")?;
    let resource_type: resources::ResourceType = type_str.parse()?;
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
    let ownership_policy = args
        .get("ownership_policy")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::OwnershipPolicy>())
        .transpose()?;
    let resource = resources::register_resource(
        &state.pool,
        resources::RegisterResourceRequest {
            name,
            resource_type,
            owner_agent_id,
            namespace,
            path,
            metadata,
            tags,
            ownership_policy,
        },
    )
    .await?;
    Ok(serde_json::to_value(resource).unwrap())
}

async fn handle_resource_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let resource_type = args
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::ResourceType>())
        .transpose()?;
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::ResourceStatus>())
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
    let ownership_policy = args
        .get("ownership_policy")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::OwnershipPolicy>())
        .transpose()?;
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
    let items = resources::list_resources(
        &state.pool,
        &resources::ListResourcesFilter {
            resource_type,
            status,
            owner_agent_id,
            namespace,
            orphaned,
            ownership_policy,
            limit,
            ..Default::default()
        },
    )
    .await?;
    Ok(serde_json::to_value(items).unwrap())
}

async fn handle_resource_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let resource = resources::get_resource_by_id(&state.pool, id).await?;
    Ok(serde_json::to_value(resource).unwrap())
}

async fn handle_resource_update(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
        .map(|s| s.parse::<resources::ResourceStatus>())
        .transpose()?;
    let ownership_policy = args
        .get("ownership_policy")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::OwnershipPolicy>())
        .transpose()?;
    let resource = resources::update_resource(
        &state.pool,
        resources::UpdateResourceRequest {
            id,
            name,
            path,
            metadata,
            tags,
            status,
            ownership_policy,
        },
    )
    .await?;
    Ok(serde_json::to_value(resource).unwrap())
}

async fn handle_resource_search(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let tags: Vec<String> = args
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let resource_type = args
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::ResourceType>())
        .transpose()?;
    let status = args
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.parse::<resources::ResourceStatus>())
        .transpose()?;
    let namespace = args
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(String::from);
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
    let items = resources::search_by_tags(
        &state.pool,
        &resources::SearchResourcesRequest {
            tags,
            resource_type,
            status,
            namespace,
            limit,
        },
    )
    .await?;
    Ok(serde_json::to_value(items).unwrap())
}

async fn handle_resource_archive(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let resource = resources::archive_resource(&state.pool, id).await?;
    Ok(serde_json::to_value(resource).unwrap())
}

async fn handle_resource_deregister(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    resources::deregister_resource(&state.pool, id, force).await?;
    Ok(serde_json::json!({"deleted": true}))
}

async fn handle_resource_heartbeat(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let resource = resources::heartbeat_resource(&state.pool, id).await?;
    Ok(serde_json::to_value(resource).unwrap())
}

async fn handle_resource_transfer(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let from = require_str(args, "from_agent_id")?;
    let to = args.get("to_agent_id").and_then(|v| v.as_str());
    let count = resources::transfer_ownership(&state.pool, from, to).await?;
    Ok(serde_json::json!({"transferred": count}))
}

async fn handle_schedule_create(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    let schedule = schedules::create_schedule(schedules::CreateScheduleParams {
        db: &state.pool,
        name,
        cron_expr,
        trigger_at,
        timezone,
        action_type,
        action_payload,
        desired_outcome,
        max_retries,
        timeout_secs,
        max_output_bytes: None,
        max_runs,
        clock: &clock,
    })
    .await?;
    Ok(serde_json::to_value(schedule).unwrap())
}

async fn handle_schedule_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let schedule = schedules::get_schedule(&state.pool, id).await?;
    Ok(serde_json::to_value(schedule).unwrap())
}

async fn handle_schedule_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let enabled = args.get("enabled").and_then(|v| v.as_bool());
    let action_type = args.get("action_type").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
    let list = schedules::list_schedules(&state.pool, enabled, action_type, limit).await?;
    Ok(serde_json::to_value(list).unwrap())
}

async fn handle_schedule_update(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    let schedule = schedules::update_schedule(schedules::UpdateScheduleParams {
        db: &state.pool,
        id,
        name,
        cron_expr,
        trigger_at,
        enabled,
        action_type,
        action_payload,
        desired_outcome,
        max_retries,
        timeout_secs: timeout_secs.map(Some),
        max_runs,
        clock: &clock,
    })
    .await?;
    Ok(serde_json::to_value(schedule).unwrap())
}

async fn handle_schedule_delete(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    schedules::delete_schedule(&state.pool, id).await?;
    Ok(serde_json::json!({"deleted": true}))
}

async fn handle_schedule_runs_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let schedule_id = require_str(args, "schedule_id")?;
    let status = args.get("status").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
    let runs = schedules::list_runs(&state.pool, schedule_id, status, limit).await?;
    Ok(serde_json::to_value(runs).unwrap())
}

async fn handle_schedule_health(
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let health = schedules::schedule_health(&state.pool).await?;
    Ok(health)
}
