use std::collections::HashSet;
use std::sync::Arc;

use nous_mcp::config::Config;
use nous_mcp::server::NousServer;
use nous_mcp::tools::{MemoryRecallParams, MemorySqlParams, MemoryStoreParams, handle_store};
use tokio::task::JoinSet;

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
    let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
    NousServer::new(cfg, embedding, db_path, None).unwrap()
}

fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = result.content[0].as_text().unwrap().text.as_str();
    serde_json::from_str(text).unwrap()
}

fn store_params(i: usize) -> MemoryStoreParams {
    MemoryStoreParams {
        title: format!("concurrent-{i}"),
        content: format!("Content for concurrent write number {i}"),
        memory_type: "fact".into(),
        tags: vec![format!("batch-{i}")],
        source: Some("concurrent-test".into()),
        importance: None,
        confidence: None,
        workspace_path: None,
        session_id: None,
        trace_id: None,
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    }
}

#[tokio::test]
async fn ten_concurrent_writes_no_data_loss() {
    let db_path = test_db_path();
    let server = Arc::new(test_server(&db_path));

    let mut join_set = JoinSet::new();

    for i in 0..10 {
        let server = Arc::clone(&server);
        let params = store_params(i);

        join_set.spawn(async move {
            handle_store(
                params,
                &server.write_channel,
                &server.embedding,
                &server.classifier,
                &server.chunker,
            )
            .await
        });
    }

    let mut ids: Vec<String> = Vec::with_capacity(10);
    while let Some(result) = join_set.join_next().await {
        let call_result = result.expect("task should not panic");
        assert_ne!(call_result.is_error, Some(true), "store should succeed");
        let json = extract_json(&call_result);
        let id = json["id"]
            .as_str()
            .expect("id should be a string")
            .to_string();
        assert!(!id.is_empty());
        ids.push(id);
    }

    assert_eq!(ids.len(), 10, "should have 10 results");

    let unique_ids: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
    assert_eq!(unique_ids.len(), 10, "all 10 IDs should be distinct");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    for id in &ids {
        let result = nous_mcp::tools::handle_recall(
            MemoryRecallParams { id: id.clone() },
            &server.read_pool,
            &server.write_channel,
        )
        .await;
        assert_ne!(
            result.is_error,
            Some(true),
            "recall should succeed for id {id}"
        );
        let json = extract_json(&result);
        assert_eq!(json["id"].as_str().unwrap(), id.as_str());
    }

    let integrity = nous_mcp::tools::handle_sql(
        MemorySqlParams {
            query: "PRAGMA integrity_check".into(),
        },
        &server.read_pool,
    )
    .await;
    assert_ne!(
        integrity.is_error,
        Some(true),
        "integrity check should succeed"
    );
    let integrity_json = extract_json(&integrity);
    assert_eq!(
        integrity_json["rows"][0][0].as_str().unwrap(),
        "ok",
        "database integrity check should pass"
    );

    let _ = std::fs::remove_file(&db_path);
}
