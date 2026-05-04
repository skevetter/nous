use serde::{Deserialize, Serialize};

use crate::entities::resources as res_entity;
use crate::error::NousError;

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
                "invalid resource type: '{other}'. Valid values: worktree, room, schedule, branch, file, docker-image, binary"
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
                "invalid resource status: '{other}'. Valid values: active, archived, deleted"
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
                "invalid ownership policy: '{other}'. Valid values: cascade-delete, orphan, transfer-to-parent"
            ))),
        }
    }
}

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
    pub(crate) fn from_model(m: res_entity::Model) -> Self {
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
