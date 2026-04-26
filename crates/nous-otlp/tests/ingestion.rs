use std::sync::{Arc, Mutex};
use std::time::Instant;

use nous_otlp::db::OtlpDb;
use nous_otlp::server::router;
use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    common::v1::{AnyValue, KeyValue, any_value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    metrics::v1::{
        Gauge, Metric as OtlpMetric, NumberDataPoint, ResourceMetrics, ScopeMetrics, metric,
        number_data_point,
    },
    resource::v1::Resource,
    trace::v1::{ResourceSpans, ScopeSpans, Span as OtlpSpan},
};
use prost::Message;

type SharedDb = Arc<Mutex<OtlpDb>>;

fn make_kv(key: &str, val: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(val.to_string())),
        }),
    }
}

fn make_resource(session_id: &str, service_name: &str) -> Option<Resource> {
    Some(Resource {
        attributes: vec![
            make_kv("service.name", service_name),
            make_kv("session.id", session_id),
        ],
        dropped_attributes_count: 0,
    })
}

fn build_log_payload(batch_idx: usize) -> (Vec<u8>, usize) {
    let session_id = format!("log-sess-{batch_idx}");
    let num_records = if batch_idx % 2 == 0 { 3 } else { 2 };

    let records: Vec<LogRecord> = (0..num_records)
        .map(|i| LogRecord {
            time_unix_nano: (batch_idx * 1000 + i) as u64,
            severity_number: 9,
            body: Some(AnyValue {
                value: Some(any_value::Value::StringValue(format!(
                    "log-batch{batch_idx}-event{i}"
                ))),
            }),
            ..Default::default()
        })
        .collect();

    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: make_resource(&session_id, &format!("log-svc-{batch_idx}")),
            scope_logs: vec![ScopeLogs {
                scope: None,
                log_records: records,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };

    let mut buf = Vec::new();
    request.encode(&mut buf).unwrap();
    (buf, num_records)
}

fn build_trace_payload(batch_idx: usize) -> (Vec<u8>, usize) {
    let trace_id_byte = (batch_idx + 100) as u8;
    let num_spans = if batch_idx % 2 == 0 { 2 } else { 1 };

    let spans: Vec<OtlpSpan> = (0..num_spans)
        .map(|i| {
            let mut span_id = vec![0u8; 8];
            span_id[0] = trace_id_byte;
            span_id[1] = i as u8;

            OtlpSpan {
                trace_id: vec![trace_id_byte; 16],
                span_id,
                name: format!("trace-batch{batch_idx}-span{i}"),
                start_time_unix_nano: (batch_idx * 1000 + i) as u64,
                end_time_unix_nano: (batch_idx * 1000 + i + 500) as u64,
                ..Default::default()
            }
        })
        .collect();

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: Some(Resource {
                attributes: vec![make_kv("service.name", &format!("trace-svc-{batch_idx}"))],
                dropped_attributes_count: 0,
            }),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };

    let mut buf = Vec::new();
    request.encode(&mut buf).unwrap();
    (buf, num_spans)
}

fn build_metric_payload(batch_idx: usize) -> (Vec<u8>, usize) {
    let num_metrics = if batch_idx % 2 == 0 { 2 } else { 1 };

    let metrics: Vec<OtlpMetric> = (0..num_metrics)
        .map(|i| OtlpMetric {
            name: format!("metric-batch{batch_idx}-gauge{i}"),
            description: String::new(),
            unit: String::new(),
            metadata: vec![],
            data: Some(metric::Data::Gauge(Gauge {
                data_points: vec![NumberDataPoint {
                    time_unix_nano: (batch_idx * 1000 + i) as u64,
                    value: Some(number_data_point::Value::AsDouble(
                        batch_idx as f64 + i as f64,
                    )),
                    ..Default::default()
                }],
            })),
        })
        .collect();

    let request = ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: Some(Resource {
                attributes: vec![make_kv("service.name", &format!("metric-svc-{batch_idx}"))],
                dropped_attributes_count: 0,
            }),
            scope_metrics: vec![ScopeMetrics {
                scope: None,
                metrics,
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };

    let mut buf = Vec::new();
    request.encode(&mut buf).unwrap();
    (buf, num_metrics)
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

enum Payload {
    Logs(Vec<u8>),
    Traces(Vec<u8>),
    Metrics(Vec<u8>),
}

impl Payload {
    fn endpoint(&self) -> &'static str {
        match self {
            Payload::Logs(_) => "/v1/logs",
            Payload::Traces(_) => "/v1/traces",
            Payload::Metrics(_) => "/v1/metrics",
        }
    }

    fn body(&self) -> &[u8] {
        match self {
            Payload::Logs(b) | Payload::Traces(b) | Payload::Metrics(b) => b,
        }
    }
}

#[tokio::test]
async fn concurrent_ingestion_20_requests() {
    let (base_url, db) = start_test_server().await;

    let mut payloads: Vec<Payload> = Vec::with_capacity(20);
    let mut expected_logs = 0usize;
    let mut expected_spans = 0usize;
    let mut expected_metrics = 0usize;

    // 10 log batches
    for i in 0..10 {
        let (buf, count) = build_log_payload(i);
        expected_logs += count;
        payloads.push(Payload::Logs(buf));
    }
    // 5 trace batches
    for i in 0..5 {
        let (buf, count) = build_trace_payload(i);
        expected_spans += count;
        payloads.push(Payload::Traces(buf));
    }
    // 5 metric batches
    for i in 0..5 {
        let (buf, count) = build_metric_payload(i);
        expected_metrics += count;
        payloads.push(Payload::Metrics(buf));
    }

    assert_eq!(payloads.len(), 20);

    let client = reqwest::Client::new();
    let start = Instant::now();

    let mut handles = Vec::with_capacity(20);
    for payload in payloads {
        let client = client.clone();
        let base_url = base_url.clone();
        let endpoint = payload.endpoint().to_string();
        let body = payload.body().to_vec();

        handles.push(tokio::spawn(async move {
            client
                .post(format!("{base_url}{endpoint}"))
                .header("content-type", "application/x-protobuf")
                .body(body)
                .send()
                .await
        }));
    }

    let mut statuses = Vec::with_capacity(20);
    for handle in handles {
        let resp = handle.await.unwrap().unwrap();
        statuses.push(resp.status().as_u16());
    }

    let elapsed = start.elapsed();

    // All 20 requests return 200
    for (i, status) in statuses.iter().enumerate() {
        assert_eq!(*status, 200, "request {i} returned {status}");
    }

    // Timing: all requests complete within 5 seconds
    assert!(
        elapsed.as_secs() < 5,
        "concurrent requests took {elapsed:?}, expected < 5s"
    );

    // Verify row counts in each table
    let db = db.lock().unwrap();
    let conn = db.connection();

    let log_count: i64 = conn
        .query_row("SELECT count(*) FROM log_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        log_count, expected_logs as i64,
        "log_events: expected {expected_logs}, got {log_count}"
    );

    let span_count: i64 = conn
        .query_row("SELECT count(*) FROM spans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        span_count, expected_spans as i64,
        "spans: expected {expected_spans}, got {span_count}"
    );

    let metric_count: i64 = conn
        .query_row("SELECT count(*) FROM metrics", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        metric_count, expected_metrics as i64,
        "metrics: expected {expected_metrics}, got {metric_count}"
    );

    // Verify no duplicate log bodies
    let distinct_log_bodies: i64 = conn
        .query_row("SELECT count(DISTINCT body) FROM log_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        distinct_log_bodies, expected_logs as i64,
        "duplicate log bodies detected"
    );

    // Verify no duplicate span names
    let distinct_span_names: i64 = conn
        .query_row("SELECT count(DISTINCT name) FROM spans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        distinct_span_names, expected_spans as i64,
        "duplicate span names detected"
    );

    // Verify no duplicate metric names
    let distinct_metric_names: i64 = conn
        .query_row("SELECT count(DISTINCT name) FROM metrics", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        distinct_metric_names, expected_metrics as i64,
        "duplicate metric names detected"
    );

    // Verify session isolation: each log session has expected count
    for i in 0..10 {
        let session_id = format!("log-sess-{i}");
        let expected = if i % 2 == 0 { 3i64 } else { 2i64 };
        let actual: i64 = conn
            .query_row(
                "SELECT count(*) FROM log_events WHERE session_id = ?1",
                [&session_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            actual, expected,
            "session {session_id}: expected {expected}, got {actual}"
        );
    }

    // Verify trace isolation: each trace_id has expected span count
    for i in 0..5 {
        let trace_id_byte = (i + 100) as u8;
        let trace_id_hex = format!(
            "{}",
            std::iter::repeat_n(format!("{trace_id_byte:02x}"), 16).collect::<String>()
        );
        let expected = if i % 2 == 0 { 2i64 } else { 1i64 };
        let actual: i64 = conn
            .query_row(
                "SELECT count(*) FROM spans WHERE trace_id = ?1",
                [&trace_id_hex],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            actual, expected,
            "trace {trace_id_hex}: expected {expected}, got {actual}"
        );
    }
}
