use std::str::FromStr;
use std::sync::Arc;

use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::chunk::Chunker;
use nous_core::classify::CategoryClassifier;
use nous_core::db::MemoryDb;
use nous_core::embed::EmbeddingBackend;
use nous_core::types::{
    Confidence, Importance, MemoryPatch, MemoryType, MemoryWithRelations, NewMemory, RelationType,
    SearchFilters, SearchMode,
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

pub async fn handle_workspaces(read_pool: &ReadPool) -> CallToolResult {
    match read_pool.with_conn(MemoryDb::workspaces_on).await {
        Ok(workspaces) => {
            let list: Vec<serde_json::Value> = workspaces
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

pub async fn handle_tags(read_pool: &ReadPool) -> CallToolResult {
    match read_pool.with_conn(MemoryDb::tags_on).await {
        Ok(tags) => {
            let list: Vec<serde_json::Value> = tags
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
        let db = MemoryDb::open(&db_path, None)?;
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
        let db = MemoryDb::open(&db_path, None)?;

        let ws_id: Option<i64> = db
            .connection()
            .query_row(
                "SELECT id FROM workspaces WHERE path = ?1",
                rusqlite::params![workspace_path],
                |row| row.get(0),
            )
            .ok();

        let Some(ws_id) = ws_id else {
            return Ok(vec![]);
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
