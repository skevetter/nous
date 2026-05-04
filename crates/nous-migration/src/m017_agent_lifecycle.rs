use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS agent_versions (\
             id TEXT NOT NULL PRIMARY KEY, \
             agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
             skill_hash TEXT NOT NULL, \
             config_hash TEXT NOT NULL, \
             skills_json TEXT NOT NULL DEFAULT '[]', \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             )",
        )
        .await?;
        db.execute_unprepared(
            "CREATE INDEX IF NOT EXISTS idx_agent_versions_agent ON agent_versions(agent_id)",
        )
        .await?;
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS agent_templates (\
             id TEXT NOT NULL PRIMARY KEY, \
             name TEXT NOT NULL UNIQUE, \
             template_type TEXT NOT NULL, \
             default_config TEXT NOT NULL DEFAULT '{}', \
             skill_refs TEXT NOT NULL DEFAULT '[]', \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             )",
        )
        .await?;
        for alter in [
            "ALTER TABLE agents ADD COLUMN current_version_id TEXT REFERENCES agent_versions(id)",
            "ALTER TABLE agents ADD COLUMN upgrade_available INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE agents ADD COLUMN template_id TEXT REFERENCES agent_templates(id)",
        ] {
            if let Err(e) = db.execute_unprepared(alter).await {
                let msg = e.to_string();
                if !msg.contains("duplicate column") {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TABLE IF EXISTS agent_templates; \
             DROP TABLE IF EXISTS agent_versions;",
        )
        .await?;
        Ok(())
    }
}
