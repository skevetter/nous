use nous_core::db::{DbPools, EMBEDDING_DIMENSION};
use nous_core::memory::embed::Embedder;
use nous_core::memory::{
    self, Chunk, Chunker, Importance, MemoryType, MockEmbedder, SaveMemoryRequest,
};
use tempfile::TempDir;

async fn setup() -> (DbPools, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools, tmp)
}

// --- Chunking tests ---

#[test]
fn chunker_default_params() {
    let chunker = Chunker::default();
    assert_eq!(chunker.chunk_size, 256);
    assert_eq!(chunker.overlap, 64);
}

#[test]
fn chunker_empty_text() {
    let chunker = Chunker::default();
    let chunks = chunker.chunk("mem-1", "");
    assert!(chunks.is_empty());
}

#[test]
fn chunker_small_text_single_chunk() {
    let chunker = Chunker::new(10, 2);
    let text = "one two three";
    let chunks = chunker.chunk("mem-1", text);
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].content, text);
    assert_eq!(chunks[0].id, "mem-1_chunk_0");
    assert_eq!(chunks[0].index, 0);
    assert_eq!(chunks[0].start_offset, 0);
    assert_eq!(chunks[0].end_offset, text.len());
}

#[test]
fn chunker_overlap_behavior() {
    let chunker = Chunker::new(4, 2);
    // 8 tokens: step = 4-2 = 2, so we get multiple chunks
    let text = "a b c d e f g h";
    let chunks = chunker.chunk("mem-1", text);

    assert!(chunks.len() >= 2);

    // Verify overlap: tokens from chunk 0 end should appear at chunk 1 start
    // chunk 0: tokens [0..4] = "a b c d"
    // chunk 1: tokens [2..6] = "c d e f" (overlaps with "c d")
    let c0_words: Vec<&str> = chunks[0].content.split_whitespace().collect();
    let c1_words: Vec<&str> = chunks[1].content.split_whitespace().collect();

    // Last 2 words of chunk 0 should be first 2 words of chunk 1
    assert_eq!(&c0_words[2..4], &c1_words[0..2]);
}

#[test]
fn chunker_offsets_map_correctly() {
    let chunker = Chunker::new(3, 1);
    let text = "hello world foo bar baz qux";
    let chunks = chunker.chunk("mem-1", text);

    for chunk in &chunks {
        assert_eq!(chunk.content, &text[chunk.start_offset..chunk.end_offset]);
    }
}

#[test]
fn chunker_whitespace_only() {
    let chunker = Chunker::default();
    let chunks = chunker.chunk("mem-1", "   \t\n  ");
    assert!(chunks.is_empty());
}

// --- Chunk storage tests ---

#[tokio::test]
async fn store_and_retrieve_chunks() {
    let (pools, _tmp) = setup().await;

    let chunks = vec![
        Chunk {
            id: "mem-1_chunk_0".to_string(),
            memory_id: "mem-1".to_string(),
            content: "first chunk content".to_string(),
            index: 0,
            start_offset: 0,
            end_offset: 19,
        },
        Chunk {
            id: "mem-1_chunk_1".to_string(),
            memory_id: "mem-1".to_string(),
            content: "second chunk content".to_string(),
            index: 1,
            start_offset: 15,
            end_offset: 35,
        },
    ];

    memory::store_chunks(&pools.vec, &chunks).unwrap();
    let retrieved = memory::get_chunks_for_memory(&pools.vec, "mem-1").unwrap();

    assert_eq!(retrieved.len(), 2);
    assert_eq!(retrieved[0].id, "mem-1_chunk_0");
    assert_eq!(retrieved[0].content, "first chunk content");
    assert_eq!(retrieved[0].index, 0);
    assert_eq!(retrieved[1].id, "mem-1_chunk_1");
    assert_eq!(retrieved[1].index, 1);
}

#[tokio::test]
async fn delete_chunks_removes_all() {
    let (pools, _tmp) = setup().await;

    let chunks = vec![
        Chunk {
            id: "mem-2_chunk_0".to_string(),
            memory_id: "mem-2".to_string(),
            content: "chunk zero".to_string(),
            index: 0,
            start_offset: 0,
            end_offset: 10,
        },
        Chunk {
            id: "mem-2_chunk_1".to_string(),
            memory_id: "mem-2".to_string(),
            content: "chunk one".to_string(),
            index: 1,
            start_offset: 8,
            end_offset: 17,
        },
    ];

    memory::store_chunks(&pools.vec, &chunks).unwrap();
    memory::delete_chunks(&pools.vec, "mem-2").unwrap();

    let retrieved = memory::get_chunks_for_memory(&pools.vec, "mem-2").unwrap();
    assert!(retrieved.is_empty());
}

#[tokio::test]
async fn store_chunk_embedding_and_search() {
    let (pools, _tmp) = setup().await;

    // Create a memory first
    let mem = memory::save_memory(
        &pools.fts,
        SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Test memory for chunks".into(),
            content: "This memory has chunks with embeddings".into(),
            memory_type: MemoryType::Fact,
            importance: Some(Importance::Moderate),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    // Store a chunk
    let chunks = vec![Chunk {
        id: format!("{}_chunk_0", mem.id),
        memory_id: mem.id.clone(),
        content: "chunk with embedding".to_string(),
        index: 0,
        start_offset: 0,
        end_offset: 20,
    }];
    memory::store_chunks(&pools.vec, &chunks).unwrap();

    // Store embedding for the chunk
    let mut embedding = vec![0.0f32; EMBEDDING_DIMENSION];
    embedding[0] = 1.0;
    memory::store_chunk_embedding(&pools.vec, &chunks[0].id, &embedding).unwrap();

    // Verify KNN search finds it
    let mut query = vec![0.0f32; EMBEDDING_DIMENSION];
    query[0] = 0.9;
    query[1] = 0.1;

    let conn = pools.vec.lock().unwrap();
    let query_bytes: Vec<u8> = query.iter().flat_map(|f| f.to_le_bytes()).collect();
    let result: String = conn
        .query_row(
            "SELECT memory_id FROM memory_embeddings WHERE embedding MATCH ?1 ORDER BY distance LIMIT 1",
            rusqlite::params![query_bytes],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(result, chunks[0].id);
}

// --- RRF reranking tests ---

#[test]
fn rerank_rrf_empty() {
    let result = memory::rerank_rrf(&[], &[], None);
    assert!(result.is_empty());
}

#[test]
fn rerank_rrf_deduplication_and_boost() {
    use nous_core::memory::{Memory, SimilarMemory};

    let make_mem = |id: &str, score: f32| SimilarMemory {
        memory: Memory {
            id: id.to_string(),
            workspace_id: "default".to_string(),
            agent_id: None,
            title: format!("Memory {id}"),
            content: format!("Content {id}"),
            memory_type: "fact".to_string(),
            importance: "moderate".to_string(),
            topic_key: None,
            valid_from: None,
            valid_until: None,
            archived: false,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        },
        score,
    };

    let fts = vec![make_mem("shared", 0.9), make_mem("fts-only", 0.8)];
    let vec_results = vec![make_mem("shared", 0.95), make_mem("vec-only", 0.85)];

    let result = memory::rerank_rrf(&fts, &vec_results, None);

    assert_eq!(result.len(), 3); // shared, fts-only, vec-only (deduplicated)
    assert_eq!(result[0].memory.id, "shared"); // boosted by appearing in both
}

// --- Hybrid search end-to-end test (mock embeddings) ---

#[tokio::test]
async fn hybrid_search_end_to_end() {
    let (pools, _tmp) = setup().await;

    // Store a memory with known content
    let mem = memory::save_memory(
        &pools.fts,
        SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Rust async patterns".into(),
            content: "Tokio runtime provides async task execution for Rust programs".into(),
            memory_type: MemoryType::Convention,
            importance: Some(Importance::High),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    // Store an embedding for it (simulating what store_with_embedding would do)
    let mut embedding = vec![0.0f32; EMBEDDING_DIMENSION];
    embedding[0] = 0.8;
    embedding[1] = 0.6;
    memory::store_embedding(&pools.fts, &pools.vec, &mem.id, &embedding)
        .await
        .unwrap();

    // Search with a query that matches both FTS and vector
    let mut query_embedding = vec![0.0f32; EMBEDDING_DIMENSION];
    query_embedding[0] = 0.7;
    query_embedding[1] = 0.7;

    let results = memory::search_hybrid_filtered(
        &pools.fts,
        &pools.vec,
        "async Rust",
        &query_embedding,
        10,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    assert!(!results.is_empty());
    assert_eq!(results[0].memory.id, mem.id);
}

// --- Mock embedder tests ---

#[test]
fn mock_embedder_correct_dimension() {
    let embedder = MockEmbedder::new();
    let results = embedder.embed(&["test"]).unwrap();
    assert_eq!(results[0].len(), EMBEDDING_DIMENSION);
}

#[test]
fn mock_embedder_batch_consistency() {
    let embedder = MockEmbedder::new();
    let batch = embedder.embed(&["hello", "world", "test"]).unwrap();
    assert_eq!(batch.len(), 3);

    let single1 = embedder.embed(&["hello"]).unwrap();
    let single2 = embedder.embed(&["world"]).unwrap();

    assert_eq!(batch[0], single1[0]);
    assert_eq!(batch[1], single2[0]);
}

// --- Full pipeline integration test ---

#[tokio::test]
async fn full_chunk_embed_pipeline() {
    let (pools, _tmp) = setup().await;

    let mem = memory::save_memory(
        &pools.fts,
        SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Long document".into(),
            content: (0..100)
                .map(|i| format!("word{i}"))
                .collect::<Vec<_>>()
                .join(" "),
            memory_type: MemoryType::Fact,
            importance: None,
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    // Chunk
    let chunker = Chunker::new(30, 10);
    let chunks = chunker.chunk(&mem.id, &mem.content);
    assert!(chunks.len() > 1);

    // Store chunks
    memory::store_chunks(&pools.vec, &chunks).unwrap();

    // Verify stored
    let retrieved = memory::get_chunks_for_memory(&pools.vec, &mem.id).unwrap();
    assert_eq!(retrieved.len(), chunks.len());

    // Embed with mock
    let embedder = MockEmbedder::new();
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embedder.embed(&texts).unwrap();
    assert_eq!(embeddings.len(), chunks.len());

    // Store chunk embeddings
    for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
        memory::store_chunk_embedding(&pools.vec, &chunk.id, embedding).unwrap();
    }

    // Delete and verify cleanup
    memory::delete_chunks(&pools.vec, &mem.id).unwrap();
    let after_delete = memory::get_chunks_for_memory(&pools.vec, &mem.id).unwrap();
    assert!(after_delete.is_empty());
}
