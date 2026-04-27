use nous_core::db::MemoryDb;
use nous_core::types::{MemoryType, NewMemory};
use nous_otlp::db::OtlpDb;
use nous_otlp::decode::{LogEvent, Span};
use rusqlite::params;

#[test]
fn session_id_links_memory_to_otlp_log() {
    let session_id = "sess-corr-001";

    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();

    let memory = NewMemory {
        title: "test memory".into(),
        content: "content for correlation".into(),
        memory_type: MemoryType::Fact,
        source: None,
        importance: Default::default(),
        confidence: Default::default(),
        tags: vec![],
        workspace_path: None,
        session_id: Some(session_id.into()),
        trace_id: None,
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    let memory_id = memory_db.store(&memory).unwrap();

    let log = LogEvent {
        timestamp: 1714000000,
        severity: "INFO".into(),
        body: "correlated log entry".into(),
        resource_attrs: "{}".into(),
        scope_attrs: "{}".into(),
        log_attrs: "{}".into(),
        session_id: Some(session_id.into()),
        trace_id: None,
        span_id: None,
    };
    otlp_db.store_logs(&[log]).unwrap();

    let recalled = memory_db.recall(&memory_id).unwrap().unwrap();
    assert_eq!(recalled.memory.session_id.as_deref(), Some(session_id));

    let logs = otlp_db.query_logs(session_id, None, None).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].session_id.as_deref(), Some(session_id));
}

#[test]
fn trace_id_links_memory_to_otlp_span() {
    let trace_id = "trace-corr-001";

    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();

    let memory = NewMemory {
        title: "traced memory".into(),
        content: "content linked by trace".into(),
        memory_type: MemoryType::Decision,
        source: None,
        importance: Default::default(),
        confidence: Default::default(),
        tags: vec![],
        workspace_path: None,
        session_id: None,
        trace_id: Some(trace_id.into()),
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    let memory_id = memory_db.store(&memory).unwrap();

    let span = Span {
        trace_id: trace_id.into(),
        span_id: "span-001".into(),
        parent_span_id: None,
        name: "correlated operation".into(),
        kind: 1,
        start_time: 1714000000,
        end_time: 1714000500,
        status_code: 0,
        status_message: None,
        resource_attrs: "{}".into(),
        span_attrs: "{}".into(),
        events_json: "[]".into(),
    };
    otlp_db.store_spans(&[span]).unwrap();

    let recalled = memory_db.recall(&memory_id).unwrap().unwrap();
    assert_eq!(recalled.memory.trace_id.as_deref(), Some(trace_id));

    let spans = otlp_db.query_spans(trace_id, None, None).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].trace_id, trace_id);
}

#[test]
fn session_and_trace_ids_together() {
    let session_id = "sess-corr-002";
    let trace_id = "trace-corr-002";

    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();

    let memory = NewMemory {
        title: "dual-linked memory".into(),
        content: "linked by both session and trace".into(),
        memory_type: MemoryType::Observation,
        source: None,
        importance: Default::default(),
        confidence: Default::default(),
        tags: vec![],
        workspace_path: None,
        session_id: Some(session_id.into()),
        trace_id: Some(trace_id.into()),
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    let memory_id = memory_db.store(&memory).unwrap();

    let log = LogEvent {
        timestamp: 1714001000,
        severity: "DEBUG".into(),
        body: "dual-linked log".into(),
        resource_attrs: "{}".into(),
        scope_attrs: "{}".into(),
        log_attrs: "{}".into(),
        session_id: Some(session_id.into()),
        trace_id: Some(trace_id.into()),
        span_id: None,
    };
    otlp_db.store_logs(&[log]).unwrap();

    let span = Span {
        trace_id: trace_id.into(),
        span_id: "span-dual-001".into(),
        parent_span_id: None,
        name: "dual operation".into(),
        kind: 2,
        start_time: 1714001000,
        end_time: 1714001200,
        status_code: 0,
        status_message: None,
        resource_attrs: "{}".into(),
        span_attrs: "{}".into(),
        events_json: "[]".into(),
    };
    otlp_db.store_spans(&[span]).unwrap();

    let recalled = memory_db.recall(&memory_id).unwrap().unwrap();
    assert_eq!(recalled.memory.session_id.as_deref(), Some(session_id));
    assert_eq!(recalled.memory.trace_id.as_deref(), Some(trace_id));

    let logs = otlp_db.query_logs(session_id, None, None).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].trace_id.as_deref(), Some(trace_id));

    let spans = otlp_db.query_spans(trace_id, None, None).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].trace_id, trace_id);
}

#[test]
fn correlation_ids_isolate_across_sessions() {
    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();

    for i in 0..3 {
        let sid = format!("sess-iso-{i}");
        let tid = format!("trace-iso-{i}");

        let memory = NewMemory {
            title: format!("memory {i}"),
            content: format!("content {i}"),
            memory_type: MemoryType::Fact,
            source: None,
            importance: Default::default(),
            confidence: Default::default(),
            tags: vec![],
            workspace_path: None,
            session_id: Some(sid.clone()),
            trace_id: Some(tid.clone()),
            agent_id: None,
            agent_model: None,
            valid_from: None,
            category_id: None,
        };
        memory_db.store(&memory).unwrap();

        otlp_db
            .store_logs(&[LogEvent {
                timestamp: 1714000000 + i,
                severity: "INFO".into(),
                body: format!("log {i}"),
                resource_attrs: "{}".into(),
                scope_attrs: "{}".into(),
                log_attrs: "{}".into(),
                session_id: Some(sid.clone()),
                trace_id: Some(tid.clone()),
                span_id: None,
            }])
            .unwrap();

        otlp_db
            .store_spans(&[Span {
                trace_id: tid.clone(),
                span_id: format!("span-iso-{i}"),
                parent_span_id: None,
                name: format!("op {i}"),
                kind: 1,
                start_time: 1714000000 + i,
                end_time: 1714000100 + i,
                status_code: 0,
                status_message: None,
                resource_attrs: "{}".into(),
                span_attrs: "{}".into(),
                events_json: "[]".into(),
            }])
            .unwrap();
    }

    for i in 0..3 {
        let sid = format!("sess-iso-{i}");
        let tid = format!("trace-iso-{i}");

        let logs = otlp_db.query_logs(&sid, None, None).unwrap();
        assert_eq!(logs.len(), 1, "session {sid} should have exactly 1 log");
        assert_eq!(logs[0].session_id.as_deref(), Some(sid.as_str()));

        let spans = otlp_db.query_spans(&tid, None, None).unwrap();
        assert_eq!(spans.len(), 1, "trace {tid} should have exactly 1 span");
        assert_eq!(spans[0].trace_id, tid);

        let mem_sid: String = memory_db
            .connection()
            .query_row(
                "SELECT session_id FROM memories WHERE session_id = ?1",
                params![sid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_sid, sid);

        let mem_tid: String = memory_db
            .connection()
            .query_row(
                "SELECT trace_id FROM memories WHERE trace_id = ?1",
                params![tid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(mem_tid, tid);
    }
}

#[test]
fn otlp_trace_context_returns_memories_and_spans() {
    let trace_id = "trace-ctx-001";
    let session_id = "sess-ctx-001";

    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();

    let memory = NewMemory {
        title: "trace context memory".into(),
        content: "memory for otlp_trace_context test".into(),
        memory_type: MemoryType::Decision,
        source: None,
        importance: Default::default(),
        confidence: Default::default(),
        tags: vec![],
        workspace_path: None,
        session_id: Some(session_id.into()),
        trace_id: Some(trace_id.into()),
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    memory_db.store(&memory).unwrap();

    otlp_db
        .store_spans(&[Span {
            trace_id: trace_id.into(),
            span_id: "span-ctx-001".into(),
            parent_span_id: None,
            name: "test operation".into(),
            kind: 1,
            start_time: 1714002000,
            end_time: 1714002500,
            status_code: 0,
            status_message: None,
            resource_attrs: "{}".into(),
            span_attrs: "{}".into(),
            events_json: "[]".into(),
        }])
        .unwrap();

    otlp_db
        .store_logs(&[LogEvent {
            timestamp: 1714002000,
            severity: "INFO".into(),
            body: "correlated log for trace context".into(),
            resource_attrs: "{}".into(),
            scope_attrs: "{}".into(),
            log_attrs: "{}".into(),
            session_id: Some(session_id.into()),
            trace_id: Some(trace_id.into()),
            span_id: None,
        }])
        .unwrap();

    let memories: Vec<String> = memory_db
        .connection()
        .prepare("SELECT id FROM memories WHERE trace_id = ?1")
        .unwrap()
        .query_map(params![trace_id], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(memories.len(), 1);

    let spans = otlp_db.query_spans(trace_id, None, None).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "test operation");

    let logs = otlp_db.query_logs(session_id, None, None).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].body, "correlated log for trace context");
}

#[test]
fn otlp_memory_context_reverse_lookup() {
    let trace_id = "trace-rev-001";
    let session_id = "sess-rev-001";

    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();

    let memory = NewMemory {
        title: "reverse lookup memory".into(),
        content: "memory for reverse lookup test".into(),
        memory_type: MemoryType::Fact,
        source: None,
        importance: Default::default(),
        confidence: Default::default(),
        tags: vec![],
        workspace_path: None,
        session_id: Some(session_id.into()),
        trace_id: Some(trace_id.into()),
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    let memory_id = memory_db.store(&memory).unwrap();

    otlp_db
        .store_spans(&[Span {
            trace_id: trace_id.into(),
            span_id: "span-rev-001".into(),
            parent_span_id: None,
            name: "reverse op".into(),
            kind: 2,
            start_time: 1714003000,
            end_time: 1714003500,
            status_code: 0,
            status_message: None,
            resource_attrs: "{}".into(),
            span_attrs: "{}".into(),
            events_json: "[]".into(),
        }])
        .unwrap();

    otlp_db
        .store_logs(&[LogEvent {
            timestamp: 1714003000,
            severity: "WARN".into(),
            body: "reverse lookup log".into(),
            resource_attrs: "{}".into(),
            scope_attrs: "{}".into(),
            log_attrs: "{}".into(),
            session_id: Some(session_id.into()),
            trace_id: None,
            span_id: None,
        }])
        .unwrap();

    let recalled = memory_db.recall(&memory_id).unwrap().unwrap();
    let tid = recalled.memory.trace_id.as_deref().unwrap();
    let sid = recalled.memory.session_id.as_deref().unwrap();

    let spans = otlp_db.query_spans(tid, None, None).unwrap();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].name, "reverse op");

    let logs = otlp_db.query_logs(sid, None, None).unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].body, "reverse lookup log");
}

#[test]
fn unknown_trace_id_returns_empty() {
    let otlp_db = OtlpDb::open(":memory:", None).unwrap();
    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();

    let spans = otlp_db
        .query_spans("nonexistent-trace-id", None, None)
        .unwrap();
    assert!(spans.is_empty());

    let memories: Vec<String> = memory_db
        .connection()
        .prepare("SELECT id FROM memories WHERE trace_id = ?1")
        .unwrap()
        .query_map(params!["nonexistent-trace-id"], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(memories.is_empty());
}

#[test]
fn memory_without_trace_id_has_no_otlp_correlation() {
    let memory_db = MemoryDb::open(":memory:", None, 384).unwrap();

    let memory = NewMemory {
        title: "no-trace memory".into(),
        content: "memory without trace_id or session_id".into(),
        memory_type: MemoryType::Fact,
        source: None,
        importance: Default::default(),
        confidence: Default::default(),
        tags: vec![],
        workspace_path: None,
        session_id: None,
        trace_id: None,
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    let memory_id = memory_db.store(&memory).unwrap();

    let recalled = memory_db.recall(&memory_id).unwrap().unwrap();
    assert!(recalled.memory.trace_id.is_none());
    assert!(recalled.memory.session_id.is_none());
}
