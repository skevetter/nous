use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

// --- room_subscriptions: agent_id NOT NULL → REFERENCES agents(id) ON DELETE CASCADE ---
const ROOM_SUBSCRIPTIONS_STMTS: &[&str] = &[
    "ALTER TABLE room_subscriptions RENAME TO _room_subscriptions_old",
    "CREATE TABLE room_subscriptions (\
     room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
     topics TEXT, \
     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
     PRIMARY KEY (room_id, agent_id)\
     )",
    "INSERT INTO room_subscriptions SELECT * FROM _room_subscriptions_old",
    "DROP TABLE _room_subscriptions_old",
];

// NOTE: tasks is skipped because it has child FK references from
// task_links, task_events, task_dependencies, and worktrees —
// RENAME-based migration is unsafe for parent tables with foreign_keys=ON.
// --- task_events: actor_id nullable → REFERENCES agents(id) ON DELETE SET NULL ---
const TASK_EVENTS_STMTS: &[&str] = &[
    "ALTER TABLE task_events RENAME TO _task_events_old",
    "CREATE TABLE task_events (\
     id TEXT PRIMARY KEY, \
     task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE, \
     event_type TEXT NOT NULL CHECK(event_type IN ('created','status_changed','assigned','priority_changed','linked','unlinked','note_added')), \
     old_value TEXT, \
     new_value TEXT, \
     actor_id TEXT REFERENCES agents(id) ON DELETE SET NULL, \
     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
     )",
    "INSERT INTO task_events SELECT * FROM _task_events_old",
    "DROP TABLE _task_events_old",
    "CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id, created_at)",
];

// --- worktrees: agent_id nullable → REFERENCES agents(id) ON DELETE SET NULL ---
const WORKTREES_STMTS: &[&str] = &[
    "DROP TRIGGER IF EXISTS worktrees_au",
    "ALTER TABLE worktrees RENAME TO _worktrees_old",
    "CREATE TABLE worktrees (\
     id TEXT PRIMARY KEY, \
     slug TEXT NOT NULL, \
     path TEXT NOT NULL, \
     branch TEXT NOT NULL, \
     repo_root TEXT NOT NULL, \
     agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL, \
     task_id TEXT REFERENCES tasks(id) ON DELETE SET NULL, \
     status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','stale','archived','deleted')), \
     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
     updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
     UNIQUE(slug, repo_root)\
     )",
    "INSERT INTO worktrees SELECT * FROM _worktrees_old",
    "DROP TABLE _worktrees_old",
    "CREATE INDEX IF NOT EXISTS idx_worktrees_agent ON worktrees(agent_id)",
    "CREATE INDEX IF NOT EXISTS idx_worktrees_task ON worktrees(task_id)",
    "CREATE INDEX IF NOT EXISTS idx_worktrees_status ON worktrees(status)",
    "CREATE INDEX IF NOT EXISTS idx_worktrees_branch ON worktrees(branch)",
    "CREATE TRIGGER IF NOT EXISTS worktrees_au AFTER UPDATE ON worktrees WHEN NEW.updated_at = OLD.updated_at BEGIN UPDATE worktrees SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id; END",
];

// NOTE: memories, memory_sessions, and search_events are skipped because
// memories has child tables (memory_relations, memory_access_log) and
// memory_sessions is referenced by memories.session_id, making
// RENAME-based migration unsafe within a transaction.
// --- search_events: agent_id nullable → REFERENCES agents(id) ON DELETE SET NULL ---
const SEARCH_EVENTS_STMTS: &[&str] = &[
    "ALTER TABLE search_events RENAME TO _search_events_old",
    "CREATE TABLE search_events (\
     id INTEGER PRIMARY KEY, \
     query_text TEXT NOT NULL, \
     search_type TEXT NOT NULL CHECK(search_type IN ('fts','vector','hybrid','fts5_fallback')), \
     result_count INTEGER NOT NULL, \
     latency_ms INTEGER NOT NULL, \
     workspace_id TEXT, \
     agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL, \
     created_at TEXT NOT NULL DEFAULT (datetime('now'))\
     )",
    "INSERT INTO search_events SELECT * FROM _search_events_old",
    "DROP TABLE _search_events_old",
    "CREATE INDEX IF NOT EXISTS idx_search_events_type ON search_events(search_type)",
    "CREATE INDEX IF NOT EXISTS idx_search_events_created ON search_events(created_at)",
];

// --- agent_workspace_access / message_cursors / notification_queue ---
const AGENT_TABLES_STMTS: &[&str] = &[
    // agent_workspace_access: agent_id NOT NULL → REFERENCES agents(id) ON DELETE CASCADE
    "ALTER TABLE agent_workspace_access RENAME TO _agent_workspace_access_old",
    "CREATE TABLE agent_workspace_access (\
     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
     workspace_id TEXT NOT NULL, \
     granted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
     PRIMARY KEY (agent_id, workspace_id)\
     )",
    "INSERT INTO agent_workspace_access SELECT * FROM _agent_workspace_access_old",
    "DROP TABLE _agent_workspace_access_old",
    // message_cursors: agent_id NOT NULL → REFERENCES agents(id) ON DELETE CASCADE
    "ALTER TABLE message_cursors RENAME TO _message_cursors_old",
    "CREATE TABLE message_cursors (\
     room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
     last_read_message_id TEXT NOT NULL, \
     last_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
     PRIMARY KEY (room_id, agent_id)\
     )",
    "INSERT INTO message_cursors SELECT * FROM _message_cursors_old",
    "DROP TABLE _message_cursors_old",
    "CREATE INDEX IF NOT EXISTS idx_cursors_agent ON message_cursors(agent_id)",
    // notification_queue: agent_id NOT NULL → REFERENCES agents(id) ON DELETE CASCADE
    "ALTER TABLE notification_queue RENAME TO _notification_queue_old",
    "CREATE TABLE notification_queue (\
     id TEXT NOT NULL PRIMARY KEY, \
     agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, \
     room_id TEXT NOT NULL, \
     message_id TEXT NOT NULL, \
     sender_id TEXT NOT NULL, \
     priority TEXT NOT NULL DEFAULT 'normal' CHECK(priority IN ('low','normal','high','urgent')), \
     topics TEXT NOT NULL DEFAULT '[]', \
     mentions TEXT NOT NULL DEFAULT '[]', \
     delivered INTEGER NOT NULL DEFAULT 0, \
     created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
     )",
    "INSERT INTO notification_queue SELECT * FROM _notification_queue_old",
    "DROP TABLE _notification_queue_old",
    "CREATE INDEX IF NOT EXISTS idx_notif_queue_agent ON notification_queue(agent_id, delivered, created_at)",
    "CREATE INDEX IF NOT EXISTS idx_notif_queue_room ON notification_queue(room_id, created_at)",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let all_groups: &[&[&str]] = &[
            ROOM_SUBSCRIPTIONS_STMTS,
            TASK_EVENTS_STMTS,
            WORKTREES_STMTS,
            SEARCH_EVENTS_STMTS,
            AGENT_TABLES_STMTS,
        ];

        for group in all_groups {
            for sql in *group {
                db.execute_unprepared(sql).await?;
            }
        }
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Migration(
            "Irreversible: removing FK constraints is not supported".to_owned(),
        ))
    }
}
