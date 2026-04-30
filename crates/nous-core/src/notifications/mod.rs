use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use tokio::sync::{broadcast, RwLock};

use crate::error::NousError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub room_id: String,
    pub message_id: String,
    pub sender_id: String,
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
    pool: &SqlitePool,
    room_id: &str,
    agent_id: &str,
    topics: Option<Vec<String>>,
) -> Result<(), NousError> {
    let room_exists: bool = sqlx::query("SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?)")
        .bind(room_id)
        .fetch_one(pool)
        .await?
        .get(0);

    if !room_exists {
        return Err(NousError::NotFound(format!("room '{room_id}' not found")));
    }

    let topics_json = topics.map(|t| serde_json::to_string(&t).unwrap_or_default());

    sqlx::query(
        "INSERT OR REPLACE INTO room_subscriptions (room_id, agent_id, topics) VALUES (?, ?, ?)",
    )
    .bind(room_id)
    .bind(agent_id)
    .bind(&topics_json)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn unsubscribe_from_room(
    pool: &SqlitePool,
    room_id: &str,
    agent_id: &str,
) -> Result<(), NousError> {
    sqlx::query("DELETE FROM room_subscriptions WHERE room_id = ? AND agent_id = ?")
        .bind(room_id)
        .bind(agent_id)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn list_subscriptions(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<Vec<Subscription>, NousError> {
    let rows = sqlx::query(
        "SELECT room_id, agent_id, topics, created_at FROM room_subscriptions WHERE agent_id = ?",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(|row: &SqliteRow| {
            let topics_str: Option<String> = row.try_get("topics")?;
            let topics = match topics_str.as_deref().map(serde_json::from_str) {
                Some(Ok(val)) => Some(val),
                Some(Err(_)) => None,
                None => None,
            };

            Ok(Subscription {
                room_id: row.try_get("room_id")?,
                agent_id: row.try_get("agent_id")?,
                topics,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(NousError::Sqlite)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::messages::{post_message, PostMessageRequest};
    use crate::rooms::create_room;
    use tempfile::TempDir;

    async fn setup() -> (SqlitePool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();
        let pool = pools.fts.clone();
        (pool, tmp)
    }

    #[tokio::test]
    async fn test_notification_registry_basic() {
        let registry = NotificationRegistry::new();
        let mut receiver = registry.subscribe("room-1").await;

        let notification = Notification {
            room_id: "room-1".into(),
            message_id: "msg-1".into(),
            sender_id: "agent-1".into(),
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
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            registry_clone
                .notify(Notification {
                    room_id: "room-wait".into(),
                    message_id: "msg-wait".into(),
                    sender_id: "sender-1".into(),
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
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            registry_clone
                .notify(Notification {
                    room_id: "topic-room".into(),
                    message_id: "msg-unrelated".into(),
                    sender_id: "sender-1".into(),
                    topics: vec!["unrelated".into()],
                    mentions: vec![],
                })
                .await;

            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            registry_clone
                .notify(Notification {
                    room_id: "topic-room".into(),
                    message_id: "msg-matching".into(),
                    sender_id: "sender-1".into(),
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
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "sub-room", None, None).await.unwrap();

        subscribe_to_room(&pool, &room.id, "agent-1", Some(vec!["topic-a".into()]))
            .await
            .unwrap();

        let subs = list_subscriptions(&pool, "agent-1").await.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].room_id, room.id);
        assert_eq!(subs[0].topics, Some(vec!["topic-a".to_string()]));

        unsubscribe_from_room(&pool, &room.id, "agent-1")
            .await
            .unwrap();

        let subs = list_subscriptions(&pool, "agent-1").await.unwrap();
        assert!(subs.is_empty());
    }

    #[tokio::test]
    async fn test_notify_after_post() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "notify-room", None, None).await.unwrap();
        let registry = NotificationRegistry::new();
        let mut receiver = registry.subscribe(&room.id).await;

        let metadata = serde_json::json!({"topics": ["deployment"], "mentions": ["agent-2"]});

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Deploy started".into(),
                reply_to: None,
                metadata: Some(metadata),
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
}
