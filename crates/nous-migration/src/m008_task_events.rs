use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS task_events (\
             id TEXT PRIMARY KEY, \
             task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
             event_type TEXT NOT NULL CHECK(event_type IN ('created','status_changed','assigned','priority_changed','linked','unlinked','note_added')), \
             old_value TEXT, \
             new_value TEXT, \
             actor_id TEXT, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id, created_at)"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TaskEvents::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum TaskEvents {
    Table,
}
