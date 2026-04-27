use chrono::Utc;
use nous_shared::{NousError, Result};
use rusqlite::{Connection, params};
use serde_json;

use crate::cron_parser::CronExpr;
use crate::types::{ActionType, RunPatch, RunStatus, Schedule, SchedulePatch, ScheduleRun};

pub struct ScheduleDb;

impl ScheduleDb {
    pub fn create_on(conn: &Connection, schedule: &Schedule) -> Result<String> {
        CronExpr::parse(&schedule.cron_expr)
            .map_err(|e| NousError::Internal(format!("invalid cron expression: {e}")))?;

        let id = if schedule.id.is_empty() {
            uuid::Uuid::now_v7().to_string()
        } else {
            schedule.id.clone()
        };

        let tz: chrono_tz::Tz = schedule
            .timezone
            .parse()
            .map_err(|_| NousError::Internal(format!("invalid timezone: {}", schedule.timezone)))?;

        let next_run_at = schedule.next_run_at.or_else(|| {
            let now = Utc::now().with_timezone(&tz);
            CronExpr::parse(&schedule.cron_expr)
                .ok()
                .and_then(|expr| expr.next_run(now))
                .map(|dt| dt.timestamp())
        });

        conn.execute(
            "INSERT INTO schedules (id, name, cron_expr, timezone, enabled, action_type,
                action_payload, desired_outcome, max_retries, timeout_secs, max_output_bytes,
                max_runs, next_run_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                schedule.name,
                schedule.cron_expr,
                schedule.timezone,
                schedule.enabled as i64,
                schedule.action_type.to_string(),
                schedule.action_payload,
                schedule.desired_outcome,
                schedule.max_retries,
                schedule.timeout_secs,
                schedule.max_output_bytes,
                schedule.max_runs,
                next_run_at,
            ],
        )?;

        Ok(id)
    }

    pub fn update_on(conn: &Connection, id: &str, patch: &SchedulePatch) -> Result<bool> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schedules WHERE id = ?1)",
            params![id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }

        if let Some(ref expr) = patch.cron_expr {
            CronExpr::parse(expr)
                .map_err(|e| NousError::Internal(format!("invalid cron expression: {e}")))?;
        }

        let mut sets = vec!["updated_at = strftime('%s', 'now')".to_owned()];
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref name) = patch.name {
            sets.push(format!("name = ?{}", param_values.len() + 1));
            param_values.push(Box::new(name.clone()));
        }
        if let Some(ref cron_expr) = patch.cron_expr {
            sets.push(format!("cron_expr = ?{}", param_values.len() + 1));
            param_values.push(Box::new(cron_expr.clone()));
        }
        if let Some(ref action_payload) = patch.action_payload {
            sets.push(format!("action_payload = ?{}", param_values.len() + 1));
            param_values.push(Box::new(action_payload.clone()));
        }
        if let Some(enabled) = patch.enabled {
            sets.push(format!("enabled = ?{}", param_values.len() + 1));
            param_values.push(Box::new(enabled as i64));
        }
        if let Some(max_retries) = patch.max_retries {
            sets.push(format!("max_retries = ?{}", param_values.len() + 1));
            param_values.push(Box::new(max_retries));
        }
        if let Some(timeout_secs) = patch.timeout_secs {
            sets.push(format!("timeout_secs = ?{}", param_values.len() + 1));
            param_values.push(Box::new(timeout_secs));
        }
        if let Some(ref desired_outcome) = patch.desired_outcome {
            sets.push(format!("desired_outcome = ?{}", param_values.len() + 1));
            param_values.push(Box::new(desired_outcome.clone()));
        }

        let idx = param_values.len() + 1;
        param_values.push(Box::new(id.to_string()));

        let sql = format!(
            "UPDATE schedules SET {} WHERE id = ?{}",
            sets.join(", "),
            idx
        );
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_ref.as_slice())?;

        Ok(true)
    }

    pub fn delete_on(conn: &Connection, id: &str) -> Result<bool> {
        let changed = conn.execute("DELETE FROM schedules WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    pub fn record_run_on(conn: &Connection, run: &ScheduleRun) -> Result<String> {
        let id = if run.id.is_empty() {
            uuid::Uuid::now_v7().to_string()
        } else {
            run.id.clone()
        };

        conn.execute(
            "INSERT INTO schedule_runs (id, schedule_id, started_at, finished_at, status,
                exit_code, output, error, attempt, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                run.schedule_id,
                run.started_at,
                run.finished_at,
                run.status.to_string(),
                run.exit_code,
                run.output,
                run.error,
                run.attempt,
                run.duration_ms,
            ],
        )?;

        let max_runs: i64 = conn
            .query_row(
                "SELECT max_runs FROM schedules WHERE id = ?1",
                params![run.schedule_id],
                |row| row.get(0),
            )
            .unwrap_or(100);

        conn.execute(
            "DELETE FROM schedule_runs WHERE schedule_id = ?1 AND id NOT IN (
                SELECT id FROM schedule_runs WHERE schedule_id = ?1
                ORDER BY started_at DESC LIMIT ?2
            )",
            params![run.schedule_id, max_runs],
        )?;

        Ok(id)
    }

    pub fn update_run_on(conn: &Connection, id: &str, patch: &RunPatch) -> Result<bool> {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schedule_runs WHERE id = ?1)",
            params![id],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }

        let mut sets = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(finished_at) = patch.finished_at {
            sets.push(format!("finished_at = ?{}", param_values.len() + 1));
            param_values.push(Box::new(finished_at));
        }
        if let Some(ref status) = patch.status {
            sets.push(format!("status = ?{}", param_values.len() + 1));
            param_values.push(Box::new(status.to_string()));
        }
        if let Some(exit_code) = patch.exit_code {
            sets.push(format!("exit_code = ?{}", param_values.len() + 1));
            param_values.push(Box::new(exit_code));
        }
        if let Some(ref output) = patch.output {
            sets.push(format!("output = ?{}", param_values.len() + 1));
            param_values.push(Box::new(output.clone()));
        }
        if let Some(ref error) = patch.error {
            sets.push(format!("error = ?{}", param_values.len() + 1));
            param_values.push(Box::new(error.clone()));
        }
        if let Some(duration_ms) = patch.duration_ms {
            sets.push(format!("duration_ms = ?{}", param_values.len() + 1));
            param_values.push(Box::new(duration_ms));
        }

        if sets.is_empty() {
            return Ok(true);
        }

        let idx = param_values.len() + 1;
        param_values.push(Box::new(id.to_string()));

        let sql = format!(
            "UPDATE schedule_runs SET {} WHERE id = ?{}",
            sets.join(", "),
            idx
        );
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        conn.execute(&sql, params_ref.as_slice())?;

        Ok(true)
    }

    pub fn compute_next_run_on(conn: &Connection, id: &str) -> Result<()> {
        let (cron_expr, timezone): (String, String) = conn
            .query_row(
                "SELECT cron_expr, timezone FROM schedules WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    NousError::NotFound(format!("schedule not found: {id}"))
                }
                other => NousError::Sqlite(other),
            })?;

        let tz: chrono_tz::Tz = timezone
            .parse()
            .map_err(|_| NousError::Internal(format!("invalid timezone: {timezone}")))?;

        let now = Utc::now().with_timezone(&tz);
        let expr = CronExpr::parse(&cron_expr)
            .map_err(|e| NousError::Internal(format!("invalid cron expression: {e}")))?;
        let next = expr.next_run(now).map(|dt| dt.timestamp());

        conn.execute(
            "UPDATE schedules SET next_run_at = ?1, updated_at = strftime('%s', 'now') WHERE id = ?2",
            params![next, id],
        )?;

        Ok(())
    }

    pub fn get(conn: &Connection, id: &str) -> Result<Option<Schedule>> {
        match conn.query_row(
            "SELECT id, name, cron_expr, timezone, enabled, action_type, action_payload,
                    desired_outcome, max_retries, timeout_secs, max_output_bytes, max_runs,
                    next_run_at, created_at, updated_at
             FROM schedules WHERE id = ?1",
            params![id],
            Self::row_to_schedule,
        ) {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list(
        conn: &Connection,
        enabled_filter: Option<bool>,
        action_type_filter: Option<&ActionType>,
        limit: Option<usize>,
    ) -> Result<Vec<Schedule>> {
        let limit = limit.unwrap_or(50) as i64;
        let mut sql = "SELECT id, name, cron_expr, timezone, enabled, action_type, action_payload,
                               desired_outcome, max_retries, timeout_secs, max_output_bytes, max_runs,
                               next_run_at, created_at, updated_at
                        FROM schedules WHERE 1=1"
            .to_string();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(enabled) = enabled_filter {
            param_values.push(Box::new(enabled as i64));
            sql.push_str(&format!(" AND enabled = ?{}", param_values.len()));
        }
        if let Some(action_type) = action_type_filter {
            param_values.push(Box::new(action_type.to_string()));
            sql.push_str(&format!(" AND action_type = ?{}", param_values.len()));
        }

        param_values.push(Box::new(limit));
        sql.push_str(&format!(
            " ORDER BY next_run_at ASC LIMIT ?{}",
            param_values.len()
        ));

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), Self::row_to_schedule)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn next_pending(conn: &Connection) -> Result<Option<Schedule>> {
        match conn.query_row(
            "SELECT id, name, cron_expr, timezone, enabled, action_type, action_payload,
                    desired_outcome, max_retries, timeout_secs, max_output_bytes, max_runs,
                    next_run_at, created_at, updated_at
             FROM schedules
             WHERE enabled = 1 AND next_run_at IS NOT NULL
             ORDER BY next_run_at ASC LIMIT 1",
            [],
            Self::row_to_schedule,
        ) {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_runs(
        conn: &Connection,
        schedule_id: &str,
        status_filter: Option<&RunStatus>,
        limit: Option<usize>,
    ) -> Result<Vec<ScheduleRun>> {
        let limit = limit.unwrap_or(50) as i64;
        let mut sql = "SELECT id, schedule_id, started_at, finished_at, status,
                               exit_code, output, error, attempt, duration_ms
                        FROM schedule_runs WHERE schedule_id = ?1"
            .to_string();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(schedule_id.to_string())];

        if let Some(status) = status_filter {
            param_values.push(Box::new(status.to_string()));
            sql.push_str(&format!(" AND status = ?{}", param_values.len()));
        }

        param_values.push(Box::new(limit));
        sql.push_str(&format!(
            " ORDER BY started_at DESC LIMIT ?{}",
            param_values.len()
        ));

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), Self::row_to_run)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_run(conn: &Connection, run_id: &str) -> Result<Option<ScheduleRun>> {
        match conn.query_row(
            "SELECT id, schedule_id, started_at, finished_at, status,
                    exit_code, output, error, attempt, duration_ms
             FROM schedule_runs WHERE id = ?1",
            params![run_id],
            Self::row_to_run,
        ) {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn query_runs(
        conn: &Connection,
        schedule_id: Option<&str>,
        status_filter: Option<&RunStatus>,
        since: Option<i64>,
        limit: Option<usize>,
    ) -> Result<Vec<ScheduleRun>> {
        let limit = limit.unwrap_or(50) as i64;
        let mut sql = "SELECT id, schedule_id, started_at, finished_at, status,
                               exit_code, output, error, attempt, duration_ms
                        FROM schedule_runs WHERE 1=1"
            .to_string();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(sid) = schedule_id {
            param_values.push(Box::new(sid.to_string()));
            sql.push_str(&format!(" AND schedule_id = ?{}", param_values.len()));
        }
        if let Some(status) = status_filter {
            param_values.push(Box::new(status.to_string()));
            sql.push_str(&format!(" AND status = ?{}", param_values.len()));
        }
        if let Some(since_ts) = since {
            param_values.push(Box::new(since_ts));
            sql.push_str(&format!(" AND started_at >= ?{}", param_values.len()));
        }

        param_values.push(Box::new(limit));
        sql.push_str(&format!(
            " ORDER BY started_at DESC LIMIT ?{}",
            param_values.len()
        ));

        let mut stmt = conn.prepare(&sql)?;
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let rows = stmt
            .query_map(params_ref.as_slice(), Self::row_to_run)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn health(conn: &Connection) -> Result<serde_json::Value> {
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM schedules", [], |row| row.get(0))?;

        let active: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schedules WHERE enabled = 1",
            [],
            |row| row.get(0),
        )?;

        let failing: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT s.id) FROM schedules s
             INNER JOIN schedule_runs r ON r.schedule_id = s.id
             WHERE r.status IN ('failed', 'timeout')
               AND r.id = (
                   SELECT r2.id FROM schedule_runs r2
                   WHERE r2.schedule_id = s.id
                   ORDER BY r2.started_at DESC LIMIT 1
               )",
            [],
            |row| row.get(0),
        )?;

        let outcome_mismatches: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schedule_runs
             WHERE status = 'failed' AND error LIKE 'outcome mismatch:%'",
            [],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(
            "SELECT id, name, cron_expr, timezone, enabled, action_type, action_payload,
                    desired_outcome, max_retries, timeout_secs, max_output_bytes, max_runs,
                    next_run_at, created_at, updated_at
             FROM schedules
             WHERE enabled = 1 AND next_run_at IS NOT NULL
             ORDER BY next_run_at ASC LIMIT 5",
        )?;
        let upcoming: Vec<Schedule> = stmt
            .query_map([], Self::row_to_schedule)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let upcoming_json: Vec<serde_json::Value> = upcoming
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "next_run_at": s.next_run_at,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "total_schedules": total,
            "active_count": active,
            "failing_count": failing,
            "outcome_mismatches": outcome_mismatches,
            "next_upcoming": upcoming_json,
        }))
    }

    fn row_to_schedule(row: &rusqlite::Row<'_>) -> rusqlite::Result<Schedule> {
        Ok(Schedule {
            id: row.get(0)?,
            name: row.get(1)?,
            cron_expr: row.get(2)?,
            timezone: row.get(3)?,
            enabled: row.get::<_, i64>(4)? != 0,
            action_type: row.get::<_, String>(5)?.parse().map_err(|e: String| {
                rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
            })?,
            action_payload: row.get(6)?,
            desired_outcome: row.get(7)?,
            max_retries: row.get(8)?,
            timeout_secs: row.get(9)?,
            max_output_bytes: row.get(10)?,
            max_runs: row.get(11)?,
            next_run_at: row.get(12)?,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
        })
    }

    fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRun> {
        Ok(ScheduleRun {
            id: row.get(0)?,
            schedule_id: row.get(1)?,
            started_at: row.get(2)?,
            finished_at: row.get(3)?,
            status: row.get::<_, String>(4)?.parse().map_err(|e: String| {
                rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, e.into())
            })?,
            exit_code: row.get(5)?,
            output: row.get(6)?,
            error: row.get(7)?,
            attempt: row.get(8)?,
            duration_ms: row.get(9)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ActionType, RunStatus, Schedule, SchedulePatch, ScheduleRun};
    use nous_shared::sqlite::run_migrations;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        let migrations = crate::db::test_migrations();
        run_migrations(&conn, &migrations).unwrap();
        conn
    }

    fn make_schedule(name: &str, cron: &str) -> Schedule {
        Schedule {
            id: String::new(),
            name: name.to_string(),
            cron_expr: cron.to_string(),
            timezone: "UTC".to_string(),
            enabled: true,
            action_type: ActionType::McpTool,
            action_payload: r#"{"tool":"memory_stats","args":{}}"#.to_string(),
            desired_outcome: None,
            max_retries: 3,
            timeout_secs: Some(300),
            max_output_bytes: 65536,
            max_runs: 100,
            next_run_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn create_and_get_schedule() {
        let conn = setup_db();
        let sched = make_schedule("test-schedule", "*/5 * * * *");
        let id = ScheduleDb::create_on(&conn, &sched).unwrap();

        let got = ScheduleDb::get(&conn, &id).unwrap().unwrap();
        assert_eq!(got.name, "test-schedule");
        assert_eq!(got.cron_expr, "*/5 * * * *");
        assert_eq!(got.timezone, "UTC");
        assert!(got.enabled);
        assert_eq!(got.action_type, ActionType::McpTool);
        assert_eq!(got.max_retries, 3);
        assert!(got.next_run_at.is_some());
    }

    #[test]
    fn update_schedule_recomputes_next_run() {
        let conn = setup_db();
        let sched = make_schedule("hourly", "0 * * * *");
        let id = ScheduleDb::create_on(&conn, &sched).unwrap();

        let before = ScheduleDb::get(&conn, &id).unwrap().unwrap();
        let old_next = before.next_run_at.unwrap();

        let patch = SchedulePatch {
            cron_expr: Some("0 0 * * *".to_string()),
            ..Default::default()
        };
        ScheduleDb::update_on(&conn, &id, &patch).unwrap();
        ScheduleDb::compute_next_run_on(&conn, &id).unwrap();

        let after = ScheduleDb::get(&conn, &id).unwrap().unwrap();
        assert_eq!(after.cron_expr, "0 0 * * *");
        assert_ne!(after.next_run_at.unwrap(), old_next);
    }

    #[test]
    fn delete_cascades_runs() {
        let conn = setup_db();
        let sched = make_schedule("to-delete", "* * * * *");
        let id = ScheduleDb::create_on(&conn, &sched).unwrap();

        let run = ScheduleRun {
            id: String::new(),
            schedule_id: id.clone(),
            started_at: 1000,
            finished_at: Some(1001),
            status: RunStatus::Completed,
            exit_code: Some(0),
            output: Some("ok".to_string()),
            error: None,
            attempt: 1,
            duration_ms: Some(1000),
        };
        let run_id = ScheduleDb::record_run_on(&conn, &run).unwrap();

        ScheduleDb::delete_on(&conn, &id).unwrap();

        assert!(ScheduleDb::get(&conn, &id).unwrap().is_none());
        assert!(ScheduleDb::get_run(&conn, &run_id).unwrap().is_none());
    }

    #[test]
    fn record_run_enforces_max_runs() {
        let conn = setup_db();
        let mut sched = make_schedule("limited", "* * * * *");
        sched.max_runs = 3;
        let id = ScheduleDb::create_on(&conn, &sched).unwrap();

        for i in 0..5 {
            let run = ScheduleRun {
                id: String::new(),
                schedule_id: id.clone(),
                started_at: 1000 + i,
                finished_at: Some(1001 + i),
                status: RunStatus::Completed,
                exit_code: Some(0),
                output: None,
                error: None,
                attempt: 1,
                duration_ms: Some(1000),
            };
            ScheduleDb::record_run_on(&conn, &run).unwrap();
        }

        let runs = ScheduleDb::get_runs(&conn, &id, None, Some(100)).unwrap();
        assert_eq!(runs.len(), 3);
        assert!(runs[0].started_at >= runs[1].started_at);
    }

    #[test]
    fn cron_expr_validation_rejects_invalid() {
        let conn = setup_db();

        let mut sched = make_schedule("bad-cron", "not a cron");
        let result = ScheduleDb::create_on(&conn, &sched);
        assert!(result.is_err());

        sched.cron_expr = "".to_string();
        let result = ScheduleDb::create_on(&conn, &sched);
        assert!(result.is_err());

        sched.cron_expr = "* * *".to_string();
        let result = ScheduleDb::create_on(&conn, &sched);
        assert!(result.is_err());

        sched.cron_expr = "60 * * * *".to_string();
        let result = ScheduleDb::create_on(&conn, &sched);
        assert!(result.is_err());
    }

    #[test]
    fn cron_expr_standard_patterns() {
        let conn = setup_db();

        let patterns = [
            ("every-minute", "* * * * *"),
            ("hourly", "0 * * * *"),
            ("daily", "0 0 * * *"),
            ("monthly", "0 0 1 * *"),
            ("yearly", "0 0 1 1 *"),
        ];

        for (name, cron) in patterns {
            let sched = make_schedule(name, cron);
            let id = ScheduleDb::create_on(&conn, &sched).unwrap();
            let got = ScheduleDb::get(&conn, &id).unwrap().unwrap();
            assert!(
                got.next_run_at.is_some(),
                "next_run_at should be set for {name} ({cron})"
            );
            assert!(
                got.next_run_at.unwrap() > 0,
                "next_run_at should be positive for {name}"
            );
        }
    }

    #[test]
    fn cron_expr_ranges_lists_steps() {
        let conn = setup_db();

        let patterns = [
            ("range", "1-5 * * * *"),
            ("list", "1,3,5 * * * *"),
            ("step-wildcard", "*/5 * * * *"),
            ("step-range", "1-10/2 * * * *"),
            ("mixed", "0 1-5 * * *"),
        ];

        for (name, cron) in patterns {
            let sched = make_schedule(name, cron);
            let id = ScheduleDb::create_on(&conn, &sched).unwrap();
            let got = ScheduleDb::get(&conn, &id).unwrap().unwrap();
            assert!(
                got.next_run_at.is_some(),
                "next_run_at should be set for {name} ({cron})"
            );
        }
    }

    #[test]
    fn cron_expr_dst_transitions() {
        let conn = setup_db();

        let sched = Schedule {
            id: String::new(),
            name: "dst-spring".to_string(),
            cron_expr: "30 2 * * *".to_string(),
            timezone: "US/Eastern".to_string(),
            enabled: true,
            action_type: ActionType::McpTool,
            action_payload: "{}".to_string(),
            desired_outcome: None,
            max_retries: 3,
            timeout_secs: Some(300),
            max_output_bytes: 65536,
            max_runs: 100,
            next_run_at: None,
            created_at: 0,
            updated_at: 0,
        };
        let id = ScheduleDb::create_on(&conn, &sched).unwrap();
        let got = ScheduleDb::get(&conn, &id).unwrap().unwrap();
        assert!(got.next_run_at.is_some());

        let sched2 = Schedule {
            name: "dst-fall".to_string(),
            cron_expr: "30 1 * * *".to_string(),
            ..sched
        };
        let id2 = ScheduleDb::create_on(&conn, &sched2).unwrap();
        let got2 = ScheduleDb::get(&conn, &id2).unwrap().unwrap();
        assert!(got2.next_run_at.is_some());
    }

    #[test]
    fn next_pending_returns_soonest() {
        let conn = setup_db();

        let mut s1 = make_schedule("later", "0 0 * * *");
        s1.next_run_at = Some(2000);
        ScheduleDb::create_on(&conn, &s1).unwrap();

        let mut s2 = make_schedule("sooner", "* * * * *");
        s2.next_run_at = Some(1000);
        ScheduleDb::create_on(&conn, &s2).unwrap();

        let mut s3 = make_schedule("disabled", "* * * * *");
        s3.enabled = false;
        s3.next_run_at = Some(500);
        ScheduleDb::create_on(&conn, &s3).unwrap();

        let pending = ScheduleDb::next_pending(&conn).unwrap().unwrap();
        assert_eq!(pending.name, "sooner");
        assert_eq!(pending.next_run_at.unwrap(), 1000);
    }
}
