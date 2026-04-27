use std::io;
use std::path::Path;

use nous_core::db::MemoryDb;
use nous_shared::ids::MemoryId;

use crate::config::Config;

use super::expand_tilde;

pub fn run_trace(
    config: &Config,
    trace_id: Option<&str>,
    memory_id: Option<&str>,
    session_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let otlp_path = expand_tilde(&config.otlp.db_path);
    if !Path::new(&otlp_path).exists() {
        return Err(format!("OTLP database not found at {otlp_path}").into());
    }
    let otlp_db = nous_otlp::db::OtlpDb::open(&otlp_path, None)?;

    if let Some(mid) = memory_id {
        let db_key = config.resolve_db_key().ok();
        let mem_db = MemoryDb::open(
            &config.memory.db_path,
            db_key.as_deref(),
            config.embedding.dimensions,
        )?;
        let id: MemoryId = mid.parse().unwrap();
        let recalled = mem_db
            .recall(&id)?
            .ok_or_else(|| format!("memory {mid} not found"))?;

        let tid = recalled.memory.trace_id.as_deref();
        let sid = recalled.memory.session_id.as_deref();

        if tid.is_none() && sid.is_none() {
            return Err("memory has no trace_id or session_id for OTLP correlation".into());
        }

        let spans = match tid {
            Some(t) => otlp_db.query_spans(t, None, None)?,
            None => vec![],
        };
        let logs = match sid {
            Some(s) => otlp_db.query_logs(s, None, None)?,
            None => vec![],
        };

        let output = serde_json::json!({
            "memory": {
                "id": recalled.memory.id,
                "title": recalled.memory.title,
                "content": recalled.memory.content,
                "memory_type": recalled.memory.memory_type.to_string(),
                "trace_id": recalled.memory.trace_id,
                "session_id": recalled.memory.session_id,
                "created_at": recalled.memory.created_at,
            },
            "spans": spans.iter().map(span_to_json).collect::<Vec<_>>(),
            "logs": logs.iter().map(log_to_json).collect::<Vec<_>>(),
        });
        serde_json::to_writer_pretty(io::stdout().lock(), &output)?;
        println!();
    } else if let Some(tid) = trace_id {
        let db_key = config.resolve_db_key().ok();
        let mem_db = MemoryDb::open(
            &config.memory.db_path,
            db_key.as_deref(),
            config.embedding.dimensions,
        )?;
        let conn = mem_db.connection();

        let mut stmt = conn.prepare(
            "SELECT id, title, content, memory_type, session_id, trace_id, created_at
             FROM memories WHERE trace_id = ?1 ORDER BY created_at DESC",
        )?;
        let memories: Vec<serde_json::Value> = stmt
            .query_map(rusqlite::params![tid], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "content": row.get::<_, String>(2)?,
                    "memory_type": row.get::<_, String>(3)?,
                    "session_id": row.get::<_, Option<String>>(4)?,
                    "trace_id": row.get::<_, Option<String>>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let spans = otlp_db.query_spans(tid, None, None)?;
        let logs = match session_id {
            Some(sid) => otlp_db.query_logs(sid, None, None)?,
            None => vec![],
        };

        let output = serde_json::json!({
            "memories": memories,
            "spans": spans.iter().map(span_to_json).collect::<Vec<_>>(),
            "logs": logs.iter().map(log_to_json).collect::<Vec<_>>(),
        });
        serde_json::to_writer_pretty(io::stdout().lock(), &output)?;
        println!();
    } else {
        return Err("either --trace-id or --memory-id is required".into());
    }

    Ok(())
}

fn span_to_json(s: &nous_otlp::decode::Span) -> serde_json::Value {
    serde_json::json!({
        "trace_id": s.trace_id,
        "span_id": s.span_id,
        "parent_span_id": s.parent_span_id,
        "name": s.name,
        "kind": s.kind,
        "start_time": s.start_time,
        "end_time": s.end_time,
        "status_code": s.status_code,
        "status_message": s.status_message,
        "resource_attrs": s.resource_attrs,
        "span_attrs": s.span_attrs,
        "events_json": s.events_json,
    })
}

fn log_to_json(l: &nous_otlp::decode::LogEvent) -> serde_json::Value {
    serde_json::json!({
        "timestamp": l.timestamp,
        "severity": l.severity,
        "body": l.body,
        "resource_attrs": l.resource_attrs,
        "scope_attrs": l.scope_attrs,
        "log_attrs": l.log_attrs,
        "session_id": l.session_id,
        "trace_id": l.trace_id,
        "span_id": l.span_id,
    })
}
