use nous_core::db::MemoryDb;
use nous_core::types::{Importance, MemoryPatch, MemoryType, NewMemory, RelationType};
use nous_shared::ids::MemoryId;
use rusqlite::params;
use std::str::FromStr;

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

// 1. Store a memory, recall by ID, assert fields match
#[test]
fn store_and_recall_basic() {
    let db = open_test_db();
    let mem = minimal_memory();
    let id = db.store(&mem).unwrap();

    let result = db.recall(&id).unwrap().expect("should find memory");
    assert_eq!(result.memory.title, "test title");
    assert_eq!(result.memory.content, "test content");
    assert_eq!(result.memory.memory_type, MemoryType::Decision);
    assert!(!result.memory.archived);
}

// 2. Store with ALL optional fields, recall, assert each
#[test]
fn store_all_optional_fields() {
    let db = open_test_db();
    let cat_id: i64 = db
        .connection()
        .query_row("SELECT id FROM categories WHERE name='k8s'", [], |row| {
            row.get(0)
        })
        .unwrap();

    let mem = NewMemory {
        title: "full memory".into(),
        content: "full content".into(),
        memory_type: MemoryType::Architecture,
        source: Some("test-source".into()),
        importance: Importance::High,
        confidence: nous_core::types::Confidence::Low,
        tags: vec!["tag1".into()],
        workspace_path: Some("/tmp/test-workspace".into()),
        session_id: Some("sess-123".into()),
        trace_id: Some("trace-456".into()),
        agent_id: Some("agent-789".into()),
        agent_model: Some("claude-opus".into()),
        valid_from: Some("2025-01-01T00:00:00Z".into()),
        category_id: Some(cat_id),
    };
    let id = db.store(&mem).unwrap();
    let result = db.recall(&id).unwrap().unwrap();

    assert_eq!(result.memory.source.as_deref(), Some("test-source"));
    assert_eq!(result.memory.importance, Importance::High);
    assert_eq!(result.memory.confidence, nous_core::types::Confidence::Low);
    assert!(result.memory.workspace_id.is_some());
    assert_eq!(result.memory.session_id.as_deref(), Some("sess-123"));
    assert_eq!(result.memory.trace_id.as_deref(), Some("trace-456"));
    assert_eq!(result.memory.agent_id.as_deref(), Some("agent-789"));
    assert_eq!(result.memory.agent_model.as_deref(), Some("claude-opus"));
    assert_eq!(
        result.memory.valid_from.as_deref(),
        Some("2025-01-01T00:00:00Z")
    );
    assert_eq!(result.memory.category_id, Some(cat_id));
    assert!(result.category.is_some());
    assert_eq!(result.category.unwrap().name, "k8s");
}

// 3. Store with tags, query memory_tags, assert 2 rows
#[test]
fn store_with_tags() {
    let db = open_test_db();
    let mut mem = minimal_memory();
    mem.tags = vec!["rust".into(), "testing".into()];
    let id = db.store(&mem).unwrap();

    let count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_tags WHERE memory_id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

// 4. Two memories same tag, assert tag table has 1 row for that tag
#[test]
fn tags_deduplicated_across_memories() {
    let db = open_test_db();
    let mut m1 = minimal_memory();
    m1.tags = vec!["shared-tag".into()];
    db.store(&m1).unwrap();

    let mut m2 = minimal_memory();
    m2.title = "second".into();
    m2.tags = vec!["shared-tag".into()];
    db.store(&m2).unwrap();

    let count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE name = 'shared-tag'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

// 5. FTS5 trigger: MATCH query on stored memory title
#[test]
fn fts5_trigger_populates_index() {
    let db = open_test_db();
    let mut mem = minimal_memory();
    mem.title = "xylophone_unique_keyword".into();
    db.store(&mem).unwrap();

    let found: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memories_fts WHERE memories_fts MATCH 'xylophone_unique_keyword')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(found, "FTS5 index should contain the stored memory");
}

// 6. Recall non-existent ID returns None
#[test]
fn recall_nonexistent_returns_none() {
    let db = open_test_db();
    let fake_id = MemoryId::from_str("nonexistent-id").unwrap();
    let result = db.recall(&fake_id).unwrap();
    assert!(result.is_none());
}

// 7. Recall logs access_log entry
#[test]
fn recall_logs_access() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    db.recall(&id).unwrap();

    let count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM access_log WHERE memory_id = ?1 AND access_type = 'recall'",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

// 8. Update title only, recall, assert title changed and content unchanged
#[test]
fn update_title_only() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    let patch = MemoryPatch {
        title: Some("updated title".into()),
        content: None,
        tags: None,
        importance: None,
        confidence: None,
        valid_until: None,
    };
    assert!(db.update(&id, &patch).unwrap());

    let result = db.recall(&id).unwrap().unwrap();
    assert_eq!(result.memory.title, "updated title");
    assert_eq!(result.memory.content, "test content");
}

// 9. Update tags from ["a"] to ["b","c"]
#[test]
fn update_replaces_tags() {
    let db = open_test_db();
    let mut mem = minimal_memory();
    mem.tags = vec!["a".into()];
    let id = db.store(&mem).unwrap();

    let patch = MemoryPatch {
        title: None,
        content: None,
        tags: Some(vec!["b".into(), "c".into()]),
        importance: None,
        confidence: None,
        valid_until: None,
    };
    db.update(&id, &patch).unwrap();

    let result = db.recall(&id).unwrap().unwrap();
    assert_eq!(result.tags.len(), 2);
    assert!(result.tags.contains(&"b".to_string()));
    assert!(result.tags.contains(&"c".to_string()));
    assert!(!result.tags.contains(&"a".to_string()));
}

// 10. Update non-existent ID returns false
#[test]
fn update_nonexistent_returns_false() {
    let db = open_test_db();
    let fake_id = MemoryId::from_str("nonexistent-id").unwrap();
    let patch = MemoryPatch {
        title: Some("x".into()),
        content: None,
        tags: None,
        importance: None,
        confidence: None,
        valid_until: None,
    };
    assert!(!db.update(&fake_id, &patch).unwrap());
}

// 11. updated_at changes on update
#[test]
fn updated_at_changes_on_update() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    let before: String = db
        .connection()
        .query_row(
            "SELECT updated_at FROM memories WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();

    std::thread::sleep(std::time::Duration::from_millis(10));

    let patch = MemoryPatch {
        title: Some("changed".into()),
        content: None,
        tags: None,
        importance: None,
        confidence: None,
        valid_until: None,
    };
    db.update(&id, &patch).unwrap();

    let after: String = db
        .connection()
        .query_row(
            "SELECT updated_at FROM memories WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();

    assert_ne!(before, after, "updated_at should change after update");
}

// 12. Soft forget: archived=1, chunks deleted
#[test]
fn soft_forget_archives_and_deletes_chunks() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    db.connection()
        .execute(
            "INSERT INTO models (name, dimensions, max_tokens) VALUES ('test-model', 384, 512)",
            [],
        )
        .unwrap();
    let model_id: i64 = db
        .connection()
        .query_row("SELECT id FROM models WHERE name='test-model'", [], |row| {
            row.get(0)
        })
        .unwrap();
    let chunk_id = "chunk-1";
    db.connection()
        .execute(
            "INSERT INTO memory_chunks (id, memory_id, chunk_index, content, token_count, model_id) VALUES (?1, ?2, 0, 'chunk text', 10, ?3)",
            params![chunk_id, id.to_string(), model_id],
        )
        .unwrap();

    assert!(db.forget(&id, false).unwrap());

    let archived: i64 = db
        .connection()
        .query_row(
            "SELECT archived FROM memories WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 1);

    let chunk_count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_chunks WHERE memory_id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(chunk_count, 0);
}

// 13. Hard forget: row gone, cascades
#[test]
fn hard_forget_deletes_row() {
    let db = open_test_db();
    let mut mem = minimal_memory();
    mem.tags = vec!["ephemeral".into()];
    let id = db.store(&mem).unwrap();

    assert!(db.forget(&id, true).unwrap());

    let exists: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1)",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);

    let tag_links: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_tags WHERE memory_id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tag_links, 0, "cascade should delete memory_tags");
}

// 14. tags_cleanup trigger fires after hard delete (orphan tags removed)
#[test]
fn tags_cleanup_trigger_removes_orphans() {
    let db = open_test_db();
    let mut mem = minimal_memory();
    mem.tags = vec!["orphan-tag".into()];
    let id = db.store(&mem).unwrap();

    let tag_exists_before: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE name = 'orphan-tag')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(tag_exists_before);

    db.forget(&id, true).unwrap();

    let tag_exists_after: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tags WHERE name = 'orphan-tag')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        !tag_exists_after,
        "orphan tag should be removed by cleanup trigger"
    );
}

// 15. Unarchive: archived=0 after archive+unarchive
#[test]
fn unarchive_restores_memory() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();

    db.forget(&id, false).unwrap();
    let archived: i64 = db
        .connection()
        .query_row(
            "SELECT archived FROM memories WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 1);

    assert!(db.unarchive(&id).unwrap());

    let archived: i64 = db
        .connection()
        .query_row(
            "SELECT archived FROM memories WHERE id = ?1",
            params![id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 0);
}

// 16. Unarchive non-archived memory returns false
#[test]
fn unarchive_non_archived_returns_false() {
    let db = open_test_db();
    let id = db.store(&minimal_memory()).unwrap();
    assert!(!db.unarchive(&id).unwrap());
}

// 17. Relate with 'related', assert row exists
#[test]
fn relate_creates_relationship() {
    let db = open_test_db();
    let id1 = db.store(&minimal_memory()).unwrap();
    let mut m2 = minimal_memory();
    m2.title = "second".into();
    let id2 = db.store(&m2).unwrap();

    db.relate(&id1, &id2, RelationType::Related).unwrap();

    let exists: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM relationships WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'related')",
            params![id1.to_string(), id2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(exists);
}

// 18. Relate with 'supersedes', assert target valid_until is set
#[test]
fn supersedes_sets_valid_until() {
    let db = open_test_db();
    let id1 = db.store(&minimal_memory()).unwrap();
    let mut m2 = minimal_memory();
    m2.title = "old memory".into();
    let id2 = db.store(&m2).unwrap();

    let before: Option<String> = db
        .connection()
        .query_row(
            "SELECT valid_until FROM memories WHERE id = ?1",
            params![id2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(before.is_none());

    db.relate(&id1, &id2, RelationType::Supersedes).unwrap();

    let after: Option<String> = db
        .connection()
        .query_row(
            "SELECT valid_until FROM memories WHERE id = ?1",
            params![id2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(after.is_some(), "valid_until should be set on target");
}

// 19. Unrelate, assert row deleted
#[test]
fn unrelate_removes_relationship() {
    let db = open_test_db();
    let id1 = db.store(&minimal_memory()).unwrap();
    let mut m2 = minimal_memory();
    m2.title = "second".into();
    let id2 = db.store(&m2).unwrap();

    db.relate(&id1, &id2, RelationType::Related).unwrap();
    assert!(db.unrelate(&id1, &id2, RelationType::Related).unwrap());

    let exists: bool = db
        .connection()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM relationships WHERE source_id = ?1 AND target_id = ?2)",
            params![id1.to_string(), id2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
}

// 20. Duplicate relationship is idempotent (INSERT OR IGNORE)
#[test]
fn duplicate_relationship_idempotent() {
    let db = open_test_db();
    let id1 = db.store(&minimal_memory()).unwrap();
    let mut m2 = minimal_memory();
    m2.title = "second".into();
    let id2 = db.store(&m2).unwrap();

    db.relate(&id1, &id2, RelationType::Related).unwrap();
    db.relate(&id1, &id2, RelationType::Related).unwrap();

    let count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM relationships WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'related'",
            params![id1.to_string(), id2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "duplicate relationship should be ignored");
}
