use crate::decode::{LogEvent, Metric, Span};
use nous_shared::Result;
use nous_shared::sqlite::{open_connection, run_migrations};
use rusqlite::{Connection, params};

const OTLP_MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS log_events (
        timestamp INTEGER NOT NULL,
        severity TEXT NOT NULL,
        body TEXT NOT NULL,
        resource_attrs TEXT,
        scope_attrs TEXT,
        log_attrs TEXT,
        session_id TEXT,
        trace_id TEXT,
        span_id TEXT
    )",
    "CREATE TABLE IF NOT EXISTS spans (
        trace_id TEXT NOT NULL,
        span_id TEXT NOT NULL,
        parent_span_id TEXT,
        name TEXT NOT NULL,
        kind INTEGER,
        start_time INTEGER NOT NULL,
        end_time INTEGER NOT NULL,
        status_code INTEGER,
        status_message TEXT,
        resource_attrs TEXT,
        span_attrs TEXT,
        events_json TEXT
    )",
    "CREATE TABLE IF NOT EXISTS metrics (
        name TEXT NOT NULL,
        description TEXT,
        unit TEXT,
        type TEXT NOT NULL,
        data_points_json TEXT NOT NULL,
        resource_attrs TEXT,
        timestamp INTEGER NOT NULL
    )",
    "CREATE INDEX IF NOT EXISTS idx_log_events_session_id ON log_events(session_id)",
    "CREATE INDEX IF NOT EXISTS idx_log_events_trace_id ON log_events(trace_id)",
    "CREATE INDEX IF NOT EXISTS idx_log_events_timestamp ON log_events(timestamp)",
    "CREATE INDEX IF NOT EXISTS idx_spans_trace_id ON spans(trace_id)",
    "CREATE INDEX IF NOT EXISTS idx_spans_start_time ON spans(start_time)",
    "CREATE INDEX IF NOT EXISTS idx_metrics_timestamp ON metrics(timestamp)",
    "CREATE INDEX IF NOT EXISTS idx_metrics_name ON metrics(name)",
];

pub struct OtlpDb {
    conn: Connection,
}

impl OtlpDb {
    pub fn open(path: &str, key: Option<&str>) -> Result<Self> {
        let conn = open_connection(path, key)?;
        run_migrations(&conn, OTLP_MIGRATIONS)?;
        Ok(Self { conn })
    }

    pub fn from_connection(conn: Connection) -> Self {
        Self { conn }
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn store_logs(&self, logs: &[LogEvent]) -> Result<usize> {
        if logs.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT INTO log_events (timestamp, severity, body, resource_attrs, scope_attrs, log_attrs, session_id, trace_id, span_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for log in logs {
            stmt.execute(params![
                log.timestamp,
                log.severity,
                log.body,
                log.resource_attrs,
                log.scope_attrs,
                log.log_attrs,
                log.session_id,
                log.trace_id,
                log.span_id,
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(logs.len())
    }

    pub fn store_spans(&self, spans: &[Span]) -> Result<usize> {
        if spans.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT INTO spans (trace_id, span_id, parent_span_id, name, kind, start_time, end_time, status_code, status_message, resource_attrs, span_attrs, events_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        )?;
        for span in spans {
            stmt.execute(params![
                span.trace_id,
                span.span_id,
                span.parent_span_id,
                span.name,
                span.kind,
                span.start_time,
                span.end_time,
                span.status_code,
                span.status_message,
                span.resource_attrs,
                span.span_attrs,
                span.events_json,
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(spans.len())
    }

    pub fn store_metrics(&self, metrics: &[Metric]) -> Result<usize> {
        if metrics.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.unchecked_transaction()?;
        let mut stmt = tx.prepare_cached(
            "INSERT INTO metrics (name, description, unit, type, data_points_json, resource_attrs, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for m in metrics {
            stmt.execute(params![
                m.name,
                m.description,
                m.unit,
                m.metric_type,
                m.data_points_json,
                m.resource_attrs,
                m.timestamp,
            ])?;
        }
        drop(stmt);
        tx.commit()?;
        Ok(metrics.len())
    }

    pub fn query_logs(
        &self,
        session_id: &str,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<LogEvent>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT timestamp, severity, body, resource_attrs, scope_attrs, log_attrs, session_id, trace_id, span_id
             FROM log_events WHERE session_id = ?1 ORDER BY timestamp LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            params![session_id, limit.unwrap_or(-1), offset.unwrap_or(0)],
            |row| {
                Ok(LogEvent {
                    timestamp: row.get(0)?,
                    severity: row.get(1)?,
                    body: row.get(2)?,
                    resource_attrs: row.get(3)?,
                    scope_attrs: row.get(4)?,
                    log_attrs: row.get(5)?,
                    session_id: row.get(6)?,
                    trace_id: row.get(7)?,
                    span_id: row.get(8)?,
                })
            },
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn query_spans(
        &self,
        trace_id: &str,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<Span>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT trace_id, span_id, parent_span_id, name, kind, start_time, end_time, status_code, status_message, resource_attrs, span_attrs, events_json
             FROM spans WHERE trace_id = ?1 ORDER BY start_time LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            params![trace_id, limit.unwrap_or(-1), offset.unwrap_or(0)],
            |row| {
                Ok(Span {
                    trace_id: row.get(0)?,
                    span_id: row.get(1)?,
                    parent_span_id: row.get(2)?,
                    name: row.get(3)?,
                    kind: row.get(4)?,
                    start_time: row.get(5)?,
                    end_time: row.get(6)?,
                    status_code: row.get(7)?,
                    status_message: row.get(8)?,
                    resource_attrs: row.get(9)?,
                    span_attrs: row.get(10)?,
                    events_json: row.get(11)?,
                })
            },
        )?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{LogEvent, Metric, Span};
    use rusqlite::params;

    #[test]
    fn tables_exist_after_open() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let conn = db.connection();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert!(
            tables.contains(&"log_events".to_string()),
            "missing log_events table"
        );
        assert!(tables.contains(&"spans".to_string()), "missing spans table");
        assert!(
            tables.contains(&"metrics".to_string()),
            "missing metrics table"
        );
    }

    #[test]
    fn indexes_exist_after_open() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let conn = db.connection();

        let indexes: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert!(
            indexes.contains(&"idx_log_events_session_id".to_string()),
            "missing session_id index"
        );
        assert!(
            indexes.contains(&"idx_log_events_trace_id".to_string()),
            "missing log trace_id index"
        );
        assert!(
            indexes.contains(&"idx_log_events_timestamp".to_string()),
            "missing log timestamp index"
        );
        assert!(
            indexes.contains(&"idx_spans_trace_id".to_string()),
            "missing spans trace_id index"
        );
        assert!(
            indexes.contains(&"idx_spans_start_time".to_string()),
            "missing spans start_time index"
        );
        assert!(
            indexes.contains(&"idx_metrics_timestamp".to_string()),
            "missing metrics timestamp index"
        );
        assert!(
            indexes.contains(&"idx_metrics_name".to_string()),
            "missing metrics name index"
        );
    }

    #[test]
    fn insert_and_select_log_event() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let conn = db.connection();

        conn.execute(
            "INSERT INTO log_events (timestamp, severity, body, resource_attrs, scope_attrs, log_attrs, session_id, trace_id, span_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                1714000000_i64,
                "ERROR",
                "something broke",
                r#"{"service":"api"}"#,
                None::<String>,
                r#"{"key":"val"}"#,
                "sess-001",
                "trace-abc",
                "span-xyz",
            ],
        ).unwrap();

        let (ts, severity, body, session_id, trace_id, span_id): (i64, String, String, String, String, String) = conn
            .query_row(
                "SELECT timestamp, severity, body, session_id, trace_id, span_id FROM log_events WHERE session_id = ?1",
                params!["sess-001"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
            )
            .unwrap();

        assert_eq!(ts, 1714000000);
        assert_eq!(severity, "ERROR");
        assert_eq!(body, "something broke");
        assert_eq!(session_id, "sess-001");
        assert_eq!(trace_id, "trace-abc");
        assert_eq!(span_id, "span-xyz");
    }

    #[test]
    fn insert_and_select_span() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let conn = db.connection();

        conn.execute(
            "INSERT INTO spans (trace_id, span_id, parent_span_id, name, kind, start_time, end_time, status_code, status_message, resource_attrs, span_attrs, events_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                "trace-001",
                "span-001",
                None::<String>,
                "HTTP GET /api",
                1_i32,
                1714000000_i64,
                1714000500_i64,
                0_i32,
                None::<String>,
                None::<String>,
                r#"{"http.method":"GET"}"#,
                "[]",
            ],
        ).unwrap();

        let (trace_id, name, start, end): (String, String, i64, i64) = conn
            .query_row(
                "SELECT trace_id, name, start_time, end_time FROM spans WHERE trace_id = ?1",
                params!["trace-001"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        assert_eq!(trace_id, "trace-001");
        assert_eq!(name, "HTTP GET /api");
        assert_eq!(start, 1714000000);
        assert_eq!(end, 1714000500);
    }

    #[test]
    fn insert_and_select_metric() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let conn = db.connection();

        conn.execute(
            "INSERT INTO metrics (name, description, unit, type, data_points_json, resource_attrs, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "http.request.duration",
                "Duration of HTTP requests",
                "ms",
                "histogram",
                r#"[{"value":42}]"#,
                None::<String>,
                1714000000_i64,
            ],
        ).unwrap();

        let (name, typ, data): (String, String, String) = conn
            .query_row(
                "SELECT name, type, data_points_json FROM metrics WHERE name = ?1",
                params!["http.request.duration"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(name, "http.request.duration");
        assert_eq!(typ, "histogram");
        assert_eq!(data, r#"[{"value":42}]"#);
    }

    fn make_log(ts: i64, session_id: &str, body: &str) -> LogEvent {
        LogEvent {
            timestamp: ts,
            severity: "INFO".to_string(),
            body: body.to_string(),
            resource_attrs: "{}".to_string(),
            scope_attrs: "{}".to_string(),
            log_attrs: "{}".to_string(),
            session_id: Some(session_id.to_string()),
            trace_id: None,
            span_id: None,
        }
    }

    fn make_span(start: i64, trace_id: &str, name: &str) -> Span {
        Span {
            trace_id: trace_id.to_string(),
            span_id: format!("span-{start}"),
            parent_span_id: None,
            name: name.to_string(),
            kind: 1,
            start_time: start,
            end_time: start + 100,
            status_code: 0,
            status_message: None,
            resource_attrs: "{}".to_string(),
            span_attrs: "{}".to_string(),
            events_json: "[]".to_string(),
        }
    }

    fn make_metric(name: &str, ts: i64) -> Metric {
        Metric {
            name: name.to_string(),
            description: None,
            unit: None,
            metric_type: "gauge".to_string(),
            data_points_json: r#"[{"value":1.0}]"#.to_string(),
            resource_attrs: "{}".to_string(),
            timestamp: ts,
        }
    }

    #[test]
    fn store_and_query_logs_by_session_id() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let logs: Vec<LogEvent> = (0..5)
            .map(|i| make_log(1000 + i, "sess-abc", &format!("log-{i}")))
            .collect();

        let count = db.store_logs(&logs).unwrap();
        assert_eq!(count, 5);

        let result = db.query_logs("sess-abc", None, None).unwrap();
        assert_eq!(result.len(), 5);
        for i in 0..5 {
            assert_eq!(result[i].timestamp, 1000 + i as i64);
            assert_eq!(result[i].body, format!("log-{i}"));
        }
    }

    #[test]
    fn query_logs_returns_only_matching_session() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        db.store_logs(&[
            make_log(100, "sess-a", "a1"),
            make_log(200, "sess-b", "b1"),
            make_log(300, "sess-a", "a2"),
        ])
        .unwrap();

        let result = db.query_logs("sess-a", None, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].body, "a1");
        assert_eq!(result[1].body, "a2");
    }

    #[test]
    fn store_and_query_spans_by_trace_id() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let spans: Vec<Span> = (0..3)
            .map(|i| make_span(2000 + i, "trace-xyz", &format!("op-{i}")))
            .collect();

        let count = db.store_spans(&spans).unwrap();
        assert_eq!(count, 3);

        let result = db.query_spans("trace-xyz", None, None).unwrap();
        assert_eq!(result.len(), 3);
        for i in 0..3 {
            assert_eq!(result[i].start_time, 2000 + i as i64);
            assert_eq!(result[i].name, format!("op-{i}"));
        }
    }

    #[test]
    fn query_spans_returns_only_matching_trace() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        db.store_spans(&[
            make_span(100, "trace-a", "a1"),
            make_span(200, "trace-b", "b1"),
            make_span(300, "trace-a", "a2"),
        ])
        .unwrap();

        let result = db.query_spans("trace-a", None, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].name, "a1");
        assert_eq!(result[1].name, "a2");
    }

    #[test]
    fn store_metrics_returns_count() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let metrics: Vec<Metric> = (0..4)
            .map(|i| make_metric(&format!("metric.{i}"), 3000 + i))
            .collect();

        let count = db.store_metrics(&metrics).unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn store_empty_vecs_returns_zero() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        assert_eq!(db.store_logs(&[]).unwrap(), 0);
        assert_eq!(db.store_spans(&[]).unwrap(), 0);
        assert_eq!(db.store_metrics(&[]).unwrap(), 0);
    }

    #[test]
    fn query_logs_pagination() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let logs: Vec<LogEvent> = (0..10)
            .map(|i| make_log(1000 + i, "sess-pg", &format!("log-{i}")))
            .collect();
        db.store_logs(&logs).unwrap();

        let page1 = db.query_logs("sess-pg", Some(3), None).unwrap();
        assert_eq!(page1.len(), 3);
        assert_eq!(page1[0].body, "log-0");

        let page2 = db.query_logs("sess-pg", Some(3), Some(3)).unwrap();
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].body, "log-3");
    }

    #[test]
    fn query_spans_pagination() {
        let db = OtlpDb::open(":memory:", None).unwrap();
        let spans: Vec<Span> = (0..10)
            .map(|i| make_span(2000 + i, "trace-pg", &format!("op-{i}")))
            .collect();
        db.store_spans(&spans).unwrap();

        let page1 = db.query_spans("trace-pg", Some(3), None).unwrap();
        assert_eq!(page1.len(), 3);
        assert_eq!(page1[0].name, "op-0");

        let page2 = db.query_spans("trace-pg", Some(3), Some(3)).unwrap();
        assert_eq!(page2.len(), 3);
        assert_eq!(page2[0].name, "op-3");
    }
}
