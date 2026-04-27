use std::path::Path;

use nous_core::embed::FixtureEmbedding;
use nous_mcp::config::Config;
use nous_mcp::server::NousServer;
use nous_mcp::tools::{MemorySearchParams, MemoryStoreParams, handle_search, handle_store};
use rmcp::model::CallToolResult;

fn fixture_path() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../nous-core/tests/fixtures/embedding_vectors.json"
    ))
}

fn test_db_path() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "/tmp/nous-test-{}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        seq,
    )
}

fn test_server(db_path: &str) -> NousServer {
    let mut cfg = Config::default();
    cfg.encryption.db_key_file = format!("{db_path}.key");
    let embedding = Box::new(FixtureEmbedding::load(fixture_path()).unwrap());
    NousServer::new(cfg, embedding, db_path).unwrap()
}

fn extract_json(result: &CallToolResult) -> serde_json::Value {
    let text = result.content[0].as_text().unwrap().text.as_str();
    serde_json::from_str(text).unwrap()
}

fn is_success(result: &CallToolResult) -> bool {
    result.is_error != Some(true)
}

async fn store_memory(server: &NousServer, title: &str, content: &str) -> String {
    let result = handle_store(
        MemoryStoreParams {
            title: title.into(),
            content: content.into(),
            memory_type: "fact".into(),
            tags: vec![],
            source: None,
            importance: None,
            confidence: None,
            workspace_path: None,
            session_id: None,
            trace_id: None,
            agent_id: None,
            agent_model: None,
            valid_from: None,
            category_id: None,
        },
        &server.write_channel,
        &server.embedding,
        &server.classifier,
        &server.chunker,
    )
    .await;
    assert!(is_success(&result), "store should succeed");
    extract_json(&result)["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn semantic_search_ranks_closest_match_first() {
    let db_path = test_db_path();
    let server = test_server(&db_path);

    let contents = [
        "Rust programming language",
        "Writing code in Rust",
        "Chocolate cake recipe",
    ];

    let id0 = store_memory(&server, "rust-lang", contents[0]).await;
    let id1 = store_memory(&server, "coding-rust", contents[1]).await;
    let id2 = store_memory(&server, "chocolate", contents[2]).await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let result = handle_search(
        MemorySearchParams {
            query: "Rust programming language".into(),
            mode: Some("semantic".into()),
            memory_type: None,
            category_id: None,
            workspace_id: None,
            trace_id: None,
            session_id: None,
            importance: None,
            confidence: None,
            tags: None,
            archived: None,
            since: None,
            until: None,
            valid_only: None,
            limit: None,
        },
        &db_path,
        384,
        &server.embedding,
    )
    .await;

    assert!(is_success(&result), "search should succeed");
    let json = extract_json(&result);
    let results = json["results"].as_array().unwrap();

    assert_eq!(results.len(), 3, "all 3 memories should be returned");

    let result_ids: Vec<&str> = results
        .iter()
        .map(|r| r["memory"]["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        result_ids[0], id0,
        "exact match (Rust programming language) should rank first"
    );

    let scores: Vec<f64> = results
        .iter()
        .map(|r| r["rank"].as_f64().unwrap())
        .collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "scores should be monotonically decreasing: {scores:?}"
        );
    }

    assert!(result_ids.contains(&id0.as_str()));
    assert!(result_ids.contains(&id1.as_str()));
    assert!(result_ids.contains(&id2.as_str()));

    let _ = std::fs::remove_file(&db_path);
}
