use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Notify;

use crate::channel::{ReadPool, WriteChannel};
use crate::schedule_db::ScheduleDb;
use crate::types::{RunPatch, RunStatus, Schedule, ScheduleRun};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub allow_shell: bool,
    pub allow_http: bool,
    pub max_concurrent: usize,
    pub default_timeout_secs: u64,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_shell: false,
            allow_http: true,
            max_concurrent: 4,
            default_timeout_secs: 300,
        }
    }
}

pub struct Scheduler {
    write_channel: WriteChannel,
    read_pool: ReadPool,
    notify: Arc<Notify>,
    config: ScheduleConfig,
}

impl Scheduler {
    pub fn spawn(
        wc: WriteChannel,
        rp: ReadPool,
        config: ScheduleConfig,
    ) -> (Arc<Notify>, tokio::task::JoinHandle<()>) {
        let notify = Arc::new(Notify::new());
        let scheduler = Scheduler {
            write_channel: wc,
            read_pool: rp,
            notify: notify.clone(),
            config,
        };
        let handle = tokio::spawn(scheduler.run());
        (notify, handle)
    }

    async fn run(self) {
        self.startup_recovery().await;

        let running = Arc::new(tokio::sync::Mutex::new(HashSet::<String>::new()));
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent));

        loop {
            let next = self.read_pool.with_conn(ScheduleDb::next_pending).await;

            match next {
                Ok(Some(schedule)) => {
                    let now = Utc::now().timestamp();
                    let fire_at = schedule.next_run_at.unwrap_or(now);
                    let delay = (fire_at - now).max(0) as u64;

                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {
                            if let Err(e) = self.write_channel.compute_next_run(schedule.id.clone()).await {
                                eprintln!("scheduler: compute_next_run failed for {}: {e}", schedule.id);
                                tokio::time::sleep(Duration::from_secs(5)).await;
                                continue;
                            }

                            let wc = self.write_channel.clone();
                            let rp = self.read_pool.clone();
                            let running = Arc::clone(&running);
                            let semaphore = Arc::clone(&semaphore);
                            let config = self.config.clone();

                            tokio::spawn(async move {
                                let schedule_id = schedule.id.clone();
                                {
                                    let mut set = running.lock().await;
                                    if set.contains(&schedule_id) {
                                        let _ = record_skipped(&wc, &schedule_id).await;
                                        return;
                                    }
                                    set.insert(schedule_id.clone());
                                }

                                let _permit = match semaphore.acquire().await {
                                    Ok(p) => p,
                                    Err(_) => {
                                        running.lock().await.remove(&schedule_id);
                                        return;
                                    }
                                };

                                execute_schedule(&schedule, &wc, &rp, &config).await;
                                running.lock().await.remove(&schedule_id);
                            });
                        }
                        _ = self.notify.notified() => {
                            continue;
                        }
                    }
                }
                Ok(None) => {
                    self.notify.notified().await;
                }
                Err(e) => {
                    eprintln!("scheduler: query error: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn startup_recovery(&self) {
        let wc = self.write_channel.clone();
        let ids = self
            .read_pool
            .with_conn(|conn| {
                let mut stmt = conn.prepare("SELECT id FROM schedules WHERE enabled = 1")?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await;

        if let Ok(ids) = ids {
            for id in ids {
                let _ = wc.compute_next_run(id).await;
            }
        }
    }
}

async fn record_skipped(wc: &WriteChannel, schedule_id: &str) -> nous_shared::Result<String> {
    let now = Utc::now().timestamp();
    let run = ScheduleRun {
        id: String::new(),
        schedule_id: schedule_id.to_string(),
        started_at: now,
        finished_at: Some(now),
        status: RunStatus::Skipped,
        exit_code: None,
        output: None,
        error: Some("previous run still in progress".to_string()),
        attempt: 1,
        duration_ms: Some(0),
    };
    wc.record_run(run).await
}

async fn execute_schedule(
    schedule: &Schedule,
    wc: &WriteChannel,
    rp: &ReadPool,
    config: &ScheduleConfig,
) {
    let timeout_secs = schedule
        .timeout_secs
        .filter(|&s| s > 0)
        .map(|s| s as u64)
        .unwrap_or(config.default_timeout_secs);

    let max_retries = schedule.max_retries.max(1);

    for attempt in 1..=max_retries {
        let started_at = Utc::now().timestamp();
        let instant = std::time::Instant::now();
        let run = ScheduleRun {
            id: String::new(),
            schedule_id: schedule.id.clone(),
            started_at,
            finished_at: None,
            status: RunStatus::Running,
            exit_code: None,
            output: None,
            error: None,
            attempt,
            duration_ms: None,
        };

        let run_id = match wc.record_run(run).await {
            Ok(id) => id,
            Err(e) => {
                eprintln!("scheduler: record_run failed: {e}");
                return;
            }
        };

        let result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            dispatch_action(schedule, wc, rp, config),
        )
        .await;

        let finished_at = Utc::now().timestamp();
        let duration_ms = instant.elapsed().as_millis() as i64;

        let patch = match result {
            Ok(Ok(output)) => RunPatch {
                finished_at: Some(finished_at),
                status: Some(RunStatus::Completed),
                exit_code: Some(0),
                output: Some(truncate_output(&output, schedule.max_output_bytes as usize)),
                error: None,
                duration_ms: Some(duration_ms),
            },
            Ok(Err(error)) => {
                let patch = RunPatch {
                    finished_at: Some(finished_at),
                    status: Some(RunStatus::Failed),
                    exit_code: Some(1),
                    output: None,
                    error: Some(truncate_output(&error, schedule.max_output_bytes as usize)),
                    duration_ms: Some(duration_ms),
                };

                let _ = wc.update_run(run_id, patch).await;

                if attempt < max_retries {
                    let backoff = 2u64.saturating_pow(attempt.min(20) as u32);
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                    continue;
                }
                return;
            }
            Err(_) => RunPatch {
                finished_at: Some(finished_at),
                status: Some(RunStatus::Timeout),
                exit_code: None,
                output: None,
                error: Some(format!("timeout after {timeout_secs}s")),
                duration_ms: Some(duration_ms),
            },
        };

        let _ = wc.update_run(run_id, patch).await;
        return;
    }
}

async fn dispatch_action(
    schedule: &Schedule,
    wc: &WriteChannel,
    rp: &ReadPool,
    config: &ScheduleConfig,
) -> Result<String, String> {
    match schedule.action_type {
        crate::types::ActionType::McpTool => dispatch_mcp_tool(schedule, wc, rp).await,
        crate::types::ActionType::Shell => {
            if !config.allow_shell {
                return Err("shell actions disabled by configuration".to_string());
            }
            Err("shell actions not implemented in Phase 1".to_string())
        }
        crate::types::ActionType::Http => {
            if !config.allow_http {
                return Err("http actions disabled by configuration".to_string());
            }
            Err("http actions not implemented in Phase 1".to_string())
        }
    }
}

#[derive(Deserialize)]
struct McpToolPayload {
    tool: String,
    #[serde(default)]
    args: Value,
}

async fn dispatch_mcp_tool(
    schedule: &Schedule,
    wc: &WriteChannel,
    rp: &ReadPool,
) -> Result<String, String> {
    let payload: McpToolPayload = serde_json::from_str(&schedule.action_payload)
        .map_err(|e| format!("invalid mcp_tool payload: {e}"))?;

    match payload.tool.as_str() {
        "memory_stats" => {
            let result = rp
                .with_conn(crate::db::MemoryDb::stats_on)
                .await
                .map_err(|e| format!("memory_stats failed: {e}"))?;
            Ok(result.to_string())
        }
        "memory_search" => {
            let query = payload
                .args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if query.is_empty() {
                return Err("memory_search requires a 'query' argument".to_string());
            }
            let result = rp
                .with_conn(move |conn| {
                    let pattern = format!("%{query}%");
                    let mut stmt = conn.prepare(
                        "SELECT id, title, memory_type FROM memories
                         WHERE archived = 0 AND (title LIKE ?1 OR content LIKE ?1)
                         ORDER BY created_at DESC LIMIT 10",
                    )?;
                    let rows = stmt
                        .query_map(rusqlite::params![pattern], |row| {
                            Ok(serde_json::json!({
                                "id": row.get::<_, String>(0)?,
                                "title": row.get::<_, String>(1)?,
                                "memory_type": row.get::<_, String>(2)?,
                            }))
                        })?
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    Ok(serde_json::json!({"results": rows}))
                })
                .await
                .map_err(|e| format!("memory_search failed: {e}"))?;
            Ok(result.to_string())
        }
        "memory_forget" => {
            let id = payload
                .args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "memory_forget requires an 'id' argument".to_string())?;
            let hard = payload
                .args
                .get("hard")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mid = id
                .parse::<nous_shared::ids::MemoryId>()
                .map_err(|e| format!("invalid memory id: {e}"))?;
            let result = wc
                .forget(mid, hard)
                .await
                .map_err(|e| format!("memory_forget failed: {e}"))?;
            Ok(serde_json::json!({"forgotten": result}).to_string())
        }
        "__test_delay" => {
            let secs = payload
                .args
                .get("secs")
                .and_then(|v| v.as_u64())
                .unwrap_or(5);
            tokio::time::sleep(Duration::from_secs(secs)).await;
            Ok("delayed".to_string())
        }
        other => Err(format!("unsupported mcp_tool: {other}")),
    }
}

fn truncate_output(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...[truncated]", &s[..end])
    }
}
