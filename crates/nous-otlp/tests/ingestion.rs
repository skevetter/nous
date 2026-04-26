use std::sync::{Arc, Mutex};

use nous_otlp::{db::OtlpDb, server::router};
use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    common::v1::{AnyValue, KeyValue, any_value},
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    metrics::v1::{Gauge, Metric as OtlpMetric, NumberDataPoint, ResourceMetrics, ScopeMetrics},
    resource::v1::Resource,
    trace::v1::{ResourceSpans, ScopeSpans, Span as OtlpSpan},
};
use prost::Message;
use tokio::task::JoinSet;

fn make_kv(key: &str, val: &str) -> KeyValue {
    KeyValue {
        key: key.to_string(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(val.to_string())),
        }),
    }
}

fn make_resource(service_name: &str) -> Option<Resource> {
    Some(Resource {
        attributes: vec![make_kv("service.name", service_name)],
        dropped_attributes_count: 0,
    })
}

fn encode_log_request(index: usize) -> Vec<u8> {
    let request = ExportLogsServiceRequest {
        resource_logs: vec![ResourceLogs {
            resource: make_resource(&format!("log-svc-{index}")),
            scope_logs: vec![ScopeLogs {
                scope: None,
                log_records: vec![LogRecord {
                    time_unix_nano: 1_700_000_000_000_000_000 + index as u64,
                    severity_number: 9,
                    body: Some(AnyValue {
                        value: Some(any_value::Value::StringValue(format!(
                            "concurrent log {index}"
                        ))),
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
    buf
}

fn encode_trace_request(index: usize) -> Vec<u8> {
    let mut trace_id = vec![0u8; 16];
    trace_id[0] = index as u8;
    trace_id[15] = 0xff;
    let mut span_id = vec![0u8; 8];
    span_id[0] = index as u8;

    let request = ExportTraceServiceRequest {
        resource_spans: vec![ResourceSpans {
            resource: make_resource(&format!("trace-svc-{index}")),
            scope_spans: vec![ScopeSpans {
                scope: None,
                spans: vec![OtlpSpan {
                    trace_id,
                    span_id,
                    name: format!("concurrent span {index}"),
                    start_time_unix_nano: 1_000 + index as u64,
                    end_time_unix_nano: 2_000 + index as u64,
                    ..Default::default()
                }],
                schema_url: String::new(),
            }],
            schema_url: String::new(),
        }],
    };
    let mut buf = Vec::new();
    request.encode(&mut buf).unwrap();
    buf
}

fn encode_metric_request(index: usize) -> Vec<u8> {
    use opentelemetry_proto::tonic::metrics::v1::{metric, number_data_point};

    let request = ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            resource: make_resource(&format!("metric-svc-{index}")),
            scope_metrics: vec![ScopeMetrics {
                scope: None,
                metrics: vec![OtlpMetric {
                    name: format!("concurrent.gauge.{index}"),
                    description: String::new(),
                    unit: String::new(),
                    metadata: vec![],
                    data: Some(metric::Data::Gauge(Gauge {
                        data_points: vec![NumberDataPoint {
                            time_unix_nano: 3_000 + index as u64,
                            value: Some(number_data_point::Value::AsDouble(index as f64)),
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
    buf
}

#[tokio::test]
async fn concurrent_ingestion_no_loss_no_duplication() {
    let db = OtlpDb::open(":memory:", None).unwrap();
    let state: Arc<Mutex<OtlpDb>> = Arc::new(Mutex::new(db));
    let app = router(Arc::clone(&state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    let base_url = format!("http://{addr}");

    let client = reqwest::Client::new();
    let mut join_set = JoinSet::new();

    let expected_logs = 7;
    let expected_traces = 7;
    let expected_metrics = 6;

    for i in 0..expected_logs {
        let url = format!("{base_url}/v1/logs");
        let c = client.clone();
        let body = encode_log_request(i);
        join_set.spawn(async move {
            c.post(&url)
                .header("content-type", "application/x-protobuf")
                .body(body)
                .send()
                .await
        });
    }

    for i in 0..expected_traces {
        let url = format!("{base_url}/v1/traces");
        let c = client.clone();
        let body = encode_trace_request(i);
        join_set.spawn(async move {
            c.post(&url)
                .header("content-type", "application/x-protobuf")
                .body(body)
                .send()
                .await
        });
    }

    for i in 0..expected_metrics {
        let url = format!("{base_url}/v1/metrics");
        let c = client.clone();
        let body = encode_metric_request(i);
        join_set.spawn(async move {
            c.post(&url)
                .header("content-type", "application/x-protobuf")
                .body(body)
                .send()
                .await
        });
    }

    let mut statuses = Vec::new();
    while let Some(result) = join_set.join_next().await {
        let resp = result.unwrap().unwrap();
        statuses.push(resp.status());
    }

    assert_eq!(statuses.len(), 20);
    for (i, status) in statuses.iter().enumerate() {
        assert_eq!(
            status.as_u16(),
            200,
            "request {i} returned {status}, expected 200"
        );
    }

    let db = state.lock().unwrap();
    let conn = db.connection();

    let log_count: i64 = conn
        .query_row("SELECT count(*) FROM log_events", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        log_count, expected_logs as i64,
        "expected {expected_logs} log rows, got {log_count}"
    );

    let span_count: i64 = conn
        .query_row("SELECT count(*) FROM spans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        span_count, expected_traces as i64,
        "expected {expected_traces} span rows, got {span_count}"
    );

    let metric_count: i64 = conn
        .query_row("SELECT count(*) FROM metrics", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        metric_count, expected_metrics as i64,
        "expected {expected_metrics} metric rows, got {metric_count}"
    );

    let distinct_log_bodies: i64 = conn
        .query_row("SELECT count(DISTINCT body) FROM log_events", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        distinct_log_bodies, expected_logs as i64,
        "duplicate log bodies detected"
    );

    let distinct_span_names: i64 = conn
        .query_row("SELECT count(DISTINCT name) FROM spans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        distinct_span_names, expected_traces as i64,
        "duplicate span names detected"
    );

    let distinct_metric_names: i64 = conn
        .query_row("SELECT count(DISTINCT name) FROM metrics", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        distinct_metric_names, expected_metrics as i64,
        "duplicate metric names detected"
    );

    let sample_body: String = conn
        .query_row(
            "SELECT body FROM log_events WHERE body = ?1",
            ["concurrent log 0"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sample_body, "concurrent log 0");

    let sample_span: String = conn
        .query_row(
            "SELECT name FROM spans WHERE name = ?1",
            ["concurrent span 3"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sample_span, "concurrent span 3");

    let sample_metric: String = conn
        .query_row(
            "SELECT name FROM metrics WHERE name = ?1",
            ["concurrent.gauge.2"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sample_metric, "concurrent.gauge.2");
}
