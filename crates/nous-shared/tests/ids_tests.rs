use std::collections::HashSet;
use std::str::FromStr;

use nous_shared::{MemoryId, SessionId, SpanId, TraceId};

#[test]
fn memory_id_new_produces_valid_uuid() {
    let id = MemoryId::new();
    uuid::Uuid::parse_str(&id.to_string()).expect("MemoryId should be a valid UUID");
}

#[test]
fn memory_id_new_produces_unique_values() {
    let a = MemoryId::new();
    let b = MemoryId::new();
    assert_ne!(a, b);
}

#[test]
fn memory_id_new_is_lexicographically_ordered() {
    let a = MemoryId::new();
    let b = MemoryId::new();
    assert!(
        a.to_string() <= b.to_string(),
        "UUID v7 values should be time-ordered"
    );
}

#[test]
fn session_id_roundtrips_display_fromstr() {
    let original = SessionId::from_str("sess-abc-123").unwrap();
    let displayed = original.to_string();
    let parsed = SessionId::from_str(&displayed).unwrap();
    assert_eq!(original, parsed);
}

#[test]
fn trace_id_roundtrips_serde_json() {
    let original = TraceId::from_str("trace-xyz-789").unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: TraceId = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

#[test]
fn types_are_distinct() {
    let _session: SessionId = SessionId::from_str("id-1").unwrap();
    let _trace: TraceId = TraceId::from_str("id-1").unwrap();
    let _span: SpanId = SpanId::from_str("id-1").unwrap();
    let _memory: MemoryId = MemoryId::from_str("id-1").unwrap();
}

#[test]
fn all_types_implement_clone_hash_eq() {
    let session = SessionId::from_str("s1").unwrap();
    let mut set = HashSet::new();
    set.insert(session.clone());
    set.insert(session);
    assert_eq!(set.len(), 1);

    let trace = TraceId::from_str("t1").unwrap();
    let mut set = HashSet::new();
    set.insert(trace.clone());
    set.insert(trace);
    assert_eq!(set.len(), 1);

    let span = SpanId::from_str("sp1").unwrap();
    let mut set = HashSet::new();
    set.insert(span.clone());
    set.insert(span);
    assert_eq!(set.len(), 1);

    let memory = MemoryId::new();
    let mut set = HashSet::new();
    set.insert(memory.clone());
    set.insert(memory);
    assert_eq!(set.len(), 1);
}
