use nous_shared::Result;
use nous_shared::sqlite::{open_connection, run_migrations};
use rusqlite::Connection;

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

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
