use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Drop existing FTS triggers that reference agent_type BEFORE dropping the column
        let drop_triggers = [
            "DROP TRIGGER IF EXISTS agents_fts_insert",
            "DROP TRIGGER IF EXISTS agents_fts_delete",
            "DROP TRIGGER IF EXISTS agents_fts_update",
        ];
        for sql in drop_triggers {
            db.execute_unprepared(sql).await?;
        }

        // SQLite doesn't support DROP COLUMN directly before 3.35.0,
        // but sea-orm targets recent SQLite. Use ALTER TABLE DROP COLUMN.
        db.execute_unprepared("ALTER TABLE agents DROP COLUMN agent_type")
            .await?;

        // Recreate FTS triggers WITHOUT agent_type
        let create_triggers = [
            "CREATE TRIGGER IF NOT EXISTS agents_fts_insert AFTER INSERT ON agents BEGIN INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_delete AFTER DELETE ON agents BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; END",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_update AFTER UPDATE ON agents WHEN NEW.name != OLD.name OR IFNULL(NEW.metadata_json, '') != IFNULL(OLD.metadata_json, '') BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END",
        ];
        for sql in create_triggers {
            db.execute_unprepared(sql).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Re-add agent_type column with a default value
        db.execute_unprepared(
            "ALTER TABLE agents ADD COLUMN agent_type TEXT NOT NULL DEFAULT 'engineer'",
        )
        .await?;

        // Drop the triggers without agent_type
        let drop_triggers = [
            "DROP TRIGGER IF EXISTS agents_fts_insert",
            "DROP TRIGGER IF EXISTS agents_fts_delete",
            "DROP TRIGGER IF EXISTS agents_fts_update",
        ];
        for sql in drop_triggers {
            db.execute_unprepared(sql).await?;
        }

        // Recreate FTS triggers WITH agent_type (original from m013)
        let create_triggers = [
            "CREATE TRIGGER IF NOT EXISTS agents_fts_insert AFTER INSERT ON agents BEGIN INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_delete AFTER DELETE ON agents BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; END",
            "CREATE TRIGGER IF NOT EXISTS agents_fts_update AFTER UPDATE ON agents WHEN NEW.name != OLD.name OR NEW.agent_type != OLD.agent_type OR IFNULL(NEW.metadata_json, '') != IFNULL(OLD.metadata_json, '') BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END",
        ];
        for sql in create_triggers {
            db.execute_unprepared(sql).await?;
        }

        Ok(())
    }
}
