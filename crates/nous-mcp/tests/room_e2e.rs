use rmcp::model::CallToolRequestParams;
use rmcp::{ClientHandler, ServiceExt};

fn test_db_path() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "/tmp/nous-room-e2e-{}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        seq,
    )
}

fn call_params(
    name: impl Into<std::borrow::Cow<'static, str>>,
    args: serde_json::Value,
) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone())
}

fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = result.content[0].as_text().unwrap().text.as_str();
    serde_json::from_str(text).unwrap()
}

fn assert_ok(result: &rmcp::model::CallToolResult, ctx: &str) {
    if result.is_error == Some(true) {
        let msg = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("<no text>");
        panic!("{ctx} failed: {msg}");
    }
}

fn assert_err(result: &rmcp::model::CallToolResult, ctx: &str) {
    assert!(
        result.is_error == Some(true),
        "{ctx}: expected error but got success"
    );
}

async fn setup() -> (
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    String,
) {
    let db_path = test_db_path();
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    let mut cfg = nous_mcp::config::Config::default();
    cfg.encryption.db_key_file = format!("{db_path}.key");
    let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
    let server = nous_mcp::server::NousServer::new(cfg, embedding, &db_path).unwrap();

    tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    let client = TestClient.serve(client_transport).await.unwrap();
    (client, db_path)
}

#[derive(Debug, Clone, Default)]
struct TestClient;
impl ClientHandler for TestClient {}

const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);

#[tokio::test]
async fn room_lifecycle_create_list_get_archive_delete() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "engineering", "purpose": "Ship features" }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "room_create");
    let create_json = extract_json(&create_result);
    let room_id = create_json["id"].as_str().unwrap().to_owned();
    assert_eq!(create_json["name"], "engineering");

    tokio::time::sleep(SETTLE).await;

    let list_result = client
        .call_tool(call_params(
            "room_list",
            serde_json::json!({ "archived": false }),
        ))
        .await
        .unwrap();
    assert_ok(&list_result, "room_list");
    let list_json = extract_json(&list_result);
    let rooms = list_json["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["name"], "engineering");

    let get_by_id = client
        .call_tool(call_params(
            "room_get",
            serde_json::json!({ "id": room_id }),
        ))
        .await
        .unwrap();
    assert_ok(&get_by_id, "room_get by ID");
    let get_json = extract_json(&get_by_id);
    assert_eq!(get_json["id"], room_id);
    assert_eq!(get_json["purpose"], "Ship features");

    let get_by_name = client
        .call_tool(call_params(
            "room_get",
            serde_json::json!({ "id": "engineering" }),
        ))
        .await
        .unwrap();
    assert_ok(&get_by_name, "room_get by name");
    let get_name_json = extract_json(&get_by_name);
    assert_eq!(get_name_json["id"], room_id);

    let archive_result = client
        .call_tool(call_params(
            "room_delete",
            serde_json::json!({ "id": room_id, "hard": false }),
        ))
        .await
        .unwrap();
    assert_ok(&archive_result, "room_delete (archive)");
    let archive_json = extract_json(&archive_result);
    assert_eq!(archive_json["archived"], true);

    tokio::time::sleep(SETTLE).await;

    let list_active = client
        .call_tool(call_params(
            "room_list",
            serde_json::json!({ "archived": false }),
        ))
        .await
        .unwrap();
    assert_ok(&list_active, "room_list after archive");
    let active_json = extract_json(&list_active);
    assert!(
        active_json["rooms"].as_array().unwrap().is_empty(),
        "archived room should not appear in active list"
    );

    let list_archived = client
        .call_tool(call_params(
            "room_list",
            serde_json::json!({ "archived": true }),
        ))
        .await
        .unwrap();
    assert_ok(&list_archived, "room_list archived");
    let archived_json = extract_json(&list_archived);
    assert_eq!(archived_json["rooms"].as_array().unwrap().len(), 1);

    let create2 = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "to-delete" }),
        ))
        .await
        .unwrap();
    assert_ok(&create2, "room_create for hard delete");
    let room2_id = extract_json(&create2)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    let hard_delete = client
        .call_tool(call_params(
            "room_delete",
            serde_json::json!({ "id": room2_id, "hard": true }),
        ))
        .await
        .unwrap();
    assert_ok(&hard_delete, "room_delete (hard)");
    let hard_json = extract_json(&hard_delete);
    assert_eq!(hard_json["deleted"], true);

    tokio::time::sleep(SETTLE).await;

    let get_deleted = client
        .call_tool(call_params(
            "room_get",
            serde_json::json!({ "id": room2_id }),
        ))
        .await
        .unwrap();
    assert_err(&get_deleted, "room_get after hard delete");

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_messages_post_read_search_reply() {
    let (client, db_path) = setup().await;

    let create = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "dev-chat" }),
        ))
        .await
        .unwrap();
    assert_ok(&create, "room_create");
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    let msg1 = client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": room_id,
                "sender_id": "alice",
                "content": "The deployment pipeline is broken again"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&msg1, "post_message 1");
    let msg1_id = extract_json(&msg1)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    let msg2 = client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": room_id,
                "sender_id": "bob",
                "content": "I found the flaky test causing the failure",
                "reply_to": msg1_id
            }),
        ))
        .await
        .unwrap();
    assert_ok(&msg2, "post_message 2 (reply)");
    let msg2_json = extract_json(&msg2);
    assert_eq!(msg2_json["sender_id"], "bob");

    let msg3 = client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": room_id,
                "sender_id": "alice",
                "content": "Great, the linter config also needs updating"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&msg3, "post_message 3");

    tokio::time::sleep(SETTLE).await;

    let read_result = client
        .call_tool(call_params(
            "room_read_messages",
            serde_json::json!({ "room_id": room_id }),
        ))
        .await
        .unwrap();
    assert_ok(&read_result, "room_read_messages");
    let read_json = extract_json(&read_result);
    let messages = read_json["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3, "should have 3 messages");

    let read_limited = client
        .call_tool(call_params(
            "room_read_messages",
            serde_json::json!({ "room_id": room_id, "limit": 2 }),
        ))
        .await
        .unwrap();
    assert_ok(&read_limited, "room_read_messages with limit");
    let limited_json = extract_json(&read_limited);
    assert_eq!(limited_json["messages"].as_array().unwrap().len(), 2);

    let search_result = client
        .call_tool(call_params(
            "room_search",
            serde_json::json!({
                "room_id": room_id,
                "query": "flaky test"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&search_result, "room_search");
    let search_json = extract_json(&search_result);
    let search_msgs = search_json["messages"].as_array().unwrap();
    assert_eq!(search_msgs.len(), 1, "FTS should find exactly one match");
    assert_eq!(search_msgs[0]["sender_id"], "bob");

    let search_none = client
        .call_tool(call_params(
            "room_search",
            serde_json::json!({
                "room_id": room_id,
                "query": "kubernetes scaling"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&search_none, "room_search (no match)");
    let none_json = extract_json(&search_none);
    assert!(
        none_json["messages"].as_array().unwrap().is_empty(),
        "search for non-existent term should return empty"
    );

    let read_by_name = client
        .call_tool(call_params(
            "room_read_messages",
            serde_json::json!({ "room_id": "dev-chat" }),
        ))
        .await
        .unwrap();
    assert_ok(&read_by_name, "room_read_messages by name");
    let name_json = extract_json(&read_by_name);
    assert_eq!(name_json["messages"].as_array().unwrap().len(), 3);

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_participants_and_info() {
    let (client, db_path) = setup().await;

    let create = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "standup", "purpose": "Daily sync" }),
        ))
        .await
        .unwrap();
    assert_ok(&create, "room_create");
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    let join1 = client
        .call_tool(call_params(
            "room_join",
            serde_json::json!({
                "room_id": room_id,
                "agent_id": "agent-alpha",
                "role": "owner"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&join1, "room_join owner");
    let join1_json = extract_json(&join1);
    assert_eq!(join1_json["role"], "owner");

    let join2 = client
        .call_tool(call_params(
            "room_join",
            serde_json::json!({
                "room_id": room_id,
                "agent_id": "agent-beta"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&join2, "room_join member (default role)");
    let join2_json = extract_json(&join2);
    assert_eq!(join2_json["role"], "member");

    let join3 = client
        .call_tool(call_params(
            "room_join",
            serde_json::json!({
                "room_id": room_id,
                "agent_id": "agent-gamma",
                "role": "observer"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&join3, "room_join observer");

    let join_bad_role = client
        .call_tool(call_params(
            "room_join",
            serde_json::json!({
                "room_id": room_id,
                "agent_id": "agent-delta",
                "role": "admin"
            }),
        ))
        .await
        .unwrap();
    assert_err(&join_bad_role, "room_join invalid role");

    tokio::time::sleep(SETTLE).await;

    client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": room_id,
                "sender_id": "agent-alpha",
                "content": "Starting standup"
            }),
        ))
        .await
        .unwrap();

    tokio::time::sleep(SETTLE).await;

    let info_result = client
        .call_tool(call_params(
            "room_info",
            serde_json::json!({ "id": room_id }),
        ))
        .await
        .unwrap();
    assert_ok(&info_result, "room_info");
    let info_json = extract_json(&info_result);
    assert_eq!(info_json["room"]["name"], "standup");
    assert_eq!(info_json["room"]["purpose"], "Daily sync");
    assert_eq!(info_json["participants"].as_array().unwrap().len(), 3);
    assert_eq!(info_json["message_count"], 1);

    let info_by_name = client
        .call_tool(call_params(
            "room_info",
            serde_json::json!({ "id": "standup" }),
        ))
        .await
        .unwrap();
    assert_ok(&info_by_name, "room_info by name");
    let info_name_json = extract_json(&info_by_name);
    assert_eq!(info_name_json["room"]["name"], "standup");

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_duplicate_name_rejected() {
    let (client, db_path) = setup().await;

    let create1 = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "unique-room" }),
        ))
        .await
        .unwrap();
    assert_ok(&create1, "room_create first");

    tokio::time::sleep(SETTLE).await;

    let create2 = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "unique-room" }),
        ))
        .await
        .unwrap();
    assert_err(&create2, "room_create duplicate name");

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_empty_room_read_and_search() {
    let (client, db_path) = setup().await;

    let create = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "empty" }),
        ))
        .await
        .unwrap();
    assert_ok(&create, "room_create");
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    let read = client
        .call_tool(call_params(
            "room_read_messages",
            serde_json::json!({ "room_id": room_id }),
        ))
        .await
        .unwrap();
    assert_ok(&read, "read empty room");
    let read_json = extract_json(&read);
    assert!(read_json["messages"].as_array().unwrap().is_empty());

    let search = client
        .call_tool(call_params(
            "room_search",
            serde_json::json!({ "room_id": room_id, "query": "anything" }),
        ))
        .await
        .unwrap();
    assert_ok(&search, "search empty room");
    let search_json = extract_json(&search);
    assert!(search_json["messages"].as_array().unwrap().is_empty());

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_not_found_errors() {
    let (client, db_path) = setup().await;

    let get_missing = client
        .call_tool(call_params(
            "room_get",
            serde_json::json!({ "id": "nonexistent-room" }),
        ))
        .await
        .unwrap();
    assert_err(&get_missing, "room_get nonexistent");

    let info_missing = client
        .call_tool(call_params(
            "room_info",
            serde_json::json!({ "id": "nonexistent-room" }),
        ))
        .await
        .unwrap();
    assert_err(&info_missing, "room_info nonexistent");

    let post_missing = client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": "nonexistent-room",
                "content": "hello"
            }),
        ))
        .await
        .unwrap();
    assert_err(&post_missing, "room_post_message to nonexistent room");

    let read_missing = client
        .call_tool(call_params(
            "room_read_messages",
            serde_json::json!({ "room_id": "nonexistent-room" }),
        ))
        .await
        .unwrap();
    assert_err(&read_missing, "room_read_messages from nonexistent room");

    let search_missing = client
        .call_tool(call_params(
            "room_search",
            serde_json::json!({ "room_id": "nonexistent-room", "query": "test" }),
        ))
        .await
        .unwrap();
    assert_err(&search_missing, "room_search in nonexistent room");

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_default_sender_is_system() {
    let (client, db_path) = setup().await;

    let create = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "sys-sender" }),
        ))
        .await
        .unwrap();
    assert_ok(&create, "room_create");
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    let post = client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": room_id,
                "content": "no sender specified"
            }),
        ))
        .await
        .unwrap();
    assert_ok(&post, "post without sender_id");
    let post_json = extract_json(&post);
    assert_eq!(post_json["sender_id"], "system");

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn room_cascade_delete_removes_messages_and_participants() {
    let (client, db_path) = setup().await;

    let create = client
        .call_tool(call_params(
            "room_create",
            serde_json::json!({ "name": "cascade-test" }),
        ))
        .await
        .unwrap();
    assert_ok(&create, "room_create");
    let room_id = extract_json(&create)["id"].as_str().unwrap().to_owned();

    tokio::time::sleep(SETTLE).await;

    client
        .call_tool(call_params(
            "room_join",
            serde_json::json!({
                "room_id": room_id,
                "agent_id": "agent-1",
                "role": "owner"
            }),
        ))
        .await
        .unwrap();

    client
        .call_tool(call_params(
            "room_post_message",
            serde_json::json!({
                "room_id": room_id,
                "sender_id": "agent-1",
                "content": "This will be deleted"
            }),
        ))
        .await
        .unwrap();

    tokio::time::sleep(SETTLE).await;

    let hard_delete = client
        .call_tool(call_params(
            "room_delete",
            serde_json::json!({ "id": room_id, "hard": true }),
        ))
        .await
        .unwrap();
    assert_ok(&hard_delete, "room_delete (hard)");

    tokio::time::sleep(SETTLE).await;

    let get_result = client
        .call_tool(call_params(
            "room_get",
            serde_json::json!({ "id": room_id }),
        ))
        .await
        .unwrap();
    assert_err(&get_result, "room_get after cascade delete");

    client.cancel().await.unwrap();
    let _ = std::fs::remove_file(&db_path);
}
