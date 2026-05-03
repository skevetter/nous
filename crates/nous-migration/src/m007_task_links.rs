use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS task_links (\
             id TEXT PRIMARY KEY, \
             source_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
             target_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
             link_type TEXT NOT NULL CHECK(link_type IN ('blocked_by','parent','related_to')), \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             UNIQUE(source_id, target_id, link_type)\
             ); \
             CREATE INDEX IF NOT EXISTS idx_task_links_source ON task_links(source_id); \
             CREATE INDEX IF NOT EXISTS idx_task_links_target ON task_links(target_id)",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(TaskLinks::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum TaskLinks {
    Table,
}
