use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS agent_invocations (\
             id TEXT NOT NULL PRIMARY KEY, \
             agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
             process_id TEXT REFERENCES agent_processes(id) ON DELETE SET NULL, \
             prompt TEXT NOT NULL, \
             result TEXT, \
             status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','running','completed','failed','timeout','cancelled')), \
             error TEXT, \
             started_at TEXT, \
             completed_at TEXT, \
             duration_ms INTEGER, \
             metadata_json TEXT, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_invoc_agent ON agent_invocations(agent_id); \
             CREATE INDEX IF NOT EXISTS idx_invoc_status ON agent_invocations(status); \
             CREATE INDEX IF NOT EXISTS idx_invoc_created ON agent_invocations(created_at DESC)"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS agent_invocations")
            .await?;
        Ok(())
    }
}
