use nous_core::chunk::Chunker;
use nous_core::db::MemoryDb;
use nous_core::embed::{EmbeddingBackend, MockEmbedding};
use nous_core::types::{NewMemory, RelationType};
use nous_mcp::commands::{build_export_data, import_data};
use rusqlite::params;

fn test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None, 384).unwrap()
}

fn mock_embedding() -> MockEmbedding {
    MockEmbedding::new(384)
}

fn store_memory(
    db: &MemoryDb,
    title: &str,
    content: &str,
    memory_type: nous_core::types::MemoryType,
    tags: Vec<String>,
    workspace_path: Option<&str>,
    importance: nous_core::types::Importance,
) -> nous_shared::ids::MemoryId {
    let memory = NewMemory {
        title: title.into(),
        content: content.into(),
        memory_type,
        source: Some("integration-test".into()),
        importance,
        confidence: nous_core::types::Confidence::Moderate,
        tags,
        workspace_path: workspace_path.map(|s| s.into()),
        session_id: Some("sess-001".into()),
        trace_id: None,
        agent_id: Some("test-agent".into()),
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    db.store(&memory).unwrap()
}

#[test]
fn export_import_round_trip_preserves_all_data() {
    let src_db = test_db();
    let embedding = mock_embedding();
    let chunker = Chunker::new(512, 64);

    // 1. Store 5 memories with different types, tags, and workspaces
    let id1 = store_memory(
        &src_db,
        "Rust error handling convention",
        "Use thiserror for library errors and anyhow for application errors",
        nous_core::types::MemoryType::Convention,
        vec!["rust".into(), "errors".into()],
        Some("/home/dev/myproject"),
        nous_core::types::Importance::High,
    );

    let id2 = store_memory(
        &src_db,
        "Database connection pooling decision",
        "We chose r2d2 for sync connection pooling with SQLite",
        nous_core::types::MemoryType::Decision,
        vec!["database".into(), "architecture".into()],
        Some("/home/dev/myproject"),
        nous_core::types::Importance::Moderate,
    );

    let id3 = store_memory(
        &src_db,
        "Fix: NULL workspace crash",
        "Fixed a crash when workspace_id was NULL in the join query",
        nous_core::types::MemoryType::Bugfix,
        vec!["bugfix".into(), "database".into()],
        Some("/home/dev/other-project"),
        nous_core::types::Importance::Moderate,
    );

    let id4 = store_memory(
        &src_db,
        "Service architecture overview",
        "The system uses a layered architecture: MCP server -> core library -> SQLite",
        nous_core::types::MemoryType::Architecture,
        vec!["architecture".into()],
        None,
        nous_core::types::Importance::High,
    );

    let id5 = store_memory(
        &src_db,
        "Embedding model observation",
        "MockEmbedding produces deterministic vectors suitable for testing",
        nous_core::types::MemoryType::Observation,
        vec!["testing".into(), "embedding".into()],
        None,
        nous_core::types::Importance::Low,
    );

    // Generate chunks/embeddings for all memories
    for id in [&id1, &id2, &id3, &id4, &id5] {
        let recalled = src_db.recall(id).unwrap().unwrap();
        let chunks = chunker.chunk(&recalled.memory.content);
        if !chunks.is_empty() {
            let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            let embeddings = embedding.embed(&texts).unwrap();
            src_db.store_chunks(id, &chunks, &embeddings).unwrap();
        }
    }

    // 2. Create relationships between memories
    src_db.relate(&id1, &id2, RelationType::Related).unwrap();
    src_db.relate(&id2, &id4, RelationType::DependsOn).unwrap();
    src_db.relate(&id3, &id2, RelationType::Supersedes).unwrap();

    // Verify supersedes side-effect: id2 should now have valid_until set
    let superseded = src_db.recall(&id2).unwrap().unwrap();
    assert!(
        superseded.memory.valid_until.is_some(),
        "supersedes should set valid_until on target"
    );

    // 3. Add a custom category and assign it to a memory
    let custom_cat_id = src_db
        .category_suggest(
            "rust-conventions",
            Some("Conventions for Rust code style and patterns"),
            None,
            &id1,
        )
        .unwrap();
    assert!(custom_cat_id > 0);

    // Count source data
    let src_conn = src_db.connection();
    let src_memory_count: i64 = src_conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE archived = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let src_tag_count: i64 = src_conn
        .query_row(
            "SELECT COUNT(DISTINCT t.name) FROM tags t JOIN memory_tags mt ON mt.tag_id = t.id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let src_rel_count: i64 = src_conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap();
    let src_cat_count: i64 = src_conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE source = 'agent'",
            [],
            |r| r.get(0),
        )
        .unwrap();

    assert_eq!(src_memory_count, 5);
    assert_eq!(src_rel_count, 3);
    assert!(src_cat_count >= 1);

    // 4. Export to JSON buffer
    let export_data = build_export_data(&src_db).unwrap();

    assert_eq!(export_data.version, 1);
    assert_eq!(export_data.memories.len(), 5);
    assert!(
        export_data
            .categories
            .iter()
            .any(|c| c.name == "rust-conventions"),
        "export should include agent-created category"
    );

    let json_buf = serde_json::to_vec_pretty(&export_data).unwrap();

    // 5. Create fresh DB and import
    let dest_db = test_db();
    let reimported: nous_mcp::commands::ExportData = serde_json::from_slice(&json_buf).unwrap();
    import_data(&dest_db, &reimported, &embedding, &chunker).unwrap();

    // 6. Verify counts match
    let dest_conn = dest_db.connection();

    let dest_memory_count: i64 = dest_conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE archived = 0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        dest_memory_count, src_memory_count,
        "memory count should match after import"
    );

    let dest_tag_count: i64 = dest_conn
        .query_row(
            "SELECT COUNT(DISTINCT t.name) FROM tags t JOIN memory_tags mt ON mt.tag_id = t.id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        dest_tag_count, src_tag_count,
        "unique tag count should match after import"
    );

    let dest_rel_count: i64 = dest_conn
        .query_row("SELECT COUNT(*) FROM relationships", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        dest_rel_count, src_rel_count,
        "relationship count should match after import"
    );

    let dest_cat_exists: bool = dest_conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM categories WHERE name = 'rust-conventions')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        dest_cat_exists,
        "agent-created category should survive import"
    );

    // 7. Spot-check memory #3 (bugfix)
    let m3_export = export_data
        .memories
        .iter()
        .find(|m| m.title == "Fix: NULL workspace crash")
        .unwrap();

    let dest_m3_title: String = dest_conn
        .query_row(
            "SELECT m.title FROM memories m
             JOIN memory_tags mt ON mt.memory_id = m.id
             JOIN tags t ON t.id = mt.tag_id
             WHERE t.name = 'bugfix' AND m.archived = 0
             LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dest_m3_title, "Fix: NULL workspace crash");

    let dest_m3_content: String = dest_conn
        .query_row(
            "SELECT content FROM memories WHERE title = ?1",
            params![m3_export.title],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        dest_m3_content,
        "Fixed a crash when workspace_id was NULL in the join query"
    );

    let dest_m3_tags: Vec<String> = {
        let mut stmt = dest_conn
            .prepare(
                "SELECT t.name FROM tags t
                 JOIN memory_tags mt ON mt.tag_id = t.id
                 JOIN memories m ON m.id = mt.memory_id
                 WHERE m.title = ?1
                 ORDER BY t.name",
            )
            .unwrap();
        stmt.query_map(params![m3_export.title], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(dest_m3_tags, vec!["bugfix", "database"]);

    // 8. Verify relationships preserved including supersedes side-effects
    let has_related: bool = dest_conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM relationships WHERE relation_type = 'related')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(has_related, "related relationship should be preserved");

    let has_depends: bool = dest_conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM relationships WHERE relation_type = 'depends_on')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(has_depends, "depends_on relationship should be preserved");

    let has_supersedes: bool = dest_conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM relationships WHERE relation_type = 'supersedes')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        has_supersedes,
        "supersedes relationship should be preserved"
    );

    // Verify supersedes side-effect in destination: the target memory should have valid_until set
    // The import calls db.relate() which triggers the supersedes side-effect
    let superseded_count: i64 = dest_conn
        .query_row(
            "SELECT COUNT(*) FROM memories m
             JOIN relationships r ON r.target_id = m.id
             WHERE r.relation_type = 'supersedes' AND m.valid_until IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        superseded_count, 1,
        "supersedes target should have valid_until set after import"
    );

    // 9. Verify chunks survived import
    let dest_chunk_count: i64 = dest_conn
        .query_row("SELECT COUNT(*) FROM memory_chunks", [], |r| r.get(0))
        .unwrap();
    assert!(dest_chunk_count > 0, "imported memories should have chunks");
}
