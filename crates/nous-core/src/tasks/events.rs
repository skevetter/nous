use sea_orm::DatabaseConnection;

use crate::error::NousError;
use crate::messages::{self, MessageType, PostMessageRequest};
use crate::notifications::NotificationRegistry;

use super::store::{close_task, link_tasks, update_task};
use super::types::{
    PostTaskEventParams, Task, TaskCommand, TaskCommandResult, UpdateTaskParams,
};

pub async fn post_task_event_to_room(params: PostTaskEventParams<'_>) -> Result<(), NousError> {
    let PostTaskEventParams {
        db,
        registry,
        task_id,
        room_id,
        event_type,
        old_value,
        new_value,
        actor_id,
    } = params;
    let content = match event_type {
        "status_changed" => format!(
            "Task status: {} → {}",
            old_value.unwrap_or("none"),
            new_value.unwrap_or("none")
        ),
        "assigned" => format!(
            "Task assigned: {} → {}",
            old_value.unwrap_or("none"),
            new_value.unwrap_or("none")
        ),
        "priority_changed" => format!(
            "Task priority: {} → {}",
            old_value.unwrap_or("none"),
            new_value.unwrap_or("none")
        ),
        "linked" => format!("Task linked: {}", new_value.unwrap_or("")),
        "created" => format!("Task created: {}", new_value.unwrap_or("")),
        _ => format!("Task event: {event_type}"),
    };

    let metadata = serde_json::json!({
        "message_type": "task_event",
        "topics": [format!("task:{task_id}")],
        "task_event": {
            "task_id": task_id,
            "event_type": event_type,
            "old_value": old_value,
            "new_value": new_value,
            "actor_id": actor_id,
        }
    });

    let sender = actor_id.unwrap_or("system");

    messages::post_message(
        db,
        PostMessageRequest {
            room_id: room_id.to_string(),
            sender_id: sender.to_string(),
            content,
            reply_to: None,
            metadata: Some(metadata),
            message_type: Some(MessageType::TaskEvent),
        },
        registry,
    )
    .await?;

    Ok(())
}

pub async fn execute_task_command(
    db: &DatabaseConnection,
    cmd: TaskCommand,
    registry: Option<&NotificationRegistry>,
) -> Result<TaskCommandResult, NousError> {
    let make_result = |success: bool, message: String, task: Option<Task>| -> TaskCommandResult {
        TaskCommandResult {
            command: cmd.command.clone(),
            task_id: cmd.task_id.clone(),
            success,
            message,
            task,
        }
    };

    match cmd.command.as_str() {
        "close" => {
            let task = close_task(db, &cmd.task_id, Some(&cmd.actor_id)).await?;
            Ok(make_result(true, "Task closed".to_string(), Some(task)))
        }
        "assign" => {
            if cmd.args.is_empty() {
                return Err(NousError::Validation(
                    "assign command requires 1 argument: assignee_id".into(),
                ));
            }
            let task = update_task(UpdateTaskParams {
                db,
                id: &cmd.task_id,
                status: None,
                priority: None,
                assignee_id: Some(&cmd.args[0]),
                description: None,
                labels: None,
                actor_id: Some(&cmd.actor_id),
                registry,
            })
            .await?;
            Ok(make_result(
                true,
                format!("Task assigned to {}", cmd.args[0]),
                Some(task),
            ))
        }
        "status" => {
            if cmd.args.is_empty() {
                return Err(NousError::Validation(
                    "status command requires 1 argument: new_status".into(),
                ));
            }
            let task = update_task(UpdateTaskParams {
                db,
                id: &cmd.task_id,
                status: Some(&cmd.args[0]),
                priority: None,
                assignee_id: None,
                description: None,
                labels: None,
                actor_id: Some(&cmd.actor_id),
                registry,
            })
            .await?;
            Ok(make_result(
                true,
                format!("Task status set to {}", cmd.args[0]),
                Some(task),
            ))
        }
        "priority" => {
            if cmd.args.is_empty() {
                return Err(NousError::Validation(
                    "priority command requires 1 argument: new_priority".into(),
                ));
            }
            let task = update_task(UpdateTaskParams {
                db,
                id: &cmd.task_id,
                status: None,
                priority: Some(&cmd.args[0]),
                assignee_id: None,
                description: None,
                labels: None,
                actor_id: Some(&cmd.actor_id),
                registry,
            })
            .await?;
            Ok(make_result(
                true,
                format!("Task priority set to {}", cmd.args[0]),
                Some(task),
            ))
        }
        "link" => {
            if cmd.args.len() < 2 {
                return Err(NousError::Validation(
                    "link command requires 2 arguments: target_id, link_type".into(),
                ));
            }
            link_tasks(
                db,
                &cmd.task_id,
                &cmd.args[0],
                &cmd.args[1],
                Some(&cmd.actor_id),
            )
            .await?;
            Ok(make_result(
                true,
                format!("Task linked to {} as {}", cmd.args[0], cmd.args[1]),
                None,
            ))
        }
        other => Err(NousError::Validation(format!(
            "unknown task command: '{other}'"
        ))),
    }
}
