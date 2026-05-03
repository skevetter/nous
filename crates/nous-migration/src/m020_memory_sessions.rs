use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS memory_sessions (\
             id TEXT NOT NULL PRIMARY KEY, \
             agent_id TEXT, \
             project TEXT, \
             started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             ended_at TEXT, \
             summary TEXT\
             ); \
             CREATE INDEX IF NOT EXISTS idx_memory_sessions_agent ON memory_sessions(agent_id); \
             CREATE INDEX IF NOT EXISTS idx_memory_sessions_project ON memory_sessions(project); \
             ALTER TABLE memories ADD COLUMN session_id TEXT REFERENCES memory_sessions(id);",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS memory_sessions")
            .await?;
        Ok(())
    }
}
