use std::str::FromStr;
use std::sync::Arc;

use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::chunk::Chunker;
use nous_core::classify::CategoryClassifier;
use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use nous_core::types::{
    Confidence, Importance, MemoryPatch, MemoryType, MemoryWithRelations, NewMemory,
};
use nous_shared::ids::MemoryId;
use rmcp::model::CallToolResult;
use rmcp::schemars;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryStoreParams {
    pub title: String,
    pub content: String,
    pub memory_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: Option<String>,
    pub importance: Option<String>,
    pub confidence: Option<String>,
    pub workspace_path: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_model: Option<String>,
    pub valid_from: Option<String>,
    pub category_id: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryRecallParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySearchParams {
    pub query: String,
    pub mode: Option<String>,
    pub memory_type: Option<String>,
    pub category_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub importance: Option<String>,
    pub confidence: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub since: Option<String>,
    pub until: Option<String>,
    pub valid_only: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryContextParams {
    pub workspace_path: String,
    #[serde(default)]
    pub summary: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryForgetParams {
    pub id: String,
    #[serde(default)]
    pub hard: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryUnarchiveParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryUpdateParams {
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub importance: Option<String>,
    pub confidence: Option<String>,
    pub valid_until: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryRelateParams {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryUnrelateParams {
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryCategorySuggestParams {
    pub memory_id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryWorkspacesParams {
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryTagsParams {
    pub prefix: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySqlParams {
    pub query: String,
}

fn ok_json(value: &serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![rmcp::model::Content::text(value.to_string())])
}

fn err_result(msg: &str) -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::Content::text(msg)])
}

fn parse_enum<T: FromStr>(value: &str, field_name: &str) -> Result<T, CallToolResult>
where
    T::Err: std::fmt::Display,
{
    T::from_str(value).map_err(|e| err_result(&format!("invalid {field_name}: {e}")))
}

fn recall_to_json(r: &MemoryWithRelations) -> serde_json::Value {
    serde_json::json!({
        "id": r.memory.id,
        "title": r.memory.title,
        "content": r.memory.content,
        "memory_type": r.memory.memory_type,
        "source": r.memory.source,
        "importance": r.memory.importance,
        "confidence": r.memory.confidence,
        "workspace_id": r.memory.workspace_id,
        "session_id": r.memory.session_id,
        "trace_id": r.memory.trace_id,
        "agent_id": r.memory.agent_id,
        "agent_model": r.memory.agent_model,
        "valid_from": r.memory.valid_from,
        "valid_until": r.memory.valid_until,
        "archived": r.memory.archived,
        "category_id": r.memory.category_id,
        "created_at": r.memory.created_at,
        "updated_at": r.memory.updated_at,
        "tags": r.tags,
        "relationships": r.relationships.iter().map(|rel| serde_json::json!({
            "source_id": rel.source_id,
            "target_id": rel.target_id,
            "relation_type": rel.relation_type,
        })).collect::<Vec<_>>(),
        "category": r.category.as_ref().map(|c| serde_json::json!({
            "id": c.id,
            "name": c.name,
        })),
        "access_count": r.access_count,
    })
}

pub async fn handle_store(
    params: MemoryStoreParams,
    write_channel: &WriteChannel,
    embedding: &Arc<dyn EmbeddingBackend>,
    classifier: &CategoryClassifier,
    chunker: &Chunker,
) -> CallToolResult {
    let memory_type = match parse_enum::<MemoryType>(&params.memory_type, "memory_type") {
        Ok(v) => v,
        Err(e) => return e,
    };

    let importance = match params.importance.as_deref() {
        Some(v) => match parse_enum::<Importance>(v, "importance") {
            Ok(v) => v,
            Err(e) => return e,
        },
        None => Importance::default(),
    };

    let confidence = match params.confidence.as_deref() {
        Some(v) => match parse_enum::<Confidence>(v, "confidence") {
            Ok(v) => v,
            Err(e) => return e,
        },
        None => Confidence::default(),
    };

    let content_embedding = match embedding.embed_one(&params.content) {
        Ok(v) => v,
        Err(e) => return err_result(&format!("embedding failed: {e}")),
    };

    let category_id = match params.category_id {
        Some(id) => Some(id),
        None => classifier.classify(&content_embedding),
    };

    let new_memory = NewMemory {
        title: params.title,
        content: params.content.clone(),
        memory_type,
        source: params.source,
        importance,
        confidence,
        tags: params.tags,
        workspace_path: params.workspace_path,
        session_id: params.session_id,
        trace_id: params.trace_id,
        agent_id: params.agent_id,
        agent_model: params.agent_model,
        valid_from: params.valid_from,
        category_id,
    };

    let id = match write_channel.store(new_memory).await {
        Ok(id) => id,
        Err(e) => return err_result(&format!("store failed: {e}")),
    };

    let chunks = chunker.chunk(&params.content);
    if !chunks.is_empty() {
        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        match embedding.embed(&texts) {
            Ok(embeddings) => {
                if let Err(e) = write_channel
                    .store_chunks(id.clone(), chunks, embeddings)
                    .await
                {
                    return err_result(&format!("store_chunks failed: {e}"));
                }
            }
            Err(e) => return err_result(&format!("chunk embedding failed: {e}")),
        }
    }

    ok_json(&serde_json::json!({
        "id": id.to_string(),
        "category_id": category_id,
    }))
}

pub async fn handle_recall(
    params: MemoryRecallParams,
    read_pool: &ReadPool,
    write_channel: &WriteChannel,
) -> CallToolResult {
    let id = match params.id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid id: {e}")),
    };

    let result = match read_pool
        .with_conn({
            let id = id.clone();
            move |conn| MemoryDb::recall_on(conn, &id)
        })
        .await
    {
        Ok(r) => r,
        Err(e) => return err_result(&format!("recall failed: {e}")),
    };

    let Some(memory) = result else {
        return err_result("memory not found");
    };

    let _ = write_channel.log_access(id, "recall".into()).await;

    ok_json(&recall_to_json(&memory))
}

pub async fn handle_update(
    params: MemoryUpdateParams,
    write_channel: &WriteChannel,
    embedding: &Arc<dyn EmbeddingBackend>,
    chunker: &Chunker,
    read_pool: &ReadPool,
) -> CallToolResult {
    let id = match params.id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid id: {e}")),
    };

    let importance = match params.importance.as_deref() {
        Some(v) => match parse_enum::<Importance>(v, "importance") {
            Ok(v) => Some(v),
            Err(e) => return e,
        },
        None => None,
    };

    let confidence = match params.confidence.as_deref() {
        Some(v) => match parse_enum::<Confidence>(v, "confidence") {
            Ok(v) => Some(v),
            Err(e) => return e,
        },
        None => None,
    };

    let content_changed = params.content.is_some();

    let patch = MemoryPatch {
        title: params.title,
        content: params.content.clone(),
        tags: params.tags,
        importance,
        confidence,
        valid_until: params.valid_until,
    };

    match write_channel.update(id.clone(), patch).await {
        Ok(true) => {}
        Ok(false) => return err_result("memory not found"),
        Err(e) => return err_result(&format!("update failed: {e}")),
    }

    if content_changed {
        let new_content = params.content.unwrap();

        if let Err(e) = write_channel.delete_chunks(id.clone()).await {
            return err_result(&format!("delete_chunks failed: {e}"));
        }

        let chunks = chunker.chunk(&new_content);
        if !chunks.is_empty() {
            let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            match embedding.embed(&texts) {
                Ok(embeddings) => {
                    if let Err(e) = write_channel
                        .store_chunks(id.clone(), chunks, embeddings)
                        .await
                    {
                        return err_result(&format!("store_chunks failed: {e}"));
                    }
                }
                Err(e) => return err_result(&format!("chunk embedding failed: {e}")),
            }
        }
    }

    let result = match read_pool
        .with_conn({
            let id = id.clone();
            move |conn| MemoryDb::recall_on(conn, &id)
        })
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return err_result("memory not found after update"),
        Err(e) => return err_result(&format!("recall after update failed: {e}")),
    };

    ok_json(&recall_to_json(&result))
}

pub async fn handle_forget(
    params: MemoryForgetParams,
    write_channel: &WriteChannel,
) -> CallToolResult {
    let id = match params.id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid id: {e}")),
    };

    match write_channel.forget(id, params.hard).await {
        Ok(true) => ok_json(&serde_json::json!({
            "status": if params.hard { "deleted" } else { "archived" },
        })),
        Ok(false) => err_result("memory not found"),
        Err(e) => err_result(&format!("forget failed: {e}")),
    }
}

pub async fn handle_unarchive(
    params: MemoryUnarchiveParams,
    write_channel: &WriteChannel,
    embedding: &Arc<dyn EmbeddingBackend>,
    chunker: &Chunker,
    read_pool: &ReadPool,
) -> CallToolResult {
    let id = match params.id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid id: {e}")),
    };

    match write_channel.unarchive(id.clone()).await {
        Ok(true) => {}
        Ok(false) => return err_result("memory not found or not archived"),
        Err(e) => return err_result(&format!("unarchive failed: {e}")),
    }

    let result = match read_pool
        .with_conn({
            let id = id.clone();
            move |conn| MemoryDb::recall_on(conn, &id)
        })
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return err_result("memory not found after unarchive"),
        Err(e) => return err_result(&format!("recall after unarchive failed: {e}")),
    };

    if let Err(e) = write_channel.delete_chunks(id.clone()).await {
        return err_result(&format!("delete_chunks failed: {e}"));
    }

    let chunks = chunker.chunk(&result.memory.content);
    if !chunks.is_empty() {
        let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
        match embedding.embed(&texts) {
            Ok(embeddings) => {
                if let Err(e) = write_channel.store_chunks(id, chunks, embeddings).await {
                    return err_result(&format!("store_chunks failed: {e}"));
                }
            }
            Err(e) => return err_result(&format!("chunk embedding failed: {e}")),
        }
    }

    ok_json(&recall_to_json(&result))
}
