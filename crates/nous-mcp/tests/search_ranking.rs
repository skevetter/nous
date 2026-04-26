use nous_core::embed::MockEmbedding;
use nous_mcp::config::Config;
use nous_mcp::server::NousServer;
use nous_mcp::tools::{MemorySearchParams, MemoryStoreParams, handle_search, handle_store};
use rmcp::model::CallToolResult;

fn test_db_path() -> String {
    format!(
        "/tmp/nous-test-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn test_server(db_path: &str) -> NousServer {
    let cfg = Config::default();
    let embedding = Box::new(MockEmbedding::new(384));
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
        "Kubernetes pod scheduling and container orchestration",
        "Python dependency management with pip and virtualenv",
        "SQL query optimization and database indexing strategies",
    ];

    let id0 = store_memory(&server, "k8s scheduling", contents[0]).await;
    let id1 = store_memory(&server, "python deps", contents[1]).await;
    let id2 = store_memory(&server, "sql optimization", contents[2]).await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let result = handle_search(
        MemorySearchParams {
            query: contents[1].into(),
            mode: Some("semantic".into()),
            memory_type: None,
            category_id: None,
            workspace_id: None,
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
        result_ids[0], id1,
        "closest semantic match (python deps) should rank first"
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
