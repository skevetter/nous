use std::path::Path;

use nous_core::chunk::Chunk;
use nous_core::db::MemoryDb;
use nous_core::embed::{EmbeddingBackend, FixtureEmbedding};
use nous_core::types::{MemoryType, NewMemory};
use rusqlite::params;

fn fixture_path() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/embedding_vectors.json"
    ))
}

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

#[test]
fn encoder_embed_store_knn_round_trip() {
    let backend = FixtureEmbedding::load(fixture_path()).unwrap();
    let db = MemoryDb::open(":memory:", None).unwrap();

    let texts = [
        "Rust programming language",
        "The weather in Tokyo",
        "Chocolate cake recipe",
    ];

    let mut memory_ids = Vec::new();
    for (i, text) in texts.iter().enumerate() {
        let mem = new_memory(&format!("memory-{i}"), text);
        let id = db.store(&mem).unwrap();

        let embedding = backend.embed_one(text).unwrap();
        let chunk = Chunk {
            idx: 0,
            start_char: 0,
            end_char: text.len(),
            text: text.to_string(),
        };
        db.store_chunks(&id, &[chunk], &[embedding]).unwrap();
        memory_ids.push(id);
    }

    // Query: embed "Writing code in Rust" — should be most similar to "Rust programming language"
    let query_text = "Writing code in Rust";
    let query_vec = backend.embed_one(query_text).unwrap();
    let query_blob: Vec<u8> = query_vec.iter().flat_map(|f| f.to_le_bytes()).collect();

    let mut stmt = db
        .connection()
        .prepare(
            "SELECT mc.content, me.distance
             FROM memory_embeddings me
             JOIN memory_chunks mc ON mc.id = me.chunk_id
             WHERE me.embedding MATCH ?1 AND me.k = 3
             ORDER BY me.distance",
        )
        .unwrap();

    let results: Vec<(String, f64)> = stmt
        .query_map(params![query_blob], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(results.len(), 3, "should return all 3 stored memories");
    assert_eq!(
        results[0].0, "Rust programming language",
        "closest match to '{}' should be 'Rust programming language', got '{}'",
        query_text, results[0].0
    );
    assert!(
        results[0].1 < results[1].1,
        "first result distance ({}) should be less than second ({})",
        results[0].1,
        results[1].1
    );
}
