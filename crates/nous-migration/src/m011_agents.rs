use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS agents (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL, \
             agent_type TEXT NOT NULL CHECK(agent_type IN ('engineer','manager','director','senior-manager')), \
             parent_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL, \
             namespace TEXT NOT NULL DEFAULT 'default', \
             status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','inactive','archived','running','idle','blocked','done')), \
             room TEXT, \
             last_seen_at TEXT, \
             metadata_json TEXT, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             UNIQUE(name, namespace)\
             ); \
             CREATE INDEX IF NOT EXISTS idx_agents_namespace ON agents(namespace); \
             CREATE INDEX IF NOT EXISTS idx_agents_parent ON agents(parent_agent_id); \
             CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(namespace, status); \
             CREATE TRIGGER IF NOT EXISTS agents_au AFTER UPDATE ON agents WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE agents SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TRIGGER IF EXISTS agents_au; \
             DROP TABLE IF EXISTS agents;",
        )
        .await?;
        Ok(())
    }
}
