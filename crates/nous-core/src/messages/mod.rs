use std::fmt;
use std::str::FromStr;

use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, Set, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::message_cursors as cursor_entity;
use crate::entities::room_messages as msg_entity;
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
    fn from_model(m: msg_entity::Model) -> Self {
        let metadata = m
            .metadata
            .as_deref()
            .and_then(|s| match serde_json::from_str(s) {
                Ok(val) => Some(val),
                Err(e) => {
                    tracing::warn!(error = %e, "malformed JSON in message metadata column, treating as null");
                    None
                }
            });

        let message_type = m
            .message_type
            .parse::<MessageType>()
            .unwrap_or(MessageType::User);

        Self {
            id: m.id,
            room_id: m.room_id,
            sender_id: m.sender_id,
            content: m.content,
            reply_to: m.reply_to,
            metadata,
            message_type,
            created_at: m.created_at,
        }
    }

    fn from_query_result(row: &sea_orm::QueryResult) -> Result<Self, sea_orm::DbErr> {
        let metadata_str: Option<String> = row.try_get_by("metadata")?;
        let metadata = match metadata_str.as_deref().map(serde_json::from_str) {
            Some(Ok(val)) => Some(val),
            Some(Err(e)) => {
                tracing::warn!(error = %e, "malformed JSON in message metadata column, treating as null");
                None
            }
            None => None,
        };

        let message_type_str: String = row.try_get_by("message_type")?;
        let message_type = message_type_str
            .parse::<MessageType>()
            .unwrap_or(MessageType::User);

        Ok(Self {
            id: row.try_get_by("id")?,
            room_id: row.try_get_by("room_id")?,
            sender_id: row.try_get_by("sender_id")?,
            content: row.try_get_by("content")?,
            reply_to: row.try_get_by("reply_to")?,
            metadata,
            message_type,
            created_at: row.try_get_by("created_at")?,
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
    db: &DatabaseConnection,
    request: PostMessageRequest,
    registry: Option<&NotificationRegistry>,
) -> Result<Message, NousError> {
    if request.content.trim().is_empty() {
        return Err(NousError::Validation(
            "message content cannot be empty".into(),
        ));
    }

    let room_exists = crate::entities::rooms::Entity::find_by_id(&request.room_id)
        .count(db)
        .await?
        > 0;

    if !room_exists {
        return Err(NousError::NotFound(format!(
            "room '{}' not found",
            request.room_id
        )));
    }

    let id = Uuid::now_v7().to_string();
    let metadata_json = request.metadata.as_ref().map(|m| m.to_string());
    let msg_type = request.message_type.unwrap_or_default().to_string();

    let model = msg_entity::ActiveModel {
        id: Set(id.clone()),
        room_id: Set(request.room_id.clone()),
        sender_id: Set(request.sender_id.clone()),
        content: Set(request.content.clone()),
        reply_to: Set(request.reply_to.clone()),
        metadata: Set(metadata_json),
        message_type: Set(msg_type),
        created_at: Set(String::new()),
    };

    msg_entity::Entity::insert(model).exec(db).await?;

    let row = msg_entity::Entity::find_by_id(&id).one(db).await?;

    let message = Message::from_model(
        row.ok_or_else(|| NousError::Internal("inserted message not found".into()))?,
    );

    if let Some(registry) = registry {
        let (topics, mentions) = extract_topics_mentions(&request.metadata);
        registry
            .notify(Notification {
                room_id: message.room_id.clone(),
                message_id: message.id.clone(),
                sender_id: message.sender_id.clone(),
                priority: crate::notifications::NotificationPriority::Normal,
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
    db: &DatabaseConnection,
    request: ReadMessagesRequest,
) -> Result<Vec<Message>, NousError> {
    let room_exists = crate::entities::rooms::Entity::find_by_id(&request.room_id)
        .count(db)
        .await?
        > 0;

    if !room_exists {
        return Err(NousError::NotFound(format!(
            "room '{}' not found",
            request.room_id
        )));
    }

    let limit = request.limit.unwrap_or(50).min(200);
    let mut sql = String::from("SELECT * FROM room_messages WHERE room_id = ?");
    let mut binds: Vec<sea_orm::Value> = vec![request.room_id.clone().into()];

    if let Some(ref since) = request.since {
        sql.push_str(" AND created_at > ?");
        binds.push(since.clone().into());
    }

    if let Some(ref before) = request.before {
        sql.push_str(" AND created_at < ?");
        binds.push(before.clone().into());
    }

    sql.push_str(" ORDER BY created_at ASC LIMIT ?");
    binds.push((limit as i64).into());

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &sql, binds);
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn search_messages(
    db: &DatabaseConnection,
    request: SearchMessagesRequest,
) -> Result<Vec<Message>, NousError> {
    if request.query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = request.limit.unwrap_or(20).min(100);

    let (sql, binds): (&str, Vec<sea_orm::Value>) = if let Some(ref room_id) = request.room_id {
        (
            "SELECT rm.* FROM room_messages rm \
             JOIN room_messages_fts fts ON rm.rowid = fts.rowid \
             WHERE room_messages_fts MATCH ? AND rm.room_id = ? \
             ORDER BY fts.rank \
             LIMIT ?",
            vec![
                request.query.clone().into(),
                room_id.clone().into(),
                (limit as i64).into(),
            ],
        )
    } else {
        (
            "SELECT rm.* FROM room_messages rm \
             JOIN room_messages_fts fts ON rm.rowid = fts.rowid \
             WHERE room_messages_fts MATCH ? \
             ORDER BY fts.rank \
             LIMIT ?",
            vec![request.query.clone().into(), (limit as i64).into()],
        )
    };

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, sql, binds);
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn list_mentions(
    db: &DatabaseConnection,
    room_id: &str,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Message>, NousError> {
    let limit = limit.unwrap_or(50).min(200);
    let pattern = format!("%@{}%", agent_id);

    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "SELECT * FROM room_messages WHERE room_id = ? AND content LIKE ? ORDER BY created_at DESC LIMIT ?",
        [room_id.into(), pattern.into(), (limit as i64).into()],
    );
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
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

pub async fn get_thread(
    db: &DatabaseConnection,
    req: GetThreadRequest,
) -> Result<ThreadView, NousError> {
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "SELECT * FROM room_messages WHERE id = ? OR reply_to = ? ORDER BY created_at ASC",
        [req.root_message_id.clone().into(), req.root_message_id.clone().into()],
    );
    let rows = db.query_all(stmt).await?;

    let messages: Vec<Message> = rows
        .iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)?;

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

pub async fn mark_read(
    db: &DatabaseConnection,
    req: MarkReadRequest,
) -> Result<ChatCursor, NousError> {
    let msg_model = msg_entity::Entity::find_by_id(&req.message_id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("message '{}' not found", req.message_id)))?;

    let msg_created_at = msg_model.created_at;

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "INSERT INTO message_cursors (room_id, agent_id, last_read_message_id, last_read_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT (room_id, agent_id) DO UPDATE SET \
         last_read_message_id = excluded.last_read_message_id, \
         last_read_at = excluded.last_read_at",
        [
            req.room_id.clone().into(),
            req.agent_id.clone().into(),
            req.message_id.clone().into(),
            msg_created_at.clone().into(),
        ],
    ))
    .await?;

    let unread_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ? AND created_at > ?",
            [req.room_id.clone().into(), msg_created_at.clone().into()],
        ))
        .await?;

    let unread: i64 = unread_row
        .map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
        .unwrap_or(0);

    Ok(ChatCursor {
        room_id: req.room_id,
        agent_id: req.agent_id,
        last_read_message_id: req.message_id,
        last_read_at: msg_created_at,
        unread_count: unread,
    })
}

pub async fn unread_count(
    db: &DatabaseConnection,
    req: UnreadCountRequest,
) -> Result<ChatCursor, NousError> {
    let cursor_model = cursor_entity::Entity::find_by_id((req.room_id.clone(), req.agent_id.clone()))
        .one(db)
        .await?;

    let (last_read_message_id, last_read_at) = match cursor_model {
        Some(m) => (m.last_read_message_id, m.last_read_at),
        None => {
            let count_row = db
                .query_one(Statement::from_sql_and_values(
                    sea_orm::DbBackend::Sqlite,
                    "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ?",
                    [req.room_id.clone().into()],
                ))
                .await?;

            let count: i64 = count_row
                .map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
                .unwrap_or(0);

            return Ok(ChatCursor {
                room_id: req.room_id,
                agent_id: req.agent_id,
                last_read_message_id: String::new(),
                last_read_at: String::new(),
                unread_count: count,
            });
        }
    };

    let unread_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ? AND created_at > ?",
            [req.room_id.clone().into(), last_read_at.clone().into()],
        ))
        .await?;

    let unread: i64 = unread_row
        .map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
        .unwrap_or(0);

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

    async fn setup() -> (DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();
        let db = pools.fts.clone();
        (db, tmp)
    }

    #[tokio::test]
    async fn test_post_message() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "test-room", None, None).await.unwrap();

        let msg = post_message(
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "chrono-room", None, None).await.unwrap();

        for i in 1..=3 {
            post_message(
                &db,
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

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "Message 1");
        assert_eq!(messages[1].content, "Message 2");
        assert_eq!(messages[2].content, "Message 3");
    }

    #[tokio::test]
    async fn test_read_messages_with_since_filter() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "since-room", None, None).await.unwrap();

        let msg1 = post_message(
            &db,
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
            &db,
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
            &db,
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
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "limit-room", None, None).await.unwrap();

        for i in 1..=5 {
            post_message(
                &db,
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
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "search-room", None, None).await.unwrap();

        post_message(
            &db,
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
            &db,
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
            &db,
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
            &db,
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
        let (db, _tmp) = setup().await;

        let result = post_message(
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "empty-room", None, None).await.unwrap();

        let result = post_message(
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "type-default-room", None, None)
            .await
            .unwrap();

        let msg = post_message(
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "system-msg-room", None, None)
            .await
            .unwrap();

        let msg = post_message(
            &db,
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

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, MessageType::System);
    }

    #[tokio::test]
    async fn test_get_thread_returns_root_and_replies() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "thread-room", None, None).await.unwrap();

        let root = post_message(
            &db,
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
            &db,
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
            &db,
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
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "thread-empty-room", None, None)
            .await
            .unwrap();

        let root = post_message(
            &db,
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
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "cursor-room", None, None).await.unwrap();

        let mut msgs = Vec::new();
        for i in 1..=5 {
            let msg = post_message(
                &db,
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
            &db,
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
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "unread-room", None, None).await.unwrap();

        let mut msgs = Vec::new();
        for i in 1..=3 {
            let msg = post_message(
                &db,
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
            &db,
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
                &db,
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
            &db,
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
