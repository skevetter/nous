use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

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
    fn from_resource(r: &crate::resources::Resource) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            artifact_type: r.resource_type.clone(),
            owner_agent_id: r.owner_agent_id.clone(),
            namespace: r.namespace.clone(),
            path: r.path.clone(),
            status: r.status.clone(),
            metadata: r.metadata.clone(),
            tags: r.tags.clone(),
            created_at: r.created_at.clone(),
            updated_at: r.updated_at.clone(),
            archived_at: r.archived_at.clone(),
        }
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
    pub status: Option<InventoryStatus>,
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

// --- Operations (delegate to resources module) ---

pub async fn register_item(
    db: &DatabaseConnection,
    req: RegisterItemRequest,
) -> Result<InventoryItem, NousError> {
    let resource_type: crate::resources::ResourceType = req.artifact_type.as_str().parse()?;
    let resource = crate::resources::register_resource(
        db,
        crate::resources::RegisterResourceRequest {
            name: req.name,
            resource_type,
            owner_agent_id: req.owner_agent_id,
            namespace: req.namespace,
            path: req.path,
            metadata: req.metadata,
            tags: req.tags,
            ownership_policy: Some(resource_type.default_ownership_policy()),
        },
    )
    .await?;
    Ok(InventoryItem::from_resource(&resource))
}

pub async fn get_item_by_id(db: &DatabaseConnection, id: &str) -> Result<InventoryItem, NousError> {
    let resource = crate::resources::get_resource_by_id(db, id).await?;
    Ok(InventoryItem::from_resource(&resource))
}

pub async fn list_items(
    db: &DatabaseConnection,
    filter: &ListItemsFilter,
) -> Result<Vec<InventoryItem>, NousError> {
    let resource_type = filter
        .artifact_type
        .map(|t| t.as_str().parse::<crate::resources::ResourceType>())
        .transpose()?;
    let status = filter
        .status
        .map(|s| s.as_str().parse::<crate::resources::ResourceStatus>())
        .transpose()?;

    let resources = crate::resources::list_resources(
        db,
        &crate::resources::ListResourcesFilter {
            resource_type,
            status,
            owner_agent_id: filter.owner_agent_id.clone(),
            namespace: filter.namespace.clone(),
            orphaned: filter.orphaned,
            limit: filter.limit,
            offset: filter.offset,
            ..Default::default()
        },
    )
    .await?;
    Ok(resources.iter().map(InventoryItem::from_resource).collect())
}

pub async fn update_item(
    db: &DatabaseConnection,
    req: UpdateItemRequest,
) -> Result<InventoryItem, NousError> {
    let status = req
        .status
        .map(|s| s.as_str().parse::<crate::resources::ResourceStatus>())
        .transpose()?;

    let resource = crate::resources::update_resource(
        db,
        crate::resources::UpdateResourceRequest {
            id: req.id,
            name: req.name,
            path: req.path,
            metadata: req.metadata,
            tags: req.tags,
            status,
            ownership_policy: None,
        },
    )
    .await?;
    Ok(InventoryItem::from_resource(&resource))
}

pub async fn archive_item(db: &DatabaseConnection, id: &str) -> Result<InventoryItem, NousError> {
    let resource = crate::resources::archive_resource(db, id).await?;
    Ok(InventoryItem::from_resource(&resource))
}

pub async fn deregister_item(
    db: &DatabaseConnection,
    id: &str,
    hard: bool,
) -> Result<(), NousError> {
    crate::resources::deregister_resource(db, id, hard).await
}

pub async fn search_by_tags(
    db: &DatabaseConnection,
    req: &SearchItemsRequest,
) -> Result<Vec<InventoryItem>, NousError> {
    let resource_type = req
        .artifact_type
        .map(|t| t.as_str().parse::<crate::resources::ResourceType>())
        .transpose()?;
    let status = req
        .status
        .map(|s| s.as_str().parse::<crate::resources::ResourceStatus>())
        .transpose()?;

    let resources = crate::resources::search_by_tags(
        db,
        &crate::resources::SearchResourcesRequest {
            tags: req.tags.clone(),
            resource_type,
            status,
            namespace: req.namespace.clone(),
            limit: req.limit,
        },
    )
    .await?;
    Ok(resources.iter().map(InventoryItem::from_resource).collect())
}

pub async fn search_fts(
    db: &DatabaseConnection,
    query_str: &str,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<InventoryItem>, NousError> {
    let resources = crate::resources::search_fts(db, query_str, namespace, limit).await?;
    Ok(resources.iter().map(InventoryItem::from_resource).collect())
}

pub async fn transfer_ownership(
    db: &DatabaseConnection,
    from_agent_id: &str,
    to_agent_id: Option<&str>,
) -> Result<u64, NousError> {
    crate::resources::transfer_ownership(db, from_agent_id, to_agent_id).await
}
