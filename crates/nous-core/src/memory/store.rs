use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, NotSet, QueryOrder, QuerySelect, Set, Statement};
use uuid::Uuid;

use crate::db::VecPool;
use crate::entities::{
    memories as mem_entity, memory_access_log as access_entity, memory_relations as rel_entity,
    memory_sessions as session_entity,
};
use crate::error::NousError;

use super::chunk::Chunk;
use super::types::{
    ContextRequest, Importance, Memory, MemoryRelation, MemorySession, MemoryType, RelationType,
    RelateRequest, SaveMemoryRequest, UpdateMemoryRequest,
};

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

fn build_memory_update_fields(
    req: &UpdateMemoryRequest,
    sets: &mut Vec<String>,
    params: &mut Vec<sea_orm::Value>,
) -> Result<(), NousError> {
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

    Ok(())
}

pub async fn update_memory(
    db: &DatabaseConnection,
    req: UpdateMemoryRequest,
) -> Result<Memory, NousError> {
    let _existing = get_memory_by_id(db, &req.id).await?;

    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<sea_orm::Value> = Vec::new();

    build_memory_update_fields(&req, &mut sets, &mut params)?;

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
        .limit(u64::from(limit))
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

pub struct SessionSummaryRequest<'a> {
    pub session_id: &'a str,
    pub summary: &'a str,
    pub agent_id: Option<&'a str>,
    pub workspace_id: Option<&'a str>,
}

pub async fn session_summary(
    db: &DatabaseConnection,
    req: SessionSummaryRequest<'_>,
) -> Result<MemorySession, NousError> {
    let SessionSummaryRequest {
        session_id,
        summary,
        agent_id,
        workspace_id,
    } = req;
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

pub struct SavePromptRequest<'a> {
    pub session_id: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    pub workspace_id: Option<&'a str>,
    pub prompt: &'a str,
}

pub async fn save_prompt(
    db: &DatabaseConnection,
    req: SavePromptRequest<'_>,
) -> Result<Memory, NousError> {
    let SavePromptRequest {
        session_id,
        agent_id,
        workspace_id,
        prompt,
    } = req;
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
            title: super::types::truncate_title(prompt),
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

pub(crate) async fn get_session_by_id(db: &DatabaseConnection, id: &str) -> Result<MemorySession, NousError> {
    let model = session_entity::Entity::find_by_id(id).one(db).await?;

    let model =
        model.ok_or_else(|| NousError::NotFound(format!("memory session '{id}' not found")))?;
    Ok(MemorySession::from_model(model))
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

pub fn delete_chunks(vec_pool: &VecPool, memory_id: &str) -> Result<(), NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    let chunk_ids: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT id FROM memory_chunks WHERE memory_id = ?1")
            .map_err(|e| NousError::Internal(format!("failed to prepare chunk query: {e}")))?;
        let rows = stmt
            .query_map(rusqlite::params![memory_id], |row| row.get(0))
            .map_err(|e| NousError::Internal(format!("failed to query chunks: {e}")))?;
        rows.filter_map(std::result::Result::ok).collect()
    };

    for chunk_id in &chunk_ids {
        conn.execute(
            "DELETE FROM memory_embeddings WHERE memory_id = ?1",
            rusqlite::params![chunk_id],
        )
        .map_err(|e| NousError::Internal(format!("failed to delete chunk embedding: {e}")))?;
    }

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
            // SQLite stores these as i64 but values are always non-negative byte offsets/indices
            let index = usize::try_from(row.get::<_, i64>(3)?).unwrap_or(0);
            let start_offset = usize::try_from(row.get::<_, i64>(4)?).unwrap_or(0);
            let end_offset = usize::try_from(row.get::<_, i64>(5)?).unwrap_or(0);
            Ok(Chunk {
                id: row.get(0)?,
                memory_id: row.get(1)?,
                content: row.get(2)?,
                index,
                start_offset,
                end_offset,
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

        let results = super::super::search::search_memories(
            &db,
            &super::super::types::SearchMemoryRequest {
                query: "archived".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 0);

        let results = super::super::search::search_memories(
            &db,
            &super::super::types::SearchMemoryRequest {
                query: "archived".into(),
                include_archived: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
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
            SessionSummaryRequest {
                session_id: &session.id,
                summary: "Completed migration refactoring",
                agent_id: Some("agent-1"),
                workspace_id: Some("ws-1"),
            },
        )
        .await
        .unwrap();

        assert_eq!(
            updated.summary.as_deref(),
            Some("Completed migration refactoring")
        );
        assert_eq!(updated.id, session.id);

        let results = super::super::search::search_memories(
            &db,
            &super::super::types::SearchMemoryRequest {
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

        let err = session_summary(
            &db,
            SessionSummaryRequest {
                session_id: &session.id,
                summary: "   ",
                agent_id: None,
                workspace_id: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn session_summary_nonexistent_session_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = session_summary(
            &db,
            SessionSummaryRequest {
                session_id: "nonexistent-id",
                summary: "some summary",
                agent_id: None,
                workspace_id: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
    }

    #[tokio::test]
    async fn save_prompt_creates_memory() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_prompt(
            &db,
            SavePromptRequest {
                session_id: None,
                agent_id: Some("agent-1"),
                workspace_id: Some("ws-1"),
                prompt: "Refactor the auth module",
            },
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
            SavePromptRequest {
                session_id: Some(&session.id),
                agent_id: Some("agent-1"),
                workspace_id: None,
                prompt: "Fix the login bug",
            },
        )
        .await
        .unwrap();

        assert!(!mem.id.is_empty());
        assert_eq!(mem.content, "Fix the login bug");
    }

    #[tokio::test]
    async fn save_prompt_empty_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = save_prompt(
            &db,
            SavePromptRequest {
                session_id: None,
                agent_id: None,
                workspace_id: None,
                prompt: "   ",
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NousError::Validation(_)));
    }

    #[tokio::test]
    async fn save_prompt_nonexistent_session_fails() {
        let (db, _vec_pool, _tmp) = setup().await;

        let err = save_prompt(
            &db,
            SavePromptRequest {
                session_id: Some("bad-session"),
                agent_id: None,
                workspace_id: None,
                prompt: "a prompt",
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, NousError::NotFound(_)));
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
        let chunker = super::super::chunk::Chunker::default();
        let chunks = chunker.chunk(&mem.id, &mem.content);
        store_chunks(&vec_pool, &chunks).unwrap();

        let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        let embeddings = embedder.embed(&texts).unwrap();
        for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
            super::super::search::store_chunk_embedding(&vec_pool, &chunk.id, embedding).unwrap();
        }

        let full_embeddings = embedder.embed(&[&mem.content]).unwrap();
        super::super::search::store_embedding(&db, &vec_pool, &mem.id, &full_embeddings[0])
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
