use serde_json::Value;

use nous_core::tasks;

use crate::state::AppState;

use super::{require_str, ToolSchema};

pub fn schemas() -> Vec<ToolSchema> {
    vec![
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
            name: "task_command",
            description: "Execute a task operation from chat (close, assign, status, priority, link)",
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "room_id": { "type": "string", "description": "Room where command is issued" },
                    "command": { "type": "string", "enum": ["close", "assign", "status", "priority", "link"] },
                    "task_id": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "actor_id": { "type": "string" }
                },
                "required": ["command", "task_id", "actor_id"]
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
        "task_create" => Some(handle_task_create(args, state).await),
        "task_list" => Some(handle_task_list(args, state).await),
        "task_get" => Some(handle_task_get(args, state).await),
        "task_update" => Some(handle_task_update(args, state).await),
        "task_close" => Some(handle_task_close(args, state).await),
        "task_link" => Some(handle_task_link(args, state).await),
        "task_unlink" => Some(handle_task_unlink(args, state).await),
        "task_list_links" => Some(handle_task_list_links(args, state).await),
        "task_add_note" => Some(handle_task_add_note(args, state).await),
        "task_depends_add" => Some(handle_task_depends_add(args, state).await),
        "task_depends_remove" => Some(handle_task_depends_remove(args, state).await),
        "task_depends_list" => Some(handle_task_depends_list(args, state).await),
        "task_template_create" => Some(handle_task_template_create(args, state).await),
        "task_template_list" => Some(handle_task_template_list(args, state).await),
        "task_template_get" => Some(handle_task_template_get(args, state).await),
        "task_template_use" => Some(handle_task_template_use(args, state).await),
        "task_batch_close" => Some(handle_task_batch_close(args, state).await),
        "task_batch_update_status" => Some(handle_task_batch_update_status(args, state).await),
        "task_batch_assign" => Some(handle_task_batch_assign(args, state).await),
        "task_command" => Some(handle_task_command(args, state).await),
        _ => None,
    }
}

async fn handle_task_create(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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
    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &state.pool,
        title,
        description,
        priority,
        assignee_id,
        labels: labels.as_deref(),
        room_id,
        create_room: create_room_flag,
        actor_id: None,
        registry: None,
    })
    .await?;
    Ok(serde_json::to_value(task).unwrap())
}

async fn handle_task_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let status = args.get("status").and_then(|v| v.as_str());
    let assignee_id = args.get("assignee_id").and_then(|v| v.as_str());
    let label = args.get("label").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
    let offset = args
        .get("offset")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let result = tasks::list_tasks(tasks::ListTasksParams {
        db: &state.pool,
        status,
        assignee_id,
        label,
        limit,
        offset,
        order_by: None,
        order_dir: None,
    })
    .await?;
    Ok(serde_json::to_value(result).unwrap())
}

async fn handle_task_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let task = tasks::get_task(&state.pool, id).await?;
    Ok(serde_json::to_value(task).unwrap())
}

async fn handle_task_update(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let status = args.get("status").and_then(|v| v.as_str());
    let priority = args.get("priority").and_then(|v| v.as_str());
    let assignee_id = args.get("assignee_id").and_then(|v| v.as_str());
    let description = args.get("description").and_then(|v| v.as_str());
    let task = tasks::update_task(tasks::UpdateTaskParams {
        db: &state.pool,
        id,
        status,
        priority,
        assignee_id,
        description,
        labels: None,
        actor_id: None,
        registry: None,
    })
    .await?;
    Ok(serde_json::to_value(task).unwrap())
}

async fn handle_task_close(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let task = tasks::close_task(&state.pool, id, None).await?;
    Ok(serde_json::to_value(task).unwrap())
}

async fn handle_task_link(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let source_id = require_str(args, "source_id")?;
    let target_id = require_str(args, "target_id")?;
    let link_type = require_str(args, "link_type")?;
    let link = tasks::link_tasks(&state.pool, source_id, target_id, link_type, None).await?;
    Ok(serde_json::to_value(link).unwrap())
}

async fn handle_task_unlink(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let source_id = require_str(args, "source_id")?;
    let target_id = require_str(args, "target_id")?;
    let link_type = require_str(args, "link_type")?;
    tasks::unlink_tasks(&state.pool, source_id, target_id, link_type, None).await?;
    Ok(serde_json::json!({"unlinked": true}))
}

async fn handle_task_list_links(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let links = tasks::list_links(&state.pool, id).await?;
    Ok(serde_json::to_value(links).unwrap())
}

async fn handle_task_add_note(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let sender_id = require_str(args, "sender_id")?;
    let content = require_str(args, "content")?;
    let msg = tasks::add_note(&state.pool, id, sender_id, content).await?;
    Ok(msg)
}

async fn handle_task_depends_add(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let task_id = require_str(args, "task_id")?;
    let depends_on_task_id = require_str(args, "depends_on_task_id")?;
    let dep_type = args.get("dep_type").and_then(|v| v.as_str());
    let dep = tasks::add_dependency(&state.pool, task_id, depends_on_task_id, dep_type).await?;
    Ok(serde_json::to_value(dep).unwrap())
}

async fn handle_task_depends_remove(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let task_id = require_str(args, "task_id")?;
    let depends_on_task_id = require_str(args, "depends_on_task_id")?;
    let dep_type = args.get("dep_type").and_then(|v| v.as_str());
    tasks::remove_dependency(&state.pool, task_id, depends_on_task_id, dep_type).await?;
    Ok(serde_json::json!({"removed": true}))
}

async fn handle_task_depends_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let task_id = require_str(args, "task_id")?;
    let deps = tasks::list_dependencies(&state.pool, task_id).await?;
    Ok(serde_json::to_value(deps).unwrap())
}

async fn handle_task_template_create(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_task_template_list(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as u32);
    let templates = tasks::list_templates(&state.pool, limit).await?;
    Ok(serde_json::to_value(templates).unwrap())
}

async fn handle_task_template_get(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let id = require_str(args, "id")?;
    let tmpl = tasks::get_template(&state.pool, id).await?;
    Ok(serde_json::to_value(tmpl).unwrap())
}

async fn handle_task_template_use(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_task_batch_close(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_task_batch_update_status(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_task_batch_assign(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
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

async fn handle_task_command(
    args: &Value,
    state: &AppState,
) -> Result<Value, nous_core::error::NousError> {
    let command = require_str(args, "command")?.to_string();
    let task_id = require_str(args, "task_id")?.to_string();
    let actor_id = require_str(args, "actor_id")?.to_string();
    let cmd_args: Vec<String> = args
        .get("args")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let cmd = tasks::TaskCommand {
        command,
        task_id,
        args: cmd_args,
        actor_id,
    };

    let result = tasks::execute_task_command(&state.pool, cmd, Some(&state.registry)).await?;
    Ok(serde_json::to_value(result).unwrap())
}
