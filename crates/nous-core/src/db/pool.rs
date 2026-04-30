use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::error::NousError;

pub const EMBEDDING_DIMENSION: usize = 384;

pub type VecPool = Arc<Mutex<Connection>>;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: "001",
        name: "schema_version",
        sql: "CREATE TABLE IF NOT EXISTS schema_version (\
              id INTEGER PRIMARY KEY, \
              version TEXT NOT NULL, \
              applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
              );",
    },
    Migration {
        version: "002",
        name: "rooms",
        sql: "CREATE TABLE IF NOT EXISTS rooms (\
              id TEXT PRIMARY KEY, \
              name TEXT NOT NULL, \
              purpose TEXT, \
              metadata TEXT, \
              archived INTEGER NOT NULL DEFAULT 0, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE UNIQUE INDEX IF NOT EXISTS idx_rooms_name_active ON rooms(name) WHERE archived = 0;",
    },
    Migration {
        version: "003",
        name: "room_messages",
        sql: "CREATE TABLE IF NOT EXISTS room_messages (\
              id TEXT PRIMARY KEY, \
              room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
              sender_id TEXT NOT NULL, \
              content TEXT NOT NULL, \
              reply_to TEXT, \
              metadata TEXT, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              );",
    },
    Migration {
        version: "004",
        name: "room_messages_fts",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS room_messages_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
              CREATE TRIGGER IF NOT EXISTS room_messages_fts_insert AFTER INSERT ON room_messages BEGIN INSERT INTO room_messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content); END; \
              CREATE TRIGGER IF NOT EXISTS room_messages_fts_delete AFTER DELETE ON room_messages BEGIN INSERT INTO room_messages_fts(room_messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content); END;",
    },
    Migration {
        version: "005",
        name: "room_subscriptions",
        sql: "CREATE TABLE IF NOT EXISTS room_subscriptions (\
              room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
              agent_id TEXT NOT NULL, \
              topics TEXT, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              PRIMARY KEY (room_id, agent_id)\
              );",
    },
    Migration {
        version: "006",
        name: "tasks",
        sql: "CREATE TABLE IF NOT EXISTS tasks (\
              id TEXT PRIMARY KEY, \
              title TEXT NOT NULL, \
              description TEXT, \
              status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open','in_progress','done','closed')), \
              priority TEXT NOT NULL DEFAULT 'medium' CHECK(priority IN ('critical','high','medium','low')), \
              assignee_id TEXT, \
              labels TEXT, \
              room_id TEXT REFERENCES rooms(id) ON DELETE SET NULL, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              closed_at TEXT\
              ); \
              CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status); \
              CREATE INDEX IF NOT EXISTS idx_tasks_assignee ON tasks(assignee_id); \
              CREATE INDEX IF NOT EXISTS idx_tasks_room ON tasks(room_id); \
              CREATE INDEX IF NOT EXISTS idx_tasks_created ON tasks(created_at);",
    },
    Migration {
        version: "007",
        name: "task_links",
        sql: "CREATE TABLE IF NOT EXISTS task_links (\
              id TEXT PRIMARY KEY, \
              source_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
              target_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
              link_type TEXT NOT NULL CHECK(link_type IN ('blocked_by','parent','related_to')), \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              UNIQUE(source_id, target_id, link_type)\
              ); \
              CREATE INDEX IF NOT EXISTS idx_task_links_source ON task_links(source_id); \
              CREATE INDEX IF NOT EXISTS idx_task_links_target ON task_links(target_id);",
    },
    Migration {
        version: "008",
        name: "task_events",
        sql: "CREATE TABLE IF NOT EXISTS task_events (\
              id TEXT PRIMARY KEY, \
              task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
              event_type TEXT NOT NULL CHECK(event_type IN ('created','status_changed','assigned','priority_changed','linked','unlinked','note_added')), \
              old_value TEXT, \
              new_value TEXT, \
              actor_id TEXT, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id, created_at);",
    },
    Migration {
        version: "009",
        name: "tasks_fts",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS tasks_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
              CREATE TRIGGER IF NOT EXISTS tasks_fts_insert AFTER INSERT ON tasks BEGIN INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
              CREATE TRIGGER IF NOT EXISTS tasks_fts_delete AFTER DELETE ON tasks BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; END; \
              CREATE TRIGGER IF NOT EXISTS tasks_fts_update AFTER UPDATE ON tasks WHEN NEW.title != OLD.title OR IFNULL(NEW.description, '') != IFNULL(OLD.description, '') BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
              CREATE TRIGGER IF NOT EXISTS tasks_au AFTER UPDATE ON tasks WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;",
    },
    Migration {
        version: "010",
        name: "worktrees",
        sql: "CREATE TABLE IF NOT EXISTS worktrees (\
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
              CREATE TRIGGER IF NOT EXISTS worktrees_au AFTER UPDATE ON worktrees WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE worktrees SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;",
    },
    Migration {
        version: "011",
        name: "agents",
        sql: "CREATE TABLE IF NOT EXISTS agents (\
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
              CREATE TRIGGER IF NOT EXISTS agents_au AFTER UPDATE ON agents WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE agents SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;",
    },
    Migration {
        version: "012",
        name: "agent_relationships_and_artifacts",
        sql: "CREATE TABLE IF NOT EXISTS agent_relationships (\
              parent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
              child_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
              relationship_type TEXT NOT NULL DEFAULT 'reports_to', \
              namespace TEXT NOT NULL DEFAULT 'default', \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              PRIMARY KEY (parent_id, child_id, namespace)\
              ); \
              CREATE INDEX IF NOT EXISTS idx_rel_parent ON agent_relationships(parent_id, namespace); \
              CREATE INDEX IF NOT EXISTS idx_rel_child ON agent_relationships(child_id, namespace); \
              CREATE TABLE IF NOT EXISTS artifacts (\
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
              ); \
              CREATE INDEX IF NOT EXISTS idx_artifacts_agent ON artifacts(agent_id); \
              CREATE INDEX IF NOT EXISTS idx_artifacts_ns ON artifacts(namespace); \
              CREATE INDEX IF NOT EXISTS idx_artifacts_type ON artifacts(agent_id, artifact_type, namespace); \
              CREATE TRIGGER IF NOT EXISTS artifacts_au AFTER UPDATE ON artifacts WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE artifacts SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;",
    },
    Migration {
        version: "013",
        name: "agents_fts",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS agents_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
              CREATE TRIGGER IF NOT EXISTS agents_fts_insert AFTER INSERT ON agents BEGIN INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END; \
              CREATE TRIGGER IF NOT EXISTS agents_fts_delete AFTER DELETE ON agents BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; END; \
              CREATE TRIGGER IF NOT EXISTS agents_fts_update AFTER UPDATE ON agents WHEN NEW.name != OLD.name OR NEW.agent_type != OLD.agent_type OR IFNULL(NEW.metadata_json, '') != IFNULL(OLD.metadata_json, '') BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END;",
    },
    Migration {
        version: "014",
        name: "schedules",
        sql: "CREATE TABLE IF NOT EXISTS schedules (\
              id TEXT NOT NULL PRIMARY KEY, \
              name TEXT NOT NULL, \
              cron_expr TEXT NOT NULL, \
              trigger_at INTEGER, \
              timezone TEXT NOT NULL DEFAULT 'UTC', \
              enabled INTEGER NOT NULL DEFAULT 1, \
              action_type TEXT NOT NULL CHECK(action_type IN ('mcp_tool','shell','http')), \
              action_payload TEXT NOT NULL, \
              desired_outcome TEXT, \
              max_retries INTEGER NOT NULL DEFAULT 3, \
              timeout_secs INTEGER, \
              max_output_bytes INTEGER NOT NULL DEFAULT 65536, \
              max_runs INTEGER NOT NULL DEFAULT 100, \
              last_run_at INTEGER, \
              next_run_at INTEGER, \
              created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')), \
              updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))\
              ); \
              CREATE INDEX IF NOT EXISTS idx_schedules_enabled_next ON schedules(enabled, next_run_at); \
              CREATE INDEX IF NOT EXISTS idx_schedules_name ON schedules(name); \
              CREATE TABLE IF NOT EXISTS schedule_runs (\
              id TEXT NOT NULL PRIMARY KEY, \
              schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE, \
              started_at INTEGER NOT NULL, \
              finished_at INTEGER, \
              status TEXT NOT NULL DEFAULT 'running' CHECK(status IN ('running','completed','failed','timeout','skipped')), \
              exit_code INTEGER, \
              output TEXT, \
              error TEXT, \
              attempt INTEGER NOT NULL DEFAULT 1, \
              duration_ms INTEGER\
              ); \
              CREATE INDEX IF NOT EXISTS idx_runs_schedule_started ON schedule_runs(schedule_id, started_at DESC); \
              CREATE INDEX IF NOT EXISTS idx_runs_status ON schedule_runs(status);",
    },
    Migration {
        version: "015",
        name: "inventory",
        sql: "CREATE TABLE IF NOT EXISTS inventory (\
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
              ); \
              CREATE INDEX IF NOT EXISTS idx_inventory_owner ON inventory(owner_agent_id); \
              CREATE INDEX IF NOT EXISTS idx_inventory_namespace_type ON inventory(namespace, artifact_type); \
              CREATE INDEX IF NOT EXISTS idx_inventory_status ON inventory(status); \
              CREATE INDEX IF NOT EXISTS idx_inventory_name ON inventory(name); \
              CREATE TRIGGER IF NOT EXISTS inventory_au AFTER UPDATE ON inventory WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE inventory SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END; \
              CREATE VIRTUAL TABLE IF NOT EXISTS inventory_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
              CREATE TRIGGER IF NOT EXISTS inventory_fts_insert AFTER INSERT ON inventory BEGIN INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
              CREATE TRIGGER IF NOT EXISTS inventory_fts_delete AFTER DELETE ON inventory BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; END; \
              CREATE TRIGGER IF NOT EXISTS inventory_fts_update AFTER UPDATE ON inventory WHEN NEW.name != OLD.name OR NEW.artifact_type != OLD.artifact_type OR IFNULL(NEW.metadata, '') != IFNULL(OLD.metadata, '') OR NEW.tags != OLD.tags BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END;",
    },
    Migration {
        version: "016",
        name: "memories",
        sql: "CREATE TABLE IF NOT EXISTS memories (\
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
              );",
    },
    Migration {
        version: "017",
        name: "agent_lifecycle",
        sql: "CREATE TABLE IF NOT EXISTS agent_versions (\
              id TEXT NOT NULL PRIMARY KEY, \
              agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
              skill_hash TEXT NOT NULL, \
              config_hash TEXT NOT NULL, \
              skills_json TEXT NOT NULL DEFAULT '[]', \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE INDEX IF NOT EXISTS idx_agent_versions_agent ON agent_versions(agent_id); \
              CREATE TABLE IF NOT EXISTS agent_templates (\
              id TEXT NOT NULL PRIMARY KEY, \
              name TEXT NOT NULL UNIQUE, \
              template_type TEXT NOT NULL, \
              default_config TEXT NOT NULL DEFAULT '{}', \
              skill_refs TEXT NOT NULL DEFAULT '[]', \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              ALTER TABLE agents ADD COLUMN current_version_id TEXT REFERENCES agent_versions(id); \
              ALTER TABLE agents ADD COLUMN upgrade_available INTEGER NOT NULL DEFAULT 0; \
              ALTER TABLE agents ADD COLUMN template_id TEXT REFERENCES agent_templates(id);",
    },
    Migration {
        version: "018",
        name: "memory_embeddings",
        sql: "ALTER TABLE memories ADD COLUMN embedding BLOB;",
    },
    Migration {
        version: "019",
        name: "task_dependencies_and_templates",
        sql: "CREATE TABLE IF NOT EXISTS task_dependencies (\
              id TEXT NOT NULL PRIMARY KEY, \
              task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
              depends_on_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
              dep_type TEXT NOT NULL DEFAULT 'blocked_by' CHECK(dep_type IN ('blocked_by','blocks','waiting_on')), \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              UNIQUE(task_id, depends_on_task_id, dep_type)\
              ); \
              CREATE INDEX IF NOT EXISTS idx_task_deps_task ON task_dependencies(task_id); \
              CREATE INDEX IF NOT EXISTS idx_task_deps_depends ON task_dependencies(depends_on_task_id); \
              CREATE TABLE IF NOT EXISTS task_templates (\
              id TEXT NOT NULL PRIMARY KEY, \
              name TEXT NOT NULL UNIQUE, \
              title_pattern TEXT NOT NULL, \
              description_template TEXT, \
              default_priority TEXT NOT NULL DEFAULT 'medium' CHECK(default_priority IN ('critical','high','medium','low')), \
              default_labels TEXT NOT NULL DEFAULT '[]', \
              checklist TEXT NOT NULL DEFAULT '[]', \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE TRIGGER IF NOT EXISTS task_templates_au AFTER UPDATE ON task_templates WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE task_templates SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END;",
    },
    Migration {
        version: "020",
        name: "memory_sessions",
        sql: "CREATE TABLE IF NOT EXISTS memory_sessions (\
              id TEXT NOT NULL PRIMARY KEY, \
              agent_id TEXT, \
              project TEXT, \
              started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              ended_at TEXT, \
              summary TEXT\
              ); \
              CREATE INDEX IF NOT EXISTS idx_memory_sessions_agent ON memory_sessions(agent_id); \
              CREATE INDEX IF NOT EXISTS idx_memory_sessions_project ON memory_sessions(project); \
              ALTER TABLE memories ADD COLUMN session_id TEXT REFERENCES memory_sessions(id);",
    },
    Migration {
        version: "021",
        name: "search_events",
        sql: "CREATE TABLE IF NOT EXISTS search_events (\
              id INTEGER PRIMARY KEY, \
              query_text TEXT NOT NULL, \
              search_type TEXT NOT NULL CHECK(search_type IN ('fts','vector','hybrid')), \
              result_count INTEGER NOT NULL, \
              latency_ms INTEGER NOT NULL, \
              workspace_id TEXT, \
              agent_id TEXT, \
              created_at TEXT NOT NULL DEFAULT (datetime('now'))\
              ); \
              CREATE INDEX IF NOT EXISTS idx_search_events_type ON search_events(search_type); \
              CREATE INDEX IF NOT EXISTS idx_search_events_created ON search_events(created_at);",
    },
];

struct Migration {
    version: &'static str,
    name: &'static str,
    sql: &'static str,
}

fn validate_tokenizer(tokenizer: &str) -> Result<(), NousError> {
    let valid_parts: &[&str] = &["porter", "unicode61", "trigram", "ascii"];
    for part in tokenizer.split_whitespace() {
        if !valid_parts.contains(&part) {
            return Err(NousError::Validation(format!(
                "invalid FTS5 tokenizer component: {}",
                part
            )));
        }
    }
    if tokenizer.trim().is_empty() {
        return Err(NousError::Validation(
            "invalid FTS5 tokenizer component: (empty)".to_string(),
        ));
    }
    Ok(())
}

fn migration_022_fts_rebuild(tokenizer: &str) -> String {
    format!(
        "DROP TABLE IF EXISTS room_messages_fts; \
         DROP TABLE IF EXISTS tasks_fts; \
         DROP TABLE IF EXISTS agents_fts; \
         DROP TABLE IF EXISTS inventory_fts; \
         DROP TABLE IF EXISTS memories_fts; \
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
         CREATE VIRTUAL TABLE room_messages_fts USING fts5(content, content_rowid='rowid', tokenize='{tokenizer}'); \
         CREATE TRIGGER room_messages_fts_insert AFTER INSERT ON room_messages BEGIN INSERT INTO room_messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content); END; \
         CREATE TRIGGER room_messages_fts_delete AFTER DELETE ON room_messages BEGIN INSERT INTO room_messages_fts(room_messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content); END; \
         INSERT INTO room_messages_fts(rowid, content) SELECT rowid, content FROM room_messages; \
         CREATE VIRTUAL TABLE tasks_fts USING fts5(content, content_rowid='rowid', tokenize='{tokenizer}'); \
         CREATE TRIGGER tasks_fts_insert AFTER INSERT ON tasks BEGIN INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
         CREATE TRIGGER tasks_fts_delete AFTER DELETE ON tasks BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; END; \
         CREATE TRIGGER tasks_fts_update AFTER UPDATE ON tasks WHEN NEW.title != OLD.title OR IFNULL(NEW.description, '') != IFNULL(OLD.description, '') BEGIN DELETE FROM tasks_fts WHERE rowid = OLD.rowid; INSERT INTO tasks_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || COALESCE(NEW.description, '')); END; \
         INSERT INTO tasks_fts(rowid, content) SELECT rowid, title || ' ' || COALESCE(description, '') FROM tasks; \
         CREATE VIRTUAL TABLE agents_fts USING fts5(content, content_rowid='rowid', tokenize='{tokenizer}'); \
         CREATE TRIGGER agents_fts_insert AFTER INSERT ON agents BEGIN INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END; \
         CREATE TRIGGER agents_fts_delete AFTER DELETE ON agents BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; END; \
         CREATE TRIGGER agents_fts_update AFTER UPDATE ON agents WHEN NEW.name != OLD.name OR NEW.agent_type != OLD.agent_type OR IFNULL(NEW.metadata_json, '') != IFNULL(OLD.metadata_json, '') BEGIN DELETE FROM agents_fts WHERE rowid = OLD.rowid; INSERT INTO agents_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.agent_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata_json, '')); END; \
         INSERT INTO agents_fts(rowid, content) SELECT rowid, name || ' ' || agent_type || ' ' || namespace || ' ' || COALESCE(metadata_json, '') FROM agents; \
         CREATE VIRTUAL TABLE inventory_fts USING fts5(content, content_rowid='rowid', tokenize='{tokenizer}'); \
         CREATE TRIGGER inventory_fts_insert AFTER INSERT ON inventory BEGIN INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
         CREATE TRIGGER inventory_fts_delete AFTER DELETE ON inventory BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; END; \
         CREATE TRIGGER inventory_fts_update AFTER UPDATE ON inventory WHEN NEW.name != OLD.name OR NEW.artifact_type != OLD.artifact_type OR IFNULL(NEW.metadata, '') != IFNULL(OLD.metadata, '') OR NEW.tags != OLD.tags BEGIN DELETE FROM inventory_fts WHERE rowid = OLD.rowid; INSERT INTO inventory_fts(rowid, content) VALUES (NEW.rowid, NEW.name || ' ' || NEW.artifact_type || ' ' || NEW.namespace || ' ' || COALESCE(NEW.metadata, '') || ' ' || NEW.tags); END; \
         INSERT INTO inventory_fts(rowid, content) SELECT rowid, name || ' ' || artifact_type || ' ' || namespace || ' ' || COALESCE(metadata, '') || ' ' || tags FROM inventory; \
         CREATE VIRTUAL TABLE memories_fts USING fts5(content, content_rowid='rowid', tokenize='{tokenizer}'); \
         CREATE TRIGGER memories_fts_insert AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || NEW.content || ' ' || NEW.memory_type || ' ' || COALESCE(NEW.topic_key, '')); END; \
         CREATE TRIGGER memories_fts_delete AFTER DELETE ON memories BEGIN DELETE FROM memories_fts WHERE rowid = OLD.rowid; END; \
         CREATE TRIGGER memories_fts_update AFTER UPDATE ON memories WHEN NEW.title != OLD.title OR NEW.content != OLD.content OR NEW.memory_type != OLD.memory_type OR IFNULL(NEW.topic_key, '') != IFNULL(OLD.topic_key, '') BEGIN DELETE FROM memories_fts WHERE rowid = OLD.rowid; INSERT INTO memories_fts(rowid, content) VALUES (NEW.rowid, NEW.title || ' ' || NEW.content || ' ' || NEW.memory_type || ' ' || COALESCE(NEW.topic_key, '')); END; \
         INSERT INTO memories_fts(rowid, content) SELECT rowid, title || ' ' || content || ' ' || memory_type || ' ' || COALESCE(topic_key, '') FROM memories;",
        tokenizer = tokenizer
    )
}

/// Holds connection pools for both SQLite databases.
/// Call `close()` before application exit to ensure WAL checkpointing completes.
pub struct DbPools {
    pub fts: SqlitePool,
    pub vec: VecPool,
}

impl DbPools {
    pub async fn connect(data_dir: &Path) -> Result<Self, NousError> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| NousError::Internal(format!("failed to create data dir: {e}")))?;

        let fts_path = data_dir.join("memory-fts.db");
        let vec_path = data_dir.join("memory-vec.db");

        let fts = create_pool(&fts_path).await?;
        let vec = create_vec_pool(&vec_path)?;

        Ok(Self { fts, vec })
    }

    pub async fn run_migrations(&self, tokenizer: &str) -> Result<(), NousError> {
        run_migrations_on_pool(&self.fts, tokenizer).await?;
        run_vec_migrations(&self.vec)?;
        Ok(())
    }

    pub async fn close(&self) {
        self.fts.close().await;
    }
}

pub fn create_vec_pool(path: &Path) -> Result<VecPool, NousError> {
    let conn = Connection::open(path)
        .map_err(|e| NousError::Internal(format!("failed to open vec db: {e}")))?;

    unsafe {
        let db = conn.handle();
        let rc = sqlite3_vec_init(db, std::ptr::null_mut(), std::ptr::null());
        if rc != rusqlite::ffi::SQLITE_OK {
            return Err(NousError::Internal(format!(
                "failed to load sqlite-vec: rc={rc}"
            )));
        }
    }

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .map_err(|e| NousError::Internal(format!("failed to set vec db pragmas: {e}")))?;

    Ok(Arc::new(Mutex::new(conn)))
}

#[link(name = "sqlite_vec0")]
extern "C" {
    fn sqlite3_vec_init(
        db: *mut rusqlite::ffi::sqlite3,
        pz_err_msg: *mut *mut std::ffi::c_char,
        p_api: *const rusqlite::ffi::sqlite3_api_routines,
    ) -> std::ffi::c_int;
}

const VEC_MIGRATIONS: &[Migration] = &[
    Migration {
        version: "vec_001",
        name: "memory_embeddings_vec0",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(\
              memory_id TEXT PRIMARY KEY, \
              embedding float[384]\
              );",
    },
    Migration {
        version: "vec_002",
        name: "memory_chunks",
        sql: "CREATE TABLE IF NOT EXISTS memory_chunks (\
              id TEXT PRIMARY KEY, \
              memory_id TEXT NOT NULL, \
              content TEXT NOT NULL, \
              chunk_index INTEGER NOT NULL, \
              start_offset INTEGER NOT NULL, \
              end_offset INTEGER NOT NULL, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE INDEX IF NOT EXISTS idx_chunks_memory_id ON memory_chunks(memory_id);",
    },
];

fn run_vec_migrations(vec_pool: &VecPool) -> Result<(), NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS vec_schema_version (\
         id INTEGER PRIMARY KEY, \
         version TEXT NOT NULL, \
         applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
         );",
    )
    .map_err(|e| NousError::Internal(format!("failed to create vec schema_version: {e}")))?;

    for migration in VEC_MIGRATIONS {
        let already_applied: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM vec_schema_version WHERE version = ?1)",
                rusqlite::params![migration.version],
                |row| row.get(0),
            )
            .map_err(|e| NousError::Internal(format!("failed to check vec migration: {e}")))?;

        if !already_applied {
            conn.execute_batch(migration.sql).map_err(|e| {
                NousError::Internal(format!(
                    "failed to run vec migration {}: {e}",
                    migration.version
                ))
            })?;
            conn.execute(
                "INSERT INTO vec_schema_version (version) VALUES (?1)",
                rusqlite::params![migration.version],
            )
            .map_err(|e| NousError::Internal(format!("failed to record vec migration: {e}")))?;
            tracing::info!(
                version = migration.version,
                name = migration.name,
                "applied vec migration"
            );
        }
    }

    Ok(())
}

async fn run_migrations_on_pool(pool: &SqlitePool, tokenizer: &str) -> Result<(), NousError> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_version (\
         id INTEGER PRIMARY KEY, \
         version TEXT NOT NULL, \
         applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
         );",
    )
    .execute(pool)
    .await?;

    for migration in MIGRATIONS {
        let already_applied: bool =
            sqlx::query("SELECT EXISTS(SELECT 1 FROM schema_version WHERE version = ?)")
                .bind(migration.version)
                .fetch_one(pool)
                .await?
                .get(0);

        if !already_applied {
            sqlx::raw_sql(migration.sql).execute(pool).await?;
            sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                .bind(migration.version)
                .execute(pool)
                .await?;
            tracing::info!(
                version = migration.version,
                name = migration.name,
                "applied migration"
            );
        }
    }

    // Migration 022: unconditionally drop+recreate FTS5 tables with configured tokenizer
    validate_tokenizer(tokenizer)?;
    let m022_version = "022";
    let m022_already_applied: bool =
        sqlx::query("SELECT EXISTS(SELECT 1 FROM schema_version WHERE version = ?)")
            .bind(m022_version)
            .fetch_one(pool)
            .await?
            .get(0);

    if !m022_already_applied {
        let sql = migration_022_fts_rebuild(tokenizer);
        sqlx::raw_sql(&sql).execute(pool).await?;
        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(m022_version)
            .execute(pool)
            .await?;
        tracing::info!(
            version = m022_version,
            name = "fts_rebuild_with_tokenizer",
            tokenizer = tokenizer,
            "applied migration"
        );
    }

    Ok(())
}

async fn create_pool(path: &Path) -> Result<SqlitePool, NousError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn connect_creates_db_files() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();

        assert!(tmp.path().join("memory-fts.db").exists());
        assert!(tmp.path().join("memory-vec.db").exists());

        pools.close().await;
    }

    #[tokio::test]
    async fn run_migrations_creates_schema_version() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();

        // MIGRATIONS array + migration 022 (dynamic)
        let expected = MIGRATIONS.len() as i64 + 1;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, expected);

        let vec_count: i64 = pools
            .vec
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM vec_schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(vec_count, VEC_MIGRATIONS.len() as i64);

        pools.close().await;
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, MIGRATIONS.len() as i64 + 1);

        pools.close().await;
    }

    #[tokio::test]
    async fn fts_tables_use_configured_tokenizer() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("trigram").await.unwrap();

        let tables = [
            "room_messages_fts",
            "tasks_fts",
            "agents_fts",
            "inventory_fts",
            "memories_fts",
        ];
        for table in tables {
            let sql: (String,) =
                sqlx::query_as("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?")
                    .bind(table)
                    .fetch_one(&pools.fts)
                    .await
                    .unwrap();
            assert!(
                sql.0.contains("trigram"),
                "table {table} should use trigram tokenizer, got: {}",
                sql.0
            );
        }

        pools.close().await;
    }

    #[tokio::test]
    async fn fts_tables_use_default_tokenizer() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();

        let sql: (String,) = sqlx::query_as(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'memories_fts'",
        )
        .fetch_one(&pools.fts)
        .await
        .unwrap();
        assert!(
            sql.0.contains("porter unicode61"),
            "memories_fts should use porter unicode61 tokenizer, got: {}",
            sql.0
        );

        pools.close().await;
    }

    #[tokio::test]
    async fn invalid_tokenizer_rejected() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        let result = pools.run_migrations("'); DROP TABLE tasks; --").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("invalid FTS5 tokenizer component"));
        pools.close().await;
    }
}
