use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS tasks (\
             id TEXT PRIMARY KEY, \
             title TEXT NOT NULL, \
             description TEXT, \
             status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','in_progress','done','closed')), \
             priority TEXT NOT NULL DEFAULT 'medium' CHECK(priority IN ('critical','high','medium','low')), \
             assignee_id TEXT, \
             labels TEXT, \
             room_id TEXT REFERENCES rooms(id) ON DELETE SET NULL, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             closed_at TEXT\
             ); \
             CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status); \
             CREATE INDEX IF NOT EXISTS idx_tasks_assignee ON tasks(assignee_id); \
             CREATE INDEX IF NOT EXISTS idx_tasks_room ON tasks(room_id); \
             CREATE INDEX IF NOT EXISTS idx_tasks_created ON tasks(created_at)"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Tasks::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
pub enum Tasks {
    Table,
}
