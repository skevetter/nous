use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceType {
    Worktree,
    Room,
    Schedule,
    Branch,
    File,
    DockerImage,
    Binary,
}

impl ResourceType {
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

    pub fn default_ownership_policy(&self) -> OwnershipPolicy {
        match self {
            Self::Worktree | Self::Branch => OwnershipPolicy::CascadeDelete,
            _ => OwnershipPolicy::Orphan,
        }
    }
}

impl std::fmt::Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ResourceType {
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
                "invalid resource type: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceStatus {
    Active,
    Archived,
    Deleted,
}

impl ResourceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }
}

impl std::fmt::Display for ResourceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ResourceStatus {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            "deleted" => Ok(Self::Deleted),
            other => Err(NousError::Validation(format!(
                "invalid resource status: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnershipPolicy {
    CascadeDelete,
    Orphan,
    TransferToParent,
}

impl OwnershipPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CascadeDelete => "cascade-delete",
            Self::Orphan => "orphan",
            Self::TransferToParent => "transfer-to-parent",
        }
    }
}

impl std::fmt::Display for OwnershipPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OwnershipPolicy {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "cascade-delete" => Ok(Self::CascadeDelete),
            "orphan" => Ok(Self::Orphan),
            "transfer-to-parent" => Ok(Self::TransferToParent),
            other => Err(NousError::Validation(format!(
                "invalid ownership policy: '{other}'"
            ))),
        }
    }
}

// --- Domain objects ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub id: String,
    pub name: String,
    pub resource_type: String,
    pub owner_agent_id: Option<String>,
    pub namespace: String,
    pub path: Option<String>,
    pub status: String,
    pub metadata: Option<String>,
    pub tags: String,
    pub ownership_policy: String,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

impl Resource {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            resource_type: row.try_get("resource_type")?,
            owner_agent_id: row.try_get("owner_agent_id")?,
            namespace: row.try_get("namespace")?,
            path: row.try_get("path")?,
            status: row.try_get("status")?,
            metadata: row.try_get("metadata")?,
            tags: row.try_get("tags")?,
            ownership_policy: row.try_get("ownership_policy")?,
            last_seen_at: row.try_get("last_seen_at")?,
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
pub struct RegisterResourceRequest {
    pub name: String,
    pub resource_type: ResourceType,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
    pub ownership_policy: Option<OwnershipPolicy>,
}

#[derive(Debug, Clone)]
pub struct UpdateResourceRequest {
    pub id: String,
    pub name: Option<String>,
    pub path: Option<String>,
    pub metadata: Option<String>,
    pub tags: Option<Vec<String>>,
    pub status: Option<ResourceStatus>,
    pub ownership_policy: Option<OwnershipPolicy>,
}

#[derive(Debug, Clone, Default)]
pub struct ListResourcesFilter {
    pub resource_type: Option<ResourceType>,
    pub status: Option<ResourceStatus>,
    pub owner_agent_id: Option<String>,
    pub namespace: Option<String>,
    pub orphaned: Option<bool>,
    pub ownership_policy: Option<OwnershipPolicy>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct SearchResourcesRequest {
    pub tags: Vec<String>,
    pub resource_type: Option<ResourceType>,
    pub status: Option<ResourceStatus>,
    pub namespace: Option<String>,
    pub limit: Option<u32>,
}

// --- Operations ---

pub async fn register_resource(
    pool: &SqlitePool,
    req: RegisterResourceRequest,
) -> Result<Resource, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation(
            "resource name cannot be empty".into(),
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
                "resource namespace must match owning agent's namespace".into(),
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

    let policy = req
        .ownership_policy
        .unwrap_or_else(|| req.resource_type.default_ownership_policy());

    sqlx::query(
        "INSERT INTO resources (id, name, resource_type, owner_agent_id, namespace, path, metadata, tags, ownership_policy) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(req.name.trim())
    .bind(req.resource_type.as_str())
    .bind(&req.owner_agent_id)
    .bind(&namespace)
    .bind(&req.path)
    .bind(&req.metadata)
    .bind(&tags_json)
    .bind(policy.as_str())
    .execute(pool)
    .await?;

    get_resource_by_id(pool, &id).await
}

pub async fn get_resource_by_id(pool: &SqlitePool, id: &str) -> Result<Resource, NousError> {
    let row = sqlx::query("SELECT * FROM resources WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("resource '{id}' not found")))?;
    Resource::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_resources(
    pool: &SqlitePool,
    filter: &ListResourcesFilter,
) -> Result<Vec<Resource>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut sql = String::from("SELECT * FROM resources");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref t) = filter.resource_type {
        conditions.push("resource_type = ?".to_string());
        binds.push(t.as_str().to_string());
    }

    if let Some(ref s) = filter.status {
        conditions.push("status = ?".to_string());
        binds.push(s.as_str().to_string());
    } else {
        conditions.push("status != ?".to_string());
        binds.push("deleted".to_string());
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

    if let Some(ref p) = filter.ownership_policy {
        conditions.push("ownership_policy = ?".to_string());
        binds.push(p.as_str().to_string());
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
        .map(Resource::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn update_resource(
    pool: &SqlitePool,
    req: UpdateResourceRequest,
) -> Result<Resource, NousError> {
    let existing = get_resource_by_id(pool, &req.id).await?;

    if existing.status == "deleted" {
        return Err(NousError::Validation(
            "cannot update a deleted resource".into(),
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

    if let Some(ref status) = req.status {
        sets.push("status = ?".to_string());
        binds.push(status.as_str().to_string());
        if *status == ResourceStatus::Archived {
            let now = chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string();
            sets.push("archived_at = COALESCE(archived_at, ?)".to_string());
            binds.push(now);
        }
    }

    if let Some(ref policy) = req.ownership_policy {
        sets.push("ownership_policy = ?".to_string());
        binds.push(policy.as_str().to_string());
    }

    if sets.is_empty() {
        return Ok(existing);
    }

    let sql = format!("UPDATE resources SET {} WHERE id = ?", sets.join(", "));
    binds.push(req.id.clone());

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }
    query.execute(pool).await?;

    get_resource_by_id(pool, &req.id).await
}

pub async fn archive_resource(pool: &SqlitePool, id: &str) -> Result<Resource, NousError> {
    let existing = get_resource_by_id(pool, id).await?;

    if existing.status != "active" {
        return Err(NousError::Validation(format!(
            "can only archive active resources, current status: '{}'",
            existing.status
        )));
    }

    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    sqlx::query("UPDATE resources SET status = 'archived', archived_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;

    get_resource_by_id(pool, id).await
}

pub async fn deregister_resource(
    pool: &SqlitePool,
    id: &str,
    hard: bool,
) -> Result<(), NousError> {
    let existing = get_resource_by_id(pool, id).await?;

    if hard {
        sqlx::query("DELETE FROM resources WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
    } else {
        if existing.status == "deleted" {
            return Err(NousError::Validation("resource is already deleted".into()));
        }
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        sqlx::query(
            "UPDATE resources SET status = 'deleted', archived_at = COALESCE(archived_at, ?) WHERE id = ?",
        )
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub async fn heartbeat_resource(pool: &SqlitePool, id: &str) -> Result<Resource, NousError> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let result = sqlx::query("UPDATE resources SET last_seen_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("resource '{id}' not found")));
    }

    get_resource_by_id(pool, id).await
}

pub async fn search_by_tags(
    pool: &SqlitePool,
    req: &SearchResourcesRequest,
) -> Result<Vec<Resource>, NousError> {
    if req.tags.is_empty() {
        return Err(NousError::Validation(
            "at least one tag is required for search".into(),
        ));
    }

    let limit = req.limit.unwrap_or(50).min(200);

    let mut sql = String::from("SELECT * FROM resources WHERE ");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    for tag in &req.tags {
        conditions.push("EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)".to_string());
        binds.push(tag.to_lowercase());
    }

    if let Some(ref t) = req.resource_type {
        conditions.push("resource_type = ?".to_string());
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
        .map(Resource::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn search_fts(
    pool: &SqlitePool,
    query_str: &str,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Resource>, NousError> {
    if query_str.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let rows = if let Some(ns) = namespace {
        sqlx::query(
            "SELECT r.* FROM resources r \
             INNER JOIN resources_fts f ON f.rowid = r.rowid \
             WHERE resources_fts MATCH ? AND r.namespace = ? \
             LIMIT ?",
        )
        .bind(query_str)
        .bind(ns)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT r.* FROM resources r \
             INNER JOIN resources_fts f ON f.rowid = r.rowid \
             WHERE resources_fts MATCH ? \
             LIMIT ?",
        )
        .bind(query_str)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    rows.iter()
        .map(Resource::from_row)
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
            "UPDATE resources SET owner_agent_id = ? \
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
            "UPDATE resources SET owner_agent_id = NULL, status = 'archived', archived_at = ? \
             WHERE owner_agent_id = ? AND status = 'active'",
        )
        .bind(&now)
        .bind(from_agent_id)
        .execute(pool)
        .await?
    };

    Ok(result.rows_affected())
}

pub async fn handle_agent_deregistration(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<(), NousError> {
    // 1. Hard-delete resources with cascade-delete policy
    sqlx::query(
        "DELETE FROM resources WHERE owner_agent_id = ? AND ownership_policy = 'cascade-delete'",
    )
    .bind(agent_id)
    .execute(pool)
    .await?;

    // 2. Transfer resources with transfer-to-parent policy
    let parent_row =
        sqlx::query("SELECT parent_id FROM agent_relationships WHERE child_id = ? LIMIT 1")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;

    if let Some(row) = parent_row {
        let parent_id: String = row.try_get("parent_id").map_err(NousError::Sqlite)?;
        sqlx::query(
            "UPDATE resources SET owner_agent_id = ? \
             WHERE owner_agent_id = ? AND ownership_policy = 'transfer-to-parent'",
        )
        .bind(&parent_id)
        .bind(agent_id)
        .execute(pool)
        .await?;
    } else {
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        sqlx::query(
            "UPDATE resources SET owner_agent_id = NULL, status = 'archived', archived_at = ? \
             WHERE owner_agent_id = ? AND ownership_policy = 'transfer-to-parent'",
        )
        .bind(&now)
        .bind(agent_id)
        .execute(pool)
        .await?;
    }

    // 3. Orphan remaining resources (policy = 'orphan')
    sqlx::query(
        "UPDATE resources SET owner_agent_id = NULL \
         WHERE owner_agent_id = ? AND ownership_policy = 'orphan'",
    )
    .bind(agent_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use tempfile::TempDir;

    async fn test_pool() -> (SqlitePool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations("porter unicode61").await.unwrap();
        (pools.fts.clone(), tmp)
    }

    async fn create_agent(pool: &SqlitePool, name: &str) -> String {
        let agent = crate::agents::register_agent(
            pool,
            crate::agents::RegisterAgentRequest {
                name: name.to_string(),
                agent_type: crate::agents::AgentType::Engineer,
                namespace: None,
                parent_id: None,
                room: None,
                metadata: None,
                status: None,
            },
        )
        .await
        .unwrap();
        agent.id
    }

    fn reg(name: &str, rt: ResourceType) -> RegisterResourceRequest {
        RegisterResourceRequest {
            name: name.to_string(),
            resource_type: rt,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
            ownership_policy: None,
        }
    }

    // --- CRUD ---

    #[tokio::test]
    async fn register_and_get() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("my-wt", ResourceType::Worktree))
            .await
            .unwrap();
        assert_eq!(r.name, "my-wt");
        assert_eq!(r.resource_type, "worktree");
        assert_eq!(r.status, "active");
        assert_eq!(r.namespace, "default");
        assert_eq!(r.ownership_policy, "cascade-delete");

        let fetched = get_resource_by_id(&pool, &r.id).await.unwrap();
        assert_eq!(fetched.id, r.id);
    }

    #[tokio::test]
    async fn register_with_owner() {
        let (pool, _tmp) = test_pool().await;
        let agent_id = create_agent(&pool, "owner-agent").await;

        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ..reg("owned-room", ResourceType::Room)
            },
        )
        .await
        .unwrap();
        assert_eq!(r.owner_agent_id.as_deref(), Some(agent_id.as_str()));
        assert_eq!(r.ownership_policy, "orphan");
    }

    #[tokio::test]
    async fn register_empty_name_rejected() {
        let (pool, _tmp) = test_pool().await;
        let err = register_resource(&pool, reg("  ", ResourceType::File))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn register_invalid_metadata_rejected() {
        let (pool, _tmp) = test_pool().await;
        let err = register_resource(
            &pool,
            RegisterResourceRequest {
                metadata: Some("not-json".into()),
                ..reg("bad-meta", ResourceType::File)
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("metadata must be valid JSON"));
    }

    #[tokio::test]
    async fn register_nonexistent_owner_rejected() {
        let (pool, _tmp) = test_pool().await;
        let err = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some("no-such-agent".into()),
                ..reg("orphan-res", ResourceType::File)
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn get_nonexistent_returns_not_found() {
        let (pool, _tmp) = test_pool().await;
        let err = get_resource_by_id(&pool, "no-such-id").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn register_with_tags_and_metadata() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                tags: Some(vec!["CI".into(), "prod".into()]),
                metadata: Some(r#"{"key":"value"}"#.into()),
                ..reg("tagged-file", ResourceType::File)
            },
        )
        .await
        .unwrap();
        assert_eq!(r.tags_vec(), vec!["ci", "prod"]);
        assert_eq!(r.metadata.as_deref(), Some(r#"{"key":"value"}"#));
    }

    #[tokio::test]
    async fn default_ownership_policy_applied() {
        let (pool, _tmp) = test_pool().await;
        let wt = register_resource(&pool, reg("wt", ResourceType::Worktree))
            .await
            .unwrap();
        assert_eq!(wt.ownership_policy, "cascade-delete");

        let br = register_resource(&pool, reg("br", ResourceType::Branch))
            .await
            .unwrap();
        assert_eq!(br.ownership_policy, "cascade-delete");

        let rm = register_resource(&pool, reg("rm", ResourceType::Room))
            .await
            .unwrap();
        assert_eq!(rm.ownership_policy, "orphan");

        let f = register_resource(&pool, reg("f", ResourceType::File))
            .await
            .unwrap();
        assert_eq!(f.ownership_policy, "orphan");
    }

    #[tokio::test]
    async fn explicit_ownership_policy_overrides_default() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("xfer-room", ResourceType::Room)
            },
        )
        .await
        .unwrap();
        assert_eq!(r.ownership_policy, "transfer-to-parent");
    }

    // --- List / Filter ---

    #[tokio::test]
    async fn list_filters_by_type_and_status() {
        let (pool, _tmp) = test_pool().await;
        register_resource(&pool, reg("wt1", ResourceType::Worktree))
            .await
            .unwrap();
        register_resource(&pool, reg("rm1", ResourceType::Room))
            .await
            .unwrap();

        let wts = list_resources(
            &pool,
            &ListResourcesFilter {
                resource_type: Some(ResourceType::Worktree),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].name, "wt1");
    }

    #[tokio::test]
    async fn list_excludes_deleted_by_default() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("del-me", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&pool, &r.id, false).await.unwrap();

        let all = list_resources(&pool, &ListResourcesFilter::default())
            .await
            .unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn list_orphaned_filter() {
        let (pool, _tmp) = test_pool().await;
        let agent_id = create_agent(&pool, "orphan-test-agent").await;

        register_resource(&pool, reg("no-owner", ResourceType::File))
            .await
            .unwrap();
        register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id),
                ..reg("has-owner", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let orphans = list_resources(
            &pool,
            &ListResourcesFilter {
                orphaned: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].name, "no-owner");
    }

    #[tokio::test]
    async fn list_pagination() {
        let (pool, _tmp) = test_pool().await;
        for i in 0..5 {
            register_resource(&pool, reg(&format!("item-{i}"), ResourceType::File))
                .await
                .unwrap();
        }

        let page1 = list_resources(
            &pool,
            &ListResourcesFilter {
                limit: Some(2),
                offset: Some(0),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page1.len(), 2);

        let page2 = list_resources(
            &pool,
            &ListResourcesFilter {
                limit: Some(2),
                offset: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].id, page2[0].id);
    }

    // --- Update ---

    #[tokio::test]
    async fn update_fields() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("orig", ResourceType::File))
            .await
            .unwrap();

        let updated = update_resource(
            &pool,
            UpdateResourceRequest {
                id: r.id.clone(),
                name: Some("renamed".into()),
                path: Some("/new/path".into()),
                metadata: Some(r#"{"updated":true}"#.into()),
                tags: Some(vec!["new-tag".into()]),
                status: None,
                ownership_policy: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.path.as_deref(), Some("/new/path"));
        assert_eq!(updated.tags_vec(), vec!["new-tag"]);
    }

    #[tokio::test]
    async fn update_no_changes_returns_existing() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("nochange", ResourceType::File))
            .await
            .unwrap();

        let same = update_resource(
            &pool,
            UpdateResourceRequest {
                id: r.id.clone(),
                name: None,
                path: None,
                metadata: None,
                tags: None,
                status: None,
                ownership_policy: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(same.id, r.id);
    }

    #[tokio::test]
    async fn update_deleted_resource_rejected() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("del-upd", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&pool, &r.id, false).await.unwrap();

        let err = update_resource(
            &pool,
            UpdateResourceRequest {
                id: r.id,
                name: Some("x".into()),
                path: None,
                metadata: None,
                tags: None,
                status: None,
                ownership_policy: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("deleted"));
    }

    #[tokio::test]
    async fn update_ownership_policy() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("policy-upd", ResourceType::Room))
            .await
            .unwrap();
        assert_eq!(r.ownership_policy, "orphan");

        let updated = update_resource(
            &pool,
            UpdateResourceRequest {
                id: r.id,
                name: None,
                path: None,
                metadata: None,
                tags: None,
                status: None,
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.ownership_policy, "transfer-to-parent");
    }

    // --- Archive ---

    #[tokio::test]
    async fn archive_sets_status_and_timestamp() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("to-archive", ResourceType::File))
            .await
            .unwrap();
        assert!(r.archived_at.is_none());

        let archived = archive_resource(&pool, &r.id).await.unwrap();
        assert_eq!(archived.status, "archived");
        assert!(archived.archived_at.is_some());
    }

    #[tokio::test]
    async fn archive_non_active_rejected() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("arch-twice", ResourceType::File))
            .await
            .unwrap();
        archive_resource(&pool, &r.id).await.unwrap();

        let err = archive_resource(&pool, &r.id).await.unwrap_err();
        assert!(err.to_string().contains("can only archive active"));
    }

    // --- Deregister ---

    #[tokio::test]
    async fn deregister_hard_deletes() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("hard-del", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&pool, &r.id, true).await.unwrap();

        let err = get_resource_by_id(&pool, &r.id).await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn deregister_soft_sets_deleted() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("soft-del", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&pool, &r.id, false).await.unwrap();

        let fetched = get_resource_by_id(&pool, &r.id).await.unwrap();
        assert_eq!(fetched.status, "deleted");
        assert!(fetched.archived_at.is_some());
    }

    #[tokio::test]
    async fn deregister_soft_already_deleted_rejected() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("del-twice", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&pool, &r.id, false).await.unwrap();

        let err = deregister_resource(&pool, &r.id, false).await.unwrap_err();
        assert!(err.to_string().contains("already deleted"));
    }

    // --- Heartbeat ---

    #[tokio::test]
    async fn heartbeat_updates_last_seen() {
        let (pool, _tmp) = test_pool().await;
        let r = register_resource(&pool, reg("hb-res", ResourceType::File))
            .await
            .unwrap();
        assert!(r.last_seen_at.is_none());

        let hb = heartbeat_resource(&pool, &r.id).await.unwrap();
        assert!(hb.last_seen_at.is_some());
    }

    #[tokio::test]
    async fn heartbeat_nonexistent_returns_not_found() {
        let (pool, _tmp) = test_pool().await;
        // Verify the table exists by doing a quick list first
        list_resources(&pool, &ListResourcesFilter::default())
            .await
            .unwrap();
        let err = heartbeat_resource(&pool, "nope").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not found"), "unexpected error: {msg}");
    }

    // --- Tag search ---

    #[tokio::test]
    async fn search_by_tags_finds_matching() {
        let (pool, _tmp) = test_pool().await;
        register_resource(
            &pool,
            RegisterResourceRequest {
                tags: Some(vec!["ci".into(), "prod".into()]),
                ..reg("tagged", ResourceType::File)
            },
        )
        .await
        .unwrap();
        register_resource(
            &pool,
            RegisterResourceRequest {
                tags: Some(vec!["dev".into()]),
                ..reg("untagged", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let results = search_by_tags(
            &pool,
            &SearchResourcesRequest {
                tags: vec!["ci".into()],
                resource_type: None,
                status: None,
                namespace: None,
                limit: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "tagged");
    }

    #[tokio::test]
    async fn search_by_tags_empty_tags_rejected() {
        let (pool, _tmp) = test_pool().await;
        let err = search_by_tags(
            &pool,
            &SearchResourcesRequest {
                tags: vec![],
                resource_type: None,
                status: None,
                namespace: None,
                limit: None,
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("at least one tag"));
    }

    // --- FTS ---

    #[tokio::test]
    async fn fts_search_finds_by_name() {
        let (pool, _tmp) = test_pool().await;
        register_resource(&pool, reg("deployment-pipeline", ResourceType::File))
            .await
            .unwrap();
        register_resource(&pool, reg("unrelated-thing", ResourceType::File))
            .await
            .unwrap();

        let results = search_fts(&pool, "deployment", None, None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "deployment-pipeline");
    }

    #[tokio::test]
    async fn fts_search_respects_namespace() {
        let (pool, _tmp) = test_pool().await;
        register_resource(
            &pool,
            RegisterResourceRequest {
                namespace: Some("ns-a".into()),
                ..reg("shared-name", ResourceType::File)
            },
        )
        .await
        .unwrap();
        register_resource(
            &pool,
            RegisterResourceRequest {
                namespace: Some("ns-b".into()),
                ..reg("shared-name", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let results = search_fts(&pool, "shared", Some("ns-a"), None)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].namespace, "ns-a");
    }

    #[tokio::test]
    async fn fts_empty_query_rejected() {
        let (pool, _tmp) = test_pool().await;
        let err = search_fts(&pool, "  ", None, None).await.unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    // --- Transfer ownership ---

    #[tokio::test]
    async fn transfer_to_agent() {
        let (pool, _tmp) = test_pool().await;
        let from_id = create_agent(&pool, "from-agent").await;
        let to_id = create_agent(&pool, "to-agent").await;

        register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(from_id.clone()),
                ..reg("xfer-file", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let count = transfer_ownership(&pool, &from_id, Some(&to_id))
            .await
            .unwrap();
        assert_eq!(count, 1);

        let resources = list_resources(
            &pool,
            &ListResourcesFilter {
                owner_agent_id: Some(to_id.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].name, "xfer-file");
    }

    #[tokio::test]
    async fn transfer_to_none_archives() {
        let (pool, _tmp) = test_pool().await;
        let from_id = create_agent(&pool, "from-agent2").await;

        register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(from_id.clone()),
                ..reg("orphan-file", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let count = transfer_ownership(&pool, &from_id, None).await.unwrap();
        assert_eq!(count, 1);

        let resources = list_resources(
            &pool,
            &ListResourcesFilter {
                status: Some(ResourceStatus::Archived),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(resources.len(), 1);
        assert!(resources[0].owner_agent_id.is_none());
    }

    #[tokio::test]
    async fn transfer_to_nonexistent_agent_rejected() {
        let (pool, _tmp) = test_pool().await;
        let from_id = create_agent(&pool, "from-agent3").await;

        let err = transfer_ownership(&pool, &from_id, Some("no-such-agent"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    // --- Agent deregistration ownership policy ---

    #[tokio::test]
    async fn deregistration_cascade_deletes() {
        let (pool, _tmp) = test_pool().await;
        let agent_id = create_agent(&pool, "cascade-agent").await;

        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ownership_policy: Some(OwnershipPolicy::CascadeDelete),
                ..reg("ephemeral-wt", ResourceType::Worktree)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&pool, &agent_id).await.unwrap();

        let err = get_resource_by_id(&pool, &r.id).await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn deregistration_orphans_resources() {
        let (pool, _tmp) = test_pool().await;
        let agent_id = create_agent(&pool, "orphan-agent").await;

        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ownership_policy: Some(OwnershipPolicy::Orphan),
                ..reg("orphan-room", ResourceType::Room)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&pool, &agent_id).await.unwrap();

        let fetched = get_resource_by_id(&pool, &r.id).await.unwrap();
        assert!(fetched.owner_agent_id.is_none());
        assert_eq!(fetched.status, "active");
    }

    #[tokio::test]
    async fn deregistration_transfers_to_parent() {
        let (pool, _tmp) = test_pool().await;
        let parent_id = create_agent(&pool, "parent-agent").await;
        let child = crate::agents::register_agent(
            &pool,
            crate::agents::RegisterAgentRequest {
                name: "child-agent".into(),
                agent_type: crate::agents::AgentType::Engineer,
                namespace: None,
                parent_id: Some(parent_id.clone()),
                room: None,
                metadata: None,
                status: None,
            },
        )
        .await
        .unwrap();

        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("xfer-sched", ResourceType::Schedule)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&pool, &child.id).await.unwrap();

        let fetched = get_resource_by_id(&pool, &r.id).await.unwrap();
        assert_eq!(fetched.owner_agent_id.as_deref(), Some(parent_id.as_str()));
        assert_eq!(fetched.status, "active");
    }

    #[tokio::test]
    async fn deregistration_transfer_no_parent_archives() {
        let (pool, _tmp) = test_pool().await;
        let agent_id = create_agent(&pool, "no-parent-agent").await;

        let r = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("xfer-no-parent", ResourceType::Schedule)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&pool, &agent_id).await.unwrap();

        let fetched = get_resource_by_id(&pool, &r.id).await.unwrap();
        assert!(fetched.owner_agent_id.is_none());
        assert_eq!(fetched.status, "archived");
    }

    #[tokio::test]
    async fn deregistration_mixed_policies() {
        let (pool, _tmp) = test_pool().await;
        let parent_id = create_agent(&pool, "mixed-parent").await;
        let child = crate::agents::register_agent(
            &pool,
            crate::agents::RegisterAgentRequest {
                name: "mixed-child".into(),
                agent_type: crate::agents::AgentType::Engineer,
                namespace: None,
                parent_id: Some(parent_id.clone()),
                room: None,
                metadata: None,
                status: None,
            },
        )
        .await
        .unwrap();

        let cascade = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::CascadeDelete),
                ..reg("cascade-wt", ResourceType::Worktree)
            },
        )
        .await
        .unwrap();
        let transfer = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("transfer-sched", ResourceType::Schedule)
            },
        )
        .await
        .unwrap();
        let orphan = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::Orphan),
                ..reg("orphan-file", ResourceType::File)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&pool, &child.id).await.unwrap();

        assert!(get_resource_by_id(&pool, &cascade.id).await.is_err());

        let t = get_resource_by_id(&pool, &transfer.id).await.unwrap();
        assert_eq!(t.owner_agent_id.as_deref(), Some(parent_id.as_str()));

        let o = get_resource_by_id(&pool, &orphan.id).await.unwrap();
        assert!(o.owner_agent_id.is_none());
        assert_eq!(o.status, "active");
    }

    // --- Type parsing ---

    #[tokio::test]
    async fn type_round_trips() {
        for (s, expected) in [
            ("worktree", ResourceType::Worktree),
            ("room", ResourceType::Room),
            ("schedule", ResourceType::Schedule),
            ("branch", ResourceType::Branch),
            ("file", ResourceType::File),
            ("docker-image", ResourceType::DockerImage),
            ("binary", ResourceType::Binary),
        ] {
            let parsed: ResourceType = s.parse().unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), s);
        }
    }

    #[tokio::test]
    async fn invalid_type_parse_fails() {
        let err = "invalid".parse::<ResourceType>().unwrap_err();
        assert!(err.to_string().contains("invalid resource type"));
    }

    #[tokio::test]
    async fn namespace_mismatch_rejected() {
        let (pool, _tmp) = test_pool().await;
        let agent_id = create_agent(&pool, "ns-agent").await;

        let err = register_resource(
            &pool,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id),
                namespace: Some("wrong-ns".into()),
                ..reg("ns-mismatch", ResourceType::File)
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("namespace must match"));
    }
}
