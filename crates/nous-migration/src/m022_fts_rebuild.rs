use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const TOKENIZER: &str = "porter unicode61";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let sql = format!(
            "DROP TABLE IF EXISTS room_messages_fts; \
             DROP TABLE IF EXISTS tasks_fts; \
             DROP TABLE IF EXISTS agents_fts; \
             DROP TABLE IF EXISTS inventory_fts; \
             DROP TABLE IF EXISTS memories_fts; \
             DROP TABLE IF EXISTS resources_fts; \
             DROP TRIGGER IF EXISTS room_messages_fts_insert; \
             DROP TRIGGER IF EXISTS room_messages_fts_delete; \
             DROP TRIGGER IF EXISTS tasks_fts_insert; \
             DROP TRIGGER IF EXISTS tasks_fts_delete; \
             DROP TRIGGER IF EXISTS tasks_fts_update; \
             DROP TRIGGER IF EXISTS agents_fts_insert; \
             DROP TRIGGER IF EXISTS agents_fts_delete; \
             DROP TRIGGER IF EXISTS agents_fts_update; \
             DROP TRIGGER IF EXISTS inventory_fts_insert; \
             DROP TRIGGER IF EXISTS inventory_fts_delete; \
             DROP TRIGGER IF EXISTS inventory_fts_update; \
             DROP TRIGGER IF EXISTS memories_fts_insert; \
             DROP TRIGGER IF EXISTS memories_fts_delete; \
             DROP TRIGGER IF EXISTS memories_fts_update; \
             DROP TRIGGER IF EXISTS resources_fts_insert; \
             DROP TRIGGER IF EXISTS resources_fts_delete; \
             DROP TRIGGER IF EXISTS resources_fts_update; \
             CREATE VIRTUAL TABLE room_messages_fts USING fts5(content, content_rowid='rowid', tokenize='{t}'); \
             CREATE TRIGGER room_messages_fts_insert AFTER INSERT ON room_messages BEGIN INSERT INTO room_messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content); END; \
             CREATE TRIGGER room_messages_fts_delete AFTER DELETE ON room_messages BEGIN INSERT INTO room_messages_fts(room_messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content); END; \
             INSERT INTO room_messages_fts(rowid, content) SELECT rowid, content FROM room_messages; \
             CREATE VIRTUAL TABLE tasks_fts USING fts5(content, content_rowid='rowid', tokenize='{t}'); \
             CREATE TRIGGER tasks_fts_insert AFTER INSERT ON tasks BEGIN INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
             CREATE TRIGGER tasks_fts_delete AFTER DELETE ON tasks BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER tasks_fts_update AFTER UPDATE ON tasks WHEN NEW.title != OLD.title OR IFNULL(NEW.description, '') != IFNULL(OLD.description, '') BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
             INSERT INTO tasks_fts(rowid, content) SELECT rowid, title || ' ' || COALESCE(description, '') FROM tasks; \
             CREATE VIRTUAL TABLE agents_fts USING fts5(content, content_rowid='rowid', tokenize='{t}'); \
             CREATE TRIGGER agents_fts_insert AFTER INSERT ON agents BEGIN INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END; \
             CREATE TRIGGER agents_fts_delete AFTER DELETE ON agents BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER agents_fts_update AFTER UPDATE ON agents WHEN NEW.name != OLD.name OR NEW.agent_type != OLD.agent_type OR IFNULL(NEW.metadata_json, '') != IFNULL(OLD.metadata_json, '') BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END; \
             INSERT INTO agents_fts(rowid, content) SELECT rowid, name || ' ' || agent_type || ' ' || namespace || ' ' || COALESCE(metadata_json, '') FROM agents; \
             CREATE VIRTUAL TABLE inventory_fts USING fts5(content, content_rowid='rowid', tokenize='{t}'); \
             CREATE TRIGGER inventory_fts_insert AFTER INSERT ON inventory BEGIN INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
             CREATE TRIGGER inventory_fts_delete AFTER DELETE ON inventory BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER inventory_fts_update AFTER UPDATE ON inventory WHEN NEW.name != OLD.name OR NEW.artifact_type != OLD.artifact_type OR IFNULL(NEW.metadata, '') != IFNULL(OLD.metadata, '') OR NEW.tags != OLD.tags BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
             INSERT INTO inventory_fts(rowid, content) SELECT rowid, name || ' ' || artifact_type || ' ' || namespace || ' ' || COALESCE(metadata, '') || ' ' || tags FROM inventory; \
             CREATE VIRTUAL TABLE memories_fts USING fts5(content, content_rowid='rowid', tokenize='{t}'); \
             CREATE TRIGGER memories_fts_insert AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || NEW.content || ' ' || NEW.memory_type || ' ' || COALESCE(NEW.topic_key, '')); END; \
             CREATE TRIGGER memories_fts_delete AFTER DELETE ON memories BEGIN DELETE FROM memories_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER memories_fts_update AFTER UPDATE ON memories WHEN NEW.title != OLD.title OR NEW.content != OLD.content OR NEW.memory_type != OLD.memory_type OR IFNULL(NEW.topic_key, '') != IFNULL(OLD.topic_key, '') BEGIN DELETE FROM memories_fts WHERE rowid = OLD.rowid; INSERT INTO memories_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || NEW.content || ' ' || NEW.memory_type || ' ' || COALESCE(NEW.topic_key, '')); END; \
             INSERT INTO memories_fts(rowid, content) SELECT rowid, title || ' ' || content || ' ' || memory_type || ' ' || COALESCE(topic_key, '') FROM memories; \
             CREATE VIRTUAL TABLE IF NOT EXISTS resources_fts USING fts5(content, content_rowid='rowid', tokenize='{t}'); \
             CREATE TRIGGER resources_fts_insert AFTER INSERT ON resources BEGIN INSERT INTO resources_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.resource_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
             CREATE TRIGGER resources_fts_delete AFTER DELETE ON resources BEGIN DELETE FROM resources_fts WHERE rowid = OLD.rowid; END; \
             CREATE TRIGGER resources_fts_update AFTER UPDATE ON resources WHEN NEW.name != OLD.name OR NEW.resource_type != OLD.resource_type OR IFNULL(NEW.metadata, '') != IFNULL(OLD.metadata, '') OR NEW.tags != OLD.tags BEGIN DELETE FROM resources_fts WHERE rowid = OLD.rowid; INSERT INTO resources_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.resource_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
             INSERT OR IGNORE INTO resources_fts(rowid, content) SELECT rowid, name || ' ' || resource_type || ' ' || namespace || ' ' || COALESCE(metadata, '') || ' ' || tags FROM resources;",
            t = TOKENIZER
        );
        db.execute_unprepared(&sql).await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
