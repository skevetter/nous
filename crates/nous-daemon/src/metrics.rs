use axum::extract::MatchedPath;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use metrics::{counter, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::time::Instant;

use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn setup() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            PrometheusBuilder::new()
                .install_recorder()
                .expect("failed to install Prometheus recorder")
        })
        .clone()
}

pub async fn track(request: Request, next: Next) -> Response {
    let path = request
        .extensions()
        .get::<MatchedPath>().map_or_else(|| request.uri().path().to_owned(), |p| p.as_str().to_owned());
    let method = request.method().to_string();

    let start = Instant::now();
    let response = next.run(request).await;
    let duration = start.elapsed().as_secs_f64();

    let status = response.status().as_u16().to_string();
    let labels = [
        ("method", method),
        ("path", path),
        ("status", status.clone()),
    ];

    counter!("http_requests_total", &labels).increment(1);
    histogram!("http_request_duration_seconds", &labels).record(duration);

    if response.status().is_server_error() {
        counter!(
            "http_errors_total",
            &[("method", labels[0].1.clone()), ("path", labels[1].1.clone())]
        )
        .increment(1);
    }

    response
}

pub async fn render(
    axum::Extension(handle): axum::Extension<PrometheusHandle>,
) -> impl IntoResponse {
    handle.render()
}
