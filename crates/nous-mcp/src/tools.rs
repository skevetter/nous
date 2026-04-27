use std::str::FromStr;
use std::sync::Arc;

use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::chunk::Chunker;
use nous_core::classify::CategoryClassifier;
use nous_core::cron_parser::CronExpr;
use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use nous_core::schedule_db::ScheduleDb;
use nous_core::types::{
    ActionType, CategorySource, Confidence, Importance, MemoryPatch, MemoryType,
    MemoryWithRelations, NewMemory, RelationType, Schedule, SchedulePatch, SearchFilters,
    SearchMode,
};
use nous_otlp::db::OtlpDb;
use nous_shared::ids::MemoryId;
use rmcp::model::CallToolResult;
use rmcp::schemars;
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::Notify;

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
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
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
    #[serde(default)]
    pub description: Option<String>,
    pub parent_id: Option<i64>,
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
pub struct MemoryCategoryListParams {
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryCategoryAddParams {
    pub name: String,
    pub parent: Option<String>,
    pub description: Option<String>,
    pub threshold: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryCategoryDeleteParams {
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryCategoryUpdateParams {
    pub name: String,
    pub new_name: Option<String>,
    pub description: Option<String>,
    pub threshold: Option<f32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySqlParams {
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OtlpTraceContextParams {
    pub trace_id: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OtlpMemoryContextParams {
    pub memory_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomCreateParams {
    pub name: String,
    pub purpose: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomListParams {
    #[serde(default)]
    pub archived: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomGetParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomDeleteParams {
    pub id: String,
    #[serde(default)]
    pub hard: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomPostMessageParams {
    pub room_id: String,
    pub content: String,
    pub sender_id: Option<String>,
    pub reply_to: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomReadMessagesParams {
    pub room_id: String,
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub since: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomSearchParams {
    pub room_id: String,
    pub query: String,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomInfoParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomJoinParams {
    pub room_id: String,
    pub agent_id: String,
    pub role: Option<String>,
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

pub async fn handle_relate(
    params: MemoryRelateParams,
    write_channel: &WriteChannel,
) -> CallToolResult {
    let source_id = match params.source_id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid source_id: {e}")),
    };
    let target_id = match params.target_id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid target_id: {e}")),
    };
    let relation_type = match parse_enum::<RelationType>(&params.relation_type, "relation_type") {
        Ok(v) => v,
        Err(e) => return e,
    };

    match write_channel
        .relate(source_id, target_id, relation_type)
        .await
    {
        Ok(()) => ok_json(&serde_json::json!({"status": "related"})),
        Err(e) => err_result(&format!("relate failed: {e}")),
    }
}

pub async fn handle_unrelate(
    params: MemoryUnrelateParams,
    write_channel: &WriteChannel,
) -> CallToolResult {
    let source_id = match params.source_id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid source_id: {e}")),
    };
    let target_id = match params.target_id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid target_id: {e}")),
    };
    let relation_type = match parse_enum::<RelationType>(&params.relation_type, "relation_type") {
        Ok(v) => v,
        Err(e) => return e,
    };

    match write_channel
        .unrelate(source_id, target_id, relation_type)
        .await
    {
        Ok(true) => ok_json(&serde_json::json!({"status": "unrelated"})),
        Ok(false) => err_result("relationship not found"),
        Err(e) => err_result(&format!("unrelate failed: {e}")),
    }
}

pub async fn handle_category_suggest(
    params: MemoryCategorySuggestParams,
    write_channel: &WriteChannel,
    embedding: &Arc<dyn EmbeddingBackend>,
) -> CallToolResult {
    let memory_id = match params.memory_id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid memory_id: {e}")),
    };

    let description = params.description.unwrap_or_default();
    let embed_text = if description.is_empty() {
        params.name.clone()
    } else {
        format!("{} {}", params.name, description)
    };

    let embedding_blob = match embedding.embed_one(&embed_text) {
        Ok(emb) => Some(
            emb.iter()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>(),
        ),
        Err(e) => return err_result(&format!("embedding failed: {e}")),
    };

    match write_channel
        .category_suggest(
            params.name,
            if description.is_empty() {
                None
            } else {
                Some(description)
            },
            params.parent_id,
            memory_id,
            embedding_blob,
        )
        .await
    {
        Ok(id) => ok_json(&serde_json::json!({"category_id": id})),
        Err(e) => err_result(&format!("category_suggest failed: {e}")),
    }
}

pub async fn handle_category_list(db_path: &str, source: Option<String>) -> CallToolResult {
    let source_filter = match source.as_deref() {
        Some(s) => match s.parse::<CategorySource>() {
            Ok(v) => Some(v),
            Err(e) => return err_result(&format!("invalid source: {e}")),
        },
        None => None,
    };

    let db_path = db_path.to_owned();
    match nous_shared::sqlite::spawn_blocking(move || {
        let db = MemoryDb::open(&db_path, None, 384)?;
        db.category_list(source_filter)
    })
    .await
    {
        Ok(trees) => {
            fn tree_to_json(tree: &nous_core::types::CategoryTree) -> serde_json::Value {
                serde_json::json!({
                    "id": tree.category.id,
                    "name": tree.category.name,
                    "source": tree.category.source.to_string(),
                    "description": tree.category.description,
                    "threshold": tree.category.threshold,
                    "children": tree.children.iter().map(tree_to_json).collect::<Vec<_>>(),
                })
            }
            let list: Vec<serde_json::Value> = trees.iter().map(tree_to_json).collect();
            ok_json(&serde_json::json!({"categories": list}))
        }
        Err(e) => err_result(&format!("category_list failed: {e}")),
    }
}

pub async fn handle_category_add(
    params: MemoryCategoryAddParams,
    db_path: &str,
    embedding: &Arc<dyn EmbeddingBackend>,
) -> CallToolResult {
    let db_path = db_path.to_owned();
    let name = params.name;
    let parent = params.parent;
    let description = params.description;
    let threshold = params.threshold;

    let embed_text = match description.as_deref() {
        Some(d) if !d.is_empty() => format!("{name} {d}"),
        _ => name.clone(),
    };
    let embedding_blob = match embedding.embed_one(&embed_text) {
        Ok(emb) => emb
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect::<Vec<u8>>(),
        Err(e) => return err_result(&format!("embedding failed: {e}")),
    };

    match nous_shared::sqlite::spawn_blocking(move || {
        let db = MemoryDb::open(&db_path, None, 384)?;

        let parent_id = match parent.as_deref() {
            Some(pname) => {
                let id: i64 = db
                    .connection()
                    .query_row(
                        "SELECT id FROM categories WHERE name = ?1 AND parent_id IS NULL",
                        rusqlite::params![pname],
                        |row| row.get(0),
                    )
                    .map_err(|_| {
                        nous_shared::NousError::Internal(format!(
                            "parent category '{pname}' not found"
                        ))
                    })?;
                Some(id)
            }
            None => None,
        };

        let cat_id = db.category_add(
            &name,
            parent_id,
            description.as_deref(),
            CategorySource::User,
        )?;

        if let Some(t) = threshold {
            db.connection().execute(
                "UPDATE categories SET threshold = ?1 WHERE id = ?2",
                rusqlite::params![t as f64, cat_id],
            )?;
        }

        db.connection().execute(
            "UPDATE categories SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![embedding_blob, cat_id],
        )?;

        Ok(serde_json::json!({"category_id": cat_id, "name": name}))
    })
    .await
    {
        Ok(json) => ok_json(&json),
        Err(e) => err_result(&format!("category_add failed: {e}")),
    }
}

pub async fn handle_category_delete(
    params: MemoryCategoryDeleteParams,
    write_channel: &WriteChannel,
) -> CallToolResult {
    match write_channel.category_delete(params.name.clone()).await {
        Ok(()) => ok_json(&serde_json::json!({"status": "deleted", "name": params.name})),
        Err(e) => err_result(&format!("{e}")),
    }
}

pub async fn handle_category_update(
    params: MemoryCategoryUpdateParams,
    write_channel: &WriteChannel,
    read_pool: &ReadPool,
    embedding: &Arc<dyn EmbeddingBackend>,
) -> CallToolResult {
    let final_name = params
        .new_name
        .as_deref()
        .unwrap_or(&params.name)
        .to_owned();

    let embedding_blob = if params.new_name.is_some() || params.description.is_some() {
        let desc_for_embed = if params.description.is_some() {
            params.description.clone()
        } else {
            let cat_name = params.name.clone();
            read_pool
                .with_conn(move |conn| {
                    conn.query_row(
                        "SELECT description FROM categories WHERE name = ?1",
                        rusqlite::params![cat_name],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .map_err(Into::into)
                })
                .await
                .ok()
                .flatten()
        };
        let embed_text = match desc_for_embed.as_deref() {
            Some(d) if !d.is_empty() => format!("{final_name} {d}"),
            _ => final_name.clone(),
        };
        match embedding.embed_one(&embed_text) {
            Ok(emb) => Some(
                emb.iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect::<Vec<u8>>(),
            ),
            Err(e) => return err_result(&format!("embedding failed: {e}")),
        }
    } else {
        None
    };

    match write_channel
        .category_update(
            params.name.clone(),
            params.new_name.clone(),
            params.description.clone(),
            params.threshold,
            embedding_blob,
        )
        .await
    {
        Ok(()) => ok_json(&serde_json::json!({"status": "updated", "name": final_name})),
        Err(e) => err_result(&format!("{e}")),
    }
}

pub async fn handle_workspaces(read_pool: &ReadPool, source: Option<String>) -> CallToolResult {
    match read_pool.with_conn(MemoryDb::workspaces_on).await {
        Ok(workspaces) => {
            let filtered: Vec<_> = match &source {
                Some(s) => {
                    let lower = s.to_lowercase();
                    workspaces
                        .into_iter()
                        .filter(|(w, _)| w.path.to_lowercase().contains(&lower))
                        .collect()
                }
                None => workspaces,
            };
            let list: Vec<serde_json::Value> = filtered
                .into_iter()
                .map(|(w, count)| {
                    serde_json::json!({
                        "id": w.id,
                        "path": w.path,
                        "name": w.name,
                        "memory_count": count,
                        "created_at": w.created_at,
                    })
                })
                .collect();
            ok_json(&serde_json::json!({"workspaces": list}))
        }
        Err(e) => err_result(&format!("workspaces failed: {e}")),
    }
}

pub async fn handle_tags(read_pool: &ReadPool, prefix: Option<String>) -> CallToolResult {
    match read_pool.with_conn(MemoryDb::tags_on).await {
        Ok(tags) => {
            let filtered: Vec<_> = match &prefix {
                Some(p) => tags
                    .into_iter()
                    .filter(|(name, _)| name.starts_with(p.as_str()))
                    .collect(),
                None => tags,
            };
            let list: Vec<serde_json::Value> = filtered
                .into_iter()
                .map(|(name, count)| serde_json::json!({"tag": name, "count": count}))
                .collect();
            ok_json(&serde_json::json!({"tags": list}))
        }
        Err(e) => err_result(&format!("tags failed: {e}")),
    }
}

pub async fn handle_stats(read_pool: &ReadPool) -> CallToolResult {
    match read_pool.with_conn(MemoryDb::stats_on).await {
        Ok(stats) => ok_json(&stats),
        Err(e) => err_result(&format!("stats failed: {e}")),
    }
}

pub async fn handle_schema(read_pool: &ReadPool) -> CallToolResult {
    match read_pool.with_conn(MemoryDb::schema_on).await {
        Ok(schema) => ok_json(&serde_json::json!({"schema": schema})),
        Err(e) => err_result(&format!("schema failed: {e}")),
    }
}

pub async fn handle_sql(params: MemorySqlParams, read_pool: &ReadPool) -> CallToolResult {
    let query = params.query.trim().to_string();

    if !is_read_only_sql(&query) {
        return err_result(
            "only SELECT, EXPLAIN, read-only PRAGMA, and read-only WITH statements are allowed",
        );
    }

    match read_pool
        .with_conn(move |conn| {
            let mut stmt = conn.prepare(&query)?;
            let column_names: Vec<String> =
                stmt.column_names().iter().map(|s| s.to_string()).collect();
            let col_count = column_names.len();

            let rows: Vec<Vec<serde_json::Value>> = stmt
                .query_map([], |row| {
                    let mut values = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let val = match row.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(n) => {
                                serde_json::Value::Number(n.into())
                            }
                            rusqlite::types::ValueRef::Real(f) => serde_json::json!(f),
                            rusqlite::types::ValueRef::Text(t) => {
                                serde_json::Value::String(String::from_utf8_lossy(t).into_owned())
                            }
                            rusqlite::types::ValueRef::Blob(b) => {
                                serde_json::Value::String(format!("<blob:{} bytes>", b.len()))
                            }
                        };
                        values.push(val);
                    }
                    Ok(values)
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let row_count = rows.len();
            Ok(serde_json::json!({
                "columns": column_names,
                "rows": rows,
                "row_count": row_count,
            }))
        })
        .await
    {
        Ok(result) => ok_json(&result),
        Err(e) => err_result(&format!("sql query failed: {e}")),
    }
}

fn is_read_only_sql(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    let trimmed = upper.trim_start();

    let first_keyword = trimmed
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("");

    match first_keyword {
        "SELECT" | "EXPLAIN" => true,
        "WITH" => !contains_write_keyword(&upper),
        "PRAGMA" => is_read_only_pragma(trimmed),
        _ => false,
    }
}

fn contains_write_keyword(upper_sql: &str) -> bool {
    let write_keywords = [
        "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE", "ATTACH", "DETACH", "REPLACE",
        "REINDEX", "VACUUM",
    ];
    let tokens: Vec<&str> = upper_sql
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .collect();
    for kw in &write_keywords {
        if tokens.iter().any(|t| t == kw) {
            return true;
        }
    }
    false
}

fn is_read_only_pragma(trimmed_upper: &str) -> bool {
    let after_pragma = trimmed_upper
        .strip_prefix("PRAGMA")
        .unwrap_or("")
        .trim_start();
    !after_pragma.contains('=')
}

pub async fn handle_search(
    params: MemorySearchParams,
    db_path: &str,
    embedding: &Arc<dyn EmbeddingBackend>,
) -> CallToolResult {
    let mode = match params.mode.as_deref() {
        Some(v) => match parse_enum::<SearchMode>(v, "mode") {
            Ok(v) => v,
            Err(e) => return e,
        },
        None => SearchMode::Hybrid,
    };

    if params.query.trim().is_empty() && matches!(mode, SearchMode::Fts | SearchMode::Hybrid) {
        return err_result("query must not be empty for fts or hybrid mode");
    }

    let memory_type = match params.memory_type.as_deref() {
        Some(v) => match parse_enum::<MemoryType>(v, "memory_type") {
            Ok(v) => Some(v),
            Err(e) => return e,
        },
        None => None,
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

    let query_embedding = match mode {
        SearchMode::Semantic | SearchMode::Hybrid => match embedding.embed_one(&params.query) {
            Ok(v) => v,
            Err(e) => return err_result(&format!("embedding failed: {e}")),
        },
        SearchMode::Fts => vec![],
    };

    let filters = SearchFilters {
        memory_type,
        category_id: params.category_id,
        workspace_id: params.workspace_id,
        trace_id: params.trace_id,
        session_id: params.session_id,
        importance,
        confidence,
        tags: params.tags,
        archived: params.archived,
        since: params.since,
        until: params.until,
        valid_only: params.valid_only,
        limit: params.limit,
    };

    let db_path = db_path.to_owned();
    let query = params.query;
    let results = match nous_shared::sqlite::spawn_blocking(move || {
        let db = MemoryDb::open(&db_path, None, 384)?;
        db.search(&query, &query_embedding, &filters, mode)
    })
    .await
    {
        Ok(r) => r,
        Err(e) => return err_result(&format!("search failed: {e}")),
    };

    let results_json: Vec<serde_json::Value> = results
        .iter()
        .map(|r| {
            serde_json::json!({
                "memory": {
                    "id": r.memory.id,
                    "title": r.memory.title,
                    "content": r.memory.content,
                    "memory_type": r.memory.memory_type,
                    "importance": r.memory.importance,
                    "confidence": r.memory.confidence,
                    "workspace_id": r.memory.workspace_id,
                    "session_id": r.memory.session_id,
                    "trace_id": r.memory.trace_id,
                    "archived": r.memory.archived,
                    "created_at": r.memory.created_at,
                },
                "tags": r.tags,
                "rank": r.rank,
            })
        })
        .collect();

    ok_json(&serde_json::json!({
        "results": results_json,
        "count": results_json.len(),
    }))
}

pub async fn handle_context(params: MemoryContextParams, db_path: &str) -> CallToolResult {
    let workspace_path = params.workspace_path;
    let summary = params.summary;

    let db_path = db_path.to_owned();
    let entries = match nous_shared::sqlite::spawn_blocking(move || {
        let db = MemoryDb::open(&db_path, None, 384)?;

        let ws_id: i64 = match db.connection().query_row(
            "SELECT id FROM workspaces WHERE path = ?1",
            rusqlite::params![workspace_path],
            |row| row.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };

        db.context(ws_id, summary)
    })
    .await
    {
        Ok(e) => e,
        Err(e) => return err_result(&format!("context failed: {e}")),
    };

    let entries_json: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "title": e.title,
                "content": e.content,
                "memory_type": e.memory_type,
                "importance": e.importance,
                "created_at": e.created_at,
            })
        })
        .collect();

    ok_json(&serde_json::json!({
        "entries": entries_json,
        "count": entries_json.len(),
    }))
}

fn span_to_json(s: &nous_otlp::decode::Span) -> serde_json::Value {
    serde_json::json!({
        "trace_id": s.trace_id,
        "span_id": s.span_id,
        "parent_span_id": s.parent_span_id,
        "name": s.name,
        "kind": s.kind,
        "start_time": s.start_time,
        "end_time": s.end_time,
        "status_code": s.status_code,
        "status_message": s.status_message,
        "resource_attrs": s.resource_attrs,
        "span_attrs": s.span_attrs,
        "events_json": s.events_json,
    })
}

fn log_to_json(l: &nous_otlp::decode::LogEvent) -> serde_json::Value {
    serde_json::json!({
        "timestamp": l.timestamp,
        "severity": l.severity,
        "body": l.body,
        "resource_attrs": l.resource_attrs,
        "scope_attrs": l.scope_attrs,
        "log_attrs": l.log_attrs,
        "session_id": l.session_id,
        "trace_id": l.trace_id,
        "span_id": l.span_id,
    })
}

pub async fn handle_otlp_trace_context(
    params: OtlpTraceContextParams,
    otlp_db_path: &str,
    read_pool: &ReadPool,
) -> CallToolResult {
    if otlp_db_path.is_empty() {
        return err_result("OTLP database not configured");
    }

    let trace_id = params.trace_id.clone();
    let session_id = params.session_id.clone();

    let tid = params.trace_id;
    let memories = match read_pool
        .with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, title, content, memory_type, session_id, trace_id, created_at
                 FROM memories WHERE trace_id = ?1 ORDER BY created_at DESC",
            )?;
            let rows = stmt.query_map(rusqlite::params![tid], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1)?,
                    "content": row.get::<_, String>(2)?,
                    "memory_type": row.get::<_, String>(3)?,
                    "session_id": row.get::<_, Option<String>>(4)?,
                    "trace_id": row.get::<_, Option<String>>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                }))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(Into::into)
        })
        .await
    {
        Ok(r) => r,
        Err(e) => return err_result(&format!("memory query failed: {e}")),
    };

    let otlp_path = otlp_db_path.to_owned();
    let trace_id_clone = trace_id.clone();
    let session_id_clone = session_id.clone();
    let (spans, logs) = match nous_shared::sqlite::spawn_blocking(move || {
        let db = OtlpDb::open(&otlp_path, None)?;
        let spans = db.query_spans(&trace_id_clone, None, None)?;
        let logs = match &session_id_clone {
            Some(sid) => db.query_logs(sid, None, None)?,
            None => vec![],
        };
        Ok((spans, logs))
    })
    .await
    {
        Ok((s, l)) => (s, l),
        Err(e) => return err_result(&format!("OTLP query failed: {e}")),
    };

    let spans_json: Vec<serde_json::Value> = spans.iter().map(span_to_json).collect();
    let logs_json: Vec<serde_json::Value> = logs.iter().map(log_to_json).collect();

    ok_json(&serde_json::json!({
        "memories": memories,
        "spans": spans_json,
        "logs": logs_json,
    }))
}

pub async fn handle_otlp_memory_context(
    params: OtlpMemoryContextParams,
    otlp_db_path: &str,
    read_pool: &ReadPool,
) -> CallToolResult {
    if otlp_db_path.is_empty() {
        return err_result("OTLP database not configured");
    }

    let id = match params.memory_id.parse::<MemoryId>() {
        Ok(v) => v,
        Err(e) => return err_result(&format!("invalid memory_id: {e}")),
    };

    let memory = match read_pool
        .with_conn({
            let id = id.clone();
            move |conn| MemoryDb::recall_on(conn, &id)
        })
        .await
    {
        Ok(Some(m)) => m,
        Ok(None) => return err_result("memory not found"),
        Err(e) => return err_result(&format!("recall failed: {e}")),
    };

    let trace_id = memory.memory.trace_id.clone();
    let session_id = memory.memory.session_id.clone();

    if trace_id.is_none() && session_id.is_none() {
        return err_result("memory has no trace_id or session_id for OTLP correlation");
    }

    let otlp_path = otlp_db_path.to_owned();
    let (spans, logs) = match nous_shared::sqlite::spawn_blocking(move || {
        let db = OtlpDb::open(&otlp_path, None)?;
        let spans = match &trace_id {
            Some(tid) => db.query_spans(tid, None, None)?,
            None => vec![],
        };
        let logs = match &session_id {
            Some(sid) => db.query_logs(sid, None, None)?,
            None => vec![],
        };
        Ok((spans, logs))
    })
    .await
    {
        Ok((s, l)) => (s, l),
        Err(e) => return err_result(&format!("OTLP query failed: {e}")),
    };

    let spans_json: Vec<serde_json::Value> = spans.iter().map(span_to_json).collect();
    let logs_json: Vec<serde_json::Value> = logs.iter().map(log_to_json).collect();

    ok_json(&serde_json::json!({
        "memory": recall_to_json(&memory),
        "spans": spans_json,
        "logs": logs_json,
    }))
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.contains('-')
}

async fn resolve_room_id(id_or_name: &str, read_pool: &ReadPool) -> Result<String, CallToolResult> {
    if looks_like_uuid(id_or_name)
        && let Ok(Some(room)) = read_pool.get_room(id_or_name).await
    {
        return Ok(room.id);
    }
    match read_pool.get_room_by_name(id_or_name).await {
        Ok(Some(room)) => Ok(room.id),
        Ok(None) => Err(err_result(&format!("room not found: {id_or_name}"))),
        Err(e) => Err(err_result(&format!("room lookup failed: {e}"))),
    }
}

pub async fn handle_room_create(
    params: RoomCreateParams,
    write_channel: &WriteChannel,
) -> CallToolResult {
    let id = MemoryId::new().to_string();
    match write_channel
        .create_room(
            id.clone(),
            params.name.clone(),
            params.purpose,
            params.metadata,
        )
        .await
    {
        Ok(_) => ok_json(&serde_json::json!({
            "id": id,
            "name": params.name,
        })),
        Err(e) => err_result(&format!("room_create failed: {e}")),
    }
}

pub async fn handle_room_list(params: RoomListParams, read_pool: &ReadPool) -> CallToolResult {
    match read_pool.list_rooms(params.archived, params.limit).await {
        Ok(rooms) => {
            let list: Vec<serde_json::Value> = rooms.iter().map(|r| serde_json::json!(r)).collect();
            ok_json(&serde_json::json!({ "rooms": list }))
        }
        Err(e) => err_result(&format!("room_list failed: {e}")),
    }
}

pub async fn handle_room_get(params: RoomGetParams, read_pool: &ReadPool) -> CallToolResult {
    if looks_like_uuid(&params.id)
        && let Ok(Some(room)) = read_pool.get_room(&params.id).await
    {
        return ok_json(&serde_json::json!(room));
    }
    match read_pool.get_room_by_name(&params.id).await {
        Ok(Some(room)) => ok_json(&serde_json::json!(room)),
        Ok(None) => err_result(&format!("room not found: {}", params.id)),
        Err(e) => err_result(&format!("room_get failed: {e}")),
    }
}

pub async fn handle_room_delete(
    params: RoomDeleteParams,
    write_channel: &WriteChannel,
) -> CallToolResult {
    match write_channel.delete_room(params.id, params.hard).await {
        Ok(true) => {
            if params.hard {
                ok_json(&serde_json::json!({"success": true, "deleted": true}))
            } else {
                ok_json(&serde_json::json!({"success": true, "archived": true}))
            }
        }
        Ok(false) => err_result("room not found"),
        Err(e) => err_result(&format!("room_delete failed: {e}")),
    }
}

pub async fn handle_room_post_message(
    params: RoomPostMessageParams,
    write_channel: &WriteChannel,
    read_pool: &ReadPool,
) -> CallToolResult {
    let room_id = match resolve_room_id(&params.room_id, read_pool).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    let msg_id = MemoryId::new().to_string();
    let sender_id = params.sender_id.unwrap_or_else(|| "system".to_string());
    match write_channel
        .post_message(
            msg_id.clone(),
            room_id.clone(),
            sender_id.clone(),
            params.content,
            params.reply_to,
            params.metadata,
        )
        .await
    {
        Ok(_) => ok_json(&serde_json::json!({
            "id": msg_id,
            "room_id": room_id,
            "sender_id": sender_id,
        })),
        Err(e) => err_result(&format!("room_post_message failed: {e}")),
    }
}

pub async fn handle_room_read_messages(
    params: RoomReadMessagesParams,
    read_pool: &ReadPool,
) -> CallToolResult {
    let room_id = match resolve_room_id(&params.room_id, read_pool).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    match read_pool
        .list_messages(&room_id, params.limit, params.before, params.since)
        .await
    {
        Ok(messages) => {
            let list: Vec<serde_json::Value> =
                messages.iter().map(|m| serde_json::json!(m)).collect();
            ok_json(&serde_json::json!({ "messages": list }))
        }
        Err(e) => err_result(&format!("room_read_messages failed: {e}")),
    }
}

pub async fn handle_room_search(params: RoomSearchParams, read_pool: &ReadPool) -> CallToolResult {
    let room_id = match resolve_room_id(&params.room_id, read_pool).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    match read_pool
        .search_messages(&room_id, &params.query, params.limit)
        .await
    {
        Ok(messages) => {
            let list: Vec<serde_json::Value> =
                messages.iter().map(|m| serde_json::json!(m)).collect();
            ok_json(&serde_json::json!({ "messages": list }))
        }
        Err(e) => err_result(&format!("room_search failed: {e}")),
    }
}

pub async fn handle_room_info(params: RoomInfoParams, read_pool: &ReadPool) -> CallToolResult {
    let room_id = match resolve_room_id(&params.id, read_pool).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    match read_pool.room_info(&room_id).await {
        Ok(Some(info)) => ok_json(&info),
        Ok(None) => err_result(&format!("room not found: {}", params.id)),
        Err(e) => err_result(&format!("room_info failed: {e}")),
    }
}

pub async fn handle_room_join(
    params: RoomJoinParams,
    write_channel: &WriteChannel,
    read_pool: &ReadPool,
) -> CallToolResult {
    let room_id = match resolve_room_id(&params.room_id, read_pool).await {
        Ok(id) => id,
        Err(e) => return e,
    };
    let role = params.role.unwrap_or_else(|| "member".to_string());
    match &*role {
        "owner" | "member" | "observer" => {}
        _ => {
            return err_result(&format!(
                "invalid role: {role}. Must be owner, member, or observer"
            ));
        }
    }
    match write_channel
        .join_room(room_id.clone(), params.agent_id.clone(), role.clone())
        .await
    {
        Ok(()) => ok_json(&serde_json::json!({
            "room_id": room_id,
            "agent_id": params.agent_id,
            "role": role,
        })),
        Err(e) => err_result(&format!("room_join failed: {e}")),
    }
}

// ── Schedule tool parameter structs ──

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleCreateParams {
    pub name: String,
    pub cron_expr: String,
    pub action_type: String,
    pub action_payload: String,
    pub timezone: Option<String>,
    pub max_retries: Option<i64>,
    pub timeout_secs: Option<i64>,
    pub desired_outcome: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleListParams {
    pub enabled: Option<bool>,
    pub action_type: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleGetParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleUpdateParams {
    pub id: String,
    pub name: Option<String>,
    pub cron_expr: Option<String>,
    pub action_payload: Option<String>,
    pub enabled: Option<bool>,
    pub max_retries: Option<i64>,
    pub timeout_secs: Option<i64>,
    pub desired_outcome: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleDeleteParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SchedulePauseParams {
    pub id: String,
    pub duration_secs: Option<u64>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleResumeParams {
    pub id: String,
}

// ── Schedule tool handlers ──

fn format_epoch(epoch: Option<i64>) -> serde_json::Value {
    match epoch {
        Some(ts) => serde_json::Value::Number(ts.into()),
        None => serde_json::Value::Null,
    }
}

fn schedule_to_json(s: &Schedule) -> serde_json::Value {
    serde_json::json!({
        "id": s.id,
        "name": s.name,
        "cron_expr": s.cron_expr,
        "timezone": s.timezone,
        "enabled": s.enabled,
        "action_type": s.action_type.to_string(),
        "action_payload": s.action_payload,
        "desired_outcome": s.desired_outcome,
        "max_retries": s.max_retries,
        "timeout_secs": s.timeout_secs,
        "max_output_bytes": s.max_output_bytes,
        "max_runs": s.max_runs,
        "next_run_at": format_epoch(s.next_run_at),
        "created_at": format_epoch(Some(s.created_at)),
        "updated_at": format_epoch(Some(s.updated_at)),
    })
}

pub async fn handle_schedule_create(
    params: ScheduleCreateParams,
    write_channel: &WriteChannel,
    scheduler_notify: &Arc<Notify>,
) -> CallToolResult {
    if let Err(e) = CronExpr::parse(&params.cron_expr) {
        return err_result(&format!("invalid cron expression: {e}"));
    }

    let action_type = match parse_enum::<ActionType>(&params.action_type, "action_type") {
        Ok(v) => v,
        Err(e) => return e,
    };

    let schedule = Schedule {
        id: String::new(),
        name: params.name,
        cron_expr: params.cron_expr,
        timezone: params.timezone.unwrap_or_else(|| "UTC".to_string()),
        enabled: true,
        action_type,
        action_payload: params.action_payload,
        desired_outcome: params.desired_outcome,
        max_retries: params.max_retries.unwrap_or(3),
        timeout_secs: params.timeout_secs,
        max_output_bytes: 65536,
        max_runs: 100,
        next_run_at: None,
        created_at: 0,
        updated_at: 0,
    };

    let id = match write_channel.create_schedule(schedule).await {
        Ok(id) => id,
        Err(e) => return err_result(&format!("schedule_create failed: {e}")),
    };

    scheduler_notify.notify_one();

    ok_json(&serde_json::json!({
        "id": id,
        "status": "created",
    }))
}

pub async fn handle_schedule_list(
    params: ScheduleListParams,
    read_pool: &ReadPool,
) -> CallToolResult {
    let enabled_filter = params.enabled;
    let action_type_filter = match params.action_type.as_deref() {
        Some(v) => match parse_enum::<ActionType>(v, "action_type") {
            Ok(v) => Some(v),
            Err(e) => return e,
        },
        None => None,
    };
    let limit = params.limit;

    match read_pool
        .with_conn(move |conn| {
            ScheduleDb::list(conn, enabled_filter, action_type_filter.as_ref(), limit)
        })
        .await
    {
        Ok(schedules) => {
            let list: Vec<serde_json::Value> = schedules.iter().map(schedule_to_json).collect();
            ok_json(&serde_json::json!({
                "schedules": list,
                "count": list.len(),
            }))
        }
        Err(e) => err_result(&format!("schedule_list failed: {e}")),
    }
}

pub async fn handle_schedule_get(
    params: ScheduleGetParams,
    read_pool: &ReadPool,
) -> CallToolResult {
    let id = params.id;
    let id_clone = id.clone();

    let schedule = match read_pool
        .with_conn(move |conn| ScheduleDb::get(conn, &id_clone))
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return err_result(&format!("schedule not found: {id}")),
        Err(e) => return err_result(&format!("schedule_get failed: {e}")),
    };

    let schedule_id = id.clone();
    let runs = match read_pool
        .with_conn(move |conn| ScheduleDb::get_runs(conn, &schedule_id, None, Some(10)))
        .await
    {
        Ok(r) => r,
        Err(e) => return err_result(&format!("get_runs failed: {e}")),
    };

    let runs_json: Vec<serde_json::Value> = runs
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "started_at": format_epoch(Some(r.started_at)),
                "finished_at": format_epoch(r.finished_at),
                "status": r.status.to_string(),
                "exit_code": r.exit_code,
                "output": r.output,
                "error": r.error,
                "attempt": r.attempt,
                "duration_ms": r.duration_ms,
            })
        })
        .collect();

    let mut json = schedule_to_json(&schedule);
    json.as_object_mut().unwrap().insert(
        "recent_runs".to_string(),
        serde_json::Value::Array(runs_json),
    );

    ok_json(&json)
}

pub async fn handle_schedule_update(
    params: ScheduleUpdateParams,
    write_channel: &WriteChannel,
    scheduler_notify: &Arc<Notify>,
) -> CallToolResult {
    if let Some(ref expr) = params.cron_expr
        && let Err(e) = CronExpr::parse(expr)
    {
        return err_result(&format!("invalid cron expression: {e}"));
    }

    let cron_changed = params.cron_expr.is_some();

    let patch = SchedulePatch {
        name: params.name,
        cron_expr: params.cron_expr,
        action_payload: params.action_payload,
        enabled: params.enabled,
        max_retries: params.max_retries,
        timeout_secs: params.timeout_secs,
        desired_outcome: params.desired_outcome,
    };

    let id = params.id;
    match write_channel.update_schedule(id.clone(), patch).await {
        Ok(true) => {}
        Ok(false) => return err_result(&format!("schedule not found: {id}")),
        Err(e) => return err_result(&format!("schedule_update failed: {e}")),
    }

    if cron_changed {
        let _ = write_channel.compute_next_run(id.clone()).await;
    }

    scheduler_notify.notify_one();

    ok_json(&serde_json::json!({
        "id": id,
        "status": "updated",
    }))
}

pub async fn handle_schedule_delete(
    params: ScheduleDeleteParams,
    write_channel: &WriteChannel,
    scheduler_notify: &Arc<Notify>,
) -> CallToolResult {
    match write_channel.delete_schedule(params.id.clone()).await {
        Ok(true) => {
            scheduler_notify.notify_one();
            ok_json(&serde_json::json!({
                "id": params.id,
                "success": true,
            }))
        }
        Ok(false) => err_result(&format!("schedule not found: {}", params.id)),
        Err(e) => err_result(&format!("schedule_delete failed: {e}")),
    }
}

pub async fn handle_schedule_pause(
    params: SchedulePauseParams,
    write_channel: &WriteChannel,
    scheduler_notify: &Arc<Notify>,
) -> CallToolResult {
    let patch = SchedulePatch {
        enabled: Some(false),
        ..Default::default()
    };

    match write_channel
        .update_schedule(params.id.clone(), patch)
        .await
    {
        Ok(true) => {
            scheduler_notify.notify_one();

            if let Some(duration) = params.duration_secs {
                let wc = write_channel.clone();
                let id = params.id.clone();
                let notify = scheduler_notify.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(duration)).await;
                    let resume_patch = SchedulePatch {
                        enabled: Some(true),
                        ..Default::default()
                    };
                    let _ = wc.update_schedule(id.clone(), resume_patch).await;
                    let _ = wc.compute_next_run(id).await;
                    notify.notify_one();
                });
            }

            ok_json(&serde_json::json!({
                "id": params.id,
                "status": "paused",
                "reason": params.reason,
            }))
        }
        Ok(false) => err_result(&format!("schedule not found: {}", params.id)),
        Err(e) => err_result(&format!("schedule_pause failed: {e}")),
    }
}

pub async fn handle_schedule_resume(
    params: ScheduleResumeParams,
    write_channel: &WriteChannel,
    scheduler_notify: &Arc<Notify>,
) -> CallToolResult {
    let patch = SchedulePatch {
        enabled: Some(true),
        ..Default::default()
    };

    match write_channel
        .update_schedule(params.id.clone(), patch)
        .await
    {
        Ok(true) => {
            let _ = write_channel.compute_next_run(params.id.clone()).await;
            scheduler_notify.notify_one();
            ok_json(&serde_json::json!({
                "id": params.id,
                "status": "resumed",
            }))
        }
        Ok(false) => err_result(&format!("schedule not found: {}", params.id)),
        Err(e) => err_result(&format!("schedule_resume failed: {e}")),
    }
}
