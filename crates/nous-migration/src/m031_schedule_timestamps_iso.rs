use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let stmts = [
            // Rename old tables out of the way (avoids FK DROP issues)
            "ALTER TABLE schedule_runs RENAME TO _schedule_runs_old",
            "ALTER TABLE schedules RENAME TO _schedules_old",
            // Create new schedules table with TEXT timestamps
            "CREATE TABLE schedules (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL, \
             cron_expr TEXT NOT NULL, \
             trigger_at TEXT, \
             timezone TEXT NOT NULL DEFAULT 'UTC', \
             enabled INTEGER NOT NULL DEFAULT 1, \
             action_type TEXT NOT NULL CHECK(action_type IN ('mcp_tool','shell','http','agent_invoke')), \
             action_payload TEXT NOT NULL, \
             desired_outcome TEXT, \
             max_retries INTEGER NOT NULL DEFAULT 3, \
             timeout_secs INTEGER, \
             max_output_bytes INTEGER NOT NULL DEFAULT 65536, \
             max_runs INTEGER NOT NULL DEFAULT 100, \
             last_run_at TEXT, \
             next_run_at TEXT, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             )",
            // Migrate schedules data: convert INTEGER unix timestamps to ISO 8601
            "INSERT INTO schedules (id, name, cron_expr, trigger_at, timezone, enabled, \
             action_type, action_payload, desired_outcome, max_retries, timeout_secs, \
             max_output_bytes, max_runs, last_run_at, next_run_at, created_at, updated_at) \
             SELECT id, name, cron_expr, \
             CASE WHEN trigger_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%fZ', trigger_at, 'unixepoch') ELSE NULL END, \
             timezone, enabled, action_type, action_payload, desired_outcome, max_retries, \
             timeout_secs, max_output_bytes, max_runs, \
             CASE WHEN last_run_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%fZ', last_run_at, 'unixepoch') ELSE NULL END, \
             CASE WHEN next_run_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%fZ', next_run_at, 'unixepoch') ELSE NULL END, \
             strftime('%Y-%m-%dT%H:%M:%fZ', created_at, 'unixepoch'), \
             strftime('%Y-%m-%dT%H:%M:%fZ', updated_at, 'unixepoch') \
             FROM _schedules_old",
            "CREATE INDEX IF NOT EXISTS idx_schedules_enabled_next ON schedules(enabled, next_run_at)",
            "CREATE INDEX IF NOT EXISTS idx_schedules_name ON schedules(name)",
            // Create new schedule_runs table with TEXT timestamps
            "CREATE TABLE schedule_runs (\
             id TEXT NOT NULL PRIMARY KEY, \
             schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE, \
             started_at TEXT NOT NULL, \
             finished_at TEXT, \
             status TEXT NOT NULL DEFAULT 'running' CHECK(status IN ('running','completed','failed','timeout','skipped')), \
             exit_code INTEGER, \
             output TEXT, \
             error TEXT, \
             attempt INTEGER NOT NULL DEFAULT 1, \
             duration_ms INTEGER\
             )",
            // Migrate schedule_runs data
            "INSERT INTO schedule_runs (id, schedule_id, started_at, finished_at, status, \
             exit_code, output, error, attempt, duration_ms) \
             SELECT id, schedule_id, \
             strftime('%Y-%m-%dT%H:%M:%fZ', started_at, 'unixepoch'), \
             CASE WHEN finished_at IS NOT NULL THEN strftime('%Y-%m-%dT%H:%M:%fZ', finished_at, 'unixepoch') ELSE NULL END, \
             status, exit_code, output, error, attempt, duration_ms \
             FROM _schedule_runs_old",
            "CREATE INDEX IF NOT EXISTS idx_runs_schedule_started ON schedule_runs(schedule_id, started_at DESC)",
            "CREATE INDEX IF NOT EXISTS idx_runs_status ON schedule_runs(status)",
            // Drop old tables
            "DROP TABLE _schedule_runs_old",
            "DROP TABLE _schedules_old",
        ];

        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let stmts = [
            // Rename new tables out of the way
            "ALTER TABLE schedule_runs RENAME TO _schedule_runs_new",
            "ALTER TABLE schedules RENAME TO _schedules_new",
            // Recreate original tables with INTEGER timestamps
            "CREATE TABLE schedules (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL, \
             cron_expr TEXT NOT NULL, \
             trigger_at INTEGER, \
             timezone TEXT NOT NULL DEFAULT 'UTC', \
             enabled INTEGER NOT NULL DEFAULT 1, \
             action_type TEXT NOT NULL CHECK(action_type IN ('mcp_tool','shell','http','agent_invoke')), \
             action_payload TEXT NOT NULL, \
             desired_outcome TEXT, \
             max_retries INTEGER NOT NULL DEFAULT 3, \
             timeout_secs INTEGER, \
             max_output_bytes INTEGER NOT NULL DEFAULT 65536, \
             max_runs INTEGER NOT NULL DEFAULT 100, \
             last_run_at INTEGER, \
             next_run_at INTEGER, \
             created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')), \
             updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))\
             )",
            "INSERT INTO schedules SELECT id, name, cron_expr, \
             CAST(strftime('%s', trigger_at) AS INTEGER), \
             timezone, enabled, action_type, action_payload, desired_outcome, max_retries, \
             timeout_secs, max_output_bytes, max_runs, \
             CAST(strftime('%s', last_run_at) AS INTEGER), \
             CAST(strftime('%s', next_run_at) AS INTEGER), \
             CAST(strftime('%s', created_at) AS INTEGER), \
             CAST(strftime('%s', updated_at) AS INTEGER) \
             FROM _schedules_new",
            "CREATE TABLE schedule_runs (\
             id TEXT NOT NULL PRIMARY KEY, \
             schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE, \
             started_at INTEGER NOT NULL, \
             finished_at INTEGER, \
             status TEXT NOT NULL DEFAULT 'running' CHECK(status IN ('running','completed','failed','timeout','skipped')), \
             exit_code INTEGER, \
             output TEXT, \
             error TEXT, \
             attempt INTEGER NOT NULL DEFAULT 1, \
             duration_ms INTEGER\
             )",
            "INSERT INTO schedule_runs SELECT id, schedule_id, \
             CAST(strftime('%s', started_at) AS INTEGER), \
             CAST(strftime('%s', finished_at) AS INTEGER), \
             status, exit_code, output, error, attempt, duration_ms \
             FROM _schedule_runs_new",
            // Drop temp tables
            "DROP TABLE _schedule_runs_new",
            "DROP TABLE _schedules_new",
        ];

        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
