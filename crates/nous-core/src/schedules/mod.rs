pub mod cron_parser;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub use cron_parser::{Clock, CronExpr, MockClock, SystemClock};

use crate::error::NousError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub trigger_at: Option<i64>,
    pub timezone: String,
    pub enabled: bool,
    pub action_type: String,
    pub action_payload: String,
    pub desired_outcome: Option<String>,
    pub max_retries: i32,
    pub timeout_secs: Option<i32>,
    pub max_output_bytes: i32,
    pub max_runs: i32,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Schedule {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            cron_expr: row.try_get("cron_expr")?,
            trigger_at: row.try_get("trigger_at")?,
            timezone: row.try_get("timezone")?,
            enabled: row.try_get::<i32, _>("enabled")? != 0,
            action_type: row.try_get("action_type")?,
            action_payload: row.try_get("action_payload")?,
            desired_outcome: row.try_get("desired_outcome")?,
            max_retries: row.try_get("max_retries")?,
            timeout_secs: row.try_get("timeout_secs")?,
            max_output_bytes: row.try_get("max_output_bytes")?,
            max_runs: row.try_get("max_runs")?,
            last_run_at: row.try_get("last_run_at")?,
            next_run_at: row.try_get("next_run_at")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRun {
    pub id: String,
    pub schedule_id: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub attempt: i32,
    pub duration_ms: Option<i64>,
}

impl ScheduleRun {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            schedule_id: row.try_get("schedule_id")?,
            started_at: row.try_get("started_at")?,
            finished_at: row.try_get("finished_at")?,
            status: row.try_get("status")?,
            exit_code: row.try_get("exit_code")?,
            output: row.try_get("output")?,
            error: row.try_get("error")?,
            attempt: row.try_get("attempt")?,
            duration_ms: row.try_get("duration_ms")?,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_schedule(
    pool: &SqlitePool,
    name: &str,
    cron_expr: &str,
    trigger_at: Option<i64>,
    timezone: Option<&str>,
    action_type: &str,
    action_payload: &str,
    desired_outcome: Option<&str>,
    max_retries: Option<i32>,
    timeout_secs: Option<i32>,
    max_output_bytes: Option<i32>,
    max_runs: Option<i32>,
    clock: &dyn Clock,
) -> Result<Schedule, NousError> {
    if name.trim().is_empty() {
        return Err(NousError::Validation(
            "schedule name cannot be empty".into(),
        ));
    }

    if cron_expr != "@once" {
        CronExpr::parse(cron_expr)?;
    }

    let valid_actions = ["mcp_tool", "shell", "http"];
    if !valid_actions.contains(&action_type) {
        return Err(NousError::Validation(format!(
            "invalid action_type: {action_type}"
        )));
    }

    let id = Uuid::now_v7().to_string();
    let timezone = timezone.unwrap_or("UTC");
    let max_retries = max_retries.unwrap_or(3);
    let max_output_bytes = max_output_bytes.unwrap_or(65536);
    let max_runs = max_runs.unwrap_or(100);

    let next_run_at = compute_next_run(cron_expr, trigger_at, clock.now_utc());

    sqlx::query(
        "INSERT INTO schedules (id, name, cron_expr, trigger_at, timezone, action_type, action_payload, \
         desired_outcome, max_retries, timeout_secs, max_output_bytes, max_runs, next_run_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(cron_expr)
    .bind(trigger_at)
    .bind(timezone)
    .bind(action_type)
    .bind(action_payload)
    .bind(desired_outcome)
    .bind(max_retries)
    .bind(timeout_secs)
    .bind(max_output_bytes)
    .bind(max_runs)
    .bind(next_run_at)
    .execute(pool)
    .await?;

    let row = sqlx::query("SELECT * FROM schedules WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;

    Schedule::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn get_schedule(pool: &SqlitePool, id: &str) -> Result<Schedule, NousError> {
    let row = sqlx::query("SELECT * FROM schedules WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("schedule '{id}' not found")))?;
    Schedule::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_schedules(
    pool: &SqlitePool,
    enabled: Option<bool>,
    action_type: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Schedule>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let mut sql = String::from("SELECT * FROM schedules");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(e) = enabled {
        conditions.push("enabled = ?".to_string());
        binds.push(if e { "1".to_string() } else { "0".to_string() });
    }

    if let Some(at) = action_type {
        conditions.push("action_type = ?".to_string());
        binds.push(at.to_string());
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    binds.push(limit.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;
    rows.iter()
        .map(Schedule::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_schedule(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    cron_expr: Option<&str>,
    trigger_at: Option<Option<i64>>,
    enabled: Option<bool>,
    action_type: Option<&str>,
    action_payload: Option<&str>,
    desired_outcome: Option<Option<&str>>,
    max_retries: Option<i32>,
    timeout_secs: Option<Option<i32>>,
    max_runs: Option<i32>,
    clock: &dyn Clock,
) -> Result<Schedule, NousError> {
    let existing = get_schedule(pool, id).await?;

    if let Some(expr) = cron_expr {
        if expr != "@once" {
            CronExpr::parse(expr)?;
        }
    }

    let final_cron = cron_expr.unwrap_or(&existing.cron_expr);
    let final_trigger_at = match trigger_at {
        Some(t) => t,
        None => existing.trigger_at,
    };
    let final_enabled = enabled.unwrap_or(existing.enabled);

    let mut sets: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(n) = name {
        sets.push("name = ?".to_string());
        binds.push(n.to_string());
    }
    if let Some(c) = cron_expr {
        sets.push("cron_expr = ?".to_string());
        binds.push(c.to_string());
    }
    if let Some(t) = trigger_at {
        if let Some(ts) = t {
            sets.push("trigger_at = ?".to_string());
            binds.push(ts.to_string());
        } else {
            sets.push("trigger_at = NULL".to_string());
        }
    }
    if let Some(e) = enabled {
        sets.push("enabled = ?".to_string());
        binds.push(if e { "1".to_string() } else { "0".to_string() });
    }
    if let Some(at) = action_type {
        let valid_actions = ["mcp_tool", "shell", "http"];
        if !valid_actions.contains(&at) {
            return Err(NousError::Validation(format!("invalid action_type: {at}")));
        }
        sets.push("action_type = ?".to_string());
        binds.push(at.to_string());
    }
    if let Some(ap) = action_payload {
        sets.push("action_payload = ?".to_string());
        binds.push(ap.to_string());
    }
    if let Some(d) = desired_outcome {
        if let Some(val) = d {
            sets.push("desired_outcome = ?".to_string());
            binds.push(val.to_string());
        } else {
            sets.push("desired_outcome = NULL".to_string());
        }
    }
    if let Some(mr) = max_retries {
        sets.push("max_retries = ?".to_string());
        binds.push(mr.to_string());
    }
    if let Some(ts) = timeout_secs {
        if let Some(val) = ts {
            sets.push("timeout_secs = ?".to_string());
            binds.push(val.to_string());
        } else {
            sets.push("timeout_secs = NULL".to_string());
        }
    }
    if let Some(mr) = max_runs {
        sets.push("max_runs = ?".to_string());
        binds.push(mr.to_string());
    }

    if final_enabled {
        let next = compute_next_run(final_cron, final_trigger_at, clock.now_utc());
        if let Some(n) = next {
            sets.push("next_run_at = ?".to_string());
            binds.push(n.to_string());
        } else {
            sets.push("next_run_at = NULL".to_string());
        }
    } else {
        sets.push("next_run_at = NULL".to_string());
    }

    sets.push("updated_at = strftime('%s','now')".to_string());

    if sets.is_empty() {
        return get_schedule(pool, id).await;
    }

    let sql = format!("UPDATE schedules SET {} WHERE id = ?", sets.join(", "));
    binds.push(id.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    query.execute(pool).await?;

    get_schedule(pool, id).await
}

pub async fn delete_schedule(pool: &SqlitePool, id: &str) -> Result<(), NousError> {
    let result = sqlx::query("DELETE FROM schedules WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("schedule '{id}' not found")));
    }
    Ok(())
}

pub async fn list_runs(
    pool: &SqlitePool,
    schedule_id: &str,
    status: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<ScheduleRun>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let (sql, binds) = if let Some(s) = status {
        (
            "SELECT * FROM schedule_runs WHERE schedule_id = ? AND status = ? ORDER BY started_at DESC LIMIT ?".to_string(),
            vec![schedule_id.to_string(), s.to_string(), limit.to_string()],
        )
    } else {
        (
            "SELECT * FROM schedule_runs WHERE schedule_id = ? ORDER BY started_at DESC LIMIT ?"
                .to_string(),
            vec![schedule_id.to_string(), limit.to_string()],
        )
    };

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;
    rows.iter()
        .map(ScheduleRun::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

#[allow(clippy::too_many_arguments)]
pub async fn record_run(
    pool: &SqlitePool,
    schedule_id: &str,
    started_at: i64,
    finished_at: i64,
    status: &str,
    exit_code: Option<i32>,
    output: Option<&str>,
    error: Option<&str>,
    attempt: i32,
) -> Result<ScheduleRun, NousError> {
    let id = Uuid::now_v7().to_string();
    let duration_ms = (finished_at - started_at) * 1000;

    sqlx::query(
        "INSERT INTO schedule_runs (id, schedule_id, started_at, finished_at, status, exit_code, output, error, attempt, duration_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(schedule_id)
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(exit_code)
    .bind(output)
    .bind(error)
    .bind(attempt)
    .bind(duration_ms)
    .execute(pool)
    .await?;

    sqlx::query("UPDATE schedules SET last_run_at = ? WHERE id = ?")
        .bind(finished_at)
        .bind(schedule_id)
        .execute(pool)
        .await?;

    // Purge runs exceeding max_runs
    let max_runs: Option<i32> = sqlx::query_scalar("SELECT max_runs FROM schedules WHERE id = ?")
        .bind(schedule_id)
        .fetch_optional(pool)
        .await?;

    if let Some(max) = max_runs {
        sqlx::query(
            "DELETE FROM schedule_runs WHERE id IN (\
             SELECT id FROM schedule_runs WHERE schedule_id = ? \
             ORDER BY started_at DESC LIMIT -1 OFFSET ?)",
        )
        .bind(schedule_id)
        .bind(max)
        .execute(pool)
        .await?;
    }

    let row = sqlx::query("SELECT * FROM schedule_runs WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await?;

    ScheduleRun::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn schedule_health(pool: &SqlitePool) -> Result<serde_json::Value, NousError> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schedules")
        .fetch_one(pool)
        .await?;

    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schedules WHERE enabled = 1")
        .fetch_one(pool)
        .await?;

    let failing: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT s.id) FROM schedules s \
         JOIN schedule_runs r ON r.schedule_id = s.id \
         WHERE r.status = 'failed' \
         AND r.started_at = (SELECT MAX(r2.started_at) FROM schedule_runs r2 WHERE r2.schedule_id = s.id)",
    )
    .fetch_one(pool)
    .await?;

    let next_upcoming: Option<i64> = sqlx::query_scalar(
        "SELECT MIN(next_run_at) FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;

    Ok(serde_json::json!({
        "total": total,
        "active": active,
        "failing": failing,
        "next_upcoming": next_upcoming,
    }))
}

pub async fn list_due_schedules(pool: &SqlitePool, now: i64) -> Result<Vec<Schedule>, NousError> {
    let rows = sqlx::query("SELECT * FROM schedules WHERE enabled = 1 AND next_run_at <= ?")
        .bind(now)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(Schedule::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn advance_next_run_at(
    pool: &SqlitePool,
    id: &str,
    clock: &dyn Clock,
) -> Result<Option<i64>, NousError> {
    let schedule = get_schedule(pool, id).await?;

    if schedule.cron_expr.starts_with("@once") {
        if let Some(t) = schedule.trigger_at {
            if t <= clock.now_utc() {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
    }

    let parsed = CronExpr::parse(&schedule.cron_expr)?;
    let next = parsed.next_run(clock.now_utc());

    match next {
        Some(ts) => {
            let now = clock.now_utc();
            sqlx::query("UPDATE schedules SET next_run_at = ?, updated_at = ? WHERE id = ?")
                .bind(ts)
                .bind(now)
                .bind(id)
                .execute(pool)
                .await?;
            Ok(Some(ts))
        }
        None => Ok(None),
    }
}

pub async fn mark_stale_runs_failed(pool: &SqlitePool) -> Result<u64, NousError> {
    let result = sqlx::query(
        "UPDATE schedule_runs SET status = 'failed', error = 'process restarted' WHERE status = 'running'",
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

fn compute_next_run(cron_expr: &str, trigger_at: Option<i64>, now: i64) -> Option<i64> {
    if let Some(t) = trigger_at {
        if t > now {
            return Some(t);
        }
        return None;
    }

    if cron_expr == "@once" {
        return None;
    }

    let parsed = CronExpr::parse(cron_expr).ok()?;
    parsed.next_run(now)
}
