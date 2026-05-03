mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nous_daemon::app;
use nous_daemon::state::AppState;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn test_state() -> (AppState, TempDir) {
    common::test_state().await
}

async fn json_body(response: axum::http::Response<Body>) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

async fn mcp_call(state: &AppState, tool: &str, args: Value) -> Value {
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp/call")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "name": tool,
                        "arguments": args
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await
}

fn mcp_result(resp: &Value) -> Value {
    assert!(
        resp.get("is_error").is_none() || resp["is_error"] == false,
        "MCP tool returned error: {:?}",
        resp["content"][0]["text"]
    );
    let text = resp["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn mcp_error_text(resp: &Value) -> String {
    assert_eq!(resp["is_error"], true);
    resp["content"][0]["text"].as_str().unwrap().to_string()
}

// =============================================================================
// Task Dependency System
// =============================================================================

#[tokio::test]
async fn mcp_task_depends_add_list_remove() {
    let (state, _tmp) = test_state().await;

    let t1 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Parent task"})).await);
    let t2 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Child task"})).await);

    // Add dependency: t2 depends on t1
    let resp = mcp_call(
        &state,
        "task_depends_add",
        json!({"task_id": t2["id"], "depends_on_task_id": t1["id"]}),
    )
    .await;
    let dep = mcp_result(&resp);
    assert_eq!(dep["task_id"], t2["id"]);
    assert_eq!(dep["depends_on_task_id"], t1["id"]);

    // List dependencies
    let resp = mcp_call(&state, "task_depends_list", json!({"task_id": t2["id"]})).await;
    let deps = mcp_result(&resp);
    let deps_arr = deps.as_array().unwrap();
    assert_eq!(deps_arr.len(), 1);
    assert_eq!(deps_arr[0]["depends_on_task_id"], t1["id"]);

    // Remove dependency
    let resp = mcp_call(
        &state,
        "task_depends_remove",
        json!({"task_id": t2["id"], "depends_on_task_id": t1["id"]}),
    )
    .await;
    mcp_result(&resp);

    // Verify empty
    let resp = mcp_call(&state, "task_depends_list", json!({"task_id": t2["id"]})).await;
    let deps = mcp_result(&resp);
    assert!(deps.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn mcp_task_command() {
    let (state, _tmp) = test_state().await;

    let task = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Command task"})).await);

    // Execute a command on the task (e.g., close)
    let resp = mcp_call(
        &state,
        "task_command",
        json!({
            "command": "close",
            "task_id": task["id"],
            "actor_id": "test-agent"
        }),
    )
    .await;
    let result = mcp_result(&resp);
    // TaskCommandResult has: command, task_id, success, message, task
    assert_eq!(result["success"], true);
    assert_eq!(result["task"]["status"], "closed");
}

#[tokio::test]
async fn mcp_task_batch_update_status() {
    let (state, _tmp) = test_state().await;

    let t1 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Batch 1"})).await);
    let t2 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Batch 2"})).await);

    let resp = mcp_call(
        &state,
        "task_batch_update_status",
        json!({
            "task_ids": [t1["id"], t2["id"]],
            "status": "in_progress"
        }),
    )
    .await;
    let result = mcp_result(&resp);
    // BatchResult has: succeeded (array), failed (array)
    assert_eq!(result["succeeded"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn mcp_task_batch_close() {
    let (state, _tmp) = test_state().await;

    let t1 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Close 1"})).await);
    let t2 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Close 2"})).await);

    let resp = mcp_call(
        &state,
        "task_batch_close",
        json!({"task_ids": [t1["id"], t2["id"]]}),
    )
    .await;
    let result = mcp_result(&resp);
    assert_eq!(result["succeeded"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn mcp_task_batch_assign() {
    let (state, _tmp) = test_state().await;

    let t1 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Assign 1"})).await);
    let t2 = mcp_result(&mcp_call(&state, "task_create", json!({"title": "Assign 2"})).await);

    let resp = mcp_call(
        &state,
        "task_batch_assign",
        json!({
            "task_ids": [t1["id"], t2["id"]],
            "assignee_id": "agent-1"
        }),
    )
    .await;
    let result = mcp_result(&resp);
    assert_eq!(result["succeeded"].as_array().unwrap().len(), 2);
}

// =============================================================================
// Schedule Management via MCP
// =============================================================================

#[tokio::test]
async fn mcp_schedule_create_list_get_update_delete() {
    let (state, _tmp) = test_state().await;

    // Create
    let resp = mcp_call(
        &state,
        "schedule_create",
        json!({
            "name": "mcp-schedule",
            "cron_expr": "0 * * * *",
            "action_type": "shell",
            "action_payload": "echo hello",
            "max_retries": 2,
            "timeout_secs": 30
        }),
    )
    .await;
    let schedule = mcp_result(&resp);
    assert_eq!(schedule["name"], "mcp-schedule");
    assert_eq!(schedule["action_type"], "shell");
    assert_eq!(schedule["enabled"], true);
    let schedule_id = schedule["id"].as_str().unwrap();

    // List
    let resp = mcp_call(&state, "schedule_list", json!({})).await;
    let schedules = mcp_result(&resp);
    let arr = schedules.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "mcp-schedule");

    // Get
    let resp = mcp_call(&state, "schedule_get", json!({"id": schedule_id})).await;
    let fetched = mcp_result(&resp);
    assert_eq!(fetched["name"], "mcp-schedule");

    // Update
    let resp = mcp_call(
        &state,
        "schedule_update",
        json!({
            "id": schedule_id,
            "enabled": false,
            "name": "updated-schedule"
        }),
    )
    .await;
    let updated = mcp_result(&resp);
    assert_eq!(updated["name"], "updated-schedule");
    assert_eq!(updated["enabled"], false);

    // Delete
    let resp = mcp_call(&state, "schedule_delete", json!({"id": schedule_id})).await;
    mcp_result(&resp);

    // Verify gone
    let resp = mcp_call(&state, "schedule_get", json!({"id": schedule_id})).await;
    assert_eq!(resp["is_error"], true);
}

#[tokio::test]
async fn mcp_schedule_runs_list() {
    let (state, _tmp) = test_state().await;

    let schedule = mcp_result(
        &mcp_call(
            &state,
            "schedule_create",
            json!({
                "name": "runs-schedule",
                "cron_expr": "0 0 * * *",
                "action_type": "shell",
                "action_payload": "echo runs"
            }),
        )
        .await,
    );
    let sid = schedule["id"].as_str().unwrap();

    let resp = mcp_call(&state, "schedule_runs_list", json!({"schedule_id": sid})).await;
    let runs = mcp_result(&resp);
    assert!(runs.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn mcp_schedule_health() {
    let (state, _tmp) = test_state().await;

    mcp_result(
        &mcp_call(
            &state,
            "schedule_create",
            json!({
                "name": "health-sched",
                "cron_expr": "0 0 * * *",
                "action_type": "shell",
                "action_payload": "echo x"
            }),
        )
        .await,
    );

    let resp = mcp_call(&state, "schedule_health", json!({})).await;
    let health = mcp_result(&resp);
    assert_eq!(health["total"], 1);
    assert_eq!(health["active"], 1);
}

// =============================================================================
// Resource Management via MCP
// =============================================================================

#[tokio::test]
async fn mcp_resource_get_update_archive_heartbeat_search() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "res-owner"})).await,
    );

    // Register
    let resource = mcp_result(
        &mcp_call(
            &state,
            "resource_register",
            json!({
                "name": "mcp-resource",
                "type": "file",
                "owner_agent_id": agent["id"],
                "tags": ["ci", "deploy"]
            }),
        )
        .await,
    );
    let res_id = resource["id"].as_str().unwrap();

    // Get
    let resp = mcp_call(&state, "resource_get", json!({"id": res_id})).await;
    let fetched = mcp_result(&resp);
    assert_eq!(fetched["name"], "mcp-resource");

    // Update
    let resp = mcp_call(
        &state,
        "resource_update",
        json!({
            "id": res_id,
            "name": "updated-resource",
            "path": "/new/path",
            "tags": ["updated"]
        }),
    )
    .await;
    let updated = mcp_result(&resp);
    assert_eq!(updated["name"], "updated-resource");
    assert_eq!(updated["path"], "/new/path");

    // Heartbeat
    let resp = mcp_call(&state, "resource_heartbeat", json!({"id": res_id})).await;
    let hb = mcp_result(&resp);
    assert!(hb["last_seen_at"].as_str().is_some());

    // Search by tags
    let resp = mcp_call(&state, "resource_search", json!({"tags": ["updated"]})).await;
    let results = mcp_result(&resp);
    let arr = results.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "updated-resource");

    // Archive
    let resp = mcp_call(&state, "resource_archive", json!({"id": res_id})).await;
    let archived = mcp_result(&resp);
    assert_eq!(archived["status"], "archived");
}

#[tokio::test]
async fn mcp_resource_transfer() {
    let (state, _tmp) = test_state().await;

    let from = mcp_result(&mcp_call(&state, "agent_register", json!({"name": "from-ag"})).await);
    let to = mcp_result(&mcp_call(&state, "agent_register", json!({"name": "to-ag"})).await);

    mcp_result(
        &mcp_call(
            &state,
            "resource_register",
            json!({"name": "xfer-res", "type": "worktree", "owner_agent_id": from["id"]}),
        )
        .await,
    );

    let resp = mcp_call(
        &state,
        "resource_transfer",
        json!({"from_agent_id": from["id"], "to_agent_id": to["id"]}),
    )
    .await;
    let result = mcp_result(&resp);
    assert_eq!(result["transferred"], 1);
}

// =============================================================================
// Agent Process Control
// =============================================================================

#[tokio::test]
async fn mcp_agent_spawn_and_process_status() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "spawn-agent"})).await,
    );
    let agent_id = agent["id"].as_str().unwrap();

    // Update agent to shell process type
    mcp_result(
        &mcp_call(
            &state,
            "agent_update",
            json!({"id": agent_id, "process_type": "shell"}),
        )
        .await,
    );

    // Spawn a short-lived process
    let resp = mcp_call(
        &state,
        "agent_spawn",
        json!({
            "agent_id": agent_id,
            "command": "sleep 10",
            "timeout_secs": 30
        }),
    )
    .await;
    let process = mcp_result(&resp);
    assert_eq!(process["agent_id"], agent_id);
    assert_eq!(process["status"], "running");

    // Check process status
    let resp = mcp_call(&state, "agent_process_status", json!({"agent_id": agent_id})).await;
    let status = mcp_result(&resp);
    assert!(status.get("runtime").is_some() || status.get("process").is_some());

    // Stop the process
    let resp = mcp_call(
        &state,
        "agent_stop",
        json!({"agent_id": agent_id, "force": true}),
    )
    .await;
    mcp_result(&resp);
}

#[tokio::test]
async fn mcp_agent_invocations_list() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "invoc-agent"})).await,
    );
    let agent_id = agent["id"].as_str().unwrap();

    // Set process_type to shell for invocation
    mcp_result(
        &mcp_call(
            &state,
            "agent_update",
            json!({"id": agent_id, "process_type": "shell"}),
        )
        .await,
    );

    // Invoke
    mcp_result(
        &mcp_call(
            &state,
            "agent_invoke",
            json!({"agent_id": agent_id, "prompt": "echo test", "timeout_secs": 10}),
        )
        .await,
    );

    // List invocations
    let resp = mcp_call(&state, "agent_invocations", json!({"agent_id": agent_id})).await;
    let invocations = mcp_result(&resp);
    let arr = invocations.as_array().unwrap();
    assert!(!arr.is_empty());
    assert_eq!(arr[0]["agent_id"], agent_id);
}

#[tokio::test]
async fn mcp_agent_invoke_result() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "result-agent"})).await,
    );
    let agent_id = agent["id"].as_str().unwrap();

    mcp_result(
        &mcp_call(
            &state,
            "agent_update",
            json!({"id": agent_id, "process_type": "shell"}),
        )
        .await,
    );

    let invocation = mcp_result(
        &mcp_call(
            &state,
            "agent_invoke",
            json!({"agent_id": agent_id, "prompt": "echo result-check", "timeout_secs": 10}),
        )
        .await,
    );
    let invocation_id = invocation["id"].as_str().unwrap();

    // Get result
    let resp = mcp_call(
        &state,
        "agent_invoke_result",
        json!({"invocation_id": invocation_id}),
    )
    .await;
    let result = mcp_result(&resp);
    assert_eq!(result["status"], "completed");
    assert!(result["result"].as_str().unwrap().contains("result-check"));
}

// =============================================================================
// Memory Tools via MCP
// =============================================================================

#[tokio::test]
async fn mcp_memory_get_update_search() {
    let (state, _tmp) = test_state().await;

    // Save
    let mem = mcp_result(
        &mcp_call(
            &state,
            "memory_save",
            json!({
                "title": "MCP memory test",
                "content": "This is a test memory about deployment patterns",
                "type": "observation",
                "importance": "high"
            }),
        )
        .await,
    );
    let mem_id = mem["id"].as_str().unwrap();

    // Get
    let resp = mcp_call(&state, "memory_get", json!({"id": mem_id})).await;
    let fetched = mcp_result(&resp);
    assert_eq!(fetched["title"], "MCP memory test");

    // Update
    let resp = mcp_call(
        &state,
        "memory_update",
        json!({
            "id": mem_id,
            "title": "Updated memory",
            "importance": "moderate"
        }),
    )
    .await;
    let updated = mcp_result(&resp);
    assert_eq!(updated["title"], "Updated memory");
    assert_eq!(updated["importance"], "moderate");

    // Search
    let resp = mcp_call(
        &state,
        "memory_search",
        json!({"query": "deployment patterns"}),
    )
    .await;
    let results = mcp_result(&resp);
    let arr = results.as_array().unwrap();
    assert!(!arr.is_empty());
}

#[tokio::test]
async fn mcp_memory_relate_and_list_relations() {
    let (state, _tmp) = test_state().await;

    let m1 = mcp_result(
        &mcp_call(
            &state,
            "memory_save",
            json!({"title": "Source mem", "content": "src", "type": "decision"}),
        )
        .await,
    );
    let m2 = mcp_result(
        &mcp_call(
            &state,
            "memory_save",
            json!({"title": "Target mem", "content": "tgt", "type": "decision"}),
        )
        .await,
    );

    // Relate
    let resp = mcp_call(
        &state,
        "memory_relate",
        json!({
            "source_id": m1["id"],
            "target_id": m2["id"],
            "relation_type": "supersedes"
        }),
    )
    .await;
    let rel = mcp_result(&resp);
    assert_eq!(rel["relation_type"], "supersedes");

    // List relations
    let resp = mcp_call(&state, "memory_relations", json!({"id": m1["id"]})).await;
    let relations = mcp_result(&resp);
    let arr = relations.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["target_id"], m2["id"]);
}

#[tokio::test]
async fn mcp_memory_context() {
    let (state, _tmp) = test_state().await;

    mcp_result(
        &mcp_call(
            &state,
            "memory_save",
            json!({
                "title": "Context memory",
                "content": "Relevant context for workspace",
                "type": "fact",
                "workspace_id": "ws-ctx",
                "agent_id": "agent-ctx"
            }),
        )
        .await,
    );

    let resp = mcp_call(
        &state,
        "memory_context",
        json!({"workspace_id": "ws-ctx", "agent_id": "agent-ctx", "limit": 10}),
    )
    .await;
    let results = mcp_result(&resp);
    let arr = results.as_array().unwrap();
    assert!(!arr.is_empty());
    assert_eq!(arr[0]["title"], "Context memory");
}

#[tokio::test]
async fn mcp_memory_session_start_end() {
    let (state, _tmp) = test_state().await;

    let resp = mcp_call(
        &state,
        "memory_session_start",
        json!({"agent_id": "sess-agent", "project": "test-project"}),
    )
    .await;
    let session = mcp_result(&resp);
    let session_id = session["id"].as_str().unwrap();
    assert!(session_id.len() > 0);

    let resp = mcp_call(
        &state,
        "memory_session_end",
        json!({"session_id": session_id}),
    )
    .await;
    mcp_result(&resp);
}


// =============================================================================
// Agent Utility Tools
// =============================================================================

#[tokio::test]
async fn mcp_agent_update_status() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "status-agent"})).await,
    );

    let resp = mcp_call(
        &state,
        "agent_update_status",
        json!({"id": agent["id"], "status": "running"}),
    )
    .await;
    let updated = mcp_result(&resp);
    assert_eq!(updated["status"], "running");
}

#[tokio::test]
async fn mcp_agent_search_and_stale() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(
            &state,
            "agent_register",
            json!({"name": "searchstale", "namespace": "stale-ns"}),
        )
        .await,
    );

    // Search
    let resp = mcp_call(
        &state,
        "agent_search",
        json!({"query": "searchstale", "namespace": "stale-ns"}),
    )
    .await;
    let results = mcp_result(&resp);
    let arr = results.as_array().unwrap();
    assert_eq!(arr.len(), 1);

    // Heartbeat then check stale
    mcp_result(
        &mcp_call(
            &state,
            "agent_heartbeat",
            json!({"id": agent["id"], "status": "active"}),
        )
        .await,
    );

    let resp = mcp_call(
        &state,
        "agent_stale",
        json!({"threshold": 0, "namespace": "stale-ns"}),
    )
    .await;
    let stale = mcp_result(&resp);
    let arr = stale.as_array().unwrap();
    assert!(!arr.is_empty());
}

#[tokio::test]
async fn mcp_agent_inspect() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "inspect-ag"})).await,
    );

    let resp = mcp_call(&state, "agent_inspect", json!({"id": agent["id"]})).await;
    let inspection = mcp_result(&resp);
    assert_eq!(inspection["name"], "inspect-ag");
}

#[tokio::test]
async fn mcp_agent_versions_and_rollback() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(&state, "agent_register", json!({"name": "ver-agent"})).await,
    );
    let agent_id = agent["id"].as_str().unwrap();

    // Record v1
    let v1 = mcp_result(
        &mcp_call(
            &state,
            "agent_record_version",
            json!({
                "agent_id": agent_id,
                "skill_hash": "hash1",
                "config_hash": "cfg1"
            }),
        )
        .await,
    );

    // Record v2
    mcp_result(
        &mcp_call(
            &state,
            "agent_record_version",
            json!({
                "agent_id": agent_id,
                "skill_hash": "hash2",
                "config_hash": "cfg2"
            }),
        )
        .await,
    );

    // List versions
    let resp = mcp_call(&state, "agent_versions", json!({"agent_id": agent_id})).await;
    let versions = mcp_result(&resp);
    assert_eq!(versions.as_array().unwrap().len(), 2);

    // Rollback to v1
    let resp = mcp_call(
        &state,
        "agent_rollback",
        json!({"agent_id": agent_id, "version_id": v1["id"]}),
    )
    .await;
    let rolled = mcp_result(&resp);
    assert_eq!(rolled["skill_hash"], "hash1");
}

#[tokio::test]
async fn mcp_agent_notify_upgrade_and_outdated() {
    let (state, _tmp) = test_state().await;

    let agent = mcp_result(
        &mcp_call(
            &state,
            "agent_register",
            json!({"name": "outdated-ag", "namespace": "upg-ns"}),
        )
        .await,
    );

    mcp_result(
        &mcp_call(
            &state,
            "agent_notify_upgrade",
            json!({"id": agent["id"]}),
        )
        .await,
    );

    let resp = mcp_call(
        &state,
        "agent_outdated",
        json!({"namespace": "upg-ns"}),
    )
    .await;
    let outdated = mcp_result(&resp);
    let arr = outdated.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "outdated-ag");
}

#[tokio::test]
async fn mcp_agent_template_lifecycle() {
    let (state, _tmp) = test_state().await;

    // Create template
    let tpl = mcp_result(
        &mcp_call(
            &state,
            "agent_template_create",
            json!({
                "name": "mcp-template",
                "type": "worker",
                "default_config": "{\"threads\": 4}"
            }),
        )
        .await,
    );
    let tpl_id = tpl["id"].as_str().unwrap();
    assert_eq!(tpl["name"], "mcp-template");

    // List
    let resp = mcp_call(&state, "agent_template_list", json!({"type": "worker"})).await;
    let templates = mcp_result(&resp);
    assert_eq!(templates.as_array().unwrap().len(), 1);

    // Get
    let resp = mcp_call(&state, "agent_template_get", json!({"id": tpl_id})).await;
    let fetched = mcp_result(&resp);
    assert_eq!(fetched["name"], "mcp-template");

    // Instantiate
    let resp = mcp_call(
        &state,
        "agent_instantiate",
        json!({"template_id": tpl_id, "name": "from-template", "namespace": "tpl-ns"}),
    )
    .await;
    let agent = mcp_result(&resp);
    assert_eq!(agent["name"], "from-template");
    assert_eq!(agent["namespace"], "tpl-ns");
}

// =============================================================================
// Task Template System
// =============================================================================

#[tokio::test]
async fn mcp_task_template_create_list_get_use() {
    let (state, _tmp) = test_state().await;

    // Create template — uses {{var}} syntax for substitution
    let resp = mcp_call(
        &state,
        "task_template_create",
        json!({
            "name": "bug-template",
            "title_pattern": "[BUG] {{component}}: {{summary}}",
            "description_template": "Steps to reproduce:\n1. ...",
            "default_priority": "high",
            "default_labels": ["bug", "triage"]
        }),
    )
    .await;
    let tpl = mcp_result(&resp);
    let tpl_id = tpl["id"].as_str().unwrap();
    assert_eq!(tpl["name"], "bug-template");

    // List
    let resp = mcp_call(&state, "task_template_list", json!({})).await;
    let templates = mcp_result(&resp);
    assert_eq!(templates.as_array().unwrap().len(), 1);

    // Get
    let resp = mcp_call(&state, "task_template_get", json!({"id": tpl_id})).await;
    let fetched = mcp_result(&resp);
    assert_eq!(fetched["name"], "bug-template");

    // Use template
    let resp = mcp_call(
        &state,
        "task_template_use",
        json!({
            "template_id": tpl_id,
            "title_vars": {"component": "auth", "summary": "login fails"},
            "assignee_id": "agent-1"
        }),
    )
    .await;
    let task = mcp_result(&resp);
    assert!(task["title"].as_str().unwrap().contains("auth"));
    assert!(task["title"].as_str().unwrap().contains("login fails"));
    assert_eq!(task["priority"], "high");
}

// =============================================================================
// Room Messaging Tools
// =============================================================================

#[tokio::test]
async fn mcp_room_mark_read_and_unread_count() {
    let (state, _tmp) = test_state().await;

    let room = mcp_result(
        &mcp_call(&state, "room_create", json!({"name": "read-track-room"})).await,
    );
    let room_id = room["id"].as_str().unwrap();

    // Post a message using pre-seeded agent
    let msg = mcp_result(
        &mcp_call(
            &state,
            "room_post_message",
            json!({"room_id": room_id, "sender_id": "agent-1", "content": "msg1"}),
        )
        .await,
    );
    let msg_id = msg["id"].as_str().expect("message should have id");

    // Check unread count using pre-seeded agent (FK requires valid agent)
    let resp = mcp_call(
        &state,
        "room_unread_count",
        json!({"room_id": room_id, "agent_id": "agent-2"}),
    )
    .await;
    let count = mcp_result(&resp);
    assert!(count["unread_count"].as_i64().unwrap() >= 1);

    // Mark as read
    let resp = mcp_call(
        &state,
        "room_mark_read",
        json!({"room_id": room_id, "agent_id": "agent-2", "message_id": msg_id}),
    )
    .await;
    mcp_result(&resp);

    // Unread count should be 0
    let resp = mcp_call(
        &state,
        "room_unread_count",
        json!({"room_id": room_id, "agent_id": "agent-2"}),
    )
    .await;
    let count = mcp_result(&resp);
    assert_eq!(count["unread_count"], 0);
}
