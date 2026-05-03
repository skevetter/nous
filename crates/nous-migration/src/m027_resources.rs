use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "CREATE TABLE IF NOT EXISTS resources (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL, \
             resource_type TEXT NOT NULL CHECK(resource_type IN ('worktree','room','schedule','branch','file','docker-image','binary')), \
             owner_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL, \
             namespace TEXT NOT NULL DEFAULT 'default', \
             path TEXT, \
             status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived','deleted')), \
             metadata TEXT, \
             tags TEXT NOT NULL DEFAULT '[]', \
             ownership_policy TEXT NOT NULL DEFAULT 'orphan' CHECK(ownership_policy IN ('cascade-delete','orphan','transfer-to-parent')), \
             last_seen_at TEXT, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             archived_at TEXT, \
             UNIQUE(owner_agent_id, resource_type, name, namespace)\
             )",
            "CREATE INDEX IF NOT EXISTS idx_resources_owner ON resources(owner_agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_resources_namespace_type ON resources(namespace, resource_type)",
            "CREATE INDEX IF NOT EXISTS idx_resources_status ON resources(status)",
            "CREATE INDEX IF NOT EXISTS idx_resources_name ON resources(name)",
            "CREATE INDEX IF NOT EXISTS idx_resources_last_seen ON resources(last_seen_at) WHERE last_seen_at IS NOT NULL",
            "CREATE TRIGGER IF NOT EXISTS resources_au AFTER UPDATE ON resources WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE resources SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END",
            "CREATE VIRTUAL TABLE IF NOT EXISTS resources_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61')",
            "CREATE TRIGGER IF NOT EXISTS resources_fts_insert AFTER INSERT ON resources BEGIN INSERT INTO resources_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.resource_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END",
            "CREATE TRIGGER IF NOT EXISTS resources_fts_delete AFTER DELETE ON resources BEGIN DELETE FROM resources_fts WHERE rowid = OLD.rowid; END",
            "CREATE TRIGGER IF NOT EXISTS resources_fts_update AFTER UPDATE ON resources WHEN NEW.name != OLD.name OR NEW.resource_type != OLD.resource_type OR IFNULL(NEW.metadata, '') != IFNULL(OLD.metadata, '') OR NEW.tags != OLD.tags BEGIN DELETE FROM resources_fts WHERE rowid = OLD.rowid; INSERT INTO resources_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.resource_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS resources_fts_insert",
            "DROP TRIGGER IF EXISTS resources_fts_delete",
            "DROP TRIGGER IF EXISTS resources_fts_update",
            "DROP TABLE IF EXISTS resources_fts",
            "DROP TRIGGER IF EXISTS resources_au",
            "DROP TABLE IF EXISTS resources",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
