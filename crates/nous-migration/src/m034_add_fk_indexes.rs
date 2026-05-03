use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let stmts = [
            "CREATE INDEX IF NOT EXISTS idx_room_subscriptions_agent ON room_subscriptions(agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_worktrees_agent ON worktrees(agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_memories_agent ON memories(agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_memory_sessions_agent ON memory_sessions(agent_id)",
            "CREATE INDEX IF NOT EXISTS idx_room_messages_room ON room_messages(room_id)",
            "CREATE INDEX IF NOT EXISTS idx_agent_workspace_access_agent ON agent_workspace_access(agent_id)",
        ];

        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let stmts = [
            "DROP INDEX IF EXISTS idx_room_subscriptions_agent",
            "DROP INDEX IF EXISTS idx_worktrees_agent",
            "DROP INDEX IF EXISTS idx_memories_agent",
            "DROP INDEX IF EXISTS idx_memory_sessions_agent",
            "DROP INDEX IF EXISTS idx_room_messages_room",
            "DROP INDEX IF EXISTS idx_agent_workspace_access_agent",
        ];

        for sql in stmts {
            db.execute_unprepared(sql).await?;
        }
        Ok(())
    }
}
