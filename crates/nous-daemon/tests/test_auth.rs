use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::middleware;
use axum::routing::get;
use axum::Extension;
use axum::Router;
use http_body_util::BodyExt;
use nous_daemon::auth::{require_api_key, ApiKey};
use std::sync::Arc;
use tower::ServiceExt;

fn app(api_key: &str) -> Router {
    Router::new()
        .route("/protected", get(|| async { "ok" }))
        .layer(middleware::from_fn(require_api_key))
        .layer(Extension(ApiKey(Arc::from(api_key))))
}

async fn response_status(app: Router, req: Request<Body>) -> StatusCode {
    app.oneshot(req).await.unwrap().status()
}

async fn response_body(app: Router, req: Request<Body>) -> String {
    let resp = app.oneshot(req).await.unwrap();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn valid_bearer_token_passes() {
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "Bearer secret-key")
        .body(Body::empty())
        .unwrap();

    let status = response_status(app("secret-key"), req).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn missing_auth_header_returns_401() {
    let req = Request::builder()
        .uri("/protected")
        .body(Body::empty())
        .unwrap();

    let status = response_status(app("secret-key"), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn missing_auth_header_error_body() {
    let req = Request::builder()
        .uri("/protected")
        .body(Body::empty())
        .unwrap();

    let body = response_body(app("secret-key"), req).await;
    assert!(body.contains("missing authorization header"));
}

#[tokio::test]
async fn wrong_token_returns_401() {
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "Bearer wrong-key")
        .body(Body::empty())
        .unwrap();

    let status = response_status(app("secret-key"), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn wrong_token_error_body() {
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "Bearer wrong-key")
        .body(Body::empty())
        .unwrap();

    let body = response_body(app("secret-key"), req).await;
    assert!(body.contains("invalid API key"));
}

#[tokio::test]
async fn non_bearer_scheme_returns_401() {
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "Basic dXNlcjpwYXNz")
        .body(Body::empty())
        .unwrap();

    let status = response_status(app("secret-key"), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_bearer_scheme_error_body() {
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "Basic dXNlcjpwYXNz")
        .body(Body::empty())
        .unwrap();

    let body = response_body(app("secret-key"), req).await;
    assert!(body.contains("Bearer scheme"));
}

#[tokio::test]
async fn empty_bearer_token_returns_401() {
    let req = Request::builder()
        .uri("/protected")
        .header("authorization", "Bearer ")
        .body(Body::empty())
        .unwrap();

    let status = response_status(app("secret-key"), req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
