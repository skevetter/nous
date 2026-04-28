use rmcp::model::CallToolRequestParams;
use rmcp::{ClientHandler, ServiceExt};

fn test_db_path() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "/tmp/nous-sched-e2e-{}-{}-{}.db",
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

#[derive(Debug, Clone, Default)]
struct TestClient;
impl ClientHandler for TestClient {}

async fn setup() -> (
    rmcp::service::RunningService<rmcp::RoleClient, TestClient>,
    String,
) {
    let db_path = test_db_path();
    let (server_transport, client_transport) = tokio::io::duplex(4096);

    let mut cfg = nous_cli::config::Config::default();
    cfg.encryption.db_key_file = format!("{db_path}.key");
    let embedding = Box::new(nous_core::embed::MockEmbedding::new(384));
    let server = nous_cli::server::NousServer::new(cfg, embedding, &db_path, None).unwrap();

    tokio::spawn(async move {
        let _ = server
            .serve(server_transport)
            .await
            .unwrap()
            .waiting()
            .await;
    });

    let client = TestClient.serve(client_transport).await.unwrap();
    (client, db_path)
}

#[tokio::test]
async fn create_list_get_delete_lifecycle() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-lifecycle",
                "cron_expr": "*/5 * * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let create_json = extract_json(&create_result);
    let id = create_json["id"].as_str().unwrap().to_string();
    assert!(!id.is_empty());

    let list_result = client
        .call_tool(call_params("schedule_list", serde_json::json!({})))
        .await
        .unwrap();
    assert_ok(&list_result, "schedule_list");
    let list_json = extract_json(&list_result);
    let schedules = list_json["schedules"].as_array().unwrap();
    assert!(
        schedules.iter().any(|s| s["id"] == id),
        "created schedule should appear in list"
    );

    let get_result = client
        .call_tool(call_params("schedule_get", serde_json::json!({"id": id})))
        .await
        .unwrap();
    assert_ok(&get_result, "schedule_get");
    let get_json = extract_json(&get_result);
    assert_eq!(get_json["name"], "e2e-lifecycle");
    assert_eq!(get_json["cron_expr"], "*/5 * * * *");
    assert_eq!(get_json["enabled"], true);

    let delete_result = client
        .call_tool(call_params(
            "schedule_delete",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&delete_result, "schedule_delete");

    let get_after_delete = client
        .call_tool(call_params("schedule_get", serde_json::json!({"id": id})))
        .await
        .unwrap();
    assert!(
        get_after_delete.is_error == Some(true),
        "schedule_get after delete should fail"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn trigger_and_check_runs() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-trigger",
                "cron_expr": "0 0 * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let id = extract_json(&create_result)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let trigger_result = client
        .call_tool(call_params(
            "schedule_trigger",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&trigger_result, "schedule_trigger");
    let trigger_json = extract_json(&trigger_result);
    assert_eq!(trigger_json["status"], "triggered");
    assert!(trigger_json["last_run"].is_object());

    let runs_result = client
        .call_tool(call_params(
            "schedule_runs",
            serde_json::json!({"schedule_id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&runs_result, "schedule_runs");
    let runs_json = extract_json(&runs_result);
    let runs = runs_json["runs"].as_array().unwrap();
    assert!(!runs.is_empty(), "should have at least one run");
    assert_eq!(runs[0]["status"], "completed");

    let run_id = runs[0]["id"].as_str().unwrap().to_string();
    let run_get_result = client
        .call_tool(call_params(
            "schedule_run_get",
            serde_json::json!({"run_id": run_id}),
        ))
        .await
        .unwrap();
    assert_ok(&run_get_result, "schedule_run_get");
    let run_detail = extract_json(&run_get_result);
    assert_eq!(run_detail["status"], "completed");
    assert!(run_detail["output"].is_string());

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn health_reports_failures() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-failing",
                "cron_expr": "0 0 * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"nonexistent_tool","args":{}}"#,
                "max_retries": 1,
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let id = extract_json(&create_result)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let _ = client
        .call_tool(call_params(
            "schedule_trigger",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();

    let health_result = client
        .call_tool(call_params("schedule_health", serde_json::json!({})))
        .await
        .unwrap();
    assert_ok(&health_result, "schedule_health");
    let health = extract_json(&health_result);
    assert!(health["total_schedules"].as_i64().unwrap() >= 1);
    assert!(health["failing_count"].as_i64().unwrap() >= 1);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn pause_resume_cycle() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-pause-resume",
                "cron_expr": "*/5 * * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let id = extract_json(&create_result)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let pause_result = client
        .call_tool(call_params(
            "schedule_pause",
            serde_json::json!({"id": id, "duration_secs": 2}),
        ))
        .await
        .unwrap();
    assert_ok(&pause_result, "schedule_pause");

    let get_result = client
        .call_tool(call_params("schedule_get", serde_json::json!({"id": id})))
        .await
        .unwrap();
    assert_ok(&get_result, "schedule_get after pause");
    let paused = extract_json(&get_result);
    assert_eq!(paused["enabled"], false);

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    let mut re_enabled = false;
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let get_result = client
            .call_tool(call_params("schedule_get", serde_json::json!({"id": id})))
            .await
            .unwrap();
        let json = extract_json(&get_result);
        if json["enabled"] == true {
            re_enabled = true;
            break;
        }
    }
    assert!(
        re_enabled,
        "schedule should auto-resume after duration_secs"
    );

    let resume_result = client
        .call_tool(call_params(
            "schedule_resume",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&resume_result, "schedule_resume");

    let final_get = client
        .call_tool(call_params("schedule_get", serde_json::json!({"id": id})))
        .await
        .unwrap();
    let final_json = extract_json(&final_get);
    assert_eq!(final_json["enabled"], true);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn desired_outcome_string_match() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-outcome-match",
                "cron_expr": "0 0 * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
                "desired_outcome": "\"total\"",
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let id = extract_json(&create_result)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let trigger_result = client
        .call_tool(call_params(
            "schedule_trigger",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&trigger_result, "schedule_trigger");

    let runs_result = client
        .call_tool(call_params(
            "schedule_runs",
            serde_json::json!({"schedule_id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&runs_result, "schedule_runs");
    let runs = extract_json(&runs_result)["runs"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!runs.is_empty());
    assert_eq!(
        runs[0]["status"], "completed",
        "string match should produce completed status"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn desired_outcome_regex_match() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-outcome-regex",
                "cron_expr": "0 0 * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
                "desired_outcome": "/\"total\":\\d+/",
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let id = extract_json(&create_result)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let trigger_result = client
        .call_tool(call_params(
            "schedule_trigger",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&trigger_result, "schedule_trigger");

    let runs_result = client
        .call_tool(call_params(
            "schedule_runs",
            serde_json::json!({"schedule_id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&runs_result, "schedule_runs");
    let runs = extract_json(&runs_result)["runs"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!runs.is_empty());
    assert_eq!(
        runs[0]["status"], "completed",
        "regex match should produce completed status"
    );

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn export_import_round_trip() {
    let (client, db_path) = setup().await;

    let create1 = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "export-test-1",
                "cron_expr": "*/10 * * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create1, "schedule_create 1");

    let create2 = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "export-test-2",
                "cron_expr": "0 */2 * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
                "desired_outcome": "total",
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create2, "schedule_create 2");

    let export_result = client
        .call_tool(call_params(
            "schedule_export",
            serde_json::json!({"format": "json"}),
        ))
        .await
        .unwrap();
    assert_ok(&export_result, "schedule_export");
    let export_json = extract_json(&export_result);
    assert!(
        export_json["count"].as_i64().unwrap() >= 2,
        "export should contain at least 2 schedules"
    );
    let exported_schedules = export_json["schedules"].as_array().unwrap();
    let has_test1 = exported_schedules
        .iter()
        .any(|s| s["name"] == "export-test-1");
    let has_test2 = exported_schedules
        .iter()
        .any(|s| s["name"] == "export-test-2");
    assert!(has_test1, "export should contain export-test-1");
    assert!(has_test2, "export should contain export-test-2");

    // Delete originals so we can reimport
    for s in exported_schedules {
        let id = s["id"].as_str().unwrap();
        let _ = client
            .call_tool(call_params(
                "schedule_delete",
                serde_json::json!({"id": id}),
            ))
            .await
            .unwrap();
    }

    let list_after_delete = client
        .call_tool(call_params("schedule_list", serde_json::json!({})))
        .await
        .unwrap();
    let list_json = extract_json(&list_after_delete);
    assert_eq!(
        list_json["schedules"].as_array().unwrap().len(),
        0,
        "all schedules should be deleted"
    );

    // Build import payload from export data
    let import_entries: Vec<serde_json::Value> = exported_schedules
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s["name"],
                "cron_expr": s["cron_expr"],
                "timezone": s["timezone"],
                "action_type": s["action_type"],
                "action_payload": s["action_payload"],
                "desired_outcome": s["desired_outcome"],
            })
        })
        .collect();
    let import_data =
        serde_json::to_string(&serde_json::json!({"schedules": import_entries})).unwrap();

    let import_result = client
        .call_tool(call_params(
            "schedule_import",
            serde_json::json!({"data": import_data}),
        ))
        .await
        .unwrap();
    assert_ok(&import_result, "schedule_import");
    let import_json = extract_json(&import_result);
    assert_eq!(
        import_json["imported"].as_i64().unwrap(),
        2,
        "should import 2 schedules"
    );

    let list_after_import = client
        .call_tool(call_params("schedule_list", serde_json::json!({})))
        .await
        .unwrap();
    let reimported = extract_json(&list_after_import)["schedules"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(reimported.len(), 2, "should have 2 reimported schedules");

    let reimported_names: Vec<&str> = reimported
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(reimported_names.contains(&"export-test-1"));
    assert!(reimported_names.contains(&"export-test-2"));

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn desired_outcome_mismatch_fails() {
    let (client, db_path) = setup().await;

    let create_result = client
        .call_tool(call_params(
            "schedule_create",
            serde_json::json!({
                "name": "e2e-outcome-mismatch",
                "cron_expr": "0 0 * * *",
                "action_type": "mcp_tool",
                "action_payload": r#"{"tool":"memory_stats","args":{}}"#,
                "desired_outcome": "this_string_will_never_appear_in_output",
            }),
        ))
        .await
        .unwrap();
    assert_ok(&create_result, "schedule_create");
    let id = extract_json(&create_result)["id"]
        .as_str()
        .unwrap()
        .to_string();

    let trigger_result = client
        .call_tool(call_params(
            "schedule_trigger",
            serde_json::json!({"id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&trigger_result, "schedule_trigger");

    let runs_result = client
        .call_tool(call_params(
            "schedule_runs",
            serde_json::json!({"schedule_id": id}),
        ))
        .await
        .unwrap();
    assert_ok(&runs_result, "schedule_runs");
    let runs = extract_json(&runs_result)["runs"]
        .as_array()
        .unwrap()
        .clone();
    assert!(!runs.is_empty());
    assert_eq!(
        runs[0]["status"], "failed",
        "outcome mismatch should produce failed status"
    );

    let _ = std::fs::remove_file(&db_path);
}
