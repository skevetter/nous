use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InventoryType {
    Worktree,
    Room,
    Schedule,
    Branch,
    File,
    DockerImage,
    Binary,
}

impl InventoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Room => "room",
            Self::Schedule => "schedule",
            Self::Branch => "branch",
            Self::File => "file",
            Self::DockerImage => "docker-image",
            Self::Binary => "binary",
        }
    }
}

impl std::fmt::Display for InventoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for InventoryType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "worktree" => Ok(Self::Worktree),
            "room" => Ok(Self::Room),
            "schedule" => Ok(Self::Schedule),
            "branch" => Ok(Self::Branch),
            "file" => Ok(Self::File),
            "docker-image" => Ok(Self::DockerImage),
            "binary" => Ok(Self::Binary),
            other => Err(NousError::Validation(format!(
                "invalid inventory type: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InventoryStatus {
    Active,
    Archived,
    Deleted,
}

impl InventoryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }
}

impl std::fmt::Display for InventoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for InventoryStatus {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            "deleted" => Ok(Self::Deleted),
            other => Err(NousError::Validation(format!(
                "invalid inventory status: '{other}'"
            ))),
        }
    }
}

// --- Domain objects ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryItem {
    pub id: String,
    pub name: String,
    pub artifact_type: String,
    pub owner_agent_id: Option<String>,
    pub namespace: String,
    pub path: Option<String>,
    pub status: String,
    pub metadata: Option<String>,
    pub tags: String,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

impl InventoryItem {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            artifact_type: row.try_get("artifact_type")?,
            owner_agent_id: row.try_get("owner_agent_id")?,
            namespace: row.try_get("namespace")?,
            path: row.try_get("path")?,
            status: row.try_get("status")?,
            metadata: row.try_get("metadata")?,
            tags: row.try_get("tags")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            archived_at: row.try_get("archived_at")?,
        })
    }

    pub fn tags_vec(&self) -> Vec<String> {
        serde_json::from_str(&self.tags).unwrap_or_default()
    }
}

// --- Request types ---

#[derive(Debug, Clone)]
pub struct RegisterItemRequest {
    pub name: String,
    pub artifact_type: InventoryType,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct UpdateItemRequest {
    pub id: String,
    pub name: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct ListItemsFilter {
    pub artifact_type: Option<InventoryType>,
    pub status: Option<InventoryStatus>,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub orphaned: Option<bool>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SearchItemsRequest {
    pub tags: Vec<String>,
    pub artifact_type: Option<InventoryType>,
    pub status: Option<InventoryStatus>,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}

// --- Operations ---

pub async fn register_item(
    pool: &SqlitePool,
    req: RegisterItemRequest,
) -> Result<InventoryItem, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation(
            "inventory item name cannot be empty".into(),
        ));
    }

    let namespace = req.namespace.unwrap_or_else(|| "default".to_string());

    if let Some(ref agent_id) = req.owner_agent_id {
        let row = sqlx::query("SELECT namespace FROM agents WHERE id = ?")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;
        let row =
            row.ok_or_else(|| NousError::NotFound(format!("owner agent '{agent_id}' not found")))?;
        let agent_ns: String = row.try_get("namespace").map_err(NousError::Sqlite)?;
        if agent_ns != namespace {
            return Err(NousError::Validation(
                "item namespace must match owning agent's namespace".into(),
            ));
        }
    }

    let id = Uuid::now_v7().to_string();
    let tags_json = serde_json::to_string(
        &req.tags
            .unwrap_or_default()
            .iter()
            .map(|t| t.to_lowercase())
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".to_string());

    if let Some(ref metadata) = req.metadata {
        serde_json::from_str::<serde_json::Value>(metadata)
            .map_err(|e| NousError::Validation(format!("metadata must be valid JSON: {e}")))?;
    }

    sqlx::query(
        "INSERT INTO inventory (id, name, artifact_type, owner_agent_id, namespace, path, metadata, tags) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(req.name.trim())
    .bind(req.artifact_type.as_str())
    .bind(&req.owner_agent_id)
    .bind(&namespace)
    .bind(&req.path)
    .bind(&req.metadata)
    .bind(&tags_json)
    .execute(pool)
    .await?;

    get_item_by_id(pool, &id).await
}

pub async fn get_item_by_id(pool: &SqlitePool, id: &str) -> Result<InventoryItem, NousError> {
    let row = sqlx::query("SELECT * FROM inventory WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("inventory item '{id}' not found")))?;
    InventoryItem::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_items(
    pool: &SqlitePool,
    filter: &ListItemsFilter,
) -> Result<Vec<InventoryItem>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut sql = String::from("SELECT * FROM inventory");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref t) = filter.artifact_type {
        conditions.push("artifact_type = ?".to_string());
        binds.push(t.as_str().to_string());
    }

    if let Some(ref s) = filter.status {
        conditions.push("status = ?".to_string());
        binds.push(s.as_str().to_string());
    }

    if let Some(ref agent_id) = filter.owner_agent_id {
        conditions.push("owner_agent_id = ?".to_string());
        binds.push(agent_id.clone());
    }

    if let Some(ref ns) = filter.namespace {
        conditions.push("namespace = ?".to_string());
        binds.push(ns.clone());
    }

    if filter.orphaned == Some(true) {
        conditions.push("owner_agent_id IS NULL".to_string());
    }

    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    binds.push(limit.to_string());
    binds.push(offset.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    rows.iter()
        .map(InventoryItem::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn update_item(
    pool: &SqlitePool,
    req: UpdateItemRequest,
) -> Result<InventoryItem, NousError> {
    let existing = get_item_by_id(pool, &req.id).await?;

    if existing.status == "deleted" {
        return Err(NousError::Validation(
            "cannot update a deleted inventory item".into(),
        ));
    }

    let mut sets: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref name) = req.name {
        if name.trim().is_empty() {
            return Err(NousError::Validation("name cannot be empty".into()));
        }
        sets.push("name = ?".to_string());
        binds.push(name.trim().to_string());
    }

    if let Some(ref path) = req.path {
        sets.push("path = ?".to_string());
        binds.push(path.clone());
    }

    if let Some(ref metadata) = req.metadata {
        serde_json::from_str::<serde_json::Value>(metadata)
            .map_err(|e| NousError::Validation(format!("metadata must be valid JSON: {e}")))?;
        sets.push("metadata = ?".to_string());
        binds.push(metadata.clone());
    }

    if let Some(ref tags) = req.tags {
        let tags_json =
            serde_json::to_string(&tags.iter().map(|t| t.to_lowercase()).collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_string());
        sets.push("tags = ?".to_string());
        binds.push(tags_json);
    }

    if sets.is_empty() {
        return Ok(existing);
    }

    let sql = format!("UPDATE inventory SET {} WHERE id = ?", sets.join(", "));
    binds.push(req.id.clone());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    query.execute(pool).await?;

    get_item_by_id(pool, &req.id).await
}

pub async fn archive_item(pool: &SqlitePool, id: &str) -> Result<InventoryItem, NousError> {
    let existing = get_item_by_id(pool, id).await?;

    if existing.status != "active" {
        return Err(NousError::Validation(format!(
            "can only archive active items, current status: '{}'",
            existing.status
        )));
    }

    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    sqlx::query("UPDATE inventory SET status = 'archived', archived_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;

    get_item_by_id(pool, id).await
}

pub async fn deregister_item(pool: &SqlitePool, id: &str, hard: bool) -> Result<(), NousError> {
    let existing = get_item_by_id(pool, id).await?;

    if hard {
        sqlx::query("DELETE FROM inventory WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    } else {
        if existing.status == "deleted" {
            return Err(NousError::Validation("item is already deleted".into()));
        }
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        sqlx::query(
            "UPDATE inventory SET status = 'deleted', archived_at = COALESCE(archived_at, ?) WHERE id = ?",
        )
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn search_by_tags(
    pool: &SqlitePool,
    req: &SearchItemsRequest,
) -> Result<Vec<InventoryItem>, NousError> {
    if req.tags.is_empty() {
        return Err(NousError::Validation(
            "at least one tag is required for search".into(),
        ));
    }

    let limit = req.limit.unwrap_or(50).min(200);

    let mut sql = String::from("SELECT * FROM inventory WHERE ");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    for tag in &req.tags {
        conditions.push("EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)".to_string());
        binds.push(tag.to_lowercase());
    }

    if let Some(ref t) = req.artifact_type {
        conditions.push("artifact_type = ?".to_string());
        binds.push(t.as_str().to_string());
    }

    if let Some(ref s) = req.status {
        conditions.push("status = ?".to_string());
        binds.push(s.as_str().to_string());
    }

    if let Some(ref ns) = req.namespace {
        conditions.push("namespace = ?".to_string());
        binds.push(ns.clone());
    }

    sql.push_str(&conditions.join(" AND "));
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    binds.push(limit.to_string());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    rows.iter()
        .map(InventoryItem::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn search_fts(
    pool: &SqlitePool,
    query_str: &str,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<InventoryItem>, NousError> {
    if query_str.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let rows = if let Some(ns) = namespace {
        sqlx::query(
            "SELECT i.* FROM inventory i \
             INNER JOIN inventory_fts f ON f.rowid = i.rowid \
             WHERE inventory_fts MATCH ? AND i.namespace = ? \
             LIMIT ?",
        )
        .bind(query_str)
        .bind(ns)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT i.* FROM inventory i \
             INNER JOIN inventory_fts f ON f.rowid = i.rowid \
             WHERE inventory_fts MATCH ? \
             LIMIT ?",
        )
        .bind(query_str)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    rows.iter()
        .map(InventoryItem::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn transfer_ownership(
    pool: &SqlitePool,
    from_agent_id: &str,
    to_agent_id: Option<&str>,
) -> Result<u64, NousError> {
    let result = if let Some(target) = to_agent_id {
        let _target_agent = sqlx::query("SELECT id FROM agents WHERE id = ?")
            .bind(target)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| NousError::NotFound(format!("target agent '{target}' not found")))?;

        sqlx::query(
            "UPDATE inventory SET owner_agent_id = ? \
             WHERE owner_agent_id = ? AND status = 'active'",
        )
        .bind(target)
        .bind(from_agent_id)
        .execute(pool)
        .await?
    } else {
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        sqlx::query(
            "UPDATE inventory SET owner_agent_id = NULL, status = 'archived', archived_at = ? \
             WHERE owner_agent_id = ? AND status = 'active'",
        )
        .bind(&now)
        .bind(from_agent_id)
        .execute(pool)
        .await?
    };

    Ok(result.rows_affected())
}
