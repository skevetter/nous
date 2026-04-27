use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::server::NousServer;

#[derive(Clone)]
pub struct AppState {
    shutdown_tx: watch::Sender<bool>,
    start_time: Instant,
    server: Arc<NousServer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusResponse {
    pub pid: u32,
    pub uptime_secs: u64,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ShutdownResponse {
    pub ok: bool,
}

pub fn daemon_router(shutdown_tx: watch::Sender<bool>, server: Arc<NousServer>) -> Router {
    let state = Arc::new(AppState {
        shutdown_tx,
        start_time: Instant::now(),
        server,
    });

    Router::new()
        .route("/status", get(handle_status))
        .route("/shutdown", post(handle_shutdown))
        .route("/rooms", post(handle_create_room))
        .route("/rooms", get(handle_list_rooms))
        .route("/rooms/{id}", get(handle_get_room))
        .route("/rooms/{id}/messages", post(handle_post_message))
        .route("/rooms/{id}/messages", get(handle_read_messages))
        .route("/memories/search", post(handle_search_memories))
        .route("/memories/store", post(handle_store_memory))
        .route("/categories", get(handle_list_categories))
        .route("/export", post(handle_export))
        .route("/import", post(handle_import))
        .with_state(state)
}

async fn handle_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    Json(StatusResponse {
        pid: std::process::id(),
        uptime_secs: state.start_time.elapsed().as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn handle_shutdown(State(state): State<Arc<AppState>>) -> Json<ShutdownResponse> {
    let _ = state.shutdown_tx.send(true);
    Json(ShutdownResponse { ok: true })
}

// --- Room handlers ---

#[derive(Debug, Deserialize)]
struct CreateRoomRequest {
    name: String,
    purpose: Option<String>,
    metadata: Option<String>,
}

async fn handle_create_room(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateRoomRequest>,
) -> impl IntoResponse {
    let id = nous_shared::ids::MemoryId::new().to_string();
    match state
        .server
        .write_channel
        .create_room(id.clone(), body.name.clone(), body.purpose, body.metadata)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "name": body.name})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ListRoomsQuery {
    archived: Option<bool>,
    limit: Option<usize>,
}

async fn handle_list_rooms(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListRoomsQuery>,
) -> impl IntoResponse {
    let archived = query.archived.unwrap_or(false);
    match state
        .server
        .read_pool
        .list_rooms(archived, query.limit)
        .await
    {
        Ok(rooms) => {
            let list: Vec<serde_json::Value> = rooms.iter().map(|r| serde_json::json!(r)).collect();
            (StatusCode::OK, Json(serde_json::json!({"rooms": list})))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

async fn handle_get_room(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if looks_like_uuid(&id) {
        match state.server.read_pool.get_room(&id).await {
            Ok(Some(room)) => return (StatusCode::OK, Json(serde_json::json!(room))),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("{e}")})),
                );
            }
            Ok(None) => {}
        }
    }
    match state.server.read_pool.get_room_by_name(&id).await {
        Ok(Some(room)) => (StatusCode::OK, Json(serde_json::json!(room))),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("room not found: {id}")})),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct PostMessageRequest {
    content: String,
    sender: Option<String>,
    reply_to: Option<String>,
    metadata: Option<String>,
}

async fn handle_post_message(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<PostMessageRequest>,
) -> impl IntoResponse {
    let room_id = match resolve_room_id(&id, &state.server.read_pool).await {
        Ok(rid) => rid,
        Err(status) => return status,
    };
    let msg_id = nous_shared::ids::MemoryId::new().to_string();
    let sender_id = body.sender.unwrap_or_else(|| "system".to_string());
    match state
        .server
        .write_channel
        .post_message(
            msg_id.clone(),
            room_id.clone(),
            sender_id.clone(),
            body.content,
            body.reply_to,
            body.metadata,
        )
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": msg_id,
                "room_id": room_id,
                "sender_id": sender_id,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

#[derive(Debug, Deserialize)]
struct ReadMessagesQuery {
    limit: Option<usize>,
    since: Option<String>,
    before: Option<String>,
}

async fn handle_read_messages(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ReadMessagesQuery>,
) -> impl IntoResponse {
    let room_id = match resolve_room_id(&id, &state.server.read_pool).await {
        Ok(rid) => rid,
        Err(status) => return status,
    };
    match state
        .server
        .read_pool
        .list_messages(&room_id, query.limit, query.before, query.since)
        .await
    {
        Ok(messages) => {
            let list: Vec<serde_json::Value> =
                messages.iter().map(|m| serde_json::json!(m)).collect();
            (StatusCode::OK, Json(serde_json::json!({"messages": list})))
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// --- Memory handlers ---

#[derive(Debug, Deserialize)]
struct StoreMemoryRequest {
    title: String,
    content: String,
    memory_type: String,
    #[serde(default)]
    tags: Vec<String>,
    source: Option<String>,
    importance: Option<String>,
    confidence: Option<String>,
    workspace_path: Option<String>,
    session_id: Option<String>,
    trace_id: Option<String>,
    agent_id: Option<String>,
    agent_model: Option<String>,
    valid_from: Option<String>,
    category_id: Option<i64>,
}

async fn handle_store_memory(
    State(state): State<Arc<AppState>>,
    Json(body): Json<StoreMemoryRequest>,
) -> impl IntoResponse {
    use crate::tools::{MemoryStoreParams, handle_store};

    let params = MemoryStoreParams {
        title: body.title,
        content: body.content,
        memory_type: body.memory_type,
        tags: body.tags,
        source: body.source,
        importance: body.importance,
        confidence: body.confidence,
        workspace_path: body.workspace_path,
        session_id: body.session_id,
        trace_id: body.trace_id,
        agent_id: body.agent_id,
        agent_model: body.agent_model,
        valid_from: body.valid_from,
        category_id: body.category_id,
    };

    let result = handle_store(
        params,
        &state.server.write_channel,
        &state.server.embedding,
        &state.server.classifier,
        &state.server.chunker,
    )
    .await;

    call_tool_result_to_response(result)
}

#[derive(Debug, Deserialize)]
struct SearchMemoriesRequest {
    query: String,
    mode: Option<String>,
    memory_type: Option<String>,
    category_id: Option<i64>,
    workspace_id: Option<i64>,
    trace_id: Option<String>,
    session_id: Option<String>,
    importance: Option<String>,
    confidence: Option<String>,
    tags: Option<Vec<String>>,
    archived: Option<bool>,
    since: Option<String>,
    until: Option<String>,
    valid_only: Option<bool>,
    limit: Option<usize>,
}

async fn handle_search_memories(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SearchMemoriesRequest>,
) -> impl IntoResponse {
    use crate::tools::{MemorySearchParams, handle_search};

    let params = MemorySearchParams {
        query: body.query,
        mode: body.mode,
        memory_type: body.memory_type,
        category_id: body.category_id,
        workspace_id: body.workspace_id,
        trace_id: body.trace_id,
        session_id: body.session_id,
        importance: body.importance,
        confidence: body.confidence,
        tags: body.tags,
        archived: body.archived,
        since: body.since,
        until: body.until,
        valid_only: body.valid_only,
        limit: body.limit,
    };

    let result = handle_search(
        params,
        &state.server.db_path,
        state.server.config.embedding.dimensions,
        &state.server.embedding,
    )
    .await;

    call_tool_result_to_response(result)
}

// --- Category handler ---

async fn handle_list_categories(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use crate::tools::handle_category_list;

    let result = handle_category_list(
        &state.server.db_path,
        state.server.config.embedding.dimensions,
        None,
    )
    .await;
    call_tool_result_to_response(result)
}

// --- Export/Import handlers ---

async fn handle_export(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    use crate::commands::build_export_data;
    use nous_core::db::MemoryDb;

    let db_path = state.server.db_path.clone();
    let dim = state.server.embedding.dimensions();
    match nous_shared::sqlite::spawn_blocking(move || {
        let db = MemoryDb::open(&db_path, None, dim)?;
        build_export_data(&db).map_err(|e| nous_shared::NousError::Internal(e.to_string()))
    })
    .await
    {
        Ok(data) => (StatusCode::OK, Json(serde_json::json!(data))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

async fn handle_import(
    State(state): State<Arc<AppState>>,
    Json(data): Json<crate::commands::ExportData>,
) -> impl IntoResponse {
    use crate::commands::import_data;
    use nous_core::db::MemoryDb;

    let db_path = state.server.db_path.clone();
    let embedding = state.server.embedding.clone();
    let chunker = state.server.chunker.clone();
    let memory_count = data.memories.len();

    match nous_shared::sqlite::spawn_blocking(move || {
        let db = MemoryDb::open(&db_path, None, embedding.dimensions())?;
        import_data(&db, &data, embedding.as_ref(), &chunker)
            .map_err(|e| nous_shared::NousError::Internal(e.to_string()))
    })
    .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "imported",
                "memories_imported": memory_count,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

// --- Helpers ---

fn looks_like_uuid(s: &str) -> bool {
    uuid::Uuid::try_parse(s).is_ok()
}

async fn resolve_room_id(
    id_or_name: &str,
    read_pool: &nous_core::channel::ReadPool,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    if looks_like_uuid(id_or_name) {
        match read_pool.get_room(id_or_name).await {
            Ok(Some(room)) => return Ok(room.id),
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("{e}")})),
                ));
            }
            Ok(None) => {}
        }
    }
    match read_pool.get_room_by_name(id_or_name).await {
        Ok(Some(room)) => Ok(room.id),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("room not found: {id_or_name}")})),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("{e}")})),
        )),
    }
}

fn call_tool_result_to_response(
    result: rmcp::model::CallToolResult,
) -> (StatusCode, Json<serde_json::Value>) {
    let is_error = result.is_error == Some(true);
    let text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("{}");

    if is_error {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": text})),
        )
    } else {
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(json) => (StatusCode::OK, Json(json)),
            Err(_) => (StatusCode::OK, Json(serde_json::json!({"result": text}))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use nous_core::embed::MockEmbedding;
    use tower::ServiceExt;

    fn test_db_path() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!(
            "/tmp/nous-daemon-api-test-{}-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            seq,
        )
    }

    fn test_server(db_path: &str) -> Arc<NousServer> {
        let mut cfg = crate::config::Config::default();
        cfg.encryption.db_key_file = format!("{db_path}.key");
        let embedding = Box::new(MockEmbedding::new(384));
        Arc::new(NousServer::new(cfg, embedding, db_path, None).unwrap())
    }

    fn test_router_with_server(server: Arc<NousServer>) -> (Router, watch::Receiver<bool>) {
        let (tx, rx) = watch::channel(false);
        (daemon_router(tx, server), rx)
    }

    fn test_router() -> (Router, watch::Receiver<bool>, String) {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, rx) = test_router_with_server(server);
        (router, rx, db_path)
    }

    async fn json_body(resp: axum::http::Response<Body>) -> serde_json::Value {
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn status_returns_pid_and_uptime() {
        let (router, _rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let json = json_body(resp).await;
        assert_eq!(json["pid"], std::process::id());
        assert!(json["uptime_secs"].as_u64().is_some());
        assert!(json["version"].as_str().is_some());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn shutdown_triggers_watch_channel() {
        let (router, mut rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/shutdown")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let json = json_body(resp).await;
        assert_eq!(json["ok"], true);

        rx.changed().await.unwrap();
        assert!(*rx.borrow());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn create_and_list_rooms() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, _rx) = test_router_with_server(server);

        // Create room
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rooms")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "test-room", "purpose": "testing"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert_eq!(json["name"], "test-room");
        assert!(json["id"].as_str().is_some());

        // List rooms
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/rooms")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        let rooms = json["rooms"].as_array().unwrap();
        assert_eq!(rooms.len(), 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_room_by_name() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, _rx) = test_router_with_server(server);

        // Create room
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rooms")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "my-room"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Get by name
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/rooms/my-room")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert_eq!(json["name"], "my-room");

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn get_room_not_found_returns_404() {
        let (router, _rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/rooms/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn post_and_read_messages() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, _rx) = test_router_with_server(server);

        // Create room
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rooms")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "msg-room"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let room_json = json_body(resp).await;
        let room_id = room_json["id"].as_str().unwrap();

        // Post message
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/rooms/{room_id}/messages"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"content": "hello world", "sender": "alice"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let msg_json = json_body(resp).await;
        assert!(msg_json["id"].as_str().is_some());
        assert_eq!(msg_json["sender_id"], "alice");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Read messages
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/rooms/{room_id}/messages"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        let messages = json["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn store_and_search_memories() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, _rx) = test_router_with_server(server);

        // Store memory
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/memories/store")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Test Memory",
                            "content": "kubernetes deployment strategy for production",
                            "memory_type": "decision",
                            "tags": ["k8s"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert!(json["id"].as_str().is_some());

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Search memories
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/memories/search")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "query": "kubernetes",
                            "mode": "fts"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        let results = json["results"].as_array().unwrap();
        assert!(!results.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn store_memory_bad_type_returns_400() {
        let (router, _rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/memories/store")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Bad",
                            "content": "bad type",
                            "memory_type": "invalid_type"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn create_room_missing_name_returns_400() {
        let (router, _rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rooms")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn post_message_to_nonexistent_room_returns_404() {
        let (router, _rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rooms/nonexistent/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"content": "hello"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn list_categories_returns_ok() {
        let (router, _rx, db_path) = test_router();

        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/categories")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert!(json["categories"].is_array());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn export_returns_data() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, _rx) = test_router_with_server(server);

        // Store a memory first
        router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/memories/store")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "Export Test",
                            "content": "content for export",
                            "memory_type": "fact"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/export")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert_eq!(json["version"], 1);
        assert!(!json["memories"].as_array().unwrap().is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn import_round_trip() {
        let db_path = test_db_path();
        let server = test_server(&db_path);
        let (router, _rx) = test_router_with_server(server);

        let import_data = serde_json::json!({
            "version": 1,
            "memories": [{
                "id": "00000000-0000-0000-0000-000000000001",
                "title": "Imported Memory",
                "content": "imported content for testing",
                "memory_type": "fact",
                "source": null,
                "importance": "moderate",
                "confidence": "moderate",
                "session_id": null,
                "trace_id": null,
                "agent_id": null,
                "agent_model": null,
                "valid_from": null,
                "valid_until": null,
                "category_id": null,
                "created_at": "2024-01-01T00:00:00Z",
                "updated_at": "2024-01-01T00:00:00Z",
                "tags": ["imported"],
                "relationships": []
            }],
            "categories": []
        });

        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/import")
                    .header("content-type", "application/json")
                    .body(Body::from(import_data.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = json_body(resp).await;
        assert_eq!(json["status"], "imported");
        assert_eq!(json["memories_imported"], 1);

        let _ = std::fs::remove_file(&db_path);
    }
}
