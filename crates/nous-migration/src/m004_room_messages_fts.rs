use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE VIRTUAL TABLE IF NOT EXISTS room_messages_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
             CREATE TRIGGER IF NOT EXISTS room_messages_fts_insert AFTER INSERT ON room_messages BEGIN INSERT INTO room_messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content); END; \
             CREATE TRIGGER IF NOT EXISTS room_messages_fts_delete AFTER DELETE ON room_messages BEGIN INSERT INTO room_messages_fts(room_messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content); END;"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TRIGGER IF EXISTS room_messages_fts_insert; \
             DROP TRIGGER IF EXISTS room_messages_fts_delete; \
             DROP TABLE IF EXISTS room_messages_fts;",
        )
        .await?;
        Ok(())
    }
}
