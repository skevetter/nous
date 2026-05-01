pub mod error;
pub mod llm_client;
pub mod process_manager;
pub mod routes;
pub mod scheduler;
pub mod state;

use axum::routing::{delete, get, post};
use axum::Router;

use state::AppState;

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(routes::health::get))
        .route("/rooms", post(routes::rooms::create))
        .route("/rooms", get(routes::rooms::list))
        .route("/rooms/{id}", get(routes::rooms::get))
        .route("/rooms/{id}", delete(routes::rooms::delete))
        .route("/rooms/{id}/messages", post(routes::messages::post))
        .route("/rooms/{id}/messages", get(routes::messages::read))
        .route("/search/messages", get(routes::search::search))
        .route(
            "/tasks",
            post(routes::tasks::create).get(routes::tasks::list),
        )
        .route(
            "/tasks/{id}",
            get(routes::tasks::get).put(routes::tasks::update),
        )
        .route("/tasks/{id}/close", post(routes::tasks::close))
        .route("/tasks/{id}/links", get(routes::tasks::list_links))
        .route("/tasks/{id}/note", post(routes::tasks::add_note))
        .route("/tasks/link", post(routes::tasks::link))
        .route("/tasks/unlink", post(routes::tasks::unlink))
        .route(
            "/worktrees",
            post(routes::worktrees::create).get(routes::worktrees::list),
        )
        .route(
            "/worktrees/{id}",
            get(routes::worktrees::get).delete(routes::worktrees::delete),
        )
        .route(
            "/worktrees/{id}/status",
            axum::routing::patch(routes::worktrees::update_status),
        )
        .route("/worktrees/{id}/archive", post(routes::worktrees::archive))
        .route(
            "/agents",
            post(routes::agents::register).get(routes::agents::list),
        )
        .route("/agents/tree", get(routes::agents::tree))
        .route("/agents/search", get(routes::agents::search))
        .route("/agents/stale", get(routes::agents::stale))
        .route(
            "/agents/{id}",
            get(routes::agents::get).delete(routes::agents::deregister),
        )
        .route("/agents/{id}/heartbeat", post(routes::agents::heartbeat))
        .route("/agents/{id}/children", get(routes::agents::children))
        .route("/agents/{id}/ancestors", get(routes::agents::ancestors))
        .route("/agents/{id}/inspect", get(routes::agents::inspect))
        .route("/agents/{id}/versions", get(routes::agents::list_versions))
        .route("/agents/{id}/rollback", post(routes::agents::rollback))
        .route(
            "/agents/{id}/notify-upgrade",
            post(routes::agents::notify_upgrade),
        )
        .route("/agents/versions", post(routes::agents::record_version))
        .route("/agents/outdated", get(routes::agents::list_outdated))
        .route(
            "/templates",
            post(routes::agents::create_template).get(routes::agents::list_templates),
        )
        .route("/templates/{id}", get(routes::agents::get_template))
        .route("/templates/instantiate", post(routes::agents::instantiate))
        .route(
            "/artifacts",
            post(routes::agents::register_artifact).get(routes::agents::list_artifacts),
        )
        .route(
            "/artifacts/{id}",
            delete(routes::agents::deregister_artifact),
        )
        .route(
            "/memories",
            post(routes::memory::save).get(routes::memory::context),
        )
        .route("/memories/search", get(routes::memory::search))
        .route("/memories/relate", post(routes::memory::relate))
        .route(
            "/memories/{id}",
            get(routes::memory::get).put(routes::memory::update),
        )
        .route(
            "/memories/{id}/relations",
            get(routes::memory::list_relations),
        )
        .route(
            "/inventory",
            post(routes::inventory::register).get(routes::inventory::list),
        )
        .route("/inventory/search", get(routes::inventory::search))
        .route(
            "/inventory/{id}",
            get(routes::inventory::get)
                .put(routes::inventory::update)
                .delete(routes::inventory::deregister),
        )
        .route("/inventory/{id}/archive", post(routes::inventory::archive))
        .route(
            "/schedules",
            post(routes::schedules::create).get(routes::schedules::list),
        )
        .route("/schedules/health", get(routes::schedules::health))
        .route(
            "/schedules/{id}",
            get(routes::schedules::get)
                .put(routes::schedules::update)
                .delete(routes::schedules::delete),
        )
        .route("/schedules/{id}/runs", get(routes::schedules::list_runs))
        .route("/mcp/tools", post(routes::mcp::list_tools))
        .route("/mcp/tools", get(routes::mcp::list_tools))
        .route("/mcp/call", post(routes::mcp::call_tool))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use nous_core::db::DbPools;
    use nous_core::memory::MockEmbedder;
    use nous_core::notifications::NotificationRegistry;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Notify;
    use tokio_util::sync::CancellationToken;
    use tower::ServiceExt;

    async fn test_state() -> (AppState, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();
        let state = AppState {
            pool: pools.fts.clone(),
            vec_pool: pools.vec.clone(),
            registry: Arc::new(NotificationRegistry::new()),
            embedder: Some(Arc::new(MockEmbedder::new())),
            schedule_notify: Arc::new(Notify::new()),
            shutdown: CancellationToken::new(),
            process_registry: Arc::new(process_manager::ProcessRegistry::new()),
        };
        (state, tmp)
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!({"status": "ok"}));
    }

    #[tokio::test]
    async fn create_room_returns_201() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/rooms")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "name": "test-room",
                            "purpose": "Testing"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "test-room");
        assert_eq!(json["purpose"], "Testing");
    }

    #[tokio::test]
    async fn list_rooms_returns_200() {
        let (state, _tmp) = test_state().await;
        let app = app(state.clone());

        nous_core::rooms::create_room(&state.pool, "room-a", None, None)
            .await
            .unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/rooms")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["name"], "room-a");
    }

    #[tokio::test]
    async fn get_room_returns_200() {
        let (state, _tmp) = test_state().await;
        let room = nous_core::rooms::create_room(&state.pool, "get-room", None, None)
            .await
            .unwrap();
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/rooms/{}", room.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["name"], "get-room");
    }

    #[tokio::test]
    async fn get_room_not_found_returns_404() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/rooms/nonexistent")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_room_returns_204() {
        let (state, _tmp) = test_state().await;
        let room = nous_core::rooms::create_room(&state.pool, "del-room", None, None)
            .await
            .unwrap();
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/rooms/{}", room.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn post_message_returns_201() {
        let (state, _tmp) = test_state().await;
        let room = nous_core::rooms::create_room(&state.pool, "msg-room", None, None)
            .await
            .unwrap();
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/rooms/{}/messages", room.id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "sender_id": "agent-1",
                            "content": "Hello!"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "Hello!");
        assert_eq!(json["sender_id"], "agent-1");
    }

    #[tokio::test]
    async fn read_messages_returns_200() {
        let (state, _tmp) = test_state().await;
        let room = nous_core::rooms::create_room(&state.pool, "read-room", None, None)
            .await
            .unwrap();

        nous_core::messages::post_message(
            &state.pool,
            nous_core::messages::PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Test message".into(),
                reply_to: None,
                metadata: None,
            },
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/rooms/{}/messages", room.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["content"], "Test message");
    }

    #[tokio::test]
    async fn search_messages_returns_200() {
        let (state, _tmp) = test_state().await;
        let room = nous_core::rooms::create_room(&state.pool, "search-room", None, None)
            .await
            .unwrap();

        nous_core::messages::post_message(
            &state.pool,
            nous_core::messages::PostMessageRequest {
                room_id: room.id.clone(),
                sender_id: "agent-1".into(),
                content: "Deploy completed successfully".into(),
                reply_to: None,
                metadata: None,
            },
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/search/messages?q=deploy")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert!(json[0]["content"].as_str().unwrap().contains("Deploy"));
    }

    #[tokio::test]
    async fn mcp_list_tools_returns_35_tools() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/tools")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let tools = json["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 105);
    }

    #[tokio::test]
    async fn mcp_call_room_create_returns_success() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/call")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "name": "room_create",
                            "arguments": { "name": "mcp-test-room", "purpose": "MCP test" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("is_error").is_none());
        let text = json["content"][0]["text"].as_str().unwrap();
        let room: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(room["name"], "mcp-test-room");
        assert_eq!(room["purpose"], "MCP test");
    }

    // --- Task HTTP route tests ---

    #[tokio::test]
    async fn test_create_task_returns_201() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "title": "Test task"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["title"], "Test task");
        assert_eq!(json["status"], "open");
        assert_eq!(json["priority"], "medium");
    }

    #[tokio::test]
    async fn test_list_tasks_returns_200() {
        let (state, _tmp) = test_state().await;

        nous_core::tasks::create_task(
            &state.pool,
            "Listed task",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/tasks")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(json[0]["title"], "Listed task");
    }

    #[tokio::test]
    async fn test_get_task_returns_200() {
        let (state, _tmp) = test_state().await;

        let task = nous_core::tasks::create_task(
            &state.pool,
            "Get me",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{}", task.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["title"], "Get me");
    }

    #[tokio::test]
    async fn test_get_task_not_found_returns_404() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/tasks/nonexistent-id")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_update_task_returns_200() {
        let (state, _tmp) = test_state().await;

        let task = nous_core::tasks::create_task(
            &state.pool,
            "Update me",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/tasks/{}", task.id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "status": "in_progress"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "in_progress");
    }

    #[tokio::test]
    async fn test_close_task_returns_200() {
        let (state, _tmp) = test_state().await;

        let task = nous_core::tasks::create_task(
            &state.pool,
            "Close me",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/{}/close", task.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "closed");
        assert!(json["closed_at"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_link_tasks_returns_201() {
        let (state, _tmp) = test_state().await;

        let t1 = nous_core::tasks::create_task(
            &state.pool,
            "Source",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        let t2 = nous_core::tasks::create_task(
            &state.pool,
            "Target",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks/link")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "source_id": t1.id,
                            "target_id": t2.id,
                            "link_type": "related_to"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn test_link_cycle_returns_409() {
        let (state, _tmp) = test_state().await;

        let t1 = nous_core::tasks::create_task(
            &state.pool,
            "A",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        let t2 = nous_core::tasks::create_task(
            &state.pool,
            "B",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        nous_core::tasks::link_tasks(&state.pool, &t1.id, &t2.id, "blocked_by", None)
            .await
            .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks/link")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "source_id": t2.id,
                            "target_id": t1.id,
                            "link_type": "blocked_by"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_unlink_tasks_returns_204() {
        let (state, _tmp) = test_state().await;

        let t1 = nous_core::tasks::create_task(
            &state.pool,
            "S",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        let t2 = nous_core::tasks::create_task(
            &state.pool,
            "T",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        nous_core::tasks::link_tasks(&state.pool, &t1.id, &t2.id, "related_to", None)
            .await
            .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/tasks/unlink")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "source_id": t1.id,
                            "target_id": t2.id,
                            "link_type": "related_to"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_list_links_returns_200() {
        let (state, _tmp) = test_state().await;

        let t1 = nous_core::tasks::create_task(
            &state.pool,
            "L1",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        let t2 = nous_core::tasks::create_task(
            &state.pool,
            "L2",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        nous_core::tasks::link_tasks(&state.pool, &t1.id, &t2.id, "parent", None)
            .await
            .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/tasks/{}/links", t1.id))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["parent"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_add_note_returns_201() {
        let (state, _tmp) = test_state().await;

        let task = nous_core::tasks::create_task(
            &state.pool,
            "Note task",
            None,
            None,
            None,
            None,
            None,
            true,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/tasks/{}/note", task.id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "sender_id": "agent-1",
                            "content": "A note"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["content"], "A note");
    }

    #[tokio::test]
    async fn test_mcp_task_create_returns_success() {
        let (state, _tmp) = test_state().await;
        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/call")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "name": "task_create",
                            "arguments": { "title": "MCP created task", "priority": "high" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("is_error").is_none());
        let text = json["content"][0]["text"].as_str().unwrap();
        let task: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(task["title"], "MCP created task");
        assert_eq!(task["priority"], "high");
    }

    #[tokio::test]
    async fn test_mcp_task_list_returns_success() {
        let (state, _tmp) = test_state().await;

        nous_core::tasks::create_task(
            &state.pool,
            "MCP listed",
            None,
            None,
            None,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();

        let app = app(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp/call")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "name": "task_list",
                            "arguments": {}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json.get("is_error").is_none());
        let text = json["content"][0]["text"].as_str().unwrap();
        let tasks: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "MCP listed");
    }
}
