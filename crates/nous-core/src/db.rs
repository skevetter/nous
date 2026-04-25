use nous_shared::Result;
use nous_shared::sqlite::{open_connection, run_migrations};
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // models
    "CREATE TABLE IF NOT EXISTS models (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE,
        dimensions INTEGER NOT NULL,
        max_tokens INTEGER NOT NULL,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )",
    // workspaces
    "CREATE TABLE IF NOT EXISTS workspaces (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        path TEXT NOT NULL UNIQUE,
        name TEXT,
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )",
    // categories
    "CREATE TABLE IF NOT EXISTS categories (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        parent_id INTEGER REFERENCES categories(id),
        source TEXT NOT NULL DEFAULT 'system',
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        UNIQUE(name, parent_id)
    )",
    // memories
    "CREATE TABLE IF NOT EXISTS memories (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        content TEXT NOT NULL,
        memory_type TEXT NOT NULL CHECK(memory_type IN ('decision','convention','bugfix','architecture','fact','observation')),
        source TEXT,
        importance TEXT NOT NULL DEFAULT 'moderate' CHECK(importance IN ('low','moderate','high')),
        confidence TEXT NOT NULL DEFAULT 'moderate' CHECK(confidence IN ('low','moderate','high')),
        workspace_id INTEGER REFERENCES workspaces(id),
        session_id TEXT,
        trace_id TEXT,
        agent_id TEXT,
        agent_model TEXT,
        valid_from TEXT,
        valid_until TEXT,
        archived INTEGER NOT NULL DEFAULT 0,
        category_id INTEGER REFERENCES categories(id),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )",
    // tags
    "CREATE TABLE IF NOT EXISTS tags (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL UNIQUE
    )",
    // memory_tags
    "CREATE TABLE IF NOT EXISTS memory_tags (
        memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
        PRIMARY KEY (memory_id, tag_id)
    )",
    // relationships
    "CREATE TABLE IF NOT EXISTS relationships (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        source_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        target_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        relation_type TEXT NOT NULL CHECK(relation_type IN ('related','supersedes','contradicts','depends_on')),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )",
    // access_log
    "CREATE TABLE IF NOT EXISTS access_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        accessed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
        access_type TEXT NOT NULL,
        session_id TEXT
    )",
    // FTS5 virtual table
    "CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
        title,
        content,
        memory_type,
        content='memories',
        content_rowid='rowid'
    )",
    // memory_chunks
    "CREATE TABLE IF NOT EXISTS memory_chunks (
        id TEXT PRIMARY KEY,
        memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
        chunk_index INTEGER NOT NULL,
        content TEXT NOT NULL,
        token_count INTEGER NOT NULL,
        model_id INTEGER NOT NULL REFERENCES models(id),
        created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
    )",
    // Fallback embeddings table (sqlite-vec is broken upstream)
    "CREATE TABLE IF NOT EXISTS memory_embeddings (
        chunk_id TEXT PRIMARY KEY,
        embedding BLOB
    )",
    // FTS triggers
    "CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
        INSERT INTO memories_fts(rowid, title, content, memory_type)
        VALUES (new.rowid, new.title, new.content, new.memory_type);
    END",
    "CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
        INSERT INTO memories_fts(memories_fts, rowid, title, content, memory_type)
        VALUES ('delete', old.rowid, old.title, old.content, old.memory_type);
    END",
    "CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
        INSERT INTO memories_fts(memories_fts, rowid, title, content, memory_type)
        VALUES ('delete', old.rowid, old.title, old.content, old.memory_type);
        INSERT INTO memories_fts(rowid, title, content, memory_type)
        VALUES (new.rowid, new.title, new.content, new.memory_type);
    END",
    // Tag cleanup trigger
    "CREATE TRIGGER IF NOT EXISTS tags_cleanup AFTER DELETE ON memory_tags BEGIN
        DELETE FROM tags WHERE id = old.tag_id
            AND NOT EXISTS (SELECT 1 FROM memory_tags WHERE tag_id = old.tag_id);
    END",
    // Indexes
    "CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(memory_type)",
    "CREATE INDEX IF NOT EXISTS idx_memories_importance ON memories(importance)",
    "CREATE INDEX IF NOT EXISTS idx_memories_workspace ON memories(workspace_id)",
    "CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category_id)",
    "CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at)",
    "CREATE INDEX IF NOT EXISTS idx_memories_archived ON memories(archived)",
    "CREATE INDEX IF NOT EXISTS idx_memory_tags_memory ON memory_tags(memory_id)",
    "CREATE INDEX IF NOT EXISTS idx_memory_tags_tag ON memory_tags(tag_id)",
    "CREATE INDEX IF NOT EXISTS idx_relationships_source ON relationships(source_id)",
    "CREATE INDEX IF NOT EXISTS idx_relationships_target ON relationships(target_id)",
    "CREATE INDEX IF NOT EXISTS idx_memory_chunks_memory ON memory_chunks(memory_id)",
];

pub struct MemoryDb {
    conn: Connection,
}

impl MemoryDb {
    pub fn open(path: &str, key: Option<&str>) -> Result<Self> {
        let conn = open_connection(path, key)?;
        run_migrations(&conn, MIGRATIONS)?;
        seed_categories(&conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

fn seed_categories(conn: &Connection) -> Result<()> {
    let categories: &[(&str, &[&str])] = &[
        (
            "infrastructure",
            &["k8s", "networking", "storage", "compute"],
        ),
        (
            "data-platform",
            &["etl", "scheduling", "data-quality", "warehousing"],
        ),
        ("ci-cd", &["pipelines", "testing", "deployment", "runners"]),
        (
            "security",
            &["auth", "secrets", "compliance", "vulnerabilities"],
        ),
        ("tooling", &["dev-environment", "cli", "editors", "scripts"]),
        (
            "observability",
            &["monitoring", "logging", "tracing", "alerting"],
        ),
        (
            "architecture",
            &["design-patterns", "system-design", "apis"],
        ),
        ("workflow", &["git", "pr-process", "code-review", "release"]),
        ("cost", &["cloud-spend", "optimization", "billing"]),
        ("languages", &["rust", "python", "go", "typescript"]),
        (
            "libraries",
            &["dependencies", "frameworks", "crate-evaluation"],
        ),
        ("team", &["people", "ownership", "decisions", "process"]),
        (
            "project",
            &["timelines", "milestones", "status", "blockers"],
        ),
        (
            "debugging",
            &["root-cause", "investigation", "troubleshooting"],
        ),
        (
            "configuration",
            &["env-vars", "feature-flags", "settings", "helm"],
        ),
    ];

    for (parent_name, children) in categories {
        conn.execute(
            "INSERT OR IGNORE INTO categories (name, parent_id, source) VALUES (?1, NULL, 'system')",
            [parent_name],
        )?;

        let parent_id: i64 = conn.query_row(
            "SELECT id FROM categories WHERE name=?1 AND parent_id IS NULL",
            [parent_name],
            |row| row.get(0),
        )?;

        for child_name in *children {
            conn.execute(
                "INSERT OR IGNORE INTO categories (name, parent_id, source) VALUES (?1, ?2, 'system')",
                rusqlite::params![child_name, parent_id],
            )?;
        }
    }

    Ok(())
}
