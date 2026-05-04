mod common;

use nous_core::agents::{self, ListAgentsFilter, RegisterAgentRequest};
use nous_core::db::DbPools;
use nous_core::messages::{self, PostMessageRequest, SearchMessagesRequest};
use nous_core::rooms;

async fn setup() -> (DbPools, tempfile::TempDir) {
    common::setup_test_db().await
}

// --- Room name tests ---

#[tokio::test]
async fn room_name_with_emoji() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "🚀 launch-pad 🎯", None, None)
        .await
        .unwrap();
    assert_eq!(room.name, "🚀 launch-pad 🎯");

    let fetched = rooms::get_room(pool, &room.name).await.unwrap();
    assert_eq!(fetched.id, room.id);

    pools.close().await;
}

#[tokio::test]
async fn room_name_with_cjk_characters() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "会議室-東京オフィス", None, None)
        .await
        .unwrap();
    assert_eq!(room.name, "会議室-東京オフィス");

    let fetched = rooms::get_room(pool, &room.name).await.unwrap();
    assert_eq!(fetched.id, room.id);

    pools.close().await;
}

#[tokio::test]
async fn room_name_with_rtl_arabic() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "غرفة-الاجتماعات", None, None)
        .await
        .unwrap();
    assert_eq!(room.name, "غرفة-الاجتماعات");

    pools.close().await;
}

#[tokio::test]
async fn room_name_with_zero_width_characters() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let name_with_zwj = "test\u{200D}room";
    let room = rooms::create_room(pool, name_with_zwj, None, None)
        .await
        .unwrap();
    assert_eq!(room.name, name_with_zwj);

    let name_zwnbsp = "room\u{FEFF}name";
    let room2 = rooms::create_room(pool, name_zwnbsp, None, None)
        .await
        .unwrap();
    assert_eq!(room2.name, name_zwnbsp);

    pools.close().await;
}

#[tokio::test]
async fn room_name_max_length_stress() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let long_name = "x".repeat(10_000);
    let room = rooms::create_room(pool, &long_name, None, None)
        .await
        .unwrap();
    assert_eq!(room.name.len(), 10_000);

    let fetched = rooms::get_room(pool, &room.id).await.unwrap();
    assert_eq!(fetched.name.len(), 10_000);

    pools.close().await;
}

#[tokio::test]
async fn room_name_with_newlines_and_control_chars() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let name = "room\n\twith\r\ncontrol\x01chars";
    let room = rooms::create_room(pool, name, None, None).await.unwrap();
    assert_eq!(room.name, name);

    pools.close().await;
}

// --- Agent name tests ---

#[tokio::test]
async fn agent_name_with_emoji_and_special_chars() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "🤖 agent/special@chars!".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("test".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(agent.name, "🤖 agent/special@chars!");

    let found = agents::lookup_agent(pool, "🤖 agent/special@chars!", Some("test"))
        .await
        .unwrap();
    assert_eq!(found.id, agent.id);

    pools.close().await;
}

#[tokio::test]
async fn agent_name_unicode_normalization_distinction() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let nfc_name = "caf\u{00E9}";
    let nfd_name = "cafe\u{0301}";

    let agent1 = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: nfc_name.into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("norm-test".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let agent2 = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: nfd_name.into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("norm-test".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    assert_ne!(agent1.id, agent2.id);

    pools.close().await;
}

// --- Message content tests ---

#[tokio::test]
async fn message_with_emoji_sequences() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "emoji-test-room", None, None)
        .await
        .unwrap();

    let content = "👨‍👩‍👧‍👦 Family emoji + 🏳️‍🌈 flag + 👍🏾 skin tone";
    let msg = messages::post_message(
        pool,
        PostMessageRequest {
            room_id: room.id.clone(),
            sender_id: "test-sender".into(),
            content: content.into(),
            reply_to: None,
            metadata: None,
            message_type: None,
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(msg.content, content);

    pools.close().await;
}

#[tokio::test]
async fn message_with_mixed_scripts_and_bidi() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "bidi-test", None, None)
        .await
        .unwrap();

    let content = "Hello مرحبا שלום 你好 こんにちは";
    let msg = messages::post_message(
        pool,
        PostMessageRequest {
            room_id: room.id.clone(),
            sender_id: "polyglot".into(),
            content: content.into(),
            reply_to: None,
            metadata: None,
            message_type: None,
        },
        None,
    )
    .await
    .unwrap();

    assert_eq!(msg.content, content);

    pools.close().await;
}

#[tokio::test]
async fn message_with_null_bytes_in_content() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "null-byte-test", None, None)
        .await
        .unwrap();

    let content = "before\x00after";
    let result = messages::post_message(
        pool,
        PostMessageRequest {
            room_id: room.id.clone(),
            sender_id: "test".into(),
            content: content.into(),
            reply_to: None,
            metadata: None,
            message_type: None,
        },
        None,
    )
    .await;

    // SQLite may reject null bytes or store them — either behavior is acceptable
    // but it must not panic or corrupt the database
    match result {
        Ok(msg) => assert!(msg.content.contains("before")),
        Err(_) => {} // rejection is fine
    }

    pools.close().await;
}

// --- FTS5 special character tests ---

#[tokio::test]
async fn search_messages_fts5_special_syntax_does_not_panic() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "fts-test-room", None, None)
        .await
        .unwrap();

    messages::post_message(
        pool,
        PostMessageRequest {
            room_id: room.id.clone(),
            sender_id: "writer".into(),
            content: "hello world normal message".into(),
            reply_to: None,
            metadata: None,
            message_type: None,
        },
        None,
    )
    .await
    .unwrap();

    let dangerous_queries = vec![
        "\"unclosed quote",
        "OR AND NOT",
        "hello OR",
        "(unbalanced(",
        "col:value",
        "near(a b, 5)",
        "* wildcard",
        "hello\"world",
        "a AND (b OR",
        "{braces}",
        "DROP TABLE room_messages; --",
        "'; DROP TABLE rooms; --",
    ];

    for query in dangerous_queries {
        let result = messages::search_messages(
            pool,
            SearchMessagesRequest {
                query: query.into(),
                room_id: Some(room.id.clone()),
                limit: Some(10),
            },
        )
        .await;

        // Must not panic. May return error (invalid FTS5 syntax) or empty results.
        match result {
            Ok(msgs) => assert!(msgs.len() <= 10),
            Err(_) => {} // FTS5 syntax error is acceptable
        }
    }

    pools.close().await;
}

#[tokio::test]
async fn search_agents_fts5_injection_attempts() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "normal-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("fts-test".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let attack_queries = vec![
        "\"unclosed",
        "* OR 1=1",
        "name:admin",
        "NEAR(a b)",
        "'; DELETE FROM agents; --",
        "\u{0000}null\u{0000}byte",
    ];

    for query in attack_queries {
        let result = agents::search_agents(pool, query, Some("fts-test"), Some(10)).await;

        match result {
            Ok(results) => assert!(results.len() <= 10),
            Err(_) => {} // syntax error is acceptable, not a panic
        }
    }

    pools.close().await;
}

// --- Proptest for Unicode fuzzing ---

#[cfg(test)]
mod proptest_unicode {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        #[test]
        fn room_name_arbitrary_unicode(name in "[\\p{L}\\p{N}\\p{Emoji}\\p{S}]{1,200}") {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let (pools, _dir) = setup().await;
                let pool = &pools.fts;

                let result = rooms::create_room(pool, &name, None, None).await;
                match result {
                    Ok(room) => {
                        assert_eq!(room.name, name);
                        let fetched = rooms::get_room(pool, &room.id).await.unwrap();
                        assert_eq!(fetched.name, name);
                    }
                    Err(e) => {
                        // Only validation errors are acceptable (e.g., if trim makes it empty)
                        let msg = e.to_string();
                        prop_assert!(
                            msg.contains("validation") || msg.contains("constraint"),
                            "Unexpected error: {msg}"
                        );
                    }
                }

                pools.close().await;
                Ok(())
            })?;
        }

        #[test]
        fn message_content_arbitrary_unicode(content in "[\\p{L}\\p{N}\\p{P}\\p{Emoji}\\s]{1,500}") {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let (pools, _dir) = setup().await;
                let pool = &pools.fts;

                let room = rooms::create_room(pool, "proptest-room", None, None)
                    .await
                    .unwrap();

                let result = messages::post_message(
                    pool,
                    PostMessageRequest {
                        room_id: room.id.clone(),
                        sender_id: "proptest".into(),
                        content: content.clone(),
                        reply_to: None,
                        metadata: None,
                        message_type: None,
                    },
                    None,
                )
                .await;

                match result {
                    Ok(msg) => {
                        assert_eq!(msg.content, content);
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        prop_assert!(
                            msg.contains("validation") || msg.contains("empty"),
                            "Unexpected error: {msg}"
                        );
                    }
                }

                pools.close().await;
                Ok(())
            })?;
        }
    }
}
