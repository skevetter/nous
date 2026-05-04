use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, NotSet, Set, Statement};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::entities::{
    notification_queue as nq_entity, room_subscriptions as rs_entity, rooms as rooms_entity,
};
use crate::error::NousError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationPriority {
    Low,
    #[default]
    Normal,
    High,
    Urgent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub room_id: String,
    pub message_id: String,
    pub sender_id: String,
    pub priority: NotificationPriority,
    pub topics: Vec<String>,
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitResult {
    pub notification: Option<Notification>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub room_id: String,
    pub agent_id: String,
    pub topics: Option<Vec<String>>,
    pub created_at: String,
}

pub struct NotificationRegistry {
    channels: Arc<RwLock<HashMap<String, broadcast::Sender<Notification>>>>,
}

impl NotificationRegistry {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn get_sender(&self, room_id: &str) -> broadcast::Sender<Notification> {
        let channels = self.channels.read().await;
        if let Some(sender) = channels.get(room_id) {
            return sender.clone();
        }
        drop(channels);

        let mut channels = self.channels.write().await;
        channels
            .entry(room_id.to_string())
            .or_insert_with(|| broadcast::channel(256).0)
            .clone()
    }

    pub async fn subscribe(&self, room_id: &str) -> broadcast::Receiver<Notification> {
        self.get_sender(room_id).await.subscribe()
    }

    pub async fn notify(&self, notification: Notification) {
        let sender = self.get_sender(&notification.room_id).await;
        let _ = sender.send(notification);
    }

    pub async fn notify_filtered(&self, db: &DatabaseConnection, notification: Notification) {
        let sender = self.get_sender(&notification.room_id).await;
        let _ = sender.send(notification.clone());

        let rows = rs_entity::Entity::find()
            .filter(rs_entity::Column::RoomId.eq(&notification.room_id))
            .all(db)
            .await;

        if let Ok(rows) = rows {
            for row in &rows {
                let sub_topics: Option<Vec<String>> = row
                    .topics
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok());

                if should_notify_agent(&notification, &row.agent_id, sub_topics.as_deref()) {
                    if let Err(e) = enqueue_notification(db, &row.agent_id, &notification).await {
                        tracing::warn!(agent_id = %row.agent_id, error = %e, "failed to enqueue notification");
                    }
                }
            }
        }
    }

    pub async fn remove_room(&self, room_id: &str) {
        let mut channels = self.channels.write().await;
        channels.remove(room_id);
    }
}

impl Default for NotificationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn subscribe_to_room(
    db: &DatabaseConnection,
    room_id: &str,
    agent_id: &str,
    topics: Option<Vec<String>>,
) -> Result<(), NousError> {
    let room_exists = rooms_entity::Entity::find_by_id(room_id).count(db).await? > 0;

    if !room_exists {
        return Err(NousError::NotFound(format!("room '{room_id}' not found")));
    }

    let topics_json = topics.map(|t| serde_json::to_string(&t).unwrap_or_default());

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "INSERT OR REPLACE INTO room_subscriptions (room_id, agent_id, topics) VALUES (?, ?, ?)",
        [room_id.into(), agent_id.into(), topics_json.into()],
    ))
    .await?;

    Ok(())
}

pub async fn unsubscribe_from_room(
    db: &DatabaseConnection,
    room_id: &str,
    agent_id: &str,
) -> Result<(), NousError> {
    rs_entity::Entity::delete_many()
        .filter(rs_entity::Column::RoomId.eq(room_id))
        .filter(rs_entity::Column::AgentId.eq(agent_id))
        .exec(db)
        .await?;

    Ok(())
}

pub async fn list_subscriptions(
    db: &DatabaseConnection,
    agent_id: &str,
) -> Result<Vec<Subscription>, NousError> {
    let models = rs_entity::Entity::find()
        .filter(rs_entity::Column::AgentId.eq(agent_id))
        .all(db)
        .await?;

    Ok(models
        .into_iter()
        .map(|m| {
            let topics = m
                .topics
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok());

            Subscription {
                room_id: m.room_id,
                agent_id: m.agent_id,
                topics,
                created_at: m.created_at,
            }
        })
        .collect())
}

pub async fn room_wait(
    registry: &NotificationRegistry,
    room_id: &str,
    timeout_ms: Option<u64>,
    topics: Option<&[String]>,
) -> Result<WaitResult, NousError> {
    let timeout = timeout_ms.unwrap_or(30_000).min(120_000);
    let duration = std::time::Duration::from_millis(timeout);
    let mut receiver = registry.subscribe(room_id).await;

    let result = if let Some(topic_filter) = topics {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break WaitResult {
                    notification: None,
                    timed_out: true,
                };
            }

            match tokio::time::timeout(remaining, receiver.recv()).await {
                Ok(Ok(notification)) => {
                    if topic_filter.is_empty()
                        || notification.topics.iter().any(|t| topic_filter.contains(t))
                    {
                        break WaitResult {
                            notification: Some(notification),
                            timed_out: false,
                        };
                    }
                }
                Ok(Err(_)) => {
                    break WaitResult {
                        notification: None,
                        timed_out: false,
                    };
                }
                Err(_) => {
                    break WaitResult {
                        notification: None,
                        timed_out: true,
                    };
                }
            }
        }
    } else {
        match tokio::time::timeout(duration, receiver.recv()).await {
            Ok(Ok(notification)) => WaitResult {
                notification: Some(notification),
                timed_out: false,
            },
            Ok(Err(_)) => WaitResult {
                notification: None,
                timed_out: false,
            },
            Err(_) => WaitResult {
                notification: None,
                timed_out: true,
            },
        }
    };

    Ok(result)
}

pub fn should_notify_agent(
    notification: &Notification,
    agent_id: &str,
    topics: Option<&[String]>,
) -> bool {
    if notification.mentions.iter().any(|m| m == agent_id) {
        return true;
    }

    match topics {
        None | Some([]) => true,
        Some(t) => notification.topics.iter().any(|nt| t.contains(nt)),
    }
}

pub async fn enqueue_notification(
    db: &DatabaseConnection,
    agent_id: &str,
    notification: &Notification,
) -> Result<(), NousError> {
    let id = Uuid::now_v7().to_string();
    let topics_json = serde_json::to_string(&notification.topics).unwrap_or_else(|_| "[]".into());
    let mentions_json =
        serde_json::to_string(&notification.mentions).unwrap_or_else(|_| "[]".into());
    let priority = serde_json::to_string(&notification.priority)
        .unwrap_or_else(|_| "\"normal\"".into())
        .trim_matches('"')
        .to_string();

    let model = nq_entity::ActiveModel {
        id: Set(id),
        agent_id: Set(agent_id.to_string()),
        room_id: Set(notification.room_id.clone()),
        message_id: Set(notification.message_id.clone()),
        sender_id: Set(notification.sender_id.clone()),
        priority: Set(priority),
        topics: Set(topics_json),
        mentions: Set(mentions_json),
        delivered: Set(false),
        created_at: NotSet,
    };

    nq_entity::Entity::insert(model).exec(db).await?;

    Ok(())
}

pub async fn dequeue_notification(
    db: &DatabaseConnection,
    agent_id: &str,
    room_id: Option<&str>,
    topics: Option<&[String]>,
) -> Result<Option<Notification>, NousError> {
    let model = {
        let mut sql = String::from(
            "SELECT * FROM notification_queue WHERE agent_id = ? AND delivered = 0",
        );
        let mut values: Vec<sea_orm::Value> = vec![agent_id.into()];

        if let Some(rid) = room_id {
            sql.push_str(" AND room_id = ?");
            values.push(rid.into());
        }

        if let Some(filter) = topics {
            if !filter.is_empty() {
                // Filter notifications that have at least one matching topic.
                // Topics are stored as JSON arrays, so we use json_each to check membership.
                let placeholders: Vec<&str> = filter.iter().map(|_| "?").collect();
                let _ = write!(
                    sql,
                    " AND EXISTS (SELECT 1 FROM json_each(notification_queue.topics) AS je WHERE je.value IN ({}))",
                    placeholders.join(", ")
                );
                for t in filter {
                    values.push(t.clone().into());
                }
            }
        }

        sql.push_str(" ORDER BY created_at ASC LIMIT 1");

        let row = db
            .query_one(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                &sql,
                values,
            ))
            .await?;

        match row {
            Some(r) => {
                Some(<nq_entity::Model as sea_orm::FromQueryResult>::from_query_result(&r, "")?)
            }
            None => None,
        }
    };

    let Some(model) = model else {
        return Ok(None);
    };

    let notif_topics: Vec<String> = serde_json::from_str(&model.topics).unwrap_or_default();

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE notification_queue SET delivered = 1 WHERE id = ?",
        [model.id.clone().into()],
    ))
    .await?;

    let priority_str = &model.priority;
    let priority: NotificationPriority =
        serde_json::from_str(&format!("\"{priority_str}\"")).unwrap_or_default();

    let mentions: Vec<String> = serde_json::from_str(&model.mentions).unwrap_or_default();

    Ok(Some(Notification {
        room_id: model.room_id,
        message_id: model.message_id,
        sender_id: model.sender_id,
        priority,
        topics: notif_topics,
        mentions,
    }))
}

pub async fn room_wait_persistent(
    db: &DatabaseConnection,
    registry: &NotificationRegistry,
    room_id: &str,
    agent_id: &str,
    timeout_ms: Option<u64>,
    topics: Option<&[String]>,
) -> Result<WaitResult, NousError> {
    if let Some(notification) = dequeue_notification(db, agent_id, Some(room_id), topics).await? {
        return Ok(WaitResult {
            notification: Some(notification),
            timed_out: false,
        });
    }

    room_wait(registry, room_id, timeout_ms, topics).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::messages::{post_message, PostMessageRequest};
    use crate::rooms::create_room;
    use tempfile::TempDir;

    async fn setup() -> (DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        let db = pools.fts.clone();
        for agent_id in ["agent-1", "agent-2", "agent-3", "agent-eq", "persist-agent"] {
            db.execute_unprepared(
                &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
            ).await.unwrap();
        }
        (db, tmp)
    }

    #[tokio::test]
    async fn test_notification_registry_basic() {
        let registry = NotificationRegistry::new();
        let mut receiver = registry.subscribe("room-1").await;

        let notification = Notification {
            room_id: "room-1".into(),
            message_id: "msg-1".into(),
            sender_id: "agent-1".into(),
            priority: NotificationPriority::Normal,
            topics: vec!["general".into()],
            mentions: vec![],
        };

        registry.notify(notification.clone()).await;

        let received = receiver.recv().await.unwrap();
        assert_eq!(received.room_id, "room-1");
        assert_eq!(received.message_id, "msg-1");
        assert_eq!(received.sender_id, "agent-1");
    }

    #[tokio::test]
    async fn test_room_wait_receives_message() {
        let registry = Arc::new(NotificationRegistry::new());
        let registry_clone = registry.clone();

        let handle = tokio::spawn(async move {
            tokio::task::yield_now().await;
            registry_clone
                .notify(Notification {
                    room_id: "room-wait".into(),
                    message_id: "msg-wait".into(),
                    sender_id: "sender-1".into(),
                    priority: NotificationPriority::Normal,
                    topics: vec![],
                    mentions: vec![],
                })
                .await;
        });

        let result = room_wait(&registry, "room-wait", Some(5000), None)
            .await
            .unwrap();

        handle.await.unwrap();

        assert!(!result.timed_out);
        assert!(result.notification.is_some());
        assert_eq!(result.notification.unwrap().message_id, "msg-wait");
    }

    #[tokio::test]
    async fn test_room_wait_timeout() {
        let registry = NotificationRegistry::new();

        let result = room_wait(&registry, "empty-room", Some(50), None)
            .await
            .unwrap();

        assert!(result.timed_out);
        assert!(result.notification.is_none());
    }

    #[tokio::test]
    async fn test_topic_filtering() {
        let registry = Arc::new(NotificationRegistry::new());
        let registry_clone = registry.clone();

        let handle = tokio::spawn(async move {
            tokio::task::yield_now().await;
            registry_clone
                .notify(Notification {
                    room_id: "topic-room".into(),
                    message_id: "msg-unrelated".into(),
                    sender_id: "sender-1".into(),
                    priority: NotificationPriority::Normal,
                    topics: vec!["unrelated".into()],
                    mentions: vec![],
                })
                .await;

            tokio::task::yield_now().await;
            registry_clone
                .notify(Notification {
                    room_id: "topic-room".into(),
                    message_id: "msg-matching".into(),
                    sender_id: "sender-1".into(),
                    priority: NotificationPriority::Normal,
                    topics: vec!["deploy".into()],
                    mentions: vec![],
                })
                .await;
        });

        let topics = vec!["deploy".to_string()];
        let result = room_wait(&registry, "topic-room", Some(5000), Some(&topics))
            .await
            .unwrap();

        handle.await.unwrap();

        assert!(!result.timed_out);
        let notif = result.notification.unwrap();
        assert_eq!(notif.message_id, "msg-matching");
        assert!(notif.topics.contains(&"deploy".to_string()));
    }

    #[tokio::test]
    async fn test_subscribe_unsubscribe_db() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "sub-room", None, None).await.unwrap();

        subscribe_to_room(&db, &room.id, "agent-1", Some(vec!["topic-a".into()]))
            .await
            .unwrap();

        let subs = list_subscriptions(&db, "agent-1").await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].room_id, room.id);
        assert_eq!(subs[0].topics, Some(vec!["topic-a".to_string()]));

        unsubscribe_from_room(&db, &room.id, "agent-1")
            .await
            .unwrap();

        let subs = list_subscriptions(&db, "agent-1").await.unwrap();
        assert!(subs.is_empty());
    }

    #[tokio::test]
    async fn test_notify_after_post() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "notify-room", None, None).await.unwrap();
        let registry = NotificationRegistry::new();
        let mut receiver = registry.subscribe(&room.id).await;

        let metadata = serde_json::json!({"topics": ["deployment"], "mentions": ["agent-2"]});

        post_message(
            &db,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Deploy started".into(),
                reply_to: None,
                metadata: Some(metadata),
                message_type: None,
            },
            Some(&registry),
        )
        .await
        .unwrap();

        let notification = receiver.recv().await.unwrap();
        assert_eq!(notification.room_id, room.id);
        assert_eq!(notification.sender_id, "agent-1");
        assert_eq!(notification.topics, vec!["deployment"]);
        assert_eq!(notification.mentions, vec!["agent-2"]);
    }

    #[test]
    fn test_should_notify_agent_mention_always() {
        let notification = Notification {
            room_id: "r".into(),
            message_id: "m".into(),
            sender_id: "s".into(),
            priority: NotificationPriority::Normal,
            topics: vec!["deploy".into()],
            mentions: vec!["agent-x".into()],
        };
        let unrelated_topics = vec!["ci".into()];
        assert!(should_notify_agent(
            &notification,
            "agent-x",
            Some(&unrelated_topics)
        ));
    }

    #[test]
    fn test_should_notify_agent_topic_match() {
        let notification = Notification {
            room_id: "r".into(),
            message_id: "m".into(),
            sender_id: "s".into(),
            priority: NotificationPriority::Normal,
            topics: vec!["deploy".into(), "ci".into()],
            mentions: vec![],
        };
        let topics = vec!["ci".into()];
        assert!(should_notify_agent(&notification, "agent-y", Some(&topics)));
    }

    #[test]
    fn test_should_notify_agent_topic_mismatch() {
        let notification = Notification {
            room_id: "r".into(),
            message_id: "m".into(),
            sender_id: "s".into(),
            priority: NotificationPriority::Normal,
            topics: vec!["deploy".into()],
            mentions: vec![],
        };
        let topics = vec!["ci".into()];
        assert!(!should_notify_agent(
            &notification,
            "agent-z",
            Some(&topics)
        ));
    }

    #[tokio::test]
    async fn test_enqueue_dequeue_notification() {
        let (db, _tmp) = setup().await;

        let notification = Notification {
            room_id: "room-eq".into(),
            message_id: "msg-eq".into(),
            sender_id: "sender-eq".into(),
            priority: NotificationPriority::High,
            topics: vec!["build".into()],
            mentions: vec!["agent-eq".into()],
        };

        enqueue_notification(&db, "agent-eq", &notification)
            .await
            .unwrap();

        let dequeued = dequeue_notification(&db, "agent-eq", None, None)
            .await
            .unwrap();
        assert!(dequeued.is_some());
        let d = dequeued.unwrap();
        assert_eq!(d.room_id, "room-eq");
        assert_eq!(d.message_id, "msg-eq");
        assert_eq!(d.priority, NotificationPriority::High);
        assert_eq!(d.topics, vec!["build".to_string()]);
        assert_eq!(d.mentions, vec!["agent-eq".to_string()]);

        let again = dequeue_notification(&db, "agent-eq", None, None)
            .await
            .unwrap();
        assert!(again.is_none());
    }

    #[test]
    fn test_notification_priority_derivation() {
        assert_eq!(
            NotificationPriority::default(),
            NotificationPriority::Normal
        );

        let low = NotificationPriority::Low;
        let urgent = NotificationPriority::Urgent;
        assert_ne!(low, urgent);

        let json = serde_json::to_string(&NotificationPriority::High).unwrap();
        assert_eq!(json, "\"high\"");
        let parsed: NotificationPriority = serde_json::from_str("\"urgent\"").unwrap();
        assert_eq!(parsed, NotificationPriority::Urgent);
    }

    #[tokio::test]
    async fn test_room_wait_persistent_checks_queue_first() {
        let (db, _tmp) = setup().await;
        let registry = NotificationRegistry::new();

        let notification = Notification {
            room_id: "persist-room".into(),
            message_id: "persist-msg".into(),
            sender_id: "persist-sender".into(),
            priority: NotificationPriority::Normal,
            topics: vec![],
            mentions: vec![],
        };

        enqueue_notification(&db, "persist-agent", &notification)
            .await
            .unwrap();

        let result = room_wait_persistent(
            &db,
            &registry,
            "persist-room",
            "persist-agent",
            Some(50),
            None,
        )
        .await
        .unwrap();

        assert!(!result.timed_out);
        assert!(result.notification.is_some());
        assert_eq!(result.notification.unwrap().message_id, "persist-msg");
    }
}
