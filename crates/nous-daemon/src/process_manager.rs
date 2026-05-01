use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use nous_core::agents::processes::{self, Invocation, Process};
use nous_core::error::NousError;

use crate::state::AppState;

pub struct ProcessHandle {
    pub process_id: String,
    pub agent_id: String,
    pub child: Option<tokio::process::Child>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub cancel: CancellationToken,
}

pub struct ProcessRegistry {
    handles: Mutex<HashMap<String, ProcessHandle>>, // keyed by agent_id
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn(
        &self,
        state: &AppState,
        agent_id: &str,
        command: &str,
        process_type: &str,
        working_dir: Option<&str>,
        env: Option<serde_json::Value>,
        timeout_secs: Option<i64>,
        restart_policy: &str,
        max_restarts: i32,
    ) -> Result<Process, NousError> {
        // Check if already running
        {
            let handles = self.handles.lock().await;
            if handles.contains_key(agent_id) {
                return Err(NousError::Conflict(format!(
                    "agent '{agent_id}' already has a running process"
                )));
            }
        }

        let env_json = env
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()));

        // Create DB record
        let process = processes::create_process(
            &state.pool,
            agent_id,
            process_type,
            command,
            working_dir,
            env_json.as_deref(),
            timeout_secs,
            Some(restart_policy),
            Some(max_restarts),
        )
        .await?;

        // Update to starting
        let process = processes::update_process_status(
            &state.pool,
            &process.id,
            "starting",
            None,
            None,
            None,
        )
        .await?;

        // Spawn the child process
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd.kill_on_drop(true);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        if let Some(wd) = working_dir {
            cmd.current_dir(wd);
        }

        if let Some(ref env_val) = env {
            if let Some(obj) = env_val.as_object() {
                for (k, v) in obj {
                    if let Some(val) = v.as_str() {
                        cmd.env(k, val);
                    }
                }
            }
        }

        let child = cmd
            .spawn()
            .map_err(|e| NousError::Internal(format!("failed to spawn process: {e}")))?;

        let pid = child.id().map(|p| p as i64);

        // Update to running with PID
        let process =
            processes::update_process_status(&state.pool, &process.id, "running", None, None, pid)
                .await?;

        let cancel = CancellationToken::new();
        let handle = ProcessHandle {
            process_id: process.id.clone(),
            agent_id: agent_id.to_string(),
            child: Some(child),
            started_at: chrono::Utc::now(),
            cancel: cancel.clone(),
        };

        // Start background monitoring task
        let process_id = process.id.clone();
        let agent_id_owned = agent_id.to_string();
        let timeout = timeout_secs;
        let restart_p = restart_policy.to_string();
        let max_r = max_restarts;
        let command_owned = command.to_string();
        let working_dir_owned = working_dir.map(String::from);
        let env_owned = env.clone();

        // Store handle first
        {
            let mut handles = self.handles.lock().await;
            handles.insert(agent_id.to_string(), handle);
        }

        // Spawn the monitor task
        let state_clone = state.clone();
        tokio::spawn(async move {
            Self::monitor_process(
                state_clone,
                process_id,
                agent_id_owned,
                cancel,
                timeout,
                restart_p,
                max_r,
                command_owned,
                working_dir_owned,
                env_owned,
            )
            .await;
        });

        Ok(process)
    }

    #[allow(clippy::too_many_arguments)]
    async fn monitor_process(
        state: AppState,
        process_id: String,
        agent_id: String,
        cancel: CancellationToken,
        timeout_secs: Option<i64>,
        restart_policy: String,
        max_restarts: i32,
        _command: String,
        _working_dir: Option<String>,
        _env: Option<serde_json::Value>,
    ) {
        // Wait for process exit or cancellation
        let timeout_duration = timeout_secs.map(|s| Duration::from_secs(s as u64));

        let result = if let Some(duration) = timeout_duration {
            tokio::select! {
                _ = cancel.cancelled() => {
                    // Intentional stop
                    None
                }
                result = tokio::time::timeout(duration, Self::wait_for_exit(&state, &agent_id)) => {
                    match result {
                        Ok(exit_result) => Some(exit_result),
                        Err(_) => {
                            // Timeout
                            let _ = processes::update_process_status(
                                &state.pool, &process_id, "failed", None,
                                Some("process timed out"), None,
                            ).await;
                            // Kill the child from handles
                            if let Some(mut handle) = state.process_registry.handles.lock().await.remove(&agent_id) {
                                if let Some(mut child) = handle.child.take() {
                                    let _ = child.kill().await;
                                }
                            }
                            return;
                        }
                    }
                }
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => None,
                result = Self::wait_for_exit(&state, &agent_id) => Some(result),
            }
        };

        if let Some((exit_code, output)) = result {
            let status = if exit_code == Some(0) {
                "stopped"
            } else {
                "crashed"
            };
            let _ = processes::update_process_status(
                &state.pool,
                &process_id,
                status,
                exit_code,
                output.as_deref(),
                None,
            )
            .await;

            // Check restart policy
            if status == "crashed" {
                let should_restart = match restart_policy.as_str() {
                    "always" => true,
                    "on-failure" => exit_code != Some(0),
                    _ => false,
                };

                if should_restart {
                    if let Ok(proc) = processes::get_process_by_id(&state.pool, &process_id).await {
                        if proc.restart_count < max_restarts {
                            let _ =
                                processes::increment_restart_count(&state.pool, &process_id).await;
                            tracing::info!(
                                agent_id = %agent_id,
                                restart_count = proc.restart_count + 1,
                                "restarting crashed agent process"
                            );
                            // Note: actual restart would require re-calling spawn, which
                            // needs the registry reference. For now we log the intent.
                            // The process_manager.restart() method should be used externally.
                        }
                    }
                }
            }
        }

        // Remove from handles
        state
            .process_registry
            .handles
            .lock()
            .await
            .remove(&agent_id);
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

    pub async fn stop(
        &self,
        state: &AppState,
        agent_id: &str,
        force: bool,
        grace_secs: u64,
    ) -> Result<Process, NousError> {
        let mut handles = self.handles.lock().await;
        let handle = handles.get_mut(agent_id).ok_or_else(|| {
            NousError::NotFound(format!("no running process for agent '{agent_id}'"))
        })?;

        let process_id = handle.process_id.clone();

        // Update status to stopping
        let _ = processes::update_process_status(
            &state.pool,
            &process_id,
            "stopping",
            None,
            None,
            None,
        )
        .await;

        // Cancel the monitor task
        handle.cancel.cancel();

        let exit_code = if let Some(mut child) = handle.child.take() {
            if force {
                let _ = child.kill().await;
            } else {
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
                match tokio::time::timeout(grace, child.wait()).await {
                    Ok(_) => {}
                    Err(_) => {
                        let _ = child.kill().await;
                    }
                }
            }
            child.try_wait().ok().flatten().and_then(|s| s.code())
        } else {
            // Child already taken by monitor wait_for_exit(). Process killed via kill_on_drop when monitor future drops.
            // Brief yield to let the kill propagate.
            tokio::task::yield_now().await;
            None
        };

        handles.remove(agent_id);
        drop(handles);

        processes::update_process_status(&state.pool, &process_id, "stopped", exit_code, None, None)
            .await
    }

    pub async fn restart(
        &self,
        state: &AppState,
        agent_id: &str,
        command: Option<&str>,
        working_dir: Option<&str>,
    ) -> Result<Process, NousError> {
        // Get current process info before stopping
        let current = {
            let handles = self.handles.lock().await;
            handles.get(agent_id).map(|h| h.process_id.clone())
        };

        let (
            old_command,
            old_working_dir,
            old_process_type,
            old_env,
            old_timeout,
            old_policy,
            old_max_restarts,
        ) = if let Some(ref pid) = current {
            let proc = processes::get_process_by_id(&state.pool, pid).await?;
            (
                proc.command,
                proc.working_dir,
                proc.process_type,
                proc.env_json,
                proc.timeout_secs,
                proc.restart_policy,
                proc.max_restarts,
            )
        } else {
            // No running process, check agent config
            let agent = nous_core::agents::get_agent_by_id(&state.pool, agent_id).await?;
            (
                agent.spawn_command.unwrap_or_default(),
                agent.working_dir,
                agent.process_type.unwrap_or_else(|| "shell".to_string()),
                Some("{}".to_string()),
                None,
                "never".to_string(),
                3,
            )
        };

        // Stop if running
        if current.is_some() {
            let _ = self.stop(state, agent_id, false, 10).await;
        }

        let cmd = command.unwrap_or(&old_command);
        let wd = working_dir.or(old_working_dir.as_deref());
        let env: Option<serde_json::Value> = old_env
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());

        self.spawn(
            state,
            agent_id,
            cmd,
            &old_process_type,
            wd,
            env,
            old_timeout,
            &old_policy,
            old_max_restarts,
        )
        .await
    }

    pub async fn invoke(
        &self,
        state: &AppState,
        agent_id: &str,
        prompt: &str,
        timeout_secs: Option<i64>,
        metadata: Option<serde_json::Value>,
        is_async: bool,
    ) -> Result<Invocation, NousError> {
        let metadata_str = metadata
            .as_ref()
            .and_then(|v| serde_json::to_string(v).ok());

        let invocation =
            processes::create_invocation(&state.pool, agent_id, prompt, metadata_str.as_deref())
                .await?;

        // Update to running
        let invocation =
            processes::update_invocation(&state.pool, &invocation.id, "running", None, None, None)
                .await?;

        let agent = match nous_core::agents::get_agent_by_id(&state.pool, agent_id).await {
            Ok(a) => a,
            Err(e) => {
                let _ = processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "failed",
                    None,
                    Some(&e.to_string()),
                    None,
                )
                .await;
                return Err(e);
            }
        };

        let result = match agent.process_type.as_deref() {
            Some("claude") => {
                self.invoke_claude(
                    state,
                    &invocation,
                    prompt,
                    timeout_secs,
                    &metadata,
                    is_async,
                )
                .await
            }
            Some("shell") | None => {
                self.invoke_shell(state, &invocation, prompt, timeout_secs, is_async)
                    .await
            }
            Some(other) => Err(NousError::Config(format!(
                "unsupported process_type '{other}'"
            ))),
        };

        if let Err(ref e) = result {
            let _ = processes::update_invocation(
                &state.pool,
                &invocation.id,
                "failed",
                None,
                Some(&e.to_string()),
                None,
            )
            .await;
        }
        result
    }

    async fn invoke_shell(
        &self,
        state: &AppState,
        invocation: &Invocation,
        prompt: &str,
        timeout_secs: Option<i64>,
        is_async: bool,
    ) -> Result<Invocation, NousError> {
        if is_async {
            let inv_id = invocation.id.clone();
            let prompt_owned = prompt.to_string();
            let timeout = Duration::from_secs(timeout_secs.unwrap_or(300) as u64);
            let state_clone = state.clone();
            tokio::spawn(async move {
                let start = std::time::Instant::now();
                let result = tokio::time::timeout(timeout, async {
                    let output = Command::new("sh")
                        .arg("-c")
                        .arg(&prompt_owned)
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
                                Err(format!(
                                    "exit code {}: {combined}",
                                    out.status.code().unwrap_or(-1)
                                ))
                            }
                        }
                        Err(e) => Err(format!("failed to execute: {e}")),
                    }
                })
                .await;
                let duration_ms = start.elapsed().as_millis() as i64;
                let update_result = match result {
                    Ok(Ok(output)) => {
                        processes::update_invocation(
                            &state_clone.pool,
                            &inv_id,
                            "completed",
                            Some(&output),
                            None,
                            Some(duration_ms),
                        )
                        .await
                    }
                    Ok(Err(err)) => {
                        processes::update_invocation(
                            &state_clone.pool,
                            &inv_id,
                            "failed",
                            None,
                            Some(&err),
                            Some(duration_ms),
                        )
                        .await
                    }
                    Err(_) => {
                        processes::update_invocation(
                            &state_clone.pool,
                            &inv_id,
                            "timeout",
                            None,
                            Some("invocation timed out"),
                            Some(duration_ms),
                        )
                        .await
                    }
                };
                if let Err(e) = update_result {
                    tracing::error!(inv_id = %inv_id, error = %e, "failed to update invocation status");
                }
            });
            return Ok(invocation.clone());
        }

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(300) as u64);

        let result = tokio::time::timeout(timeout, async {
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
                        Err(format!(
                            "exit code {}: {combined}",
                            out.status.code().unwrap_or(-1)
                        ))
                    }
                }
                Err(e) => Err(format!("failed to execute: {e}")),
            }
        })
        .await;

        let duration_ms = start.elapsed().as_millis() as i64;

        match result {
            Ok(Ok(output)) => {
                processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "completed",
                    Some(&output),
                    None,
                    Some(duration_ms),
                )
                .await
            }
            Ok(Err(err)) => {
                processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "failed",
                    None,
                    Some(&err),
                    Some(duration_ms),
                )
                .await
            }
            Err(_) => {
                processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "timeout",
                    None,
                    Some("invocation timed out"),
                    Some(duration_ms),
                )
                .await
            }
        }
    }

    async fn invoke_claude(
        &self,
        state: &AppState,
        invocation: &Invocation,
        prompt: &str,
        timeout_secs: Option<i64>,
        metadata: &Option<serde_json::Value>,
        is_async: bool,
    ) -> Result<Invocation, NousError> {
        use rig::client::completion::CompletionClient;
        use rig::completion::Prompt as _;

        let client = state.llm_client.as_ref().ok_or_else(|| {
            NousError::Config("LLM client not configured — set AWS credentials".into())
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
            let timeout = Duration::from_secs(timeout_secs.unwrap_or(300) as u64);
            let state_clone = state.clone();
            let client = client.clone();
            tokio::spawn(async move {
                let agent = if preamble.is_empty() {
                    client.agent(&model).build()
                } else {
                    client.agent(&model).preamble(&preamble).build()
                };
                let start = std::time::Instant::now();
                let result = tokio::time::timeout(timeout, agent.prompt(&prompt_owned)).await;
                let duration_ms = start.elapsed().as_millis() as i64;
                let update_result = match result {
                    Ok(Ok(output)) => {
                        processes::update_invocation(
                            &state_clone.pool,
                            &inv_id,
                            "completed",
                            Some(&output),
                            None,
                            Some(duration_ms),
                        )
                        .await
                    }
                    Ok(Err(err)) => {
                        processes::update_invocation(
                            &state_clone.pool,
                            &inv_id,
                            "failed",
                            None,
                            Some(&err.to_string()),
                            Some(duration_ms),
                        )
                        .await
                    }
                    Err(_) => {
                        processes::update_invocation(
                            &state_clone.pool,
                            &inv_id,
                            "timeout",
                            None,
                            Some("invocation timed out"),
                            Some(duration_ms),
                        )
                        .await
                    }
                };
                if let Err(e) = update_result {
                    tracing::error!(inv_id = %inv_id, error = %e, "failed to update invocation status");
                }
            });
            return Ok(invocation.clone());
        }

        let agent = if preamble.is_empty() {
            client.agent(&model).build()
        } else {
            client.agent(&model).preamble(&preamble).build()
        };

        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(300) as u64);
        let result = tokio::time::timeout(timeout, agent.prompt(prompt)).await;
        let duration_ms = start.elapsed().as_millis() as i64;

        match result {
            Ok(Ok(output)) => {
                processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "completed",
                    Some(&output),
                    None,
                    Some(duration_ms),
                )
                .await
            }
            Ok(Err(err)) => {
                processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "failed",
                    None,
                    Some(&err.to_string()),
                    Some(duration_ms),
                )
                .await
            }
            Err(_) => {
                processes::update_invocation(
                    &state.pool,
                    &invocation.id,
                    "timeout",
                    None,
                    Some("invocation timed out"),
                    Some(duration_ms),
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
                pid: handle.child.as_ref().and_then(|c| c.id()),
                uptime_secs: uptime.num_seconds(),
            })
        } else {
            None
        }
    }

    pub async fn shutdown(&self, state: &AppState) {
        let agent_ids: Vec<String> = {
            let handles = self.handles.lock().await;
            handles.keys().cloned().collect()
        };

        for agent_id in agent_ids {
            tracing::info!(agent_id = %agent_id, "stopping agent process for shutdown");
            if let Err(e) = self.stop(state, &agent_id, false, 5).await {
                tracing::warn!(agent_id = %agent_id, error = %e, "failed to gracefully stop process, force killing");
                let _ = self.stop(state, &agent_id, true, 0).await;
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessStatus {
    pub process_id: String,
    pub agent_id: String,
    pub pid: Option<u32>,
    pub uptime_secs: i64,
}
