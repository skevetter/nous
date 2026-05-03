use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS memories (\
             id TEXT NOT NULL PRIMARY KEY, \
             workspace_id TEXT NOT NULL DEFAULT 'default', \
             agent_id TEXT, \
             title TEXT NOT NULL, \
             content TEXT NOT NULL, \
             memory_type TEXT NOT NULL CHECK(memory_type IN ('decision','convention','bugfix','architecture','fact','observation')), \
             importance TEXT NOT NULL DEFAULT 'moderate' CHECK(importance IN ('low','moderate','high')), \
             topic_key TEXT, \
             valid_from TEXT, \
             valid_until TEXT, \
             archived INTEGER NOT NULL DEFAULT 0, \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_memories_workspace ON memories(workspace_id); \
             CREATE INDEX IF NOT EXISTS idx_memories_agent ON memories(agent_id); \
             CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type); \
             CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance); \
             CREATE INDEX IF NOT EXISTS idx_memories_topic ON memories(topic_key); \
             CREATE INDEX IF NOT EXISTS idx_memories_archived ON memories(archived); \
             CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE memories SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END; \
             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
             CREATE TRIGGER IF NOT EXISTS memories_fts_insert AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || NEW.content || ' ' || NEW.memory_type || ' ' || COALESCE(NEW.topic_key, '')); END; \
             CREATE TRIGGER IF NOT EXISTS memories_fts_delete AFTER DELETE ON memories BEGIN DELETE FROM memories_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER IF NOT EXISTS memories_fts_update AFTER UPDATE ON memories WHEN NEW.title != OLD.title OR NEW.content != OLD.content OR NEW.memory_type != OLD.memory_type OR IFNULL(NEW.topic_key, '') != IFNULL(OLD.topic_key, '') BEGIN DELETE FROM memories_fts WHERE rowid = OLD.rowid; INSERT INTO memories_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || NEW.content || ' ' || NEW.memory_type || ' ' || COALESCE(NEW.topic_key, '')); END; \
             CREATE TABLE IF NOT EXISTS memory_relations (\
             id TEXT NOT NULL PRIMARY KEY, \
             source_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE, \
             target_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE, \
             relation_type TEXT NOT NULL CHECK(relation_type IN ('supersedes','conflicts_with','related','compatible','scoped','not_conflict')), \
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             UNIQUE(source_id, target_id, relation_type)\
             ); \
             CREATE INDEX IF NOT EXISTS idx_memory_relations_source ON memory_relations(source_id); \
             CREATE INDEX IF NOT EXISTS idx_memory_relations_target ON memory_relations(target_id); \
             CREATE TABLE IF NOT EXISTS memory_access_log (\
             id INTEGER PRIMARY KEY AUTOINCREMENT, \
             memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE, \
             access_type TEXT NOT NULL CHECK(access_type IN ('recall','search','context')), \
             session_id TEXT, \
             accessed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
             ); \
             CREATE INDEX IF NOT EXISTS idx_access_log_memory ON memory_access_log(memory_id); \
             CREATE INDEX IF NOT EXISTS idx_access_log_accessed ON memory_access_log(accessed_at); \
             CREATE TABLE IF NOT EXISTS agent_workspace_access (\
             agent_id TEXT NOT NULL, \
             workspace_id TEXT NOT NULL, \
             granted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
             PRIMARY KEY (agent_id, workspace_id)\
             );"
        ).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(
            "DROP TABLE IF EXISTS agent_workspace_access; \
             DROP TABLE IF EXISTS memory_access_log; \
             DROP TABLE IF EXISTS memory_relations; \
             DROP TRIGGER IF EXISTS memories_fts_insert; \
             DROP TRIGGER IF EXISTS memories_fts_delete; \
             DROP TRIGGER IF EXISTS memories_fts_update; \
             DROP TABLE IF EXISTS memories_fts; \
             DROP TRIGGER IF EXISTS memories_au; \
             DROP TABLE IF EXISTS memories;",
        )
        .await?;
        Ok(())
    }
}
