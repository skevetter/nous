use rmcp::model::CallToolRequestParams;
use rmcp::{ClientHandler, ServiceExt};

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

fn call_params(
    name: impl Into<std::borrow::Cow<'static, str>>,
    args: serde_json::Value,
) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(args.as_object().unwrap().clone())
}

fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
    let text = result.content[0].as_text().unwrap().text.as_str();
    serde_json::from_str(text).unwrap()
}

fn assert_ok(result: &rmcp::model::CallToolResult, ctx: &str) {
    if result.is_error == Some(true) {
        let msg = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("<no text>");
        panic!("{ctx} failed: {msg}");
    }
}

fn search_has_id(json: &serde_json::Value, id: &str) -> bool {
    json["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["memory"]["id"] == id)
}

#[tokio::test]
async fn memory_lifecycle_roundtrip() {
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    let db_path = test_db_path();
    let mut cfg = nous_mcp::config::Config::default();
    cfg.encryption.db_key_file = format!("{db_path}.key");
    let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
    let server = nous_mcp::server::NousServer::new(cfg, embedding, &db_path, None).unwrap();

    let server_handle = tokio::spawn(async move {
        server.serve(server_transport).await?.waiting().await?;
        anyhow::Ok(())
    });

    #[derive(Debug, Clone, Default)]
    struct TestClient;
    impl ClientHandler for TestClient {}

    let client = TestClient.serve(client_transport).await.unwrap();

    // --- Step 1: memory_store -> memory_search (FTS) -> assert found ---

    let store_result = client
        .call_tool(call_params(
            "memory_store",
            serde_json::json!({
                "title": "Rust borrow checker rules",
                "content": "The borrow checker enforces ownership rules at compile time to guarantee memory safety without a garbage collector.",
                "memory_type": "fact",
                "tags": ["rust", "compiler"],
                "source": "integration-test"
            }),
        ))
        .await
        .unwrap();

    assert_ok(&store_result, "memory_store");
    let store_json = extract_json(&store_result);
    let id = store_json["id"].as_str().unwrap().to_owned();
    assert!(!id.is_empty(), "store should return a non-empty ID");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let fts_result = client
        .call_tool(call_params(
            "memory_search",
            serde_json::json!({
                "query": "borrow checker",
                "mode": "fts"
            }),
        ))
        .await
        .unwrap();

    assert_ok(&fts_result, "memory_search (fts)");
    let fts_json = extract_json(&fts_result);
    assert!(
        search_has_id(&fts_json, &id),
        "FTS search should find the stored memory"
    );

    // --- Step 2: memory_search (semantic) -> assert found ---

    let sem_result = client
        .call_tool(call_params(
            "memory_search",
            serde_json::json!({
                "query": "ownership rules compile time safety",
                "mode": "semantic"
            }),
        ))
        .await
        .unwrap();

    assert_ok(&sem_result, "memory_search (semantic)");
    let sem_json = extract_json(&sem_result);
    assert!(
        search_has_id(&sem_json, &id),
        "semantic search should find the stored memory"
    );

    // --- Step 3: memory_recall by ID -> assert full fields ---

    let recall_result = client
        .call_tool(call_params(
            "memory_recall",
            serde_json::json!({ "id": id }),
        ))
        .await
        .unwrap();

    assert_ok(&recall_result, "memory_recall");
    let recall_json = extract_json(&recall_result);
    assert_eq!(recall_json["id"], id);
    assert_eq!(recall_json["title"], "Rust borrow checker rules");
    assert_eq!(recall_json["memory_type"], "fact");
    assert_eq!(recall_json["source"], "integration-test");
    assert_eq!(recall_json["archived"], false);
    assert_eq!(
        recall_json["tags"].as_array().unwrap(),
        &[serde_json::json!("compiler"), serde_json::json!("rust")]
    );

    // --- Step 4: memory_update title -> memory_recall -> assert updated ---

    let update_result = client
        .call_tool(call_params(
            "memory_update",
            serde_json::json!({
                "id": id,
                "title": "Rust ownership and borrowing"
            }),
        ))
        .await
        .unwrap();

    assert_ok(&update_result, "memory_update");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let recall2_result = client
        .call_tool(call_params(
            "memory_recall",
            serde_json::json!({ "id": id }),
        ))
        .await
        .unwrap();

    assert_ok(&recall2_result, "memory_recall after update");
    let recall2_json = extract_json(&recall2_result);
    assert_eq!(recall2_json["title"], "Rust ownership and borrowing");

    // --- Step 5: memory_forget (soft) -> memory_search -> assert not in results ---

    let forget_result = client
        .call_tool(call_params(
            "memory_forget",
            serde_json::json!({ "id": id, "hard": false }),
        ))
        .await
        .unwrap();

    assert_ok(&forget_result, "memory_forget (soft)");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let search_after_forget = client
        .call_tool(call_params(
            "memory_search",
            serde_json::json!({
                "query": "borrow checker",
                "mode": "fts"
            }),
        ))
        .await
        .unwrap();

    assert_ok(&search_after_forget, "memory_search after forget");
    let forget_json = extract_json(&search_after_forget);
    assert!(
        !search_has_id(&forget_json, &id),
        "soft-forgotten memory should not appear in default search"
    );

    // --- Step 6: memory_unarchive -> memory_search -> assert found again ---

    let unarchive_result = client
        .call_tool(call_params(
            "memory_unarchive",
            serde_json::json!({ "id": id }),
        ))
        .await
        .unwrap();

    assert_ok(&unarchive_result, "memory_unarchive");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let search_after_unarchive = client
        .call_tool(call_params(
            "memory_search",
            serde_json::json!({
                "query": "borrow checker",
                "mode": "fts"
            }),
        ))
        .await
        .unwrap();

    assert_ok(&search_after_unarchive, "memory_search after unarchive");
    let unarchive_json = extract_json(&search_after_unarchive);
    assert!(
        search_has_id(&unarchive_json, &id),
        "unarchived memory should appear in search again"
    );

    // Cleanup
    client.cancel().await.unwrap();
    server_handle.await.unwrap().unwrap();
    let _ = std::fs::remove_file(&db_path);
}
