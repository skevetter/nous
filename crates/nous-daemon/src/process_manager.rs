use std::collections::{HashMap, HashSet};
#[cfg(feature = "sandbox")]
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use nous_core::db::DatabaseConnection;

use nous_core::agents::processes::{self, Invocation, Process, UpdateProcessStatusRequest, UpdateInvocationRequest};
#[cfg(feature = "sandbox")]
use nous_core::agents::sandbox::SandboxConfig;
use nous_core::error::NousError;

use crate::state::AppState;

/// Convert a potentially-negative i64 timeout to a Duration, falling back to
/// `default` seconds when the value is negative or otherwise cannot be
/// represented as u64.
fn safe_timeout(secs: i64, default: u64) -> Duration {
    Duration::from_secs(secs.try_into().unwrap_or(default))
}

pub struct ProcessHandle {
    pub process_id: String,
    pub agent_id: String,
    /// `None` for sandbox processes — their lifecycle is managed by `SandboxManager`, not a child process.
    pub child: Option<tokio::process::Child>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub cancel: CancellationToken,
}

pub struct ProcessRegistry {
    handles: Mutex<HashMap<String, ProcessHandle>>, // keyed by agent_id
    /// Tracks `agent_ids` currently in the process of being spawned.
    /// Prevents TOCTOU races where concurrent `spawn()` calls for the same
    /// `agent_id` could both pass the `handles` check before either inserts.
    spawning: Mutex<HashSet<String>>,
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Guard that removes an `agent_id` from the `spawning` set on drop,
/// ensuring the reservation is released even if the spawn fails.
struct SpawnReservation<'a> {
    registry: &'a ProcessRegistry,
    agent_id: Option<String>,
}

impl SpawnReservation<'_> {
    /// Consume the reservation without removing from the spawning set
    /// (caller has successfully inserted into handles).
    fn defuse(mut self) {
        self.agent_id = None;
    }
}

impl Drop for SpawnReservation<'_> {
    fn drop(&mut self) {
        if let Some(ref agent_id) = self.agent_id {
            // Use try_lock to remove from spawning set synchronously.
            // If the lock is contended, spawn a task to clean up.
            if let Ok(mut spawning) = self.registry.spawning.try_lock() {
                spawning.remove(agent_id);
            }
            // If try_lock fails we are in an async drop path; the entry
            // will be cleaned up on the next spawn attempt for this agent_id
            // since we'll hold the handles lock at that point.
            // In practice, try_lock will succeed because contention is minimal.
        }
    }
}

pub struct SpawnParams<'a> {
    pub state: &'a AppState,
    pub agent_id: &'a str,
    pub command: &'a str,
    pub process_type: &'a str,
    pub working_dir: Option<&'a str>,
    pub env: Option<serde_json::Value>,
    pub timeout_secs: Option<i64>,
}

pub struct MonitorProcessParams {
    pub state: AppState,
    pub process_id: String,
    pub agent_id: String,
    pub cancel: CancellationToken,
    pub timeout_secs: Option<i64>,
}

struct SpawnMonitorTaskParams {
    state: AppState,
    process_id: String,
    agent_id: String,
    cancel: CancellationToken,
    timeout_secs: Option<i64>,
}

struct WaitForExitParams<'a> {
    state: &'a AppState,
    agent_id: &'a str,
    process_id: &'a str,
    cancel: CancellationToken,
    timeout_secs: Option<i64>,
}

pub struct StopParams<'a> {
    pub state: &'a AppState,
    pub agent_id: &'a str,
    pub force: bool,
    pub grace_secs: u64,
}

pub struct RestartParams<'a> {
    pub state: &'a AppState,
    pub agent_id: &'a str,
    pub command: Option<&'a str>,
    pub working_dir: Option<&'a str>,
}

pub struct InvokeParams<'a> {
    pub state: &'a AppState,
    pub agent_id: &'a str,
    pub prompt: &'a str,
    pub timeout_secs: Option<i64>,
    pub metadata: Option<serde_json::Value>,
    pub is_async: bool,
}

struct InvokeShellParams<'a> {
    state: &'a AppState,
    invocation: &'a Invocation,
    prompt: &'a str,
    timeout_secs: Option<i64>,
    is_async: bool,
}

struct InvokeClaudeParams<'a> {
    state: &'a AppState,
    invocation: &'a Invocation,
    prompt: &'a str,
    timeout_secs: Option<i64>,
    metadata: &'a Option<serde_json::Value>,
    is_async: bool,
}

struct DispatchInvocationParams<'a> {
    state: &'a AppState,
    invocation: &'a Invocation,
    prompt: &'a str,
    timeout_secs: Option<i64>,
    metadata: &'a Option<serde_json::Value>,
    is_async: bool,
    process_type: Option<&'a str>,
}

enum LlmOutcome {
    Completed { output: String, duration_ms: i64 },
    Failed { error_detail: String, duration_ms: i64 },
    TimedOut { duration_ms: i64 },
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            spawning: Mutex::new(HashSet::new()),
        }
    }

    pub async fn spawn(&self, params: SpawnParams<'_>) -> Result<Process, NousError> {
        let SpawnParams {
            state,
            agent_id,
            command,
            process_type,
            working_dir,
            env,
            timeout_secs,
        } = params;

        let reservation = self.reserve_spawn_slot(agent_id).await?;

        #[cfg(feature = "sandbox")]
        if process_type == "sandbox" {
            return self.spawn_sandbox(state, agent_id, timeout_secs, reservation).await;
        }

        let env_json = env
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()));

        let process = processes::create_process(processes::CreateProcessParams {
            db: &state.pool,
            agent_id,
            process_type,
            command,
            working_dir,
            env_json: env_json.as_deref(),
            timeout_secs,
        })
        .await?;

        let process = processes::update_process_status(
            &state.pool,
            UpdateProcessStatusRequest {
                process_id: &process.id,
                status: "starting",
                exit_code: None,
                output: None,
                pid: None,
            },
        )
        .await?;

        let mut cmd = Self::build_child_command(command, working_dir, env.as_ref());
        let child = cmd
            .spawn()
            .map_err(|e| NousError::Internal(format!("failed to spawn process: {e}")))?;

        let pid = child.id().map(i64::from);

        let process = processes::update_process_status(
            &state.pool,
            UpdateProcessStatusRequest {
                process_id: &process.id,
                status: "running",
                exit_code: None,
                output: None,
                pid,
            },
        )
        .await?;

        let cancel = CancellationToken::new();
        let handle = ProcessHandle {
            process_id: process.id.clone(),
            agent_id: agent_id.to_string(),
            child: Some(child),
            started_at: chrono::Utc::now(),
            cancel: cancel.clone(),
        };

        let process_id = process.id.clone();
        let agent_id_owned = agent_id.to_string();

        {
            let mut handles = self.handles.lock().await;
            handles.insert(agent_id.to_string(), handle);
            self.spawning.lock().await.remove(agent_id);
            reservation.defuse();
        }

        Self::spawn_monitor_task(SpawnMonitorTaskParams {
            state: state.clone(),
            process_id,
            agent_id: agent_id_owned,
            cancel,
            timeout_secs,
        });

        Ok(process)
    }

    async fn reserve_spawn_slot<'a>(
        &'a self,
        agent_id: &str,
    ) -> Result<SpawnReservation<'a>, NousError> {
        // Reserve the agent_id slot atomically: check both handles and spawning set
        // while holding both locks to prevent TOCTOU races.
        let handles = self.handles.lock().await;
        if handles.contains_key(agent_id) {
            return Err(NousError::Conflict(format!(
                "agent '{agent_id}' already has a running process"
            )));
        }
        let mut spawning = self.spawning.lock().await;
        if !spawning.insert(agent_id.to_string()) {
            return Err(NousError::Conflict(format!(
                "agent '{agent_id}' already has a running process"
            )));
        }
        drop(handles);
        Ok(SpawnReservation {
            registry: self,
            agent_id: Some(agent_id.to_string()),
        })
    }

    fn build_child_command(
        command: &str,
        working_dir: Option<&str>,
        env: Option<&serde_json::Value>,
    ) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.kill_on_drop(true);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(wd) = working_dir {
            cmd.current_dir(wd);
        }

        if let Some(env_val) = env {
            if let Some(obj) = env_val.as_object() {
                for (k, v) in obj {
                    if let Some(val) = v.as_str() {
                        cmd.env(k, val);
                    }
                }
            }
        }
        cmd
    }

    fn spawn_monitor_task(params: SpawnMonitorTaskParams) {
        let SpawnMonitorTaskParams {
            state,
            process_id,
            agent_id,
            cancel,
            timeout_secs,
        } = params;
        let monitor_agent_id = agent_id.clone();
        tokio::spawn(async move {
            let result =
                std::panic::AssertUnwindSafe(Self::monitor_process(MonitorProcessParams {
                    state,
                    process_id,
                    agent_id,
                    cancel,
                    timeout_secs,
                }));
            if let Err(panic_info) = futures::FutureExt::catch_unwind(result).await {
                let msg = panic_info
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| panic_info.downcast_ref::<String>().map(std::string::String::as_str))
                    .unwrap_or("unknown panic");
                tracing::error!(
                    agent_id = %monitor_agent_id,
                    panic = %msg,
                    "process monitor task panicked"
                );
            }
        });
    }

    #[cfg(feature = "sandbox")]
    async fn spawn_sandbox(
        &self,
        state: &AppState,
        agent_id: &str,
        timeout_secs: Option<i64>,
        reservation: SpawnReservation<'_>,
    ) -> Result<Process, NousError> {
        let sandbox_mgr = state.sandbox_manager().ok_or_else(|| {
            NousError::Config("sandbox feature enabled but SandboxManager not initialized".into())
        })?;

        let agent = nous_core::agents::get_agent_by_id(&state.pool, agent_id).await?;
        let config = Self::build_sandbox_config(&agent)?;

        let volumes_json = config
            .volumes
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string()));

        let process = processes::create_sandbox_process(processes::CreateSandboxProcessParams {
            db: &state.pool,
            agent_id,
            sandbox_image: &config.image,
            sandbox_cpus: config.cpus,
            sandbox_memory_mib: config.memory_mib,
            sandbox_network_policy: config.network_policy.as_deref(),
            sandbox_volumes_json: volumes_json.as_deref(),
            sandbox_name: None,
            timeout_secs,
        })
        .await?;

        let sandbox_name = {
            let mut mgr = sandbox_mgr.lock().await;
            mgr.create(&config, agent_id).await?
        };

        // Update sandbox_name and status to running
        use sea_orm::{ConnectionTrait, Statement};
        state
            .pool
            .execute(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "UPDATE agent_processes SET sandbox_name = ?, status = 'running', \
             started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
                [sandbox_name.clone().into(), process.id.clone().into()],
            ))
            .await?;

        let cancel = CancellationToken::new();
        let handle = ProcessHandle {
            process_id: process.id.clone(),
            agent_id: agent_id.to_string(),
            child: None,
            started_at: chrono::Utc::now(),
            cancel: cancel.clone(),
        };

        {
            let mut handles = self.handles.lock().await;
            handles.insert(agent_id.to_string(), handle);
            self.spawning.lock().await.remove(agent_id);
            reservation.defuse();
        }

        let process_id = process.id.clone();
        let agent_id_owned = agent_id.to_string();
        let state_clone = state.clone();
        tokio::spawn(async move {
            Self::monitor_sandbox(state_clone, process_id, agent_id_owned, cancel).await;
        });

        processes::get_process_by_id(&state.pool, &process.id).await
    }

    #[cfg(feature = "sandbox")]
    fn build_sandbox_config(agent: &nous_core::agents::Agent) -> Result<SandboxConfig, NousError> {
        let metadata: serde_json::Value = agent
            .metadata_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        let sandbox_obj = metadata.get("sandbox").cloned().unwrap_or_default();

        let image = sandbox_obj
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or("ubuntu:24.04")
            .to_string();

        let cpus = sandbox_obj
            .get("cpus")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let memory_mib = sandbox_obj
            .get("memory_mib")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let network_policy = sandbox_obj
            .get("network_policy")
            .and_then(|v| v.as_str())
            .map(String::from);

        let max_duration_secs = sandbox_obj
            .get("max_duration_secs")
            .and_then(|v| v.as_u64());

        let idle_timeout_secs = sandbox_obj
            .get("idle_timeout_secs")
            .and_then(|v| v.as_u64());

        Ok(SandboxConfig {
            image,
            cpus,
            memory_mib,
            network_policy,
            volumes: None,
            secrets: None,
            max_duration_secs,
            idle_timeout_secs,
        })
    }

    #[cfg(feature = "sandbox")]
    async fn monitor_sandbox(
        state: AppState,
        process_id: String,
        agent_id: String,
        cancel: CancellationToken,
    ) {
        let poll_interval = Duration::from_secs(10);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    break;
                }
                _ = tokio::time::sleep(poll_interval) => {
                    let sandbox_mgr = match state.sandbox_manager() {
                        Some(mgr) => mgr,
                        None => break,
                    };

                    let status = {
                        let mgr = sandbox_mgr.lock().await;
                        mgr.get(&agent_id).map(|h| h.status.clone())
                    };

                    match status {
                        Some(ref s) if s != "running" => {
                            if let Err(e) = processes::update_process_status(
                                &state.pool,
                                UpdateProcessStatusRequest {
                                    process_id: &process_id,
                                    status: "crashed",
                                    exit_code: None,
                                    output: Some(&format!("sandbox status: {s}")),
                                    pid: None,
                                },
                            ).await {
                                tracing::warn!(process_id = %process_id, error = %e, "failed to update process status to crashed");
                            }
                            state.process_registry.handles.lock().await.remove(&agent_id);
                            return;
                        }
                        Some(_) => {
                            let mgr = sandbox_mgr.lock().await;
                            if let Ok(metrics) = mgr.metrics(&agent_id).await {
                                let summary = format!(
                                    "cpu={:.1}% mem={}MiB disk={}MiB",
                                    metrics.cpu_usage_percent,
                                    metrics.memory_used_mib,
                                    metrics.disk_used_mib,
                                );
                                if let Err(e) = processes::update_process_status(
                                    &state.pool,
                                    UpdateProcessStatusRequest {
                                        process_id: &process_id,
                                        status: "running",
                                        exit_code: None,
                                        output: Some(&summary),
                                        pid: None,
                                    },
                                ).await {
                                    tracing::warn!(process_id = %process_id, error = %e, "failed to update sandbox metrics");
                                }
                            }
                        }
                        None => {
                            if let Err(e) = processes::update_process_status(
                                &state.pool,
                                UpdateProcessStatusRequest {
                                    process_id: &process_id,
                                    status: "crashed",
                                    exit_code: None,
                                    output: Some("sandbox no longer tracked by manager"),
                                    pid: None,
                                },
                            ).await {
                                tracing::warn!(process_id = %process_id, error = %e, "failed to update process status to crashed");
                            }
                            state.process_registry.handles.lock().await.remove(&agent_id);
                            return;
                        }
                    }
                }
            }
        }

        state
            .process_registry
            .handles
            .lock()
            .await
            .remove(&agent_id);
    }

    async fn monitor_process(params: MonitorProcessParams) {
        let MonitorProcessParams {
            state,
            process_id,
            agent_id,
            cancel,
            timeout_secs,
        } = params;

        let result = Self::wait_for_exit_with_timeout(WaitForExitParams {
            state: &state,
            agent_id: &agent_id,
            process_id: &process_id,
            cancel,
            timeout_secs,
        })
        .await;

        if let Some((exit_code, output)) = result {
            Self::update_exit_status(&state, &process_id, exit_code, output.as_deref()).await;
        }

        state
            .process_registry
            .handles
            .lock()
            .await
            .remove(&agent_id);
    }

    async fn wait_for_exit_with_timeout(
        params: WaitForExitParams<'_>,
    ) -> Option<(Option<i32>, Option<String>)> {
        let WaitForExitParams {
            state,
            agent_id,
            process_id,
            cancel,
            timeout_secs,
        } = params;

        match timeout_secs.map(|s| safe_timeout(s, 300)) {
            Some(duration) => {
                Self::wait_with_deadline(state, (agent_id, process_id), cancel, duration).await
            }
            None => {
                tokio::select! {
                    () = cancel.cancelled() => None,
                    result = Self::wait_for_exit(state, agent_id) => Some(result),
                }
            }
        }
    }

    async fn wait_with_deadline(
        state: &AppState,
        ids: (&str, &str),
        cancel: CancellationToken,
        duration: Duration,
    ) -> Option<(Option<i32>, Option<String>)> {
        let (agent_id, process_id) = ids;
        tokio::select! {
            () = cancel.cancelled() => None,
            result = tokio::time::timeout(duration, Self::wait_for_exit(state, agent_id)) => {
                Self::handle_deadline_result(state, agent_id, process_id, result).await
            }
        }
    }

    async fn handle_deadline_result(
        state: &AppState,
        agent_id: &str,
        process_id: &str,
        result: Result<(Option<i32>, Option<String>), tokio::time::error::Elapsed>,
    ) -> Option<(Option<i32>, Option<String>)> {
        match result {
            Ok(exit_result) => Some(exit_result),
            Err(_) => {
                Self::handle_process_timeout(state, agent_id, process_id).await;
                None
            }
        }
    }

    async fn handle_process_timeout(state: &AppState, agent_id: &str, process_id: &str) {
        Self::log_process_status_update(
            &state.pool,
            UpdateProcessStatusRequest {
                process_id,
                status: "failed",
                exit_code: None,
                output: Some("process timed out"),
                pid: None,
            },
        )
        .await;
        Self::kill_process_handle(state, agent_id).await;
    }

    async fn kill_process_handle(state: &AppState, agent_id: &str) {
        let handle = state
            .process_registry
            .handles
            .lock()
            .await
            .remove(agent_id);
        if let Some(mut handle) = handle {
            if let Some(mut child) = handle.child.take() {
                let _ = child.kill().await;
            }
        }
    }

    async fn update_exit_status(
        state: &AppState,
        process_id: &str,
        exit_code: Option<i32>,
        output: Option<&str>,
    ) {
        let status = if exit_code == Some(0) { "stopped" } else { "crashed" };
        Self::log_process_status_update(
            &state.pool,
            UpdateProcessStatusRequest { process_id, status, exit_code, output, pid: None },
        )
        .await;
    }

    async fn log_process_status_update(
        pool: &DatabaseConnection,
        req: UpdateProcessStatusRequest<'_>,
    ) {
        let process_id = req.process_id;
        if let Err(e) = processes::update_process_status(pool, req).await {
            tracing::warn!(process_id = %process_id, error = %e, "failed to update process exit status");
        }
    }

    async fn wait_for_exit(state: &AppState, agent_id: &str) -> (Option<i32>, Option<String>) {
        // Take the child out of the handle so we release the lock before awaiting.
        // The handle entry stays in the map so stop() can still find the process.
        let child = {
            let mut handles = state.process_registry.handles.lock().await;
            handles.get_mut(agent_id).and_then(|h| h.child.take())
        };

        if let Some(mut child) = child {
            let status = child.wait().await;
            let exit_code = status.ok().and_then(|s| s.code());
            (exit_code, None)
        } else {
            (None, None)
        }
    }

    pub async fn stop(&self, params: StopParams<'_>) -> Result<Process, NousError> {
        let StopParams { state, agent_id, force, grace_secs } = params;
        let mut handles = self.handles.lock().await;
        let handle = handles.get_mut(agent_id).ok_or_else(|| {
            NousError::NotFound(format!("no running process for agent '{agent_id}'"))
        })?;

        let process_id = handle.process_id.clone();
        let is_sandbox = handle.child.is_none();

        Self::mark_stopping(&state.pool, &process_id).await;
        handle.cancel.cancel();

        #[cfg(feature = "sandbox")]
        if is_sandbox {
            return self
                .stop_sandbox(state, agent_id, &process_id, handles)
                .await;
        }

        #[cfg(not(feature = "sandbox"))]
        let _ = is_sandbox;

        let exit_code = Self::stop_native_child(handle, force, grace_secs).await;

        handles.remove(agent_id);
        drop(handles);

        processes::update_process_status(
            &state.pool,
            UpdateProcessStatusRequest {
                process_id: &process_id,
                status: "stopped",
                exit_code,
                output: None,
                pid: None,
            },
        )
        .await
    }

    async fn mark_stopping(pool: &DatabaseConnection, process_id: &str) {
        if let Err(e) = processes::update_process_status(
            pool,
            UpdateProcessStatusRequest {
                process_id,
                status: "stopping",
                exit_code: None,
                output: None,
                pid: None,
            },
        )
        .await
        {
            tracing::warn!(process_id = %process_id, error = %e, "failed to update process status to stopping");
        }
    }

    async fn stop_native_child(
        handle: &mut ProcessHandle,
        force: bool,
        grace_secs: u64,
    ) -> Option<i32> {
        if let Some(child) = handle.child.take() {
            Self::terminate_child(child, force, grace_secs).await
        } else {
            tokio::task::yield_now().await;
            None
        }
    }

    #[cfg(feature = "sandbox")]
    async fn stop_sandbox(
        &self,
        state: &AppState,
        agent_id: &str,
        process_id: &str,
        mut handles: tokio::sync::MutexGuard<'_, HashMap<String, ProcessHandle>>,
    ) -> Result<Process, NousError> {
        handles.remove(agent_id);
        drop(handles);

        if let Some(sandbox_mgr) = state.sandbox_manager() {
            let mut mgr = sandbox_mgr.lock().await;
            let _ = mgr.stop(agent_id).await;
        }

        processes::update_process_status(
            &state.pool,
            UpdateProcessStatusRequest {
                process_id,
                status: "stopped",
                exit_code: None,
                output: None,
                pid: None,
            },
        )
        .await
    }

    async fn terminate_child(
        mut child: tokio::process::Child,
        force: bool,
        grace_secs: u64,
    ) -> Option<i32> {
        if force {
            let _ = child.kill().await;
        } else {
            Self::graceful_terminate(&mut child, grace_secs).await;
        }
        child.try_wait().ok().flatten().and_then(|s| s.code())
    }

    async fn graceful_terminate(child: &mut tokio::process::Child, grace_secs: u64) {
        #[cfg(unix)]
        {
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
        }
        #[cfg(not(unix))]
        {
            let _ = child.kill().await;
        }

        let grace = Duration::from_secs(grace_secs);
        if tokio::time::timeout(grace, child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
    }

    pub async fn restart(&self, params: RestartParams<'_>) -> Result<Process, NousError> {
        let RestartParams { state, agent_id, command, working_dir } = params;

        let current = {
            let handles = self.handles.lock().await;
            handles.get(agent_id).map(|h| h.process_id.clone())
        };

        let (old_command, old_working_dir, old_process_type, old_env, old_timeout) =
            self.load_process_config(state, agent_id, current.as_deref()).await?;

        if current.is_some() {
            let _ = self.stop(StopParams { state, agent_id, force: false, grace_secs: 10 }).await;
        }

        let cmd = command.unwrap_or(&old_command);
        let wd = working_dir.or(old_working_dir.as_deref());
        let env: Option<serde_json::Value> = old_env
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        self.spawn(SpawnParams {
            state,
            agent_id,
            command: cmd,
            process_type: &old_process_type,
            working_dir: wd,
            env,
            timeout_secs: old_timeout,
        })
        .await
    }

    async fn load_process_config(
        &self,
        state: &AppState,
        agent_id: &str,
        current_process_id: Option<&str>,
    ) -> Result<(String, Option<String>, String, Option<String>, Option<i64>), NousError> {
        if let Some(pid) = current_process_id {
            let proc = processes::get_process_by_id(&state.pool, pid).await?;
            Ok((proc.command, proc.working_dir, proc.process_type, proc.env_json, proc.timeout_secs))
        } else {
            let agent = nous_core::agents::get_agent_by_id(&state.pool, agent_id).await?;
            Ok((
                agent.spawn_command.unwrap_or_default(),
                agent.working_dir,
                agent.process_type.unwrap_or_else(|| "shell".to_string()),
                Some("{}".to_string()),
                None,
            ))
        }
    }

    pub async fn invoke(&self, params: InvokeParams<'_>) -> Result<Invocation, NousError> {
        let InvokeParams { state, agent_id, prompt, timeout_secs, metadata, is_async } = params;

        let metadata_str = metadata
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        let invocation =
            processes::create_invocation(&state.pool, agent_id, prompt, metadata_str.as_deref())
                .await?;

        let invocation = processes::update_invocation(
            &state.pool,
            UpdateInvocationRequest {
                invocation_id: &invocation.id,
                status: "running",
                result: None,
                error: None,
                duration_ms: None,
            },
        )
        .await?;

        let agent = match nous_core::agents::get_agent_by_id(&state.pool, agent_id).await {
            Ok(a) => a,
            Err(e) => {
                Self::mark_invocation_failed(&state.pool, &invocation.id, &e.to_string()).await;
                return Err(e);
            }
        };

        let result = self
            .dispatch_invocation(DispatchInvocationParams {
                state,
                invocation: &invocation,
                prompt,
                timeout_secs,
                metadata: &metadata,
                is_async,
                process_type: agent.process_type.as_deref(),
            })
            .await;

        if let Err(ref e) = result {
            Self::mark_invocation_failed(&state.pool, &invocation.id, &e.to_string()).await;
        }
        result
    }

    async fn dispatch_invocation(
        &self,
        params: DispatchInvocationParams<'_>,
    ) -> Result<Invocation, NousError> {
        let DispatchInvocationParams {
            state,
            invocation,
            prompt,
            timeout_secs,
            metadata,
            is_async,
            process_type,
        } = params;
        match process_type {
            Some("claude") | Some("sandbox") => {
                self.invoke_claude(InvokeClaudeParams {
                    state,
                    invocation,
                    prompt,
                    timeout_secs,
                    metadata,
                    is_async,
                })
                .await
            }
            Some("shell") | None => {
                self.invoke_shell(InvokeShellParams {
                    state,
                    invocation,
                    prompt,
                    timeout_secs,
                    is_async,
                })
                .await
            }
            Some(other) => Err(NousError::Config(format!(
                "unsupported process_type '{other}'"
            ))),
        }
    }

    async fn mark_invocation_failed(
        pool: &DatabaseConnection,
        invocation_id: &str,
        error: &str,
    ) {
        if let Err(db_err) = processes::update_invocation(
            pool,
            UpdateInvocationRequest {
                invocation_id,
                status: "failed",
                result: None,
                error: Some(error),
                duration_ms: None,
            },
        )
        .await
        {
            tracing::warn!(invocation_id = %invocation_id, error = %db_err, "failed to mark invocation as failed");
        }
    }

    async fn invoke_shell(&self, params: InvokeShellParams<'_>) -> Result<Invocation, NousError> {
        let InvokeShellParams { state, invocation, prompt, timeout_secs, is_async } = params;

        if is_async {
            let inv_id = invocation.id.clone();
            let prompt_owned = prompt.to_string();
            let timeout = safe_timeout(timeout_secs.unwrap_or(300), 300);
            let state_clone = state.clone();
            tokio::spawn(async move {
                let start = std::time::Instant::now();
                let result =
                    tokio::time::timeout(timeout, Self::run_shell_command(&prompt_owned)).await;
                let duration_ms = start.elapsed().as_millis() as i64;
                let update_result =
                    Self::update_invocation_from_result(&state_clone.pool, &inv_id, result, duration_ms)
                        .await;
                if let Err(e) = update_result {
                    tracing::error!(inv_id = %inv_id, error = %e, "failed to update invocation status");
                }
            });
            return Ok(invocation.clone());
        }

        let start = std::time::Instant::now();
        let timeout = safe_timeout(timeout_secs.unwrap_or(300), 300);
        let result = tokio::time::timeout(timeout, Self::run_shell_command(prompt)).await;
        let duration_ms = start.elapsed().as_millis() as i64;

        Self::update_invocation_from_result(&state.pool, &invocation.id, result, duration_ms).await
    }

    async fn run_shell_command(prompt: &str) -> Result<String, String> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(prompt)
            .kill_on_drop(true)
            .output()
            .await;

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{stdout}\n{stderr}")
                };
                if out.status.success() {
                    Ok(combined)
                } else {
                    Err(format!("exit code {}: {combined}", out.status.code().unwrap_or(-1)))
                }
            }
            Err(e) => Err(format!("failed to execute: {e}")),
        }
    }

    async fn update_invocation_from_result(
        pool: &DatabaseConnection,
        invocation_id: &str,
        result: Result<Result<String, String>, tokio::time::error::Elapsed>,
        duration_ms: i64,
    ) -> Result<Invocation, NousError> {
        match result {
            Ok(Ok(output)) => {
                processes::update_invocation(
                    pool,
                    UpdateInvocationRequest {
                        invocation_id,
                        status: "completed",
                        result: Some(&output),
                        error: None,
                        duration_ms: Some(duration_ms),
                    },
                )
                .await
            }
            Ok(Err(err)) => {
                processes::update_invocation(
                    pool,
                    UpdateInvocationRequest {
                        invocation_id,
                        status: "failed",
                        result: None,
                        error: Some(&err),
                        duration_ms: Some(duration_ms),
                    },
                )
                .await
            }
            Err(_) => {
                processes::update_invocation(
                    pool,
                    UpdateInvocationRequest {
                        invocation_id,
                        status: "timeout",
                        result: None,
                        error: Some("invocation timed out"),
                        duration_ms: Some(duration_ms),
                    },
                )
                .await
            }
        }
    }

    async fn invoke_claude(
        &self,
        params: InvokeClaudeParams<'_>,
    ) -> Result<Invocation, NousError> {
        use rig::client::completion::CompletionClient;
        use rig::completion::Prompt as _;

        let InvokeClaudeParams { state, invocation, prompt, timeout_secs, metadata, is_async } =
            params;

        let client = state.llm_client.as_ref().ok_or_else(|| {
            NousError::Unavailable("LLM client not configured — set AWS credentials".into())
        })?;

        let model = metadata
            .as_ref()
            .and_then(|m| m.get("model"))
            .and_then(|v| v.as_str())
            .unwrap_or(&state.default_model)
            .to_string();

        let preamble = metadata
            .as_ref()
            .and_then(|m| m.get("preamble"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if is_async {
            let inv_id = invocation.id.clone();
            let prompt_owned = prompt.to_string();
            let timeout = safe_timeout(timeout_secs.unwrap_or(300), 300);
            let state_clone = state.clone();
            let client = client.clone();
            tokio::spawn(async move {
                let agent = Self::build_llm_agent(&client, &model, &preamble);
                let start = std::time::Instant::now();
                let result = tokio::time::timeout(timeout, agent.prompt(&prompt_owned)).await;
                let duration_ms = start.elapsed().as_millis() as i64;
                let outcome = Self::classify_llm_result(result, &model, &inv_id, duration_ms);
                let update_result =
                    Self::record_llm_outcome(&state_clone.pool, &inv_id, outcome).await;
                if let Err(e) = update_result {
                    tracing::error!(inv_id = %inv_id, error = %e, "failed to update invocation status");
                }
            });
            return Ok(invocation.clone());
        }

        let agent = Self::build_llm_agent(client, &model, &preamble);
        let start = std::time::Instant::now();
        let timeout = safe_timeout(timeout_secs.unwrap_or(300), 300);
        let result = tokio::time::timeout(timeout, agent.prompt(prompt)).await;
        let duration_ms = start.elapsed().as_millis() as i64;

        let outcome = Self::classify_llm_result(result, &model, &invocation.id, duration_ms);
        Self::record_llm_outcome(&state.pool, &invocation.id, outcome).await
    }

    fn build_llm_agent(
        client: &crate::llm_client::LlmClient,
        model: &str,
        preamble: &str,
    ) -> impl rig::completion::Prompt {
        use rig::client::completion::CompletionClient;
        if preamble.is_empty() {
            client.agent(model).build()
        } else {
            client.agent(model).preamble(preamble).build()
        }
    }

    fn classify_llm_result<E: std::fmt::Debug>(
        result: Result<Result<String, E>, tokio::time::error::Elapsed>,
        model: &str,
        invocation_id: &str,
        duration_ms: i64,
    ) -> LlmOutcome {
        match result {
            Ok(Ok(output)) => LlmOutcome::Completed { output, duration_ms },
            Ok(Err(err)) => {
                let error_detail = format!("Bedrock error (model={model}): {err:?}");
                tracing::error!(
                    invocation_id = %invocation_id,
                    model = %model,
                    duration_ms = %duration_ms,
                    error = %error_detail,
                    "LLM invocation failed"
                );
                LlmOutcome::Failed { error_detail, duration_ms }
            }
            Err(_) => LlmOutcome::TimedOut { duration_ms },
        }
    }

    async fn record_llm_outcome(
        pool: &DatabaseConnection,
        invocation_id: &str,
        outcome: LlmOutcome,
    ) -> Result<Invocation, NousError> {
        match outcome {
            LlmOutcome::Completed { output, duration_ms } => {
                processes::update_invocation(
                    pool,
                    UpdateInvocationRequest {
                        invocation_id,
                        status: "completed",
                        result: Some(&output),
                        error: None,
                        duration_ms: Some(duration_ms),
                    },
                )
                .await
            }
            LlmOutcome::Failed { error_detail, duration_ms } => {
                processes::update_invocation(
                    pool,
                    UpdateInvocationRequest {
                        invocation_id,
                        status: "failed",
                        result: None,
                        error: Some(&error_detail),
                        duration_ms: Some(duration_ms),
                    },
                )
                .await
            }
            LlmOutcome::TimedOut { duration_ms } => {
                processes::update_invocation(
                    pool,
                    UpdateInvocationRequest {
                        invocation_id,
                        status: "timeout",
                        result: None,
                        error: Some("invocation timed out"),
                        duration_ms: Some(duration_ms),
                    },
                )
                .await
            }
        }
    }

    pub async fn get_status(&self, agent_id: &str) -> Option<ProcessStatus> {
        let handles = self.handles.lock().await;
        if let Some(handle) = handles.get(agent_id) {
            let uptime = chrono::Utc::now() - handle.started_at;
            Some(ProcessStatus {
                process_id: handle.process_id.clone(),
                agent_id: handle.agent_id.clone(),
                pid: handle.child.as_ref().and_then(tokio::process::Child::id),
                uptime_secs: uptime.num_seconds(),
            })
        } else {
            None
        }
    }

    #[cfg(feature = "sandbox")]
    pub async fn recover_sandboxes(
        &self,
        pool: &DatabaseConnection,
        sandbox_mgr: &Arc<tokio::sync::Mutex<crate::sandbox::SandboxManager>>,
    ) -> Result<(), NousError> {
        use sea_orm::{ConnectionTrait, Statement};
        let rows = pool
            .query_all(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT * FROM agent_processes WHERE process_type = 'sandbox' AND status IN ('running', 'starting')",
            ))
            .await?;

        let mut processes = Vec::new();
        for row in &rows {
            use sea_orm::TryGetable;
            let id: String = row.try_get_by("id")?;
            let proc = processes::get_process_by_id(pool, &id).await?;
            processes.push(proc);
        }

        let mut recovered = 0u32;
        let mut crashed = 0u32;

        for proc in processes {
            let sandbox_name = match proc.sandbox_name.as_deref() {
                Some(name) => name,
                None => {
                    tracing::warn!(
                        agent_id = %proc.agent_id,
                        process_id = %proc.id,
                        "sandbox process missing sandbox_name, marking crashed"
                    );
                    if let Err(e) = processes::update_process_status(
                        pool,
                        UpdateProcessStatusRequest {
                            process_id: &proc.id,
                            status: "crashed",
                            exit_code: None,
                            output: Some("no sandbox_name on recovery"),
                            pid: None,
                        },
                    )
                    .await
                    {
                        tracing::warn!(
                            process_id = %proc.id,
                            error = %e,
                            "failed to mark sandbox process as crashed"
                        );
                    }
                    crashed += 1;
                    continue;
                }
            };
            let image = proc.sandbox_image.as_deref().unwrap_or("unknown");

            let reconnect_result = {
                let mut mgr = sandbox_mgr.lock().await;
                mgr.reconnect(&proc.agent_id, sandbox_name, image).await
            };

            match reconnect_result {
                Ok(true) => {
                    let cancel = CancellationToken::new();
                    let handle = ProcessHandle {
                        process_id: proc.id.clone(),
                        agent_id: proc.agent_id.clone(),
                        child: None,
                        started_at: chrono::Utc::now(),
                        cancel,
                    };
                    {
                        let mut handles = self.handles.lock().await;
                        handles.insert(proc.agent_id.clone(), handle);
                    }
                    tracing::info!(
                        agent_id = %proc.agent_id,
                        sandbox_name = sandbox_name,
                        "recovered sandbox on daemon restart"
                    );
                    recovered += 1;
                }
                Ok(false) | Err(_) => {
                    if let Err(e) = processes::update_process_status(
                        pool,
                        UpdateProcessStatusRequest {
                            process_id: &proc.id,
                            status: "crashed",
                            exit_code: None,
                            output: Some("sandbox unreachable on daemon restart"),
                            pid: None,
                        },
                    )
                    .await
                    {
                        tracing::warn!(process_id = %proc.id, error = %e, "failed to mark process as crashed during recovery");
                    }
                    tracing::warn!(
                        agent_id = %proc.agent_id,
                        sandbox_name = sandbox_name,
                        "sandbox crashed while daemon was down"
                    );
                    crashed += 1;
                }
            }
        }

        tracing::info!(recovered, crashed, "sandbox recovery complete");
        Ok(())
    }

    pub async fn shutdown(&self, state: &AppState) {
        let agent_ids: Vec<String> = {
            let handles = self.handles.lock().await;
            handles.keys().cloned().collect()
        };

        for agent_id in agent_ids {
            self.shutdown_agent(state, &agent_id).await;
        }
    }

    async fn shutdown_agent(&self, state: &AppState, agent_id: &str) {
        tracing::info!(agent_id = %agent_id, "stopping agent process for shutdown");
        self.stop_with_fallback(state, agent_id).await;
    }

    async fn stop_with_fallback(&self, state: &AppState, agent_id: &str) {
        let graceful_result = self.try_graceful_stop(state, agent_id).await;
        self.handle_graceful_stop_result(state, agent_id, graceful_result).await;
    }

    async fn handle_graceful_stop_result(
        &self,
        state: &AppState,
        agent_id: &str,
        result: Result<Process, NousError>,
    ) {
        if let Err(e) = result {
            tracing::warn!(agent_id = %agent_id, error = %e, "failed to gracefully stop process, force killing");
            self.force_stop_agent(state, agent_id).await;
        }
    }

    async fn try_graceful_stop(
        &self,
        state: &AppState,
        agent_id: &str,
    ) -> Result<Process, NousError> {
        self.stop(StopParams { state, agent_id, force: false, grace_secs: 5 })
            .await
    }

    async fn force_stop_agent(&self, state: &AppState, agent_id: &str) {
        let _ = self
            .stop(StopParams { state, agent_id, force: true, grace_secs: 0 })
            .await;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub process_id: String,
    pub agent_id: String,
    pub pid: Option<u32>,
    pub uptime_secs: i64,
}
