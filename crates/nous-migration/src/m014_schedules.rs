use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS schedules (\
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
             ); \
             CREATE INDEX IF NOT EXISTS idx_schedules_enabled_next ON schedules(enabled, next_run_at); \
             CREATE INDEX IF NOT EXISTS idx_schedules_name ON schedules(name); \
             CREATE TABLE IF NOT EXISTS schedule_runs (\
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
             ); \
             CREATE INDEX IF NOT EXISTS idx_runs_schedule_started ON schedule_runs(schedule_id, started_at DESC); \
             CREATE INDEX IF NOT EXISTS idx_runs_status ON schedule_runs(status)"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TABLE IF EXISTS schedule_runs; \
             DROP TABLE IF EXISTS schedules;",
        )
        .await?;
        Ok(())
    }
}
