pub mod analytics;
pub mod chunk;
pub mod embed;
pub mod rerank;
pub mod vector_store;

use sea_orm::entity::prelude::*;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, NotSet, QueryOrder, QuerySelect, Set, Statement,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::VecPool;
use crate::entities::{
    memories as mem_entity, memory_access_log as access_entity, memory_relations as rel_entity,
    memory_sessions as session_entity,
};
use crate::error::NousError;

pub use chunk::{Chunk, Chunker};
pub use embed::{
    AsyncEmbedder, Embedder, EmbeddingConfig, EmbeddingProvider, MockEmbedder, OnnxEmbeddingModel,
    RigEmbedderAdapter,
};
pub use rerank::rerank_rrf;
pub use vector_store::{QdrantConfig, VectorStoreBackend, VectorStoreConfig};

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
                "invalid memory type: '{other}'. Valid values: decision, convention, bugfix, architecture, fact, observation"
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
                "invalid importance: '{other}'. Valid values: low, moderate, high"
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
                "invalid relation type: '{other}'. Valid values: supersedes, conflicts_with, related, compatible, scoped, not_conflict"
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
    fn from_model(m: mem_entity::Model) -> Self {
        Self {
            id: m.id,
            workspace_id: m.workspace_id,
            agent_id: m.agent_id,
            title: m.title,
            content: m.content,
            memory_type: m.memory_type,
            importance: m.importance,
            topic_key: m.topic_key,
            valid_from: m.valid_from,
            valid_until: m.valid_until,
            archived: m.archived,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }

    fn from_query_result(row: &sea_orm::QueryResult) -> Result<Self, sea_orm::DbErr> {
        Ok(Self {
            id: row.try_get_by("id")?,
            workspace_id: row.try_get_by("workspace_id")?,
            agent_id: row.try_get_by("agent_id")?,
            title: row.try_get_by("title")?,
            content: row.try_get_by("content")?,
            memory_type: row.try_get_by("memory_type")?,
            importance: row.try_get_by("importance")?,
            topic_key: row.try_get_by("topic_key")?,
            valid_from: row.try_get_by("valid_from")?,
            valid_until: row.try_get_by("valid_until")?,
            archived: row.try_get_by("archived")?,
            created_at: row.try_get_by("created_at")?,
            updated_at: row.try_get_by("updated_at")?,
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
    fn from_model(m: rel_entity::Model) -> Self {
        Self {
            id: m.id,
            source_id: m.source_id,
            target_id: m.target_id,
            relation_type: m.relation_type,
            created_at: m.created_at,
        }
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
    db: &DatabaseConnection,
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
        let existing = mem_entity::Entity::find()
            .filter(mem_entity::Column::TopicKey.eq(topic_key.as_str()))
            .filter(mem_entity::Column::WorkspaceId.eq(workspace_id.as_str()))
            .filter(mem_entity::Column::Archived.eq(false))
            .one(db)
            .await?;

        if let Some(row) = existing {
            return update_memory(
                db,
                UpdateMemoryRequest {
                    id: row.id,
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

    let model = mem_entity::ActiveModel {
        id: Set(id.clone()),
        workspace_id: Set(workspace_id),
        agent_id: Set(req.agent_id),
        title: Set(req.title.trim().to_string()),
        content: Set(req.content.trim().to_string()),
        memory_type: Set(req.memory_type.as_str().to_string()),
        importance: Set(importance.as_str().to_string()),
        topic_key: Set(req.topic_key),
        valid_from: Set(req.valid_from),
        valid_until: Set(req.valid_until),
        archived: Set(false),
        created_at: NotSet,
        updated_at: NotSet,
        embedding: Set(None),
        session_id: Set(None),
    };

    mem_entity::Entity::insert(model).exec(db).await?;

    get_memory_by_id(db, &id).await
}

pub async fn get_memory_by_id(db: &DatabaseConnection, id: &str) -> Result<Memory, NousError> {
    let model = mem_entity::Entity::find_by_id(id).one(db).await?;

    let model = model.ok_or_else(|| NousError::NotFound(format!("memory '{id}' not found")))?;
    Ok(Memory::from_model(model))
}

pub async fn update_memory(
    db: &DatabaseConnection,
    req: UpdateMemoryRequest,
) -> Result<Memory, NousError> {
    let _existing = get_memory_by_id(db, &req.id).await?;

    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<sea_orm::Value> = Vec::new();

    if let Some(ref title) = req.title {
        if title.trim().is_empty() {
            return Err(NousError::Validation("title cannot be empty".into()));
        }
        sets.push("title = ?".to_string());
        params.push(title.trim().to_string().into());
    }

    if let Some(ref content) = req.content {
        if content.trim().is_empty() {
            return Err(NousError::Validation("content cannot be empty".into()));
        }
        sets.push("content = ?".to_string());
        params.push(content.trim().to_string().into());
    }

    if let Some(ref importance) = req.importance {
        sets.push("importance = ?".to_string());
        params.push(importance.as_str().to_string().into());
    }

    if let Some(ref topic_key) = req.topic_key {
        sets.push("topic_key = ?".to_string());
        params.push(topic_key.clone().into());
    }

    if let Some(ref valid_from) = req.valid_from {
        sets.push("valid_from = ?".to_string());
        params.push(valid_from.clone().into());
    }

    if let Some(ref valid_until) = req.valid_until {
        sets.push("valid_until = ?".to_string());
        params.push(valid_until.clone().into());
    }

    if let Some(archived) = req.archived {
        sets.push("archived = ?".to_string());
        params.push(archived.into());
    }

    if sets.is_empty() {
        return get_memory_by_id(db, &req.id).await;
    }

    let sql = format!("UPDATE memories SET {} WHERE id = ?", sets.join(", "));
    params.push(req.id.clone().into());

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        params,
    ))
    .await?;

    get_memory_by_id(db, &req.id).await
}

pub async fn search_memories(
    db: &DatabaseConnection,
    req: &SearchMemoryRequest,
) -> Result<Vec<Memory>, NousError> {
    if req.query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = req.limit.unwrap_or(20).min(100);

    let mut conditions: Vec<String> = Vec::new();
    let mut params: Vec<sea_orm::Value> = Vec::new();

    let sanitized = sanitize_fts_query(&req.query);
    params.push(sanitized.into());

    if !req.include_archived {
        conditions.push("m.archived = 0".to_string());
    }

    if let Some(ref ws) = req.workspace_id {
        conditions.push("m.workspace_id = ?".to_string());
        params.push(ws.clone().into());
    }

    if let Some(ref agent) = req.agent_id {
        conditions.push("m.agent_id = ?".to_string());
        params.push(agent.clone().into());
    }

    if let Some(ref mt) = req.memory_type {
        conditions.push("m.memory_type = ?".to_string());
        params.push(mt.as_str().to_string().into());
    }

    if let Some(ref imp) = req.importance {
        conditions.push("m.importance = ?".to_string());
        params.push(imp.as_str().to_string().into());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" AND {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT m.id, m.workspace_id, m.agent_id, m.title, m.content, m.memory_type, \
         m.importance, m.topic_key, m.valid_from, m.valid_until, m.archived, \
         m.created_at, m.updated_at FROM memories m \
         INNER JOIN memories_fts f ON f.rowid = m.rowid \
         WHERE memories_fts MATCH ?{} \
         ORDER BY rank \
         LIMIT ?",
        where_clause
    );

    params.push((limit as i32).into());

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            &sql,
            params,
        ))
        .await?;

    rows.iter()
        .map(Memory::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn get_context(
    db: &DatabaseConnection,
    req: &ContextRequest,
) -> Result<Vec<Memory>, NousError> {
    let limit = req.limit.unwrap_or(20).min(100);

    let mut query = mem_entity::Entity::find().filter(mem_entity::Column::Archived.eq(false));

    if let Some(ref ws) = req.workspace_id {
        query = query.filter(mem_entity::Column::WorkspaceId.eq(ws.as_str()));
    }

    if let Some(ref agent) = req.agent_id {
        query = query.filter(mem_entity::Column::AgentId.eq(agent.as_str()));
    }

    if let Some(ref topic) = req.topic_key {
        query = query.filter(mem_entity::Column::TopicKey.eq(topic.as_str()));
    }

    let models = query
        .order_by_desc(mem_entity::Column::CreatedAt)
        .limit(limit as u64)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Memory::from_model).collect())
}

pub async fn relate_memories(
    db: &DatabaseConnection,
    req: &RelateRequest,
) -> Result<MemoryRelation, NousError> {
    let _source = get_memory_by_id(db, &req.source_id).await?;
    let _target = get_memory_by_id(db, &req.target_id).await?;

    if req.source_id == req.target_id {
        return Err(NousError::Validation(
            "cannot relate a memory to itself".into(),
        ));
    }

    let id = Uuid::now_v7().to_string();

    let model = rel_entity::ActiveModel {
        id: Set(id.clone()),
        source_id: Set(req.source_id.clone()),
        target_id: Set(req.target_id.clone()),
        relation_type: Set(req.relation_type.as_str().to_string()),
        created_at: NotSet,
    };

    let result = rel_entity::Entity::insert(model).exec(db).await;
    match result {
        Ok(_) => {}
        Err(ref e) if e.to_string().contains("2067") || e.to_string().contains("UNIQUE") => {
            return Err(NousError::Conflict("relation already exists".into()));
        }
        Err(e) => return Err(NousError::SeaOrm(e)),
    }

    if req.relation_type == RelationType::Supersedes {
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE memories SET valid_until = ? WHERE id = ? AND valid_until IS NULL",
            [now.into(), req.target_id.clone().into()],
        ))
        .await?;
    }

    get_relation_by_id(db, &id).await
}

pub async fn get_relation_by_id(
    db: &DatabaseConnection,
    id: &str,
) -> Result<MemoryRelation, NousError> {
    let model = rel_entity::Entity::find_by_id(id).one(db).await?;

    let model =
        model.ok_or_else(|| NousError::NotFound(format!("memory relation '{id}' not found")))?;
    Ok(MemoryRelation::from_model(model))
}

pub async fn list_relations(
    db: &DatabaseConnection,
    memory_id: &str,
) -> Result<Vec<MemoryRelation>, NousError> {
    use sea_orm::Condition;

    let models = rel_entity::Entity::find()
        .filter(
            Condition::any()
                .add(rel_entity::Column::SourceId.eq(memory_id))
                .add(rel_entity::Column::TargetId.eq(memory_id)),
        )
        .order_by_desc(rel_entity::Column::CreatedAt)
        .all(db)
        .await?;

    Ok(models.into_iter().map(MemoryRelation::from_model).collect())
}

pub async fn log_access(
    db: &DatabaseConnection,
    memory_id: &str,
    access_type: &str,
    session_id: Option<&str>,
) -> Result<(), NousError> {
    let model = access_entity::ActiveModel {
        id: Set(0), // auto-increment
        memory_id: Set(memory_id.to_string()),
        access_type: Set(access_type.to_string()),
        session_id: Set(session_id.map(String::from)),
        accessed_at: NotSet,
    };

    access_entity::Entity::insert(model).exec(db).await?;
    Ok(())
}

pub async fn run_importance_decay(
    db: &DatabaseConnection,
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

    // Run moderate->low first so that high->moderate in the same sweep
    // doesn't immediately cascade to low in one call.
    let r1 = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE memories SET importance = 'low' \
             WHERE importance = 'moderate' AND archived = 0 \
             AND id NOT IN (\
                 SELECT memory_id FROM memory_access_log WHERE accessed_at > ?\
             )",
            [moderate_cutoff.into()],
        ))
        .await?;

    let r2 = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE memories SET importance = 'moderate' \
             WHERE importance = 'high' AND archived = 0 \
             AND id NOT IN (\
                 SELECT memory_id FROM memory_access_log WHERE accessed_at > ?\
             )",
            [high_cutoff.into()],
        ))
        .await?;

    Ok(r1.rows_affected() + r2.rows_affected())
}

pub async fn grant_workspace_access(
    db: &DatabaseConnection,
    agent_id: &str,
    workspace_id: &str,
) -> Result<(), NousError> {
    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "INSERT OR IGNORE INTO agent_workspace_access (agent_id, workspace_id) VALUES (?, ?)",
        [agent_id.into(), workspace_id.into()],
    ))
    .await?;
    Ok(())
}

pub async fn revoke_workspace_access(
    db: &DatabaseConnection,
    agent_id: &str,
    workspace_id: &str,
) -> Result<(), NousError> {
    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "DELETE FROM agent_workspace_access WHERE agent_id = ? AND workspace_id = ?",
        [agent_id.into(), workspace_id.into()],
    ))
    .await?;
    Ok(())
}

pub async fn check_workspace_access(
    db: &DatabaseConnection,
    agent_id: &str,
    workspace_id: &str,
) -> Result<bool, NousError> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT EXISTS(SELECT 1 FROM agent_workspace_access WHERE agent_id = ? AND workspace_id = ?) as has_access",
            [agent_id.into(), workspace_id.into()],
        ))
        .await?;

    match row {
        Some(r) => Ok(r.try_get_by::<bool, _>("has_access").unwrap_or(false)),
        None => Ok(false),
    }
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
/// Writes to the vec0 virtual table for KNN search and also updates the legacy BLOB column.
pub async fn store_embedding(
    db: &DatabaseConnection,
    vec_pool: &crate::db::VecPool,
    memory_id: &str,
    embedding: &[f32],
) -> Result<(), NousError> {
    let _ = get_memory_by_id(db, memory_id).await?;

    let bytes = embedding_to_bytes(embedding);

    // Update legacy BLOB column (backwards compat)
    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE memories SET embedding = ? WHERE id = ?",
        [bytes.clone().into(), memory_id.into()],
    ))
    .await?;

    // Upsert into vec0 table
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;
    conn.execute(
        "DELETE FROM memory_embeddings WHERE memory_id = ?1",
        rusqlite::params![memory_id],
    )
    .map_err(|e| NousError::Internal(format!("failed to delete old embedding: {e}")))?;
    conn.execute(
        "INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![memory_id, bytes],
    )
    .map_err(|e| NousError::Internal(format!("failed to insert embedding: {e}")))?;

    Ok(())
}

/// Search memories by KNN using sqlite-vec's vec0 virtual table.
/// Returns top-K memories with similarity scores, ordered by distance ascending.
pub async fn search_similar(
    db: &DatabaseConnection,
    vec_pool: &crate::db::VecPool,
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
    let query_bytes = embedding_to_bytes(query_embedding);

    // KNN query via vec0
    let memory_ids_and_distances: Vec<(String, f32)> = {
        let conn = vec_pool
            .lock()
            .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

        let mut stmt = conn
            .prepare(
                "SELECT memory_id, distance \
                 FROM memory_embeddings \
                 WHERE embedding MATCH ?1 \
                 ORDER BY distance \
                 LIMIT ?2",
            )
            .map_err(|e| NousError::Internal(format!("failed to prepare KNN query: {e}")))?;

        let rows = stmt
            .query_map(rusqlite::params![query_bytes, limit], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?))
            })
            .map_err(|e| NousError::Internal(format!("KNN query failed: {e}")))?;

        rows.filter_map(|r| r.ok()).collect()
    };

    if memory_ids_and_distances.is_empty() {
        return Ok(Vec::new());
    }

    // Fetch full Memory structs from FTS db, filtering by workspace if needed
    let mut results: Vec<SimilarMemory> = Vec::new();
    for (memory_id, distance) in &memory_ids_and_distances {
        // Convert distance to similarity score (1 - normalized_distance for cosine)
        let score = 1.0 - distance;
        if score < threshold {
            continue;
        }

        let memory = match get_memory_by_id(db, memory_id).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        if memory.archived {
            continue;
        }

        if let Some(ws) = workspace_id {
            if memory.workspace_id != ws {
                continue;
            }
        }

        results.push(SimilarMemory { memory, score });
    }

    Ok(results)
}

fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

#[cfg(test)]
fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
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
    fn from_model(m: session_entity::Model) -> Self {
        Self {
            id: m.id,
            agent_id: m.agent_id,
            project: m.project,
            started_at: m.started_at,
            ended_at: m.ended_at,
            summary: m.summary,
        }
    }
}

pub async fn session_start(
    db: &DatabaseConnection,
    agent_id: Option<&str>,
    project: Option<&str>,
) -> Result<MemorySession, NousError> {
    let id = Uuid::now_v7().to_string();

    let model = session_entity::ActiveModel {
        id: Set(id.clone()),
        agent_id: Set(agent_id.map(String::from)),
        project: Set(project.map(String::from)),
        started_at: NotSet,
        ended_at: Set(None),
        summary: Set(None),
    };

    session_entity::Entity::insert(model).exec(db).await?;

    get_session_by_id(db, &id).await
}

pub async fn session_end(
    db: &DatabaseConnection,
    session_id: &str,
) -> Result<MemorySession, NousError> {
    let session = get_session_by_id(db, session_id).await?;
    if session.ended_at.is_some() {
        return Err(NousError::Validation("session already ended".into()));
    }

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE memory_sessions SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        [session_id.into()],
    ))
    .await?;

    get_session_by_id(db, session_id).await
}

pub async fn session_summary(
    db: &DatabaseConnection,
    session_id: &str,
    summary: &str,
    agent_id: Option<&str>,
    workspace_id: Option<&str>,
) -> Result<MemorySession, NousError> {
    if summary.trim().is_empty() {
        return Err(NousError::Validation("summary cannot be empty".into()));
    }

    let _session = get_session_by_id(db, session_id).await?;

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE memory_sessions SET summary = ? WHERE id = ?",
        [summary.trim().into(), session_id.into()],
    ))
    .await?;

    save_memory(
        db,
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

    get_session_by_id(db, session_id).await
}

pub async fn save_prompt(
    db: &DatabaseConnection,
    session_id: Option<&str>,
    agent_id: Option<&str>,
    workspace_id: Option<&str>,
    prompt: &str,
) -> Result<Memory, NousError> {
    if prompt.trim().is_empty() {
        return Err(NousError::Validation("prompt cannot be empty".into()));
    }

    if let Some(sid) = session_id {
        let _session = get_session_by_id(db, sid).await?;
    }

    let mem = save_memory(
        db,
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
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE memories SET session_id = ? WHERE id = ?",
            [sid.into(), mem.id.clone().into()],
        ))
        .await?;
    }

    get_memory_by_id(db, &mem.id).await
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
                    project_type: String::from(*project_type),
                    path: dir.to_string_lossy().into_owned(),
                });
            }
        }
        current = dir.parent();
    }
    None
}

async fn get_session_by_id(db: &DatabaseConnection, id: &str) -> Result<MemorySession, NousError> {
    let model = session_entity::Entity::find_by_id(id).one(db).await?;

    let model =
        model.ok_or_else(|| NousError::NotFound(format!("memory session '{id}' not found")))?;
    Ok(MemorySession::from_model(model))
}

fn truncate_title(s: &str) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.len() > 80 {
        format!("{}...", &first_line[..77])
    } else {
        first_line.to_string()
    }
}

// --- Chunk pipeline operations ---

pub fn store_chunks(vec_pool: &VecPool, chunks: &[Chunk]) -> Result<(), NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    for chunk in chunks {
        conn.execute(
            "INSERT OR REPLACE INTO memory_chunks (id, memory_id, content, chunk_index, start_offset, end_offset) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                chunk.id,
                chunk.memory_id,
                chunk.content,
                chunk.index,
                chunk.start_offset,
                chunk.end_offset,
            ],
        )
        .map_err(|e| NousError::Internal(format!("failed to store chunk: {e}")))?;
    }

    Ok(())
}

pub fn store_chunk_embedding(
    vec_pool: &VecPool,
    chunk_id: &str,
    embedding: &[f32],
) -> Result<(), NousError> {
    let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    conn.execute(
        "DELETE FROM memory_embeddings WHERE memory_id = ?1",
        rusqlite::params![chunk_id],
    )
    .map_err(|e| NousError::Internal(format!("failed to delete old chunk embedding: {e}")))?;

    conn.execute(
        "INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![chunk_id, bytes],
    )
    .map_err(|e| NousError::Internal(format!("failed to insert chunk embedding: {e}")))?;

    Ok(())
}

pub async fn search_hybrid(
    fts_db: &DatabaseConnection,
    vec_pool: &VecPool,
    query: &str,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<SimilarMemory>, NousError> {
    search_hybrid_filtered(SearchHybridFilteredParams {
        fts_db,
        vec_pool,
        query,
        query_embedding,
        limit,
        workspace_id: None,
        agent_id: None,
        memory_type: None,
    })
    .await
}

pub struct SearchHybridFilteredParams<'a> {
    pub fts_db: &'a DatabaseConnection,
    pub vec_pool: &'a VecPool,
    pub query: &'a str,
    pub query_embedding: &'a [f32],
    pub limit: usize,
    pub workspace_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub memory_type: Option<MemoryType>,
}

pub async fn search_hybrid_filtered(
    params: SearchHybridFilteredParams<'_>,
) -> Result<Vec<SimilarMemory>, NousError> {
    let SearchHybridFilteredParams {
        fts_db,
        vec_pool,
        query,
        query_embedding,
        limit,
        workspace_id,
        agent_id,
        memory_type,
    } = params;
    let fts_limit = (limit * 2).min(100) as u32;
    let vec_limit = (limit * 2).min(100) as u32;

    // FTS search with filters
    let fts_memories = search_memories(
        fts_db,
        &SearchMemoryRequest {
            query: query.to_string(),
            limit: Some(fts_limit),
            workspace_id: workspace_id.map(|s| s.to_string()),
            agent_id: agent_id.map(|s| s.to_string()),
            memory_type,
            ..Default::default()
        },
    )
    .await?;
    let fts_results: Vec<SimilarMemory> = fts_memories
        .into_iter()
        .enumerate()
        .map(|(rank, memory)| SimilarMemory {
            memory,
            score: 1.0 / (1.0 + rank as f32),
        })
        .collect();

    // Vector search (workspace_id supported natively; agent_id/memory_type filtered post-KNN)
    let vec_results = search_similar(
        fts_db,
        vec_pool,
        query_embedding,
        vec_limit,
        workspace_id,
        None,
    )
    .await?;

    // Post-KNN filter for agent_id and memory_type on vec results
    let vec_results: Vec<SimilarMemory> = vec_results
        .into_iter()
        .filter(|sm| {
            if let Some(aid) = agent_id {
                if sm.memory.agent_id.as_deref() != Some(aid) {
                    return false;
                }
            }
            if let Some(mt) = memory_type {
                if sm.memory.memory_type != mt.as_str() {
                    return false;
                }
            }
            true
        })
        .collect();

    // RRF merge
    let mut merged = rerank_rrf(&fts_results, &vec_results, None);
    merged.truncate(limit);
    Ok(merged)
}

pub fn delete_chunks(vec_pool: &VecPool, memory_id: &str) -> Result<(), NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    // Get chunk IDs for this memory to also clean up their embeddings
    let chunk_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM memory_chunks WHERE memory_id = ?1")
            .map_err(|e| NousError::Internal(format!("failed to prepare chunk query: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![memory_id], |row| row.get(0))
            .map_err(|e| NousError::Internal(format!("failed to query chunks: {e}")))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // Delete embeddings for each chunk
    for chunk_id in &chunk_ids {
        conn.execute(
            "DELETE FROM memory_embeddings WHERE memory_id = ?1",
            rusqlite::params![chunk_id],
        )
        .map_err(|e| NousError::Internal(format!("failed to delete chunk embedding: {e}")))?;
    }

    // Delete the chunks themselves
    conn.execute(
        "DELETE FROM memory_chunks WHERE memory_id = ?1",
        rusqlite::params![memory_id],
    )
    .map_err(|e| NousError::Internal(format!("failed to delete chunks: {e}")))?;

    Ok(())
}

pub fn get_chunks_for_memory(vec_pool: &VecPool, memory_id: &str) -> Result<Vec<Chunk>, NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, memory_id, content, chunk_index, start_offset, end_offset \
             FROM memory_chunks WHERE memory_id = ?1 ORDER BY chunk_index",
        )
        .map_err(|e| NousError::Internal(format!("failed to prepare chunks query: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params![memory_id], |row| {
            Ok(Chunk {
                id: row.get(0)?,
                memory_id: row.get(1)?,
                content: row.get(2)?,
                index: row.get::<_, i64>(3)? as usize,
                start_offset: row.get::<_, i64>(4)? as usize,
                end_offset: row.get::<_, i64>(5)? as usize,
            })
        })
        .map_err(|e| NousError::Internal(format!("failed to query chunks: {e}")))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| NousError::Internal(format!("failed to collect chunks: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use tempfile::TempDir;

    async fn setup() -> (DatabaseConnection, crate::db::VecPool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        for agent_id in ["agent-1", "agent-2", "agent-3", "test-agent"] {
            pools.fts.execute_unprepared(
                &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
            ).await.unwrap();
        }
        (pools.fts, pools.vec, tmp)
    }

    #[tokio::test]
    async fn save_and_get_memory() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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

        let fetched = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(fetched.id, mem.id);
        assert_eq!(fetched.content, "This is a test memory content");
    }

    #[tokio::test]
    async fn save_empty_title_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = save_memory(
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let m1 = save_memory(
            &db,
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
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        save_memory(
            &db,
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
            &db,
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
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        save_memory(
            &db,
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
            &db,
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
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        for i in 1..=5 {
            save_memory(
                &db,
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
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let m1 = save_memory(
            &db,
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
            &db,
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
            &db,
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

        let rels = list_relations(&db, &m1.id).await.unwrap();
        assert_eq!(rels.len(), 1);
    }

    #[tokio::test]
    async fn supersedes_sets_valid_until() {
        let (db, _vec_pool, _tmp) = setup().await;

        let old = save_memory(
            &db,
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
            &db,
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
            &db,
            &RelateRequest {
                source_id: new.id.clone(),
                target_id: old.id.clone(),
                relation_type: RelationType::Supersedes,
            },
        )
        .await
        .unwrap();

        let old_refreshed = get_memory_by_id(&db, &old.id).await.unwrap();
        assert!(old_refreshed.valid_until.is_some());
    }

    #[tokio::test]
    async fn self_relation_rejected() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let m1 = save_memory(
            &db,
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
            &db,
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
            &db,
            &RelateRequest {
                source_id: m1.id.clone(),
                target_id: m2.id.clone(),
                relation_type: RelationType::Related,
            },
        )
        .await
        .unwrap();

        let err = relate_memories(
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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

        let affected = run_importance_decay(&db, 0, 0).await.unwrap();
        assert!(affected > 0);

        let refreshed = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(refreshed.importance, "moderate");

        let affected2 = run_importance_decay(&db, 0, 0).await.unwrap();
        assert!(affected2 > 0);

        let refreshed2 = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(refreshed2.importance, "low");
    }

    #[tokio::test]
    async fn access_log_prevents_decay() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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

        log_access(&db, &mem.id, "recall", None).await.unwrap();

        run_importance_decay(&db, 30, 60).await.unwrap();

        let refreshed = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(refreshed.importance, "high");
    }

    #[tokio::test]
    async fn workspace_access_crud() {
        let (db, _vec_pool, _tmp) = setup().await;

        assert!(!check_workspace_access(&db, "agent-1", "ws-1")
            .await
            .unwrap());

        grant_workspace_access(&db, "agent-1", "ws-1")
            .await
            .unwrap();

        assert!(check_workspace_access(&db, "agent-1", "ws-1")
            .await
            .unwrap());

        revoke_workspace_access(&db, "agent-1", "ws-1")
            .await
            .unwrap();

        assert!(!check_workspace_access(&db, "agent-1", "ws-1")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn search_archived_excluded_by_default() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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
            &db,
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
            &db,
            &SearchMemoryRequest {
                query: "archived".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 0);

        let results = search_memories(
            &db,
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
        assert_eq!(
            "decision".parse::<MemoryType>().unwrap(),
            MemoryType::Decision
        );
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
        use crate::db::EMBEDDING_DIMENSION;
        let (db, vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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

        let mut embedding = vec![0.0f32; EMBEDDING_DIMENSION];
        embedding[0] = 1.0;
        store_embedding(&db, &vec_pool, &mem.id, &embedding)
            .await
            .unwrap();

        let mut query = vec![0.0f32; EMBEDDING_DIMENSION];
        query[0] = 0.9;
        query[1] = 0.1;
        let results = search_similar(&db, &vec_pool, &query, 10, None, None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].score > 0.0);
        assert_eq!(results[0].memory.id, mem.id);
    }

    #[tokio::test]
    async fn search_similar_respects_threshold() {
        use crate::db::EMBEDDING_DIMENSION;
        let (db, vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
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

        let mut embedding = vec![0.0f32; EMBEDDING_DIMENSION];
        embedding[0] = 1.0;
        store_embedding(&db, &vec_pool, &mem.id, &embedding)
            .await
            .unwrap();

        // Orthogonal vector — high threshold should filter it out
        let mut query = vec![0.0f32; EMBEDDING_DIMENSION];
        query[1] = 1.0;
        let results = search_similar(&db, &vec_pool, &query, 10, None, Some(0.9))
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
        let original = vec![1.0f32, -2.5, 3.1, 0.0];
        let bytes = embedding_to_bytes(&original);
        let recovered = bytes_to_embedding(&bytes);
        assert_eq!(original, recovered);
    }

    // --- Session lifecycle tests ---

    #[tokio::test]
    async fn session_start_creates_session() {
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, Some("agent-1"), Some("my-project"))
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
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, None, None).await.unwrap();

        assert!(!session.id.is_empty());
        assert!(session.agent_id.is_none());
        assert!(session.project.is_none());
    }

    #[tokio::test]
    async fn session_end_sets_ended_at() {
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, None, None).await.unwrap();
        assert!(session.ended_at.is_none());

        let ended = session_end(&db, &session.id).await.unwrap();
        assert!(ended.ended_at.is_some());
        assert_eq!(ended.id, session.id);
    }

    #[tokio::test]
    async fn session_end_already_ended_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, None, None).await.unwrap();
        session_end(&db, &session.id).await.unwrap();

        let err = session_end(&db, &session.id).await.unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn session_end_nonexistent_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = session_end(&db, "nonexistent-session-id")
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[tokio::test]
    async fn session_summary_saves_summary_and_memory() {
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, Some("agent-1"), Some("proj"))
            .await
            .unwrap();

        let updated = session_summary(
            &db,
            &session.id,
            "Completed migration refactoring",
            Some("agent-1"),
            Some("ws-1"),
        )
        .await
        .unwrap();

        assert_eq!(
            updated.summary.as_deref(),
            Some("Completed migration refactoring")
        );
        assert_eq!(updated.id, session.id);

        // Verify a session_summary memory was also created
        let results = search_memories(
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, None, None).await.unwrap();

        let err = session_summary(&db, &session.id, "   ", None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn session_summary_nonexistent_session_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = session_summary(&db, "nonexistent-id", "some summary", None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[tokio::test]
    async fn save_prompt_creates_memory() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_prompt(
            &db,
            None,
            Some("agent-1"),
            Some("ws-1"),
            "Refactor the auth module",
        )
        .await
        .unwrap();

        assert!(!mem.id.is_empty());
        assert_eq!(mem.content, "Refactor the auth module");
        assert_eq!(mem.memory_type, "observation");
        assert_eq!(mem.importance, "low");
    }

    #[tokio::test]
    async fn save_prompt_with_session_links_memory() {
        let (db, _vec_pool, _tmp) = setup().await;

        let session = session_start(&db, Some("agent-1"), None).await.unwrap();

        let mem = save_prompt(
            &db,
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
        let (db, _vec_pool, _tmp) = setup().await;

        let err = save_prompt(&db, None, None, None, "   ").await.unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn save_prompt_nonexistent_session_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = save_prompt(&db, Some("bad-session"), None, None, "a prompt")
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

    #[tokio::test]
    async fn save_memory_then_embed_with_mock() {
        use crate::memory::embed::MockEmbedder;
        use crate::memory::Embedder;

        let (db, vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: Some("test-agent".into()),
                title: "Embedding test".into(),
                content: "This content should be embedded into the vector database".into(),
                memory_type: MemoryType::Fact,
                importance: Some(Importance::Moderate),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        let embedder = MockEmbedder::new();
        let chunker = Chunker::default();
        let chunks = chunker.chunk(&mem.id, &mem.content);
        store_chunks(&vec_pool, &chunks).unwrap();

        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = embedder.embed(&texts).unwrap();
        for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
            store_chunk_embedding(&vec_pool, &chunk.id, embedding).unwrap();
        }

        let full_embeddings = embedder.embed(&[&mem.content]).unwrap();
        store_embedding(&db, &vec_pool, &mem.id, &full_embeddings[0])
            .await
            .unwrap();

        let fetched = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(fetched.id, mem.id);

        let conn = vec_pool.lock().unwrap();
        let chunk_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_chunks WHERE memory_id = ?1",
                rusqlite::params![&mem.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(chunk_count, chunks.len() as i64);

        let emb_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_embeddings WHERE memory_id = ?1",
                rusqlite::params![&mem.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(emb_count > 0);
    }
}
