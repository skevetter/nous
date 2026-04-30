use nous_core::db::DbPools;
use nous_core::schedules::{self, Clock, CronExpr, MockClock};
use tempfile::TempDir;

async fn setup() -> (sqlx::SqlitePool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools.fts, tmp)
}

#[tokio::test]
async fn create_schedule_basic() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let schedule = schedules::create_schedule(
        &pool,
        "test-schedule",
        "*/5 * * * *",
        None,
        None,
        "mcp_tool",
        r#"{"tool":"memory_stats","args":{}}"#,
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    assert_eq!(schedule.name, "test-schedule");
    assert_eq!(schedule.cron_expr, "*/5 * * * *");
    assert_eq!(schedule.action_type, "mcp_tool");
    assert!(schedule.enabled);
    assert!(schedule.next_run_at.is_some());
    assert!(schedule.next_run_at.unwrap() > 1_700_000_000);
}

#[tokio::test]
async fn create_schedule_with_trigger_at() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);
    let trigger = 1_700_001_000;

    let schedule = schedules::create_schedule(
        &pool,
        "oneshot",
        "@once",
        Some(trigger),
        None,
        "http",
        r#"{"method":"GET","url":"http://example.com"}"#,
        None,
        None,
        None,
        None,
        Some(1),
        &clock,
    )
    .await
    .unwrap();

    assert_eq!(schedule.trigger_at, Some(trigger));
    assert_eq!(schedule.next_run_at, Some(trigger));
    assert_eq!(schedule.max_runs, 1);
}

#[tokio::test]
async fn create_schedule_invalid_cron() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let result = schedules::create_schedule(
        &pool,
        "bad-cron",
        "invalid expression",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn create_schedule_empty_name() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let result = schedules::create_schedule(
        &pool,
        "",
        "* * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn get_schedule() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let created = schedules::create_schedule(
        &pool,
        "get-test",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    let fetched = schedules::get_schedule(&pool, &created.id).await.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.name, "get-test");
}

#[tokio::test]
async fn get_schedule_not_found() {
    let (pool, _tmp) = setup().await;
    let result = schedules::get_schedule(&pool, "nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn list_schedules_all() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    for i in 0..3 {
        schedules::create_schedule(
            &pool,
            &format!("sched-{i}"),
            "0 * * * *",
            None,
            None,
            "mcp_tool",
            "{}",
            None,
            None,
            None,
            None,
            None,
            &clock,
        )
        .await
        .unwrap();
    }

    let list = schedules::list_schedules(&pool, None, None, None)
        .await
        .unwrap();
    assert_eq!(list.len(), 3);
}

#[tokio::test]
async fn list_schedules_filter_enabled() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "enabled-one",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    schedules::update_schedule(
        &pool,
        &s.id,
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
        &clock,
    )
    .await
    .unwrap();

    let enabled = schedules::list_schedules(&pool, Some(true), None, None)
        .await
        .unwrap();
    assert_eq!(enabled.len(), 0);

    let disabled = schedules::list_schedules(&pool, Some(false), None, None)
        .await
        .unwrap();
    assert_eq!(disabled.len(), 1);
}

#[tokio::test]
async fn update_schedule_fields() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "update-me",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    let updated = schedules::update_schedule(
        &pool,
        &s.id,
        Some("new-name"),
        Some("*/10 * * * *"),
        None,
        None,
        None,
        None,
        None,
        Some(5),
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "new-name");
    assert_eq!(updated.cron_expr, "*/10 * * * *");
    assert_eq!(updated.max_retries, 5);
}

#[tokio::test]
async fn update_schedule_disable_clears_next_run() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "disable-me",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();
    assert!(s.next_run_at.is_some());

    let updated = schedules::update_schedule(
        &pool,
        &s.id,
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
        &clock,
    )
    .await
    .unwrap();

    assert!(!updated.enabled);
    assert!(updated.next_run_at.is_none());
}

#[tokio::test]
async fn delete_schedule_success() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "delete-me",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    schedules::delete_schedule(&pool, &s.id).await.unwrap();

    let result = schedules::get_schedule(&pool, &s.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn delete_schedule_not_found() {
    let (pool, _tmp) = setup().await;
    let result = schedules::delete_schedule(&pool, "nope").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn record_run_and_list() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "run-test",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    let run = schedules::record_run(
        &pool,
        &s.id,
        1_700_000_100,
        1_700_000_102,
        "completed",
        Some(0),
        Some("ok"),
        None,
        1,
    )
    .await
    .unwrap();

    assert_eq!(run.schedule_id, s.id);
    assert_eq!(run.status, "completed");
    assert_eq!(run.duration_ms, Some(2000));

    let runs = schedules::list_runs(&pool, &s.id, None, None)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, run.id);
}

#[tokio::test]
async fn record_run_updates_last_run_at() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "last-run-test",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();
    assert!(s.last_run_at.is_none());

    schedules::record_run(&pool, &s.id, 1_700_000_100, 1_700_000_105, "completed", None, None, None, 1)
        .await
        .unwrap();

    let updated = schedules::get_schedule(&pool, &s.id).await.unwrap();
    assert_eq!(updated.last_run_at, Some(1_700_000_105));
}

#[tokio::test]
async fn record_run_purges_old_runs() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "purge-test",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        Some(3),
        &clock,
    )
    .await
    .unwrap();

    for i in 0..5 {
        schedules::record_run(
            &pool,
            &s.id,
            1_700_000_000 + i * 100,
            1_700_000_000 + i * 100 + 10,
            "completed",
            None,
            None,
            None,
            1,
        )
        .await
        .unwrap();
    }

    let runs = schedules::list_runs(&pool, &s.id, None, None)
        .await
        .unwrap();
    assert_eq!(runs.len(), 3);
}

#[tokio::test]
async fn list_runs_filter_by_status() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "status-filter",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    schedules::record_run(&pool, &s.id, 100, 110, "completed", None, None, None, 1)
        .await
        .unwrap();
    schedules::record_run(&pool, &s.id, 200, 210, "failed", None, None, Some("err"), 1)
        .await
        .unwrap();

    let completed = schedules::list_runs(&pool, &s.id, Some("completed"), None)
        .await
        .unwrap();
    assert_eq!(completed.len(), 1);

    let failed = schedules::list_runs(&pool, &s.id, Some("failed"), None)
        .await
        .unwrap();
    assert_eq!(failed.len(), 1);
}

#[tokio::test]
async fn schedule_health_returns_counts() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    schedules::create_schedule(
        &pool,
        "health-1",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    let health = schedules::schedule_health(&pool).await.unwrap();
    assert_eq!(health["total"], 1);
    assert_eq!(health["active"], 1);
}

#[tokio::test]
async fn trigger_at_expired_gives_no_next_run() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let schedule = schedules::create_schedule(
        &pool,
        "expired-trigger",
        "@once",
        Some(1_699_999_000), // in the past
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        Some(1),
        &clock,
    )
    .await
    .unwrap();

    assert!(schedule.next_run_at.is_none());
}

#[tokio::test]
async fn cron_next_run_advances_with_mock_clock() {
    let clock = MockClock::new(1_700_000_000);

    let expr = CronExpr::parse("0 * * * *").unwrap();
    let next1 = expr.next_run(clock.now_utc()).unwrap();
    assert!(next1 > 1_700_000_000);

    clock.advance(3600);
    let next2 = expr.next_run(clock.now_utc()).unwrap();
    assert!(next2 > next1);
}

#[tokio::test]
async fn delete_schedule_cascades_runs() {
    let (pool, _tmp) = setup().await;
    let clock = MockClock::new(1_700_000_000);

    let s = schedules::create_schedule(
        &pool,
        "cascade-test",
        "0 * * * *",
        None,
        None,
        "mcp_tool",
        "{}",
        None,
        None,
        None,
        None,
        None,
        &clock,
    )
    .await
    .unwrap();

    schedules::record_run(&pool, &s.id, 100, 110, "completed", None, None, None, 1)
        .await
        .unwrap();

    schedules::delete_schedule(&pool, &s.id).await.unwrap();

    let runs = schedules::list_runs(&pool, &s.id, None, None)
        .await
        .unwrap();
    assert_eq!(runs.len(), 0);
}
