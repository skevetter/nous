use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nous_cli::config::Config;
use nous_cli::server::NousServer;
use nous_cli::tools::*;
use nous_core::embed::MockEmbedding;
use rmcp::model::CallToolResult;

fn test_db_path() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "/tmp/nous-e2e-room-{}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        seq,
    )
}

fn test_server(db_path: &str) -> NousServer {
    NousServer::new(
        Config::default(),
        Box::new(MockEmbedding::new(384)),
        db_path,
        None,
    )
    .unwrap()
}

fn extract_json(result: &CallToolResult) -> serde_json::Value {
    let text = result.content[0].as_text().unwrap().text.as_str();
    serde_json::from_str(text).unwrap()
}

fn is_success(result: &CallToolResult) -> bool {
    result.is_error != Some(true)
}

async fn wait_wal() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

// ---------------------------------------------------------------------------
// 1. Room CRUD lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn room_crud_lifecycle() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    // Create
    let create = handle_room_create(
        RoomCreateParams {
            name: "test-room".into(),
            purpose: Some("unit testing".into()),
            metadata: None,
        },
        &server.write_channel,
    )
    .await;
    assert!(is_success(&create), "create should succeed");
    let create_json = extract_json(&create);
    let room_id = create_json["id"].as_str().unwrap().to_string();
    assert_eq!(create_json["name"], "test-room");

    wait_wal().await;

    // Get by ID
    let get = handle_room_get(
        RoomGetParams {
            id: room_id.clone(),
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&get), "get by ID should succeed");
    let get_json = extract_json(&get);
    assert_eq!(get_json["name"], "test-room");
    assert_eq!(get_json["purpose"], "unit testing");

    // List
    let list = handle_room_list(
        RoomListParams {
            archived: false,
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&list));
    let rooms = extract_json(&list)["rooms"].as_array().unwrap().clone();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["name"], "test-room");

    // Archive (soft delete)
    let archive = handle_room_delete(
        RoomDeleteParams {
            id: room_id.clone(),
            hard: false,
        },
        &server.write_channel,
    )
    .await;
    assert!(is_success(&archive));
    assert_eq!(extract_json(&archive)["archived"], true);

    wait_wal().await;

    // Archived room should not appear in active list
    let list_active = handle_room_list(
        RoomListParams {
            archived: false,
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    let active_rooms = extract_json(&list_active)["rooms"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        active_rooms.is_empty(),
        "archived room should not appear in active list"
    );

    // Archived room should appear in archived list
    let list_archived = handle_room_list(
        RoomListParams {
            archived: true,
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    let archived_rooms = extract_json(&list_archived)["rooms"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(archived_rooms.len(), 1);
    assert_eq!(archived_rooms[0]["name"], "test-room");

    let _ = std::fs::remove_file(&db_path);
}

// ---------------------------------------------------------------------------
// 2. Message post/read with pagination
// ---------------------------------------------------------------------------

#[tokio::test]
async fn message_post_read_pagination() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    // Create room
    let create = handle_room_create(
        RoomCreateParams {
            name: "msg-room".into(),
            purpose: None,
            metadata: None,
        },
        &server.write_channel,
    )
    .await;
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_string();

    wait_wal().await;

    // Post 5 messages with small delay between each to ensure ordering
    let mut msg_ids = Vec::new();
    for i in 0..5 {
        let post = handle_room_post_message(
            RoomPostMessageParams {
                room_id: room_id.clone(),
                content: format!("message {i}"),
                sender_id: Some("agent-a".into()),
                reply_to: None,
                metadata: None,
            },
            &server.write_channel,
            &server.read_pool,
        )
        .await;
        assert!(is_success(&post), "post message {i} should succeed");
        msg_ids.push(extract_json(&post)["id"].as_str().unwrap().to_string());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    wait_wal().await;

    // Read all messages
    let read_all = handle_room_read_messages(
        RoomReadMessagesParams {
            room_id: room_id.clone(),
            limit: None,
            before: None,
            since: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&read_all));
    let all_msgs = extract_json(&read_all)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(all_msgs.len(), 5);

    // Read with limit=2
    let read_limited = handle_room_read_messages(
        RoomReadMessagesParams {
            room_id: room_id.clone(),
            limit: Some(2),
            before: None,
            since: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&read_limited));
    let limited_msgs = extract_json(&read_limited)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(limited_msgs.len(), 2);

    // Read with before cursor using created_at from the 3rd message (index 2, DESC order)
    // Messages are returned DESC, so all_msgs[0] is newest (msg 4) and all_msgs[4] is oldest (msg 0)
    // We want messages created before the middle message's timestamp
    let mid_created_at = all_msgs[2]["created_at"].as_str().unwrap().to_string();
    let read_before = handle_room_read_messages(
        RoomReadMessagesParams {
            room_id: room_id.clone(),
            limit: None,
            before: Some(mid_created_at),
            since: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&read_before));
    let before_msgs = extract_json(&read_before)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        before_msgs.len() < 5,
        "before cursor should filter some messages"
    );

    // Verify sender_id is preserved
    assert_eq!(all_msgs[0]["sender_id"], "agent-a");

    let _ = std::fs::remove_file(&db_path);
}

// ---------------------------------------------------------------------------
// 3. FTS search verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fts_search() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    let create = handle_room_create(
        RoomCreateParams {
            name: "search-room".into(),
            purpose: None,
            metadata: None,
        },
        &server.write_channel,
    )
    .await;
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_string();

    wait_wal().await;

    // Post messages with distinct content
    for (i, content) in [
        "kubernetes deployment strategy",
        "database connection pooling",
        "kubernetes pod autoscaling",
        "react component lifecycle",
    ]
    .iter()
    .enumerate()
    {
        let post = handle_room_post_message(
            RoomPostMessageParams {
                room_id: room_id.clone(),
                content: content.to_string(),
                sender_id: Some(format!("agent-{i}")),
                reply_to: None,
                metadata: None,
            },
            &server.write_channel,
            &server.read_pool,
        )
        .await;
        assert!(is_success(&post));
    }

    wait_wal().await;

    // Search for "kubernetes" — should find 2 messages
    let search = handle_room_search(
        RoomSearchParams {
            room_id: room_id.clone(),
            query: "kubernetes".into(),
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&search));
    let results = extract_json(&search)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(results.len(), 2, "should find 2 kubernetes messages");

    // Search for "database" — should find 1
    let search_db = handle_room_search(
        RoomSearchParams {
            room_id: room_id.clone(),
            query: "database".into(),
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&search_db));
    let db_results = extract_json(&search_db)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(db_results.len(), 1);

    // Search with limit
    let search_limited = handle_room_search(
        RoomSearchParams {
            room_id: room_id.clone(),
            query: "kubernetes".into(),
            limit: Some(1),
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&search_limited));
    let limited = extract_json(&search_limited)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(limited.len(), 1);

    // Search for nonexistent term — should return empty
    let search_empty = handle_room_search(
        RoomSearchParams {
            room_id: room_id.clone(),
            query: "xylophone".into(),
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&search_empty));
    let empty = extract_json(&search_empty)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert!(empty.is_empty());

    let _ = std::fs::remove_file(&db_path);
}

// ---------------------------------------------------------------------------
// 4. Join/Info with participants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn join_and_info() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    let create = handle_room_create(
        RoomCreateParams {
            name: "team-room".into(),
            purpose: Some("collaboration".into()),
            metadata: None,
        },
        &server.write_channel,
    )
    .await;
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_string();

    wait_wal().await;

    // Join with different roles
    let join_owner = handle_room_join(
        RoomJoinParams {
            room_id: room_id.clone(),
            agent_id: "agent-owner".into(),
            role: Some("owner".into()),
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&join_owner));
    let owner_json = extract_json(&join_owner);
    assert_eq!(owner_json["role"], "owner");

    let join_member = handle_room_join(
        RoomJoinParams {
            room_id: room_id.clone(),
            agent_id: "agent-member".into(),
            role: None, // defaults to "member"
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&join_member));
    assert_eq!(extract_json(&join_member)["role"], "member");

    let join_observer = handle_room_join(
        RoomJoinParams {
            room_id: room_id.clone(),
            agent_id: "agent-observer".into(),
            role: Some("observer".into()),
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&join_observer));

    wait_wal().await;

    // Post a message so message_count > 0
    let post = handle_room_post_message(
        RoomPostMessageParams {
            room_id: room_id.clone(),
            content: "hello team".into(),
            sender_id: Some("agent-owner".into()),
            reply_to: None,
            metadata: None,
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&post));

    wait_wal().await;

    // Get room info
    let info = handle_room_info(
        RoomInfoParams {
            id: room_id.clone(),
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&info));
    let info_json = extract_json(&info);
    assert_eq!(info_json["room"]["name"], "team-room");
    assert_eq!(info_json["room"]["purpose"], "collaboration");
    assert_eq!(info_json["message_count"], 1);
    let participants = info_json["participants"].as_array().unwrap();
    assert_eq!(participants.len(), 3);

    // Verify invalid role is rejected
    let join_invalid = handle_room_join(
        RoomJoinParams {
            room_id: room_id.clone(),
            agent_id: "agent-bad".into(),
            role: Some("admin".into()),
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert_eq!(join_invalid.is_error, Some(true));

    let _ = std::fs::remove_file(&db_path);
}

// ---------------------------------------------------------------------------
// 5. Hard delete verification
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hard_delete() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    // Create room and post a message
    let create = handle_room_create(
        RoomCreateParams {
            name: "doomed-room".into(),
            purpose: None,
            metadata: None,
        },
        &server.write_channel,
    )
    .await;
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_string();

    wait_wal().await;

    let post = handle_room_post_message(
        RoomPostMessageParams {
            room_id: room_id.clone(),
            content: "this will be deleted".into(),
            sender_id: None,
            reply_to: None,
            metadata: None,
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&post));

    // Join a participant
    let join = handle_room_join(
        RoomJoinParams {
            room_id: room_id.clone(),
            agent_id: "agent-x".into(),
            role: None,
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&join));

    wait_wal().await;

    // Hard delete
    let delete = handle_room_delete(
        RoomDeleteParams {
            id: room_id.clone(),
            hard: true,
        },
        &server.write_channel,
    )
    .await;
    assert!(is_success(&delete));
    assert_eq!(extract_json(&delete)["deleted"], true);

    wait_wal().await;

    // Room should not be found by ID
    let get = handle_room_get(
        RoomGetParams {
            id: room_id.clone(),
        },
        &server.read_pool,
    )
    .await;
    assert_eq!(
        get.is_error,
        Some(true),
        "hard-deleted room should not be found"
    );

    // Room should not appear in active or archived lists
    let list_active = handle_room_list(
        RoomListParams {
            archived: false,
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(
        extract_json(&list_active)["rooms"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let list_archived = handle_room_list(
        RoomListParams {
            archived: true,
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(
        extract_json(&list_archived)["rooms"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Messages should also be gone — reading from the room should error
    let read = handle_room_read_messages(
        RoomReadMessagesParams {
            room_id: room_id.clone(),
            limit: None,
            before: None,
            since: None,
        },
        &server.read_pool,
    )
    .await;
    // Either error (room not found) or empty messages list is acceptable
    if is_success(&read) {
        let msgs = extract_json(&read)["messages"].as_array().unwrap().clone();
        assert!(msgs.is_empty(), "messages should be gone after hard delete");
    }

    let _ = std::fs::remove_file(&db_path);
}

// ---------------------------------------------------------------------------
// 6. Dual-lookup everywhere (by name instead of ID)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dual_lookup_by_name() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    // Create room
    let create = handle_room_create(
        RoomCreateParams {
            name: "named-room".into(),
            purpose: Some("dual-lookup test".into()),
            metadata: None,
        },
        &server.write_channel,
    )
    .await;
    assert!(is_success(&create));

    wait_wal().await;

    // Get by name (not UUID)
    let get = handle_room_get(
        RoomGetParams {
            id: "named-room".into(),
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&get), "get by name should succeed");
    let get_json = extract_json(&get);
    assert_eq!(get_json["name"], "named-room");
    assert_eq!(get_json["purpose"], "dual-lookup test");

    // Post message by name
    let post = handle_room_post_message(
        RoomPostMessageParams {
            room_id: "named-room".into(),
            content: "posted by name".into(),
            sender_id: Some("agent-1".into()),
            reply_to: None,
            metadata: None,
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&post), "post by name should succeed");

    wait_wal().await;

    // Read messages by name
    let read = handle_room_read_messages(
        RoomReadMessagesParams {
            room_id: "named-room".into(),
            limit: None,
            before: None,
            since: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&read), "read by name should succeed");
    let msgs = extract_json(&read)["messages"].as_array().unwrap().clone();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["content"], "posted by name");

    // Search by name
    let search = handle_room_search(
        RoomSearchParams {
            room_id: "named-room".into(),
            query: "posted".into(),
            limit: None,
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&search), "search by name should succeed");
    let search_results = extract_json(&search)["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(search_results.len(), 1);

    // Join by name
    let join = handle_room_join(
        RoomJoinParams {
            room_id: "named-room".into(),
            agent_id: "agent-2".into(),
            role: Some("member".into()),
        },
        &server.write_channel,
        &server.read_pool,
    )
    .await;
    assert!(is_success(&join), "join by name should succeed");

    wait_wal().await;

    // Info by name
    let info = handle_room_info(
        RoomInfoParams {
            id: "named-room".into(),
        },
        &server.read_pool,
    )
    .await;
    assert!(is_success(&info), "info by name should succeed");
    let info_json = extract_json(&info);
    assert_eq!(info_json["room"]["name"], "named-room");
    assert_eq!(info_json["message_count"], 1);
    assert_eq!(info_json["participants"].as_array().unwrap().len(), 1);

    // Nonexistent name should error
    let get_missing = handle_room_get(
        RoomGetParams {
            id: "nonexistent-room".into(),
        },
        &server.read_pool,
    )
    .await;
    assert_eq!(
        get_missing.is_error,
        Some(true),
        "nonexistent name should return error"
    );

    let _ = std::fs::remove_file(&db_path);
}
