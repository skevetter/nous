use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::error::NousError;
use crate::messages::{self, Message, MessageType, PostMessageRequest};
use crate::notifications::NotificationRegistry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub message_kind: AgentMessageKind,
    pub correlation_id: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    Handoff,
    StatusUpdate,
    Question,
    Answer,
    Completion,
    Escalation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffPayload {
    pub task_id: Option<String>,
    pub branch: Option<String>,
    pub scope: Option<String>,
    pub acceptance_criteria: Vec<String>,
    pub context: serde_json::Value,
    pub deadline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceEvent {
    pub agent_id: String,
    pub agent_name: String,
    pub status: String,
    pub current_task_id: Option<String>,
    pub room_id: String,
}

pub async fn post_handoff(
    db: &DatabaseConnection,
    registry: Option<&NotificationRegistry>,
    room_id: &str,
    from_agent: &str,
    to_agent: &str,
    payload: HandoffPayload,
) -> Result<Message, NousError> {
    let metadata = serde_json::json!({
        "message_type": "handoff",
        "mentions": [to_agent],
        "topics": ["handoff"],
        "handoff": {
            "from_agent": from_agent,
            "to_agent": to_agent,
            "task_id": payload.task_id,
            "branch": payload.branch,
            "scope": payload.scope,
            "acceptance_criteria": payload.acceptance_criteria,
            "context": payload.context,
            "deadline": payload.deadline,
        }
    });

    let content = format!(
        "Handoff to @{to_agent}: {}",
        payload
            .acceptance_criteria
            .first()
            .unwrap_or(&"See metadata".to_string())
    );

    messages::post_message(
        db,
        PostMessageRequest {
            room_id: room_id.to_string(),
            sender_id: from_agent.to_string(),
            content,
            reply_to: None,
            metadata: Some(metadata),
            message_type: Some(MessageType::Handoff),
        },
        registry,
    )
    .await
}

pub async fn broadcast_presence(
    db: &DatabaseConnection,
    registry: Option<&NotificationRegistry>,
    event: &PresenceEvent,
) -> Result<(), NousError> {
    let metadata = serde_json::json!({
        "message_type": "system",
        "topics": ["presence"],
        "presence": {
            "agent_id": event.agent_id,
            "agent_name": event.agent_name,
            "status": event.status,
            "current_task_id": event.current_task_id,
        }
    });

    let content = format!("Agent {} is now {}", event.agent_name, event.status);

    messages::post_message(
        db,
        PostMessageRequest {
            room_id: event.room_id.clone(),
            sender_id: event.agent_id.clone(),
            content,
            reply_to: None,
            metadata: Some(metadata),
            message_type: Some(MessageType::System),
        },
        registry,
    )
    .await?;

    Ok(())
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
        pools.run_migrations().await.unwrap();
        let db = pools.fts.clone();
        (db, tmp)
    }

    #[tokio::test]
    async fn test_post_handoff_creates_message_with_metadata() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "handoff-room", None, None).await.unwrap();

        let payload = HandoffPayload {
            task_id: Some("TASK-001".into()),
            branch: Some("feat/handoff".into()),
            scope: Some("coordination module".into()),
            acceptance_criteria: vec!["Implement post_handoff".into()],
            context: serde_json::json!({"priority": "high"}),
            deadline: None,
        };

        let msg = post_handoff(&db, None, &room.id, "agent-a", "agent-b", payload)
            .await
            .unwrap();

        assert_eq!(msg.room_id, room.id);
        assert_eq!(msg.sender_id, "agent-a");
        assert_eq!(msg.message_type, MessageType::Handoff);
        assert!(msg.content.contains("@agent-b"));

        let meta = msg.metadata.unwrap();
        assert_eq!(meta["handoff"]["from_agent"], "agent-a");
        assert_eq!(meta["handoff"]["to_agent"], "agent-b");
        assert_eq!(meta["handoff"]["task_id"], "TASK-001");
    }

    #[tokio::test]
    async fn test_broadcast_presence_posts_to_room() {
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "presence-room", None, None).await.unwrap();

        let event = PresenceEvent {
            agent_id: "agent-x".into(),
            agent_name: "builder".into(),
            status: "active".into(),
            current_task_id: Some("TASK-002".into()),
            room_id: room.id.clone(),
        };

        broadcast_presence(&db, None, &event).await.unwrap();

        let msgs = messages::read_messages(
            &db,
            messages::ReadMessagesRequest {
                room_id: room.id.clone(),
                since: None,
                before: None,
                limit: Some(10),
            },
        )
        .await
        .unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].message_type, MessageType::System);
        assert!(msgs[0].content.contains("builder"));
        assert!(msgs[0].content.contains("active"));

        let meta = msgs[0].metadata.as_ref().unwrap();
        assert_eq!(meta["presence"]["agent_id"], "agent-x");
        assert_eq!(meta["presence"]["status"], "active");
    }

    #[tokio::test]
    async fn test_handoff_payload_serialization() {
        let payload = HandoffPayload {
            task_id: Some("TASK-003".into()),
            branch: Some("feat/serial".into()),
            scope: Some("tests".into()),
            acceptance_criteria: vec!["Round-trip serialization".into(), "All fields match".into()],
            context: serde_json::json!({"key": "value"}),
            deadline: Some("2026-05-10".into()),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: HandoffPayload = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.task_id, Some("TASK-003".into()));
        assert_eq!(deserialized.branch, Some("feat/serial".into()));
        assert_eq!(deserialized.scope, Some("tests".into()));
        assert_eq!(deserialized.acceptance_criteria.len(), 2);
        assert_eq!(
            deserialized.acceptance_criteria[0],
            "Round-trip serialization"
        );
        assert_eq!(deserialized.context["key"], "value");
        assert_eq!(deserialized.deadline, Some("2026-05-10".into()));
    }
}
