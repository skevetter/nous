use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use nous_core::db::DatabaseConnection;
use sea_orm::{ConnectionTrait, Statement};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use nous_core::error::NousError;
use nous_core::net::validate_url;
use nous_core::schedules::{
    advance_next_run_at, list_due_schedules, mark_stale_runs_failed, record_run, update_schedule,
    Clock, RecordRunParams, Schedule, UpdateScheduleParams,
};

use crate::routes::mcp;
use crate::state::AppState;

pub struct SchedulerConfig {
    pub max_concurrent: usize,
    pub allow_shell: bool,
    pub default_timeout_secs: u64,
    /// When true, an API key is configured and the daemon authenticates callers.
    /// Shell and HTTP actions are only permitted when this is true — without auth,
    /// those actions would allow unauthenticated remote code execution.
    pub auth_configured: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            allow_shell: true,
            default_timeout_secs: 300,
            auth_configured: false,
        }
    }
}

pub struct Scheduler;

impl Scheduler {
    pub fn spawn(
        state: AppState,
        config: SchedulerConfig,
        clock: Arc<dyn Clock>,
        shutdown: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(run_loop(state, config, clock, shutdown))
    }
}

async fn cleanup_stale_runs(pool: &DatabaseConnection) {
    if let Err(e) = mark_stale_runs_failed(pool).await {
        tracing::error!(error = %e, "failed to mark stale runs");
    }
}

/// Waits until all in-flight schedule tasks have released their semaphore permits.
async fn drain_semaphore(semaphore: &Arc<Semaphore>, max_concurrent: usize) {
    let _ = semaphore.acquire_many(max_concurrent as u32).await;
}

fn log_scheduler_shutdown_start() {
    tracing::info!("scheduler shutting down, waiting for in-flight tasks");
}

fn log_scheduler_shutdown_complete() {
    tracing::info!("scheduler stopped");
}

async fn run_loop(
    state: AppState,
    config: SchedulerConfig,
    clock: Arc<dyn Clock>,
    shutdown: CancellationToken,
) {
    cleanup_stale_runs(&state.pool).await;

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
    let active: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let config = Arc::new(config);

    run_scheduler_loop(SchedulerLoopParams {
        state: &state,
        config: &config,
        clock: &clock,
        shutdown: &shutdown,
        semaphore: &semaphore,
        active: &active,
    })
    .await;

    log_scheduler_shutdown_start();
    drain_semaphore(&semaphore, config.max_concurrent).await;
    log_scheduler_shutdown_complete();
}

struct SchedulerLoopParams<'a> {
    state: &'a AppState,
    config: &'a Arc<SchedulerConfig>,
    clock: &'a Arc<dyn Clock>,
    shutdown: &'a CancellationToken,
    semaphore: &'a Arc<Semaphore>,
    active: &'a Arc<Mutex<HashSet<String>>>,
}

async fn run_scheduler_loop(params: SchedulerLoopParams<'_>) {
    let SchedulerLoopParams { state, config, clock, shutdown, semaphore, active } = params;
    loop {
        if shutdown.is_cancelled() {
            break;
        }

        wait_for_next_tick(state, clock, shutdown).await;

        if shutdown.is_cancelled() {
            break;
        }

        dispatch_due_schedules(DispatchDueSchedulesParams {
            state,
            config,
            clock,
            semaphore,
            active,
        })
        .await;
    }
}

async fn wait_for_next_tick(
    state: &AppState,
    clock: &Arc<dyn Clock>,
    shutdown: &CancellationToken,
) {
    let sleep_dur = next_wake_duration(&state.pool, &**clock).await;
    tokio::select! {
        _ = tokio::time::sleep(sleep_dur) => {}
        _ = state.schedule_notify.notified() => {}
        _ = shutdown.cancelled() => {}
    }
}

struct DispatchDueSchedulesParams<'a> {
    state: &'a AppState,
    config: &'a Arc<SchedulerConfig>,
    clock: &'a Arc<dyn Clock>,
    semaphore: &'a Arc<Semaphore>,
    active: &'a Arc<Mutex<HashSet<String>>>,
}

async fn load_due_schedules(
    pool: &DatabaseConnection,
    now_utc: i64,
) -> Option<Vec<Schedule>> {
    match list_due_schedules(pool, now_utc).await {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!(error = %e, "failed to list due schedules");
            None
        }
    }
}

async fn try_acquire_permit(
    semaphore: &Arc<Semaphore>,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    semaphore.clone().acquire_owned().await.ok()
}

async fn dispatch_due_schedules(params: DispatchDueSchedulesParams<'_>) {
    let DispatchDueSchedulesParams { state, config, clock, semaphore, active } = params;
    let Some(due) = load_due_schedules(&state.pool, clock.now_utc()).await else {
        return;
    };

    for schedule in due {
        if !try_claim_schedule(active, &schedule.id).await {
            continue;
        }
        let Some(permit) = try_acquire_permit(semaphore).await else {
            break;
        };
        spawn_schedule_task(SpawnScheduleTaskParams {
            state: state.clone(),
            clock: clock.clone(),
            config: config.clone(),
            active: active.clone(),
            schedule,
            permit,
        });
    }
}

/// Returns true if the schedule was successfully claimed (not already running).
async fn try_claim_schedule(
    active: &Arc<Mutex<HashSet<String>>>,
    schedule_id: &str,
) -> bool {
    let mut guard = active.lock().await;
    if guard.contains(schedule_id) {
        return false;
    }
    guard.insert(schedule_id.to_string());
    true
}

struct SpawnScheduleTaskParams {
    state: AppState,
    clock: Arc<dyn Clock>,
    config: Arc<SchedulerConfig>,
    active: Arc<Mutex<HashSet<String>>>,
    schedule: Schedule,
    permit: tokio::sync::OwnedSemaphorePermit,
}

fn spawn_schedule_task(params: SpawnScheduleTaskParams) {
    let schedule_id = params.schedule.id.clone();
    let schedule_name = params.schedule.name.clone();
    let SpawnScheduleTaskParams { state, clock, config, active, schedule, permit } = params;
    tokio::spawn(async move {
        let result = std::panic::AssertUnwindSafe(execute_schedule(
            &state, &schedule, &config, &clock,
        ));
        let outcome = futures::FutureExt::catch_unwind(result).await;
        if let Err(panic_info) = outcome {
            log_schedule_panic(&schedule.id, &schedule.name, &panic_info);
        }
        active.lock().await.remove(&schedule.id);
        drop(permit);
    });
    tracing::debug!(schedule_id = %schedule_id, schedule_name = %schedule_name, "spawned schedule execution task");
}

fn log_schedule_panic(
    schedule_id: &str,
    schedule_name: &str,
    panic_info: &Box<dyn std::any::Any + Send>,
) {
    let msg = panic_info
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic_info.downcast_ref::<String>().map(|s| s.as_str()))
        .unwrap_or("unknown panic");
    tracing::error!(
        schedule_id = %schedule_id,
        schedule_name = %schedule_name,
        panic = %msg,
        "schedule execution task panicked"
    );
}

fn duration_until_timestamp(iso: &str, clock: &dyn Clock) -> Duration {
    match nous_core::schedules::iso_to_ts(iso) {
        Ok(ts) => {
            let diff = ts - clock.now_utc();
            if diff <= 0 {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(diff as u64)
            }
        }
        Err(e) => {
            tracing::error!("failed to parse next_run_at ISO timestamp: {e}");
            Duration::from_secs(60)
        }
    }
}

async fn next_wake_duration(pool: &DatabaseConnection, clock: &dyn Clock) -> Duration {
    let result = pool
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT MIN(next_run_at) FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL",
        ))
        .await;

    let next_iso: Option<String> = result.ok().flatten().and_then(|row| {
        use sea_orm::TryGetable;
        String::try_get_by(&row, 0usize).ok()
    });

    next_iso
        .map(|iso| duration_until_timestamp(&iso, clock))
        .unwrap_or_else(|| Duration::from_secs(60))
}

/// Convert a timed dispatch result into a `DispatchResult`.
fn interpret_timed_result(
    result: Result<Result<String, NousError>, tokio::time::error::Elapsed>,
    schedule: &Schedule,
) -> DispatchResult {
    match result {
        Ok(Ok(output)) => {
            let truncated = truncate_output(&output, schedule.max_output_bytes as usize);
            evaluate_outcome(truncated, &schedule.desired_outcome)
        }
        Ok(Err(e)) => DispatchResult {
            status: "failed".to_string(),
            output: None,
            error: Some(e.to_string()),
            exit_code: None,
        },
        Err(_) => DispatchResult {
            status: "timeout".to_string(),
            output: None,
            error: Some("execution timed out".to_string()),
            exit_code: None,
        },
    }
}

struct RetryParams<'a> {
    state: &'a AppState,
    schedule: &'a Schedule,
    config: &'a SchedulerConfig,
    timeout_secs: u64,
    max_retries: u32,
}

/// Run the schedule action with retries, returning the final result.
async fn run_with_retries(params: RetryParams<'_>) -> DispatchResult {
    let RetryParams { state, schedule, config, timeout_secs, max_retries } = params;
    let mut last_result = DispatchResult {
        status: "failed".to_string(),
        output: None,
        error: Some("no attempts made".to_string()),
        exit_code: None,
    };

    for attempt in 0..=max_retries {
        let timed = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            dispatch_action(state, schedule, config),
        )
        .await;

        last_result = interpret_timed_result(timed, schedule);

        if last_result.status == "completed" || last_result.status == "timeout" {
            break;
        }
        if !should_retry(&schedule.action_type) || attempt == max_retries {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1u64 << attempt)).await;
    }

    last_result
}

/// Advance the next run time or disable the schedule if it was a one-shot.
async fn advance_or_disable_schedule(
    state: &AppState,
    schedule: &Schedule,
    clock: &Arc<dyn Clock>,
) {
    let advance_result = advance_next_run_at(&state.pool, &schedule.id, &**clock).await;
    handle_advance_result(advance_result, state, schedule, clock).await;
}

async fn handle_advance_result(
    result: Result<Option<String>, NousError>,
    state: &AppState,
    schedule: &Schedule,
    clock: &Arc<dyn Clock>,
) {
    match result {
        Ok(None) => disable_once_schedule(state, schedule, clock).await,
        Ok(Some(_)) => {}
        Err(e) => {
            tracing::error!(schedule_id = %schedule.id, error = %e, "failed to advance next_run_at");
        }
    }
}

async fn disable_once_schedule(
    state: &AppState,
    schedule: &Schedule,
    clock: &Arc<dyn Clock>,
) {
    if let Err(e) = update_schedule(UpdateScheduleParams {
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
        clock: &**clock,
    })
    .await
    {
        tracing::error!(schedule_id = %schedule.id, error = %e, "failed to disable @once schedule");
    }
}

struct RecordScheduleRunParams<'a> {
    state: &'a AppState,
    schedule: &'a Schedule,
    started_at: i64,
    finished_at: i64,
    result: &'a DispatchResult,
}

async fn record_schedule_run(params: RecordScheduleRunParams<'_>) {
    let RecordScheduleRunParams { state, schedule, started_at, finished_at, result } = params;
    if let Err(e) = record_run(RecordRunParams {
        db: &state.pool,
        schedule_id: &schedule.id,
        started_at,
        finished_at,
        status: &result.status,
        exit_code: result.exit_code,
        output: result.output.as_deref(),
        error: result.error.as_deref(),
        attempt: 1,
    })
    .await
    {
        tracing::error!(schedule_id = %schedule.id, error = %e, "failed to record run");
    }
}

async fn execute_schedule(
    state: &AppState,
    schedule: &Schedule,
    config: &SchedulerConfig,
    clock: &Arc<dyn Clock>,
) {
    let started_at = clock.now_utc();
    // Schedule timeout: use schedule value if set, else fallback to config default.
    // Both are non-negative; i32.cast_unsigned() is safe for positive values.
    let timeout_secs = schedule
        .timeout_secs
        .map_or(config.default_timeout_secs, |s| u64::from(s.cast_unsigned()));
    let max_retries = schedule.max_retries.max(0).cast_unsigned();

    let last_result = run_with_retries(RetryParams {
        state,
        schedule,
        config,
        timeout_secs,
        max_retries,
    })
    .await;

    let finished_at = clock.now_utc();
    record_schedule_run(RecordScheduleRunParams {
        state,
        schedule,
        started_at,
        finished_at,
        result: &last_result,
    })
    .await;

    advance_or_disable_schedule(state, schedule, clock).await;
}

fn should_retry(action_type: &str) -> bool {
    matches!(action_type, "shell" | "http" | "agent_invoke")
}

struct DispatchResult {
    status: String,
    output: Option<String>,
    error: Option<String>,
    exit_code: Option<i32>,
}

fn require_auth_for_action(action_type: &str, config: &SchedulerConfig) -> Result<(), NousError> {
    if matches!(action_type, "shell" | "http") && !config.auth_configured {
        return Err(NousError::Validation(format!(
            "{action_type} actions require API key authentication to be configured"
        )));
    }
    Ok(())
}

async fn dispatch_action(
    state: &AppState,
    schedule: &Schedule,
    config: &SchedulerConfig,
) -> Result<String, NousError> {
    require_auth_for_action(&schedule.action_type, config)?;

    match schedule.action_type.as_str() {
        "mcp_tool" => dispatch_mcp_tool(state, &schedule.action_payload).await,
        "shell" => dispatch_shell(&schedule.action_payload, config).await,
        "http" => dispatch_http(&schedule.action_payload).await,
        "agent_invoke" => dispatch_agent_invoke(state, &schedule.action_payload).await,
        other => Err(NousError::Validation(format!(
            "unknown action_type: {other}"
        ))),
    }
}

#[derive(Deserialize)]
struct McpToolPayload {
    tool: String,
    args: Value,
}

async fn dispatch_mcp_tool(state: &AppState, payload: &str) -> Result<String, NousError> {
    let parsed: McpToolPayload = serde_json::from_str(payload)
        .map_err(|e| NousError::Validation(format!("invalid mcp_tool payload: {e}")))?;

    let result = mcp::dispatch(state, &parsed.tool, &parsed.args).await?;
    Ok(result.to_string())
}

#[derive(Deserialize)]
struct ShellPayload {
    command: String,
}

async fn dispatch_shell(payload: &str, config: &SchedulerConfig) -> Result<String, NousError> {
    if !config.allow_shell {
        return Err(NousError::Validation("shell actions disabled".to_string()));
    }

    let parsed: ShellPayload = serde_json::from_str(payload)
        .map_err(|e| NousError::Validation(format!("invalid shell payload: {e}")))?;

    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&parsed.command)
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| NousError::Internal(format!("failed to spawn shell: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if stderr.is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout}\n{stderr}")
    };

    if output.status.success() {
        Ok(combined)
    } else {
        Err(NousError::Internal(format!(
            "shell exited with {}: {combined}",
            output.status.code().unwrap_or(-1)
        )))
    }
}

#[derive(Deserialize)]
struct HttpPayload {
    method: String,
    url: String,
    #[serde(default)]
    headers: std::collections::HashMap<String, String>,
    body: Option<String>,
}

async fn dispatch_http(payload: &str) -> Result<String, NousError> {
    let parsed: HttpPayload = serde_json::from_str(payload)
        .map_err(|e| NousError::Validation(format!("invalid http payload: {e}")))?;

    validate_url(&parsed.url)?;

    let client = reqwest::Client::new();
    let method: reqwest::Method = parsed
        .method
        .parse()
        .map_err(|e| NousError::Validation(format!("invalid HTTP method: {e}")))?;

    let mut request = client.request(method, &parsed.url);

    for (k, v) in &parsed.headers {
        request = request.header(k.as_str(), v.as_str());
    }

    if let Some(body) = parsed.body {
        request = request.body(body);
    }

    let response = request
        .send()
        .await
        .map_err(|e| NousError::Internal(format!("http request failed: {e}")))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| NousError::Internal(format!("failed to read response body: {e}")))?;

    if status.is_success() {
        Ok(text)
    } else {
        Err(NousError::Internal(format!("http {status}: {text}")))
    }
}

#[derive(Deserialize)]
struct AgentInvokePayload {
    agent_id: String,
    prompt: String,
    timeout_secs: Option<i64>,
}

async fn dispatch_agent_invoke(state: &AppState, payload: &str) -> Result<String, NousError> {
    let parsed: AgentInvokePayload = serde_json::from_str(payload)
        .map_err(|e| NousError::Validation(format!("invalid agent_invoke payload: {e}")))?;

    let invocation = state
        .process_registry
        .invoke(crate::process_manager::InvokeParams {
            state,
            agent_id: &parsed.agent_id,
            prompt: &parsed.prompt,
            timeout_secs: parsed.timeout_secs,
            metadata: None,
            is_async: false, // synchronous for schedule actions
        })
        .await?;

    Ok(serde_json::to_string(&invocation).unwrap_or_default())
}

fn truncate_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        output.to_string()
    } else {
        let mut end = max_bytes;
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        output[..end].to_string()
    }
}

fn output_matches_pattern(output: &str, pattern: &str) -> bool {
    if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 2 {
        let re_pattern = &pattern[1..pattern.len() - 1];
        match regex::Regex::new(re_pattern) {
            Ok(re) => re.is_match(output),
            Err(_) => output.contains(pattern),
        }
    } else {
        output.contains(pattern)
    }
}

fn dispatch_result_completed(output: String) -> DispatchResult {
    DispatchResult {
        status: "completed".to_string(),
        output: Some(output),
        error: None,
        exit_code: None,
    }
}

fn evaluate_outcome(output: String, desired_outcome: &Option<String>) -> DispatchResult {
    let Some(pattern) = desired_outcome else {
        return dispatch_result_completed(output);
    };

    if output_matches_pattern(&output, pattern) {
        dispatch_result_completed(output)
    } else {
        DispatchResult {
            status: "failed".to_string(),
            output: Some(output),
            error: Some("desired outcome mismatch".to_string()),
            exit_code: None,
        }
    }
}
