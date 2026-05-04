use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::entities::room_messages as msg_entity;

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
            other => Err(format!("unknown message type: '{other}'. Valid values: user, system, task_event, command, handoff")),
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
    pub(super) fn from_model(m: msg_entity::Model) -> Self {
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

    pub(super) fn from_query_result(row: &sea_orm::QueryResult) -> Result<Self, sea_orm::DbErr> {
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
