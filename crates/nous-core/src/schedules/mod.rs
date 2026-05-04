pub mod cron_parser;

use chrono::{DateTime, TimeZone, Utc};
use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, Set, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use cron_parser::{Clock, CronExpr, MockClock, SystemClock};

use crate::entities::schedule_runs as run_entity;
use crate::entities::schedules as sched_entity;
use crate::error::NousError;

pub fn ts_to_iso(ts: i64) -> Result<String, NousError> {
    Utc.timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .ok_or_else(|| NousError::Validation(format!("invalid timestamp: {ts}")))
}

pub fn iso_to_ts(iso: &str) -> Result<i64, NousError> {
    DateTime::parse_from_rfc3339(iso)
        .map(|dt| dt.timestamp())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(iso, "%Y-%m-%dT%H:%M:%S%.fZ")
                .map(|ndt| ndt.and_utc().timestamp())
        })
        .map_err(|_| NousError::Validation(format!("invalid ISO timestamp: {iso}")))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    pub id: String,
    pub name: String,
    pub cron_expr: String,
    pub trigger_at: Option<String>,
    pub timezone: String,
    pub enabled: bool,
    pub action_type: String,
    pub action_payload: String,
    pub desired_outcome: Option<String>,
    pub max_retries: i32,
    pub timeout_secs: Option<i32>,
    pub max_output_bytes: i32,
    pub max_runs: i32,
    pub last_run_at: Option<String>,
    pub next_run_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Schedule {
    fn from_model(m: sched_entity::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            cron_expr: m.cron_expr,
            trigger_at: m.trigger_at,
            timezone: m.timezone,
            enabled: m.enabled,
            action_type: m.action_type,
            action_payload: m.action_payload,
            desired_outcome: m.desired_outcome,
            max_retries: m.max_retries,
            timeout_secs: m.timeout_secs,
            max_output_bytes: m.max_output_bytes,
            max_runs: m.max_runs,
            last_run_at: m.last_run_at,
            next_run_at: m.next_run_at,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }

    pub fn trigger_at_ts(&self) -> Result<Option<i64>, NousError> {
        self.trigger_at.as_deref().map(iso_to_ts).transpose()
    }

    pub fn next_run_at_ts(&self) -> Result<Option<i64>, NousError> {
        self.next_run_at.as_deref().map(iso_to_ts).transpose()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRun {
    pub id: String,
    pub schedule_id: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub attempt: i32,
    pub duration_ms: Option<i64>,
}

impl ScheduleRun {
    fn from_model(m: run_entity::Model) -> Self {
        Self {
            id: m.id,
            schedule_id: m.schedule_id,
            started_at: m.started_at,
            finished_at: m.finished_at,
            status: m.status,
            exit_code: m.exit_code,
            output: m.output,
            error: m.error,
            attempt: m.attempt,
            duration_ms: m.duration_ms,
        }
    }
}

pub struct CreateScheduleParams<'a> {
    pub db: &'a DatabaseConnection,
    pub name: &'a str,
    pub cron_expr: &'a str,
    pub trigger_at: Option<i64>,
    pub timezone: Option<&'a str>,
    pub action_type: &'a str,
    pub action_payload: &'a str,
    pub desired_outcome: Option<&'a str>,
    pub max_retries: Option<i32>,
    pub timeout_secs: Option<i32>,
    pub max_output_bytes: Option<i32>,
    pub max_runs: Option<i32>,
    pub clock: &'a dyn Clock,
}

pub async fn create_schedule(params: CreateScheduleParams<'_>) -> Result<Schedule, NousError> {
    let CreateScheduleParams {
        db,
        name,
        cron_expr,
        trigger_at,
        timezone,
        action_type,
        action_payload,
        desired_outcome,
        max_retries,
        timeout_secs,
        max_output_bytes,
        max_runs,
        clock,
    } = params;
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

    let next_run_at = compute_next_run(cron_expr, trigger_at, clock.now_utc())
        .map(ts_to_iso)
        .transpose()?;
    let now_iso = ts_to_iso(clock.now_utc())?;

    let model = sched_entity::ActiveModel {
        id: Set(id.clone()),
        name: Set(name.to_string()),
        cron_expr: Set(cron_expr.to_string()),
        trigger_at: Set(trigger_at.map(ts_to_iso).transpose()?),
        timezone: Set(timezone.to_string()),
        enabled: Set(true),
        action_type: Set(action_type.to_string()),
        action_payload: Set(action_payload.to_string()),
        desired_outcome: Set(desired_outcome.map(String::from)),
        max_retries: Set(max_retries),
        timeout_secs: Set(timeout_secs),
        max_output_bytes: Set(max_output_bytes),
        max_runs: Set(max_runs),
        last_run_at: Set(None),
        next_run_at: Set(next_run_at),
        created_at: Set(now_iso.clone()),
        updated_at: Set(now_iso),
    };

    sched_entity::Entity::insert(model).exec(db).await?;

    get_schedule(db, &id).await
}

pub async fn get_schedule(db: &DatabaseConnection, id: &str) -> Result<Schedule, NousError> {
    let model = sched_entity::Entity::find_by_id(id).one(db).await?;

    match model {
        Some(m) => Ok(Schedule::from_model(m)),
        None => Err(NousError::NotFound(format!("schedule '{id}' not found"))),
    }
}

pub async fn list_schedules(
    db: &DatabaseConnection,
    enabled: Option<bool>,
    action_type: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Schedule>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let mut sql = String::from("SELECT * FROM schedules");
    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<sea_orm::Value> = Vec::new();

    if let Some(e) = enabled {
        conditions.push("enabled = ?".to_string());
        values.push(if e { 1i32.into() } else { 0i32.into() });
    }

    if let Some(at) = action_type {
        conditions.push("action_type = ?".to_string());
        values.push(at.to_string().into());
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    // limit is capped by caller; safe to cast to i32
    values.push(limit.cast_signed().into());

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;

    let mut schedules = Vec::new();
    for row in rows {
        let m = <sched_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        schedules.push(Schedule::from_model(m));
    }
    Ok(schedules)
}

pub struct UpdateScheduleParams<'a> {
    pub db: &'a DatabaseConnection,
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub cron_expr: Option<&'a str>,
    pub trigger_at: Option<Option<i64>>,
    pub enabled: Option<bool>,
    pub action_type: Option<&'a str>,
    pub action_payload: Option<&'a str>,
    pub desired_outcome: Option<Option<&'a str>>,
    pub max_retries: Option<i32>,
    pub timeout_secs: Option<Option<i32>>,
    pub max_runs: Option<i32>,
    pub clock: &'a dyn Clock,
}

pub async fn update_schedule(params: UpdateScheduleParams<'_>) -> Result<Schedule, NousError> {
    let UpdateScheduleParams {
        db,
        id,
        name,
        cron_expr,
        trigger_at,
        enabled,
        action_type,
        action_payload,
        desired_outcome,
        max_retries,
        timeout_secs,
        max_runs,
        clock,
    } = params;
    let existing = get_schedule(db, id).await?;

    if let Some(expr) = cron_expr {
        if expr != "@once" {
            CronExpr::parse(expr)?;
        }
    }

    let final_cron = cron_expr.unwrap_or(&existing.cron_expr);
    let final_trigger_at = match trigger_at {
        Some(t) => t,
        None => existing.trigger_at_ts()?,
    };
    let final_enabled = enabled.unwrap_or(existing.enabled);

    let mut sets: Vec<String> = Vec::new();
    let mut values: Vec<sea_orm::Value> = Vec::new();

    if let Some(n) = name {
        sets.push("name = ?".to_string());
        values.push(n.to_string().into());
    }
    if let Some(c) = cron_expr {
        sets.push("cron_expr = ?".to_string());
        values.push(c.to_string().into());
    }
    if let Some(t) = trigger_at {
        if let Some(ts) = t {
            sets.push("trigger_at = ?".to_string());
            values.push(ts_to_iso(ts)?.into());
        } else {
            sets.push("trigger_at = NULL".to_string());
        }
    }
    if let Some(e) = enabled {
        sets.push("enabled = ?".to_string());
        values.push(if e { 1i32.into() } else { 0i32.into() });
    }
    if let Some(at) = action_type {
        let valid_actions = ["mcp_tool", "shell", "http"];
        if !valid_actions.contains(&at) {
            return Err(NousError::Validation(format!("invalid action_type: {at}")));
        }
        sets.push("action_type = ?".to_string());
        values.push(at.to_string().into());
    }
    if let Some(ap) = action_payload {
        sets.push("action_payload = ?".to_string());
        values.push(ap.to_string().into());
    }
    if let Some(d) = desired_outcome {
        if let Some(val) = d {
            sets.push("desired_outcome = ?".to_string());
            values.push(val.to_string().into());
        } else {
            sets.push("desired_outcome = NULL".to_string());
        }
    }
    if let Some(mr) = max_retries {
        sets.push("max_retries = ?".to_string());
        values.push(mr.into());
    }
    if let Some(ts) = timeout_secs {
        if let Some(val) = ts {
            sets.push("timeout_secs = ?".to_string());
            values.push(val.into());
        } else {
            sets.push("timeout_secs = NULL".to_string());
        }
    }
    if let Some(mr) = max_runs {
        sets.push("max_runs = ?".to_string());
        values.push(mr.into());
    }

    if final_enabled {
        let next = compute_next_run(final_cron, final_trigger_at, clock.now_utc());
        if let Some(n) = next {
            sets.push("next_run_at = ?".to_string());
            values.push(ts_to_iso(n)?.into());
        } else {
            sets.push("next_run_at = NULL".to_string());
        }
    } else {
        sets.push("next_run_at = NULL".to_string());
    }

    sets.push("updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')".to_string());

    if sets.is_empty() {
        return get_schedule(db, id).await;
    }

    let sql = format!("UPDATE schedules SET {} WHERE id = ?", sets.join(", "));
    values.push(id.to_string().into());

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        values,
    ))
    .await?;

    get_schedule(db, id).await
}

pub async fn delete_schedule(db: &DatabaseConnection, id: &str) -> Result<(), NousError> {
    let result = sched_entity::Entity::delete_by_id(id).exec(db).await?;

    if result.rows_affected == 0 {
        return Err(NousError::NotFound(format!("schedule '{id}' not found")));
    }
    Ok(())
}

pub async fn list_runs(
    db: &DatabaseConnection,
    schedule_id: &str,
    status: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<ScheduleRun>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    let mut sql = String::from("SELECT * FROM schedule_runs WHERE schedule_id = ?");
    let mut values: Vec<sea_orm::Value> = vec![schedule_id.to_string().into()];

    if let Some(s) = status {
        sql.push_str(" AND status = ?");
        values.push(s.to_string().into());
    }

    sql.push_str(" ORDER BY started_at DESC LIMIT ?");
    // limit is capped by caller; safe to cast to i32
    values.push(limit.cast_signed().into());

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            &sql,
            values,
        ))
        .await?;

    let mut runs = Vec::new();
    for row in rows {
        let m = <run_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        runs.push(ScheduleRun::from_model(m));
    }
    Ok(runs)
}

pub struct RecordRunParams<'a> {
    pub db: &'a DatabaseConnection,
    pub schedule_id: &'a str,
    pub started_at: i64,
    pub finished_at: i64,
    pub status: &'a str,
    pub exit_code: Option<i32>,
    pub output: Option<&'a str>,
    pub error: Option<&'a str>,
    pub attempt: i32,
}

pub async fn record_run(params: RecordRunParams<'_>) -> Result<ScheduleRun, NousError> {
    let RecordRunParams {
        db,
        schedule_id,
        started_at,
        finished_at,
        status,
        exit_code,
        output,
        error,
        attempt,
    } = params;
    let id = Uuid::now_v7().to_string();
    let duration_ms = (finished_at - started_at) * 1000;

    let started_at_iso = ts_to_iso(started_at)?;
    let finished_at_iso = ts_to_iso(finished_at)?;

    let model = run_entity::ActiveModel {
        id: Set(id.clone()),
        schedule_id: Set(schedule_id.to_string()),
        started_at: Set(started_at_iso),
        finished_at: Set(Some(finished_at_iso.clone())),
        status: Set(status.to_string()),
        exit_code: Set(exit_code),
        output: Set(output.map(String::from)),
        error: Set(error.map(String::from)),
        attempt: Set(attempt),
        duration_ms: Set(Some(duration_ms)),
    };

    run_entity::Entity::insert(model).exec(db).await?;

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE schedules SET last_run_at = ? WHERE id = ?",
        [finished_at_iso.into(), schedule_id.to_string().into()],
    ))
    .await?;

    let schedule = sched_entity::Entity::find_by_id(schedule_id)
        .one(db)
        .await?;

    if let Some(s) = schedule {
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "DELETE FROM schedule_runs WHERE id IN (\
             SELECT id FROM schedule_runs WHERE schedule_id = ? \
             ORDER BY started_at DESC LIMIT -1 OFFSET ?)",
            [schedule_id.to_string().into(), s.max_runs.into()],
        ))
        .await?;
    }

    let run_model = run_entity::Entity::find_by_id(&id).one(db).await?;
    match run_model {
        Some(m) => Ok(ScheduleRun::from_model(m)),
        None => Err(NousError::NotFound(format!(
            "schedule run '{id}' not found"
        ))),
    }
}

pub async fn schedule_health(db: &DatabaseConnection) -> Result<serde_json::Value, NousError> {
    let total_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) as cnt FROM schedules",
            [],
        ))
        .await?;
    let total: i64 = total_row
        .map_or(0, |r| r.try_get_by::<i64, _>("cnt").unwrap_or(0));

    let active_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) as cnt FROM schedules WHERE enabled = 1",
            [],
        ))
        .await?;
    let active: i64 = active_row
        .map_or(0, |r| r.try_get_by::<i64, _>("cnt").unwrap_or(0));

    let failing_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(DISTINCT s.id) as cnt FROM schedules s \
             JOIN schedule_runs r ON r.schedule_id = s.id \
             WHERE r.status = 'failed' \
             AND r.started_at = (SELECT MAX(r2.started_at) FROM schedule_runs r2 WHERE r2.schedule_id = s.id)",
            [],
        ))
        .await?;
    let failing: i64 = failing_row
        .map_or(0, |r| r.try_get_by::<i64, _>("cnt").unwrap_or(0));

    let next_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT MIN(next_run_at) as val FROM schedules WHERE enabled = 1 AND next_run_at IS NOT NULL",
            [],
        ))
        .await?;
    let next_upcoming: Option<String> =
        next_row.and_then(|r| r.try_get_by::<Option<String>, _>("val").ok().flatten());

    Ok(serde_json::json!({
        "total": total,
        "active": active,
        "failing": failing,
        "next_upcoming": next_upcoming,
    }))
}

pub async fn list_due_schedules(
    db: &DatabaseConnection,
    now: i64,
) -> Result<Vec<Schedule>, NousError> {
    let now_iso = ts_to_iso(now)?;
    let models = sched_entity::Entity::find()
        .filter(sched_entity::Column::Enabled.eq(true))
        .filter(sched_entity::Column::NextRunAt.lte(now_iso))
        .all(db)
        .await?;

    Ok(models.into_iter().map(Schedule::from_model).collect())
}

pub async fn advance_next_run_at(
    db: &DatabaseConnection,
    id: &str,
    clock: &dyn Clock,
) -> Result<Option<String>, NousError> {
    let schedule = get_schedule(db, id).await?;

    if schedule.cron_expr.starts_with("@once") {
        if let Some(trigger_ts) = schedule.trigger_at_ts()? {
            if trigger_ts <= clock.now_utc() {
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
            let next_iso = ts_to_iso(ts)?;
            let now_iso = ts_to_iso(clock.now_utc())?;
            db.execute(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "UPDATE schedules SET next_run_at = ?, updated_at = ? WHERE id = ?",
                [next_iso.clone().into(), now_iso.into(), id.to_string().into()],
            ))
            .await?;
            Ok(Some(next_iso))
        }
        None => Ok(None),
    }
}

pub async fn mark_stale_runs_failed(db: &DatabaseConnection) -> Result<u64, NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE schedule_runs SET status = 'failed', error = 'process restarted' WHERE status = 'running'",
            [],
        ))
        .await?;
    Ok(result.rows_affected())
}

fn compute_next_run(cron_expr: &str, trigger_at: Option<i64>, now: i64) -> Option<i64> {
    if let Some(t) = trigger_at {
        if t >= now {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iso_to_ts_valid_rfc3339() {
        let result = iso_to_ts("2024-01-01T00:00:00Z");
        assert_eq!(result.unwrap(), 1704067200);
    }

    #[test]
    fn test_iso_to_ts_valid_naive() {
        let result = iso_to_ts("2024-01-01T00:00:00.000Z");
        assert_eq!(result.unwrap(), 1704067200);
    }

    #[test]
    fn test_iso_to_ts_invalid() {
        let result = iso_to_ts("not-a-date");
        assert!(result.is_err());
    }

    #[test]
    fn test_ts_to_iso_valid() {
        let result = ts_to_iso(1704067200);
        assert_eq!(result.unwrap(), "2024-01-01T00:00:00.000Z");
    }

    #[test]
    fn test_ts_to_iso_invalid() {
        let result = ts_to_iso(i64::MAX);
        assert!(result.is_err());
    }
}
