use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;
use crate::notifications::{Notification, NotificationRegistry};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    #[default]
    User,
    System,
    TaskEvent,
    Command,
    Handoff,
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::System => write!(f, "system"),
            Self::TaskEvent => write!(f, "task_event"),
            Self::Command => write!(f, "command"),
            Self::Handoff => write!(f, "handoff"),
        }
    }
}

impl FromStr for MessageType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "system" => Ok(Self::System),
            "task_event" => Ok(Self::TaskEvent),
            "command" => Ok(Self::Command),
            "handoff" => Ok(Self::Handoff),
            other => Err(format!("unknown message type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub room_id: String,
    pub sender_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub message_type: MessageType,
    pub created_at: String,
}

impl Message {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        let metadata_str: Option<String> = row.try_get("metadata")?;
        let metadata = match metadata_str.as_deref().map(serde_json::from_str) {
            Some(Ok(val)) => Some(val),
            Some(Err(e)) => {
                tracing::warn!(error = %e, "malformed JSON in message metadata column, treating as null");
                None
            }
            None => None,
        };

        let message_type_str: String = row.try_get("message_type")?;
        let message_type = message_type_str
            .parse::<MessageType>()
            .unwrap_or(MessageType::User);

        Ok(Self {
            id: row.try_get("id")?,
            room_id: row.try_get("room_id")?,
            sender_id: row.try_get("sender_id")?,
            content: row.try_get("content")?,
            reply_to: row.try_get("reply_to")?,
            metadata,
            message_type,
            created_at: row.try_get("created_at")?,
        })
    }
}

#[derive(Debug)]
pub struct PostMessageRequest {
    pub room_id: String,
    pub sender_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub message_type: Option<MessageType>,
}

#[derive(Debug)]
pub struct ReadMessagesRequest {
    pub room_id: String,
    pub since: Option<String>,
    pub before: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug)]
pub struct SearchMessagesRequest {
    pub query: String,
    pub room_id: Option<String>,
    pub limit: Option<u32>,
}

pub async fn post_message(
    pool: &SqlitePool,
    request: PostMessageRequest,
    registry: Option<&NotificationRegistry>,
) -> Result<Message, NousError> {
    if request.content.trim().is_empty() {
        return Err(NousError::Validation(
            "message content cannot be empty".into(),
        ));
    }

    let room_exists: bool = sqlx::query("SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?)")
        .bind(&request.room_id)
        .fetch_one(pool)
        .await?
        .get(0);

    if !room_exists {
        return Err(NousError::NotFound(format!(
            "room '{}' not found",
            request.room_id
        )));
    }

    let id = Uuid::now_v7().to_string();
    let metadata_json = request.metadata.as_ref().map(|m| m.to_string());
    let msg_type = request.message_type.unwrap_or_default().to_string();

    sqlx::query(
        "INSERT INTO room_messages (id, room_id, sender_id, content, reply_to, metadata, message_type) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&request.room_id)
    .bind(&request.sender_id)
    .bind(&request.content)
    .bind(&request.reply_to)
    .bind(&metadata_json)
    .bind(&msg_type)
    .execute(pool)
    .await?;

    let row = sqlx::query("SELECT * FROM room_messages WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;

    let message = Message::from_row(&row).map_err(NousError::Sqlite)?;

    if let Some(registry) = registry {
        let (topics, mentions) = extract_topics_mentions(&request.metadata);
        registry
            .notify(Notification {
                room_id: message.room_id.clone(),
                message_id: message.id.clone(),
                sender_id: message.sender_id.clone(),
                topics,
                mentions,
            })
            .await;
    }

    Ok(message)
}

fn extract_topics_mentions(metadata: &Option<serde_json::Value>) -> (Vec<String>, Vec<String>) {
    let Some(meta) = metadata else {
        return (vec![], vec![]);
    };

    let topics = meta
        .get("topics")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mentions = meta
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    (topics, mentions)
}

pub async fn read_messages(
    pool: &SqlitePool,
    request: ReadMessagesRequest,
) -> Result<Vec<Message>, NousError> {
    let room_exists: bool = sqlx::query("SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?)")
        .bind(&request.room_id)
        .fetch_one(pool)
        .await?
        .get(0);

    if !room_exists {
        return Err(NousError::NotFound(format!(
            "room '{}' not found",
            request.room_id
        )));
    }

    let limit = request.limit.unwrap_or(50).min(200);
    let mut sql = String::from("SELECT * FROM room_messages WHERE room_id = ?");
    let mut binds: Vec<String> = vec![request.room_id.clone()];

    if let Some(ref since) = request.since {
        sql.push_str(" AND created_at > ?");
        binds.push(since.clone());
    }

    if let Some(ref before) = request.before {
        sql.push_str(" AND created_at < ?");
        binds.push(before.clone());
    }

    sql.push_str(" ORDER BY created_at ASC LIMIT ?");
    binds.push(limit.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    rows.iter()
        .map(Message::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn search_messages(
    pool: &SqlitePool,
    request: SearchMessagesRequest,
) -> Result<Vec<Message>, NousError> {
    if request.query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = request.limit.unwrap_or(20).min(100);

    let (sql, has_room_filter) = if request.room_id.is_some() {
        (
            "SELECT rm.* FROM room_messages rm \
             JOIN room_messages_fts fts ON rm.rowid = fts.rowid \
             WHERE room_messages_fts MATCH ? AND rm.room_id = ? \
             ORDER BY fts.rank \
             LIMIT ?",
            true,
        )
    } else {
        (
            "SELECT rm.* FROM room_messages rm \
             JOIN room_messages_fts fts ON rm.rowid = fts.rowid \
             WHERE room_messages_fts MATCH ? \
             ORDER BY fts.rank \
             LIMIT ?",
            false,
        )
    };

    let rows = if has_room_filter {
        sqlx::query(sql)
            .bind(&request.query)
            .bind(request.room_id.as_ref().unwrap())
            .bind(limit)
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query(sql)
            .bind(&request.query)
            .bind(limit)
            .fetch_all(pool)
            .await?
    };

    rows.iter()
        .map(Message::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn list_mentions(
    pool: &SqlitePool,
    room_id: &str,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Message>, NousError> {
    let limit = limit.unwrap_or(50).min(200);
    let pattern = format!("%@{}%", agent_id);

    let rows = sqlx::query(
        "SELECT * FROM room_messages WHERE room_id = ? AND content LIKE ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(room_id)
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(Message::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadView {
    pub root: Message,
    pub replies: Vec<Message>,
    pub reply_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCursor {
    pub room_id: String,
    pub agent_id: String,
    pub last_read_message_id: String,
    pub last_read_at: String,
    pub unread_count: i64,
}

#[derive(Debug)]
pub struct GetThreadRequest {
    pub room_id: String,
    pub root_message_id: String,
}

#[derive(Debug)]
pub struct MarkReadRequest {
    pub room_id: String,
    pub agent_id: String,
    pub message_id: String,
}

#[derive(Debug)]
pub struct UnreadCountRequest {
    pub room_id: String,
    pub agent_id: String,
}

pub async fn get_thread(pool: &SqlitePool, req: GetThreadRequest) -> Result<ThreadView, NousError> {
    let rows = sqlx::query(
        "SELECT * FROM room_messages WHERE id = ? OR reply_to = ? ORDER BY created_at ASC",
    )
    .bind(&req.root_message_id)
    .bind(&req.root_message_id)
    .fetch_all(pool)
    .await?;

    let messages: Vec<Message> = rows
        .iter()
        .map(Message::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)?;

    let root = messages
        .iter()
        .find(|m| m.id == req.root_message_id)
        .cloned()
        .ok_or_else(|| {
            NousError::NotFound(format!("message '{}' not found", req.root_message_id))
        })?;

    let replies: Vec<Message> = messages
        .into_iter()
        .filter(|m| m.id != req.root_message_id)
        .collect();
    let reply_count = replies.len();

    Ok(ThreadView {
        root,
        replies,
        reply_count,
    })
}

pub async fn mark_read(pool: &SqlitePool, req: MarkReadRequest) -> Result<ChatCursor, NousError> {
    let msg_row = sqlx::query("SELECT created_at FROM room_messages WHERE id = ?")
        .bind(&req.message_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("message '{}' not found", req.message_id)))?;

    let msg_created_at: String = msg_row.try_get("created_at").map_err(NousError::Sqlite)?;

    sqlx::query(
        "INSERT INTO message_cursors (room_id, agent_id, last_read_message_id, last_read_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT (room_id, agent_id) DO UPDATE SET \
         last_read_message_id = excluded.last_read_message_id, \
         last_read_at = excluded.last_read_at",
    )
    .bind(&req.room_id)
    .bind(&req.agent_id)
    .bind(&req.message_id)
    .bind(&msg_created_at)
    .execute(pool)
    .await?;

    let unread: i64 = sqlx::query(
        "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ? AND created_at > ?",
    )
    .bind(&req.room_id)
    .bind(&msg_created_at)
    .fetch_one(pool)
    .await?
    .try_get("cnt")
    .map_err(NousError::Sqlite)?;

    Ok(ChatCursor {
        room_id: req.room_id,
        agent_id: req.agent_id,
        last_read_message_id: req.message_id,
        last_read_at: msg_created_at,
        unread_count: unread,
    })
}

pub async fn unread_count(
    pool: &SqlitePool,
    req: UnreadCountRequest,
) -> Result<ChatCursor, NousError> {
    let cursor_row = sqlx::query(
        "SELECT last_read_message_id, last_read_at FROM message_cursors WHERE room_id = ? AND agent_id = ?",
    )
    .bind(&req.room_id)
    .bind(&req.agent_id)
    .fetch_optional(pool)
    .await?;

    let (last_read_message_id, last_read_at) = match cursor_row {
        Some(row) => {
            let mid: String = row
                .try_get("last_read_message_id")
                .map_err(NousError::Sqlite)?;
            let lat: String = row.try_get("last_read_at").map_err(NousError::Sqlite)?;
            (mid, lat)
        }
        None => {
            let count: i64 =
                sqlx::query("SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ?")
                    .bind(&req.room_id)
                    .fetch_one(pool)
                    .await?
                    .try_get("cnt")
                    .map_err(NousError::Sqlite)?;
            return Ok(ChatCursor {
                room_id: req.room_id,
                agent_id: req.agent_id,
                last_read_message_id: String::new(),
                last_read_at: String::new(),
                unread_count: count,
            });
        }
    };

    let unread: i64 = sqlx::query(
        "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ? AND created_at > ?",
    )
    .bind(&req.room_id)
    .bind(&last_read_at)
    .fetch_one(pool)
    .await?
    .try_get("cnt")
    .map_err(NousError::Sqlite)?;

    Ok(ChatCursor {
        room_id: req.room_id,
        agent_id: req.agent_id,
        last_read_message_id,
        last_read_at,
        unread_count: unread,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
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
    async fn test_post_message() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "test-room", None, None).await.unwrap();

        let msg = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Hello, world!".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(msg.room_id, room.id);
        assert_eq!(msg.sender_id, "agent-1");
        assert_eq!(msg.content, "Hello, world!");
        assert!(!msg.id.is_empty());
        assert!(!msg.created_at.is_empty());
    }

    #[tokio::test]
    async fn test_read_messages_chronological() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "chrono-room", None, None).await.unwrap();

        for i in 1..=3 {
            post_message(
                &pool,
                PostMessageRequest {
                    room_id: room.id.clone(),
                    sender_id: "agent-1".into(),
                    content: format!("Message {i}"),
                    reply_to: None,
                    metadata: None,
                    message_type: None,
                },
                None,
            )
            .await
            .unwrap();
        }

        let messages = read_messages(
            &pool,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: None,
                before: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "Message 1");
        assert_eq!(messages[1].content, "Message 2");
        assert_eq!(messages[2].content, "Message 3");
    }

    #[tokio::test]
    async fn test_read_messages_with_since_filter() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "since-room", None, None).await.unwrap();

        let msg1 = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "First".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Second".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Third".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        let messages = read_messages(
            &pool,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: Some(msg1.created_at.clone()),
                before: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Second");
        assert_eq!(messages[1].content, "Third");
    }

    #[tokio::test]
    async fn test_read_messages_with_limit() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "limit-room", None, None).await.unwrap();

        for i in 1..=5 {
            post_message(
                &pool,
                PostMessageRequest {
                    room_id: room.id.clone(),
                    sender_id: "agent-1".into(),
                    content: format!("Message {i}"),
                    reply_to: None,
                    metadata: None,
                    message_type: None,
                },
                None,
            )
            .await
            .unwrap();
        }

        let messages = read_messages(
            &pool,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: None,
                before: None,
                limit: Some(2),
            },
        )
        .await
        .unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Message 1");
        assert_eq!(messages[1].content, "Message 2");
    }

    #[tokio::test]
    async fn test_search_messages_fts() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "search-room", None, None).await.unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "The quick brown fox jumps over the lazy dog".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "A fast red car drives down the road".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "The fox went home after a long day".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        let results = search_messages(
            &pool,
            SearchMessagesRequest {
                query: "fox".into(),
                room_id: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|m| m.content.contains("fox")));
    }

    #[tokio::test]
    async fn test_post_message_to_nonexistent_room() {
        let (pool, _tmp) = setup().await;

        let result = post_message(
            &pool,
            PostMessageRequest {
                room_id: "nonexistent-room-id".into(),
                sender_id: "agent-1".into(),
                content: "Hello".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await;

        assert!(matches!(result, Err(NousError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_post_empty_content() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "empty-room", None, None).await.unwrap();

        let result = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "   ".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await;

        assert!(matches!(result, Err(NousError::Validation(_))));
    }

    #[tokio::test]
    async fn test_message_type_defaults_to_user() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "type-default-room", None, None)
            .await
            .unwrap();

        let msg = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "No explicit type".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(msg.message_type, MessageType::User);
    }

    #[tokio::test]
    async fn test_post_system_message() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "system-msg-room", None, None)
            .await
            .unwrap();

        let msg = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "system".into(),
                content: "Room created".into(),
                reply_to: None,
                metadata: None,
                message_type: Some(MessageType::System),
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(msg.message_type, MessageType::System);

        let messages = read_messages(
            &pool,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: None,
                before: None,
                limit: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, MessageType::System);
    }

    #[tokio::test]
    async fn test_get_thread_returns_root_and_replies() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "thread-room", None, None).await.unwrap();

        let root = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Root message".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-2".into(),
                content: "Reply 1".into(),
                reply_to: Some(root.id.clone()),
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-3".into(),
                content: "Reply 2".into(),
                reply_to: Some(root.id.clone()),
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        let thread = get_thread(
            &pool,
            GetThreadRequest {
                room_id: room.id.clone(),
                root_message_id: root.id.clone(),
            },
        )
        .await
        .unwrap();

        assert_eq!(thread.root.id, root.id);
        assert_eq!(thread.root.content, "Root message");
        assert_eq!(thread.reply_count, 2);
        assert_eq!(thread.replies[0].content, "Reply 1");
        assert_eq!(thread.replies[1].content, "Reply 2");
    }

    #[tokio::test]
    async fn test_get_thread_empty_replies() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "thread-empty-room", None, None)
            .await
            .unwrap();

        let root = post_message(
            &pool,
            PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Standalone message".into(),
                reply_to: None,
                metadata: None,
                message_type: None,
            },
            None,
        )
        .await
        .unwrap();

        let thread = get_thread(
            &pool,
            GetThreadRequest {
                room_id: room.id.clone(),
                root_message_id: root.id.clone(),
            },
        )
        .await
        .unwrap();

        assert_eq!(thread.root.id, root.id);
        assert_eq!(thread.reply_count, 0);
        assert!(thread.replies.is_empty());
    }

    #[tokio::test]
    async fn test_mark_read_advances_cursor() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "cursor-room", None, None).await.unwrap();

        let mut msgs = Vec::new();
        for i in 1..=5 {
            let msg = post_message(
                &pool,
                PostMessageRequest {
                    room_id: room.id.clone(),
                    sender_id: "agent-1".into(),
                    content: format!("Message {i}"),
                    reply_to: None,
                    metadata: None,
                    message_type: None,
                },
                None,
            )
            .await
            .unwrap();
            msgs.push(msg);
        }

        let cursor = mark_read(
            &pool,
            MarkReadRequest {
                room_id: room.id.clone(),
                agent_id: "reader-agent".into(),
                message_id: msgs[2].id.clone(),
            },
        )
        .await
        .unwrap();

        assert_eq!(cursor.room_id, room.id);
        assert_eq!(cursor.agent_id, "reader-agent");
        assert_eq!(cursor.last_read_message_id, msgs[2].id);
        assert_eq!(cursor.unread_count, 2);
    }

    #[tokio::test]
    async fn test_unread_count_after_new_messages() {
        let (pool, _tmp) = setup().await;
        let room = create_room(&pool, "unread-room", None, None).await.unwrap();

        let mut msgs = Vec::new();
        for i in 1..=3 {
            let msg = post_message(
                &pool,
                PostMessageRequest {
                    room_id: room.id.clone(),
                    sender_id: "agent-1".into(),
                    content: format!("Message {i}"),
                    reply_to: None,
                    metadata: None,
                    message_type: None,
                },
                None,
            )
            .await
            .unwrap();
            msgs.push(msg);
        }

        mark_read(
            &pool,
            MarkReadRequest {
                room_id: room.id.clone(),
                agent_id: "reader-agent".into(),
                message_id: msgs[2].id.clone(),
            },
        )
        .await
        .unwrap();

        for i in 4..=6 {
            post_message(
                &pool,
                PostMessageRequest {
                    room_id: room.id.clone(),
                    sender_id: "agent-1".into(),
                    content: format!("Message {i}"),
                    reply_to: None,
                    metadata: None,
                    message_type: None,
                },
                None,
            )
            .await
            .unwrap();
        }

        let cursor = unread_count(
            &pool,
            UnreadCountRequest {
                room_id: room.id.clone(),
                agent_id: "reader-agent".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(cursor.unread_count, 3);
        assert_eq!(cursor.last_read_message_id, msgs[2].id);
    }
}
