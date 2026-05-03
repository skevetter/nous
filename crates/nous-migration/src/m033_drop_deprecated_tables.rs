use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS inventory_fts_insert",
            "DROP TRIGGER IF EXISTS inventory_fts_delete",
            "DROP TRIGGER IF EXISTS inventory_fts_update",
            "DROP TABLE IF EXISTS inventory_fts",
            "DROP TRIGGER IF EXISTS inventory_au",
            "DROP TABLE IF EXISTS inventory",
            "DROP TRIGGER IF EXISTS artifacts_au",
            "DROP TABLE IF EXISTS artifacts",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
