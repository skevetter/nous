use nous_core::chunk::Chunk;
use nous_core::db::MemoryDb;
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

fn make_chunks(n: usize) -> Vec<Chunk> {
    (0..n)
        .map(|i| Chunk {
            idx: i,
            start_char: i * 10,
            end_char: (i + 1) * 10,
            text: format!("chunk text number {i}"),
        })
        .collect()
}

const VEC_DIMS: usize = 384;

fn make_embeddings(n: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| (0..VEC_DIMS).map(|d| (i * VEC_DIMS + d) as f32).collect())
        .collect()
}

// 1. Store 3 chunks, query memory_chunks, assert 3 rows with correct chunk_index
#[test]
fn store_chunks_inserts_correct_rows() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(3);
    let embeddings = make_embeddings(3);

    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();

    let mut stmt = db
        .connection()
        .prepare(
            "SELECT id, chunk_index FROM memory_chunks WHERE memory_id = ?1 ORDER BY chunk_index",
        )
        .unwrap();
    let rows: Vec<(String, i64)> = stmt
        .query_map(params![memory_id.to_string()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(rows.len(), 3);
    for (i, (id, chunk_index)) in rows.iter().enumerate() {
        assert_eq!(*chunk_index, i as i64);
        assert_eq!(*id, format!("{}:{}", memory_id, i));
    }
}

// 2. Assert memory_embeddings has rows for each chunk_id via point lookup
#[test]
fn store_chunks_inserts_embeddings() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(3);
    let embeddings = make_embeddings(3);

    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();

    for i in 0..3 {
        let chunk_id = format!("{}:{}", memory_id, i);
        let exists: bool = db
            .connection()
            .query_row(
                "SELECT COUNT(*) > 0 FROM memory_embeddings WHERE chunk_id = ?1",
                params![chunk_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "embedding missing for chunk {chunk_id}");
    }
}

// 3. Delete chunks, assert 0 rows in both tables
#[test]
fn delete_chunks_removes_all() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(3);
    let embeddings = make_embeddings(3);

    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();
    db.delete_chunks(&memory_id).unwrap();

    let chunk_count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_chunks WHERE memory_id = ?1",
            params![memory_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(chunk_count, 0);

    for i in 0..3 {
        let chunk_id = format!("{}:{}", memory_id, i);
        let exists: bool = db
            .connection()
            .query_row(
                "SELECT COUNT(*) > 0 FROM memory_embeddings WHERE chunk_id = ?1",
                params![chunk_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!exists, "embedding for {chunk_id} should be deleted");
    }
}

// 4. Store chunks for two memories, delete one, assert other's chunks intact
#[test]
fn delete_chunks_isolated_per_memory() {
    let db = open_test_db();
    let id1 = db.store(&minimal_memory()).unwrap();
    let mut mem2 = minimal_memory();
    mem2.title = "second".into();
    let id2 = db.store(&mem2).unwrap();

    let chunks = make_chunks(2);
    let embeddings = make_embeddings(2);

    db.store_chunks(&id1, &chunks, &embeddings).unwrap();
    db.store_chunks(&id2, &chunks, &embeddings).unwrap();

    db.delete_chunks(&id1).unwrap();

    let count1: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_chunks WHERE memory_id = ?1",
            params![id1.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count1, 0);

    let count2: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_chunks WHERE memory_id = ?1",
            params![id2.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count2, 2);

    for i in 0..2 {
        let chunk_id = format!("{}:{}", id2, i);
        let exists: bool = db
            .connection()
            .query_row(
                "SELECT COUNT(*) > 0 FROM memory_embeddings WHERE chunk_id = ?1",
                params![chunk_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "embedding for {chunk_id} should still exist");
    }
}

// 5. Embedding BLOB round-trips correctly
#[test]
fn embedding_blob_roundtrips() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(1);
    let mut original: Vec<f32> = (0..VEC_DIMS).map(|i| (i as f32) * 0.01).collect();
    original[0] = 1.0;
    original[1] = 2.5;
    original[2] = -3.15;
    original[3] = 0.0;
    let embeddings = vec![original.clone()];

    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();

    let chunk_id = format!("{}:0", memory_id);
    let blob: Vec<u8> = db
        .connection()
        .query_row(
            "SELECT embedding FROM memory_embeddings WHERE chunk_id = ?1",
            params![chunk_id],
            |row| row.get(0),
        )
        .unwrap();

    let recovered: Vec<f32> = blob
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();

    assert_eq!(recovered, original);
}
