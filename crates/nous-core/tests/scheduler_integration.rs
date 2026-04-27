use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;

use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::db::MemoryDb;
use nous_core::schedule_db::ScheduleDb;
use nous_core::scheduler::{ScheduleConfig, Scheduler};
use nous_core::types::{ActionType, RunStatus, Schedule};

fn test_db_path() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "/tmp/nous-sched-test-{}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        seq,
    )
}

struct TestHarness {
    write_channel: WriteChannel,
    read_pool: ReadPool,
    scheduler_notify: Arc<tokio::sync::Notify>,
    _scheduler_handle: tokio::task::JoinHandle<()>,
    _write_handle: tokio::task::JoinHandle<()>,
    db_path: String,
}

impl TestHarness {
    fn new(config: ScheduleConfig) -> Self {
        let db_path = test_db_path();
        let db = MemoryDb::open(&db_path, None, 384).unwrap();
        let (write_channel, write_handle) = WriteChannel::new(db);
        let read_pool = ReadPool::new(&db_path, None, 2).unwrap();

        let (notify, scheduler_handle) =
            Scheduler::spawn(write_channel.clone(), read_pool.clone(), config);

        Self {
            write_channel,
            read_pool,
            scheduler_notify: notify,
            _scheduler_handle: scheduler_handle,
            _write_handle: write_handle,
            db_path,
        }
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.db_path);
    }
}

fn make_mcp_schedule(name: &str, next_run_at: i64) -> Schedule {
    Schedule {
        id: String::new(),
        name: name.to_string(),
        cron_expr: "* * * * *".to_string(),
        timezone: "UTC".to_string(),
        enabled: true,
        action_type: ActionType::McpTool,
        action_payload: r#"{"tool":"memory_stats","args":{}}"#.to_string(),
        desired_outcome: None,
        max_retries: 1,
        timeout_secs: Some(10),
        max_output_bytes: 65536,
        max_runs: 100,
        next_run_at: Some(next_run_at),
        created_at: 0,
        updated_at: 0,
    }
}

#[tokio::test]
async fn scheduler_fires_on_time() {
    let h = TestHarness::new(ScheduleConfig::default());

    let now = chrono::Utc::now().timestamp();
    let schedule = make_mcp_schedule("fires-on-time", now + 1);
    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    let deadline = Instant::now() + Duration::from_secs(15);
    let runs = loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(10))
            })
            .await
            .unwrap();
        if runs
            .iter()
            .any(|r| r.status == RunStatus::Completed || r.status == RunStatus::Failed)
        {
            break runs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for run record"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let last = &runs[0];
    assert!(
        last.status == RunStatus::Completed || last.status == RunStatus::Failed,
        "expected completed or failed, got {:?}",
        last.status
    );
}

#[tokio::test]
async fn scheduler_skips_overlap() {
    let h = TestHarness::new(ScheduleConfig::default());

    let now = chrono::Utc::now().timestamp();
    let mut schedule = make_mcp_schedule("skips-overlap", now + 1);
    schedule.timeout_secs = Some(30);
    schedule.action_payload = r#"{"tool":"__test_delay","args":{"secs":10}}"#.to_string();

    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    // Poll until the first run starts (status Running)
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(50))
            })
            .await
            .unwrap();
        if runs.iter().any(|r| r.status == RunStatus::Running) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for first run to start"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let _ = h
        .write_channel
        .force_next_run_at(id.clone(), chrono::Utc::now().timestamp())
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    h.scheduler_notify.notify_one();

    // Poll until we see a Skipped run
    let deadline = Instant::now() + Duration::from_secs(15);
    let runs = loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(50))
            })
            .await
            .unwrap();
        if runs.iter().any(|r| r.status == RunStatus::Skipped) {
            break runs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for Skipped run"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    assert!(
        runs.len() >= 2,
        "expected at least 2 runs, got {}",
        runs.len()
    );
    let has_skipped = runs.iter().any(|r| r.status == RunStatus::Skipped);
    assert!(has_skipped, "expected at least one run with Skipped status");
}

#[tokio::test]
async fn scheduler_retries_on_failure() {
    let h = TestHarness::new(ScheduleConfig::default());

    let now = chrono::Utc::now().timestamp();
    let mut schedule = make_mcp_schedule("retries-on-failure", now + 1);
    schedule.max_retries = 3;
    schedule.action_payload = r#"{"tool":"nonexistent_tool","args":{}}"#.to_string();

    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    let deadline = Instant::now() + Duration::from_secs(30);
    let runs = loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(50))
            })
            .await
            .unwrap();
        if runs.len() >= 2 && runs.iter().any(|r| r.status == RunStatus::Failed) {
            break runs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for retry attempts (got {} runs)",
            runs.len()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    assert!(
        runs.len() >= 2,
        "expected at least 2 retry attempts, got {}",
        runs.len()
    );
    let has_failure = runs.iter().any(|r| r.status == RunStatus::Failed);
    assert!(has_failure, "expected at least one failed run");
}

#[tokio::test]
async fn scheduler_respects_timeout() {
    let h = TestHarness::new(ScheduleConfig::default());

    let now = chrono::Utc::now().timestamp();
    let mut schedule = make_mcp_schedule("respects-timeout", now + 1);
    schedule.timeout_secs = Some(1);
    schedule.max_retries = 1;
    schedule.action_payload = r#"{"tool":"__test_delay","args":{"secs":10}}"#.to_string();

    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    let deadline = Instant::now() + Duration::from_secs(30);
    let runs = loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(50))
            })
            .await
            .unwrap();
        if runs.iter().any(|r| r.status == RunStatus::Timeout) {
            break runs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for Timeout status"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    assert!(!runs.is_empty(), "expected at least one run");
    let has_timeout = runs.iter().any(|r| r.status == RunStatus::Timeout);
    assert!(has_timeout, "expected at least one run with Timeout status");
}

#[tokio::test]
async fn notify_wakes_scheduler() {
    let h = TestHarness::new(ScheduleConfig::default());

    tokio::time::sleep(Duration::from_millis(500)).await;

    let now = chrono::Utc::now().timestamp();
    let schedule = make_mcp_schedule("notify-wake", now + 1);
    let id = h.write_channel.create_schedule(schedule).await.unwrap();

    h.scheduler_notify.notify_one();

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(10))
            })
            .await
            .unwrap();
        if !runs.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "scheduler should wake and fire after notify"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[tokio::test]
async fn shell_action_gated() {
    let config = ScheduleConfig {
        allow_shell: false,
        ..Default::default()
    };
    let h = TestHarness::new(config);

    let now = chrono::Utc::now().timestamp();
    let schedule = Schedule {
        id: String::new(),
        name: "shell-gated".to_string(),
        cron_expr: "* * * * *".to_string(),
        timezone: "UTC".to_string(),
        enabled: true,
        action_type: ActionType::Shell,
        action_payload: r#"{"command":"echo hello","working_dir":"/tmp"}"#.to_string(),
        desired_outcome: None,
        max_retries: 1,
        timeout_secs: Some(10),
        max_output_bytes: 65536,
        max_runs: 100,
        next_run_at: Some(now + 1),
        created_at: 0,
        updated_at: 0,
    };

    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    let mut runs = Vec::new();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(10))
            })
            .await
            .unwrap();
        if runs.iter().any(|r| r.status == RunStatus::Failed) {
            break;
        }
    }

    assert!(!runs.is_empty(), "expected a run record for gated shell");
    let run = &runs[0];
    assert_eq!(run.status, RunStatus::Failed);
    assert!(
        run.error
            .as_deref()
            .unwrap_or("")
            .contains("shell actions disabled"),
        "expected 'shell actions disabled' error, got: {:?}",
        run.error
    );
}

#[tokio::test]
async fn shell_action_allowed() {
    let config = ScheduleConfig {
        allow_shell: true,
        ..Default::default()
    };
    let h = TestHarness::new(config);

    let now = chrono::Utc::now().timestamp();
    let schedule = Schedule {
        id: String::new(),
        name: "shell-allowed".to_string(),
        cron_expr: "* * * * *".to_string(),
        timezone: "UTC".to_string(),
        enabled: true,
        action_type: ActionType::Shell,
        action_payload: r#"{"command":"echo hello-from-shell"}"#.to_string(),
        desired_outcome: None,
        max_retries: 1,
        timeout_secs: Some(10),
        max_output_bytes: 65536,
        max_runs: 100,
        next_run_at: Some(now + 1),
        created_at: 0,
        updated_at: 0,
    };

    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    let deadline = Instant::now() + Duration::from_secs(15);
    let runs = loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(10))
            })
            .await
            .unwrap();
        if runs.iter().any(|r| r.status == RunStatus::Completed) {
            break runs;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for shell run to complete"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let run = &runs[0];
    assert_eq!(run.status, RunStatus::Completed);
    assert!(
        run.output
            .as_deref()
            .unwrap_or("")
            .contains("hello-from-shell"),
        "expected shell output, got: {:?}",
        run.output
    );
}

#[tokio::test]
async fn http_action_dispatches() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut buf = [0u8; 1024];
            let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n{\"ok\":\"true\"}";
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });

    let config = ScheduleConfig {
        allow_http: true,
        ..Default::default()
    };
    let h = TestHarness::new(config);

    let now = chrono::Utc::now().timestamp();
    let payload = format!(
        r#"{{"method":"GET","url":"http://{}","headers":{{}}}}"#,
        addr
    );
    let schedule = Schedule {
        id: String::new(),
        name: "http-dispatch".to_string(),
        cron_expr: "* * * * *".to_string(),
        timezone: "UTC".to_string(),
        enabled: true,
        action_type: ActionType::Http,
        action_payload: payload,
        desired_outcome: None,
        max_retries: 1,
        timeout_secs: Some(10),
        max_output_bytes: 65536,
        max_runs: 100,
        next_run_at: Some(now + 1),
        created_at: 0,
        updated_at: 0,
    };

    let id = h.write_channel.create_schedule(schedule).await.unwrap();
    h.scheduler_notify.notify_one();

    let deadline = Instant::now() + Duration::from_secs(15);
    let runs = loop {
        let runs = h
            .read_pool
            .with_conn({
                let id = id.clone();
                move |conn| ScheduleDb::get_runs(conn, &id, None, Some(10))
            })
            .await
            .unwrap();
        if runs
            .iter()
            .any(|r| r.status == RunStatus::Completed || r.status == RunStatus::Failed)
        {
            break runs;
        }
        assert!(Instant::now() < deadline, "timed out waiting for http run");
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let run = &runs[0];
    assert_eq!(run.status, RunStatus::Completed);
    assert!(
        run.output.as_deref().unwrap_or("").contains("ok"),
        "expected HTTP response in output, got: {:?}",
        run.output
    );
}
