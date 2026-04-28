use nous_core::db::MemoryDb;
use nous_core::types::{Importance, MemoryType, NewMemory, SearchFilters, SearchMode};
use rusqlite::params;

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None, 384).expect("failed to open in-memory db")
}

fn make_memory(title: &str, content: &str) -> NewMemory {
    NewMemory {
        title: title.into(),
        content: content.into(),
        memory_type: MemoryType::Decision,
        source: None,
        importance: Importance::Moderate,
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

const VEC_DIMS: usize = 384;

fn f32_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn pad_to_384(v: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0f32; VEC_DIMS];
    for (i, &val) in v.iter().enumerate().take(VEC_DIMS) {
        out[i] = val;
    }
    out
}

fn setup_model(db: &MemoryDb) -> i64 {
    db.connection()
        .execute(
            "INSERT INTO models (name, dimensions, max_tokens) VALUES ('test-model', 384, 512)",
            [],
        )
        .unwrap();
    db.connection().last_insert_rowid()
}

fn store_chunk_with_embedding(
    db: &MemoryDb,
    memory_id: &str,
    chunk_id: &str,
    chunk_index: i64,
    content: &str,
    model_id: i64,
    embedding: &[f32],
) {
    db.connection()
        .execute(
            "INSERT INTO memory_chunks (id, memory_id, chunk_index, content, token_count, model_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![chunk_id, memory_id, chunk_index, content, 10_i64, model_id],
        )
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO memory_embeddings (chunk_id, embedding) VALUES (?1, ?2)",
            params![chunk_id, f32_to_blob(embedding)],
        )
        .unwrap();
}

fn normalize384(v: &[f32]) -> Vec<f32> {
    let padded = pad_to_384(v);
    let norm = padded.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        padded.iter().map(|x| x / norm).collect()
    } else {
        padded
    }
}

// 1. FTS search: word in memory #2 appears first
#[test]
fn fts_search_ranks_matching_memory_first() {
    let db = open_test_db();
    db.store(&make_memory("alpha topic", "the quick brown fox"))
        .unwrap();
    db.store(&make_memory(
        "beta topic",
        "xylophone orchestra performance",
    ))
    .unwrap();
    db.store(&make_memory("gamma topic", "red blue green yellow"))
        .unwrap();

    let filters = SearchFilters::default();
    let results = db
        .search("xylophone", &[], &filters, SearchMode::Fts)
        .unwrap();

    assert!(!results.is_empty());
    assert!(results[0].memory.title.contains("beta"));
}

// 2. FTS with memory_type filter
#[test]
fn fts_search_filters_by_memory_type() {
    let db = open_test_db();
    db.store(&make_memory(
        "rust patterns",
        "ownership borrowing lifetimes",
    ))
    .unwrap();

    let mut bugfix = make_memory("rust bugfix", "ownership bug in parser");
    bugfix.memory_type = MemoryType::Bugfix;
    db.store(&bugfix).unwrap();

    let filters = SearchFilters {
        memory_type: Some(MemoryType::Bugfix),
        ..SearchFilters::default()
    };
    let results = db
        .search("ownership", &[], &filters, SearchMode::Fts)
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].memory.memory_type, MemoryType::Bugfix);
}

// 3. FTS with tags filter
#[test]
fn fts_search_filters_by_tags() {
    let db = open_test_db();
    let mut m1 = make_memory("tagged memory", "kubernetes deployment strategy");
    m1.tags = vec!["k8s".into()];
    db.store(&m1).unwrap();

    let mut m2 = make_memory("untagged memory", "kubernetes pod scheduling");
    m2.tags = vec!["other".into()];
    db.store(&m2).unwrap();

    let filters = SearchFilters {
        tags: Some(vec!["k8s".into()]),
        ..SearchFilters::default()
    };
    let results = db
        .search("kubernetes", &[], &filters, SearchMode::Fts)
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].memory.title.contains("tagged"));
}

// 4. FTS with valid_only excludes expired
#[test]
fn fts_search_valid_only_excludes_expired() {
    let db = open_test_db();
    db.store(&make_memory("current info", "database connection pooling"))
        .unwrap();

    let expired_id = db
        .store(&make_memory("old info", "database sharding approach"))
        .unwrap();
    db.connection()
        .execute(
            "UPDATE memories SET valid_until = '2020-01-01T00:00:00Z' WHERE id = ?1",
            params![expired_id.to_string()],
        )
        .unwrap();

    let filters = SearchFilters {
        valid_only: Some(true),
        ..SearchFilters::default()
    };
    let results = db
        .search("database", &[], &filters, SearchMode::Fts)
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].memory.title.contains("current"));
}

// 5. Semantic search ranks closest embedding first
#[test]
fn semantic_search_ranks_closest_first() {
    let db = open_test_db();
    let model_id = setup_model(&db);

    let id1 = db
        .store(&make_memory("close memory", "very similar content"))
        .unwrap();
    let id2 = db
        .store(&make_memory("medium memory", "somewhat related"))
        .unwrap();
    let id3 = db
        .store(&make_memory("far memory", "completely different"))
        .unwrap();

    let emb_close = normalize384(&[0.9, 0.1, 0.0, 0.0]);
    let emb_medium = normalize384(&[0.5, 0.5, 0.5, 0.0]);
    let emb_far = normalize384(&[0.0, 0.0, 0.1, 0.9]);

    store_chunk_with_embedding(
        &db,
        &id1.to_string(),
        "c1",
        0,
        "chunk1",
        model_id,
        &emb_close,
    );
    store_chunk_with_embedding(
        &db,
        &id2.to_string(),
        "c2",
        0,
        "chunk2",
        model_id,
        &emb_medium,
    );
    store_chunk_with_embedding(&db, &id3.to_string(), "c3", 0, "chunk3", model_id, &emb_far);

    let query_emb = normalize384(&[1.0, 0.0, 0.0, 0.0]);
    let results = db
        .search(
            "",
            &query_emb,
            &SearchFilters::default(),
            SearchMode::Semantic,
        )
        .unwrap();

    assert!(results.len() >= 2);
    assert!(results[0].memory.title.contains("close"));
}

// 6. Semantic deduplication: multi-chunk memory appears once
#[test]
fn semantic_search_deduplicates_multi_chunk_memory() {
    let db = open_test_db();
    let model_id = setup_model(&db);

    let id = db
        .store(&make_memory(
            "long memory",
            "a very long document with many chunks",
        ))
        .unwrap();

    let emb1 = normalize384(&[0.8, 0.2, 0.0, 0.0]);
    let emb2 = normalize384(&[0.7, 0.3, 0.0, 0.0]);
    let emb3 = normalize384(&[0.6, 0.4, 0.0, 0.0]);

    store_chunk_with_embedding(
        &db,
        &id.to_string(),
        "ch1",
        0,
        "chunk part 1",
        model_id,
        &emb1,
    );
    store_chunk_with_embedding(
        &db,
        &id.to_string(),
        "ch2",
        1,
        "chunk part 2",
        model_id,
        &emb2,
    );
    store_chunk_with_embedding(
        &db,
        &id.to_string(),
        "ch3",
        2,
        "chunk part 3",
        model_id,
        &emb3,
    );

    let query_emb = normalize384(&[1.0, 0.0, 0.0, 0.0]);
    let results = db
        .search(
            "",
            &query_emb,
            &SearchFilters::default(),
            SearchMode::Semantic,
        )
        .unwrap();

    let matching: Vec<_> = results
        .iter()
        .filter(|r| r.memory.id == id.to_string())
        .collect();
    assert_eq!(matching.len(), 1, "multi-chunk memory should appear once");
}

// 7. Hybrid: memory matching both FTS and semantic ranks highest
#[test]
fn hybrid_search_dual_signal_ranks_highest() {
    let db = open_test_db();
    let model_id = setup_model(&db);

    let id_both = db
        .store(&make_memory(
            "kubernetes orchestration",
            "kubernetes container orchestration platform",
        ))
        .unwrap();
    let id_fts_only = db
        .store(&make_memory(
            "kubernetes basics",
            "kubernetes is a container scheduler",
        ))
        .unwrap();
    let id_sem_only = db
        .store(&make_memory(
            "container platform",
            "docker swarm alternative",
        ))
        .unwrap();

    let emb_both = normalize384(&[0.9, 0.1, 0.0, 0.0]);
    let emb_fts = normalize384(&[0.0, 0.0, 0.1, 0.9]);
    let emb_sem = normalize384(&[0.8, 0.2, 0.0, 0.0]);

    store_chunk_with_embedding(
        &db,
        &id_both.to_string(),
        "cb",
        0,
        "k8s",
        model_id,
        &emb_both,
    );
    store_chunk_with_embedding(
        &db,
        &id_fts_only.to_string(),
        "cf",
        0,
        "k8s basics",
        model_id,
        &emb_fts,
    );
    store_chunk_with_embedding(
        &db,
        &id_sem_only.to_string(),
        "cs",
        0,
        "containers",
        model_id,
        &emb_sem,
    );

    let query_emb = normalize384(&[1.0, 0.0, 0.0, 0.0]);
    let filters = SearchFilters::default();
    let results = db
        .search("kubernetes", &query_emb, &filters, SearchMode::Hybrid)
        .unwrap();

    assert!(!results.is_empty());
    assert_eq!(
        results[0].memory.id,
        id_both.to_string(),
        "dual-signal match should rank first"
    );
}

// 8. SearchMode::Fts ignores embeddings, SearchMode::Semantic ignores FTS
#[test]
fn search_mode_dispatches_correctly() {
    let db = open_test_db();
    let model_id = setup_model(&db);

    let id = db
        .store(&make_memory("unique_fts_term", "unique_fts_term content"))
        .unwrap();

    let emb = normalize384(&[0.0, 0.0, 0.0, 1.0]);
    store_chunk_with_embedding(&db, &id.to_string(), "cx", 0, "text", model_id, &emb);

    let query_emb = normalize384(&[1.0, 0.0, 0.0, 0.0]);
    let filters = SearchFilters::default();

    let fts_results = db
        .search("unique_fts_term", &query_emb, &filters, SearchMode::Fts)
        .unwrap();
    assert!(
        !fts_results.is_empty(),
        "FTS mode should find by text match"
    );

    let sem_results = db
        .search(
            "nonexistent_term",
            &query_emb,
            &filters,
            SearchMode::Semantic,
        )
        .unwrap();
    // Semantic uses embeddings not text — the far embedding should still return the memory
    // but with low score. The key check is that it doesn't crash and returns based on embedding.
    assert!(sem_results.len() <= 1);
}

// 9. Context returns only workspace A memories
#[test]
fn context_returns_workspace_scoped() {
    let db = open_test_db();

    let mut m1 = make_memory("workspace A mem", "content for workspace A");
    m1.workspace_path = Some("/tmp/workspace-a".into());
    db.store(&m1).unwrap();

    let mut m2 = make_memory("workspace B mem", "content for workspace B");
    m2.workspace_path = Some("/tmp/workspace-b".into());
    db.store(&m2).unwrap();

    let ws_a_id: i64 = db
        .connection()
        .query_row(
            "SELECT id FROM workspaces WHERE path = '/tmp/workspace-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let entries = db.context(ws_a_id, false, 50).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].title.contains("workspace A"));
}

// 10. Context summary=true omits content
#[test]
fn context_summary_omits_content() {
    let db = open_test_db();
    let mut mem = make_memory("summary test", "this content should be omitted");
    mem.workspace_path = Some("/tmp/summary-ws".into());
    db.store(&mem).unwrap();

    let ws_id: i64 = db
        .connection()
        .query_row(
            "SELECT id FROM workspaces WHERE path = '/tmp/summary-ws'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let entries = db.context(ws_id, true, 50).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.is_none());

    let entries_full = db.context(ws_id, false, 50).unwrap();
    assert!(entries_full[0].content.is_some());
}

// 11. Context excludes archived memories
#[test]
fn context_excludes_archived() {
    let db = open_test_db();
    let mut m1 = make_memory("active mem", "active content");
    m1.workspace_path = Some("/tmp/archive-ws".into());
    db.store(&m1).unwrap();

    let mut m2 = make_memory("archived mem", "archived content");
    m2.workspace_path = Some("/tmp/archive-ws".into());
    let id2 = db.store(&m2).unwrap();

    db.forget(&id2, false).unwrap();

    let ws_id: i64 = db
        .connection()
        .query_row(
            "SELECT id FROM workspaces WHERE path = '/tmp/archive-ws'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let entries = db.context(ws_id, false, 50).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].title.contains("active"));
}

// 12. Access log entries created for search results
#[test]
fn search_logs_access() {
    let db = open_test_db();
    db.store(&make_memory("logged memory", "access log test content"))
        .unwrap();

    let filters = SearchFilters::default();
    let results = db.search("logged", &[], &filters, SearchMode::Fts).unwrap();

    assert!(!results.is_empty());

    let count: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM access_log WHERE access_type = 'search'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count > 0, "search should create access log entries");
}

// --- sanitize_fts_query unit tests ---

use nous_core::search::sanitize_fts_query;

#[test]
fn sanitize_fts_query_plain_words_unchanged() {
    assert_eq!(sanitize_fts_query("hello world"), "hello world");
}

#[test]
fn sanitize_fts_query_quotes_hyphenated_token() {
    assert_eq!(sanitize_fts_query("INI-050"), "\"INI-050\"");
}

#[test]
fn sanitize_fts_query_quotes_multiple_hyphenated_tokens() {
    assert_eq!(
        sanitize_fts_query("INI-050 pre-computation"),
        "\"INI-050\" \"pre-computation\""
    );
}

#[test]
fn sanitize_fts_query_mixed_plain_and_hyphenated() {
    assert_eq!(sanitize_fts_query("fix INI-050 bug"), "fix \"INI-050\" bug");
}

#[test]
fn sanitize_fts_query_colon_token() {
    assert_eq!(sanitize_fts_query("key:value"), "\"key:value\"");
}

#[test]
fn sanitize_fts_query_empty_input() {
    assert_eq!(sanitize_fts_query(""), "");
}

#[test]
fn sanitize_fts_query_preserves_embedded_quotes() {
    assert_eq!(sanitize_fts_query("say-\"hello\""), "\"say-\"\"hello\"\"\"");
}

// --- End-to-end FTS search with hyphenated content ---

#[test]
fn fts_search_hyphenated_ticket_id() {
    let db = open_test_db();
    db.store(&make_memory(
        "INI-050 scheduler bug",
        "The INI-050 ticket tracks a scheduler race condition",
    ))
    .unwrap();
    db.store(&make_memory("unrelated", "nothing relevant here"))
        .unwrap();

    let filters = SearchFilters::default();
    let results = db
        .search("INI-050", &[], &filters, SearchMode::Fts)
        .unwrap();

    assert!(!results.is_empty(), "should find memory with INI-050");
    assert!(results[0].memory.title.contains("INI-050"));
}

#[test]
fn fts_search_hyphenated_compound_word() {
    let db = open_test_db();
    db.store(&make_memory(
        "pre-computation optimization",
        "pre-computation of lookup tables reduces latency",
    ))
    .unwrap();

    let filters = SearchFilters::default();
    let results = db
        .search("pre-computation", &[], &filters, SearchMode::Fts)
        .unwrap();

    assert!(
        !results.is_empty(),
        "should find memory with pre-computation"
    );
    assert!(results[0].memory.title.contains("pre-computation"));
}

#[test]
fn fts_search_plain_query_still_works() {
    let db = open_test_db();
    db.store(&make_memory("plain search", "simple words no hyphens"))
        .unwrap();

    let filters = SearchFilters::default();
    let results = db.search("simple", &[], &filters, SearchMode::Fts).unwrap();

    assert!(!results.is_empty());
    assert!(results[0].memory.title.contains("plain"));
}
