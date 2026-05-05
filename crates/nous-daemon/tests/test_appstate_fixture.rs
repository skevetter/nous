mod common;

use nous_daemon::process_manager::ProcessRegistry;
use nous_daemon::state::AppState;
use tempfile::TempDir;

async fn setup() -> (AppState, TempDir) {
    common::test_state().await
}

#[tokio::test]
async fn appstate_fixture_builds_successfully() {
    let (state, _tmp) = setup().await;

    assert_eq!(state.default_model, "test-model");
    assert!(state.llm_client.is_none());
    assert!(state.embedder.is_some());
}

#[tokio::test]
async fn appstate_shutdown_token_starts_uncancelled() {
    let (state, _tmp) = setup().await;

    assert!(!state.shutdown.is_cancelled());
}

#[tokio::test]
async fn appstate_shutdown_token_can_be_cancelled() {
    let (state, _tmp) = setup().await;

    state.shutdown.cancel();

    assert!(state.shutdown.is_cancelled());
}

#[tokio::test]
async fn appstate_clone_shares_shutdown_token() {
    let (state, _tmp) = setup().await;
    let cloned = state.clone();

    state.shutdown.cancel();

    // Clone shares the same token — cancelling one cancels both
    assert!(cloned.shutdown.is_cancelled());
}

#[tokio::test]
async fn appstate_process_registry_starts_empty() {
    let (state, _tmp) = setup().await;

    let status = state.process_registry.get_status("any-agent").await;
    assert!(status.is_none());
}

#[tokio::test]
async fn process_registry_shutdown_with_no_processes_completes() {
    let (state, _tmp) = setup().await;

    // shutdown() with no registered processes should complete without error
    state.process_registry.shutdown(&state).await;
}

#[tokio::test]
async fn appstate_schedule_notify_can_be_triggered() {
    let (state, _tmp) = setup().await;

    // Triggering notify should not panic or block
    state.schedule_notify.notify_one();
}

#[tokio::test]
async fn process_registry_new_is_empty() {
    let registry = ProcessRegistry::new();

    let status = registry.get_status("agent-x").await;
    assert!(status.is_none());
}

#[tokio::test]
async fn appstate_pool_is_usable() {
    use sea_orm::ConnectionTrait;

    let (state, _tmp) = setup().await;

    // Verify the DB connection is live by running a simple query
    let result = state
        .pool
        .execute_unprepared("SELECT 1")
        .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn appstate_shutdown_cancellation_is_observable_via_future() {
    let (state, _tmp) = setup().await;
    let shutdown = state.shutdown.clone();

    // Spawn a task that waits for cancellation
    let handle = tokio::spawn(async move {
        shutdown.cancelled().await;
        "cancelled"
    });

    state.shutdown.cancel();

    let result = tokio::time::timeout(std::time::Duration::from_millis(100), handle)
        .await
        .expect("timed out waiting for shutdown future")
        .expect("task panicked");

    assert_eq!(result, "cancelled");
}
