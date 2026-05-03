use std::sync::Arc;

use nous_core::agents::definition::{MemoryScope, RetrievalStrategy};
use nous_core::db::DatabaseConnection;
use nous_core::db::VecPool;
use nous_core::error::NousError;
use nous_core::memory::{
    self, ContextRequest, Embedder, MemoryType, SearchMemoryRequest, SimilarMemory,
};
use rig::vector_store::request::Filter;
use rig::vector_store::{VectorSearchRequest, VectorStoreError, VectorStoreIndex};
use serde::Deserialize;

pub struct NousMemoryIndex {
    fts_pool: DatabaseConnection,
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
        fts_pool: DatabaseConnection,
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

/// A parsed memory block extracted from agent output.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMemoryBlock {
    pub memory_type: MemoryType,
    pub importance: String,
    pub title: String,
    pub content: String,
}

/// Parse MEMORY[...]...END_MEMORY blocks from agent completion text.
///
/// Format:
///   MEMORY[type=decision, importance=high, title="Use RRF for hybrid search"]
///   Content goes here...
///   END_MEMORY
pub fn parse_memory_blocks(text: &str) -> Vec<ParsedMemoryBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("MEMORY[") {
            if let Some(header) = parse_header(line) {
                let mut content_lines = Vec::new();
                i += 1;
                let mut found_end = false;
                while i < lines.len() {
                    if lines[i].trim() == "END_MEMORY" {
                        found_end = true;
                        break;
                    }
                    content_lines.push(lines[i]);
                    i += 1;
                }
                if found_end {
                    blocks.push(ParsedMemoryBlock {
                        memory_type: header.0,
                        importance: header.1,
                        title: header.2,
                        content: content_lines.join("\n"),
                    });
                }
            }
        }
        i += 1;
    }

    blocks
}

fn parse_header(line: &str) -> Option<(MemoryType, String, String)> {
    let start = line.find('[')?;
    let end = line.rfind(']')?;
    if start >= end {
        return None;
    }
    let attrs_str = &line[start + 1..end];

    let mut mem_type: Option<MemoryType> = None;
    let mut importance = "moderate".to_string();
    let mut title: Option<String> = None;

    let mut remaining = attrs_str.trim();
    while !remaining.is_empty() {
        remaining = remaining.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        if remaining.is_empty() {
            break;
        }

        let eq_pos = remaining.find('=')?;
        let key = remaining[..eq_pos].trim();
        remaining = remaining[eq_pos + 1..].trim();

        let value;
        if remaining.starts_with('"') {
            let close_quote = remaining[1..].find('"')?;
            value = &remaining[1..close_quote + 1];
            remaining = &remaining[close_quote + 2..];
        } else {
            let end_pos = remaining.find(',').unwrap_or(remaining.len());
            value = remaining[..end_pos].trim();
            remaining = &remaining[end_pos..];
        }

        match key {
            "type" => {
                mem_type = Some(value.parse::<MemoryType>().ok()?);
            }
            "importance" => {
                importance = value.to_string();
            }
            "title" => {
                title = Some(value.to_string());
            }
            _ => {}
        }
    }

    Some((mem_type?, importance, title?))
}

/// Extract memories from agent response and save them based on auto_save config.
/// Only saves memories whose type is in `allowed_types`.
pub async fn extract_and_save_memories(
    response: &str,
    agent_id: &str,
    workspace_id: &str,
    allowed_types: &[MemoryType],
    fts_pool: &DatabaseConnection,
) -> Result<Vec<String>, NousError> {
    let blocks = parse_memory_blocks(response);
    let mut saved_ids = Vec::new();
    for block in blocks {
        if !allowed_types.contains(&block.memory_type) {
            continue;
        }
        let imp = block
            .importance
            .parse::<memory::Importance>()
            .unwrap_or(memory::Importance::Moderate);
        let req = memory::SaveMemoryRequest {
            workspace_id: Some(workspace_id.to_string()),
            agent_id: Some(agent_id.to_string()),
            title: block.title,
            content: block.content,
            memory_type: block.memory_type,
            importance: Some(imp),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        };
        let saved = memory::save_memory(fts_pool, req).await?;
        saved_ids.push(saved.id);
    }
    Ok(saved_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nous_core::memory::MockEmbedder;

    fn make_index(
        fts_pool: DatabaseConnection,
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
            pools.run_migrations().await.unwrap();

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

    #[test]
    fn test_parse_memory_blocks_valid() {
        let input = r#"Some preamble text.
MEMORY[type=decision, importance=high, title="Use RRF for hybrid search"]
We decided to use Reciprocal Rank Fusion for combining FTS and vector results.
END_MEMORY
Some trailing text."#;

        let blocks = super::parse_memory_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].memory_type, MemoryType::Decision);
        assert_eq!(blocks[0].importance, "high");
        assert_eq!(blocks[0].title, "Use RRF for hybrid search");
        assert!(blocks[0].content.contains("Reciprocal Rank Fusion"));
    }

    #[test]
    fn test_parse_memory_blocks_multiple() {
        let input = r#"MEMORY[type=convention, importance=moderate, title="snake_case for functions"]
All function names must use snake_case.
END_MEMORY
Some text in between.
MEMORY[type=bugfix, importance=high, title="Fix null pointer in parser"]
The parser was dereferencing a null pointer when input was empty.
END_MEMORY"#;

        let blocks = super::parse_memory_blocks(input);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].memory_type, MemoryType::Convention);
        assert_eq!(blocks[0].title, "snake_case for functions");
        assert_eq!(blocks[1].memory_type, MemoryType::Bugfix);
        assert_eq!(blocks[1].title, "Fix null pointer in parser");
    }

    #[test]
    fn test_parse_memory_blocks_malformed() {
        let input = r#"MEMORY[type=decision, importance=high, title="Unclosed block"]
This block has no END_MEMORY marker.
Some more text."#;

        let blocks = super::parse_memory_blocks(input);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_parse_memory_blocks_invalid_type() {
        let input = r#"MEMORY[type=invalid, importance=high, title="Bad type"]
This has an invalid memory type.
END_MEMORY"#;

        let blocks = super::parse_memory_blocks(input);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_parse_memory_blocks_default_importance() {
        let input = r#"MEMORY[type=fact, title="Rust is fast"]
Rust compiles to native code and has zero-cost abstractions.
END_MEMORY"#;

        let blocks = super::parse_memory_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].importance, "moderate");
        assert_eq!(blocks[0].memory_type, MemoryType::Fact);
    }

    #[test]
    fn test_extract_and_save_memories() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tmp = tempfile::TempDir::new().unwrap();
            let pools = nous_core::db::DbPools::connect(tmp.path()).await.unwrap();
            pools.run_migrations().await.unwrap();

            let response = r#"Here is my analysis.
MEMORY[type=decision, importance=high, title="Use SQLite for storage"]
We chose SQLite for its simplicity and embedded nature.
END_MEMORY
MEMORY[type=observation, importance=low, title="Performance is good"]
Benchmarks show sub-millisecond queries.
END_MEMORY"#;

            let allowed = vec![MemoryType::Decision];
            let ids =
                super::extract_and_save_memories(response, "agent-1", "ws-1", &allowed, &pools.fts)
                    .await
                    .unwrap();

            assert_eq!(ids.len(), 1, "only Decision type should be saved");

            let mem = memory::get_memory_by_id(&pools.fts, &ids[0]).await.unwrap();
            assert_eq!(mem.title, "Use SQLite for storage");
            assert_eq!(mem.agent_id.as_deref(), Some("agent-1"));
            assert_eq!(mem.workspace_id, "ws-1");
        });
    }
}
