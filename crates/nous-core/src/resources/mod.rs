use sea_orm::entity::prelude::*;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, NotSet, QueryOrder, QuerySelect, Set, Statement,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::resources as res_entity;
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
    fn from_model(m: res_entity::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            resource_type: m.resource_type,
            owner_agent_id: m.owner_agent_id,
            namespace: m.namespace,
            path: m.path,
            status: m.status,
            metadata: m.metadata,
            tags: m.tags,
            ownership_policy: m.ownership_policy,
            last_seen_at: m.last_seen_at,
            created_at: m.created_at,
            updated_at: m.updated_at,
            archived_at: m.archived_at,
        }
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
    db: &DatabaseConnection,
    req: RegisterResourceRequest,
) -> Result<Resource, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation(
            "resource name cannot be empty".into(),
        ));
    }

    let namespace = req.namespace.unwrap_or_else(|| "default".to_string());

    if let Some(ref agent_id) = req.owner_agent_id {
        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT namespace FROM agents WHERE id = ?",
            [agent_id.clone().into()],
        );
        let row = db.query_one(stmt).await?;
        let row =
            row.ok_or_else(|| NousError::NotFound(format!("owner agent '{agent_id}' not found")))?;
        let agent_ns: String = row.try_get("", "namespace")?;
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

    let model = res_entity::ActiveModel {
        id: Set(id.clone()),
        name: Set(req.name.trim().to_string()),
        resource_type: Set(req.resource_type.as_str().to_string()),
        owner_agent_id: Set(req.owner_agent_id),
        namespace: Set(namespace),
        path: Set(req.path),
        status: Set("active".to_string()),
        metadata: Set(req.metadata),
        tags: Set(tags_json),
        ownership_policy: Set(policy.as_str().to_string()),
        last_seen_at: Set(None),
        created_at: NotSet,
        updated_at: NotSet,
        archived_at: Set(None),
    };

    res_entity::Entity::insert(model).exec(db).await?;

    get_resource_by_id(db, &id).await
}

pub async fn get_resource_by_id(db: &DatabaseConnection, id: &str) -> Result<Resource, NousError> {
    let model = res_entity::Entity::find_by_id(id).one(db).await?;

    let model = model.ok_or_else(|| NousError::NotFound(format!("resource '{id}' not found")))?;
    Ok(Resource::from_model(model))
}

pub async fn list_resources(
    db: &DatabaseConnection,
    filter: &ListResourcesFilter,
) -> Result<Vec<Resource>, NousError> {
    use sea_orm::Condition;

    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut cond = Condition::all();

    if let Some(ref t) = filter.resource_type {
        cond = cond.add(res_entity::Column::ResourceType.eq(t.as_str()));
    }

    if let Some(ref s) = filter.status {
        cond = cond.add(res_entity::Column::Status.eq(s.as_str()));
    } else {
        cond = cond.add(res_entity::Column::Status.ne("deleted"));
    }

    if let Some(ref agent_id) = filter.owner_agent_id {
        cond = cond.add(res_entity::Column::OwnerAgentId.eq(agent_id.as_str()));
    }

    if let Some(ref ns) = filter.namespace {
        cond = cond.add(res_entity::Column::Namespace.eq(ns.as_str()));
    }

    if filter.orphaned == Some(true) {
        cond = cond.add(res_entity::Column::OwnerAgentId.is_null());
    }

    if let Some(ref p) = filter.ownership_policy {
        cond = cond.add(res_entity::Column::OwnershipPolicy.eq(p.as_str()));
    }

    let models = res_entity::Entity::find()
        .filter(cond)
        .order_by_desc(res_entity::Column::CreatedAt)
        .limit(Some(limit as u64))
        .offset(Some(offset as u64))
        .all(db)
        .await?;

    Ok(models.into_iter().map(Resource::from_model).collect())
}

pub async fn update_resource(
    db: &DatabaseConnection,
    req: UpdateResourceRequest,
) -> Result<Resource, NousError> {
    let existing = get_resource_by_id(db, &req.id).await?;

    if existing.status == "deleted" {
        return Err(NousError::Validation(
            "cannot update a deleted resource".into(),
        ));
    }

    let mut sets: Vec<String> = Vec::new();
    let mut params: Vec<sea_orm::Value> = Vec::new();

    if let Some(ref name) = req.name {
        if name.trim().is_empty() {
            return Err(NousError::Validation("name cannot be empty".into()));
        }
        sets.push("name = ?".to_string());
        params.push(name.trim().to_string().into());
    }

    if let Some(ref path) = req.path {
        sets.push("path = ?".to_string());
        params.push(path.clone().into());
    }

    if let Some(ref metadata) = req.metadata {
        serde_json::from_str::<serde_json::Value>(metadata)
            .map_err(|e| NousError::Validation(format!("metadata must be valid JSON: {e}")))?;
        sets.push("metadata = ?".to_string());
        params.push(metadata.clone().into());
    }

    if let Some(ref tags) = req.tags {
        let tags_json =
            serde_json::to_string(&tags.iter().map(|t| t.to_lowercase()).collect::<Vec<_>>())
                .unwrap_or_else(|_| "[]".to_string());
        sets.push("tags = ?".to_string());
        params.push(tags_json.into());
    }

    if let Some(ref status) = req.status {
        sets.push("status = ?".to_string());
        params.push(status.as_str().to_string().into());
        if *status == ResourceStatus::Archived {
            let now = chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string();
            sets.push("archived_at = COALESCE(archived_at, ?)".to_string());
            params.push(now.into());
        }
    }

    if let Some(ref policy) = req.ownership_policy {
        sets.push("ownership_policy = ?".to_string());
        params.push(policy.as_str().to_string().into());
    }

    if sets.is_empty() {
        return Ok(existing);
    }

    let sql = format!("UPDATE resources SET {} WHERE id = ?", sets.join(", "));
    params.push(req.id.clone().into());

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &sql, params);
    db.execute(stmt).await?;

    get_resource_by_id(db, &req.id).await
}

pub async fn archive_resource(db: &DatabaseConnection, id: &str) -> Result<Resource, NousError> {
    let existing = get_resource_by_id(db, id).await?;

    if existing.status != "active" {
        return Err(NousError::Validation(format!(
            "can only archive active resources, current status: '{}'",
            existing.status
        )));
    }

    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE resources SET status = 'archived', archived_at = ? WHERE id = ?",
        [now.into(), id.into()],
    );
    db.execute(stmt).await?;

    get_resource_by_id(db, id).await
}

pub async fn deregister_resource(
    db: &DatabaseConnection,
    id: &str,
    hard: bool,
) -> Result<(), NousError> {
    let existing = get_resource_by_id(db, id).await?;

    if hard {
        res_entity::Entity::delete_by_id(id).exec(db).await?;
    } else {
        if existing.status == "deleted" {
            return Err(NousError::Validation("resource is already deleted".into()));
        }
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE resources SET status = 'deleted', archived_at = COALESCE(archived_at, ?) WHERE id = ?",
            [now.into(), id.into()],
        );
        db.execute(stmt).await?;
    }

    Ok(())
}

pub async fn heartbeat_resource(db: &DatabaseConnection, id: &str) -> Result<Resource, NousError> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE resources SET last_seen_at = ? WHERE id = ?",
        [now.into(), id.into()],
    );
    let result = db.execute(stmt).await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("resource '{id}' not found")));
    }

    get_resource_by_id(db, id).await
}

pub async fn search_by_tags(
    db: &DatabaseConnection,
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
    let mut params: Vec<sea_orm::Value> = Vec::new();

    for tag in &req.tags {
        conditions.push("EXISTS (SELECT 1 FROM json_each(tags) WHERE value = ?)".to_string());
        params.push(tag.to_lowercase().into());
    }

    if let Some(ref t) = req.resource_type {
        conditions.push("resource_type = ?".to_string());
        params.push(t.as_str().to_string().into());
    }

    if let Some(ref s) = req.status {
        conditions.push("status = ?".to_string());
        params.push(s.as_str().to_string().into());
    }

    if let Some(ref ns) = req.namespace {
        conditions.push("namespace = ?".to_string());
        params.push(ns.clone().into());
    }

    sql.push_str(&conditions.join(" AND "));
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    params.push((limit as i32).into());

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &sql, params);
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(|row| {
            Ok(Resource {
                id: row.try_get("", "id")?,
                name: row.try_get("", "name")?,
                resource_type: row.try_get("", "resource_type")?,
                owner_agent_id: row.try_get("", "owner_agent_id")?,
                namespace: row.try_get("", "namespace")?,
                path: row.try_get("", "path")?,
                status: row.try_get("", "status")?,
                metadata: row.try_get("", "metadata")?,
                tags: row.try_get("", "tags")?,
                ownership_policy: row.try_get("", "ownership_policy")?,
                last_seen_at: row.try_get("", "last_seen_at")?,
                created_at: row.try_get("", "created_at")?,
                updated_at: row.try_get("", "updated_at")?,
                archived_at: row.try_get("", "archived_at")?,
            })
        })
        .collect::<Result<Vec<_>, sea_orm::DbErr>>()
        .map_err(NousError::SeaOrm)
}

pub async fn search_fts(
    db: &DatabaseConnection,
    query_str: &str,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Resource>, NousError> {
    if query_str.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let (sql, params): (&str, Vec<sea_orm::Value>) = if let Some(ns) = namespace {
        (
            "SELECT r.* FROM resources r \
             INNER JOIN resources_fts f ON f.rowid = r.rowid \
             WHERE resources_fts MATCH ? AND r.namespace = ? \
             LIMIT ?",
            vec![query_str.into(), ns.into(), (limit as i32).into()],
        )
    } else {
        (
            "SELECT r.* FROM resources r \
             INNER JOIN resources_fts f ON f.rowid = r.rowid \
             WHERE resources_fts MATCH ? \
             LIMIT ?",
            vec![query_str.into(), (limit as i32).into()],
        )
    };

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, sql, params);
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(|row| {
            Ok(Resource {
                id: row.try_get("", "id")?,
                name: row.try_get("", "name")?,
                resource_type: row.try_get("", "resource_type")?,
                owner_agent_id: row.try_get("", "owner_agent_id")?,
                namespace: row.try_get("", "namespace")?,
                path: row.try_get("", "path")?,
                status: row.try_get("", "status")?,
                metadata: row.try_get("", "metadata")?,
                tags: row.try_get("", "tags")?,
                ownership_policy: row.try_get("", "ownership_policy")?,
                last_seen_at: row.try_get("", "last_seen_at")?,
                created_at: row.try_get("", "created_at")?,
                updated_at: row.try_get("", "updated_at")?,
                archived_at: row.try_get("", "archived_at")?,
            })
        })
        .collect::<Result<Vec<_>, sea_orm::DbErr>>()
        .map_err(NousError::SeaOrm)
}

pub async fn transfer_ownership(
    db: &DatabaseConnection,
    from_agent_id: &str,
    to_agent_id: Option<&str>,
) -> Result<u64, NousError> {
    let result = if let Some(target) = to_agent_id {
        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT id FROM agents WHERE id = ?",
            [target.into()],
        );
        db.query_one(stmt)
            .await?
            .ok_or_else(|| NousError::NotFound(format!("target agent '{target}' not found")))?;

        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE resources SET owner_agent_id = ? \
             WHERE owner_agent_id = ? AND status = 'active'",
            [target.into(), from_agent_id.into()],
        );
        db.execute(stmt).await?
    } else {
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE resources SET owner_agent_id = NULL, status = 'archived', archived_at = ? \
             WHERE owner_agent_id = ? AND status = 'active'",
            [now.into(), from_agent_id.into()],
        );
        db.execute(stmt).await?
    };

    Ok(result.rows_affected())
}

pub async fn handle_agent_deregistration(
    db: &DatabaseConnection,
    agent_id: &str,
) -> Result<(), NousError> {
    // 1. Hard-delete resources with cascade-delete policy
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "DELETE FROM resources WHERE owner_agent_id = ? AND ownership_policy = 'cascade-delete'",
        [agent_id.into()],
    );
    db.execute(stmt).await?;

    // 2. Transfer resources with transfer-to-parent policy
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "SELECT parent_id FROM agent_relationships WHERE child_id = ? LIMIT 1",
        [agent_id.into()],
    );
    let parent_row = db.query_one(stmt).await?;

    if let Some(row) = parent_row {
        let parent_id: String = row.try_get("", "parent_id")?;
        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE resources SET owner_agent_id = ? \
             WHERE owner_agent_id = ? AND ownership_policy = 'transfer-to-parent'",
            [parent_id.into(), agent_id.into()],
        );
        db.execute(stmt).await?;
    } else {
        let now = chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string();
        let stmt = Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE resources SET owner_agent_id = NULL, status = 'archived', archived_at = ? \
             WHERE owner_agent_id = ? AND ownership_policy = 'transfer-to-parent'",
            [now.into(), agent_id.into()],
        );
        db.execute(stmt).await?;
    }

    // 3. Orphan remaining resources (policy = 'orphan')
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE resources SET owner_agent_id = NULL \
         WHERE owner_agent_id = ? AND ownership_policy = 'orphan'",
        [agent_id.into()],
    );
    db.execute(stmt).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use tempfile::TempDir;

    async fn test_pool() -> (DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        (pools.fts.clone(), tmp)
    }

    async fn create_agent(db: &DatabaseConnection, name: &str) -> String {
        let agent = crate::agents::register_agent(
            db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("my-wt", ResourceType::Worktree))
            .await
            .unwrap();
        assert_eq!(r.name, "my-wt");
        assert_eq!(r.resource_type, "worktree");
        assert_eq!(r.status, "active");
        assert_eq!(r.namespace, "default");
        assert_eq!(r.ownership_policy, "cascade-delete");

        let fetched = get_resource_by_id(&db, &r.id).await.unwrap();
        assert_eq!(fetched.id, r.id);
    }

    #[tokio::test]
    async fn register_with_owner() {
        let (db, _tmp) = test_pool().await;
        let agent_id = create_agent(&db, "owner-agent").await;

        let r = register_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let err = register_resource(&db, reg("  ", ResourceType::File))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    #[tokio::test]
    async fn register_invalid_metadata_rejected() {
        let (db, _tmp) = test_pool().await;
        let err = register_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let err = register_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let err = get_resource_by_id(&db, "no-such-id").await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn register_with_tags_and_metadata() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let wt = register_resource(&db, reg("wt", ResourceType::Worktree))
            .await
            .unwrap();
        assert_eq!(wt.ownership_policy, "cascade-delete");

        let br = register_resource(&db, reg("br", ResourceType::Branch))
            .await
            .unwrap();
        assert_eq!(br.ownership_policy, "cascade-delete");

        let rm = register_resource(&db, reg("rm", ResourceType::Room))
            .await
            .unwrap();
        assert_eq!(rm.ownership_policy, "orphan");

        let f = register_resource(&db, reg("f", ResourceType::File))
            .await
            .unwrap();
        assert_eq!(f.ownership_policy, "orphan");
    }

    #[tokio::test]
    async fn explicit_ownership_policy_overrides_default() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        register_resource(&db, reg("wt1", ResourceType::Worktree))
            .await
            .unwrap();
        register_resource(&db, reg("rm1", ResourceType::Room))
            .await
            .unwrap();

        let wts = list_resources(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("del-me", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&db, &r.id, false).await.unwrap();

        let all = list_resources(&db, &ListResourcesFilter::default())
            .await
            .unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn list_orphaned_filter() {
        let (db, _tmp) = test_pool().await;
        let agent_id = create_agent(&db, "orphan-test-agent").await;

        register_resource(&db, reg("no-owner", ResourceType::File))
            .await
            .unwrap();
        register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id),
                ..reg("has-owner", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let orphans = list_resources(
            &db,
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
        let (db, _tmp) = test_pool().await;
        for i in 0..5 {
            register_resource(&db, reg(&format!("item-{i}"), ResourceType::File))
                .await
                .unwrap();
        }

        let page1 = list_resources(
            &db,
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
            &db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("orig", ResourceType::File))
            .await
            .unwrap();

        let updated = update_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("nochange", ResourceType::File))
            .await
            .unwrap();

        let same = update_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("del-upd", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&db, &r.id, false).await.unwrap();

        let err = update_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("policy-upd", ResourceType::Room))
            .await
            .unwrap();
        assert_eq!(r.ownership_policy, "orphan");

        let updated = update_resource(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("to-archive", ResourceType::File))
            .await
            .unwrap();
        assert!(r.archived_at.is_none());

        let archived = archive_resource(&db, &r.id).await.unwrap();
        assert_eq!(archived.status, "archived");
        assert!(archived.archived_at.is_some());
    }

    #[tokio::test]
    async fn archive_non_active_rejected() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("arch-twice", ResourceType::File))
            .await
            .unwrap();
        archive_resource(&db, &r.id).await.unwrap();

        let err = archive_resource(&db, &r.id).await.unwrap_err();
        assert!(err.to_string().contains("can only archive active"));
    }

    // --- Deregister ---

    #[tokio::test]
    async fn deregister_hard_deletes() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("hard-del", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&db, &r.id, true).await.unwrap();

        let err = get_resource_by_id(&db, &r.id).await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn deregister_soft_sets_deleted() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("soft-del", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&db, &r.id, false).await.unwrap();

        let fetched = get_resource_by_id(&db, &r.id).await.unwrap();
        assert_eq!(fetched.status, "deleted");
        assert!(fetched.archived_at.is_some());
    }

    #[tokio::test]
    async fn deregister_soft_already_deleted_rejected() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("del-twice", ResourceType::File))
            .await
            .unwrap();
        deregister_resource(&db, &r.id, false).await.unwrap();

        let err = deregister_resource(&db, &r.id, false).await.unwrap_err();
        assert!(err.to_string().contains("already deleted"));
    }

    // --- Heartbeat ---

    #[tokio::test]
    async fn heartbeat_updates_last_seen() {
        let (db, _tmp) = test_pool().await;
        let r = register_resource(&db, reg("hb-res", ResourceType::File))
            .await
            .unwrap();
        assert!(r.last_seen_at.is_none());

        let hb = heartbeat_resource(&db, &r.id).await.unwrap();
        assert!(hb.last_seen_at.is_some());
    }

    #[tokio::test]
    async fn heartbeat_nonexistent_returns_not_found() {
        let (db, _tmp) = test_pool().await;
        // Verify the table exists by doing a quick list first
        list_resources(&db, &ListResourcesFilter::default())
            .await
            .unwrap();
        let err = heartbeat_resource(&db, "nope").await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not found"), "unexpected error: {msg}");
    }

    // --- Tag search ---

    #[tokio::test]
    async fn search_by_tags_finds_matching() {
        let (db, _tmp) = test_pool().await;
        register_resource(
            &db,
            RegisterResourceRequest {
                tags: Some(vec!["ci".into(), "prod".into()]),
                ..reg("tagged", ResourceType::File)
            },
        )
        .await
        .unwrap();
        register_resource(
            &db,
            RegisterResourceRequest {
                tags: Some(vec!["dev".into()]),
                ..reg("untagged", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let results = search_by_tags(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let err = search_by_tags(
            &db,
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
        let (db, _tmp) = test_pool().await;
        register_resource(&db, reg("deployment-pipeline", ResourceType::File))
            .await
            .unwrap();
        register_resource(&db, reg("unrelated-thing", ResourceType::File))
            .await
            .unwrap();

        let results = search_fts(&db, "deployment", None, None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "deployment-pipeline");
    }

    #[tokio::test]
    async fn fts_search_respects_namespace() {
        let (db, _tmp) = test_pool().await;
        register_resource(
            &db,
            RegisterResourceRequest {
                namespace: Some("ns-a".into()),
                ..reg("shared-name", ResourceType::File)
            },
        )
        .await
        .unwrap();
        register_resource(
            &db,
            RegisterResourceRequest {
                namespace: Some("ns-b".into()),
                ..reg("shared-name", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let results = search_fts(&db, "shared", Some("ns-a"), None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].namespace, "ns-a");
    }

    #[tokio::test]
    async fn fts_empty_query_rejected() {
        let (db, _tmp) = test_pool().await;
        let err = search_fts(&db, "  ", None, None).await.unwrap_err();
        assert!(err.to_string().contains("cannot be empty"));
    }

    // --- Transfer ownership ---

    #[tokio::test]
    async fn transfer_to_agent() {
        let (db, _tmp) = test_pool().await;
        let from_id = create_agent(&db, "from-agent").await;
        let to_id = create_agent(&db, "to-agent").await;

        register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(from_id.clone()),
                ..reg("xfer-file", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let count = transfer_ownership(&db, &from_id, Some(&to_id))
            .await
            .unwrap();
        assert_eq!(count, 1);

        let resources = list_resources(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let from_id = create_agent(&db, "from-agent2").await;

        register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(from_id.clone()),
                ..reg("orphan-file", ResourceType::File)
            },
        )
        .await
        .unwrap();

        let count = transfer_ownership(&db, &from_id, None).await.unwrap();
        assert_eq!(count, 1);

        let resources = list_resources(
            &db,
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
        let (db, _tmp) = test_pool().await;
        let from_id = create_agent(&db, "from-agent3").await;

        let err = transfer_ownership(&db, &from_id, Some("no-such-agent"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    // --- Agent deregistration ownership policy ---

    #[tokio::test]
    async fn deregistration_cascade_deletes() {
        let (db, _tmp) = test_pool().await;
        let agent_id = create_agent(&db, "cascade-agent").await;

        let r = register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ownership_policy: Some(OwnershipPolicy::CascadeDelete),
                ..reg("ephemeral-wt", ResourceType::Worktree)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&db, &agent_id).await.unwrap();

        let err = get_resource_by_id(&db, &r.id).await.unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn deregistration_orphans_resources() {
        let (db, _tmp) = test_pool().await;
        let agent_id = create_agent(&db, "orphan-agent").await;

        let r = register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ownership_policy: Some(OwnershipPolicy::Orphan),
                ..reg("orphan-room", ResourceType::Room)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&db, &agent_id).await.unwrap();

        let fetched = get_resource_by_id(&db, &r.id).await.unwrap();
        assert!(fetched.owner_agent_id.is_none());
        assert_eq!(fetched.status, "active");
    }

    #[tokio::test]
    async fn deregistration_transfers_to_parent() {
        let (db, _tmp) = test_pool().await;
        let parent_id = create_agent(&db, "parent-agent").await;
        let child = crate::agents::register_agent(
            &db,
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
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("xfer-sched", ResourceType::Schedule)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&db, &child.id).await.unwrap();

        let fetched = get_resource_by_id(&db, &r.id).await.unwrap();
        assert_eq!(fetched.owner_agent_id.as_deref(), Some(parent_id.as_str()));
        assert_eq!(fetched.status, "active");
    }

    #[tokio::test]
    async fn deregistration_transfer_no_parent_archives() {
        let (db, _tmp) = test_pool().await;
        let agent_id = create_agent(&db, "no-parent-agent").await;

        let r = register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(agent_id.clone()),
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("xfer-no-parent", ResourceType::Schedule)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&db, &agent_id).await.unwrap();

        let fetched = get_resource_by_id(&db, &r.id).await.unwrap();
        assert!(fetched.owner_agent_id.is_none());
        assert_eq!(fetched.status, "archived");
    }

    #[tokio::test]
    async fn deregistration_mixed_policies() {
        let (db, _tmp) = test_pool().await;
        let parent_id = create_agent(&db, "mixed-parent").await;
        let child = crate::agents::register_agent(
            &db,
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
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::CascadeDelete),
                ..reg("cascade-wt", ResourceType::Worktree)
            },
        )
        .await
        .unwrap();
        let transfer = register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::TransferToParent),
                ..reg("transfer-sched", ResourceType::Schedule)
            },
        )
        .await
        .unwrap();
        let orphan = register_resource(
            &db,
            RegisterResourceRequest {
                owner_agent_id: Some(child.id.clone()),
                ownership_policy: Some(OwnershipPolicy::Orphan),
                ..reg("orphan-file", ResourceType::File)
            },
        )
        .await
        .unwrap();

        handle_agent_deregistration(&db, &child.id).await.unwrap();

        assert!(get_resource_by_id(&db, &cascade.id).await.is_err());

        let t = get_resource_by_id(&db, &transfer.id).await.unwrap();
        assert_eq!(t.owner_agent_id.as_deref(), Some(parent_id.as_str()));

        let o = get_resource_by_id(&db, &orphan.id).await.unwrap();
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
        let (db, _tmp) = test_pool().await;
        let agent_id = create_agent(&db, "ns-agent").await;

        let err = register_resource(
            &db,
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
