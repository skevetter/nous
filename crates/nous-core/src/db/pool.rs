use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::error::NousError;

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
];

struct Migration {
    version: &'static str,
    name: &'static str,
    sql: &'static str,
}

/// Holds connection pools for both SQLite databases.
/// Call `close()` before application exit to ensure WAL checkpointing completes.
pub struct DbPools {
    pub fts: SqlitePool,
    pub vec: SqlitePool,
}

impl DbPools {
    pub async fn connect(data_dir: &Path) -> Result<Self, NousError> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| NousError::Internal(format!("failed to create data dir: {e}")))?;

        let fts_path = data_dir.join("memory-fts.db");
        let vec_path = data_dir.join("memory-vec.db");

        let fts = create_pool(&fts_path).await?;
        let vec = create_pool(&vec_path).await?;

        Ok(Self { fts, vec })
    }

    pub async fn run_migrations(&self) -> Result<(), NousError> {
        run_migrations_on_pool(&self.fts).await?;
        run_migrations_on_pool(&self.vec).await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.fts.close().await;
        self.vec.close().await;
    }
}

async fn run_migrations_on_pool(pool: &SqlitePool) -> Result<(), NousError> {
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
        pools.run_migrations().await.unwrap();

        let expected = MIGRATIONS.len() as i64;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, expected);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.vec)
            .await
            .unwrap();
        assert_eq!(row.0, expected);

        pools.close().await;
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        pools.run_migrations().await.unwrap();

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, MIGRATIONS.len() as i64);

        pools.close().await;
    }
}
