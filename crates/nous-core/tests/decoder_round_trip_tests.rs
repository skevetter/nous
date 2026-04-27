use nous_core::chunk::Chunk;
use nous_core::db::MemoryDb;
use nous_core::embed::{EmbeddingBackend, MockEmbedding};
use nous_core::types::{MemoryType, NewMemory};
use rusqlite::params;

fn new_memory(title: &str, content: &str) -> NewMemory {
    NewMemory {
        title: title.into(),
        content: content.into(),
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
    }
}

fn store_text(db: &MemoryDb, backend: &MockEmbedding, idx: usize, text: &str) {
    let mem = new_memory(&format!("memory-{idx}"), text);
    let id = db.store(&mem).unwrap();
    let embedding = backend.embed_one(text).unwrap();
    let chunk = Chunk {
        idx: 0,
        start_char: 0,
        end_char: text.len(),
        text: text.to_string(),
    };
    db.store_chunks(&id, &[chunk], &[embedding]).unwrap();
}

fn knn_query(db: &MemoryDb, query_vec: &[f32], k: usize) -> Vec<(String, f64)> {
    let query_blob: Vec<u8> = query_vec.iter().flat_map(|f| f.to_le_bytes()).collect();
    let sql = format!(
        "SELECT mc.content, me.distance FROM memory_embeddings me
         JOIN memory_chunks mc ON mc.id = me.chunk_id
         WHERE me.embedding MATCH ?1 AND me.k = {k}
         ORDER BY me.distance"
    );
    let mut stmt = db.connection().prepare(&sql).unwrap();
    stmt.query_map(params![query_blob], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn decoder_embed_store_knn_round_trip() {
    let backend = MockEmbedding::new(1024);
    let db = MemoryDb::open(":memory:", None, 1024).unwrap();

    let texts = [
        "Rust programming language",
        "The weather in Tokyo",
        "Chocolate cake recipe",
    ];

    for (i, text) in texts.iter().enumerate() {
        store_text(&db, &backend, i, text);
    }

    let query_vec = backend.embed_one("Rust programming language").unwrap();
    let results = knn_query(&db, &query_vec, 3);

    assert_eq!(results.len(), 3, "should return all 3 stored memories");
    assert_eq!(
        results[0].0, "Rust programming language",
        "exact-match query should return itself first, got '{}'",
        results[0].0
    );
    assert!(
        results[0].1 < results[1].1,
        "exact-match distance ({}) should be less than second ({})",
        results[0].1,
        results[1].1
    );
}

#[test]
fn decoder_dimension_reset_preserves_search() {
    let encoder_backend = MockEmbedding::new(384);
    let db = MemoryDb::open(":memory:", None, 384).unwrap();

    store_text(&db, &encoder_backend, 0, "Old encoder memory");

    db.reset_embeddings(1024).unwrap();

    let decoder_backend = MockEmbedding::new(1024);
    let texts = [
        "Quantum computing fundamentals",
        "Machine learning pipelines",
        "Database indexing strategies",
    ];
    for (i, text) in texts.iter().enumerate() {
        store_text(&db, &decoder_backend, i + 1, text);
    }

    let query_vec = decoder_backend
        .embed_one("Deep learning and neural networks")
        .unwrap();
    let results = knn_query(&db, &query_vec, 3);

    assert_eq!(results.len(), 3, "should return 3 results after reset");
    for (content, _distance) in &results {
        assert_ne!(
            content, "Old encoder memory",
            "old 384-dim embedding should not appear in 1024-dim results"
        );
    }
    assert!(
        results[0].1 <= results[1].1 && results[1].1 <= results[2].1,
        "distances should be non-decreasing: {:?}",
        results.iter().map(|r| r.1).collect::<Vec<_>>()
    );
}

#[test]
fn decoder_large_batch_knn() {
    let backend = MockEmbedding::new(1024);
    let db = MemoryDb::open(":memory:", None, 1024).unwrap();

    let texts = [
        "Rust ownership and borrowing",
        "Python data science libraries",
        "JavaScript async await patterns",
        "Go concurrency with goroutines",
        "TypeScript type system features",
        "C++ memory management techniques",
        "Java virtual machine internals",
        "Haskell functional programming",
        "Swift protocol-oriented design",
        "Kotlin coroutines for Android",
        "Ruby metaprogramming patterns",
        "Elixir OTP supervision trees",
    ];

    for (i, text) in texts.iter().enumerate() {
        store_text(&db, &backend, i, text);
    }

    let query_vec = backend.embed_one("Rust ownership and borrowing").unwrap();
    let results = knn_query(&db, &query_vec, 5);

    assert_eq!(results.len(), 5, "should return exactly k=5 results");
    assert_eq!(
        results[0].0, "Rust ownership and borrowing",
        "exact-match query should return itself first, got '{}'",
        results[0].0
    );
    for window in results.windows(2) {
        assert!(
            window[0].1 <= window[1].1,
            "distances should be non-decreasing: {} > {}",
            window[0].1,
            window[1].1
        );
    }
}
