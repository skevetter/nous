use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS search_events (\
             id INTEGER PRIMARY KEY, \
             query_text TEXT NOT NULL, \
             search_type TEXT NOT NULL CHECK(search_type IN ('fts','vector','hybrid','fts5_fallback')), \
             result_count INTEGER NOT NULL, \
             latency_ms INTEGER NOT NULL, \
             workspace_id TEXT, \
             agent_id TEXT, \
             created_at TEXT NOT NULL DEFAULT (datetime('now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_search_events_type ON search_events(search_type); \
             CREATE INDEX IF NOT EXISTS idx_search_events_created ON search_events(created_at)"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TABLE IF EXISTS search_events")
            .await?;
        Ok(())
    }
}
