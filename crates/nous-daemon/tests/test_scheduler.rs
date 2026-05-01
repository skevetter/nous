use std::sync::Arc;
use std::time::Duration;

use nous_core::db::DbPools;
use nous_core::memory::MockEmbedder;
use nous_core::notifications::NotificationRegistry;
use nous_core::schedules::{create_schedule, get_schedule, list_runs, MockClock};
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tempfile::TempDir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

async fn setup(ts: i64) -> (AppState, Arc<MockClock>, CancellationToken, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let clock = Arc::new(MockClock::new(ts));
    let shutdown = CancellationToken::new();
    let state = AppState {
        pool: pools.fts.clone(),
        vec_pool: pools.vec.clone(),
        registry: Arc::new(NotificationRegistry::new()),
        embedder: Some(Arc::new(MockEmbedder::new())),
        schedule_notify: Arc::new(Notify::new()),
        shutdown: shutdown.clone(),
        process_registry: Arc::new(nous_daemon::process_manager::ProcessRegistry::new()),
    };
    (state, clock, shutdown, tmp)
}

/// Spawn scheduler, nudge it awake, wait for processing, then shut down.
async fn run_scheduler_cycle(
    state: &AppState,
    config: SchedulerConfig,
    clock: &Arc<MockClock>,
    shutdown: &CancellationToken,
    wait: Duration,
) {
    let handle = Scheduler::spawn(state.clone(), config, clock.clone(), shutdown.clone());
    // Give the scheduler time to enter its select! loop, then nudge repeatedly
    // to handle the race between mark_stale_runs_failed and the select!.
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        state.schedule_notify.notify_one();
    }
    tokio::time::sleep(wait).await;
    shutdown.cancel();
    let _ = handle.await;
}

// Base timestamp: 2026-01-01 00:00:00 UTC
const BASE_TS: i64 = 1_767_225_600;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_schedule_fires_at_correct_time() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(
        &state.pool,
        "recurring-fire",
        "* * * * *",
        None,
        None,
        "shell",
        r#"{"command": "echo fired"}"#,
        None,
        Some(0),
        None,
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    // next_run_at is ~60s after BASE_TS. Advance clock past it so schedule is due.
    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, Duration::from_secs(3)).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(!runs.is_empty(), "expected at least one run recorded");
    assert_eq!(runs[0].status, "completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_once_schedule_fires_and_disables() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let trigger_at = BASE_TS + 60;
    let schedule = create_schedule(
        &state.pool,
        "once-fire",
        "@once",
        Some(trigger_at),
        None,
        "shell",
        r#"{"command": "echo once"}"#,
        None,
        Some(0),
        None,
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    assert!(schedule.enabled);
    assert_eq!(schedule.next_run_at, Some(trigger_at));

    // Advance past trigger_at
    clock.set(trigger_at + 1);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, Duration::from_secs(2)).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(!runs.is_empty(), "expected a run recorded for @once schedule");
    assert_eq!(runs[0].status, "completed");

    let updated = get_schedule(&state.pool, &schedule.id).await.unwrap();
    assert!(!updated.enabled, "expected @once schedule to be disabled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_disabled_schedule_does_not_fire() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(
        &state.pool,
        "disabled-sched",
        "* * * * *",
        None,
        None,
        "shell",
        r#"{"command": "echo should-not-run"}"#,
        None,
        Some(0),
        None,
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    nous_core::schedules::update_schedule(
        &state.pool,
        &schedule.id,
        None,
        None,
        None,
        Some(false),
        None,
        None,
        None,
        None,
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, Duration::from_secs(2)).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(runs.is_empty(), "disabled schedule should not produce runs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_failed_action_recorded() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(
        &state.pool,
        "failing-shell",
        "* * * * *",
        None,
        None,
        "shell",
        r#"{"command": "exit 1"}"#,
        None,
        Some(0),
        None,
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, Duration::from_secs(2)).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(!runs.is_empty(), "expected a run recorded for failed action");
    assert_eq!(runs[0].status, "failed");
    assert!(
        runs[0].error.is_some(),
        "expected error message on failed run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_timeout() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(
        &state.pool,
        "timeout-shell",
        "* * * * *",
        None,
        None,
        "shell",
        r#"{"command": "sleep 60"}"#,
        None,
        Some(0),
        Some(1),
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, Duration::from_secs(4)).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(!runs.is_empty(), "expected a run recorded for timed-out action");
    assert_eq!(runs[0].status, "timeout");
    assert!(
        runs[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("timed out"),
        "expected timeout error message"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_desired_outcome_mismatch() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(
        &state.pool,
        "outcome-mismatch",
        "* * * * *",
        None,
        None,
        "shell",
        r#"{"command": "echo hello"}"#,
        Some("goodbye"),
        Some(0),
        None,
        None,
        None,
        &*clock,
    )
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, Duration::from_secs(2)).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(
        !runs.is_empty(),
        "expected a run recorded for outcome mismatch"
    );
    assert_eq!(runs[0].status, "failed");
    assert!(
        runs[0]
            .error
            .as_deref()
            .unwrap_or("")
            .contains("desired outcome"),
        "expected desired outcome mismatch error"
    );
}
