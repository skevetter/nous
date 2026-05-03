mod common;

use std::sync::Arc;
use std::time::Duration;

use nous_core::schedules::{
    create_schedule, get_schedule, list_runs, CreateScheduleParams, MockClock,
};
use nous_daemon::scheduler::{Scheduler, SchedulerConfig};
use nous_daemon::state::AppState;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

async fn setup(ts: i64) -> (AppState, Arc<MockClock>, CancellationToken, TempDir) {
    let (mut state, tmp) = common::test_state().await;
    let clock = Arc::new(MockClock::new(ts));
    let shutdown = CancellationToken::new();
    state.shutdown = shutdown.clone();
    (state, clock, shutdown, tmp)
}

/// Spawn the scheduler, nudge it until at least one run appears for `schedule_id`, then shut down.
async fn run_scheduler_cycle(
    state: &AppState,
    config: SchedulerConfig,
    clock: &Arc<MockClock>,
    shutdown: &CancellationToken,
    schedule_id: &str,
) {
    let handle = Scheduler::spawn(state.clone(), config, clock.clone(), shutdown.clone());
    poll_until_run(state, schedule_id, Duration::from_secs(10)).await;
    shutdown.cancel();
    let _ = handle.await;
}

/// Spawn the scheduler for a negative test (no run expected). Nudge and give a short window, then shut down.
async fn run_scheduler_cycle_no_run(
    state: &AppState,
    config: SchedulerConfig,
    clock: &Arc<MockClock>,
    shutdown: &CancellationToken,
) {
    let handle = Scheduler::spawn(state.clone(), config, clock.clone(), shutdown.clone());
    // Nudge a few times with minimal delay to ensure the scheduler processes the tick.
    for _ in 0..5 {
        tokio::task::yield_now().await;
        state.schedule_notify.notify_one();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Short grace period — enough for any incorrectly-triggered run to be recorded.
    tokio::time::sleep(Duration::from_millis(200)).await;
    shutdown.cancel();
    let _ = handle.await;
}

/// Poll `list_runs` until at least one run exists, nudging the scheduler each iteration.
async fn poll_until_run(state: &AppState, schedule_id: &str, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        state.schedule_notify.notify_one();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let runs = list_runs(&state.pool, schedule_id, None, None)
            .await
            .unwrap();
        if !runs.is_empty() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for schedule run to appear"
        );
    }
}

// Base timestamp: 2026-01-01 00:00:00 UTC
const BASE_TS: i64 = 1_767_225_600;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_schedule_fires_at_correct_time() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(CreateScheduleParams {
        db: &state.pool,
        name: "recurring-fire",
        cron_expr: "* * * * *",
        trigger_at: None,
        timezone: None,
        action_type: "shell",
        action_payload: r#"{"command": "echo fired"}"#,
        desired_outcome: None,
        max_retries: Some(0),
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    // next_run_at is ~60s after BASE_TS. Advance clock past it so schedule is due.
    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, &schedule.id).await;

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
    let schedule = create_schedule(CreateScheduleParams {
        db: &state.pool,
        name: "once-fire",
        cron_expr: "@once",
        trigger_at: Some(trigger_at),
        timezone: None,
        action_type: "shell",
        action_payload: r#"{"command": "echo once"}"#,
        desired_outcome: None,
        max_retries: Some(0),
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    assert!(schedule.enabled);
    assert_eq!(
        schedule.next_run_at,
        Some(nous_core::schedules::ts_to_iso(trigger_at))
    );

    // Advance past trigger_at
    clock.set(trigger_at + 1);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, &schedule.id).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(
        !runs.is_empty(),
        "expected a run recorded for @once schedule"
    );
    assert_eq!(runs[0].status, "completed");

    let updated = get_schedule(&state.pool, &schedule.id).await.unwrap();
    assert!(!updated.enabled, "expected @once schedule to be disabled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_disabled_schedule_does_not_fire() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(CreateScheduleParams {
        db: &state.pool,
        name: "disabled-sched",
        cron_expr: "* * * * *",
        trigger_at: None,
        timezone: None,
        action_type: "shell",
        action_payload: r#"{"command": "echo should-not-run"}"#,
        desired_outcome: None,
        max_retries: Some(0),
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    nous_core::schedules::update_schedule(nous_core::schedules::UpdateScheduleParams {
        db: &state.pool,
        id: &schedule.id,
        name: None,
        cron_expr: None,
        trigger_at: None,
        enabled: Some(false),
        action_type: None,
        action_payload: None,
        desired_outcome: None,
        max_retries: None,
        timeout_secs: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle_no_run(&state, config, &clock, &shutdown).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(runs.is_empty(), "disabled schedule should not produce runs");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_failed_action_recorded() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(CreateScheduleParams {
        db: &state.pool,
        name: "failing-shell",
        cron_expr: "* * * * *",
        trigger_at: None,
        timezone: None,
        action_type: "shell",
        action_payload: r#"{"command": "exit 1"}"#,
        desired_outcome: None,
        max_retries: Some(0),
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, &schedule.id).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(
        !runs.is_empty(),
        "expected a run recorded for failed action"
    );
    assert_eq!(runs[0].status, "failed");
    assert!(
        runs[0].error.is_some(),
        "expected error message on failed run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_shell_timeout() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(CreateScheduleParams {
        db: &state.pool,
        name: "timeout-shell",
        cron_expr: "* * * * *",
        trigger_at: None,
        timezone: None,
        action_type: "shell",
        action_payload: r#"{"command": "sleep 60"}"#,
        desired_outcome: None,
        max_retries: Some(0),
        timeout_secs: Some(1),
        max_output_bytes: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, &schedule.id).await;

    let runs = list_runs(&state.pool, &schedule.id, None, None)
        .await
        .unwrap();
    assert!(
        !runs.is_empty(),
        "expected a run recorded for timed-out action"
    );
    assert_eq!(runs[0].status, "timeout");
    assert!(
        runs[0].error.as_deref().unwrap_or("").contains("timed out"),
        "expected timeout error message"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_desired_outcome_mismatch() {
    let (state, clock, shutdown, _tmp) = setup(BASE_TS).await;

    let schedule = create_schedule(CreateScheduleParams {
        db: &state.pool,
        name: "outcome-mismatch",
        cron_expr: "* * * * *",
        trigger_at: None,
        timezone: None,
        action_type: "shell",
        action_payload: r#"{"command": "echo hello"}"#,
        desired_outcome: Some("goodbye"),
        max_retries: Some(0),
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &*clock,
    })
    .await
    .unwrap();

    clock.advance(120);

    let config = SchedulerConfig {
        allow_shell: true,
        ..Default::default()
    };
    run_scheduler_cycle(&state, config, &clock, &shutdown, &schedule.id).await;

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
