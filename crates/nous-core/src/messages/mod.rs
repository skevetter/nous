mod search;
mod store;
mod types;

pub use search::search_messages;
pub use store::{get_thread, list_mentions, mark_read, post_message, read_messages, unread_count};
pub use types::{
    ChatCursor, GetThreadRequest, MarkReadRequest, Message, MessageType, PostMessageRequest,
    ReadMessagesRequest, SearchMessagesRequest, ThreadView, UnreadCountRequest,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::error::NousError;
    use crate::rooms::create_room;
    use tempfile::TempDir;

    async fn setup() -> (sea_orm::DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        let db = pools.fts.clone();
        for agent_id in ["agent-1", "agent-2", "agent-3", "reader-agent"] {
            sea_orm::ConnectionTrait::execute_unprepared(
                &db,
                &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
            ).await.unwrap();
        }
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
        use sea_orm::ConnectionTrait;
        use uuid::Uuid;
        let (db, _tmp) = setup().await;
        let room = create_room(&db, "since-room", None, None).await.unwrap();

        // Insert with explicit timestamps spaced 1 second apart so the
        // since filter is deterministic regardless of wall-clock speed.
        let t1 = "2025-01-01T10:00:01.000000Z";
        let t2 = "2025-01-01T10:00:02.000000Z";
        let t3 = "2025-01-01T10:00:03.000000Z";
        for (content, ts) in [("First", t1), ("Second", t2), ("Third", t3)] {
            let id = Uuid::now_v7().to_string();
            db.execute_unprepared(&format!(
                "INSERT INTO room_messages (id, room_id, sender_id, content, message_type, created_at) \
                 VALUES ('{id}', '{}', 'agent-1', '{content}', 'user', '{ts}')",
                room.id
            ))
            .await
            .unwrap();
        }

        let messages = read_messages(
            &db,
            ReadMessagesRequest {
                room_id: room.id.clone(),
                since: Some(t1.to_string()),
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
