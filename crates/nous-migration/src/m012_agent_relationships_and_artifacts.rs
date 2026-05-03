use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "CREATE TABLE IF NOT EXISTS agent_relationships (\
             parent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
             child_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
             relationship_type TEXT NOT NULL DEFAULT 'reports_to', \
             namespace TEXT NOT NULL DEFAULT 'default', \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             PRIMARY KEY (parent_id, child_id, namespace)\
             )",
            "CREATE INDEX IF NOT EXISTS idx_rel_parent ON agent_relationships(parent_id, namespace)",
            "CREATE INDEX IF NOT EXISTS idx_rel_child ON agent_relationships(child_id, namespace)",
            "CREATE TABLE IF NOT EXISTS artifacts (\
             id TEXT NOT NULL PRIMARY KEY, \
             agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
             artifact_type TEXT NOT NULL CHECK(artifact_type IN ('worktree','room','schedule','branch')), \
             name TEXT NOT NULL, \
             path TEXT, \
             status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','archived','deleted')), \
             namespace TEXT NOT NULL DEFAULT 'default', \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             last_seen_at TEXT, \
             UNIQUE(agent_id, artifact_type, name, namespace)\
             )",
            "CREATE INDEX IF NOT EXISTS idx_artifacts_agent ON artifacts(agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_artifacts_ns ON artifacts(namespace)",
            "CREATE INDEX IF NOT EXISTS idx_artifacts_type ON artifacts(agent_id, artifact_type, namespace)",
            "CREATE TRIGGER IF NOT EXISTS artifacts_au AFTER UPDATE ON artifacts WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE artifacts SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let stmts = [
            "DROP TRIGGER IF EXISTS artifacts_au",
            "DROP TABLE IF EXISTS artifacts",
            "DROP TABLE IF EXISTS agent_relationships",
        ];
        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
