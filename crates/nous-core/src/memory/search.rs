use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::db::VecPool;
use crate::error::NousError;
use crate::fts::sanitize_fts5_query;

use super::rerank::rerank_rrf;
use super::store::get_memory_by_id;
use super::types::{Memory, MemoryType, SearchMemoryRequest, SimilarMemory};

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

    let sanitized = sanitize_fts5_query(&req.query);
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
         WHERE memories_fts MATCH ?{where_clause} \
         ORDER BY rank \
         LIMIT ?"
    );

    // limit is capped by caller (e.g. 200); safe to cast to i32
    params.push(limit.cast_signed().into());

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

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE memories SET embedding = ? WHERE id = ?",
        [bytes.clone().into(), memory_id.into()],
    ))
    .await?;

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

pub struct SearchSimilarParams<'a> {
    pub db: &'a DatabaseConnection,
    pub vec_pool: &'a crate::db::VecPool,
    pub query_embedding: &'a [f32],
    pub limit: u32,
    pub workspace_id: Option<&'a str>,
    pub threshold: Option<f32>,
}

/// Search memories by KNN using sqlite-vec's vec0 virtual table.
/// Returns top-K memories with similarity scores, ordered by distance ascending.
pub async fn search_similar(
    params: SearchSimilarParams<'_>,
) -> Result<Vec<SimilarMemory>, NousError> {
    let SearchSimilarParams {
        db,
        vec_pool,
        query_embedding,
        limit,
        workspace_id,
        threshold,
    } = params;
    if query_embedding.is_empty() {
        return Err(NousError::Validation("embedding cannot be empty".into()));
    }

    let limit = limit.min(100);
    let threshold = threshold.unwrap_or(0.0);
    let query_bytes = embedding_to_bytes(query_embedding);

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

        rows.filter_map(std::result::Result::ok).collect()
    };

    if memory_ids_and_distances.is_empty() {
        return Ok(Vec::new());
    }

    let mut results: Vec<SimilarMemory> = Vec::new();
    for (memory_id, distance) in &memory_ids_and_distances {
        let score = 1.0 - distance;
        if score < threshold {
            continue;
        }

        let Ok(memory) = get_memory_by_id(db, memory_id).await else {
            continue;
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

pub struct SearchHybridRequest<'a> {
    pub fts_db: &'a DatabaseConnection,
    pub vec_pool: &'a VecPool,
    pub query: &'a str,
    pub query_embedding: &'a [f32],
    pub limit: usize,
}

pub async fn search_hybrid(req: SearchHybridRequest<'_>) -> Result<Vec<SimilarMemory>, NousError> {
    search_hybrid_filtered(SearchHybridFilteredParams {
        fts_db: req.fts_db,
        vec_pool: req.vec_pool,
        query: req.query,
        query_embedding: req.query_embedding,
        limit: req.limit,
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

    let fts_memories = search_memories(
        fts_db,
        &SearchMemoryRequest {
            query: query.to_string(),
            limit: Some(fts_limit),
            workspace_id: workspace_id.map(std::string::ToString::to_string),
            agent_id: agent_id.map(std::string::ToString::to_string),
            memory_type,
            ..Default::default()
        },
    )
    .await?;
    let fts_results: Vec<SimilarMemory> = fts_memories
        .into_iter()
        .enumerate()
        .map(|(rank, memory)| {
            // rank fits in u16 (result lists are small); u16→f32 is lossless
            let rank_f = f32::from(u16::try_from(rank).unwrap_or(u16::MAX));
            SimilarMemory {
                memory,
                score: 1.0 / (1.0 + rank_f),
            }
        })
        .collect();

    let vec_results = search_similar(SearchSimilarParams {
        db: fts_db,
        vec_pool,
        query_embedding,
        limit: vec_limit,
        workspace_id,
        threshold: None,
    })
    .await?;

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

    let mut merged = rerank_rrf(&fts_results, &vec_results, None);
    merged.truncate(limit);
    Ok(merged)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::memory::store::{save_memory, update_memory};
    use crate::memory::types::*;
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

    #[test]
    fn sanitize_fts_handles_special_chars() {
        assert_eq!(sanitize_fts5_query("INI-076"), "\"INI-076\"");
        assert_eq!(sanitize_fts5_query("simple query"), "simple query");
        assert_eq!(
            sanitize_fts5_query("fix auth@service"),
            "fix \"auth@service\""
        );
    }

    #[tokio::test]
    async fn store_and_search_embedding() {
        const DIM: usize = 1024;
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

        let mut embedding = vec![0.0f32; DIM];
        embedding[0] = 1.0;
        store_embedding(&db, &vec_pool, &mem.id, &embedding)
            .await
            .unwrap();

        let mut query = vec![0.0f32; DIM];
        query[0] = 0.9;
        query[1] = 0.1;
        let results = search_similar(SearchSimilarParams {
            db: &db,
            vec_pool: &vec_pool,
            query_embedding: &query,
            limit: 10,
            workspace_id: None,
            threshold: None,
        })
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].score > 0.0);
        assert_eq!(results[0].memory.id, mem.id);
    }

    #[tokio::test]
    async fn search_similar_respects_threshold() {
        const DIM: usize = 1024;
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

        let mut embedding = vec![0.0f32; DIM];
        embedding[0] = 1.0;
        store_embedding(&db, &vec_pool, &mem.id, &embedding)
            .await
            .unwrap();

        let mut query = vec![0.0f32; DIM];
        query[1] = 1.0;
        let results = search_similar(SearchSimilarParams {
            db: &db,
            vec_pool: &vec_pool,
            query_embedding: &query,
            limit: 10,
            workspace_id: None,
            threshold: Some(0.9),
        })
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
}
