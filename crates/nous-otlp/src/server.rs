use std::sync::{Arc, Mutex};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};

use crate::db::OtlpDb;
use crate::decode::{decode_logs, decode_metrics, decode_traces};

type SharedDb = Arc<Mutex<OtlpDb>>;

pub async fn run_server(db: OtlpDb, addr: std::net::SocketAddr) -> anyhow::Result<()> {
    let state: SharedDb = Arc::new(Mutex::new(db));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}

pub fn router(state: SharedDb) -> Router {
    Router::new()
        .route("/v1/logs", post(handle_logs))
        .route("/v1/traces", post(handle_traces))
        .route("/v1/metrics", post(handle_metrics))
        .with_state(state)
}

fn is_protobuf(headers: &HeaderMap) -> bool {
    headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/x-protobuf"))
}

async fn handle_logs(State(db): State<SharedDb>, headers: HeaderMap, body: Bytes) -> StatusCode {
    if !is_protobuf(&headers) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }
    let events = match decode_logs(&body) {
        Ok(e) => e,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let db = db.lock().unwrap();
    match db.store_logs(&events) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn handle_traces(State(db): State<SharedDb>, headers: HeaderMap, body: Bytes) -> StatusCode {
    if !is_protobuf(&headers) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }
    let spans = match decode_traces(&body) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let db = db.lock().unwrap();
    match db.store_spans(&spans) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn handle_metrics(State(db): State<SharedDb>, headers: HeaderMap, body: Bytes) -> StatusCode {
    if !is_protobuf(&headers) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE;
    }
    let metrics = match decode_metrics(&body) {
        Ok(m) => m,
        Err(_) => return StatusCode::BAD_REQUEST,
    };
    let db = db.lock().unwrap();
    match db.store_metrics(&metrics) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::{
        collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
        common::v1::{AnyValue, KeyValue, any_value},
        logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
        resource::v1::Resource,
        trace::v1::{ResourceSpans, ScopeSpans, Span as OtlpSpan},
    };
    use prost::Message;

    fn make_kv(key: &str, val: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(val.to_string())),
            }),
        }
    }

    async fn start_test_server() -> (String, SharedDb) {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let state: SharedDb = Arc::new(Mutex::new(db));
        let app = router(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        (format!("http://{addr}"), state)
    }

    #[tokio::test]
    async fn post_logs_returns_200_and_stores() {
        let (base_url, db) = start_test_server().await;

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![
                        make_kv("service.name", "test-svc"),
                        make_kv("session.id", "sess-test"),
                    ],
                    dropped_attributes_count: 0,
                }),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_700_000_000_000_000_000,
                        severity_number: 9,
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue(
                                "test log message".to_string(),
                            )),
                        }),
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base_url}/v1/logs"))
            .header("content-type", "application/x-protobuf")
            .body(buf)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);

        let db = db.lock().unwrap();
        let count: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM log_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn post_traces_returns_200_and_stores() {
        let (base_url, db) = start_test_server().await;

        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: Some(Resource {
                    attributes: vec![make_kv("service.name", "trace-svc")],
                    dropped_attributes_count: 0,
                }),
                scope_spans: vec![ScopeSpans {
                    scope: None,
                    spans: vec![OtlpSpan {
                        trace_id: vec![1; 16],
                        span_id: vec![2; 8],
                        name: "test-span".to_string(),
                        start_time_unix_nano: 1_000,
                        end_time_unix_nano: 2_000,
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base_url}/v1/traces"))
            .header("content-type", "application/x-protobuf")
            .body(buf)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);

        let db = db.lock().unwrap();
        let count: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM spans", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn wrong_content_type_returns_415() {
        let (base_url, _db) = start_test_server().await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base_url}/v1/logs"))
            .header("content-type", "application/json")
            .body(b"{}".to_vec())
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 415);
    }

    #[tokio::test]
    async fn malformed_protobuf_returns_400() {
        let (base_url, _db) = start_test_server().await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base_url}/v1/logs"))
            .header("content-type", "application/x-protobuf")
            .body(vec![0xff, 0xfe, 0xfd, 0xfc, 0xfb])
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 400);
    }

    #[tokio::test]
    async fn post_metrics_returns_200_and_stores() {
        use opentelemetry_proto::tonic::{
            collector::metrics::v1::ExportMetricsServiceRequest,
            metrics::v1::{
                Gauge, Metric as OtlpMetric, NumberDataPoint, ResourceMetrics, ScopeMetrics,
                metric, number_data_point,
            },
        };

        let (base_url, db) = start_test_server().await;

        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![make_kv("service.name", "metric-svc")],
                    dropped_attributes_count: 0,
                }),
                scope_metrics: vec![ScopeMetrics {
                    scope: None,
                    metrics: vec![OtlpMetric {
                        name: "test.gauge".to_string(),
                        description: String::new(),
                        unit: String::new(),
                        metadata: vec![],
                        data: Some(metric::Data::Gauge(Gauge {
                            data_points: vec![NumberDataPoint {
                                time_unix_nano: 1_000,
                                value: Some(number_data_point::Value::AsDouble(42.0)),
                                ..Default::default()
                            }],
                        })),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base_url}/v1/metrics"))
            .header("content-type", "application/x-protobuf")
            .body(buf)
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);

        let db = db.lock().unwrap();
        let count: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM metrics", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
