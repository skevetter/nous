use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nous_core::db::DbPools;
use nous_core::notifications::NotificationRegistry;
use nous_daemon::app;
use nous_daemon::state::AppState;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

async fn test_state() -> (AppState, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    let state = AppState {
        pool: pools.fts.clone(),
        registry: Arc::new(NotificationRegistry::new()),
    };
    (state, tmp)
}

async fn json_body(response: axum::http::Response<Body>) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

// --- Happy Path: Full E2E Flow ---

#[tokio::test]
async fn e2e_create_post_read_search_delete() {
    let (state, _tmp) = test_state().await;

    // 1. Create room
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "e2e-room",
                        "purpose": "End-to-end test"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let room: Value = json_body(response).await;
    assert_eq!(room["name"], "e2e-room");
    assert_eq!(room["purpose"], "End-to-end test");
    let room_id = room["id"].as_str().unwrap().to_string();

    // 2. Post message
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rooms/{room_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "sender_id": "test-agent",
                        "content": "Integration test message about deployment"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let msg: Value = json_body(response).await;
    assert_eq!(msg["content"], "Integration test message about deployment");
    assert_eq!(msg["sender_id"], "test-agent");
    assert_eq!(msg["room_id"], room_id);

    // 3. Read messages
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/rooms/{room_id}/messages"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let messages: Vec<Value> = serde_json::from_slice(
        &response.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0]["content"],
        "Integration test message about deployment"
    );

    // 4. Search messages via FTS5
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/search/messages?q=deployment")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let results: Vec<Value> = serde_json::from_slice(
        &response.into_body().collect().await.unwrap().to_bytes(),
    )
    .unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0]["content"]
        .as_str()
        .unwrap()
        .contains("deployment"));

    // 5. room_wait via MCP (with short timeout — expect timeout response)
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_wait",
                        "arguments": { "room_id": room_id, "timeout_ms": 50 }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let wait_resp: Value = json_body(response).await;
    assert!(wait_resp.get("is_error").is_none());
    let wait_text = wait_resp["content"][0]["text"].as_str().unwrap();
    let wait_result: Value = serde_json::from_str(wait_text).unwrap();
    assert_eq!(wait_result["timed_out"], true);

    // 6. Delete room
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rooms/{room_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // 7. Verify 404 on GET after deletion (soft-delete archives it, name lookup fails)
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/rooms/{room_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Soft delete still returns by ID (archived), so we verify the room is archived
    let fetched: Value = json_body(response).await;
    assert_eq!(fetched["archived"], true);
}

#[tokio::test]
async fn e2e_hard_delete_then_404() {
    let (state, _tmp) = test_state().await;

    // Create room
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "hard-del-room"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let room: Value = json_body(response).await;
    let room_id = room["id"].as_str().unwrap().to_string();

    // Hard delete
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/rooms/{room_id}?hard=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify 404
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/rooms/{room_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// --- Edge Cases ---

#[tokio::test]
async fn error_create_room_empty_name() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "",
                        "purpose": "Should fail"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = json_body(response).await;
    assert!(body["error"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn error_create_room_whitespace_name() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "   "
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn error_post_message_empty_content() {
    let (state, _tmp) = test_state().await;
    let room = nous_core::rooms::create_room(&state.pool, "msg-err-room", None, None)
        .await
        .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/rooms/{}/messages", room.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "sender_id": "agent-x",
                        "content": ""
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = json_body(response).await;
    assert!(body["error"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn error_create_room_duplicate_name() {
    let (state, _tmp) = test_state().await;

    // Create first room
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "dup-room"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    // Attempt duplicate
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "dup-room"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value = json_body(response).await;
    assert!(body["error"].as_str().unwrap().contains("already exists"));
}

#[tokio::test]
async fn error_get_nonexistent_room() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/rooms/nonexistent-id-12345")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn error_mcp_call_invalid_tool() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "nonexistent_tool",
                        "arguments": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["is_error"], true);
    assert!(body["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown tool"));
}

#[tokio::test]
async fn error_mcp_call_missing_required_args() {
    let (state, _tmp) = test_state().await;

    // room_create requires "name" field
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_create",
                        "arguments": {}
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["is_error"], true);
    assert!(body["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("missing required field"));
}

#[tokio::test]
async fn error_post_message_to_nonexistent_room() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/rooms/nonexistent-room-id/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "sender_id": "agent-x",
                        "content": "Hello"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mcp_full_flow_via_tool_dispatch() {
    let (state, _tmp) = test_state().await;

    // Create room via MCP
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_create",
                        "arguments": { "name": "mcp-flow-room", "purpose": "MCP e2e" }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let resp: Value = json_body(response).await;
    assert!(resp.get("is_error").is_none());
    let room: Value = serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let room_id = room["id"].as_str().unwrap();

    // Post message via MCP
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_post_message",
                        "arguments": {
                            "room_id": room_id,
                            "sender_id": "mcp-agent",
                            "content": "MCP dispatched message about refactoring"
                        }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let resp: Value = json_body(response).await;
    assert!(resp.get("is_error").is_none());

    // Read messages via MCP
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_read_messages",
                        "arguments": { "room_id": room_id }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let resp: Value = json_body(response).await;
    let messages: Vec<Value> =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["content"], "MCP dispatched message about refactoring");

    // Search via MCP
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_search",
                        "arguments": { "query": "refactoring" }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let resp: Value = json_body(response).await;
    assert!(resp.get("is_error").is_none());
    let results: Vec<Value> =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(results.len(), 1);

    // Delete via MCP (soft — the default)
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_delete",
                        "arguments": { "id": room_id }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let resp: Value = json_body(response).await;
    assert!(resp.get("is_error").is_none(), "room_delete error: {resp:?}");

    // Verify room is now archived via room_get
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": "room_get",
                        "arguments": { "id": room_id }
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let resp: Value = json_body(response).await;
    assert!(resp.get("is_error").is_none());
    let room_after: Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(room_after["archived"], true);
}
