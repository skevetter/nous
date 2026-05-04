use std::sync::Arc;

use serde_json::Value;

use crate::db::{DatabaseConnection, VecPool};
use crate::memory::{self, Embedder};
use crate::messages::{self, PostMessageRequest, ReadMessagesRequest};
use crate::notifications::NotificationRegistry;
use crate::rooms;
use crate::tasks;

use super::ToolError;
use super::ToolServices;

pub struct DaemonToolServices {
    pub pool: DatabaseConnection,
    pub vec_pool: VecPool,
    pub embedder: Option<Arc<dyn Embedder>>,
    pub registry: Arc<NotificationRegistry>,
}

impl DaemonToolServices {
    pub fn new(
        pool: DatabaseConnection,
        vec_pool: VecPool,
        embedder: Option<Arc<dyn Embedder>>,
        registry: Arc<NotificationRegistry>,
    ) -> Self {
        Self {
            pool,
            vec_pool,
            embedder,
            registry,
        }
    }
}

fn map_err(e: impl std::fmt::Display) -> ToolError {
    ToolError::ExecutionFailed(e.to_string())
}

fn to_json<T: serde::Serialize>(v: T) -> Result<serde_json::Value, ToolError> {
    serde_json::to_value(v).map_err(|e| ToolError::ExecutionFailed(e.to_string()))
}

#[async_trait::async_trait]
impl ToolServices for DaemonToolServices {
    async fn save_memory(&self, params: super::SaveMemoryParams) -> Result<Value, ToolError> {
        let super::SaveMemoryParams { workspace_id, agent_id, content, memory_type, importance, .. } = params;
        let memory_type: memory::MemoryType =
            memory_type.parse().map_err(|e: crate::error::NousError| ToolError::InvalidArgs(e.to_string()))?;
        let importance: memory::Importance =
            importance.parse().map_err(|e: crate::error::NousError| ToolError::InvalidArgs(e.to_string()))?;
        let mem = memory::save_memory(
            &self.pool,
            memory::SaveMemoryRequest {
                workspace_id,
                agent_id: Some(agent_id),
                title: content.chars().take(80).collect(),
                content,
                memory_type,
                importance: Some(importance),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .map_err(map_err)?;
        to_json(mem)
    }

    async fn search_memories(
        &self,
        params: super::SearchMemoriesParams,
    ) -> Result<Value, ToolError> {
        let super::SearchMemoriesParams { query, agent_id, workspace_id, memory_type, limit } =
            params;
        let memory_type = memory_type
            .map(|s| s.parse::<memory::MemoryType>())
            .transpose()
            .map_err(|e: crate::error::NousError| ToolError::InvalidArgs(e.to_string()))?;
        let start = std::time::Instant::now();
        let results = memory::search_memories(
            &self.pool,
            &memory::SearchMemoryRequest {
                query: query.clone(),
                workspace_id: workspace_id.clone(),
                agent_id: agent_id.clone(),
                memory_type,
                importance: None,
                include_archived: false,
                limit,
            },
        )
        .await
        .map_err(map_err)?;
        let latency_ms = start.elapsed().as_millis() as i64;
        if let Err(e) = memory::analytics::record_search_event(
            &self.pool,
            &memory::analytics::SearchEvent {
                query_text: query,
                search_type: "fts".to_string(),
                result_count: i64::try_from(results.len()).unwrap_or(i64::MAX),
                latency_ms,
                workspace_id,
                agent_id,
            },
        )
        .await
        {
            tracing::debug!(error = %e, "failed to record fts search analytics");
        }
        to_json(results)
    }

    async fn search_memories_hybrid(
        &self,
        params: super::SearchMemoriesHybridParams,
    ) -> Result<Value, ToolError> {
        let super::SearchMemoriesHybridParams { query, agent_id, limit, fts_weight: _ } = params;
        let limit = limit.unwrap_or(10) as usize;
        let start = std::time::Instant::now();

        let query_embedding = self
            .embedder
            .as_ref()
            .and_then(|embedder| embedder.embed(&[&query]).ok())
            .and_then(|mut vecs| vecs.pop());

        if let Some(embedding) = query_embedding {
            let results = memory::search_hybrid_filtered(memory::SearchHybridFilteredParams {
                fts_db: &self.pool,
                vec_pool: &self.vec_pool,
                query: &query,
                query_embedding: &embedding,
                limit,
                workspace_id: None,
                agent_id: agent_id.as_deref(),
                memory_type: None,
            })
            .await
            .map_err(map_err)?;
            let latency_ms = start.elapsed().as_millis() as i64;
            if let Err(e) = memory::analytics::record_search_event(
                &self.pool,
                &memory::analytics::SearchEvent {
                    query_text: query,
                    search_type: "hybrid".to_string(),
                    result_count: i64::try_from(results.len()).unwrap_or(i64::MAX),
                    latency_ms,
                    workspace_id: None,
                    agent_id,
                },
            )
            .await
            {
                tracing::debug!(error = %e, "failed to record hybrid search analytics");
            }
            to_json(results)
        } else {
            let fts_results = memory::search_memories(
                &self.pool,
                &memory::SearchMemoryRequest {
                    query: query.clone(),
                    agent_id: agent_id.clone(),
                    limit: Some(limit as u32),
                    ..Default::default()
                },
            )
            .await
            .map_err(map_err)?;
            let latency_ms = start.elapsed().as_millis() as i64;
            if let Err(e) = memory::analytics::record_search_event(
                &self.pool,
                &memory::analytics::SearchEvent {
                    query_text: query,
                    search_type: "fts5_fallback".to_string(),
                    result_count: i64::try_from(fts_results.len()).unwrap_or(i64::MAX),
                    latency_ms,
                    workspace_id: None,
                    agent_id,
                },
            )
            .await
            {
                tracing::debug!(error = %e, "failed to record fts5_fallback search analytics");
            }
            Ok(serde_json::json!({
                "results": fts_results,
                "_warning": "embedding unavailable, fell back to FTS5-only search"
            }))
        }
    }

    async fn get_memory_context(
        &self,
        params: super::GetMemoryContextParams,
    ) -> Result<Value, ToolError> {
        let super::GetMemoryContextParams { agent_id, workspace_id, topic_key, limit } = params;
        let results = memory::get_context(
            &self.pool,
            &memory::ContextRequest {
                workspace_id,
                agent_id,
                topic_key,
                limit,
            },
        )
        .await
        .map_err(map_err)?;
        to_json(results)
    }

    async fn relate_memories(
        &self,
        source_id: String,
        target_id: String,
        relation_type: String,
    ) -> Result<Value, ToolError> {
        let relation_type: memory::RelationType = relation_type
            .parse()
            .map_err(|e: crate::error::NousError| ToolError::InvalidArgs(e.to_string()))?;
        let rel = memory::relate_memories(
            &self.pool,
            &memory::RelateRequest {
                source_id,
                target_id,
                relation_type,
            },
        )
        .await
        .map_err(map_err)?;
        to_json(rel)
    }

    async fn update_memory(
        &self,
        memory_id: String,
        content: Option<String>,
        importance: Option<String>,
    ) -> Result<Value, ToolError> {
        let importance = importance
            .map(|s| s.parse::<memory::Importance>())
            .transpose()
            .map_err(|e: crate::error::NousError| ToolError::InvalidArgs(e.to_string()))?;
        let mem = memory::update_memory(
            &self.pool,
            memory::UpdateMemoryRequest {
                id: memory_id,
                title: None,
                content,
                importance,
                topic_key: None,
                valid_from: None,
                valid_until: None,
                archived: None,
            },
        )
        .await
        .map_err(map_err)?;
        to_json(mem)
    }

    async fn post_to_room(&self, params: super::PostToRoomParams) -> Result<Value, ToolError> {
        let super::PostToRoomParams { room, sender_id, content, reply_to } = params;
        let room_obj = rooms::get_room(&self.pool, &room)
            .await
            .map_err(map_err)?;
        let msg = messages::post_message(
            &self.pool,
            PostMessageRequest {
                room_id: room_obj.id,
                sender_id,
                content,
                reply_to,
                metadata: None,
                message_type: None,
            },
            Some(&self.registry),
        )
        .await
        .map_err(map_err)?;
        to_json(msg)
    }

    async fn read_room(&self, room: String, limit: Option<u32>) -> Result<Value, ToolError> {
        let room_obj = rooms::get_room(&self.pool, &room)
            .await
            .map_err(map_err)?;
        let messages = messages::read_messages(
            &self.pool,
            ReadMessagesRequest {
                room_id: room_obj.id,
                since: None,
                before: None,
                limit,
            },
        )
        .await
        .map_err(map_err)?;
        to_json(messages)
    }

    async fn create_room(&self, name: String, purpose: Option<String>) -> Result<Value, ToolError> {
        let room = rooms::create_room(&self.pool, &name, purpose.as_deref(), None)
            .await
            .map_err(map_err)?;
        to_json(room)
    }

    async fn wait_for_message(&self, room: String, timeout_secs: u64) -> Result<Value, ToolError> {
        let room_obj = rooms::get_room(&self.pool, &room)
            .await
            .map_err(map_err)?;

        // Get the current latest message's created_at as the baseline so we only
        // return messages that arrive AFTER this call begins.
        let existing = messages::read_messages(
            &self.pool,
            ReadMessagesRequest {
                room_id: room_obj.id.clone(),
                since: None,
                before: None,
                limit: Some(200),
            },
        )
        .await
        .map_err(map_err)?;
        let mut last_created_at = existing.last().map(|m| m.created_at.clone());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        loop {
            if std::time::Instant::now() >= deadline {
                return Ok(serde_json::json!({"timeout": true, "message": null}));
            }

            let messages = messages::read_messages(
                &self.pool,
                ReadMessagesRequest {
                    room_id: room_obj.id.clone(),
                    since: last_created_at.clone(),
                    before: None,
                    limit: Some(1),
                },
            )
            .await
            .map_err(map_err)?;

            if let Some(msg) = messages.into_iter().next() {
                // Update baseline so subsequent iterations won't re-return this message
                let _ = last_created_at.insert(msg.created_at.clone());
                return to_json(&msg);
            }

            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    async fn create_task(&self, params: super::ToolCreateTaskParams) -> Result<Value, ToolError> {
        let super::ToolCreateTaskParams { title, description, assignee, priority } = params;
        let task = tasks::create_task(tasks::CreateTaskParams {
            db: &self.pool,
            title: &title,
            description: description.as_deref(),
            priority: priority.as_deref(),
            assignee_id: assignee.as_deref(),
            labels: None,
            room_id: None,
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .map_err(map_err)?;
        to_json(task)
    }

    async fn update_task(
        &self,
        task_id: String,
        status: Option<String>,
        _note: Option<String>,
    ) -> Result<Value, ToolError> {
        let task = tasks::update_task(tasks::UpdateTaskParams {
            db: &self.pool,
            id: &task_id,
            status: status.as_deref(),
            priority: None,
            assignee_id: None,
            description: None,
            labels: None,
            actor_id: None,
            registry: None,
        })
        .await
        .map_err(map_err)?;
        to_json(task)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::memory::{Importance, MemoryType, MockEmbedder};
    use tempfile::TempDir;

    async fn setup() -> (DaemonToolServices, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();

        use sea_orm::ConnectionTrait;
        for agent_id in ["test-agent", "agent-1"] {
            pools
                .fts
                .execute_unprepared(&format!(
                    "INSERT OR IGNORE INTO agents (id, name, namespace, status) \
                     VALUES ('{agent_id}', '{agent_id}', 'default', 'active')"
                ))
                .await
                .unwrap();
        }

        let svc = DaemonToolServices::new(
            pools.fts,
            pools.vec,
            Some(Arc::new(MockEmbedder::new())),
            Arc::new(NotificationRegistry::new()),
        );
        (svc, tmp)
    }

    #[tokio::test]
    async fn save_and_search_memory() {
        let (svc, _tmp) = setup().await;
        let result = svc
            .save_memory(crate::tools::SaveMemoryParams {
                workspace_id: None,
                agent_id: "test-agent".into(),
                content: "Rust async patterns with tokio".into(),
                memory_type: "convention".into(),
                importance: "high".into(),
                tags: vec![],
            })
            .await
            .unwrap();
        assert!(result.get("id").is_some());

        let results = svc
            .search_memories(crate::tools::SearchMemoriesParams {
                query: "tokio".into(),
                agent_id: None,
                workspace_id: None,
                memory_type: None,
                limit: Some(10),
            })
            .await
            .unwrap();
        let arr = results.as_array().unwrap();
        assert!(!arr.is_empty());
    }

    #[tokio::test]
    async fn hybrid_search_with_embedder() {
        let (svc, _tmp) = setup().await;
        svc.save_memory(crate::tools::SaveMemoryParams {
            workspace_id: None,
            agent_id: "test-agent".into(),
            content: "Database connection pooling strategies".into(),
            memory_type: "architecture".into(),
            importance: "moderate".into(),
            tags: vec![],
        })
        .await
        .unwrap();

        let results = svc
            .search_memories_hybrid(crate::tools::SearchMemoriesHybridParams {
                query: "database pooling".into(),
                agent_id: None,
                limit: Some(10),
                fts_weight: None,
            })
            .await
            .unwrap();
        assert!(results.get("results").is_some() || results.as_array().is_some());
    }

    #[tokio::test]
    async fn get_context() {
        let (svc, _tmp) = setup().await;
        svc.save_memory(crate::tools::SaveMemoryParams {
            workspace_id: Some("ws-1".into()),
            agent_id: "test-agent".into(),
            content: "Context memory content".into(),
            memory_type: "decision".into(),
            importance: "high".into(),
            tags: vec![],
        })
        .await
        .unwrap();

        let results = svc
            .get_memory_context(crate::tools::GetMemoryContextParams {
                agent_id: Some("test-agent".into()),
                workspace_id: Some("ws-1".into()),
                topic_key: None,
                limit: Some(5),
            })
            .await
            .unwrap();
        let arr = results.as_array().unwrap();
        assert!(!arr.is_empty());
    }

    #[tokio::test]
    async fn relate_and_update_memory() {
        let (svc, _tmp) = setup().await;
        let mem1 = svc
            .save_memory(crate::tools::SaveMemoryParams {
                workspace_id: None,
                agent_id: "test-agent".into(),
                content: "First memory".into(),
                memory_type: "convention".into(),
                importance: "high".into(),
                tags: vec![],
            })
            .await
            .unwrap();
        let mem2 = svc
            .save_memory(crate::tools::SaveMemoryParams {
                workspace_id: None,
                agent_id: "test-agent".into(),
                content: "Second memory".into(),
                memory_type: "convention".into(),
                importance: "high".into(),
                tags: vec![],
            })
            .await
            .unwrap();

        let id1 = mem1["id"].as_str().unwrap().to_string();
        let id2 = mem2["id"].as_str().unwrap().to_string();

        let rel = svc
            .relate_memories(id1.clone(), id2, "supersedes".into())
            .await
            .unwrap();
        assert!(rel.get("source_id").is_some());

        let updated = svc
            .update_memory(id1, Some("Updated content".into()), Some("moderate".into()))
            .await
            .unwrap();
        assert_eq!(updated["content"], "Updated content");
    }

    #[tokio::test]
    async fn room_operations() {
        let (svc, _tmp) = setup().await;
        let room = svc
            .create_room("test-room".into(), Some("testing".into()))
            .await
            .unwrap();
        assert_eq!(room["name"], "test-room");

        let msg = svc
            .post_to_room(crate::tools::PostToRoomParams {
                room: "test-room".into(),
                sender_id: "test-agent".into(),
                content: "Hello world".into(),
                reply_to: None,
            })
            .await
            .unwrap();
        assert_eq!(msg["content"], "Hello world");

        let messages = svc.read_room("test-room".into(), Some(10)).await.unwrap();
        let arr = messages.as_array().unwrap();
        assert!(!arr.is_empty());
    }

    #[tokio::test]
    async fn task_operations() {
        let (svc, _tmp) = setup().await;
        let task = svc
            .create_task(crate::tools::ToolCreateTaskParams {
                title: "Test task".into(),
                description: Some("A test task description".into()),
                assignee: None,
                priority: Some("high".into()),
            })
            .await
            .unwrap();
        assert_eq!(task["title"], "Test task");
        assert_eq!(task["status"], "open");

        let task_id = task["id"].as_str().unwrap().to_string();
        let updated = svc
            .update_task(task_id, Some("in_progress".into()), None)
            .await
            .unwrap();
        assert_eq!(updated["status"], "in_progress");
    }
}
