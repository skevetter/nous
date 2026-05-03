use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS agent_processes (\
             id TEXT NOT NULL PRIMARY KEY, \
             agent_id TEXT NOT NULL REFERENCES agents(id), \
             process_type TEXT NOT NULL CHECK(process_type IN ('claude','shell','http')), \
             command TEXT NOT NULL, \
             working_dir TEXT, \
             env_json TEXT DEFAULT '{}', \
             pid INTEGER, \
             status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','starting','running','stopping','stopped','failed','crashed')), \
             exit_code INTEGER, \
             started_at TEXT, \
             stopped_at TEXT, \
             last_output TEXT, \
             max_output_bytes INTEGER NOT NULL DEFAULT 65536, \
             restart_policy TEXT NOT NULL DEFAULT 'never' CHECK(restart_policy IN ('never','on-failure','always')), \
             restart_count INTEGER NOT NULL DEFAULT 0, \
             max_restarts INTEGER NOT NULL DEFAULT 3, \
             timeout_secs INTEGER, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_agent_proc_agent ON agent_processes(agent_id); \
             CREATE INDEX IF NOT EXISTS idx_agent_proc_status ON agent_processes(status); \
             CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_proc_active ON agent_processes(agent_id) WHERE status IN ('pending','starting','running','stopping')"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS agent_processes")
            .await?;
        Ok(())
    }
}
