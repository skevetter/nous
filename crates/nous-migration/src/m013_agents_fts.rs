use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "CREATE VIRTUAL TABLE IF NOT EXISTS agents_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61')",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_insert AFTER INSERT ON agents BEGIN INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_delete AFTER DELETE ON agents BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; END",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_update AFTER UPDATE ON agents WHEN NEW.name != OLD.name OR NEW.agent_type != OLD.agent_type OR IFNULL(NEW.metadata_json, '') != IFNULL(OLD.metadata_json, '') BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS agents_fts_insert",
            "DROP TRIGGER IF EXISTS agents_fts_delete",
            "DROP TRIGGER IF EXISTS agents_fts_update",
            "DROP TABLE IF EXISTS agents_fts",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
