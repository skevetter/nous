use nous_shared::ids::MemoryId;
use nous_shared::sqlite::{open_connection, run_migrations};
use nous_shared::{NousError, Result};
use rusqlite::{Connection, params};

use crate::chunk::Chunk;
use crate::types::{
    Category, CategorySource, CategoryTree, Memory, MemoryPatch, MemoryWithRelations, NewMemory,
    RelationType, Relationship,
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
    "CREATE INDEX IF NOT EXISTS idx_access_log_time ON access_log(accessed_at)",
];

pub struct MemoryDb {
    conn: Connection,
}

impl MemoryDb {
    pub fn open(path: &str, key: Option<&str>, dimensions: usize) -> Result<Self> {
        let conn = open_connection(path, key)?;
        crate::sqlite_vec::load(&conn)?;
        run_migrations(&conn, MIGRATIONS)?;
        migrate_models_columns(&conn)?;
        seed_placeholder_model(&conn)?;
        ensure_vec0_table(&conn, dimensions)?;
        migrate_categories_columns(&conn)?;
        seed_categories(&conn)?;
        Ok(Self { conn })
    }

    pub fn from_connection(conn: Connection) -> Self {
        Self { conn }
    }

    pub fn reset_embeddings(&self, new_dim: usize) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute_batch("DROP TABLE IF EXISTS memory_embeddings")?;
        tx.execute_batch(&format!(
            "CREATE VIRTUAL TABLE memory_embeddings USING vec0(
                chunk_id TEXT PRIMARY KEY, embedding float[{new_dim}]
            )"
        ))?;
        tx.commit()?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn store(&self, memory: &NewMemory) -> Result<MemoryId> {
        let tx = self.conn.unchecked_transaction()?;
        let id = Self::store_on(&tx, memory)?;
        tx.commit()?;
        Ok(id)
    }

    pub(crate) fn store_on(conn: &Connection, memory: &NewMemory) -> Result<MemoryId> {
        let id = MemoryId::new();

        let workspace_id: Option<i64> = if let Some(ref path) = memory.workspace_path {
            conn.execute(
                "INSERT OR IGNORE INTO workspaces (path) VALUES (?1)",
                params![path],
            )?;
            Some(conn.query_row(
                "SELECT id FROM workspaces WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )?)
        } else {
            None
        };

        conn.execute(
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
            conn.execute(
                "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
                params![tag_name],
            )?;
            let tag_id: i64 = conn.query_row(
                "SELECT id FROM tags WHERE name = ?1",
                params![tag_name],
                |row| row.get(0),
            )?;
            conn.execute(
                "INSERT INTO memory_tags (memory_id, tag_id) VALUES (?1, ?2)",
                params![id.to_string(), tag_id],
            )?;
        }

        Ok(id)
    }

    pub fn recall(&self, id: &MemoryId) -> Result<Option<MemoryWithRelations>> {
        let result = Self::recall_on(&self.conn, id)?;
        if result.is_some() {
            self.conn.execute(
                "INSERT INTO access_log (memory_id, access_type) VALUES (?1, 'recall')",
                params![id.to_string()],
            )?;
        }
        Ok(result)
    }

    pub fn recall_on(conn: &Connection, id: &MemoryId) -> Result<Option<MemoryWithRelations>> {
        let id_str = id.to_string();

        let memory = match conn.query_row(
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

        let tags = Self::load_tags_on(conn, &id_str)?;
        let relationships = Self::load_relationships_on(conn, &id_str)?;
        let category = Self::load_category_on(conn, memory.category_id)?;
        let access_count = Self::count_access_on(conn, &id_str)?;

        Ok(Some(MemoryWithRelations {
            memory,
            tags,
            relationships,
            category,
            access_count,
        }))
    }

    pub fn update(&self, id: &MemoryId, patch: &MemoryPatch) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let result = Self::update_on(&tx, id, patch)?;
        tx.commit()?;
        Ok(result)
    }

    pub(crate) fn update_on(conn: &Connection, id: &MemoryId, patch: &MemoryPatch) -> Result<bool> {
        let id_str = id.to_string();

        let exists: bool = conn.query_row(
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
        conn.execute(&sql, params_ref.as_slice())?;

        if let Some(ref new_tags) = patch.tags {
            conn.execute(
                "DELETE FROM memory_tags WHERE memory_id = ?1",
                params![id_str],
            )?;
            for tag_name in new_tags {
                conn.execute(
                    "INSERT OR IGNORE INTO tags (name) VALUES (?1)",
                    params![tag_name],
                )?;
                let tag_id: i64 = conn.query_row(
                    "SELECT id FROM tags WHERE name = ?1",
                    params![tag_name],
                    |row| row.get(0),
                )?;
                conn.execute(
                    "INSERT INTO memory_tags (memory_id, tag_id) VALUES (?1, ?2)",
                    params![id_str, tag_id],
                )?;
            }
        }

        Ok(true)
    }

    pub fn forget(&self, id: &MemoryId, hard: bool) -> Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let result = Self::forget_on(&tx, id, hard)?;
        tx.commit()?;
        Ok(result)
    }

    pub(crate) fn forget_on(conn: &Connection, id: &MemoryId, hard: bool) -> Result<bool> {
        let id_str = id.to_string();

        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1)",
            params![id_str],
            |row| row.get(0),
        )?;
        if !exists {
            return Ok(false);
        }

        if hard {
            conn.execute("DELETE FROM memories WHERE id = ?1", params![id_str])?;
        } else {
            {
                let chunk_ids: Vec<String> = {
                    let mut stmt =
                        conn.prepare("SELECT id FROM memory_chunks WHERE memory_id = ?1")?;
                    stmt.query_map(params![id_str], |row| row.get(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                };
                for chunk_id in &chunk_ids {
                    conn.execute(
                        "DELETE FROM memory_embeddings WHERE chunk_id = ?1",
                        params![chunk_id],
                    )?;
                }
            }
            conn.execute(
                "DELETE FROM memory_chunks WHERE memory_id = ?1",
                params![id_str],
            )?;
            conn.execute(
                "UPDATE memories SET archived = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![id_str],
            )?;
        }

        Ok(true)
    }

    pub fn unarchive(&self, id: &MemoryId) -> Result<bool> {
        Self::unarchive_on(&self.conn, id)
    }

    pub(crate) fn unarchive_on(conn: &Connection, id: &MemoryId) -> Result<bool> {
        let id_str = id.to_string();
        let changed = conn.execute(
            "UPDATE memories SET archived = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND archived = 1",
            params![id_str],
        )?;
        Ok(changed > 0)
    }

    pub fn relate(&self, from: &MemoryId, to: &MemoryId, relation: RelationType) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        Self::relate_on(&tx, from, to, relation)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn relate_on(
        conn: &Connection,
        from: &MemoryId,
        to: &MemoryId,
        relation: RelationType,
    ) -> Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO relationships (source_id, target_id, relation_type)
             VALUES (?1, ?2, ?3)",
            params![from.to_string(), to.to_string(), relation.to_string()],
        )?;

        if relation == RelationType::Supersedes && conn.changes() > 0 {
            conn.execute(
                "UPDATE memories SET valid_until = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                params![to.to_string()],
            )?;
        }

        Ok(())
    }

    pub fn unrelate(&self, from: &MemoryId, to: &MemoryId, relation: RelationType) -> Result<bool> {
        Self::unrelate_on(&self.conn, from, to, relation)
    }

    pub(crate) fn unrelate_on(
        conn: &Connection,
        from: &MemoryId,
        to: &MemoryId,
        relation: RelationType,
    ) -> Result<bool> {
        let changed = conn.execute(
            "DELETE FROM relationships WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3",
            params![from.to_string(), to.to_string(), relation.to_string()],
        )?;
        Ok(changed > 0)
    }

    pub fn category_add(
        &self,
        name: &str,
        parent_id: Option<i64>,
        description: Option<&str>,
        source: CategorySource,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO categories (name, parent_id, description, source) VALUES (?1, ?2, ?3, ?4)",
            params![name, parent_id, description, source.to_string()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn category_list(
        &self,
        source_filter: Option<CategorySource>,
    ) -> Result<Vec<CategoryTree>> {
        let categories = match source_filter {
            Some(ref src) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, name, parent_id, source, description, embedding, created_at FROM categories WHERE source = ?1",
                )?;
                stmt.query_map(params![src.to_string()], Self::row_to_category)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
            None => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, name, parent_id, source, description, embedding, created_at FROM categories",
                )?;
                stmt.query_map([], Self::row_to_category)?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            }
        };

        Ok(build_tree(categories))
    }

    pub fn category_suggest(
        &self,
        name: &str,
        description: Option<&str>,
        parent_id: Option<i64>,
        memory_id: &MemoryId,
    ) -> Result<i64> {
        Self::category_suggest_on(&self.conn, name, description, parent_id, memory_id)
    }

    pub(crate) fn category_suggest_on(
        conn: &Connection,
        name: &str,
        description: Option<&str>,
        parent_id: Option<i64>,
        memory_id: &MemoryId,
    ) -> Result<i64> {
        conn.execute(
            "INSERT INTO categories (name, parent_id, description, source) VALUES (?1, ?2, ?3, 'agent')",
            params![name, parent_id, description],
        )?;
        let id = conn.last_insert_rowid();
        conn.execute(
            "UPDATE memories SET category_id = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?2",
            params![id, memory_id.to_string()],
        )?;
        Ok(id)
    }

    fn row_to_category(row: &rusqlite::Row<'_>) -> rusqlite::Result<Category> {
        Ok(Category {
            id: row.get(0)?,
            name: row.get(1)?,
            parent_id: row.get(2)?,
            source: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, e.into())
            })?,
            description: row.get(4)?,
            embedding: row.get(5)?,
            created_at: row.get(6)?,
        })
    }

    pub fn load_tags_on(conn: &Connection, memory_id: &str) -> Result<Vec<String>> {
        let mut stmt = conn.prepare(
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

    pub fn load_relationships_on(conn: &Connection, memory_id: &str) -> Result<Vec<Relationship>> {
        let mut stmt = conn.prepare(
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

    pub(crate) fn load_category_on(
        conn: &Connection,
        category_id: Option<i64>,
    ) -> Result<Option<Category>> {
        let Some(cat_id) = category_id else {
            return Ok(None);
        };
        match conn.query_row(
            "SELECT id, name, parent_id, source, description, embedding, created_at FROM categories WHERE id = ?1",
            params![cat_id],
            |row| {
                Ok(Category {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    parent_id: row.get(2)?,
                    source: row.get::<_, String>(3)?.parse().map_err(|e: String| {
                        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, e.into())
                    })?,
                    description: row.get(4)?,
                    embedding: row.get(5)?,
                    created_at: row.get(6)?,
                })
            },
        ) {
            Ok(c) => Ok(Some(c)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn log_access(&self, memory_id: &MemoryId, tool_name: &str) -> Result<()> {
        Self::log_access_on(&self.conn, memory_id, tool_name)
    }

    pub(crate) fn log_access_on(
        conn: &Connection,
        memory_id: &MemoryId,
        access_type: &str,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO access_log (memory_id, access_type) VALUES (?1, ?2)",
            params![memory_id.to_string(), access_type],
        )?;
        Ok(())
    }

    pub fn most_accessed(&self, since: Option<&str>, limit: usize) -> Result<Vec<(String, u64)>> {
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match since {
            Some(ts) => (
                "SELECT memory_id, COUNT(*) as cnt FROM access_log \
                 WHERE accessed_at >= ?1 GROUP BY memory_id ORDER BY cnt DESC LIMIT ?2"
                    .into(),
                vec![Box::new(ts.to_owned()), Box::new(limit as i64)],
            ),
            None => (
                "SELECT memory_id, COUNT(*) as cnt FROM access_log \
                 GROUP BY memory_id ORDER BY cnt DESC LIMIT ?1"
                    .into(),
                vec![Box::new(limit as i64)],
            ),
        };

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn access_count(&self, memory_id: &MemoryId) -> Result<u64> {
        Self::count_access_on(&self.conn, &memory_id.to_string())
    }

    pub(crate) fn count_access_on(conn: &Connection, memory_id: &str) -> Result<u64> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM access_log WHERE memory_id = ?1",
            params![memory_id],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    pub fn store_chunks(
        &self,
        memory_id: &MemoryId,
        chunks: &[Chunk],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        Self::store_chunks_on(&self.conn, memory_id, chunks, embeddings)
    }

    pub(crate) fn store_chunks_on(
        conn: &Connection,
        memory_id: &MemoryId,
        chunks: &[Chunk],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        if chunks.len() != embeddings.len() {
            return Err(NousError::Internal(format!(
                "chunks length {} != embeddings length {}",
                chunks.len(),
                embeddings.len()
            )));
        }
        let memory_id_str = memory_id.to_string();

        for (i, chunk) in chunks.iter().enumerate() {
            let chunk_id = format!("{memory_id_str}:{i}");
            let token_count = chunk.text.split_whitespace().count() as i64;

            conn.execute(
                "INSERT INTO memory_chunks (id, memory_id, chunk_index, content, token_count, model_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, (SELECT id FROM models WHERE name = 'placeholder'))",
                params![chunk_id, memory_id_str, i as i64, chunk.text, token_count],
            )?;

            let blob: Vec<u8> = embeddings[i].iter().flat_map(|f| f.to_le_bytes()).collect();

            conn.execute(
                "DELETE FROM memory_embeddings WHERE chunk_id = ?1",
                params![chunk_id],
            )?;
            conn.execute(
                "INSERT INTO memory_embeddings (chunk_id, embedding) VALUES (?1, ?2)",
                params![chunk_id, blob],
            )?;
        }

        Ok(())
    }

    pub fn delete_chunks(&self, memory_id: &MemoryId) -> Result<()> {
        Self::delete_chunks_on(&self.conn, memory_id)
    }

    pub(crate) fn delete_chunks_on(conn: &Connection, memory_id: &MemoryId) -> Result<()> {
        let memory_id_str = memory_id.to_string();

        let chunk_ids: Vec<String> = {
            let mut stmt = conn.prepare("SELECT id FROM memory_chunks WHERE memory_id = ?1")?;
            stmt.query_map(params![memory_id_str], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for chunk_id in &chunk_ids {
            conn.execute(
                "DELETE FROM memory_embeddings WHERE chunk_id = ?1",
                params![chunk_id],
            )?;
        }
        conn.execute(
            "DELETE FROM memory_chunks WHERE memory_id = ?1",
            params![memory_id_str],
        )?;

        Ok(())
    }

    pub fn workspaces_on(conn: &Connection) -> Result<Vec<(crate::types::Workspace, i64)>> {
        let mut stmt = conn.prepare(
            "SELECT w.id, w.path, w.name, w.created_at, COUNT(m.id) as memory_count
             FROM workspaces w
             LEFT JOIN memories m ON m.workspace_id = w.id AND m.archived = 0
             GROUP BY w.id
             ORDER BY memory_count DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    crate::types::Workspace {
                        id: row.get(0)?,
                        path: row.get(1)?,
                        name: row.get(2)?,
                        created_at: row.get(3)?,
                    },
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn tags_on(conn: &Connection) -> Result<Vec<(String, i64)>> {
        let mut stmt = conn.prepare(
            "SELECT t.name, COUNT(mt.memory_id) as usage_count
             FROM tags t
             JOIN memory_tags mt ON mt.tag_id = t.id
             GROUP BY t.id
             ORDER BY usage_count DESC, t.name ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn stats_on(conn: &Connection) -> Result<serde_json::Value> {
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived = 0",
            [],
            |row| row.get(0),
        )?;
        let archived: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE archived = 1",
            [],
            |row| row.get(0),
        )?;

        let mut by_type = serde_json::Map::new();
        {
            let mut stmt =
                conn.prepare("SELECT memory_type, COUNT(*) FROM memories WHERE archived = 0 GROUP BY memory_type")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (k, v) = row?;
                by_type.insert(k, serde_json::Value::Number(v.into()));
            }
        }

        let mut by_importance = serde_json::Map::new();
        {
            let mut stmt = conn.prepare(
                "SELECT importance, COUNT(*) FROM memories WHERE archived = 0 GROUP BY importance",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (k, v) = row?;
                by_importance.insert(k, serde_json::Value::Number(v.into()));
            }
        }

        let mut by_workspace = serde_json::Map::new();
        {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(w.path, 'unassigned'), COUNT(*)
                 FROM memories m
                 LEFT JOIN workspaces w ON w.id = m.workspace_id
                 WHERE m.archived = 0
                 GROUP BY m.workspace_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (k, v) = row?;
                by_workspace.insert(k, serde_json::Value::Number(v.into()));
            }
        }

        let mut by_category = serde_json::Map::new();
        {
            let mut stmt = conn.prepare(
                "SELECT COALESCE(c.name, 'uncategorized'), COUNT(*)
                 FROM memories m
                 LEFT JOIN categories c ON c.id = m.category_id
                 WHERE m.archived = 0
                 GROUP BY m.category_id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (k, v) = row?;
                by_category.insert(k, serde_json::Value::Number(v.into()));
            }
        }

        let mut top_tags = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT t.name, COUNT(mt.memory_id) as cnt
                 FROM tags t JOIN memory_tags mt ON mt.tag_id = t.id
                 GROUP BY t.id ORDER BY cnt DESC LIMIT 20",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (name, count) = row?;
                top_tags.push(serde_json::json!({"tag": name, "count": count}));
            }
        }

        let total_accesses: i64 =
            conn.query_row("SELECT COUNT(*) FROM access_log", [], |row| row.get(0))?;

        Ok(serde_json::json!({
            "total": total,
            "archived": archived,
            "by_type": by_type,
            "by_importance": by_importance,
            "by_workspace": by_workspace,
            "by_category": by_category,
            "top_tags": top_tags,
            "total_accesses": total_accesses,
        }))
    }

    pub fn schema_on(conn: &Connection) -> Result<String> {
        let mut stmt = conn.prepare(
            "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type DESC, name ASC",
        )?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows.join(";\n\n") + ";")
    }
}

pub fn build_validity_clause(valid_only: Option<bool>) -> Option<String> {
    match valid_only {
        Some(true) => Some("AND (valid_until IS NULL OR valid_until > datetime('now'))".into()),
        _ => None,
    }
}

fn build_tree(categories: Vec<Category>) -> Vec<CategoryTree> {
    use std::collections::HashMap;

    let mut children_map: HashMap<Option<i64>, Vec<Category>> = HashMap::new();
    for cat in categories {
        children_map.entry(cat.parent_id).or_default().push(cat);
    }

    fn build_children(
        parent_id: Option<i64>,
        children_map: &mut HashMap<Option<i64>, Vec<Category>>,
    ) -> Vec<CategoryTree> {
        let cats = children_map.remove(&parent_id).unwrap_or_default();
        cats.into_iter()
            .map(|cat| {
                let id = cat.id;
                let children = build_children(Some(id), children_map);
                CategoryTree {
                    category: cat,
                    children,
                }
            })
            .collect()
    }

    build_children(None, &mut children_map)
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

pub fn ensure_vec0_table(conn: &Connection, dimensions: usize) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memory_embeddings'",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE memory_embeddings USING vec0(
                chunk_id TEXT PRIMARY KEY,
                embedding float[{dimensions}]
            )"
        ))?;
    }
    Ok(())
}

fn seed_placeholder_model(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO models (name, dimensions, max_tokens) VALUES ('placeholder', 0, 0)",
        [],
    )?;
    Ok(())
}

fn migrate_categories_columns(conn: &Connection) -> Result<()> {
    let alters: &[(&str, &str)] = &[
        (
            "description",
            "ALTER TABLE categories ADD COLUMN description TEXT",
        ),
        (
            "embedding",
            "ALTER TABLE categories ADD COLUMN embedding BLOB",
        ),
    ];
    for (col, sql) in alters {
        if !has_column(conn, "categories", col)? {
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
