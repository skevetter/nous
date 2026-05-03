use std::sync::Arc;

use nous_core::agents::definition::{MemoryScope, RetrievalStrategy};
use nous_core::db::VecPool;
use nous_core::error::NousError;
use nous_core::memory::{
    self, ContextRequest, Embedder, MemoryType, SearchMemoryRequest, SimilarMemory,
};
use rig::vector_store::request::Filter;
use rig::vector_store::{VectorSearchRequest, VectorStoreError, VectorStoreIndex};
use serde::Deserialize;
use sqlx::SqlitePool;

pub struct NousMemoryIndex {
    fts_pool: SqlitePool,
    vec_pool: VecPool,
    embedder: Arc<dyn Embedder>,
    workspace_id: String,
    agent_id: Option<String>,
    #[allow(dead_code)]
    session_id: Option<String>,
    scope: MemoryScope,
    retrieval: RetrievalStrategy,
    memory_types: Vec<MemoryType>,
}

impl NousMemoryIndex {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fts_pool: SqlitePool,
        vec_pool: VecPool,
        embedder: Arc<dyn Embedder>,
        workspace_id: String,
        agent_id: Option<String>,
        session_id: Option<String>,
        scope: MemoryScope,
        retrieval: RetrievalStrategy,
        memory_types: Vec<MemoryType>,
    ) -> Self {
        Self {
            fts_pool,
            vec_pool,
            embedder,
            workspace_id,
            agent_id,
            session_id,
            scope,
            retrieval,
            memory_types,
        }
    }

    fn scope_workspace_id(&self) -> Option<&str> {
        match &self.scope {
            MemoryScope::Workspace | MemoryScope::Shared(_) => Some(&self.workspace_id),
            MemoryScope::Agent | MemoryScope::Session => Some(&self.workspace_id),
        }
    }

    fn scope_agent_id(&self) -> Option<&str> {
        match &self.scope {
            MemoryScope::Workspace | MemoryScope::Shared(_) => None,
            MemoryScope::Agent => self.agent_id.as_deref(),
            MemoryScope::Session => self.agent_id.as_deref(),
        }
    }

    fn first_memory_type(&self) -> Option<MemoryType> {
        self.memory_types.first().copied()
    }

    async fn search_by_strategy(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SimilarMemory>, NousError> {
        let ws = self.scope_workspace_id();
        let agent = self.scope_agent_id();
        let mt = self.first_memory_type();

        match &self.retrieval {
            RetrievalStrategy::Hybrid => {
                let embeddings = self
                    .embedder
                    .embed(&[query])
                    .map_err(|e| NousError::Internal(format!("embedding failed: {e}")))?;
                let embedding = embeddings.into_iter().next().ok_or_else(|| {
                    NousError::Internal("embedder returned no vectors".to_string())
                })?;

                memory::search_hybrid_filtered(
                    &self.fts_pool,
                    &self.vec_pool,
                    query,
                    &embedding,
                    limit,
                    ws,
                    agent,
                    mt,
                )
                .await
            }
            RetrievalStrategy::Fts => {
                let memories = memory::search_memories(
                    &self.fts_pool,
                    &SearchMemoryRequest {
                        query: query.to_string(),
                        workspace_id: ws.map(String::from),
                        agent_id: agent.map(String::from),
                        memory_type: mt,
                        limit: Some(limit as u32),
                        ..Default::default()
                    },
                )
                .await?;

                Ok(memories
                    .into_iter()
                    .enumerate()
                    .map(|(rank, m)| SimilarMemory {
                        memory: m,
                        score: 1.0 / (1.0 + rank as f32),
                    })
                    .collect())
            }
            RetrievalStrategy::Vector => {
                let embeddings = self
                    .embedder
                    .embed(&[query])
                    .map_err(|e| NousError::Internal(format!("embedding failed: {e}")))?;
                let embedding = embeddings.into_iter().next().ok_or_else(|| {
                    NousError::Internal("embedder returned no vectors".to_string())
                })?;

                memory::search_similar(
                    &self.fts_pool,
                    &self.vec_pool,
                    &embedding,
                    limit as u32,
                    ws,
                    None,
                )
                .await
            }
            RetrievalStrategy::Recency => {
                let memories = memory::get_context(
                    &self.fts_pool,
                    &ContextRequest {
                        workspace_id: ws.map(String::from),
                        agent_id: agent.map(String::from),
                        limit: Some(limit as u32),
                        ..Default::default()
                    },
                )
                .await?;

                Ok(memories
                    .into_iter()
                    .map(|m| SimilarMemory {
                        memory: m,
                        score: 1.0,
                    })
                    .collect())
            }
        }
    }
}

fn nous_err_to_store(e: NousError) -> VectorStoreError {
    VectorStoreError::DatastoreError(Box::new(e))
}

impl VectorStoreIndex for NousMemoryIndex {
    type Filter = Filter<serde_json::Value>;

    async fn top_n<T: for<'a> Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String, T)>, VectorStoreError> {
        let query = req.query();
        let limit = req.samples() as usize;

        let results = self
            .search_by_strategy(query, limit)
            .await
            .map_err(nous_err_to_store)?;

        let mut out = Vec::with_capacity(results.len());
        for sm in results {
            let doc = serde_json::json!({
                "title": sm.memory.title,
                "content": sm.memory.content,
                "memory_type": sm.memory.memory_type,
                "importance": sm.memory.importance,
            });
            let item: T = serde_json::from_value(doc)?;
            out.push((sm.score as f64, sm.memory.id, item));
        }
        Ok(out)
    }

    async fn top_n_ids(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String)>, VectorStoreError> {
        let query = req.query();
        let limit = req.samples() as usize;

        let results = self
            .search_by_strategy(query, limit)
            .await
            .map_err(nous_err_to_store)?;

        Ok(results
            .into_iter()
            .map(|sm| (sm.score as f64, sm.memory.id))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_core::memory::MockEmbedder;

    fn make_index(
        fts_pool: SqlitePool,
        vec_pool: VecPool,
        scope: MemoryScope,
        retrieval: RetrievalStrategy,
    ) -> NousMemoryIndex {
        NousMemoryIndex::new(
            fts_pool,
            vec_pool,
            Arc::new(MockEmbedder::new()),
            "test-workspace".to_string(),
            Some("test-agent".to_string()),
            Some("test-session".to_string()),
            scope,
            retrieval,
            vec![MemoryType::Decision, MemoryType::Convention],
        )
    }

    #[test]
    fn test_nous_memory_index_new() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = tempfile::TempDir::new().unwrap();
            let pools = nous_core::db::DbPools::connect(tmp.path()).await.unwrap();

            let index = NousMemoryIndex::new(
                pools.fts.clone(),
                pools.vec.clone(),
                Arc::new(MockEmbedder::new()),
                "ws-1".to_string(),
                Some("agent-1".to_string()),
                Some("session-1".to_string()),
                MemoryScope::Agent,
                RetrievalStrategy::Hybrid,
                vec![MemoryType::Decision],
            );

            assert_eq!(index.workspace_id, "ws-1");
            assert_eq!(index.agent_id.as_deref(), Some("agent-1"));
            assert_eq!(index.session_id.as_deref(), Some("session-1"));
            assert_eq!(index.scope, MemoryScope::Agent);
            assert_eq!(index.retrieval, RetrievalStrategy::Hybrid);
            assert_eq!(index.memory_types, vec![MemoryType::Decision]);
        });
    }

    #[test]
    fn test_scope_filtering() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = tempfile::TempDir::new().unwrap();
            let pools = nous_core::db::DbPools::connect(tmp.path()).await.unwrap();

            let ws_index = make_index(
                pools.fts.clone(),
                pools.vec.clone(),
                MemoryScope::Workspace,
                RetrievalStrategy::Fts,
            );
            assert_eq!(ws_index.scope_workspace_id(), Some("test-workspace"));
            assert_eq!(ws_index.scope_agent_id(), None);

            let agent_index = make_index(
                pools.fts.clone(),
                pools.vec.clone(),
                MemoryScope::Agent,
                RetrievalStrategy::Fts,
            );
            assert_eq!(agent_index.scope_workspace_id(), Some("test-workspace"));
            assert_eq!(agent_index.scope_agent_id(), Some("test-agent"));

            let session_index = make_index(
                pools.fts.clone(),
                pools.vec.clone(),
                MemoryScope::Session,
                RetrievalStrategy::Fts,
            );
            assert_eq!(session_index.scope_workspace_id(), Some("test-workspace"));
            assert_eq!(session_index.scope_agent_id(), Some("test-agent"));

            let shared_index = make_index(
                pools.fts.clone(),
                pools.vec.clone(),
                MemoryScope::Shared(vec!["a".into(), "b".into()]),
                RetrievalStrategy::Fts,
            );
            assert_eq!(shared_index.scope_workspace_id(), Some("test-workspace"));
            assert_eq!(shared_index.scope_agent_id(), None);
        });
    }

    #[test]
    fn test_retrieval_strategy_dispatch() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = tempfile::TempDir::new().unwrap();
            let pools = nous_core::db::DbPools::connect(tmp.path()).await.unwrap();
            pools.run_migrations("porter unicode61").await.unwrap();

            for strategy in [
                RetrievalStrategy::Fts,
                RetrievalStrategy::Vector,
                RetrievalStrategy::Hybrid,
                RetrievalStrategy::Recency,
            ] {
                let index = make_index(
                    pools.fts.clone(),
                    pools.vec.clone(),
                    MemoryScope::Workspace,
                    strategy,
                );

                let results = index.search_by_strategy("test query", 5).await;
                assert!(
                    results.is_ok(),
                    "strategy {:?} failed: {:?}",
                    index.retrieval,
                    results.err()
                );
                assert!(results.unwrap().is_empty());
            }
        });
    }

    #[test]
    fn test_trait_impl_compiles() {
        fn assert_vector_store_index<T: VectorStoreIndex>() {}
        assert_vector_store_index::<NousMemoryIndex>();
    }

    #[test]
    fn test_error_conversion() {
        let nous_err = NousError::NotFound("test".into());
        let store_err = super::nous_err_to_store(nous_err);
        assert!(matches!(store_err, VectorStoreError::DatastoreError(_)));
    }
}
