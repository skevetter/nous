use std::sync::Arc;
use std::time::Instant;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

#[derive(Clone)]
pub struct AppState {
    shutdown_tx: watch::Sender<bool>,
    start_time: Instant,
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

#[derive(Serialize)]
struct StubResponse {
    error: &'static str,
}

pub fn daemon_router(shutdown_tx: watch::Sender<bool>) -> Router {
    let state = Arc::new(AppState {
        shutdown_tx,
        start_time: Instant::now(),
    });

    Router::new()
        .route("/status", get(handle_status))
        .route("/shutdown", post(handle_shutdown))
        .route("/rooms", post(stub_501))
        .route("/rooms", get(stub_501))
        .route("/rooms/{id}", get(stub_501))
        .route("/rooms/{id}/messages", post(stub_501))
        .route("/rooms/{id}/messages", get(stub_501))
        .route("/memories/search", post(stub_501))
        .route("/memories/store", post(stub_501))
        .route("/categories", get(stub_501))
        .route("/export", post(stub_501))
        .route("/import", post(stub_501))
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

async fn stub_501() -> impl IntoResponse {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(StubResponse {
            error: "not implemented",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_router() -> (Router, watch::Receiver<bool>) {
        let (tx, rx) = watch::channel(false);
        (daemon_router(tx), rx)
    }

    #[tokio::test]
    async fn status_returns_pid_and_uptime() {
        let (router, _rx) = test_router();

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

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(json["pid"], std::process::id());
        assert!(json["uptime_secs"].as_u64().is_some());
        assert!(json["version"].as_str().is_some());
    }

    #[tokio::test]
    async fn shutdown_triggers_watch_channel() {
        let (router, mut rx) = test_router();

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

        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true);

        rx.changed().await.unwrap();
        assert_eq!(*rx.borrow(), true);
    }

    #[tokio::test]
    async fn stub_endpoints_return_501() {
        let (router, _rx) = test_router();

        let stubs = vec![
            ("POST", "/rooms"),
            ("GET", "/rooms"),
            ("GET", "/rooms/123"),
            ("POST", "/rooms/123/messages"),
            ("GET", "/rooms/123/messages"),
            ("POST", "/memories/search"),
            ("POST", "/memories/store"),
            ("GET", "/categories"),
            ("POST", "/export"),
            ("POST", "/import"),
        ];

        for (method, path) in stubs {
            let resp = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(
                resp.status(),
                StatusCode::NOT_IMPLEMENTED,
                "{method} {path} should return 501"
            );
        }
    }
}
