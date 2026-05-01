use nous_core::db::{create_vec_pool, DbPools, VecPool, EMBEDDING_DIMENSION};
use nous_core::memory::{self, MemoryType, SaveMemoryRequest};
use tempfile::TempDir;

fn create_test_vec_pool(tmp: &TempDir) -> VecPool {
    let path = tmp.path().join("test-vec.db");
    create_vec_pool(&path).unwrap()
}

#[test]
fn sqlite_vec_extension_loads() {
    let tmp = TempDir::new().unwrap();
    let vec_pool = create_test_vec_pool(&tmp);
    let conn = vec_pool.lock().unwrap();

    let version: String = conn
        .query_row("SELECT vec_version()", [], |row| row.get(0))
        .unwrap();
    assert!(!version.is_empty());
}

#[test]
fn vec0_table_creation() {
    let tmp = TempDir::new().unwrap();
    let pools_future = DbPools::connect(tmp.path());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let pools = rt.block_on(pools_future).unwrap();
    rt.block_on(pools.run_migrations("porter unicode61"))
        .unwrap();

    let conn = pools.vec.lock().unwrap();
    let table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='memory_embeddings')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(table_exists);
}

#[test]
fn insert_and_knn_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let vec_pool = create_test_vec_pool(&tmp);

    // Run vec migrations
    {
        let conn = vec_pool.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vec_schema_version (id INTEGER PRIMARY KEY, version TEXT NOT NULL);\
             CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(\
             memory_id TEXT PRIMARY KEY, embedding float[384]);",
        )
        .unwrap();
    }

    // Insert embeddings
    let mut embedding = vec![0.0f32; EMBEDDING_DIMENSION];
    embedding[0] = 1.0;
    let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

    {
        let conn = vec_pool.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params!["mem-001", bytes],
        )
        .unwrap();
    }

    // Query KNN
    let mut query_embedding = vec![0.0f32; EMBEDDING_DIMENSION];
    query_embedding[0] = 0.9;
    query_embedding[1] = 0.1;
    let query_bytes: Vec<u8> = query_embedding
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();

    let conn = vec_pool.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, distance FROM memory_embeddings \
             WHERE embedding MATCH ?1 ORDER BY distance LIMIT 5",
        )
        .unwrap();

    let results: Vec<(String, f32)> = stmt
        .query_map(rusqlite::params![query_bytes], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "mem-001");
    assert!(results[0].1 >= 0.0);
}

#[test]
fn upsert_replaces_embedding() {
    let tmp = TempDir::new().unwrap();
    let vec_pool = create_test_vec_pool(&tmp);

    {
        let conn = vec_pool.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vec_schema_version (id INTEGER PRIMARY KEY, version TEXT NOT NULL);\
             CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(\
             memory_id TEXT PRIMARY KEY, embedding float[384]);",
        )
        .unwrap();
    }

    let mut emb1 = vec![0.0f32; EMBEDDING_DIMENSION];
    emb1[0] = 1.0;
    let bytes1: Vec<u8> = emb1.iter().flat_map(|f| f.to_le_bytes()).collect();

    let mut emb2 = vec![0.0f32; EMBEDDING_DIMENSION];
    emb2[1] = 1.0;
    let bytes2: Vec<u8> = emb2.iter().flat_map(|f| f.to_le_bytes()).collect();

    {
        let conn = vec_pool.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params!["mem-001", bytes1],
        )
        .unwrap();

        // Upsert: delete + re-insert
        conn.execute(
            "DELETE FROM memory_embeddings WHERE memory_id = ?1",
            rusqlite::params!["mem-001"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params!["mem-001", bytes2],
        )
        .unwrap();
    }

    // Query with emb2-like vector — should find the updated embedding
    let mut query = vec![0.0f32; EMBEDDING_DIMENSION];
    query[1] = 1.0;
    let query_bytes: Vec<u8> = query.iter().flat_map(|f| f.to_le_bytes()).collect();

    let conn = vec_pool.lock().unwrap();
    let (id, distance): (String, f32) = conn
        .query_row(
            "SELECT memory_id, distance FROM memory_embeddings \
             WHERE embedding MATCH ?1 ORDER BY distance LIMIT 1",
            rusqlite::params![query_bytes],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(id, "mem-001");
    assert!(distance < 0.01); // Should be very close (near-zero distance)
}

#[test]
fn knn_returns_correct_top_k() {
    let tmp = TempDir::new().unwrap();
    let vec_pool = create_test_vec_pool(&tmp);

    {
        let conn = vec_pool.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS vec_schema_version (id INTEGER PRIMARY KEY, version TEXT NOT NULL);\
             CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(\
             memory_id TEXT PRIMARY KEY, embedding float[384]);",
        )
        .unwrap();
    }

    // Insert 5 embeddings at different positions
    for i in 0..5u32 {
        let mut emb = vec![0.0f32; EMBEDDING_DIMENSION];
        emb[i as usize] = 1.0;
        let bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();

        let conn = vec_pool.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![format!("mem-{:03}", i), bytes],
        )
        .unwrap();
    }

    // Query close to mem-002 (index 2)
    let mut query = vec![0.0f32; EMBEDDING_DIMENSION];
    query[2] = 1.0;
    let query_bytes: Vec<u8> = query.iter().flat_map(|f| f.to_le_bytes()).collect();

    let conn = vec_pool.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, distance FROM memory_embeddings \
             WHERE embedding MATCH ?1 ORDER BY distance LIMIT 3",
        )
        .unwrap();

    let results: Vec<(String, f32)> = stmt
        .query_map(rusqlite::params![query_bytes], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    assert_eq!(results.len(), 3);
    // First result should be mem-002 with near-zero distance
    assert_eq!(results[0].0, "mem-002");
    assert!(results[0].1 < 0.01);
    // Other results should have larger distance
    assert!(results[1].1 > results[0].1);
}

#[tokio::test]
async fn store_and_search_via_api() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let mem = memory::save_memory(
        &pools.fts,
        SaveMemoryRequest {
            workspace_id: Some("ws-1".into()),
            agent_id: None,
            title: "API test".into(),
            content: "Test via store+search API".into(),
            memory_type: MemoryType::Fact,
            importance: None,
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    let mut embedding = vec![0.0f32; EMBEDDING_DIMENSION];
    embedding[0] = 1.0;

    memory::store_embedding(&pools.fts, &pools.vec, &mem.id, &embedding)
        .await
        .unwrap();

    let mut query = vec![0.0f32; EMBEDDING_DIMENSION];
    query[0] = 0.9;
    query[1] = 0.1;

    let results = memory::search_similar(&pools.fts, &pools.vec, &query, 10, Some("ws-1"), None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].memory.id, mem.id);
    assert!(results[0].score > 0.0);
}
