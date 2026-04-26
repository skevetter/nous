use opentelemetry_proto::tonic::{
    collector::{
        logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
        trace::v1::ExportTraceServiceRequest,
    },
    common::v1::{AnyValue, KeyValue, any_value},
};
use prost::Message;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("protobuf decode failed: {0}")]
    Prost(#[from] prost::DecodeError),
    #[error("json failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, DecodeError>;

#[derive(Debug, Clone, PartialEq)]
pub struct LogEvent {
    pub timestamp: i64,
    pub severity: String,
    pub body: String,
    pub resource_attrs: String,
    pub scope_attrs: String,
    pub log_attrs: String,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub span_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: i32,
    pub start_time: i64,
    pub end_time: i64,
    pub status_code: i32,
    pub status_message: Option<String>,
    pub resource_attrs: String,
    pub span_attrs: String,
    pub events_json: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Metric {
    pub name: String,
    pub description: Option<String>,
    pub unit: Option<String>,
    pub metric_type: String,
    pub data_points_json: String,
    pub resource_attrs: String,
    pub timestamp: i64,
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn non_empty_hex(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.iter().all(|&b| b == 0) {
        None
    } else {
        Some(hex_encode(bytes))
    }
}

fn any_value_to_json(v: &AnyValue) -> serde_json::Value {
    match &v.value {
        Some(any_value::Value::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(any_value::Value::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(any_value::Value::IntValue(i)) => serde_json::json!(*i),
        Some(any_value::Value::DoubleValue(d)) => serde_json::json!(*d),
        Some(any_value::Value::ArrayValue(arr)) => {
            serde_json::Value::Array(arr.values.iter().map(any_value_to_json).collect())
        }
        Some(any_value::Value::KvlistValue(kv)) => {
            let map: serde_json::Map<String, serde_json::Value> = kv
                .values
                .iter()
                .map(|kv| {
                    let val = kv
                        .value
                        .as_ref()
                        .map(any_value_to_json)
                        .unwrap_or(serde_json::Value::Null);
                    (kv.key.clone(), val)
                })
                .collect();
            serde_json::Value::Object(map)
        }
        Some(any_value::Value::BytesValue(b)) => serde_json::Value::String(hex_encode(b)),
        None => serde_json::Value::Null,
    }
}

fn attrs_to_json(attrs: &[KeyValue]) -> String {
    let map: serde_json::Map<String, serde_json::Value> = attrs
        .iter()
        .map(|kv| {
            let val = kv
                .value
                .as_ref()
                .map(any_value_to_json)
                .unwrap_or(serde_json::Value::Null);
            (kv.key.clone(), val)
        })
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}

fn find_string_attr(attrs: &[KeyValue], key: &str) -> Option<String> {
    attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
        kv.value.as_ref().and_then(|v| match &v.value {
            Some(any_value::Value::StringValue(s)) => Some(s.clone()),
            _ => None,
        })
    })
}

fn severity_number_to_string(n: i32) -> String {
    match n {
        0 => "UNSPECIFIED".to_string(),
        1..=4 => "TRACE".to_string(),
        5..=8 => "DEBUG".to_string(),
        9..=12 => "INFO".to_string(),
        13..=16 => "WARN".to_string(),
        17..=20 => "ERROR".to_string(),
        21..=24 => "FATAL".to_string(),
        _ => format!("UNKNOWN({n})"),
    }
}

pub fn decode_logs_json(body: &[u8]) -> Result<Vec<LogEvent>> {
    let request: ExportLogsServiceRequest = serde_json::from_slice(body)?;
    Ok(convert_logs(&request))
}

pub fn decode_logs(body: &[u8]) -> Result<Vec<LogEvent>> {
    let request = ExportLogsServiceRequest::decode(body)?;
    Ok(convert_logs(&request))
}

fn convert_logs(request: &ExportLogsServiceRequest) -> Vec<LogEvent> {
    let mut events = Vec::new();

    for rl in &request.resource_logs {
        let resource_attrs_json = rl
            .resource
            .as_ref()
            .map(|r| attrs_to_json(&r.attributes))
            .unwrap_or_else(|| "{}".to_string());

        let resource_kvs = rl
            .resource
            .as_ref()
            .map(|r| r.attributes.as_slice())
            .unwrap_or_default();
        let session_id = find_string_attr(resource_kvs, "session.id");

        for sl in &rl.scope_logs {
            let scope_attrs_json = sl
                .scope
                .as_ref()
                .map(|s| attrs_to_json(&s.attributes))
                .unwrap_or_else(|| "{}".to_string());

            for lr in &sl.log_records {
                let body_str = lr
                    .body
                    .as_ref()
                    .map(|v| match &v.value {
                        Some(any_value::Value::StringValue(s)) => s.clone(),
                        Some(other) => {
                            let av = AnyValue {
                                value: Some(other.clone()),
                            };
                            serde_json::to_string(&any_value_to_json(&av)).unwrap_or_default()
                        }
                        None => String::new(),
                    })
                    .unwrap_or_default();

                events.push(LogEvent {
                    timestamp: lr.time_unix_nano as i64,
                    severity: severity_number_to_string(lr.severity_number),
                    body: body_str,
                    resource_attrs: resource_attrs_json.clone(),
                    scope_attrs: scope_attrs_json.clone(),
                    log_attrs: attrs_to_json(&lr.attributes),
                    session_id: session_id.clone(),
                    trace_id: non_empty_hex(&lr.trace_id),
                    span_id: non_empty_hex(&lr.span_id),
                });
            }
        }
    }

    events
}

pub fn decode_traces_json(body: &[u8]) -> Result<Vec<Span>> {
    let request: ExportTraceServiceRequest = serde_json::from_slice(body)?;
    Ok(convert_traces(&request))
}

pub fn decode_traces(body: &[u8]) -> Result<Vec<Span>> {
    let request = ExportTraceServiceRequest::decode(body)?;
    Ok(convert_traces(&request))
}

fn convert_traces(request: &ExportTraceServiceRequest) -> Vec<Span> {
    let mut spans = Vec::new();

    for rs in &request.resource_spans {
        let resource_attrs_json = rs
            .resource
            .as_ref()
            .map(|r| attrs_to_json(&r.attributes))
            .unwrap_or_else(|| "{}".to_string());

        for ss in &rs.scope_spans {
            for span in &ss.spans {
                let events_json = {
                    let evts: Vec<serde_json::Value> = span
                        .events
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "time_unix_nano": e.time_unix_nano,
                                "name": e.name,
                                "attributes": serde_json::from_str::<serde_json::Value>(&attrs_to_json(&e.attributes)).unwrap_or_default(),
                            })
                        })
                        .collect();
                    serde_json::to_string(&evts).unwrap_or_else(|_| "[]".to_string())
                };

                let parent = non_empty_hex(&span.parent_span_id);

                let (status_code, status_message) = span
                    .status
                    .as_ref()
                    .map(|s| {
                        let msg = if s.message.is_empty() {
                            None
                        } else {
                            Some(s.message.clone())
                        };
                        (s.code, msg)
                    })
                    .unwrap_or((0, None));

                spans.push(Span {
                    trace_id: hex_encode(&span.trace_id),
                    span_id: hex_encode(&span.span_id),
                    parent_span_id: parent,
                    name: span.name.clone(),
                    kind: span.kind,
                    start_time: span.start_time_unix_nano as i64,
                    end_time: span.end_time_unix_nano as i64,
                    status_code,
                    status_message,
                    resource_attrs: resource_attrs_json.clone(),
                    span_attrs: attrs_to_json(&span.attributes),
                    events_json,
                });
            }
        }
    }

    spans
}

#[derive(Serialize)]
struct DataPointJson {
    time_unix_nano: u64,
    start_time_unix_nano: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    value_double: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value_int: Option<i64>,
    attributes: serde_json::Value,
}

fn extract_metric_data(
    m: &opentelemetry_proto::tonic::metrics::v1::Metric,
) -> (String, String, i64) {
    use opentelemetry_proto::tonic::metrics::v1::metric::Data;
    use opentelemetry_proto::tonic::metrics::v1::number_data_point;

    match &m.data {
        Some(Data::Gauge(g)) => {
            let points: Vec<DataPointJson> = g
                .data_points
                .iter()
                .map(|dp| DataPointJson {
                    time_unix_nano: dp.time_unix_nano,
                    start_time_unix_nano: dp.start_time_unix_nano,
                    value_double: match &dp.value {
                        Some(number_data_point::Value::AsDouble(d)) => Some(*d),
                        _ => None,
                    },
                    value_int: match &dp.value {
                        Some(number_data_point::Value::AsInt(i)) => Some(*i),
                        _ => None,
                    },
                    attributes: serde_json::from_str(&attrs_to_json(&dp.attributes))
                        .unwrap_or_default(),
                })
                .collect();
            let ts = g
                .data_points
                .first()
                .map(|dp| dp.time_unix_nano as i64)
                .unwrap_or(0);
            (
                "gauge".to_string(),
                serde_json::to_string(&points).unwrap_or_else(|_| "[]".to_string()),
                ts,
            )
        }
        Some(Data::Sum(s)) => {
            let points: Vec<DataPointJson> = s
                .data_points
                .iter()
                .map(|dp| DataPointJson {
                    time_unix_nano: dp.time_unix_nano,
                    start_time_unix_nano: dp.start_time_unix_nano,
                    value_double: match &dp.value {
                        Some(number_data_point::Value::AsDouble(d)) => Some(*d),
                        _ => None,
                    },
                    value_int: match &dp.value {
                        Some(number_data_point::Value::AsInt(i)) => Some(*i),
                        _ => None,
                    },
                    attributes: serde_json::from_str(&attrs_to_json(&dp.attributes))
                        .unwrap_or_default(),
                })
                .collect();
            let ts = s
                .data_points
                .first()
                .map(|dp| dp.time_unix_nano as i64)
                .unwrap_or(0);
            (
                "sum".to_string(),
                serde_json::to_string(&points).unwrap_or_else(|_| "[]".to_string()),
                ts,
            )
        }
        Some(Data::Histogram(h)) => {
            let ts = h
                .data_points
                .first()
                .map(|dp| dp.time_unix_nano as i64)
                .unwrap_or(0);
            (
                "histogram".to_string(),
                serde_json::to_string(&h.data_points).unwrap_or_else(|_| "[]".to_string()),
                ts,
            )
        }
        Some(Data::ExponentialHistogram(eh)) => {
            let ts = eh
                .data_points
                .first()
                .map(|dp| dp.time_unix_nano as i64)
                .unwrap_or(0);
            (
                "exponential_histogram".to_string(),
                serde_json::to_string(&eh.data_points).unwrap_or_else(|_| "[]".to_string()),
                ts,
            )
        }
        Some(Data::Summary(s)) => {
            let ts = s
                .data_points
                .first()
                .map(|dp| dp.time_unix_nano as i64)
                .unwrap_or(0);
            (
                "summary".to_string(),
                serde_json::to_string(&s.data_points).unwrap_or_else(|_| "[]".to_string()),
                ts,
            )
        }
        None => ("unknown".to_string(), "[]".to_string(), 0),
    }
}

pub fn decode_metrics_json(body: &[u8]) -> Result<Vec<Metric>> {
    let request: ExportMetricsServiceRequest = serde_json::from_slice(body)?;
    Ok(convert_metrics(&request))
}

pub fn decode_metrics(body: &[u8]) -> Result<Vec<Metric>> {
    let request = ExportMetricsServiceRequest::decode(body)?;
    Ok(convert_metrics(&request))
}

fn convert_metrics(request: &ExportMetricsServiceRequest) -> Vec<Metric> {
    let mut metrics = Vec::new();

    for rm in &request.resource_metrics {
        let resource_attrs_json = rm
            .resource
            .as_ref()
            .map(|r| attrs_to_json(&r.attributes))
            .unwrap_or_else(|| "{}".to_string());

        for sm in &rm.scope_metrics {
            for m in &sm.metrics {
                let (metric_type, data_points_json, timestamp) = extract_metric_data(m);

                metrics.push(Metric {
                    name: m.name.clone(),
                    description: if m.description.is_empty() {
                        None
                    } else {
                        Some(m.description.clone())
                    },
                    unit: if m.unit.is_empty() {
                        None
                    } else {
                        Some(m.unit.clone())
                    },
                    metric_type,
                    data_points_json,
                    resource_attrs: resource_attrs_json.clone(),
                    timestamp,
                });
            }
        }
    }

    metrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::{
        collector::{
            logs::v1::ExportLogsServiceRequest, metrics::v1::ExportMetricsServiceRequest,
            trace::v1::ExportTraceServiceRequest,
        },
        common::v1::{AnyValue, InstrumentationScope, KeyValue, any_value},
        logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
        metrics::v1::{
            Gauge, Metric as OtlpMetric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
            metric, number_data_point,
        },
        resource::v1::Resource,
        trace::v1::{
            ResourceSpans, ScopeSpans, Span as OtlpSpan, Status,
            span::{Event, SpanKind},
            status::StatusCode,
        },
    };

    fn make_kv(key: &str, val: &str) -> KeyValue {
        KeyValue {
            key: key.to_string(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(val.to_string())),
            }),
        }
    }

    fn make_resource(attrs: Vec<KeyValue>) -> Option<Resource> {
        Some(Resource {
            attributes: attrs,
            dropped_attributes_count: 0,
        })
    }

    fn make_scope(name: &str) -> Option<InstrumentationScope> {
        Some(InstrumentationScope {
            name: name.to_string(),
            version: "1.0".to_string(),
            attributes: vec![make_kv("scope.key", "scope.val")],
            dropped_attributes_count: 0,
        })
    }

    #[test]
    fn decode_logs_round_trip() {
        let trace_id = vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ];
        let span_id = vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x11, 0x22];

        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: make_resource(vec![
                    make_kv("service.name", "test-svc"),
                    make_kv("session.id", "sess-123"),
                ]),
                scope_logs: vec![ScopeLogs {
                    scope: make_scope("my-lib"),
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_700_000_000_000_000_000,
                        observed_time_unix_nano: 1_700_000_000_000_000_001,
                        severity_number: 9,
                        severity_text: "INFO".to_string(),
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue(
                                "something happened".to_string(),
                            )),
                        }),
                        attributes: vec![make_kv("log.key", "log.val")],
                        dropped_attributes_count: 0,
                        flags: 0,
                        trace_id: trace_id.clone(),
                        span_id: span_id.clone(),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let events = decode_logs(&buf).unwrap();
        assert_eq!(events.len(), 1);

        let ev = &events[0];
        assert_eq!(ev.timestamp, 1_700_000_000_000_000_000);
        assert_eq!(ev.severity, "INFO");
        assert_eq!(ev.body, "something happened");
        assert_eq!(ev.session_id.as_deref(), Some("sess-123"));
        assert_eq!(
            ev.trace_id.as_deref(),
            Some("0102030405060708090a0b0c0d0e0f10")
        );
        assert_eq!(ev.span_id.as_deref(), Some("aabbccddeeff1122"));

        let resource: serde_json::Value = serde_json::from_str(&ev.resource_attrs).unwrap();
        assert_eq!(resource["service.name"], "test-svc");
        assert_eq!(resource["session.id"], "sess-123");

        let scope: serde_json::Value = serde_json::from_str(&ev.scope_attrs).unwrap();
        assert_eq!(scope["scope.key"], "scope.val");

        let log_a: serde_json::Value = serde_json::from_str(&ev.log_attrs).unwrap();
        assert_eq!(log_a["log.key"], "log.val");
    }

    #[test]
    fn decode_logs_multiple_resources_and_scopes() {
        let request = ExportLogsServiceRequest {
            resource_logs: vec![
                ResourceLogs {
                    resource: make_resource(vec![make_kv("service.name", "svc-a")]),
                    scope_logs: vec![
                        ScopeLogs {
                            scope: make_scope("lib-1"),
                            log_records: vec![
                                LogRecord {
                                    time_unix_nano: 100,
                                    severity_number: 17,
                                    body: Some(AnyValue {
                                        value: Some(any_value::Value::StringValue(
                                            "err1".to_string(),
                                        )),
                                    }),
                                    ..Default::default()
                                },
                                LogRecord {
                                    time_unix_nano: 200,
                                    severity_number: 5,
                                    body: Some(AnyValue {
                                        value: Some(any_value::Value::StringValue(
                                            "dbg1".to_string(),
                                        )),
                                    }),
                                    ..Default::default()
                                },
                            ],
                            schema_url: String::new(),
                        },
                        ScopeLogs {
                            scope: make_scope("lib-2"),
                            log_records: vec![LogRecord {
                                time_unix_nano: 300,
                                severity_number: 21,
                                body: Some(AnyValue {
                                    value: Some(any_value::Value::StringValue(
                                        "fatal1".to_string(),
                                    )),
                                }),
                                ..Default::default()
                            }],
                            schema_url: String::new(),
                        },
                    ],
                    schema_url: String::new(),
                },
                ResourceLogs {
                    resource: make_resource(vec![make_kv("service.name", "svc-b")]),
                    scope_logs: vec![ScopeLogs {
                        scope: make_scope("lib-3"),
                        log_records: vec![LogRecord {
                            time_unix_nano: 400,
                            severity_number: 1,
                            body: Some(AnyValue {
                                value: Some(any_value::Value::StringValue("trace1".to_string())),
                            }),
                            ..Default::default()
                        }],
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                },
            ],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let events = decode_logs(&buf).unwrap();
        assert_eq!(events.len(), 4);

        assert_eq!(events[0].severity, "ERROR");
        assert_eq!(events[0].body, "err1");
        assert_eq!(events[1].severity, "DEBUG");
        assert_eq!(events[1].body, "dbg1");
        assert_eq!(events[2].severity, "FATAL");
        assert_eq!(events[2].body, "fatal1");
        assert_eq!(events[3].severity, "TRACE");
        assert_eq!(events[3].body, "trace1");

        let r0: serde_json::Value = serde_json::from_str(&events[0].resource_attrs).unwrap();
        assert_eq!(r0["service.name"], "svc-a");
        let r3: serde_json::Value = serde_json::from_str(&events[3].resource_attrs).unwrap();
        assert_eq!(r3["service.name"], "svc-b");
    }

    #[test]
    fn decode_logs_no_trace_id_or_span_id() {
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: make_resource(vec![]),
                scope_logs: vec![ScopeLogs {
                    scope: None,
                    log_records: vec![LogRecord {
                        time_unix_nano: 500,
                        severity_number: 13,
                        body: Some(AnyValue {
                            value: Some(any_value::Value::StringValue("warn msg".to_string())),
                        }),
                        trace_id: vec![],
                        span_id: vec![],
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let events = decode_logs(&buf).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].trace_id.is_none());
        assert!(events[0].span_id.is_none());
        assert!(events[0].session_id.is_none());
        assert_eq!(events[0].severity, "WARN");
    }

    #[test]
    fn decode_traces_round_trip() {
        let trace_id = vec![
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10,
        ];
        let span_id = vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x11, 0x22];
        let parent_span_id = vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: make_resource(vec![make_kv("service.name", "trace-svc")]),
                scope_spans: vec![ScopeSpans {
                    scope: make_scope("tracer"),
                    spans: vec![OtlpSpan {
                        trace_id: trace_id.clone(),
                        span_id: span_id.clone(),
                        parent_span_id: parent_span_id.clone(),
                        name: "GET /api/users".to_string(),
                        kind: SpanKind::Server as i32,
                        start_time_unix_nano: 1_700_000_000_000_000_000,
                        end_time_unix_nano: 1_700_000_000_500_000_000,
                        attributes: vec![make_kv("http.method", "GET")],
                        status: Some(Status {
                            code: StatusCode::Ok as i32,
                            message: String::new(),
                        }),
                        events: vec![Event {
                            time_unix_nano: 1_700_000_000_100_000_000,
                            name: "fetching users".to_string(),
                            attributes: vec![make_kv("event.key", "event.val")],
                            dropped_attributes_count: 0,
                        }],
                        ..Default::default()
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let spans = decode_traces(&buf).unwrap();
        assert_eq!(spans.len(), 1);

        let s = &spans[0];
        assert_eq!(s.trace_id, "0102030405060708090a0b0c0d0e0f10");
        assert_eq!(s.span_id, "aabbccddeeff1122");
        assert_eq!(s.parent_span_id.as_deref(), Some("1122334455667788"));
        assert_eq!(s.name, "GET /api/users");
        assert_eq!(s.kind, SpanKind::Server as i32);
        assert_eq!(s.start_time, 1_700_000_000_000_000_000);
        assert_eq!(s.end_time, 1_700_000_000_500_000_000);
        assert_eq!(s.status_code, StatusCode::Ok as i32);
        assert!(s.status_message.is_none());

        let attrs: serde_json::Value = serde_json::from_str(&s.resource_attrs).unwrap();
        assert_eq!(attrs["service.name"], "trace-svc");

        let span_a: serde_json::Value = serde_json::from_str(&s.span_attrs).unwrap();
        assert_eq!(span_a["http.method"], "GET");

        let events: Vec<serde_json::Value> = serde_json::from_str(&s.events_json).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["name"], "fetching users");
    }

    #[test]
    fn decode_traces_multiple_resources() {
        let request = ExportTraceServiceRequest {
            resource_spans: vec![
                ResourceSpans {
                    resource: make_resource(vec![make_kv("service.name", "svc-a")]),
                    scope_spans: vec![ScopeSpans {
                        scope: None,
                        spans: vec![OtlpSpan {
                            trace_id: vec![1; 16],
                            span_id: vec![2; 8],
                            name: "span-a".to_string(),
                            ..Default::default()
                        }],
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                },
                ResourceSpans {
                    resource: make_resource(vec![make_kv("service.name", "svc-b")]),
                    scope_spans: vec![ScopeSpans {
                        scope: None,
                        spans: vec![OtlpSpan {
                            trace_id: vec![3; 16],
                            span_id: vec![4; 8],
                            name: "span-b".to_string(),
                            ..Default::default()
                        }],
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                },
            ],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let spans = decode_traces(&buf).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].name, "span-a");
        assert_eq!(spans[1].name, "span-b");

        let r0: serde_json::Value = serde_json::from_str(&spans[0].resource_attrs).unwrap();
        assert_eq!(r0["service.name"], "svc-a");
        let r1: serde_json::Value = serde_json::from_str(&spans[1].resource_attrs).unwrap();
        assert_eq!(r1["service.name"], "svc-b");
    }

    #[test]
    fn decode_traces_root_span_no_parent() {
        let request = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                resource: make_resource(vec![]),
                scope_spans: vec![ScopeSpans {
                    scope: None,
                    spans: vec![OtlpSpan {
                        trace_id: vec![0xab; 16],
                        span_id: vec![0xcd; 8],
                        parent_span_id: vec![],
                        name: "root".to_string(),
                        status: Some(Status {
                            code: StatusCode::Error as i32,
                            message: "something broke".to_string(),
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

        let spans = decode_traces(&buf).unwrap();
        assert_eq!(spans.len(), 1);
        assert!(spans[0].parent_span_id.is_none());
        assert_eq!(spans[0].status_code, StatusCode::Error as i32);
        assert_eq!(spans[0].status_message.as_deref(), Some("something broke"));
    }

    #[test]
    fn decode_metrics_gauge_round_trip() {
        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: make_resource(vec![make_kv("service.name", "metric-svc")]),
                scope_metrics: vec![ScopeMetrics {
                    scope: make_scope("meter"),
                    metrics: vec![OtlpMetric {
                        name: "cpu.usage".to_string(),
                        description: "CPU usage percentage".to_string(),
                        unit: "%".to_string(),
                        metadata: vec![],
                        data: Some(metric::Data::Gauge(Gauge {
                            data_points: vec![NumberDataPoint {
                                time_unix_nano: 1_700_000_000_000_000_000,
                                start_time_unix_nano: 1_699_999_999_000_000_000,
                                value: Some(number_data_point::Value::AsDouble(75.5)),
                                attributes: vec![make_kv("host", "web-01")],
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

        let metrics = decode_metrics(&buf).unwrap();
        assert_eq!(metrics.len(), 1);

        let m = &metrics[0];
        assert_eq!(m.name, "cpu.usage");
        assert_eq!(m.description.as_deref(), Some("CPU usage percentage"));
        assert_eq!(m.unit.as_deref(), Some("%"));
        assert_eq!(m.metric_type, "gauge");
        assert_eq!(m.timestamp, 1_700_000_000_000_000_000);

        let r: serde_json::Value = serde_json::from_str(&m.resource_attrs).unwrap();
        assert_eq!(r["service.name"], "metric-svc");

        let points: Vec<serde_json::Value> = serde_json::from_str(&m.data_points_json).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0]["value_double"], 75.5);
    }

    #[test]
    fn decode_metrics_sum_round_trip() {
        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: make_resource(vec![make_kv("service.name", "counter-svc")]),
                scope_metrics: vec![ScopeMetrics {
                    scope: None,
                    metrics: vec![OtlpMetric {
                        name: "http.requests".to_string(),
                        description: String::new(),
                        unit: String::new(),
                        metadata: vec![],
                        data: Some(metric::Data::Sum(Sum {
                            data_points: vec![NumberDataPoint {
                                time_unix_nano: 2_000_000_000_000_000_000,
                                value: Some(number_data_point::Value::AsInt(42)),
                                ..Default::default()
                            }],
                            aggregation_temporality: 2,
                            is_monotonic: true,
                        })),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let metrics = decode_metrics(&buf).unwrap();
        assert_eq!(metrics.len(), 1);

        let m = &metrics[0];
        assert_eq!(m.name, "http.requests");
        assert!(m.description.is_none());
        assert!(m.unit.is_none());
        assert_eq!(m.metric_type, "sum");

        let points: Vec<serde_json::Value> = serde_json::from_str(&m.data_points_json).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0]["value_int"], 42);
    }

    #[test]
    fn decode_metrics_multiple_resources() {
        let request = ExportMetricsServiceRequest {
            resource_metrics: vec![
                ResourceMetrics {
                    resource: make_resource(vec![make_kv("service.name", "svc-a")]),
                    scope_metrics: vec![ScopeMetrics {
                        scope: None,
                        metrics: vec![OtlpMetric {
                            name: "metric.a".to_string(),
                            description: String::new(),
                            unit: String::new(),
                            metadata: vec![],
                            data: Some(metric::Data::Gauge(Gauge {
                                data_points: vec![NumberDataPoint {
                                    time_unix_nano: 100,
                                    value: Some(number_data_point::Value::AsDouble(1.0)),
                                    ..Default::default()
                                }],
                            })),
                        }],
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                },
                ResourceMetrics {
                    resource: make_resource(vec![make_kv("service.name", "svc-b")]),
                    scope_metrics: vec![ScopeMetrics {
                        scope: None,
                        metrics: vec![OtlpMetric {
                            name: "metric.b".to_string(),
                            description: String::new(),
                            unit: String::new(),
                            metadata: vec![],
                            data: Some(metric::Data::Gauge(Gauge {
                                data_points: vec![NumberDataPoint {
                                    time_unix_nano: 200,
                                    value: Some(number_data_point::Value::AsDouble(2.0)),
                                    ..Default::default()
                                }],
                            })),
                        }],
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                },
            ],
        };

        let mut buf = Vec::new();
        request.encode(&mut buf).unwrap();

        let metrics = decode_metrics(&buf).unwrap();
        assert_eq!(metrics.len(), 2);
        assert_eq!(metrics[0].name, "metric.a");
        assert_eq!(metrics[1].name, "metric.b");

        let r0: serde_json::Value = serde_json::from_str(&metrics[0].resource_attrs).unwrap();
        assert_eq!(r0["service.name"], "svc-a");
        let r1: serde_json::Value = serde_json::from_str(&metrics[1].resource_attrs).unwrap();
        assert_eq!(r1["service.name"], "svc-b");
    }

    #[test]
    fn severity_mapping() {
        assert_eq!(severity_number_to_string(0), "UNSPECIFIED");
        assert_eq!(severity_number_to_string(1), "TRACE");
        assert_eq!(severity_number_to_string(4), "TRACE");
        assert_eq!(severity_number_to_string(5), "DEBUG");
        assert_eq!(severity_number_to_string(8), "DEBUG");
        assert_eq!(severity_number_to_string(9), "INFO");
        assert_eq!(severity_number_to_string(12), "INFO");
        assert_eq!(severity_number_to_string(13), "WARN");
        assert_eq!(severity_number_to_string(16), "WARN");
        assert_eq!(severity_number_to_string(17), "ERROR");
        assert_eq!(severity_number_to_string(20), "ERROR");
        assert_eq!(severity_number_to_string(21), "FATAL");
        assert_eq!(severity_number_to_string(24), "FATAL");
        assert_eq!(severity_number_to_string(99), "UNKNOWN(99)");
    }
}
