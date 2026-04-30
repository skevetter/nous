pub mod error;
mod routes;
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
    use nous_core::notifications::NotificationRegistry;
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
                    .uri(&format!("/rooms/{}", room.id))
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
                    .uri(&format!("/rooms/{}", room.id))
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
                    .uri(&format!("/rooms/{}/messages", room.id))
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
                    .uri(&format!("/rooms/{}/messages", room.id))
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
}
