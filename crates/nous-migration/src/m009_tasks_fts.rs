use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
             CREATE TRIGGER IF NOT EXISTS tasks_fts_insert AFTER INSERT ON tasks BEGIN INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
             CREATE TRIGGER IF NOT EXISTS tasks_fts_delete AFTER DELETE ON tasks BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER IF NOT EXISTS tasks_fts_update AFTER UPDATE ON tasks WHEN NEW.title != OLD.title OR IFNULL(NEW.description, '') != IFNULL(OLD.description, '') BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
             CREATE TRIGGER IF NOT EXISTS tasks_au AFTER UPDATE ON tasks WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TRIGGER IF EXISTS tasks_fts_insert; \
             DROP TRIGGER IF EXISTS tasks_fts_delete; \
             DROP TRIGGER IF EXISTS tasks_fts_update; \
             DROP TRIGGER IF EXISTS tasks_au; \
             DROP TABLE IF EXISTS tasks_fts;",
        )
        .await?;
        Ok(())
    }
}
