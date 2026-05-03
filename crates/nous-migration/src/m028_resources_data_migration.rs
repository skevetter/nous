use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "INSERT OR IGNORE INTO resources (id, name, resource_type, owner_agent_id, namespace, path, status, metadata, tags, ownership_policy, last_seen_at, created_at, updated_at) \
             SELECT id, name, artifact_type, agent_id, namespace, path, status, NULL, '[]', \
                 CASE artifact_type \
                     WHEN 'worktree' THEN 'cascade-delete' \
                     WHEN 'branch' THEN 'cascade-delete' \
                     ELSE 'orphan' \
                 END, \
                 last_seen_at, created_at, updated_at \
             FROM artifacts; \
             INSERT OR IGNORE INTO resources (id, name, resource_type, owner_agent_id, namespace, path, status, metadata, tags, ownership_policy, created_at, updated_at, archived_at) \
             SELECT id, name, artifact_type, owner_agent_id, namespace, path, status, metadata, tags, 'orphan', created_at, updated_at, archived_at \
             FROM inventory"
        ).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
