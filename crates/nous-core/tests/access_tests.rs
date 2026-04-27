use nous_core::db::MemoryDb;
use nous_core::types::{MemoryPatch, MemoryType, NewMemory, RelationType};
use rusqlite::params;

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None, 384).expect("failed to open in-memory db")
}

fn minimal_memory() -> NewMemory {
    NewMemory {
        title: "test title".into(),
        content: "test content".into(),
        memory_type: MemoryType::Decision,
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
    }
}

// 1. Store a memory, log 5 accesses with tool_name 'memory_recall'. access_count returns 5
#[test]
fn log_access_and_count() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    for _ in 0..5 {
        db.log_access(&id, "memory_recall").unwrap();
    }

    assert_eq!(db.access_count(&id).unwrap(), 5);
}

// 2. Store 2 memories, log different access counts. most_accessed returns correct ranking
#[test]
fn most_accessed_ranking() {
    let db = open_test_db();
    let id1 = db.store(&minimal_memory()).unwrap();
    let mut m2 = minimal_memory();
    m2.title = "second".into();
    let id2 = db.store(&m2).unwrap();

    for _ in 0..3 {
        db.log_access(&id1, "recall").unwrap();
    }
    for _ in 0..7 {
        db.log_access(&id2, "recall").unwrap();
    }

    let ranked = db.most_accessed(None, 10).unwrap();
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, id2.to_string());
    assert_eq!(ranked[0].1, 7);
    assert_eq!(ranked[1].0, id1.to_string());
    assert_eq!(ranked[1].1, 3);
}

// 3. most_accessed with since filter returns correct counts
#[test]
fn most_accessed_with_since_filter() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    // Insert an old access log entry with a past timestamp
    db.connection()
        .execute(
            "INSERT INTO access_log (memory_id, access_type, accessed_at) VALUES (?1, 'recall', '2020-01-01T00:00:00.000Z')",
            params![id.to_string()],
        )
        .unwrap();

    // Insert a recent access via API
    db.log_access(&id, "recall").unwrap();

    // Filter since a date after the old entry
    let ranked = db
        .most_accessed(Some("2024-01-01T00:00:00.000Z"), 10)
        .unwrap();
    assert_eq!(ranked.len(), 1);
    assert_eq!(ranked[0].1, 1);

    // Without filter, both show up
    let all = db.most_accessed(None, 10).unwrap();
    assert_eq!(all[0].1, 2);
}

// 4. log_access stores tool_name correctly in access_type column
#[test]
fn log_access_stores_tool_name() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    db.log_access(&id, "semantic_search").unwrap();

    let access_type: String = db
        .connection()
        .query_row(
            "SELECT access_type FROM access_log WHERE memory_id = ?1 ORDER BY id DESC LIMIT 1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(access_type, "semantic_search");
}

// 5. Create memory A and B, relate B supersedes A, assert A.valid_until is set
#[test]
fn supersedes_sets_valid_until_on_target() {
    let db = open_test_db();
    let a = db.store(&minimal_memory()).unwrap();
    let mut m_b = minimal_memory();
    m_b.title = "replacement".into();
    let b = db.store(&m_b).unwrap();

    // Before: valid_until is NULL
    let before: Option<String> = db
        .connection()
        .query_row(
            "SELECT valid_until FROM memories WHERE id = ?1",
            params![a.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(before.is_none());

    db.relate(&b, &a, RelationType::Supersedes).unwrap();

    // After: valid_until is set
    let after: Option<String> = db
        .connection()
        .query_row(
            "SELECT valid_until FROM memories WHERE id = ?1",
            params![a.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        after.is_some(),
        "valid_until should be set on superseded memory"
    );
}

// 6. build_validity_clause(Some(true)) returns the expected SQL fragment
#[test]
fn build_validity_clause_true() {
    let clause = nous_core::db::build_validity_clause(Some(true));
    assert_eq!(
        clause.as_deref(),
        Some("AND (valid_until IS NULL OR valid_until > datetime('now'))")
    );
}

// 7. build_validity_clause(None) returns None
#[test]
fn build_validity_clause_none() {
    assert!(nous_core::db::build_validity_clause(None).is_none());
    assert!(nous_core::db::build_validity_clause(Some(false)).is_none());
}

// 8. Explicit valid_until via update() independent of relationships
#[test]
fn update_sets_valid_until_independently() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    let patch = MemoryPatch {
        title: None,
        content: None,
        tags: None,
        importance: None,
        confidence: None,
        valid_until: Some("2020-01-01T00:00:00.000Z".into()),
    };
    assert!(db.update(&id, &patch).unwrap());

    let result = db.recall(&id).unwrap().unwrap();
    assert_eq!(
        result.memory.valid_until.as_deref(),
        Some("2020-01-01T00:00:00.000Z")
    );
}
