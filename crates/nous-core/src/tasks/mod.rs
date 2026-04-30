use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;
use crate::messages;
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
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        let labels_str: Option<String> = row.try_get("labels")?;
        let labels: Option<Vec<String>> = match labels_str.as_deref().map(serde_json::from_str) {
            Some(Ok(val)) => Some(val),
            Some(Err(e)) => {
                tracing::warn!(error = %e, "malformed JSON in tasks labels column, treating as null");
                None
            }
            None => None,
        };

        Ok(Self {
            id: row.try_get("id")?,
            title: row.try_get("title")?,
            description: row.try_get("description")?,
            status: row.try_get("status")?,
            priority: row.try_get("priority")?,
            assignee_id: row.try_get("assignee_id")?,
            labels,
            room_id: row.try_get("room_id")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            closed_at: row.try_get("closed_at")?,
            links: None,
            recent_discussion: None,
        })
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
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            source_id: row.try_get("source_id")?,
            target_id: row.try_get("target_id")?,
            link_type: row.try_get("link_type")?,
            created_at: row.try_get("created_at")?,
        })
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
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            task_id: row.try_get("task_id")?,
            event_type: row.try_get("event_type")?,
            old_value: row.try_get("old_value")?,
            new_value: row.try_get("new_value")?,
            actor_id: row.try_get("actor_id")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskLinks {
    pub blocked_by: Vec<String>,
    pub parent: Vec<String>,
    pub related_to: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_task(
    pool: &SqlitePool,
    title: &str,
    description: Option<&str>,
    priority: Option<&str>,
    assignee_id: Option<&str>,
    labels: Option<&[String]>,
    room_id: Option<&str>,
    create_room: bool,
    actor_id: Option<&str>,
) -> Result<Task, NousError> {
    if title.trim().is_empty() {
        return Err(NousError::Validation("task title cannot be empty".into()));
    }

    let id = Uuid::now_v7().to_string();
    let priority = priority.unwrap_or("medium");

    let effective_room_id = if create_room {
        let room_name = format!("task-{id}");
        let room = rooms::create_room(pool, &room_name, Some(&format!("Discussion for task: {title}")), None).await?;
        Some(room.id)
    } else {
        room_id.map(String::from)
    };

    let labels_json = labels.map(|l| serde_json::to_string(l).unwrap_or_else(|_| "[]".to_string()));

    sqlx::query(
        "INSERT INTO tasks (id, title, description, priority, assignee_id, labels, room_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(title)
    .bind(description)
    .bind(priority)
    .bind(assignee_id)
    .bind(&labels_json)
    .bind(&effective_room_id)
    .execute(pool)
    .await?;

    let event_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO task_events (id, task_id, event_type, new_value, actor_id) VALUES (?, ?, 'created', ?, ?)",
    )
    .bind(&event_id)
    .bind(&id)
    .bind(title)
    .bind(actor_id)
    .execute(pool)
    .await?;

    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;

    Task::from_row(&row).map_err(NousError::Sqlite)
}

#[allow(clippy::too_many_arguments)]
pub async fn list_tasks(
    pool: &SqlitePool,
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
    let mut binds: Vec<String> = Vec::new();

    if label.is_some() {
        sql.push_str(", json_each(tasks.labels) AS je");
    }

    if let Some(s) = status {
        conditions.push("tasks.status = ?".to_string());
        binds.push(s.to_string());
    }

    if let Some(a) = assignee_id {
        conditions.push("tasks.assignee_id = ?".to_string());
        binds.push(a.to_string());
    }

    if let Some(l) = label {
        conditions.push("je.value = ?".to_string());
        binds.push(l.to_string());
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(&format!(" ORDER BY {order_clause} LIMIT ? OFFSET ?"));
    binds.push(limit.to_string());
    binds.push(offset.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    rows.iter()
        .map(Task::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn get_task(pool: &SqlitePool, id: &str) -> Result<Task, NousError> {
    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;
    let mut task = Task::from_row(&row).map_err(NousError::Sqlite)?;

    let links = list_links(pool, id).await?;
    task.links = Some(links);

    if let Some(ref room_id) = task.room_id {
        let msgs = messages::read_messages(
            pool,
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
    pool: &SqlitePool,
    id: &str,
    status: Option<&str>,
    priority: Option<&str>,
    assignee_id: Option<&str>,
    description: Option<&str>,
    labels: Option<&[String]>,
    actor_id: Option<&str>,
) -> Result<Task, NousError> {
    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("task '{id}' not found")))?;
    let existing = Task::from_row(&row).map_err(NousError::Sqlite)?;

    if let Some(new_status) = status {
        if new_status != existing.status {
            let event_id = Uuid::now_v7().to_string();
            sqlx::query(
                "INSERT INTO task_events (id, task_id, event_type, old_value, new_value, actor_id) VALUES (?, ?, 'status_changed', ?, ?, ?)",
            )
            .bind(&event_id)
            .bind(id)
            .bind(&existing.status)
            .bind(new_status)
            .bind(actor_id)
            .execute(pool)
            .await?;

            sqlx::query("UPDATE tasks SET status = ? WHERE id = ?")
                .bind(new_status)
                .bind(id)
                .execute(pool)
                .await?;

            if new_status == "closed" {
                sqlx::query("UPDATE tasks SET closed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
        }
    }

    if let Some(new_priority) = priority {
        if new_priority != existing.priority {
            let event_id = Uuid::now_v7().to_string();
            sqlx::query(
                "INSERT INTO task_events (id, task_id, event_type, old_value, new_value, actor_id) VALUES (?, ?, 'priority_changed', ?, ?, ?)",
            )
            .bind(&event_id)
            .bind(id)
            .bind(&existing.priority)
            .bind(new_priority)
            .bind(actor_id)
            .execute(pool)
            .await?;

            sqlx::query("UPDATE tasks SET priority = ? WHERE id = ?")
                .bind(new_priority)
                .bind(id)
                .execute(pool)
                .await?;
        }
    }

    if let Some(new_assignee) = assignee_id {
        let old_assignee = existing.assignee_id.as_deref().unwrap_or("");
        if new_assignee != old_assignee {
            let event_id = Uuid::now_v7().to_string();
            sqlx::query(
                "INSERT INTO task_events (id, task_id, event_type, old_value, new_value, actor_id) VALUES (?, ?, 'assigned', ?, ?, ?)",
            )
            .bind(&event_id)
            .bind(id)
            .bind(old_assignee)
            .bind(new_assignee)
            .bind(actor_id)
            .execute(pool)
            .await?;

            sqlx::query("UPDATE tasks SET assignee_id = ? WHERE id = ?")
                .bind(new_assignee)
                .bind(id)
                .execute(pool)
                .await?;
        }
    }

    if let Some(desc) = description {
        sqlx::query("UPDATE tasks SET description = ? WHERE id = ?")
            .bind(desc)
            .bind(id)
            .execute(pool)
            .await?;
    }

    if let Some(new_labels) = labels {
        let labels_json = serde_json::to_string(new_labels).unwrap_or_else(|_| "[]".to_string());
        sqlx::query("UPDATE tasks SET labels = ? WHERE id = ?")
            .bind(&labels_json)
            .bind(id)
            .execute(pool)
            .await?;
    }

    let row = sqlx::query("SELECT * FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;

    Task::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn close_task(
    pool: &SqlitePool,
    id: &str,
    actor_id: Option<&str>,
) -> Result<Task, NousError> {
    update_task(pool, id, Some("closed"), None, None, None, None, actor_id).await
}

pub async fn link_tasks(
    pool: &SqlitePool,
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

            let rows = sqlx::query(
                "SELECT target_id FROM task_links WHERE source_id = ? AND link_type = ?",
            )
            .bind(&current)
            .bind(link_type)
            .fetch_all(pool)
            .await?;

            for row in &rows {
                let tid: String = row.try_get("target_id").map_err(NousError::Sqlite)?;
                stack.push(tid);
            }
        }
    }

    let id = Uuid::now_v7().to_string();

    sqlx::query(
        "INSERT INTO task_links (id, source_id, target_id, link_type) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(source_id)
    .bind(target_id)
    .bind(link_type)
    .execute(pool)
    .await?;

    let event_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO task_events (id, task_id, event_type, new_value, actor_id) VALUES (?, ?, 'linked', ?, ?)",
    )
    .bind(&event_id)
    .bind(source_id)
    .bind(format!("{link_type}:{target_id}"))
    .bind(actor_id)
    .execute(pool)
    .await?;

    let row = sqlx::query("SELECT * FROM task_links WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;

    TaskLink::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn unlink_tasks(
    pool: &SqlitePool,
    source_id: &str,
    target_id: &str,
    link_type: &str,
    actor_id: Option<&str>,
) -> Result<(), NousError> {
    let result = sqlx::query(
        "DELETE FROM task_links WHERE source_id = ? AND target_id = ? AND link_type = ?",
    )
    .bind(source_id)
    .bind(target_id)
    .bind(link_type)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!(
            "link {source_id} -> {target_id} ({link_type}) not found"
        )));
    }

    let event_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO task_events (id, task_id, event_type, old_value, actor_id) VALUES (?, ?, 'unlinked', ?, ?)",
    )
    .bind(&event_id)
    .bind(source_id)
    .bind(format!("{link_type}:{target_id}"))
    .bind(actor_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn list_links(pool: &SqlitePool, task_id: &str) -> Result<TaskLinks, NousError> {
    let rows = sqlx::query("SELECT * FROM task_links WHERE source_id = ? OR target_id = ?")
        .bind(task_id)
        .bind(task_id)
        .fetch_all(pool)
        .await?;

    let mut blocked_by = Vec::new();
    let mut parent = Vec::new();
    let mut related_to = Vec::new();

    for row in &rows {
        let link = TaskLink::from_row(row).map_err(NousError::Sqlite)?;
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
    pool: &SqlitePool,
    task_id: &str,
    sender_id: &str,
    content: &str,
) -> Result<serde_json::Value, NousError> {
    let row = sqlx::query("SELECT room_id FROM tasks WHERE id = ?")
        .bind(task_id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("task '{task_id}' not found")))?;
    let room_id: Option<String> = row.try_get("room_id").map_err(NousError::Sqlite)?;

    let room_id = room_id.ok_or_else(|| {
        NousError::NoLinkedRoom(format!("task '{task_id}' has no linked room"))
    })?;

    let metadata = serde_json::json!({
        "topics": [format!("task:{task_id}")]
    });

    let msg = messages::post_message(
        pool,
        messages::PostMessageRequest {
            room_id,
            sender_id: sender_id.to_string(),
            content: content.to_string(),
            reply_to: None,
            metadata: Some(metadata),
        },
        None,
    )
    .await?;

    let event_id = Uuid::now_v7().to_string();
    sqlx::query(
        "INSERT INTO task_events (id, task_id, event_type, new_value, actor_id) VALUES (?, ?, 'note_added', ?, ?)",
    )
    .bind(&event_id)
    .bind(task_id)
    .bind(&msg.id)
    .bind(sender_id)
    .execute(pool)
    .await?;

    serde_json::to_value(&msg).map_err(|e| NousError::Internal(e.to_string()))
}

pub async fn task_history(
    pool: &SqlitePool,
    task_id: &str,
    limit: Option<u32>,
) -> Result<Vec<TaskEvent>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let rows = sqlx::query(
        "SELECT * FROM task_events WHERE task_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(TaskEvent::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn search_tasks(
    pool: &SqlitePool,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<Task>, NousError> {
    if query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let rows = sqlx::query(
        "SELECT t.* FROM tasks t \
         JOIN tasks_fts fts ON t.rowid = fts.rowid \
         WHERE tasks_fts MATCH ?1 \
         LIMIT ?2",
    )
    .bind(query)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(Task::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}
