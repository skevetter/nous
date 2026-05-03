use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS worktrees (\
             id TEXT PRIMARY KEY, \
             slug TEXT NOT NULL, \
             path TEXT NOT NULL, \
             branch TEXT NOT NULL, \
             repo_root TEXT NOT NULL, \
             agent_id TEXT, \
             task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL, \
             status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','stale','archived','deleted')), \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             UNIQUE(slug, repo_root)\
             ); \
             CREATE INDEX IF NOT EXISTS idx_worktrees_agent ON worktrees(agent_id); \
             CREATE INDEX IF NOT EXISTS idx_worktrees_task ON worktrees(task_id); \
             CREATE INDEX IF NOT EXISTS idx_worktrees_status ON worktrees(status); \
             CREATE INDEX IF NOT EXISTS idx_worktrees_branch ON worktrees(branch); \
             CREATE TRIGGER IF NOT EXISTS worktrees_au AFTER UPDATE ON worktrees WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE worktrees SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TRIGGER IF EXISTS worktrees_au; \
             DROP TABLE IF EXISTS worktrees;",
        )
        .await?;
        Ok(())
    }
}
