use nous_core::db::DbPools;
use nous_core::memory::{
    self, analytics, Embedder, Importance, MemoryType, MockEmbedder, SaveMemoryRequest,
    SearchMemoryRequest,
};
use tempfile::TempDir;

async fn setup() -> (sqlx::SqlitePool, nous_core::db::VecPool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools.fts, pools.vec, tmp)
}

async fn seed_memories(pool: &sqlx::SqlitePool) {
    memory::save_memory(
        pool,
        SaveMemoryRequest {
            workspace_id: Some("ws-1".into()),
            agent_id: Some("agent-1".into()),
            title: "Rust async patterns".into(),
            content: "Use tokio for async runtime. Pin futures when needed.".into(),
            memory_type: MemoryType::Convention,
            importance: Some(Importance::High),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    memory::save_memory(
        pool,
        SaveMemoryRequest {
            workspace_id: Some("ws-1".into()),
            agent_id: Some("agent-2".into()),
            title: "Database connection pooling".into(),
            content: "Always use connection pools for SQLite. Set WAL mode.".into(),
            memory_type: MemoryType::Architecture,
            importance: Some(Importance::Moderate),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    memory::save_memory(
        pool,
        SaveMemoryRequest {
            workspace_id: Some("ws-2".into()),
            agent_id: Some("agent-1".into()),
            title: "Error handling strategy".into(),
            content: "Use thiserror for library errors, anyhow for application errors.".into(),
            memory_type: MemoryType::Decision,
            importance: Some(Importance::High),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn hybrid_search_auto_embed_with_mock() {
    let (pool, vec_pool, _tmp) = setup().await;
    seed_memories(&pool).await;

    let embedder = MockEmbedder::new();

    // Embed and store vectors for the seeded memories
    let memories = memory::search_memories(
        &pool,
        &SearchMemoryRequest {
            query: "async patterns database errors".into(),
            limit: Some(10),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for mem in &memories {
        let emb = embedder.embed(&[mem.content.as_str()]).unwrap();
        memory::store_embedding(&pool, &vec_pool, &mem.id, &emb[0])
            .await
            .unwrap();
    }

    // Auto-embed: generate query embedding using MockEmbedder, then search
    let query = "async runtime";
    let query_emb = embedder
        .embed(&[query])
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let results =
        memory::search_hybrid_filtered(&pool, &vec_pool, query, &query_emb, 10, None, None, None)
            .await
            .unwrap();

    assert!(
        !results.is_empty(),
        "hybrid search should return results with auto-embedded query"
    );
}

#[tokio::test]
async fn hybrid_search_with_explicit_embedding() {
    let (pool, vec_pool, _tmp) = setup().await;
    seed_memories(&pool).await;

    let embedder = MockEmbedder::new();

    let memories = memory::search_memories(
        &pool,
        &SearchMemoryRequest {
            query: "async database errors".into(),
            limit: Some(10),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for mem in &memories {
        let emb = embedder.embed(&[mem.content.as_str()]).unwrap();
        memory::store_embedding(&pool, &vec_pool, &mem.id, &emb[0])
            .await
            .unwrap();
    }

    // Provide explicit embedding (backwards compatible)
    let explicit_embedding = embedder
        .embed(&["database connection"])
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let results = memory::search_hybrid_filtered(
        &pool,
        &vec_pool,
        "database connection",
        &explicit_embedding,
        10,
        None,
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        !results.is_empty(),
        "hybrid search should work with explicit embedding"
    );
}

#[tokio::test]
async fn search_analytics_recording() {
    let (pool, _vec_pool, _tmp) = setup().await;

    // Record some search events
    analytics::record_search_event(
        &pool,
        &analytics::SearchEvent {
            query_text: "test query".to_string(),
            search_type: "fts".to_string(),
            result_count: 5,
            latency_ms: 12,
            workspace_id: Some("ws-1".to_string()),
            agent_id: Some("agent-1".to_string()),
        },
    )
    .await
    .unwrap();

    analytics::record_search_event(
        &pool,
        &analytics::SearchEvent {
            query_text: "vector search".to_string(),
            search_type: "vector".to_string(),
            result_count: 0,
            latency_ms: 45,
            workspace_id: None,
            agent_id: None,
        },
    )
    .await
    .unwrap();

    analytics::record_search_event(
        &pool,
        &analytics::SearchEvent {
            query_text: "hybrid query".to_string(),
            search_type: "hybrid".to_string(),
            result_count: 3,
            latency_ms: 30,
            workspace_id: Some("ws-1".to_string()),
            agent_id: Some("agent-2".to_string()),
        },
    )
    .await
    .unwrap();

    let stats = analytics::get_search_stats(&pool, None).await.unwrap();

    assert_eq!(stats.total_searches, 3);
    assert_eq!(stats.fts_count, 1);
    assert_eq!(stats.vector_count, 1);
    assert_eq!(stats.hybrid_count, 1);
    assert!((stats.zero_result_rate - 33.33).abs() < 1.0);
    assert!(stats.avg_latency_ms > 0.0);
    assert_eq!(stats.top_queries.len(), 3);
}

#[tokio::test]
async fn search_analytics_with_since_filter() {
    let (pool, _vec_pool, _tmp) = setup().await;

    analytics::record_search_event(
        &pool,
        &analytics::SearchEvent {
            query_text: "old query".to_string(),
            search_type: "fts".to_string(),
            result_count: 2,
            latency_ms: 10,
            workspace_id: None,
            agent_id: None,
        },
    )
    .await
    .unwrap();

    // Query with a future 'since' date — should return no events
    let stats = analytics::get_search_stats(&pool, Some("2099-01-01T00:00:00"))
        .await
        .unwrap();
    assert_eq!(stats.total_searches, 0);
}

#[tokio::test]
async fn filter_by_workspace_id() {
    let (pool, vec_pool, _tmp) = setup().await;
    seed_memories(&pool).await;

    let embedder = MockEmbedder::new();

    let memories = memory::search_memories(
        &pool,
        &SearchMemoryRequest {
            query: "async database errors".into(),
            limit: Some(10),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for mem in &memories {
        let emb = embedder.embed(&[mem.content.as_str()]).unwrap();
        memory::store_embedding(&pool, &vec_pool, &mem.id, &emb[0])
            .await
            .unwrap();
    }

    let query_emb = embedder
        .embed(&["async"])
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let results = memory::search_hybrid_filtered(
        &pool,
        &vec_pool,
        "async",
        &query_emb,
        10,
        Some("ws-1"),
        None,
        None,
    )
    .await
    .unwrap();

    for r in &results {
        assert_eq!(
            r.memory.workspace_id, "ws-1",
            "all results should be from ws-1"
        );
    }
}

#[tokio::test]
async fn filter_by_agent_id() {
    let (pool, vec_pool, _tmp) = setup().await;
    seed_memories(&pool).await;

    let embedder = MockEmbedder::new();

    let memories = memory::search_memories(
        &pool,
        &SearchMemoryRequest {
            query: "async database errors".into(),
            limit: Some(10),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for mem in &memories {
        let emb = embedder.embed(&[mem.content.as_str()]).unwrap();
        memory::store_embedding(&pool, &vec_pool, &mem.id, &emb[0])
            .await
            .unwrap();
    }

    let query_emb = embedder
        .embed(&["errors"])
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let results = memory::search_hybrid_filtered(
        &pool,
        &vec_pool,
        "errors",
        &query_emb,
        10,
        None,
        Some("agent-1"),
        None,
    )
    .await
    .unwrap();

    for r in &results {
        assert_eq!(
            r.memory.agent_id.as_deref(),
            Some("agent-1"),
            "all results should be from agent-1"
        );
    }
}

#[tokio::test]
async fn filter_by_memory_type() {
    let (pool, vec_pool, _tmp) = setup().await;
    seed_memories(&pool).await;

    let embedder = MockEmbedder::new();

    let memories = memory::search_memories(
        &pool,
        &SearchMemoryRequest {
            query: "async database errors".into(),
            limit: Some(10),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    for mem in &memories {
        let emb = embedder.embed(&[mem.content.as_str()]).unwrap();
        memory::store_embedding(&pool, &vec_pool, &mem.id, &emb[0])
            .await
            .unwrap();
    }

    let query_emb = embedder
        .embed(&["patterns"])
        .unwrap()
        .into_iter()
        .next()
        .unwrap();

    let results = memory::search_hybrid_filtered(
        &pool,
        &vec_pool,
        "patterns",
        &query_emb,
        10,
        None,
        None,
        Some(MemoryType::Convention),
    )
    .await
    .unwrap();

    for r in &results {
        assert_eq!(
            r.memory.memory_type, "convention",
            "all results should be conventions"
        );
    }
}
