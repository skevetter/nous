use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use nous_core::config::RateLimitConfig;
use nous_daemon::rate_limit;
use tower::ServiceExt;

fn app(config: &RateLimitConfig) -> Router {
    let router = Router::new().route("/api", get(|| async { "ok" }));
    rate_limit::apply(config, router)
}

fn request_with_ip(ip: &str) -> Request<Body> {
    Request::builder()
        .uri("/api")
        .extension(axum::extract::ConnectInfo(
            ip.parse::<std::net::SocketAddr>().unwrap(),
        ))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn single_request_within_limit_succeeds() {
    let config = RateLimitConfig {
        requests_per_minute: 60,
        burst_size: 5,
    };
    let router = app(&config);

    let req = request_with_ip("127.0.0.1:1234");
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn burst_exceeded_returns_429() {
    let config = RateLimitConfig {
        requests_per_minute: 60,
        burst_size: 2,
    };

    let router = app(&config).into_service();
    let mut service = tower::ServiceBuilder::new().service(router);

    // First two requests should succeed (burst_size = 2)
    for _ in 0..2 {
        let req = request_with_ip("127.0.0.1:1234");
        let resp = tower::Service::call(&mut service, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Third request should be rate-limited
    let req = request_with_ip("127.0.0.1:1234");
    let resp = tower::Service::call(&mut service, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn different_ips_have_independent_limits() {
    let config = RateLimitConfig {
        requests_per_minute: 60,
        burst_size: 1,
    };

    let router = app(&config).into_service();
    let mut service = tower::ServiceBuilder::new().service(router);

    // First IP uses its burst
    let req = request_with_ip("10.0.0.1:1234");
    let resp = tower::Service::call(&mut service, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // First IP is now rate-limited
    let req = request_with_ip("10.0.0.1:1234");
    let resp = tower::Service::call(&mut service, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

    // Second IP should still work
    let req = request_with_ip("10.0.0.2:1234");
    let resp = tower::Service::call(&mut service, req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
