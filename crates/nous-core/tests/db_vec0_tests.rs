use nous_core::chunk::Chunk;
use nous_core::db::{MemoryDb, ensure_vec0_table};
use nous_core::types::{MemoryType, NewMemory};
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

#[test]
fn ensure_vec0_creates_table() {
    let db = open_test_db();
    let exists: bool = db
        .connection()
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_embeddings'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        exists,
        "memory_embeddings table should exist after MemoryDb::open"
    );
}

#[test]
fn ensure_vec0_idempotent() {
    let db = open_test_db();
    let conn = db.connection();
    // MemoryDb::open already called ensure_vec0_table with 384
    // Calling again with same dim should not error
    ensure_vec0_table(conn, 384).unwrap();
    ensure_vec0_table(conn, 384).unwrap();
}

#[test]
fn reset_embeddings_changes_dimension() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();

    let chunks = vec![Chunk {
        idx: 0,
        start_char: 0,
        end_char: 10,
        text: "chunk zero".into(),
    }];
    let embeddings_384 = vec![vec![0.1f32; 384]];
    db.store_chunks(&memory_id, &chunks, &embeddings_384)
        .unwrap();

    // Verify 384-dim data exists
    let count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_embeddings WHERE chunk_id = ?1",
            params![format!("{}:0", memory_id)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // Reset to 1024
    db.reset_embeddings(1024).unwrap();

    // Old data should be gone (table was dropped and recreated)
    let count_after: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count_after, 0, "old embeddings should be gone after reset");

    // Inserting a 1024-dim vector should succeed
    let blob_1024: Vec<u8> = vec![0.5f32; 1024]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    db.connection()
        .execute(
            "INSERT INTO memory_embeddings (chunk_id, embedding) VALUES (?1, ?2)",
            params!["test:0", blob_1024],
        )
        .unwrap();

    let inserted: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(inserted, 1, "1024-dim insert should succeed");
}

#[test]
fn reset_embeddings_preserves_other_tables() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();

    let chunks = vec![Chunk {
        idx: 0,
        start_char: 0,
        end_char: 10,
        text: "chunk zero".into(),
    }];
    let embeddings = vec![vec![0.1f32; 384]];
    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();

    // Reset embeddings
    db.reset_embeddings(1024).unwrap();

    // Memory should still exist
    let memory = db.recall(&memory_id).unwrap();
    assert!(memory.is_some(), "memory should survive reset_embeddings");

    // Chunks should still exist (only vec0 was reset, not memory_chunks)
    let chunk_count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_chunks WHERE memory_id = ?1",
            params![memory_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        chunk_count, 1,
        "memory_chunks should survive reset_embeddings"
    );
}

#[test]
fn dimension_reset_end_to_end() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();

    // Store with 384-dim
    let chunks = vec![
        Chunk {
            idx: 0,
            start_char: 0,
            end_char: 5,
            text: "alpha".into(),
        },
        Chunk {
            idx: 1,
            start_char: 5,
            end_char: 10,
            text: "bravo".into(),
        },
    ];
    let embeddings = vec![vec![0.1f32; 384], vec![0.2f32; 384]];
    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();

    let before: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(before, 2);

    // Reset to 1024
    db.reset_embeddings(1024).unwrap();

    // Old data gone
    let after: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(after, 0, "384-dim data should be gone");

    // vec0 now accepts 1024-dim
    let blob: Vec<u8> = vec![0.3f32; 1024]
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    db.connection()
        .execute(
            "INSERT INTO memory_embeddings (chunk_id, embedding) VALUES (?1, ?2)",
            params!["new:0", blob],
        )
        .unwrap();

    let final_count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(final_count, 1);
}
