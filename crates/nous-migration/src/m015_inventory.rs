use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "CREATE TABLE IF NOT EXISTS inventory (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL, \
             artifact_type TEXT NOT NULL CHECK(artifact_type IN ('worktree','room','schedule','branch','file','docker-image','binary')), \
             owner_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL, \
             namespace TEXT NOT NULL DEFAULT 'default', \
             path TEXT, \
             status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived','deleted')), \
             metadata TEXT, \
             tags TEXT NOT NULL DEFAULT '[]', \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             archived_at TEXT\
             )",
            "CREATE INDEX IF NOT EXISTS idx_inventory_owner ON inventory(owner_agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_inventory_namespace_type ON inventory(namespace, artifact_type)",
            "CREATE INDEX IF NOT EXISTS idx_inventory_status ON inventory(status)",
            "CREATE INDEX IF NOT EXISTS idx_inventory_name ON inventory(name)",
            "CREATE TRIGGER IF NOT EXISTS inventory_au AFTER UPDATE ON inventory WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE inventory SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END",
            "CREATE VIRTUAL TABLE IF NOT EXISTS inventory_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61')",
            "CREATE TRIGGER IF NOT EXISTS inventory_fts_insert AFTER INSERT ON inventory BEGIN INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END",
            "CREATE TRIGGER IF NOT EXISTS inventory_fts_delete AFTER DELETE ON inventory BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; END",
            "CREATE TRIGGER IF NOT EXISTS inventory_fts_update AFTER UPDATE ON inventory WHEN NEW.name != OLD.name OR NEW.artifact_type != OLD.artifact_type OR IFNULL(NEW.metadata, '') != IFNULL(OLD.metadata, '') OR NEW.tags != OLD.tags BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS inventory_fts_insert",
            "DROP TRIGGER IF EXISTS inventory_fts_delete",
            "DROP TRIGGER IF EXISTS inventory_fts_update",
            "DROP TABLE IF EXISTS inventory_fts",
            "DROP TRIGGER IF EXISTS inventory_au",
            "DROP TABLE IF EXISTS inventory",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
