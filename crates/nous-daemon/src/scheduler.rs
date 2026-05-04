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
use nous_core::schedules::{
    advance_next_run_at, list_due_schedules, mark_stale_runs_failed, record_run,
    update_schedule, Clock, RecordRunParams, Schedule, UpdateScheduleParams,
};

use crate::routes::mcp;
use crate::state::AppState;

pub struct SchedulerConfig {
    pub max_concurrent: usize,
    pub allow_shell: bool,
    pub default_timeout_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            allow_shell: true,
            default_timeout_secs: 300,
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

async fn run_loop(
    state: AppState,
    config: SchedulerConfig,
    clock: Arc<dyn Clock>,
    shutdown: CancellationToken,
) {
    if let Err(e) = mark_stale_runs_failed(&state.pool).await {
        tracing::error!("failed to mark stale runs: {e}");
    }

    let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
    let active: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let config = Arc::new(config);

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        let sleep_dur = next_wake_duration(&state.pool, &*clock).await;

        tokio::select! {
            _ = tokio::time::sleep(sleep_dur) => {}
            _ = state.schedule_notify.notified() => {}
            _ = shutdown.cancelled() => { break; }
        }

        if shutdown.is_cancelled() {
            break;
        }

        let now = clock.now_utc();
        let due = match list_due_schedules(&state.pool, now).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("failed to list due schedules: {e}");
                continue;
            }
        };

        for schedule in due {
            let mut guard = active.lock().await;
            if guard.contains(&schedule.id) {
                continue;
            }
            guard.insert(schedule.id.clone());
            drop(guard);

            let permit = match semaphore.clone().acquire_owned().await {
                Ok(p) => p,
                Err(_) => break,
            };

            let state = state.clone();
            let clock = clock.clone();
            let config = config.clone();
            let active = active.clone();

            let schedule_id = schedule.id.clone();
            let schedule_name = schedule.name.clone();
            tokio::spawn(async move {
                let result = std::panic::AssertUnwindSafe(
                    execute_schedule(&state, &schedule, &config, &clock),
                );
                let outcome = futures::FutureExt::catch_unwind(result).await;
                if let Err(panic_info) = outcome {
                    let msg = panic_info
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic_info.downcast_ref::<String>().map(|s| s.as_str()))
                        .unwrap_or("unknown panic");
                    tracing::error!(
                        schedule_id = %schedule.id,
                        schedule_name = %schedule.name,
                        panic = %msg,
                        "schedule execution task panicked"
                    );
                }
                active.lock().await.remove(&schedule.id);
                drop(permit);
            });
            tracing::debug!(schedule_id = %schedule_id, schedule_name = %schedule_name, "spawned schedule execution task");
        }
    }

    tracing::info!("scheduler shutting down, waiting for in-flight tasks");
    let _ = semaphore.acquire_many(config.max_concurrent as u32).await;
    tracing::info!("scheduler stopped");
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

    match next_iso {
        Some(iso) => {
            let ts = match nous_core::schedules::iso_to_ts(&iso) {
                Ok(ts) => ts,
                Err(e) => {
                    tracing::error!("failed to parse next_run_at ISO timestamp: {e}");
                    return Duration::from_secs(60);
                }
            };
            let now = clock.now_utc();
            let diff = ts - now;
            if diff <= 0 {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(diff as u64)
            }
        }
        None => Duration::from_secs(60),
    }
}

async fn execute_schedule(
    state: &AppState,
    schedule: &Schedule,
    config: &SchedulerConfig,
    clock: &Arc<dyn Clock>,
) {
    let started_at = clock.now_utc();
    let timeout_secs = schedule
        .timeout_secs
        .unwrap_or(config.default_timeout_secs as i32) as u64;

    let max_retries = schedule.max_retries.max(0) as u32;
    let mut last_result: DispatchResult = DispatchResult {
        status: "failed".to_string(),
        output: None,
        error: Some("no attempts made".to_string()),
        exit_code: None,
    };

    for attempt in 0..=max_retries {
        let result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            dispatch_action(state, schedule, config),
        )
        .await;

        last_result = match result {
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
        };

        if last_result.status == "completed" || last_result.status == "timeout" {
            break;
        }

        if !should_retry(&schedule.action_type) || attempt == max_retries {
            break;
        }

        let backoff = Duration::from_secs(1u64 << attempt);
        tokio::time::sleep(backoff).await;
    }

    let finished_at = clock.now_utc();
    if let Err(e) = record_run(RecordRunParams {
        db: &state.pool,
        schedule_id: &schedule.id,
        started_at,
        finished_at,
        status: &last_result.status,
        exit_code: last_result.exit_code,
        output: last_result.output.as_deref(),
        error: last_result.error.as_deref(),
        attempt: 1,
    })
    .await
    {
        tracing::error!("failed to record run for {}: {e}", schedule.id);
    }

    match advance_next_run_at(&state.pool, &schedule.id, &**clock).await {
        Ok(None) => {
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
                tracing::error!("failed to disable @once schedule {}: {e}", schedule.id);
            }
        }
        Ok(Some(_)) => {}
        Err(e) => {
            tracing::error!("failed to advance next_run_at for {}: {e}", schedule.id);
        }
    }
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

async fn dispatch_action(
    state: &AppState,
    schedule: &Schedule,
    config: &SchedulerConfig,
) -> Result<String, NousError> {
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
        .invoke(
            state,
            &parsed.agent_id,
            &parsed.prompt,
            parsed.timeout_secs,
            None,
            false, // synchronous for schedule actions
        )
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

fn evaluate_outcome(output: String, desired_outcome: &Option<String>) -> DispatchResult {
    let Some(pattern) = desired_outcome else {
        return DispatchResult {
            status: "completed".to_string(),
            output: Some(output),
            error: None,
            exit_code: None,
        };
    };

    let matches = if pattern.starts_with('/') && pattern.ends_with('/') && pattern.len() > 2 {
        let re_pattern = &pattern[1..pattern.len() - 1];
        match regex::Regex::new(re_pattern) {
            Ok(re) => re.is_match(&output),
            Err(_) => output.contains(pattern.as_str()),
        }
    } else {
        output.contains(pattern.as_str())
    };

    if matches {
        DispatchResult {
            status: "completed".to_string(),
            output: Some(output),
            error: None,
            exit_code: None,
        }
    } else {
        DispatchResult {
            status: "failed".to_string(),
            output: Some(output),
            error: Some("desired outcome mismatch".to_string()),
            exit_code: None,
        }
    }
}
