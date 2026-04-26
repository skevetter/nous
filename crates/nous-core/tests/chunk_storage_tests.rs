use nous_core::chunk::Chunk;
use nous_core::db::MemoryDb;
use nous_core::types::{MemoryType, NewMemory};
use rusqlite::params;

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None).expect("failed to open in-memory db")
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

fn make_embeddings(n: usize, dims: usize) -> Vec<Vec<f32>> {
    (0..n)
        .map(|i| (0..dims).map(|d| (i * dims + d) as f32).collect())
        .collect()
}

// 1. Store 3 chunks, query memory_chunks, assert 3 rows with correct chunk_index
#[test]
fn store_chunks_inserts_correct_rows() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(3);
    let embeddings = make_embeddings(3, 4);

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

// 2. Assert memory_embeddings has 3 rows with correct chunk_ids
#[test]
fn store_chunks_inserts_embeddings() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(3);
    let embeddings = make_embeddings(3, 4);

    db.store_chunks(&memory_id, &chunks, &embeddings).unwrap();

    let mut stmt = db
        .connection()
        .prepare("SELECT chunk_id FROM memory_embeddings ORDER BY chunk_id")
        .unwrap();
    let chunk_ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(chunk_ids.len(), 3);
    for i in 0..3 {
        assert_eq!(chunk_ids[i], format!("{}:{}", memory_id, i));
    }
}

// 3. Delete chunks, assert 0 rows in both tables
#[test]
fn delete_chunks_removes_all() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(3);
    let embeddings = make_embeddings(3, 4);

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

    let emb_count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_embeddings WHERE chunk_id LIKE ?1",
            params![format!("{}:%", memory_id)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(emb_count, 0);
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
    let embeddings = make_embeddings(2, 4);

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

    let emb_count2: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_embeddings WHERE chunk_id LIKE ?1",
            params![format!("{}:%", id2)],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(emb_count2, 2);
}

// 5. Embedding BLOB round-trips correctly
#[test]
fn embedding_blob_roundtrips() {
    let db = open_test_db();
    let memory_id = db.store(&minimal_memory()).unwrap();
    let chunks = make_chunks(1);
    let original: Vec<f32> = vec![1.0, 2.5, -3.14, 0.0, f32::MAX, f32::MIN];
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
