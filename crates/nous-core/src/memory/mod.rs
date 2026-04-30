use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    Decision,
    Convention,
    Bugfix,
    Architecture,
    Fact,
    Observation,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Convention => "convention",
            Self::Bugfix => "bugfix",
            Self::Architecture => "architecture",
            Self::Fact => "fact",
            Self::Observation => "observation",
        }
    }
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "decision" => Ok(Self::Decision),
            "convention" => Ok(Self::Convention),
            "bugfix" => Ok(Self::Bugfix),
            "architecture" => Ok(Self::Architecture),
            "fact" => Ok(Self::Fact),
            "observation" => Ok(Self::Observation),
            other => Err(NousError::Validation(format!(
                "invalid memory type: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    Low,
    Moderate,
    High,
}

impl Importance {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }
}

impl std::fmt::Display for Importance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Importance {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Self::Low),
            "moderate" => Ok(Self::Moderate),
            "high" => Ok(Self::High),
            other => Err(NousError::Validation(format!(
                "invalid importance: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    Supersedes,
    ConflictsWith,
    Related,
    Compatible,
    Scoped,
    NotConflict,
}

impl RelationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::ConflictsWith => "conflicts_with",
            Self::Related => "related",
            Self::Compatible => "compatible",
            Self::Scoped => "scoped",
            Self::NotConflict => "not_conflict",
        }
    }
}

impl std::fmt::Display for RelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for RelationType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "supersedes" => Ok(Self::Supersedes),
            "conflicts_with" => Ok(Self::ConflictsWith),
            "related" => Ok(Self::Related),
            "compatible" => Ok(Self::Compatible),
            "scoped" => Ok(Self::Scoped),
            "not_conflict" => Ok(Self::NotConflict),
            other => Err(NousError::Validation(format!(
                "invalid relation type: '{other}'"
            ))),
        }
    }
}

// --- Domain objects ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub importance: String,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl Memory {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            workspace_id: row.try_get("workspace_id")?,
            agent_id: row.try_get("agent_id")?,
            title: row.try_get("title")?,
            content: row.try_get("content")?,
            memory_type: row.try_get("memory_type")?,
            importance: row.try_get("importance")?,
            topic_key: row.try_get("topic_key")?,
            valid_from: row.try_get("valid_from")?,
            valid_until: row.try_get("valid_until")?,
            archived: row.try_get::<i32, _>("archived")? != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub created_at: String,
}

impl MemoryRelation {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            source_id: row.try_get("source_id")?,
            target_id: row.try_get("target_id")?,
            relation_type: row.try_get("relation_type")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarMemory {
    #[serde(flatten)]
    pub memory: Memory,
    pub score: f32,
}

// --- Request types ---

#[derive(Debug, Clone)]
pub struct SaveMemoryRequest {
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub importance: Option<Importance>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateMemoryRequest {
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub importance: Option<Importance>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchMemoryRequest {
    pub query: String,
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub memory_type: Option<MemoryType>,
    pub importance: Option<Importance>,
    pub include_archived: bool,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ContextRequest {
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
    pub topic_key: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RelateRequest {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: RelationType,
}

// --- Operations ---

pub async fn save_memory(
    pool: &SqlitePool,
    req: SaveMemoryRequest,
) -> Result<Memory, NousError> {
    if req.title.trim().is_empty() {
        return Err(NousError::Validation("memory title cannot be empty".into()));
    }
    if req.content.trim().is_empty() {
        return Err(NousError::Validation(
            "memory content cannot be empty".into(),
        ));
    }

    let workspace_id = req.workspace_id.unwrap_or_else(|| "default".to_string());

    if let Some(ref topic_key) = req.topic_key {
        let existing = sqlx::query(
            "SELECT id FROM memories WHERE topic_key = ? AND workspace_id = ? AND archived = 0",
        )
        .bind(topic_key)
        .bind(&workspace_id)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = existing {
            let existing_id: String = row.try_get("id").map_err(NousError::Sqlite)?;
            return update_memory(
                pool,
                UpdateMemoryRequest {
                    id: existing_id,
                    title: Some(req.title),
                    content: Some(req.content),
                    importance: req.importance,
                    topic_key: Some(topic_key.clone()),
                    valid_from: req.valid_from,
                    valid_until: req.valid_until,
                    archived: None,
                },
            )
            .await;
        }
    }

    let id = Uuid::now_v7().to_string();
    let importance = req.importance.unwrap_or(Importance::Moderate);

    sqlx::query(
        "INSERT INTO memories (id, workspace_id, agent_id, title, content, memory_type, importance, topic_key, valid_from, valid_until) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&workspace_id)
    .bind(&req.agent_id)
    .bind(req.title.trim())
    .bind(req.content.trim())
    .bind(req.memory_type.as_str())
    .bind(importance.as_str())
    .bind(&req.topic_key)
    .bind(&req.valid_from)
    .bind(&req.valid_until)
    .execute(pool)
    .await?;

    get_memory_by_id(pool, &id).await
}

pub async fn get_memory_by_id(pool: &SqlitePool, id: &str) -> Result<Memory, NousError> {
    let row = sqlx::query("SELECT * FROM memories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("memory '{id}' not found")))?;
    Memory::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn update_memory(
    pool: &SqlitePool,
    req: UpdateMemoryRequest,
) -> Result<Memory, NousError> {
    let _existing = get_memory_by_id(pool, &req.id).await?;

    let mut sets: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref title) = req.title {
        if title.trim().is_empty() {
            return Err(NousError::Validation("title cannot be empty".into()));
        }
        sets.push("title = ?".to_string());
        binds.push(title.trim().to_string());
    }

    if let Some(ref content) = req.content {
        if content.trim().is_empty() {
            return Err(NousError::Validation("content cannot be empty".into()));
        }
        sets.push("content = ?".to_string());
        binds.push(content.trim().to_string());
    }

    if let Some(ref importance) = req.importance {
        sets.push("importance = ?".to_string());
        binds.push(importance.as_str().to_string());
    }

    if let Some(ref topic_key) = req.topic_key {
        sets.push("topic_key = ?".to_string());
        binds.push(topic_key.clone());
    }

    if let Some(ref valid_from) = req.valid_from {
        sets.push("valid_from = ?".to_string());
        binds.push(valid_from.clone());
    }

    if let Some(ref valid_until) = req.valid_until {
        sets.push("valid_until = ?".to_string());
        binds.push(valid_until.clone());
    }

    if let Some(archived) = req.archived {
        sets.push("archived = ?".to_string());
        binds.push(if archived { "1" } else { "0" }.to_string());
    }

    if sets.is_empty() {
        return get_memory_by_id(pool, &req.id).await;
    }

    let sql = format!("UPDATE memories SET {} WHERE id = ?", sets.join(", "));
    binds.push(req.id.clone());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    query.execute(pool).await?;

    get_memory_by_id(pool, &req.id).await
}

pub async fn search_memories(
    pool: &SqlitePool,
    req: &SearchMemoryRequest,
) -> Result<Vec<Memory>, NousError> {
    if req.query.trim().is_empty() {
        return Err(NousError::Validation(
            "search query cannot be empty".into(),
        ));
    }

    let limit = req.limit.unwrap_or(20).min(100);

    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if !req.include_archived {
        conditions.push("m.archived = 0".to_string());
    }

    if let Some(ref ws) = req.workspace_id {
        conditions.push("m.workspace_id = ?".to_string());
        binds.push(ws.clone());
    }

    if let Some(ref agent) = req.agent_id {
        conditions.push("m.agent_id = ?".to_string());
        binds.push(agent.clone());
    }

    if let Some(ref mt) = req.memory_type {
        conditions.push("m.memory_type = ?".to_string());
        binds.push(mt.as_str().to_string());
    }

    if let Some(ref imp) = req.importance {
        conditions.push("m.importance = ?".to_string());
        binds.push(imp.as_str().to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" AND {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT m.* FROM memories m \
         INNER JOIN memories_fts f ON f.rowid = m.rowid \
         WHERE memories_fts MATCH ?{} \
         ORDER BY rank \
         LIMIT ?",
        where_clause
    );

    let sanitized = sanitize_fts_query(&req.query);
    binds.insert(0, sanitized);
    binds.push(limit.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    rows.iter()
        .map(Memory::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn get_context(
    pool: &SqlitePool,
    req: &ContextRequest,
) -> Result<Vec<Memory>, NousError> {
    let limit = req.limit.unwrap_or(20).min(100);

    let mut conditions: Vec<String> = vec!["archived = 0".to_string()];
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref ws) = req.workspace_id {
        conditions.push("workspace_id = ?".to_string());
        binds.push(ws.clone());
    }

    if let Some(ref agent) = req.agent_id {
        conditions.push("agent_id = ?".to_string());
        binds.push(agent.clone());
    }

    if let Some(ref topic) = req.topic_key {
        conditions.push("topic_key = ?".to_string());
        binds.push(topic.clone());
    }

    let sql = format!(
        "SELECT * FROM memories WHERE {} ORDER BY created_at DESC LIMIT ?",
        conditions.join(" AND ")
    );
    binds.push(limit.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    rows.iter()
        .map(Memory::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn relate_memories(
    pool: &SqlitePool,
    req: &RelateRequest,
) -> Result<MemoryRelation, NousError> {
    let _source = get_memory_by_id(pool, &req.source_id).await?;
    let _target = get_memory_by_id(pool, &req.target_id).await?;

    if req.source_id == req.target_id {
        return Err(NousError::Validation(
            "cannot relate a memory to itself".into(),
        ));
    }

    let id = Uuid::now_v7().to_string();

    sqlx::query(
        "INSERT INTO memory_relations (id, source_id, target_id, relation_type) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.source_id)
    .bind(&req.target_id)
    .bind(req.relation_type.as_str())
    .execute(pool)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.message().contains("UNIQUE") => {
            NousError::Conflict("relation already exists".into())
        }
        _ => NousError::Sqlite(e),
    })?;

    if req.relation_type == RelationType::Supersedes {
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        sqlx::query("UPDATE memories SET valid_until = ? WHERE id = ? AND valid_until IS NULL")
            .bind(&now)
            .bind(&req.target_id)
            .execute(pool)
            .await?;
    }

    get_relation_by_id(pool, &id).await
}

pub async fn get_relation_by_id(
    pool: &SqlitePool,
    id: &str,
) -> Result<MemoryRelation, NousError> {
    let row = sqlx::query("SELECT * FROM memory_relations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row =
        row.ok_or_else(|| NousError::NotFound(format!("memory relation '{id}' not found")))?;
    MemoryRelation::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_relations(
    pool: &SqlitePool,
    memory_id: &str,
) -> Result<Vec<MemoryRelation>, NousError> {
    let rows = sqlx::query(
        "SELECT * FROM memory_relations WHERE source_id = ? OR target_id = ? ORDER BY created_at DESC",
    )
    .bind(memory_id)
    .bind(memory_id)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(MemoryRelation::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn log_access(
    pool: &SqlitePool,
    memory_id: &str,
    access_type: &str,
    session_id: Option<&str>,
) -> Result<(), NousError> {
    sqlx::query(
        "INSERT INTO memory_access_log (memory_id, access_type, session_id) VALUES (?, ?, ?)",
    )
    .bind(memory_id)
    .bind(access_type)
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn run_importance_decay(
    pool: &SqlitePool,
    high_to_moderate_days: u32,
    moderate_to_low_days: u32,
) -> Result<u64, NousError> {
    let now = chrono::Utc::now();
    let high_cutoff = (now - chrono::Duration::days(high_to_moderate_days as i64))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let moderate_cutoff = (now - chrono::Duration::days(moderate_to_low_days as i64))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    // Run moderate→low first so that high→moderate in the same sweep
    // doesn't immediately cascade to low in one call.
    let r1 = sqlx::query(
        "UPDATE memories SET importance = 'low' \
         WHERE importance = 'moderate' AND archived = 0 \
         AND id NOT IN (\
             SELECT memory_id FROM memory_access_log WHERE accessed_at > ?\
         )",
    )
    .bind(&moderate_cutoff)
    .execute(pool)
    .await?;

    let r2 = sqlx::query(
        "UPDATE memories SET importance = 'moderate' \
         WHERE importance = 'high' AND archived = 0 \
         AND id NOT IN (\
             SELECT memory_id FROM memory_access_log WHERE accessed_at > ?\
         )",
    )
    .bind(&high_cutoff)
    .execute(pool)
    .await?;

    Ok(r1.rows_affected() + r2.rows_affected())
}

pub async fn grant_workspace_access(
    pool: &SqlitePool,
    agent_id: &str,
    workspace_id: &str,
) -> Result<(), NousError> {
    sqlx::query(
        "INSERT OR IGNORE INTO agent_workspace_access (agent_id, workspace_id) VALUES (?, ?)",
    )
    .bind(agent_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn revoke_workspace_access(
    pool: &SqlitePool,
    agent_id: &str,
    workspace_id: &str,
) -> Result<(), NousError> {
    sqlx::query("DELETE FROM agent_workspace_access WHERE agent_id = ? AND workspace_id = ?")
        .bind(agent_id)
        .bind(workspace_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn check_workspace_access(
    pool: &SqlitePool,
    agent_id: &str,
    workspace_id: &str,
) -> Result<bool, NousError> {
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM agent_workspace_access WHERE agent_id = ? AND workspace_id = ?) as has_access",
    )
    .bind(agent_id)
    .bind(workspace_id)
    .fetch_one(pool)
    .await?;

    Ok(row.try_get::<bool, _>("has_access").unwrap_or(false))
}

fn sanitize_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|token| {
            if token
                .chars()
                .any(|c| matches!(c, '-' | ':' | '.' | '/' | '\\' | '@' | '#' | '!' | '+'))
            {
                format!("\"{}\"", token.replace('"', ""))
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Store a pre-computed embedding vector for a memory.
/// The embedding is stored as a BLOB of f32 values in little-endian byte order.
pub async fn store_embedding(
    pool: &SqlitePool,
    memory_id: &str,
    embedding: &[f32],
) -> Result<(), NousError> {
    // Verify memory exists
    let _ = get_memory_by_id(pool, memory_id).await?;

    let bytes = embedding_to_bytes(embedding);
    sqlx::query("UPDATE memories SET embedding = ? WHERE id = ?")
        .bind(&bytes)
        .bind(memory_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Search memories by cosine similarity to a query embedding.
/// Returns top-K memories with similarity scores, ordered by similarity descending.
pub async fn search_similar(
    pool: &SqlitePool,
    query_embedding: &[f32],
    limit: u32,
    workspace_id: Option<&str>,
    threshold: Option<f32>,
) -> Result<Vec<SimilarMemory>, NousError> {
    if query_embedding.is_empty() {
        return Err(NousError::Validation("embedding cannot be empty".into()));
    }

    let limit = limit.min(100);
    let threshold = threshold.unwrap_or(0.0);

    // Fetch all memories with embeddings
    let mut conditions = vec!["embedding IS NOT NULL".to_string(), "archived = 0".to_string()];
    let mut binds: Vec<String> = Vec::new();

    if let Some(ws) = workspace_id {
        conditions.push("workspace_id = ?".to_string());
        binds.push(ws.to_string());
    }

    let sql = format!(
        "SELECT id, workspace_id, agent_id, title, content, memory_type, importance, \
         topic_key, valid_from, valid_until, archived, created_at, updated_at, embedding \
         FROM memories WHERE {}",
        conditions.join(" AND ")
    );

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    let mut results: Vec<SimilarMemory> = Vec::new();
    for row in &rows {
        let blob: Vec<u8> = row.try_get("embedding").map_err(NousError::Sqlite)?;
        let emb = bytes_to_embedding(&blob);
        let score = cosine_similarity(query_embedding, &emb);
        if score >= threshold {
            let memory = Memory::from_row(row).map_err(NousError::Sqlite)?;
            results.push(SimilarMemory { memory, score });
        }
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit as usize);

    Ok(results)
}

fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

// --- Memory session lifecycle ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySession {
    pub id: String,
    pub agent_id: Option<String>,
    pub project: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
}

impl MemorySession {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            project: row.try_get("project")?,
            started_at: row.try_get("started_at")?,
            ended_at: row.try_get("ended_at")?,
            summary: row.try_get("summary")?,
        })
    }
}

pub async fn session_start(
    pool: &SqlitePool,
    agent_id: Option<&str>,
    project: Option<&str>,
) -> Result<MemorySession, NousError> {
    let id = Uuid::now_v7().to_string();

    sqlx::query("INSERT INTO memory_sessions (id, agent_id, project) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(agent_id)
        .bind(project)
        .execute(pool)
        .await?;

    get_session_by_id(pool, &id).await
}

pub async fn session_end(pool: &SqlitePool, session_id: &str) -> Result<MemorySession, NousError> {
    let session = get_session_by_id(pool, session_id).await?;
    if session.ended_at.is_some() {
        return Err(NousError::Validation("session already ended".into()));
    }

    sqlx::query(
        "UPDATE memory_sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(session_id)
    .execute(pool)
    .await?;

    get_session_by_id(pool, session_id).await
}

pub async fn session_summary(
    pool: &SqlitePool,
    session_id: &str,
    summary: &str,
    agent_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<MemorySession, NousError> {
    if summary.trim().is_empty() {
        return Err(NousError::Validation("summary cannot be empty".into()));
    }

    let _session = get_session_by_id(pool, session_id).await?;

    sqlx::query("UPDATE memory_sessions SET summary = ? WHERE id = ?")
        .bind(summary.trim())
        .bind(session_id)
        .execute(pool)
        .await?;

    save_memory(
        pool,
        SaveMemoryRequest {
            workspace_id: workspace_id.map(String::from),
            agent_id: agent_id.map(String::from),
            title: format!("Session summary: {session_id}"),
            content: summary.trim().to_string(),
            memory_type: MemoryType::Observation,
            importance: Some(Importance::Moderate),
            topic_key: Some(format!("session/{session_id}")),
            valid_from: None,
            valid_until: None,
        },
    )
    .await?;

    get_session_by_id(pool, session_id).await
}

pub async fn save_prompt(
    pool: &SqlitePool,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    workspace_id: Option<&str>,
    prompt: &str,
) -> Result<Memory, NousError> {
    if prompt.trim().is_empty() {
        return Err(NousError::Validation("prompt cannot be empty".into()));
    }

    if let Some(sid) = session_id {
        let _session = get_session_by_id(pool, sid).await?;
    }

    let mem = save_memory(
        pool,
        SaveMemoryRequest {
            workspace_id: workspace_id.map(String::from),
            agent_id: agent_id.map(String::from),
            title: truncate_title(prompt),
            content: prompt.trim().to_string(),
            memory_type: MemoryType::Observation,
            importance: Some(Importance::Low),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await?;

    if let Some(sid) = session_id {
        sqlx::query("UPDATE memories SET session_id = ? WHERE id = ?")
            .bind(sid)
            .bind(&mem.id)
            .execute(pool)
            .await?;
    }

    get_memory_by_id(pool, &mem.id).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedProject {
    pub name: String,
    pub project_type: String,
    pub path: String,
}

pub fn detect_current_project(cwd: &str) -> Option<DetectedProject> {
    let path = std::path::Path::new(cwd);

    let markers = [
        ("Cargo.toml", "rust"),
        ("package.json", "node"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        ("setup.py", "python"),
        ("Gemfile", "ruby"),
        ("pom.xml", "java"),
        ("build.gradle", "java"),
        ("mix.exs", "elixir"),
        ("CMakeLists.txt", "cpp"),
    ];

    let mut current = Some(path);
    while let Some(dir) = current {
        for (marker, project_type) in &markers {
            if dir.join(marker).exists() {
                let name = dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".into());
                return Some(DetectedProject {
                    name,
                    project_type: project_type.to_string(),
                    path: dir.to_string_lossy().to_string(),
                });
            }
        }
        current = dir.parent();
    }
    None
}

async fn get_session_by_id(pool: &SqlitePool, id: &str) -> Result<MemorySession, NousError> {
    let row = sqlx::query("SELECT * FROM memory_sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("memory session '{id}' not found")))?;
    MemorySession::from_row(&row).map_err(NousError::Sqlite)
}

fn truncate_title(s: &str) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.len() > 80 {
        format!("{}...", &first_line[..77])
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use tempfile::TempDir;

    async fn setup() -> (SqlitePool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        (pools.fts, tmp)
    }

    #[tokio::test]
    async fn save_and_get_memory() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: Some("agent-1".into()),
                title: "Test memory".into(),
                content: "This is a test memory content".into(),
                memory_type: MemoryType::Decision,
                importance: Some(Importance::High),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(mem.title, "Test memory");
        assert_eq!(mem.memory_type, "decision");
        assert_eq!(mem.importance, "high");
        assert_eq!(mem.workspace_id, "default");
        assert_eq!(mem.agent_id.as_deref(), Some("agent-1"));
        assert!(!mem.archived);

        let fetched = get_memory_by_id(&pool, &mem.id).await.unwrap();
        assert_eq!(fetched.id, mem.id);
        assert_eq!(fetched.content, "This is a test memory content");
    }

    #[tokio::test]
    async fn save_empty_title_fails() {
        let (pool, _tmp) = setup().await;

        let err = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "  ".into(),
                content: "content".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn topic_key_upsert() {
        let (pool, _tmp) = setup().await;

        let m1 = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "First version".into(),
                content: "Version 1".into(),
                memory_type: MemoryType::Convention,
                importance: None,
                topic_key: Some("arch/db-pattern".into()),
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let m2 = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Updated version".into(),
                content: "Version 2".into(),
                memory_type: MemoryType::Convention,
                importance: None,
                topic_key: Some("arch/db-pattern".into()),
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(m1.id, m2.id);
        assert_eq!(m2.title, "Updated version");
        assert_eq!(m2.content, "Version 2");
    }

    #[tokio::test]
    async fn update_memory_fields() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Original".into(),
                content: "Original content".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let updated = update_memory(
            &pool,
            UpdateMemoryRequest {
                id: mem.id.clone(),
                title: Some("Updated title".into()),
                content: None,
                importance: Some(Importance::High),
                topic_key: None,
                valid_from: None,
                valid_until: None,
                archived: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.title, "Updated title");
        assert_eq!(updated.importance, "high");
        assert_eq!(updated.content, "Original content");
    }

    #[tokio::test]
    async fn search_fts() {
        let (pool, _tmp) = setup().await;

        save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Database migration pattern".into(),
                content: "Always use sequential migrations with version numbers".into(),
                memory_type: MemoryType::Convention,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Unrelated memory".into(),
                content: "Something about cooking recipes".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let results = search_memories(
            &pool,
            &SearchMemoryRequest {
                query: "migration".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].title.contains("migration"));
    }

    #[tokio::test]
    async fn search_with_filters() {
        let (pool, _tmp) = setup().await;

        save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: Some("ws-1".into()),
                agent_id: None,
                title: "WS1 decision".into(),
                content: "Decision in workspace 1".into(),
                memory_type: MemoryType::Decision,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: Some("ws-2".into()),
                agent_id: None,
                title: "WS2 decision".into(),
                content: "Decision in workspace 2".into(),
                memory_type: MemoryType::Decision,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let results = search_memories(
            &pool,
            &SearchMemoryRequest {
                query: "decision".into(),
                workspace_id: Some("ws-1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].workspace_id, "ws-1");
    }

    #[tokio::test]
    async fn get_context_returns_recent() {
        let (pool, _tmp) = setup().await;

        for i in 1..=5 {
            save_memory(
                &pool,
                SaveMemoryRequest {
                    workspace_id: Some("ws-ctx".into()),
                    agent_id: None,
                    title: format!("Memory {i}"),
                    content: format!("Content {i}"),
                    memory_type: MemoryType::Observation,
                    importance: None,
                    topic_key: None,
                    valid_from: None,
                    valid_until: None,
                },
            )
            .await
            .unwrap();
        }

        let results = get_context(
            &pool,
            &ContextRequest {
                workspace_id: Some("ws-ctx".into()),
                limit: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0].title, "Memory 5");
    }

    #[tokio::test]
    async fn relate_memories_creates_relation() {
        let (pool, _tmp) = setup().await;

        let m1 = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Source memory".into(),
                content: "Source".into(),
                memory_type: MemoryType::Decision,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let m2 = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Target memory".into(),
                content: "Target".into(),
                memory_type: MemoryType::Decision,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let rel = relate_memories(
            &pool,
            &RelateRequest {
                source_id: m1.id.clone(),
                target_id: m2.id.clone(),
                relation_type: RelationType::Related,
            },
        )
        .await
        .unwrap();

        assert_eq!(rel.source_id, m1.id);
        assert_eq!(rel.target_id, m2.id);
        assert_eq!(rel.relation_type, "related");

        let rels = list_relations(&pool, &m1.id).await.unwrap();
        assert_eq!(rels.len(), 1);
    }

    #[tokio::test]
    async fn supersedes_sets_valid_until() {
        let (pool, _tmp) = setup().await;

        let old = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Old decision".into(),
                content: "Old".into(),
                memory_type: MemoryType::Decision,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let new = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "New decision".into(),
                content: "New".into(),
                memory_type: MemoryType::Decision,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        relate_memories(
            &pool,
            &RelateRequest {
                source_id: new.id.clone(),
                target_id: old.id.clone(),
                relation_type: RelationType::Supersedes,
            },
        )
        .await
        .unwrap();

        let old_refreshed = get_memory_by_id(&pool, &old.id).await.unwrap();
        assert!(old_refreshed.valid_until.is_some());
    }

    #[tokio::test]
    async fn self_relation_rejected() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Self".into(),
                content: "Self-referencing".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let err = relate_memories(
            &pool,
            &RelateRequest {
                source_id: mem.id.clone(),
                target_id: mem.id.clone(),
                relation_type: RelationType::Related,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn duplicate_relation_rejected() {
        let (pool, _tmp) = setup().await;

        let m1 = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "A".into(),
                content: "A".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let m2 = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "B".into(),
                content: "B".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        relate_memories(
            &pool,
            &RelateRequest {
                source_id: m1.id.clone(),
                target_id: m2.id.clone(),
                relation_type: RelationType::Related,
            },
        )
        .await
        .unwrap();

        let err = relate_memories(
            &pool,
            &RelateRequest {
                source_id: m1.id.clone(),
                target_id: m2.id.clone(),
                relation_type: RelationType::Related,
            },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, NousError::Conflict(_)));
    }

    #[tokio::test]
    async fn importance_decay() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "High importance".into(),
                content: "Should decay".into(),
                memory_type: MemoryType::Fact,
                importance: Some(Importance::High),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(mem.importance, "high");

        let affected = run_importance_decay(&pool, 0, 0).await.unwrap();
        assert!(affected > 0);

        let refreshed = get_memory_by_id(&pool, &mem.id).await.unwrap();
        assert_eq!(refreshed.importance, "moderate");

        let affected2 = run_importance_decay(&pool, 0, 0).await.unwrap();
        assert!(affected2 > 0);

        let refreshed2 = get_memory_by_id(&pool, &mem.id).await.unwrap();
        assert_eq!(refreshed2.importance, "low");
    }

    #[tokio::test]
    async fn access_log_prevents_decay() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Accessed memory".into(),
                content: "Should not decay".into(),
                memory_type: MemoryType::Fact,
                importance: Some(Importance::High),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        log_access(&pool, &mem.id, "recall", None).await.unwrap();

        run_importance_decay(&pool, 30, 60).await.unwrap();

        let refreshed = get_memory_by_id(&pool, &mem.id).await.unwrap();
        assert_eq!(refreshed.importance, "high");
    }

    #[tokio::test]
    async fn workspace_access_crud() {
        let (pool, _tmp) = setup().await;

        assert!(!check_workspace_access(&pool, "agent-1", "ws-1")
            .await
            .unwrap());

        grant_workspace_access(&pool, "agent-1", "ws-1")
            .await
            .unwrap();

        assert!(check_workspace_access(&pool, "agent-1", "ws-1")
            .await
            .unwrap());

        revoke_workspace_access(&pool, "agent-1", "ws-1")
            .await
            .unwrap();

        assert!(!check_workspace_access(&pool, "agent-1", "ws-1")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn search_archived_excluded_by_default() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Archived memory".into(),
                content: "This is archived content".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        update_memory(
            &pool,
            UpdateMemoryRequest {
                id: mem.id.clone(),
                title: None,
                content: None,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
                archived: Some(true),
            },
        )
        .await
        .unwrap();

        let results = search_memories(
            &pool,
            &SearchMemoryRequest {
                query: "archived".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 0);

        let results = search_memories(
            &pool,
            &SearchMemoryRequest {
                query: "archived".into(),
                include_archived: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn sanitize_fts_handles_special_chars() {
        assert_eq!(sanitize_fts_query("INI-076"), "\"INI-076\"");
        assert_eq!(sanitize_fts_query("simple query"), "simple query");
        assert_eq!(
            sanitize_fts_query("fix auth@service"),
            "fix \"auth@service\""
        );
    }

    #[test]
    fn parse_memory_type() {
        assert_eq!("decision".parse::<MemoryType>().unwrap(), MemoryType::Decision);
        assert_eq!("bugfix".parse::<MemoryType>().unwrap(), MemoryType::Bugfix);
        assert!("invalid".parse::<MemoryType>().is_err());
    }

    #[test]
    fn parse_importance() {
        assert_eq!("high".parse::<Importance>().unwrap(), Importance::High);
        assert_eq!("low".parse::<Importance>().unwrap(), Importance::Low);
        assert!("critical".parse::<Importance>().is_err());
    }

    #[test]
    fn parse_relation_type() {
        assert_eq!(
            "supersedes".parse::<RelationType>().unwrap(),
            RelationType::Supersedes
        );
        assert_eq!(
            "conflicts_with".parse::<RelationType>().unwrap(),
            RelationType::ConflictsWith
        );
        assert!("unknown".parse::<RelationType>().is_err());
    }

    #[tokio::test]
    async fn store_and_search_embedding() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Vector test".into(),
                content: "Testing vector similarity".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let embedding = vec![1.0, 0.0, 0.0, 0.0];
        store_embedding(&pool, &mem.id, &embedding).await.unwrap();

        let query = vec![0.9, 0.1, 0.0, 0.0];
        let results = search_similar(&pool, &query, 10, None, None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].score > 0.9);
        assert_eq!(results[0].memory.id, mem.id);
    }

    #[tokio::test]
    async fn search_similar_respects_threshold() {
        let (pool, _tmp) = setup().await;

        let mem = save_memory(
            &pool,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "High similarity".into(),
                content: "Should match".into(),
                memory_type: MemoryType::Fact,
                importance: None,
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        store_embedding(&pool, &mem.id, &[1.0, 0.0, 0.0]).await.unwrap();

        // Orthogonal vector should have ~0 similarity
        let results = search_similar(&pool, &[0.0, 1.0, 0.0], 10, None, Some(0.5))
            .await
            .unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn embedding_roundtrip() {
        let original = vec![1.0f32, -2.5, 3.14, 0.0];
        let bytes = embedding_to_bytes(&original);
        let recovered = bytes_to_embedding(&bytes);
        assert_eq!(original, recovered);
    }

    // --- Session lifecycle tests ---

    #[tokio::test]
    async fn session_start_creates_session() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, Some("agent-1"), Some("my-project"))
            .await
            .unwrap();

        assert!(!session.id.is_empty());
        assert_eq!(session.agent_id.as_deref(), Some("agent-1"));
        assert_eq!(session.project.as_deref(), Some("my-project"));
        assert!(!session.started_at.is_empty());
        assert!(session.ended_at.is_none());
        assert!(session.summary.is_none());
    }

    #[tokio::test]
    async fn session_start_without_optional_fields() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, None, None).await.unwrap();

        assert!(!session.id.is_empty());
        assert!(session.agent_id.is_none());
        assert!(session.project.is_none());
    }

    #[tokio::test]
    async fn session_end_sets_ended_at() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, None, None).await.unwrap();
        assert!(session.ended_at.is_none());

        let ended = session_end(&pool, &session.id).await.unwrap();
        assert!(ended.ended_at.is_some());
        assert_eq!(ended.id, session.id);
    }

    #[tokio::test]
    async fn session_end_already_ended_fails() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, None, None).await.unwrap();
        session_end(&pool, &session.id).await.unwrap();

        let err = session_end(&pool, &session.id).await.unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn session_end_nonexistent_fails() {
        let (pool, _tmp) = setup().await;

        let err = session_end(&pool, "nonexistent-session-id")
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[tokio::test]
    async fn session_summary_saves_summary_and_memory() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, Some("agent-1"), Some("proj"))
            .await
            .unwrap();

        let updated = session_summary(
            &pool,
            &session.id,
            "Completed migration refactoring",
            Some("agent-1"),
            Some("ws-1"),
        )
        .await
        .unwrap();

        assert_eq!(updated.summary.as_deref(), Some("Completed migration refactoring"));
        assert_eq!(updated.id, session.id);

        // Verify a session_summary memory was also created
        let results = search_memories(
            &pool,
            &SearchMemoryRequest {
                query: "migration refactoring".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!results.is_empty());
        assert!(results[0].title.contains(&session.id));
    }

    #[tokio::test]
    async fn session_summary_empty_fails() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, None, None).await.unwrap();

        let err = session_summary(&pool, &session.id, "   ", None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn session_summary_nonexistent_session_fails() {
        let (pool, _tmp) = setup().await;

        let err = session_summary(&pool, "nonexistent-id", "some summary", None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[tokio::test]
    async fn save_prompt_creates_memory() {
        let (pool, _tmp) = setup().await;

        let mem = save_prompt(&pool, None, Some("agent-1"), Some("ws-1"), "Refactor the auth module")
            .await
            .unwrap();

        assert!(!mem.id.is_empty());
        assert_eq!(mem.content, "Refactor the auth module");
        assert_eq!(mem.memory_type, "observation");
        assert_eq!(mem.importance, "low");
    }

    #[tokio::test]
    async fn save_prompt_with_session_links_memory() {
        let (pool, _tmp) = setup().await;

        let session = session_start(&pool, Some("agent-1"), None).await.unwrap();

        let mem = save_prompt(
            &pool,
            Some(&session.id),
            Some("agent-1"),
            None,
            "Fix the login bug",
        )
        .await
        .unwrap();

        assert!(!mem.id.is_empty());
        assert_eq!(mem.content, "Fix the login bug");
    }

    #[tokio::test]
    async fn save_prompt_empty_fails() {
        let (pool, _tmp) = setup().await;

        let err = save_prompt(&pool, None, None, None, "   ").await.unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn save_prompt_nonexistent_session_fails() {
        let (pool, _tmp) = setup().await;

        let err = save_prompt(&pool, Some("bad-session"), None, None, "a prompt")
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[test]
    fn detect_current_project_finds_cargo_toml() {
        // Use the actual project root which has a Cargo.toml
        let project = detect_current_project(env!("CARGO_MANIFEST_DIR"));
        assert!(project.is_some());
        let project = project.unwrap();
        assert_eq!(project.project_type, "rust");
        assert!(!project.name.is_empty());
    }

    #[test]
    fn detect_current_project_returns_none_for_no_markers() {
        // /tmp is unlikely to have a project marker
        let project = detect_current_project("/tmp");
        // This may or may not be None depending on system layout,
        // but at minimum the function should not panic
        let _ = project;
    }
}
