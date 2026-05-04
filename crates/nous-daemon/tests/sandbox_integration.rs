#![cfg(feature = "sandbox")]

mod common;

use std::sync::Arc;

use nous_core::db::DatabaseConnection;
use nous_core::memory::{EmbeddingConfig, MockEmbedder, VectorStoreConfig};
use nous_core::notifications::NotificationRegistry;
use nous_daemon::process_manager::{ProcessRegistry, SpawnParams};
use nous_daemon::sandbox::SandboxManager;
use nous_daemon::state::AppState;
use sea_orm::{ConnectionTrait, Statement};
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

async fn setup() -> (AppState, TempDir) {
    let (pools, tmp) = common::setup_test_db().await;
    let sandbox_mgr = Arc::new(tokio::sync::Mutex::new(SandboxManager::new()));
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder: Some(Arc::new(MockEmbedder::new())),
        embedding_config: EmbeddingConfig::default(),
        vector_store_config: VectorStoreConfig::default(),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: CancellationToken::new(),
        process_registry: Arc::new(ProcessRegistry::new()),
        llm_client: None,
        default_model: "test-model".to_string(),
        sandbox_manager: Some(sandbox_mgr),
    };
    (state, tmp)
}

async fn create_test_agent(pool: &DatabaseConnection, agent_id: &str) {
    pool.execute(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Sqlite,
        "INSERT INTO agents (id, name, namespace, status, process_type, metadata_json, created_at, updated_at) \
         VALUES (?, ?, 'default', 'active', 'sandbox', '{\"sandbox\":{\"image\":\"ubuntu:24.04\"}}', \
         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        [agent_id.into(), format!("test-agent-{}", agent_id).into()],
    ))
    .await
    .unwrap();
}

async fn insert_sandbox_process(
    pool: &DatabaseConnection,
    agent_id: &str,
    status: &str,
    sandbox_name: &str,
) -> String {
    let id = uuid::Uuid::now_v7().to_string();
    pool.execute(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Sqlite,
        "INSERT INTO agent_processes (id, agent_id, process_type, command, status, \
         max_output_bytes, restart_policy, restart_count, max_restarts, \
         sandbox_image, sandbox_name, created_at, updated_at) \
         VALUES (?, ?, 'sandbox', '', ?, 65536, 'never', 0, 3, \
         'ubuntu:24.04', ?, \
         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        [
            id.clone().into(),
            agent_id.into(),
            status.into(),
            sandbox_name.into(),
        ],
    ))
    .await
    .unwrap();
    id
}

async fn get_process_status(pool: &DatabaseConnection, process_id: &str) -> String {
    use sea_orm::TryGetable;
    let row = pool
        .query_one(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT status FROM agent_processes WHERE id = ?",
            [process_id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    String::try_get_by(&row, 0usize).unwrap()
}

#[tokio::test]
async fn test_sandbox_spawn_and_stop() {
    let (state, _tmp) = setup().await;
    let agent_id = "spawn-stop-agent";
    create_test_agent(&state.pool, agent_id).await;

    let process = state
        .process_registry
        .spawn(SpawnParams {
            state: &state,
            agent_id,
            command: "",
            process_type: "sandbox",
            working_dir: None,
            env: None,
            timeout_secs: None,
        })
        .await
        .unwrap();

    assert_eq!(process.process_type, "sandbox");
    assert_eq!(process.status, "running");
    assert!(process.sandbox_name.is_some());

    let status = state.process_registry.get_status(agent_id).await;
    assert!(status.is_some());

    let stopped = state
        .process_registry
        .stop(&state, agent_id, false, 5)
        .await
        .unwrap();
    assert_eq!(stopped.status, "stopped");

    let status = state.process_registry.get_status(agent_id).await;
    assert!(status.is_none());
}

#[tokio::test]
async fn test_sandbox_recovery_on_restart() {
    let (state, _tmp) = setup().await;
    let agent_id = "recovery-agent";
    create_test_agent(&state.pool, agent_id).await;

    let sandbox_name = "sandbox-recovery-agent-test123";
    let process_id = insert_sandbox_process(&state.pool, agent_id, "running", sandbox_name).await;

    let sandbox_mgr = state.sandbox_manager.as_ref().unwrap();

    // Pre-register the sandbox as live so reconnect verification succeeds
    {
        let mut mgr = sandbox_mgr.lock().await;
        mgr.register_known_sandbox(sandbox_name);
    }

    let new_registry = Arc::new(ProcessRegistry::new());
    new_registry
        .recover_sandboxes(&state.pool, sandbox_mgr)
        .await
        .unwrap();

    let status = new_registry.get_status(agent_id).await;
    assert!(status.is_some(), "sandbox should be recovered in HashMap");
    assert_eq!(status.unwrap().process_id, process_id);

    let mgr = sandbox_mgr.lock().await;
    let handle = mgr.get(agent_id);
    assert!(handle.is_some());
    assert_eq!(handle.unwrap().name, sandbox_name);
    assert_eq!(handle.unwrap().status, "running");
}

#[tokio::test]
async fn test_missing_sandbox_name_skipped() {
    let (state, _tmp) = setup().await;

    let crash_agent = "crash-no-name-agent";
    create_test_agent(&state.pool, crash_agent).await;
    let crash_id = {
        let id = uuid::Uuid::now_v7().to_string();
        state
            .pool
            .execute(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT INTO agent_processes (id, agent_id, process_type, command, status, \
             max_output_bytes, restart_policy, restart_count, max_restarts, \
             sandbox_image, sandbox_name, created_at, updated_at) \
             VALUES (?, ?, 'sandbox', '', 'running', 65536, 'never', 0, 3, \
             'ubuntu:24.04', NULL, \
             strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                [id.clone().into(), crash_agent.into()],
            ))
            .await
            .unwrap();
        id
    };

    let sandbox_mgr = state.sandbox_manager.as_ref().unwrap();
    let new_registry = Arc::new(ProcessRegistry::new());
    new_registry
        .recover_sandboxes(&state.pool, sandbox_mgr)
        .await
        .unwrap();

    let status = get_process_status(&state.pool, &crash_id).await;
    assert_eq!(status, "crashed");

    let registry_status = new_registry.get_status(crash_agent).await;
    assert!(
        registry_status.is_none(),
        "process with no sandbox_name should not be in registry"
    );
}

#[tokio::test]
async fn test_unreachable_sandbox_marked_crashed() {
    let (state, _tmp) = setup().await;
    let agent_id = "unreachable-agent";
    create_test_agent(&state.pool, agent_id).await;

    let sandbox_name = "sandbox-unreachable-test";
    let process_id = insert_sandbox_process(&state.pool, agent_id, "running", sandbox_name).await;

    // Do NOT register sandbox_name as live — reconnect should return Ok(false)
    let sandbox_mgr = state.sandbox_manager.as_ref().unwrap();
    let new_registry = Arc::new(ProcessRegistry::new());
    new_registry
        .recover_sandboxes(&state.pool, sandbox_mgr)
        .await
        .unwrap();

    let status = get_process_status(&state.pool, &process_id).await;
    assert_eq!(status, "crashed");

    let registry_status = new_registry.get_status(agent_id).await;
    assert!(
        registry_status.is_none(),
        "unreachable sandbox should not be in registry"
    );
}

#[tokio::test]
async fn test_sandbox_spawn_does_not_affect_shell() {
    let (state, _tmp) = setup().await;
    let agent_id = "shell-agent";
    create_test_agent(&state.pool, agent_id).await;

    // Override the agent's process_type so shell spawn works
    state
        .pool
        .execute(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "UPDATE agents SET process_type = 'shell' WHERE id = ?",
            [agent_id.into()],
        ))
        .await
        .unwrap();

    let process = state
        .process_registry
        .spawn(SpawnParams {
            state: &state,
            agent_id,
            command: "echo hello",
            process_type: "shell",
            working_dir: None,
            env: None,
            timeout_secs: Some(5),
        })
        .await
        .unwrap();

    assert_eq!(process.process_type, "shell");
    assert_eq!(process.status, "running");
    assert!(process.sandbox_name.is_none());

    // Give the process a moment to finish
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Shell process lifecycle remains unchanged
    let db_proc = nous_core::agents::processes::get_process_by_id(&state.pool, &process.id)
        .await
        .unwrap();
    assert!(db_proc.status == "running" || db_proc.status == "stopped");
}
