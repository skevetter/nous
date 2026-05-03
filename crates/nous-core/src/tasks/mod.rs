use std::collections::{HashMap, HashSet, VecDeque};

use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, NotSet, Set, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agents;
use crate::entities::{
    task_dependencies as dep_entity, task_events as event_entity, task_links as link_entity,
    task_templates as template_entity, tasks as task_entity,
};
use crate::error::NousError;
use crate::messages::{self, MessageType, PostMessageRequest};
use crate::notifications::NotificationRegistry;
use crate::rooms;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub priority: String,
    pub assignee_id: Option<String>,
    pub labels: Option<Vec<String>>,
    pub room_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<TaskLinks>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_discussion: Option<Vec<serde_json::Value>>,
}

impl Task {
    fn from_model(m: task_entity::Model) -> Self {
        let labels = m.labels.as_deref().and_then(|s| match serde_json::from_str(s) {
            Ok(val) => Some(val),
            Err(e) => {
                tracing::warn!(error = %e, "malformed JSON in tasks labels column, treating as null");
                None
            }
        });

        Self {
            id: m.id,
            title: m.title,
            description: m.description,
            status: m.status,
            priority: m.priority,
            assignee_id: m.assignee_id,
            labels,
            room_id: m.room_id,
            created_at: m.created_at,
            updated_at: m.updated_at,
            closed_at: m.closed_at,
            links: None,
            recent_discussion: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLink {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub link_type: String,
    pub created_at: String,
}

impl TaskLink {
    fn from_model(m: link_entity::Model) -> Self {
        Self {
            id: m.id,
            source_id: m.source_id,
            target_id: m.target_id,
            link_type: m.link_type,
            created_at: m.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEvent {
    pub id: String,
    pub task_id: String,
    pub event_type: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub actor_id: Option<String>,
    pub created_at: String,
}

impl TaskEvent {
    fn from_model(m: event_entity::Model) -> Self {
        Self {
            id: m.id,
            task_id: m.task_id,
            event_type: m.event_type,
            old_value: m.old_value,
            new_value: m.new_value,
            actor_id: m.actor_id,
            created_at: m.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLinks {
    pub blocked_by: Vec<String>,
    pub parent: Vec<String>,
    pub related_to: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCommand {
    pub command: String,
    pub task_id: String,
    pub args: Vec<String>,
    pub actor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCommandResult {
    pub command: String,
    pub task_id: String,
    pub success: bool,
    pub message: String,
    pub task: Option<Task>,
}

#[allow(clippy::too_many_arguments)]
pub async fn post_task_event_to_room(
    db: &DatabaseConnection,
    registry: Option<&NotificationRegistry>,
    task_id: &str,
    room_id: &str,
    event_type: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
    actor_id: Option<&str>,
) -> Result<(), NousError> {
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
            let task = update_task(
                db,
                &cmd.task_id,
                None,
                None,
                Some(&cmd.args[0]),
                None,
                None,
                Some(&cmd.actor_id),
                registry,
            )
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
            let task = update_task(
                db,
                &cmd.task_id,
                Some(&cmd.args[0]),
                None,
                None,
                None,
                None,
                Some(&cmd.actor_id),
                registry,
            )
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
            let task = update_task(
                db,
                &cmd.task_id,
                None,
                Some(&cmd.args[0]),
                None,
                None,
                None,
                Some(&cmd.actor_id),
                registry,
            )
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

#[allow(clippy::too_many_arguments)]
pub async fn create_task(
    db: &DatabaseConnection,
    title: &str,
    description: Option<&str>,
    priority: Option<&str>,
    assignee_id: Option<&str>,
    labels: Option<&[String]>,
    room_id: Option<&str>,
    create_room: bool,
    actor_id: Option<&str>,
    registry: Option<&NotificationRegistry>,
) -> Result<Task, NousError> {
    if title.trim().is_empty() {
        return Err(NousError::Validation("task title cannot be empty".into()));
    }

    let id = Uuid::now_v7().to_string();
    let priority = priority.unwrap_or("medium");

    let effective_room_id = if create_room {
        let room_name = format!("task-{id}");
        let room = rooms::create_room(
            db,
            &room_name,
            Some(&format!("Discussion for task: {title}")),
            None,
        )
        .await?;
        Some(room.id)
    } else {
        room_id.map(String::from)
    };

    let labels_json = labels.map(|l| serde_json::to_string(l).unwrap_or_else(|_| "[]".to_string()));

    let model = task_entity::ActiveModel {
        id: Set(id.clone()),
        title: Set(title.to_string()),
        description: Set(description.map(String::from)),
        status: Set("open".to_string()),
        priority: Set(priority.to_string()),
        assignee_id: Set(assignee_id.map(String::from)),
        labels: Set(labels_json),
        room_id: Set(effective_room_id.clone()),
        created_at: NotSet,
        updated_at: NotSet,
        closed_at: Set(None),
    };

    task_entity::Entity::insert(model).exec(db).await?;

    let event_model = event_entity::ActiveModel {
        id: Set(Uuid::now_v7().to_string()),
        task_id: Set(id.clone()),
        event_type: Set("created".to_string()),
        old_value: Set(None),
        new_value: Set(Some(title.to_string())),
        actor_id: Set(actor_id.map(String::from)),
        created_at: NotSet,
    };

    event_entity::Entity::insert(event_model).exec(db).await?;

    if let Some(ref rid) = effective_room_id {
        let _ = post_task_event_to_room(
            db,
            registry,
            &id,
            rid,
            "created",
            None,
            Some(title),
            actor_id,
        )
        .await;
    }

    let model = task_entity::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;

    Ok(Task::from_model(model))
}

#[allow(clippy::too_many_arguments)]
pub async fn list_tasks(
    db: &DatabaseConnection,
    status: Option<&str>,
    assignee_id: Option<&str>,
    label: Option<&str>,
    limit: Option<u32>,
    offset: Option<u32>,
    order_by: Option<&str>,
    order_dir: Option<&str>,
) -> Result<Vec<Task>, NousError> {
    let limit = limit.unwrap_or(50).min(200);
    let offset = offset.unwrap_or(0);
    let order_dir = order_dir.unwrap_or("DESC");

    let order_clause = match order_by.unwrap_or("created_at") {
        "priority" => format!(
            "CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END {order_dir}"
        ),
        "updated_at" => format!("updated_at {order_dir}"),
        "status" => format!("status {order_dir}"),
        _ => format!("created_at {order_dir}"),
    };

    let mut sql = String::from("SELECT tasks.* FROM tasks");
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<sea_orm::Value> = Vec::new();

    if label.is_some() {
        sql.push_str(", json_each(tasks.labels) AS je");
    }

    if let Some(s) = status {
        conditions.push("tasks.status = ?".to_string());
        values.push(s.to_string().into());
    }

    if let Some(a) = assignee_id {
        conditions.push("tasks.assignee_id = ?".to_string());
        values.push(a.to_string().into());
    }

    if let Some(l) = label {
        conditions.push("je.value = ?".to_string());
        values.push(l.to_string().into());
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(&format!(" ORDER BY {order_clause} LIMIT ? OFFSET ?"));
    values.push((limit as i32).into());
    values.push((offset as i32).into());

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;

    let mut tasks = Vec::new();
    for row in rows {
        let m = <task_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        tasks.push(Task::from_model(m));
    }
    Ok(tasks)
}

pub async fn get_task(db: &DatabaseConnection, id: &str) -> Result<Task, NousError> {
    let model = task_entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;

    let mut task = Task::from_model(model);

    let links = list_links(db, id).await?;
    task.links = Some(links);

    if let Some(ref room_id) = task.room_id {
        let msgs = messages::read_messages(
            db,
            messages::ReadMessagesRequest {
                room_id: room_id.clone(),
                since: None,
                before: None,
                limit: Some(5),
            },
        )
        .await
        .unwrap_or_default();

        if !msgs.is_empty() {
            task.recent_discussion = Some(
                msgs.into_iter()
                    .map(|m| serde_json::to_value(m).unwrap_or_default())
                    .collect(),
            );
        }
    }

    Ok(task)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_task(
    db: &DatabaseConnection,
    id: &str,
    status: Option<&str>,
    priority: Option<&str>,
    assignee_id: Option<&str>,
    description: Option<&str>,
    labels: Option<&[String]>,
    actor_id: Option<&str>,
    registry: Option<&NotificationRegistry>,
) -> Result<Task, NousError> {
    let existing_model = task_entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;

    let existing = Task::from_model(existing_model);

    if let Some(new_status) = status {
        if new_status != existing.status {
            let event_model = event_entity::ActiveModel {
                id: Set(Uuid::now_v7().to_string()),
                task_id: Set(id.to_string()),
                event_type: Set("status_changed".to_string()),
                old_value: Set(Some(existing.status.clone())),
                new_value: Set(Some(new_status.to_string())),
                actor_id: Set(actor_id.map(String::from)),
                created_at: NotSet,
            };
            event_entity::Entity::insert(event_model).exec(db).await?;

            db.execute(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "UPDATE tasks SET status = ? WHERE id = ?",
                [new_status.into(), id.into()],
            ))
            .await?;

            if new_status == "closed" {
                db.execute(Statement::from_sql_and_values(
                    sea_orm::DbBackend::Sqlite,
                    "UPDATE tasks SET closed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
                    [id.into()],
                ))
                .await?;
            }

            if let Some(ref rid) = existing.room_id {
                let _ = post_task_event_to_room(
                    db,
                    registry,
                    id,
                    rid,
                    "status_changed",
                    Some(&existing.status),
                    Some(new_status),
                    actor_id,
                )
                .await;
            }
        }
    }

    if let Some(new_priority) = priority {
        if new_priority != existing.priority {
            let event_model = event_entity::ActiveModel {
                id: Set(Uuid::now_v7().to_string()),
                task_id: Set(id.to_string()),
                event_type: Set("priority_changed".to_string()),
                old_value: Set(Some(existing.priority.clone())),
                new_value: Set(Some(new_priority.to_string())),
                actor_id: Set(actor_id.map(String::from)),
                created_at: NotSet,
            };
            event_entity::Entity::insert(event_model).exec(db).await?;

            db.execute(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "UPDATE tasks SET priority = ? WHERE id = ?",
                [new_priority.into(), id.into()],
            ))
            .await?;

            if let Some(ref rid) = existing.room_id {
                let _ = post_task_event_to_room(
                    db,
                    registry,
                    id,
                    rid,
                    "priority_changed",
                    Some(&existing.priority),
                    Some(new_priority),
                    actor_id,
                )
                .await;
            }
        }
    }

    if let Some(new_assignee) = assignee_id {
        let old_assignee = existing.assignee_id.as_deref().unwrap_or("");
        if new_assignee != old_assignee {
            match agents::get_agent_by_id(db, new_assignee).await {
                Ok(agent) => {
                    tracing::info!(
                        task_id = id,
                        assignee = new_assignee,
                        agent_name = %agent.name,
                        "task assigned to registered agent"
                    );
                }
                Err(_) => {
                    tracing::warn!(
                        task_id = id,
                        assignee = new_assignee,
                        "task assigned to unregistered agent ID"
                    );
                }
            }

            let event_model = event_entity::ActiveModel {
                id: Set(Uuid::now_v7().to_string()),
                task_id: Set(id.to_string()),
                event_type: Set("assigned".to_string()),
                old_value: Set(Some(old_assignee.to_string())),
                new_value: Set(Some(new_assignee.to_string())),
                actor_id: Set(actor_id.map(String::from)),
                created_at: NotSet,
            };
            event_entity::Entity::insert(event_model).exec(db).await?;

            db.execute(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "UPDATE tasks SET assignee_id = ? WHERE id = ?",
                [new_assignee.into(), id.into()],
            ))
            .await?;

            if let Some(ref rid) = existing.room_id {
                let _ = post_task_event_to_room(
                    db,
                    registry,
                    id,
                    rid,
                    "assigned",
                    Some(old_assignee),
                    Some(new_assignee),
                    actor_id,
                )
                .await;
            }
        }
    }

    if let Some(desc) = description {
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE tasks SET description = ? WHERE id = ?",
            [desc.into(), id.into()],
        ))
        .await?;
    }

    if let Some(new_labels) = labels {
        let labels_json = serde_json::to_string(new_labels).unwrap_or_else(|_| "[]".to_string());
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE tasks SET labels = ? WHERE id = ?",
            [labels_json.into(), id.into()],
        ))
        .await?;
    }

    let model = task_entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;

    Ok(Task::from_model(model))
}

pub async fn close_task(
    db: &DatabaseConnection,
    id: &str,
    actor_id: Option<&str>,
) -> Result<Task, NousError> {
    update_task(
        db,
        id,
        Some("closed"),
        None,
        None,
        None,
        None,
        actor_id,
        None,
    )
    .await
}

pub async fn link_tasks(
    db: &DatabaseConnection,
    source_id: &str,
    target_id: &str,
    link_type: &str,
    actor_id: Option<&str>,
) -> Result<TaskLink, NousError> {
    // Cycle detection for blocked_by and parent link types
    if link_type == "blocked_by" || link_type == "parent" {
        let mut visited = std::collections::HashSet::new();
        let mut stack = vec![target_id.to_string()];

        while let Some(current) = stack.pop() {
            if current == source_id {
                return Err(NousError::CyclicLink(format!(
                    "linking {source_id} -> {target_id} would create a cycle"
                )));
            }
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current.clone());

            let rows = db
                .query_all(Statement::from_sql_and_values(
                    sea_orm::DbBackend::Sqlite,
                    "SELECT target_id FROM task_links WHERE source_id = ? AND link_type = ?",
                    [current.clone().into(), link_type.into()],
                ))
                .await?;

            for row in &rows {
                let tid: String = row.try_get("", "target_id")?;
                stack.push(tid);
            }
        }
    }

    let id = Uuid::now_v7().to_string();

    let model = link_entity::ActiveModel {
        id: Set(id.clone()),
        source_id: Set(source_id.to_string()),
        target_id: Set(target_id.to_string()),
        link_type: Set(link_type.to_string()),
        created_at: NotSet,
    };

    link_entity::Entity::insert(model).exec(db).await?;

    let event_model = event_entity::ActiveModel {
        id: Set(Uuid::now_v7().to_string()),
        task_id: Set(source_id.to_string()),
        event_type: Set("linked".to_string()),
        old_value: Set(None),
        new_value: Set(Some(format!("{link_type}:{target_id}"))),
        actor_id: Set(actor_id.map(String::from)),
        created_at: NotSet,
    };

    event_entity::Entity::insert(event_model).exec(db).await?;

    let model = link_entity::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("link '{id}' not found")))?;

    Ok(TaskLink::from_model(model))
}

pub async fn unlink_tasks(
    db: &DatabaseConnection,
    source_id: &str,
    target_id: &str,
    link_type: &str,
    actor_id: Option<&str>,
) -> Result<(), NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "DELETE FROM task_links WHERE source_id = ? AND target_id = ? AND link_type = ?",
            [source_id.into(), target_id.into(), link_type.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!(
            "link {source_id} -> {target_id} ({link_type}) not found"
        )));
    }

    let event_model = event_entity::ActiveModel {
        id: Set(Uuid::now_v7().to_string()),
        task_id: Set(source_id.to_string()),
        event_type: Set("unlinked".to_string()),
        old_value: Set(Some(format!("{link_type}:{target_id}"))),
        new_value: Set(None),
        actor_id: Set(actor_id.map(String::from)),
        created_at: NotSet,
    };

    event_entity::Entity::insert(event_model).exec(db).await?;

    Ok(())
}

pub async fn list_links(db: &DatabaseConnection, task_id: &str) -> Result<TaskLinks, NousError> {
    let models = link_entity::Entity::find()
        .filter(
            sea_orm::Condition::any()
                .add(link_entity::Column::SourceId.eq(task_id))
                .add(link_entity::Column::TargetId.eq(task_id)),
        )
        .all(db)
        .await?;

    let mut blocked_by = Vec::new();
    let mut parent = Vec::new();
    let mut related_to = Vec::new();

    for m in models {
        let link = TaskLink::from_model(m);
        match link.link_type.as_str() {
            "blocked_by" if link.source_id == task_id => {
                blocked_by.push(link.target_id);
            }
            "parent" if link.source_id == task_id => {
                parent.push(link.target_id);
            }
            "related_to" => {
                if link.source_id == task_id {
                    related_to.push(link.target_id.clone());
                } else {
                    related_to.push(link.source_id.clone());
                }
            }
            _ => {}
        }
    }

    Ok(TaskLinks {
        blocked_by,
        parent,
        related_to,
    })
}

pub async fn add_note(
    db: &DatabaseConnection,
    task_id: &str,
    sender_id: &str,
    content: &str,
) -> Result<serde_json::Value, NousError> {
    let task_model = task_entity::Entity::find_by_id(task_id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("task '{task_id}' not found")))?;

    let room_id = match task_model.room_id {
        Some(rid) => rid,
        None => {
            // Auto-create a room for this task
            let room_name = format!("task-{task_id}");
            let room = rooms::create_room(
                db,
                &room_name,
                Some(&format!("Auto-created discussion room for task {task_id}")),
                None,
            )
            .await?;
            // Link the room to the task
            db.execute(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "UPDATE tasks SET room_id = ? WHERE id = ?",
                [room.id.clone().into(), task_id.into()],
            ))
            .await?;
            room.id
        }
    };

    let metadata = serde_json::json!({
        "topics": [format!("task:{task_id}")]
    });

    let msg = messages::post_message(
        db,
        messages::PostMessageRequest {
            room_id,
            sender_id: sender_id.to_string(),
            content: content.to_string(),
            reply_to: None,
            metadata: Some(metadata),
            message_type: None,
        },
        None,
    )
    .await?;

    let event_model = event_entity::ActiveModel {
        id: Set(Uuid::now_v7().to_string()),
        task_id: Set(task_id.to_string()),
        event_type: Set("note_added".to_string()),
        old_value: Set(None),
        new_value: Set(Some(msg.id.clone())),
        actor_id: Set(Some(sender_id.to_string())),
        created_at: NotSet,
    };

    event_entity::Entity::insert(event_model).exec(db).await?;

    serde_json::to_value(&msg).map_err(|e| NousError::Internal(e.to_string()))
}

pub async fn task_history(
    db: &DatabaseConnection,
    task_id: &str,
    limit: Option<u32>,
) -> Result<Vec<TaskEvent>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT * FROM task_events WHERE task_id = ? ORDER BY created_at DESC LIMIT ?",
            [task_id.into(), (limit as i32).into()],
        ))
        .await?;

    let mut events = Vec::new();
    for row in rows {
        let m = <event_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        events.push(TaskEvent::from_model(m));
    }
    Ok(events)
}

pub async fn search_tasks(
    db: &DatabaseConnection,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<Task>, NousError> {
    if query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT t.* FROM tasks t \
             JOIN tasks_fts fts ON t.rowid = fts.rowid \
             WHERE tasks_fts MATCH ?1 \
             LIMIT ?2",
            [query.into(), (limit as i64).into()],
        ))
        .await?;

    let mut tasks = Vec::new();
    for row in rows {
        let m = <task_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        tasks.push(Task::from_model(m));
    }
    Ok(tasks)
}

// --- Task Dependencies ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDependency {
    pub id: String,
    pub task_id: String,
    pub depends_on_task_id: String,
    pub dep_type: String,
    pub created_at: String,
}

impl TaskDependency {
    fn from_model(m: dep_entity::Model) -> Self {
        Self {
            id: m.id,
            task_id: m.task_id,
            depends_on_task_id: m.depends_on_task_id,
            dep_type: m.dep_type,
            created_at: m.created_at,
        }
    }
}

pub async fn add_dependency(
    db: &DatabaseConnection,
    task_id: &str,
    depends_on_task_id: &str,
    dep_type: Option<&str>,
) -> Result<TaskDependency, NousError> {
    // Validate both tasks exist
    let _ = get_task(db, task_id).await?;
    let _ = get_task(db, depends_on_task_id).await?;

    if task_id == depends_on_task_id {
        return Err(NousError::Validation("task cannot depend on itself".into()));
    }

    let dep_type = dep_type.unwrap_or("blocked_by");
    if !matches!(dep_type, "blocked_by" | "blocks" | "waiting_on") {
        return Err(NousError::Validation(format!(
            "invalid dep_type: '{dep_type}' — must be blocked_by, blocks, or waiting_on"
        )));
    }

    // Check for circular dependencies
    if would_create_cycle(db, task_id, depends_on_task_id).await? {
        return Err(NousError::Conflict(
            "would create circular dependency".into(),
        ));
    }

    let id = Uuid::now_v7().to_string();
    let model = dep_entity::ActiveModel {
        id: Set(id.clone()),
        task_id: Set(task_id.to_string()),
        depends_on_task_id: Set(depends_on_task_id.to_string()),
        dep_type: Set(dep_type.to_string()),
        created_at: NotSet,
    };

    let result = dep_entity::Entity::insert(model).exec(db).await;
    match result {
        Ok(_) => {}
        Err(ref e) if e.to_string().contains("2067") || e.to_string().contains("UNIQUE") => {
            return Err(NousError::Conflict("dependency already exists".into()));
        }
        Err(e) => return Err(NousError::SeaOrm(e)),
    }

    get_dependency_by_id(db, &id).await
}

async fn get_dependency_by_id(
    db: &DatabaseConnection,
    id: &str,
) -> Result<TaskDependency, NousError> {
    let model = dep_entity::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("dependency '{id}' not found")))?;

    Ok(TaskDependency::from_model(model))
}

pub async fn remove_dependency(
    db: &DatabaseConnection,
    task_id: &str,
    depends_on_task_id: &str,
    dep_type: Option<&str>,
) -> Result<(), NousError> {
    let dep_type = dep_type.unwrap_or("blocked_by");
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "DELETE FROM task_dependencies WHERE task_id = ? AND depends_on_task_id = ? AND dep_type = ?",
            [task_id.into(), depends_on_task_id.into(), dep_type.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound("dependency not found".into()));
    }
    Ok(())
}

pub async fn list_dependencies(
    db: &DatabaseConnection,
    task_id: &str,
) -> Result<Vec<TaskDependency>, NousError> {
    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT * FROM task_dependencies WHERE task_id = ? OR depends_on_task_id = ? ORDER BY created_at DESC",
            [task_id.into(), task_id.into()],
        ))
        .await?;

    let mut deps = Vec::new();
    for row in rows {
        let m = <dep_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        deps.push(TaskDependency::from_model(m));
    }
    Ok(deps)
}

async fn would_create_cycle(
    db: &DatabaseConnection,
    task_id: &str,
    depends_on_task_id: &str,
) -> Result<bool, NousError> {
    // BFS from depends_on_task_id — if we reach task_id, it's a cycle
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(depends_on_task_id.to_string());

    while let Some(current) = queue.pop_front() {
        if current == task_id {
            return Ok(true);
        }
        if !visited.insert(current.clone()) {
            continue;
        }
        let rows = db
            .query_all(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT depends_on_task_id FROM task_dependencies WHERE task_id = ?",
                [current.into()],
            ))
            .await?;
        for row in rows {
            let dep: String = row.try_get("", "depends_on_task_id")?;
            queue.push_back(dep);
        }
    }
    Ok(false)
}

// --- Task Templates ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskTemplate {
    pub id: String,
    pub name: String,
    pub title_pattern: String,
    pub description_template: Option<String>,
    pub default_priority: String,
    pub default_labels: Vec<String>,
    pub checklist: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TaskTemplate {
    fn from_model(m: template_entity::Model) -> Self {
        let default_labels: Vec<String> =
            serde_json::from_str(&m.default_labels).unwrap_or_default();
        let checklist: Vec<String> = serde_json::from_str(&m.checklist).unwrap_or_default();

        Self {
            id: m.id,
            name: m.name,
            title_pattern: m.title_pattern,
            description_template: m.description_template,
            default_priority: m.default_priority,
            default_labels,
            checklist,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

pub async fn create_template(
    db: &DatabaseConnection,
    name: &str,
    title_pattern: &str,
    description_template: Option<&str>,
    default_priority: Option<&str>,
    default_labels: Option<&[String]>,
    checklist: Option<&[String]>,
) -> Result<TaskTemplate, NousError> {
    if name.trim().is_empty() {
        return Err(NousError::Validation(
            "template name cannot be empty".into(),
        ));
    }
    if title_pattern.trim().is_empty() {
        return Err(NousError::Validation(
            "title_pattern cannot be empty".into(),
        ));
    }

    let id = Uuid::now_v7().to_string();
    let priority = default_priority.unwrap_or("medium");
    let labels_json = serde_json::to_string(&default_labels.unwrap_or(&[])).unwrap();
    let checklist_json = serde_json::to_string(&checklist.unwrap_or(&[])).unwrap();

    let model = template_entity::ActiveModel {
        id: Set(id.clone()),
        name: Set(name.trim().to_string()),
        title_pattern: Set(title_pattern.trim().to_string()),
        description_template: Set(description_template.map(String::from)),
        default_priority: Set(priority.to_string()),
        default_labels: Set(labels_json),
        checklist: Set(checklist_json),
        created_at: NotSet,
        updated_at: NotSet,
    };

    let result = template_entity::Entity::insert(model).exec(db).await;
    match result {
        Ok(_) => {}
        Err(ref e) if e.to_string().contains("2067") || e.to_string().contains("UNIQUE") => {
            return Err(NousError::Conflict(format!(
                "template '{}' already exists",
                name.trim()
            )));
        }
        Err(e) => return Err(NousError::SeaOrm(e)),
    }

    get_template(db, &id).await
}

pub async fn get_template(db: &DatabaseConnection, id: &str) -> Result<TaskTemplate, NousError> {
    // Try by ID first
    let model = template_entity::Entity::find_by_id(id).one(db).await?;
    if let Some(m) = model {
        return Ok(TaskTemplate::from_model(m));
    }

    // Try by name
    let model = template_entity::Entity::find()
        .filter(template_entity::Column::Name.eq(id))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(TaskTemplate::from_model(m)),
        None => Err(NousError::NotFound(format!("template '{id}' not found"))),
    }
}

pub async fn list_templates(
    db: &DatabaseConnection,
    limit: Option<u32>,
) -> Result<Vec<TaskTemplate>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT * FROM task_templates ORDER BY created_at DESC LIMIT ?",
            [(limit as i32).into()],
        ))
        .await?;

    let mut templates = Vec::new();
    for row in rows {
        let m = <template_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        templates.push(TaskTemplate::from_model(m));
    }
    Ok(templates)
}

pub async fn create_from_template(
    db: &DatabaseConnection,
    template_id: &str,
    title_vars: Option<&HashMap<String, String>>,
    overrides_description: Option<&str>,
    overrides_assignee: Option<&str>,
    overrides_labels: Option<&[String]>,
) -> Result<Task, NousError> {
    let template = get_template(db, template_id).await?;

    // Substitute variables in title pattern: {{var_name}} -> value
    let mut title = template.title_pattern.clone();
    if let Some(vars) = title_vars {
        for (key, value) in vars {
            title = title.replace(&format!("{{{{{key}}}}}"), value);
        }
    }

    let labels = overrides_labels
        .map(|l| l.to_vec())
        .unwrap_or(template.default_labels);

    // Substitute variables in description_template too
    let description = if let Some(desc_override) = overrides_description {
        Some(desc_override.to_string())
    } else if let Some(mut desc) = template.description_template {
        if let Some(vars) = title_vars {
            for (key, value) in vars {
                desc = desc.replace(&format!("{{{{{key}}}}}"), value);
            }
        }
        Some(desc)
    } else {
        None
    };

    create_task(
        db,
        &title,
        description.as_deref(),
        Some(&template.default_priority),
        overrides_assignee,
        Some(&labels),
        None,
        false,
        None,
        None,
    )
    .await
}

// --- Batch Operations ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResult {
    pub succeeded: Vec<String>,
    pub failed: Vec<BatchError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchError {
    pub id: String,
    pub error: String,
}

pub async fn batch_close(
    db: &DatabaseConnection,
    task_ids: &[String],
) -> Result<BatchResult, NousError> {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for id in task_ids {
        match close_task(db, id, None).await {
            Ok(_) => succeeded.push(id.clone()),
            Err(e) => failed.push(BatchError {
                id: id.clone(),
                error: e.to_string(),
            }),
        }
    }

    Ok(BatchResult { succeeded, failed })
}

pub async fn batch_update_status(
    db: &DatabaseConnection,
    task_ids: &[String],
    status: &str,
) -> Result<BatchResult, NousError> {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for id in task_ids {
        match update_task(db, id, Some(status), None, None, None, None, None, None).await {
            Ok(_) => succeeded.push(id.clone()),
            Err(e) => failed.push(BatchError {
                id: id.clone(),
                error: e.to_string(),
            }),
        }
    }

    Ok(BatchResult { succeeded, failed })
}

pub async fn batch_assign(
    db: &DatabaseConnection,
    task_ids: &[String],
    assignee_id: &str,
) -> Result<BatchResult, NousError> {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();

    for id in task_ids {
        match update_task(
            db,
            id,
            None,
            None,
            Some(assignee_id),
            None,
            None,
            None,
            None,
        )
        .await
        {
            Ok(_) => succeeded.push(id.clone()),
            Err(e) => failed.push(BatchError {
                id: id.clone(),
                error: e.to_string(),
            }),
        }
    }

    Ok(BatchResult { succeeded, failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::messages::{read_messages, ReadMessagesRequest};
    use crate::rooms::create_room;
    use tempfile::TempDir;

    async fn setup() -> (DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        let db = pools.fts.clone();
        (db, tmp)
    }

    #[tokio::test]
    async fn test_post_task_event_to_room_status_change() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "task-event-room", None, None)
            .await
            .unwrap();

        let task = create_task(
            &db,
            "Bridge test task",
            None,
            None,
            None,
            None,
            Some(&room.id),
            false,
            None,
            None,
        )
        .await
        .unwrap();

        post_task_event_to_room(
            &db,
            None,
            &task.id,
            &room.id,
            "status_changed",
            Some("open"),
            Some("in_progress"),
            Some("actor-1"),
        )
        .await
        .unwrap();

        let msgs = read_messages(
            &db,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: None,
                before: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        let status_events: Vec<_> = msgs
            .iter()
            .filter(|m| {
                m.message_type == MessageType::TaskEvent && m.content.contains("Task status:")
            })
            .collect();
        assert!(!status_events.is_empty());
        assert!(status_events[0].content.contains("open"));
        assert!(status_events[0].content.contains("in_progress"));
    }

    #[tokio::test]
    async fn test_task_event_not_posted_when_no_room() {
        let (db, _tmp) = setup().await;

        let task = create_task(
            &db,
            "No room task",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(task.room_id.is_none());

        let result = update_task(
            &db,
            &task.id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_task_command_close() {
        let (db, _tmp) = setup().await;

        let task = create_task(
            &db,
            "Close cmd",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        let result = execute_task_command(
            &db,
            TaskCommand {
                command: "close".to_string(),
                task_id: task.id.clone(),
                args: vec![],
                actor_id: "actor-1".to_string(),
            },
            None,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert_eq!(result.command, "close");
        assert!(result.task.is_some());
        assert_eq!(result.task.unwrap().status, "closed");
    }

    #[tokio::test]
    async fn test_execute_task_command_assign() {
        let (db, _tmp) = setup().await;

        let task = create_task(
            &db,
            "Assign cmd",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        let result = execute_task_command(
            &db,
            TaskCommand {
                command: "assign".to_string(),
                task_id: task.id.clone(),
                args: vec!["agent-1".to_string()],
                actor_id: "actor-1".to_string(),
            },
            None,
        )
        .await
        .unwrap();

        assert!(result.success);
        assert!(result.task.is_some());
        assert_eq!(result.task.unwrap().assignee_id.as_deref(), Some("agent-1"));
    }

    #[tokio::test]
    async fn test_execute_task_command_invalid() {
        let (db, _tmp) = setup().await;

        let task = create_task(
            &db,
            "Invalid cmd",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();

        let result = execute_task_command(
            &db,
            TaskCommand {
                command: "foobar".to_string(),
                task_id: task.id.clone(),
                args: vec![],
                actor_id: "actor-1".to_string(),
            },
            None,
        )
        .await;

        assert!(matches!(result, Err(NousError::Validation(_))));
    }

    #[tokio::test]
    async fn test_update_task_with_room_posts_event() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "update-event-room", None, None)
            .await
            .unwrap();

        let task = create_task(
            &db,
            "Room event task",
            None,
            None,
            None,
            None,
            Some(&room.id),
            false,
            None,
            None,
        )
        .await
        .unwrap();

        update_task(
            &db,
            &task.id,
            Some("in_progress"),
            None,
            None,
            None,
            None,
            Some("actor-1"),
            None,
        )
        .await
        .unwrap();

        let msgs = read_messages(
            &db,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: None,
                before: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        let task_events: Vec<_> = msgs
            .iter()
            .filter(|m| m.message_type == MessageType::TaskEvent)
            .collect();

        assert!(task_events.len() >= 2);

        let status_events: Vec<_> = task_events
            .iter()
            .filter(|m| m.content.contains("Task status:"))
            .collect();
        assert!(!status_events.is_empty());
    }
}
