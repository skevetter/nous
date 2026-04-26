use nous_shared::Result;
use nous_shared::ids::MemoryId;
use nous_shared::sqlite::{open_connection, run_migrations};
use rusqlite::{Connection, params};

use crate::types::{
    Category, Memory, MemoryPatch, MemoryWithRelations, NewMemory, RelationType, Relationship,
};

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
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_categories_name_root ON categories(name) WHERE parent_id IS NULL",
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
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_relationships_unique ON relationships(source_id, target_id, relation_type)",
    "CREATE INDEX IF NOT EXISTS idx_memory_chunks_memory ON memory_chunks(memory_id)",
];

pub struct MemoryDb {
    conn: Connection,
}

impl MemoryDb {
    pub fn open(path: &str, key: Option<&str>) -> Result<Self> {
        let conn = open_connection(path, key)?;
        run_migrations(&conn, MIGRATIONS)?;
        migrate_models_columns(&conn)?;
        seed_categories(&conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn store(&self, memory: &NewMemory) -> Result<MemoryId> {
        let id = MemoryId::new();
        let tx = self.conn.unchecked_transaction()?;

        let workspace_id: Option<i64> = if let Some(ref path) = memory.workspace_path {
            tx.execute(
                "INSERT OR IGNORE INTO workspaces (path) VALUES (?1)",
                params![path],
            )?;
            Some(tx.query_row(
                "SELECT id FROM workspaces WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )?)
        } else {
            None
        };

        tx.execute(
            "INSERT INTO memories (id, title, content, memory_type, source, importance, confidence,
                workspace_id, session_id, trace_id, agent_id, agent_model, valid_from, archived, category_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                id.to_string(),
                memory.title,
                memory.content,
                memory.memory_type.to_string(),
                memory.source,
                memory.importance.to_string(),
                memory.confidence.to_string(),
                workspace_id,
                memory.session_id,
                memory.trace_id,
                memory.agent_id,
                memory.agent_model,
                memory.valid_from,
                0,
                memory.category_id,
            ],
        )?;

        for tag_name in &memory.tags {
            tx.execute(
                "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
                params![tag_name],
            )?;
            let tag_id: i64 = tx.query_row(
                "SELECT id FROM tags WHERE name = ?1",
                params![tag_name],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO memory_tags (memory_id, tag_id) VALUES (?1, ?2)",
                params![id.to_string(), tag_id],
            )?;
        }

        tx.commit()?;
        Ok(id)
    }

    pub fn recall(&self, id: &MemoryId) -> Result<Option<MemoryWithRelations>> {
        let id_str = id.to_string();

        let memory = match self.conn.query_row(
            "SELECT id, title, content, memory_type, source, importance, confidence,
                    workspace_id, session_id, trace_id, agent_id, agent_model,
                    valid_from, valid_until, archived, category_id, created_at, updated_at
             FROM memories WHERE id = ?1",
            params![id_str],
            |row| {
                Ok(Memory {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    content: row.get(2)?,
                    memory_type: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    source: row.get(4)?,
                    importance: row.get::<_, String>(5)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    confidence: row.get::<_, String>(6)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    workspace_id: row.get(7)?,
                    session_id: row.get(8)?,
                    trace_id: row.get(9)?,
                    agent_id: row.get(10)?,
                    agent_model: row.get(11)?,
                    valid_from: row.get(12)?,
                    valid_until: row.get(13)?,
                    archived: row.get::<_, i64>(14)? != 0,
                    category_id: row.get(15)?,
                    created_at: row.get(16)?,
                    updated_at: row.get(17)?,
                })
            },
        ) {
            Ok(m) => m,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        self.conn.execute(
            "INSERT INTO access_log (memory_id, access_type) VALUES (?1, 'recall')",
            params![id_str],
        )?;

        let tags = self.load_tags(&id_str)?;
        let relationships = self.load_relationships(&id_str)?;
        let category = self.load_category(memory.category_id)?;
        let access_count = self.count_access(&id_str)?;

        Ok(Some(MemoryWithRelations {
            memory,
            tags,
            relationships,
            category,
            access_count,
        }))
    }

    pub fn update(&self, id: &MemoryId, patch: &MemoryPatch) -> Result<bool> {
        let id_str = id.to_string();
        let tx = self.conn.unchecked_transaction()?;

        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1)",
            params![id_str],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }

        let mut sets = vec!["updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')".to_owned()];
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref title) = patch.title {
            sets.push(format!("title = ?{}", param_values.len() + 1));
            param_values.push(Box::new(title.clone()));
        }
        if let Some(ref content) = patch.content {
            sets.push(format!("content = ?{}", param_values.len() + 1));
            param_values.push(Box::new(content.clone()));
        }
        if let Some(ref importance) = patch.importance {
            sets.push(format!("importance = ?{}", param_values.len() + 1));
            param_values.push(Box::new(importance.to_string()));
        }
        if let Some(ref confidence) = patch.confidence {
            sets.push(format!("confidence = ?{}", param_values.len() + 1));
            param_values.push(Box::new(confidence.to_string()));
        }
        if let Some(ref valid_until) = patch.valid_until {
            sets.push(format!("valid_until = ?{}", param_values.len() + 1));
            param_values.push(Box::new(valid_until.clone()));
        }

        let idx = param_values.len() + 1;
        param_values.push(Box::new(id_str.clone()));

        let sql = format!(
            "UPDATE memories SET {} WHERE id = ?{}",
            sets.join(", "),
            idx
        );
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        tx.execute(&sql, params_ref.as_slice())?;

        if let Some(ref new_tags) = patch.tags {
            tx.execute(
                "DELETE FROM memory_tags WHERE memory_id = ?1",
                params![id_str],
            )?;
            for tag_name in new_tags {
                tx.execute(
                    "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
                    params![tag_name],
                )?;
                let tag_id: i64 = tx.query_row(
                    "SELECT id FROM tags WHERE name = ?1",
                    params![tag_name],
                    |row| row.get(0),
                )?;
                tx.execute(
                    "INSERT INTO memory_tags (memory_id, tag_id) VALUES (?1, ?2)",
                    params![id_str, tag_id],
                )?;
            }
        }

        tx.commit()?;
        Ok(true)
    }

    pub fn forget(&self, id: &MemoryId, hard: bool) -> Result<bool> {
        let id_str = id.to_string();

        let exists: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1)",
            params![id_str],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }

        if hard {
            self.conn
                .execute("DELETE FROM memories WHERE id = ?1", params![id_str])?;
        } else {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM memory_embeddings WHERE chunk_id IN (SELECT id FROM memory_chunks WHERE memory_id = ?1)",
                params![id_str],
            )?;
            tx.execute(
                "DELETE FROM memory_chunks WHERE memory_id = ?1",
                params![id_str],
            )?;
            tx.execute(
                "UPDATE memories SET archived = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![id_str],
            )?;
            tx.commit()?;
        }

        Ok(true)
    }

    pub fn unarchive(&self, id: &MemoryId) -> Result<bool> {
        let id_str = id.to_string();
        let changed = self.conn.execute(
            "UPDATE memories SET archived = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND archived = 1",
            params![id_str],
        )?;
        Ok(changed > 0)
    }

    pub fn relate(&self, from: &MemoryId, to: &MemoryId, relation: RelationType) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;

        tx.execute(
            "INSERT OR IGNORE INTO relationships (source_id, target_id, relation_type)
             VALUES (?1, ?2, ?3)",
            params![from.to_string(), to.to_string(), relation.to_string()],
        )?;

        if relation == RelationType::Supersedes && tx.changes() > 0 {
            tx.execute(
                "UPDATE memories SET valid_until = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![to.to_string()],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    pub fn unrelate(&self, from: &MemoryId, to: &MemoryId, relation: RelationType) -> Result<bool> {
        let changed = self.conn.execute(
            "DELETE FROM relationships WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3",
            params![from.to_string(), to.to_string(), relation.to_string()],
        )?;
        Ok(changed > 0)
    }

    fn load_tags(&self, memory_id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name FROM tags t
             JOIN memory_tags mt ON mt.tag_id = t.id
             WHERE mt.memory_id = ?1
             ORDER BY t.name",
        )?;
        let tags = stmt
            .query_map(params![memory_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(tags)
    }

    fn load_relationships(&self, memory_id: &str) -> Result<Vec<Relationship>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_id, target_id, relation_type, created_at
             FROM relationships
             WHERE source_id = ?1 OR target_id = ?1",
        )?;
        let rels = stmt
            .query_map(params![memory_id], |row| {
                Ok(Relationship {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    target_id: row.get(2)?,
                    relation_type: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    created_at: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rels)
    }

    fn load_category(&self, category_id: Option<i64>) -> Result<Option<Category>> {
        let Some(cat_id) = category_id else {
            return Ok(None);
        };
        match self.conn.query_row(
            "SELECT id, name, parent_id, source, created_at FROM categories WHERE id = ?1",
            params![cat_id],
            |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    parent_id: row.get(2)?,
                    source: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            e.into(),
                        )
                    })?,
                    created_at: row.get(4)?,
                })
            },
        ) {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn count_access(&self, memory_id: &str) -> Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM access_log WHERE memory_id = ?1",
            params![memory_id],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let found = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .any(|r| r.as_deref() == Ok(column));
    Ok(found)
}

fn migrate_models_columns(conn: &Connection) -> Result<()> {
    let alters: &[(&str, &str)] = &[
        ("variant", "ALTER TABLE models ADD COLUMN variant TEXT"),
        (
            "chunk_size",
            "ALTER TABLE models ADD COLUMN chunk_size INTEGER NOT NULL DEFAULT 512",
        ),
        (
            "chunk_overlap",
            "ALTER TABLE models ADD COLUMN chunk_overlap INTEGER NOT NULL DEFAULT 64",
        ),
        (
            "active",
            "ALTER TABLE models ADD COLUMN active INTEGER NOT NULL DEFAULT 0",
        ),
    ];
    for (col, sql) in alters {
        if !has_column(conn, "models", col)? {
            conn.execute_batch(sql)?;
        }
    }
    Ok(())
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
