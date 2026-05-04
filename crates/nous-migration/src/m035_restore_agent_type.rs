use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // m030 dropped agent_type; re-add it for databases that already ran m030.
        // For fresh databases where m030 was neutralized, the column already exists
        // from m011 — the ADD COLUMN will fail, which we tolerate.
        let result = db
            .execute_unprepared(
                "ALTER TABLE agents ADD COLUMN agent_type TEXT NOT NULL DEFAULT 'engineer'",
            )
            .await;

        match result {
            Ok(_) => {}
            Err(e) if e.to_string().contains("duplicate column") => {}
            Err(e) => return Err(e),
        }

        // Rebuild FTS triggers to include agent_type
        let drop_triggers = [
            "DROP TRIGGER IF EXISTS agents_fts_insert",
            "DROP TRIGGER IF EXISTS agents_fts_delete",
            "DROP TRIGGER IF EXISTS agents_fts_update",
        ];
        for sql in drop_triggers {
            db.execute_unprepared(sql).await?;
        }

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

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let drop_triggers = [
            "DROP TRIGGER IF EXISTS agents_fts_insert",
            "DROP TRIGGER IF EXISTS agents_fts_delete",
            "DROP TRIGGER IF EXISTS agents_fts_update",
        ];
        for sql in drop_triggers {
            db.execute_unprepared(sql).await?;
        }

        db.execute_unprepared("ALTER TABLE agents DROP COLUMN agent_type")
            .await?;

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
}
