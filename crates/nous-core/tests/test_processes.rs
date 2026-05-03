mod common;

use nous_core::agents::processes::{
    cleanup_agent_processes, create_invocation, create_process, get_active_process,
    get_invocation, get_latest_process, get_process_by_id, increment_restart_count,
    list_all_active_processes, list_invocations, list_processes, update_invocation,
    update_process_status, CreateProcessParams,
};
use nous_core::agents::{self, RegisterAgentRequest};

async fn setup() -> (sea_orm::DatabaseConnection, tempfile::TempDir) {
    let (pools, tmp) = common::setup_test_db().await;
    (pools.fts, tmp)
}

async fn create_test_agent(db: &sea_orm::DatabaseConnection, name: &str) -> String {
    let agent = agents::register_agent(
        db,
        RegisterAgentRequest {
            name: name.into(),
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();
    agent.id
}

// --- Process CRUD ---

#[tokio::test]
async fn create_and_get_process() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "proc-agent").await;

    let proc = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "echo test",
        working_dir: None,
        env_json: None,
        timeout_secs: Some(30),
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    assert_eq!(proc.agent_id, agent_id);
    assert_eq!(proc.process_type, "shell");
    assert_eq!(proc.command, "echo test");
    assert_eq!(proc.status, "pending");
    assert_eq!(proc.timeout_secs, Some(30));
    assert_eq!(proc.restart_policy, "never");
    assert_eq!(proc.max_restarts, 3);

    let fetched = get_process_by_id(&db, &proc.id).await.unwrap();
    assert_eq!(fetched.id, proc.id);
}

#[tokio::test]
async fn create_process_for_nonexistent_agent_fails() {
    let (db, _tmp) = setup().await;

    let result = create_process(CreateProcessParams {
        db: &db,
        agent_id: "nonexistent",
        process_type: "shell",
        command: "echo test",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn update_process_status_transitions() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "status-agent").await;

    let proc = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "long-running",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    assert_eq!(proc.status, "pending");

    let updated = update_process_status(&db, &proc.id, "running", None, None, Some(1234)).await.unwrap();
    assert_eq!(updated.status, "running");
    assert_eq!(updated.pid, Some(1234));
    assert!(updated.started_at.is_some());

    let stopped = update_process_status(&db, &proc.id, "stopped", Some(0), Some("done"), None)
        .await
        .unwrap();
    assert_eq!(stopped.status, "stopped");
    assert_eq!(stopped.exit_code, Some(0));
    assert_eq!(stopped.last_output.as_deref(), Some("done"));
    assert!(stopped.stopped_at.is_some());
}

#[tokio::test]
async fn get_active_process_returns_running() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "active-agent").await;

    let proc = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "sleep 100",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    // pending is an active status
    let active = get_active_process(&db, &agent_id).await.unwrap();
    assert!(active.is_some());
    assert_eq!(active.unwrap().id, proc.id);

    // After stopping, no active process
    update_process_status(&db, &proc.id, "stopped", Some(0), None, None)
        .await
        .unwrap();
    let active = get_active_process(&db, &agent_id).await.unwrap();
    assert!(active.is_none());
}

#[tokio::test]
async fn get_latest_process_ordering() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "latest-agent").await;

    let first = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "first",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    // Stop first so partial unique index allows creating another
    update_process_status(&db, &first.id, "stopped", Some(0), None, None)
        .await
        .unwrap();

    let second = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "second",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    let latest = get_latest_process(&db, &agent_id).await.unwrap();
    assert_eq!(latest.unwrap().id, second.id);
}

#[tokio::test]
async fn increment_restart_count_works() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "restart-agent").await;

    let proc = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "crasher",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: Some("always"),
        max_restarts: Some(5),
    })
    .await
    .unwrap();

    assert_eq!(proc.restart_count, 0);

    increment_restart_count(&db, &proc.id).await.unwrap();
    increment_restart_count(&db, &proc.id).await.unwrap();

    let fetched = get_process_by_id(&db, &proc.id).await.unwrap();
    assert_eq!(fetched.restart_count, 2);
}

#[tokio::test]
async fn list_processes_respects_limit() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "list-agent").await;

    let mut last_id = String::new();
    for i in 0..5 {
        if !last_id.is_empty() {
            update_process_status(&db, &last_id, "stopped", Some(0), None, None)
                .await
                .unwrap();
        }
        let proc = create_process(CreateProcessParams {
            db: &db,
            agent_id: &agent_id,
            process_type: "shell",
            command: &format!("cmd-{i}"),
            working_dir: None,
            env_json: None,
            timeout_secs: None,
            restart_policy: None,
            max_restarts: None,
        })
        .await
        .unwrap();
        last_id = proc.id;
    }

    let all = list_processes(&db, &agent_id, None).await.unwrap();
    assert_eq!(all.len(), 5);

    let limited = list_processes(&db, &agent_id, Some(3)).await.unwrap();
    assert_eq!(limited.len(), 3);
}

#[tokio::test]
async fn list_all_active_processes_filters_correctly() {
    let (db, _tmp) = setup().await;
    let agent1 = create_test_agent(&db, "allactive-agent-1").await;
    let agent2 = create_test_agent(&db, "allactive-agent-2").await;

    let p1 = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent1,
        process_type: "shell",
        command: "active-one",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    let p2 = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent2,
        process_type: "shell",
        command: "stopped-one",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    update_process_status(&db, &p2.id, "stopped", Some(0), None, None)
        .await
        .unwrap();

    let active = list_all_active_processes(&db).await.unwrap();
    assert!(active.iter().any(|p| p.id == p1.id));
    assert!(!active.iter().any(|p| p.id == p2.id));
}

#[tokio::test]
async fn cleanup_agent_processes_removes_all() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "cleanup-agent").await;

    let p1 = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "proc-1",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    // Stop first so we can create a second
    update_process_status(&db, &p1.id, "stopped", Some(0), None, None)
        .await
        .unwrap();

    create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "proc-2",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    cleanup_agent_processes(&db, &agent_id).await.unwrap();

    let remaining = list_processes(&db, &agent_id, None).await.unwrap();
    assert!(remaining.is_empty());
}

// --- Invocation CRUD ---

#[tokio::test]
async fn create_and_get_invocation() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "inv-agent").await;

    let inv = create_invocation(&db, &agent_id, "hello world", None)
        .await
        .unwrap();

    assert_eq!(inv.agent_id, agent_id);
    assert_eq!(inv.prompt, "hello world");
    assert_eq!(inv.status, "pending");
    assert!(inv.result.is_none());
    assert!(inv.error.is_none());

    let fetched = get_invocation(&db, &inv.id).await.unwrap();
    assert_eq!(fetched.id, inv.id);
}

#[tokio::test]
async fn create_invocation_for_nonexistent_agent_fails() {
    let (db, _tmp) = setup().await;

    let result = create_invocation(&db, "nonexistent", "test", None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn update_invocation_to_completed() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "inv-complete-agent").await;

    let inv = create_invocation(&db, &agent_id, "test prompt", None)
        .await
        .unwrap();

    let updated = update_invocation(&db, &inv.id, "completed", Some("result text"), None, Some(150))
        .await
        .unwrap();

    assert_eq!(updated.status, "completed");
    assert_eq!(updated.result.as_deref(), Some("result text"));
    assert_eq!(updated.duration_ms, Some(150));
    assert!(updated.completed_at.is_some());
}

#[tokio::test]
async fn update_invocation_to_failed() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "inv-fail-agent").await;

    let inv = create_invocation(&db, &agent_id, "test prompt", None)
        .await
        .unwrap();

    let updated = update_invocation(&db, &inv.id, "failed", None, Some("something broke"), Some(50))
        .await
        .unwrap();

    assert_eq!(updated.status, "failed");
    assert_eq!(updated.error.as_deref(), Some("something broke"));
    assert!(updated.completed_at.is_some());
}

#[tokio::test]
async fn list_invocations_with_status_filter() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "inv-list-agent").await;

    let inv1 = create_invocation(&db, &agent_id, "prompt-1", None)
        .await
        .unwrap();
    let _inv2 = create_invocation(&db, &agent_id, "prompt-2", None)
        .await
        .unwrap();

    update_invocation(&db, &inv1.id, "completed", Some("done"), None, None)
        .await
        .unwrap();

    let all = list_invocations(&db, &agent_id, None, None).await.unwrap();
    assert_eq!(all.len(), 2);

    let completed = list_invocations(&db, &agent_id, Some("completed"), None)
        .await
        .unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].id, inv1.id);

    let pending = list_invocations(&db, &agent_id, Some("pending"), None)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
}

#[tokio::test]
async fn invocation_links_to_active_process() {
    let (db, _tmp) = setup().await;
    let agent_id = create_test_agent(&db, "inv-link-agent").await;

    let proc = create_process(CreateProcessParams {
        db: &db,
        agent_id: &agent_id,
        process_type: "shell",
        command: "running-cmd",
        working_dir: None,
        env_json: None,
        timeout_secs: None,
        restart_policy: None,
        max_restarts: None,
    })
    .await
    .unwrap();

    let inv = create_invocation(&db, &agent_id, "test", None)
        .await
        .unwrap();

    assert_eq!(inv.process_id.as_deref(), Some(proc.id.as_str()));
}
