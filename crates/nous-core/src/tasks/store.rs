use std::collections::{HashMap, HashSet, VecDeque};

use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, NotSet, Set, Statement};
use uuid::Uuid;

use crate::agents;
use crate::entities::{
    task_dependencies as dep_entity, task_events as event_entity, task_links as link_entity,
    task_templates as template_entity, tasks as task_entity,
};
use crate::error::NousError;
use crate::messages;
use crate::rooms;

use super::events::post_task_event_to_room;
use super::types::{
    BatchError, BatchResult, CreateTaskParams, ListTasksParams, PostTaskEventParams, Task,
    TaskDependency, TaskEvent, TaskLink, TaskLinks, TaskTemplate, UpdateTaskParams,
};

pub async fn create_task(params: CreateTaskParams<'_>) -> Result<Task, NousError> {
    let CreateTaskParams {
        db,
        title,
        description,
        priority,
        assignee_id,
        labels,
        room_id,
        create_room,
        actor_id,
        registry,
    } = params;
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
        if let Err(e) = post_task_event_to_room(PostTaskEventParams {
            db,
            registry,
            task_id: &id,
            room_id: rid,
            event_type: "created",
            old_value: None,
            new_value: Some(title),
            actor_id,
        })
        .await
        {
            tracing::warn!(task_id = %id, error = %e, "failed to post task created event to room");
        }
    }

    let model = task_entity::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;

    Ok(Task::from_model(model))
}

pub async fn list_tasks(params: ListTasksParams<'_>) -> Result<Vec<Task>, NousError> {
    let ListTasksParams {
        db,
        status,
        assignee_id,
        label,
        limit,
        offset,
        order_by,
        order_dir,
    } = params;
    let limit = limit.unwrap_or(50).min(200);
    let offset = offset.unwrap_or(0);
    let order_dir = match order_dir.unwrap_or("DESC") {
        d if d.eq_ignore_ascii_case("asc") => "ASC",
        d if d.eq_ignore_ascii_case("desc") => "DESC",
        _ => "DESC",
    };

    let order_column = match order_by.unwrap_or("created_at") {
        "priority" | "updated_at" | "status" | "title" | "created_at" => {
            order_by.unwrap_or("created_at")
        }
        _ => "created_at",
    };

    let order_clause = match order_column {
        "priority" => format!(
            "CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END {order_dir}"
        ),
        "updated_at" => format!("updated_at {order_dir}"),
        "status" => format!("status {order_dir}"),
        "title" => format!("title {order_dir}"),
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

pub async fn update_task(params: UpdateTaskParams<'_>) -> Result<Task, NousError> {
    let UpdateTaskParams {
        db,
        id,
        status,
        priority,
        assignee_id,
        description,
        labels,
        actor_id,
        registry,
    } = params;
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
                if let Err(e) = post_task_event_to_room(PostTaskEventParams {
                    db,
                    registry,
                    task_id: id,
                    room_id: rid,
                    event_type: "status_changed",
                    old_value: Some(&existing.status),
                    new_value: Some(new_status),
                    actor_id,
                })
                .await
                {
                    tracing::warn!(task_id = %id, error = %e, "failed to post task status_changed event to room");
                }
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
                if let Err(e) = post_task_event_to_room(PostTaskEventParams {
                    db,
                    registry,
                    task_id: id,
                    room_id: rid,
                    event_type: "priority_changed",
                    old_value: Some(&existing.priority),
                    new_value: Some(new_priority),
                    actor_id,
                })
                .await
                {
                    tracing::warn!(task_id = %id, error = %e, "failed to post task priority_changed event to room");
                }
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
                if let Err(e) = post_task_event_to_room(PostTaskEventParams {
                    db,
                    registry,
                    task_id: id,
                    room_id: rid,
                    event_type: "assigned",
                    old_value: Some(old_assignee),
                    new_value: Some(new_assignee),
                    actor_id,
                })
                .await
                {
                    tracing::warn!(task_id = %id, error = %e, "failed to post task assigned event to room");
                }
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
    update_task(UpdateTaskParams {
        db,
        id,
        status: Some("closed"),
        priority: None,
        assignee_id: None,
        description: None,
        labels: None,
        actor_id,
        registry: None,
    })
    .await
}

pub async fn link_tasks(
    db: &DatabaseConnection,
    source_id: &str,
    target_id: &str,
    link_type: &str,
    actor_id: Option<&str>,
) -> Result<TaskLink, NousError> {
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
            let room_name = format!("task-{task_id}");
            let room = rooms::create_room(
                db,
                &room_name,
                Some(&format!("Auto-created discussion room for task {task_id}")),
                None,
            )
            .await?;
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

pub async fn add_dependency(
    db: &DatabaseConnection,
    task_id: &str,
    depends_on_task_id: &str,
    dep_type: Option<&str>,
) -> Result<TaskDependency, NousError> {
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
    let model = template_entity::Entity::find_by_id(id).one(db).await?;
    if let Some(m) = model {
        return Ok(TaskTemplate::from_model(m));
    }

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

    let mut title = template.title_pattern.clone();
    if let Some(vars) = title_vars {
        for (key, value) in vars {
            title = title.replace(&format!("{{{{{key}}}}}"), value);
        }
    }

    let labels = overrides_labels
        .map(|l| l.to_vec())
        .unwrap_or(template.default_labels);

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

    create_task(CreateTaskParams {
        db,
        title: &title,
        description: description.as_deref(),
        priority: Some(&template.default_priority),
        assignee_id: overrides_assignee,
        labels: Some(&labels),
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
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
        match update_task(UpdateTaskParams { db, id, status: Some(status), priority: None, assignee_id: None, description: None, labels: None, actor_id: None, registry: None }).await {
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
        match update_task(UpdateTaskParams {
            db,
            id,
            status: None,
            priority: None,
            assignee_id: Some(assignee_id),
            description: None,
            labels: None,
            actor_id: None,
            registry: None,
        })
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
    use crate::messages::{read_messages, MessageType, ReadMessagesRequest};
    use crate::rooms::create_room;
    use crate::tasks::types::TaskCommand;
    use tempfile::TempDir;

    async fn setup() -> (DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        let db = pools.fts.clone();
        for agent_id in ["agent-1", "agent-2", "agent-3", "actor-1"] {
            db.execute_unprepared(
                &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
            ).await.unwrap();
        }
        (db, tmp)
    }

    #[tokio::test]
    async fn test_post_task_event_to_room_status_change() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "task-event-room", None, None)
            .await
            .unwrap();

        let task = create_task(CreateTaskParams {
            db: &db,
            title: "Bridge test task",
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: Some(&room.id),
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();

        post_task_event_to_room(PostTaskEventParams {
            db: &db,
            registry: None,
            task_id: &task.id,
            room_id: &room.id,
            event_type: "status_changed",
            old_value: Some("open"),
            new_value: Some("in_progress"),
            actor_id: Some("actor-1"),
        })
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

        let task = create_task(CreateTaskParams {
            db: &db,
            title: "No room task",
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: None,
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();

        assert!(task.room_id.is_none());

        let result = update_task(UpdateTaskParams {
            db: &db,
            id: &task.id,
            status: Some("in_progress"),
            priority: None,
            assignee_id: None,
            description: None,
            labels: None,
            actor_id: None,
            registry: None,
        })
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_task_command_close() {
        let (db, _tmp) = setup().await;

        let task = create_task(CreateTaskParams {
            db: &db,
            title: "Close cmd",
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: None,
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();

        let result = super::super::events::execute_task_command(
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

        let task = create_task(CreateTaskParams {
            db: &db,
            title: "Assign cmd",
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: None,
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();

        let result = super::super::events::execute_task_command(
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

        let task = create_task(CreateTaskParams {
            db: &db,
            title: "Invalid cmd",
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: None,
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();

        let result = super::super::events::execute_task_command(
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

        let task = create_task(CreateTaskParams {
            db: &db,
            title: "Room event task",
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: Some(&room.id),
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();

        update_task(UpdateTaskParams {
            db: &db,
            id: &task.id,
            status: Some("in_progress"),
            priority: None,
            assignee_id: None,
            description: None,
            labels: None,
            actor_id: Some("actor-1"),
            registry: None,
        })
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
