use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::entities::{
    task_dependencies as dep_entity, task_events as event_entity, task_links as link_entity,
    task_templates as template_entity, tasks as task_entity,
};
use crate::notifications::NotificationRegistry;

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
    pub(crate) fn from_model(m: task_entity::Model) -> Self {
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
    pub(crate) fn from_model(m: link_entity::Model) -> Self {
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
    pub(crate) fn from_model(m: event_entity::Model) -> Self {
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

pub struct PostTaskEventParams<'a> {
    pub db: &'a DatabaseConnection,
    pub registry: Option<&'a NotificationRegistry>,
    pub task_id: &'a str,
    pub room_id: &'a str,
    pub event_type: &'a str,
    pub old_value: Option<&'a str>,
    pub new_value: Option<&'a str>,
    pub actor_id: Option<&'a str>,
}

pub struct CreateTaskParams<'a> {
    pub db: &'a DatabaseConnection,
    pub title: &'a str,
    pub description: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub assignee_id: Option<&'a str>,
    pub labels: Option<&'a [String]>,
    pub room_id: Option<&'a str>,
    pub create_room: bool,
    pub actor_id: Option<&'a str>,
    pub registry: Option<&'a NotificationRegistry>,
}

pub struct ListTasksParams<'a> {
    pub db: &'a DatabaseConnection,
    pub status: Option<&'a str>,
    pub assignee_id: Option<&'a str>,
    pub label: Option<&'a str>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub order_by: Option<&'a str>,
    pub order_dir: Option<&'a str>,
}

pub struct UpdateTaskParams<'a> {
    pub db: &'a DatabaseConnection,
    pub id: &'a str,
    pub status: Option<&'a str>,
    pub priority: Option<&'a str>,
    pub assignee_id: Option<&'a str>,
    pub description: Option<&'a str>,
    pub labels: Option<&'a [String]>,
    pub actor_id: Option<&'a str>,
    pub registry: Option<&'a NotificationRegistry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDependency {
    pub id: String,
    pub task_id: String,
    pub depends_on_task_id: String,
    pub dep_type: String,
    pub created_at: String,
}

impl TaskDependency {
    pub(crate) fn from_model(m: dep_entity::Model) -> Self {
        Self {
            id: m.id,
            task_id: m.task_id,
            depends_on_task_id: m.depends_on_task_id,
            dep_type: m.dep_type,
            created_at: m.created_at,
        }
    }
}

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
    pub(crate) fn from_model(m: template_entity::Model) -> Self {
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
