use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "CREATE VIRTUAL TABLE IF NOT EXISTS room_messages_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61')",
            "CREATE TRIGGER IF NOT EXISTS room_messages_fts_insert AFTER INSERT ON room_messages BEGIN INSERT INTO room_messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content); END",
            "CREATE TRIGGER IF NOT EXISTS room_messages_fts_delete AFTER DELETE ON room_messages BEGIN INSERT INTO room_messages_fts(room_messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content); END",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS room_messages_fts_insert",
            "DROP TRIGGER IF EXISTS room_messages_fts_delete",
            "DROP TABLE IF EXISTS room_messages_fts",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
