pub mod coordination;
pub mod definition;
pub mod processes;
pub mod sandbox;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;
use crate::notifications::subscribe_to_room;
use crate::rooms;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentType {
    Engineer,
    Manager,
    Director,
    SeniorManager,
}

impl AgentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Engineer => "engineer",
            Self::Manager => "manager",
            Self::Director => "director",
            Self::SeniorManager => "senior-manager",
        }
    }
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AgentType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "engineer" => Ok(Self::Engineer),
            "manager" => Ok(Self::Manager),
            "director" => Ok(Self::Director),
            "senior-manager" => Ok(Self::SeniorManager),
            other => Err(NousError::Validation(format!(
                "invalid agent type: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Active,
    Inactive,
    Archived,
    Running,
    Idle,
    Blocked,
    Done,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Inactive => "inactive",
            Self::Archived => "archived",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for AgentStatus {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "inactive" => Ok(Self::Inactive),
            "archived" => Ok(Self::Archived),
            "running" => Ok(Self::Running),
            "idle" => Ok(Self::Idle),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            other => Err(NousError::Validation(format!(
                "invalid agent status: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactType {
    Worktree,
    Room,
    Schedule,
    Branch,
}

impl ArtifactType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Worktree => "worktree",
            Self::Room => "room",
            Self::Schedule => "schedule",
            Self::Branch => "branch",
        }
    }
}

impl std::fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ArtifactType {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "worktree" => Ok(Self::Worktree),
            "room" => Ok(Self::Room),
            "schedule" => Ok(Self::Schedule),
            "branch" => Ok(Self::Branch),
            other => Err(NousError::Validation(format!(
                "invalid artifact type: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactStatus {
    Active,
    Archived,
    Deleted,
}

impl ArtifactStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }
}

impl std::fmt::Display for ArtifactStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ArtifactStatus {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "archived" => Ok(Self::Archived),
            "deleted" => Ok(Self::Deleted),
            other => Err(NousError::Validation(format!(
                "invalid artifact status: '{other}'"
            ))),
        }
    }
}

// --- Domain objects ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub agent_type: String,
    pub parent_agent_id: Option<String>,
    pub namespace: String,
    pub status: String,
    pub room: Option<String>,
    pub last_seen_at: Option<String>,
    pub metadata_json: Option<String>,
    pub current_version_id: Option<String>,
    pub upgrade_available: bool,
    pub template_id: Option<String>,
    pub process_type: Option<String>,
    pub spawn_command: Option<String>,
    pub working_dir: Option<String>,
    pub auto_restart: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl Agent {
    pub(crate) fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        let upgrade_flag: i32 = row.try_get("upgrade_available")?;
        let auto_restart_flag: i32 = row.try_get("auto_restart")?;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            agent_type: row.try_get("agent_type")?,
            parent_agent_id: row.try_get("parent_agent_id")?,
            namespace: row.try_get("namespace")?,
            status: row.try_get("status")?,
            room: row.try_get("room")?,
            last_seen_at: row.try_get("last_seen_at")?,
            metadata_json: row.try_get("metadata_json")?,
            current_version_id: row.try_get("current_version_id")?,
            upgrade_available: upgrade_flag != 0,
            template_id: row.try_get("template_id")?,
            process_type: row.try_get("process_type")?,
            spawn_command: row.try_get("spawn_command")?,
            working_dir: row.try_get("working_dir")?,
            auto_restart: auto_restart_flag != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub agent_id: String,
    pub artifact_type: String,
    pub name: String,
    pub path: Option<String>,
    pub status: String,
    pub namespace: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_seen_at: Option<String>,
}

impl Artifact {
    fn from_resource(r: &crate::resources::Resource) -> Self {
        Self {
            id: r.id.clone(),
            agent_id: r.owner_agent_id.clone().unwrap_or_default(),
            artifact_type: r.resource_type.clone(),
            name: r.name.clone(),
            path: r.path.clone(),
            status: r.status.clone(),
            namespace: r.namespace.clone(),
            created_at: r.created_at.clone(),
            updated_at: r.updated_at.clone(),
            last_seen_at: r.last_seen_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    #[serde(flatten)]
    pub agent: Agent,
    pub children: Vec<TreeNode>,
}

// --- Request types ---

#[derive(Debug, Clone)]
pub struct RegisterAgentRequest {
    pub name: String,
    pub agent_type: AgentType,
    pub parent_id: Option<String>,
    pub namespace: Option<String>,
    pub room: Option<String>,
    pub metadata: Option<String>,
    pub status: Option<AgentStatus>,
}

#[derive(Debug, Clone, Default)]
pub struct ListAgentsFilter {
    pub namespace: Option<String>,
    pub status: Option<AgentStatus>,
    pub agent_type: Option<AgentType>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RegisterArtifactRequest {
    pub agent_id: String,
    pub artifact_type: ArtifactType,
    pub name: String,
    pub path: Option<String>,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListArtifactsFilter {
    pub agent_id: Option<String>,
    pub artifact_type: Option<ArtifactType>,
    pub namespace: Option<String>,
    pub status: Option<ArtifactStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// --- Agent operations ---

pub async fn register_agent(
    pool: &SqlitePool,
    req: RegisterAgentRequest,
) -> Result<Agent, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation("agent name cannot be empty".into()));
    }

    let namespace = req.namespace.unwrap_or_else(|| "default".to_string());
    let status = req.status.unwrap_or(AgentStatus::Active);
    let id = Uuid::now_v7().to_string();

    if let Some(ref parent_id) = req.parent_id {
        let parent = get_agent_by_id(pool, parent_id).await?;
        if parent.namespace != namespace {
            return Err(NousError::Validation(
                "parent agent must be in the same namespace".into(),
            ));
        }
    }

    sqlx::query(
        "INSERT INTO agents (id, name, agent_type, parent_agent_id, namespace, status, room, metadata_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(req.name.trim())
    .bind(req.agent_type.as_str())
    .bind(&req.parent_id)
    .bind(&namespace)
    .bind(status.as_str())
    .bind(&req.room)
    .bind(&req.metadata)
    .execute(pool)
    .await?;

    if let Some(ref parent_id) = req.parent_id {
        sqlx::query(
            "INSERT INTO agent_relationships (parent_id, child_id, namespace) VALUES (?, ?, ?)",
        )
        .bind(parent_id)
        .bind(&id)
        .bind(&namespace)
        .execute(pool)
        .await?;

        let parent = get_agent_by_id(pool, parent_id).await?;
        let coord_room_name = format!("coord-{}-{}", namespace, parent.name);
        match rooms::create_room(
            pool,
            &coord_room_name,
            Some(&format!("Coordination room for {}", parent.name)),
            None,
        )
        .await
        {
            Ok(room) => {
                subscribe_to_room(pool, &room.id, parent_id, None).await?;
                subscribe_to_room(pool, &room.id, &id, None).await?;
            }
            Err(NousError::Conflict(_)) => {
                let room = rooms::get_room(pool, &coord_room_name).await?;
                subscribe_to_room(pool, &room.id, &id, None).await?;
            }
            Err(e) => return Err(e),
        }
    }

    get_agent_by_id(pool, &id).await
}

pub async fn get_agent_by_id(pool: &SqlitePool, id: &str) -> Result<Agent, NousError> {
    let row = sqlx::query("SELECT * FROM agents WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("agent '{id}' not found")))?;
    Agent::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn lookup_agent(
    pool: &SqlitePool,
    name: &str,
    namespace: Option<&str>,
) -> Result<Agent, NousError> {
    let ns = namespace.unwrap_or("default");
    let row = sqlx::query("SELECT * FROM agents WHERE name = ? AND namespace = ?")
        .bind(name)
        .bind(ns)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| {
        NousError::NotFound(format!("agent '{name}' not found in namespace '{ns}'"))
    })?;
    Agent::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_agents(
    pool: &SqlitePool,
    filter: &ListAgentsFilter,
) -> Result<Vec<Agent>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut sql = String::from("SELECT * FROM agents");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref ns) = filter.namespace {
        conditions.push("namespace = ?".to_string());
        binds.push(ns.clone());
    }

    if let Some(ref s) = filter.status {
        conditions.push("status = ?".to_string());
        binds.push(s.as_str().to_string());
    }

    if let Some(ref t) = filter.agent_type {
        conditions.push("agent_type = ?".to_string());
        binds.push(t.as_str().to_string());
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
        .map(Agent::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub fn deregister_agent<'a>(
    pool: &'a SqlitePool,
    id: &'a str,
    cascade: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, NousError>> + Send + 'a>> {
    let id = id.to_string();
    Box::pin(async move {
        let _agent = get_agent_by_id(pool, &id).await?;

        let children: Vec<Agent> = {
            let rows = sqlx::query(
                "SELECT a.* FROM agents a \
                 INNER JOIN agent_relationships r ON r.child_id = a.id \
                 WHERE r.parent_id = ? AND r.namespace = a.namespace",
            )
            .bind(&id)
            .fetch_all(pool)
            .await?;

            rows.iter()
                .map(Agent::from_row)
                .collect::<Result<Vec<_>, _>>()
                .map_err(NousError::Sqlite)?
        };

        if !children.is_empty() && !cascade {
            return Err(NousError::Conflict(format!(
                "agent '{id}' has {} children; use cascade=true to delete",
                children.len()
            )));
        }

        if cascade {
            for child in &children {
                deregister_agent(pool, &child.id, true).await?;
            }
        }

        // Handle resource ownership policies before deleting
        crate::resources::handle_agent_deregistration(pool, &id).await?;

        // Clean up process records before deleting (NO CASCADE on agent_processes FK)
        processes::cleanup_agent_processes(pool, &id).await?;

        sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(&id)
            .execute(pool)
            .await?;

        if cascade && !children.is_empty() {
            Ok("cascaded".to_string())
        } else {
            Ok("deleted".to_string())
        }
    })
}

pub async fn update_agent_status(
    pool: &SqlitePool,
    id: &str,
    status: AgentStatus,
) -> Result<Agent, NousError> {
    let result = sqlx::query("UPDATE agents SET status = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("agent '{id}' not found")));
    }

    get_agent_by_id(pool, id).await
}

pub async fn heartbeat(
    pool: &SqlitePool,
    id: &str,
    status: Option<AgentStatus>,
) -> Result<(), NousError> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let new_status = status.unwrap_or(AgentStatus::Active);

    let result = sqlx::query("UPDATE agents SET last_seen_at = ?, status = ? WHERE id = ?")
        .bind(&now)
        .bind(new_status.as_str())
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("agent '{id}' not found")));
    }

    Ok(())
}

// --- Tree traversal ---

pub async fn list_children(
    pool: &SqlitePool,
    parent_id: &str,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");
    let rows = sqlx::query(
        "SELECT a.* FROM agents a \
         INNER JOIN agent_relationships r ON r.child_id = a.id \
         WHERE r.parent_id = ? AND r.namespace = ? \
         ORDER BY a.created_at",
    )
    .bind(parent_id)
    .bind(ns)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(Agent::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn list_ancestors(
    pool: &SqlitePool,
    agent_id: &str,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");
    let mut ancestors = Vec::new();
    let mut current_id = agent_id.to_string();

    loop {
        let row = sqlx::query(
            "SELECT parent_id FROM agent_relationships WHERE child_id = ? AND namespace = ?",
        )
        .bind(&current_id)
        .bind(ns)
        .fetch_optional(pool)
        .await?;

        match row {
            Some(r) => {
                let parent_id: String = r.try_get("parent_id").map_err(NousError::Sqlite)?;
                let parent = get_agent_by_id(pool, &parent_id).await?;
                ancestors.push(parent);
                current_id = parent_id;
            }
            None => break,
        }
    }

    ancestors.reverse();
    Ok(ancestors)
}

pub async fn get_tree(
    pool: &SqlitePool,
    root_id: Option<&str>,
    namespace: Option<&str>,
) -> Result<Vec<TreeNode>, NousError> {
    let ns = namespace.unwrap_or("default");

    let roots = if let Some(id) = root_id {
        vec![get_agent_by_id(pool, id).await?]
    } else {
        let rows = sqlx::query(
            "SELECT * FROM agents WHERE parent_agent_id IS NULL AND namespace = ? ORDER BY created_at",
        )
        .bind(ns)
        .fetch_all(pool)
        .await?;

        rows.iter()
            .map(Agent::from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(NousError::Sqlite)?
    };

    let mut tree = Vec::new();
    for root in roots {
        let node = build_tree_node(pool, root, ns).await?;
        tree.push(node);
    }

    Ok(tree)
}

async fn build_tree_node(
    pool: &SqlitePool,
    agent: Agent,
    namespace: &str,
) -> Result<TreeNode, NousError> {
    let children_agents = list_children(pool, &agent.id, Some(namespace)).await?;
    let mut children = Vec::new();

    for child in children_agents {
        let node = Box::pin(build_tree_node(pool, child, namespace)).await?;
        children.push(node);
    }

    Ok(TreeNode { agent, children })
}

// --- Search ---

pub async fn search_agents(
    pool: &SqlitePool,
    query: &str,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Agent>, NousError> {
    if query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let sql = if let Some(ns) = namespace {
        let rows = sqlx::query(
            "SELECT a.* FROM agents a \
             INNER JOIN agents_fts f ON f.rowid = a.rowid \
             WHERE agents_fts MATCH ? AND a.namespace = ? \
             LIMIT ?",
        )
        .bind(query)
        .bind(ns)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows
    } else {
        let rows = sqlx::query(
            "SELECT a.* FROM agents a \
             INNER JOIN agents_fts f ON f.rowid = a.rowid \
             WHERE agents_fts MATCH ? \
             LIMIT ?",
        )
        .bind(query)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows
    };

    sql.iter()
        .map(Agent::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

// --- Artifact operations (delegating to resources module) ---

pub async fn register_artifact(
    pool: &SqlitePool,
    req: RegisterArtifactRequest,
) -> Result<Artifact, NousError> {
    let resource_type: crate::resources::ResourceType = req.artifact_type.as_str().parse()?;
    let resource = crate::resources::register_resource(
        pool,
        crate::resources::RegisterResourceRequest {
            name: req.name,
            resource_type,
            owner_agent_id: Some(req.agent_id.clone()),
            namespace: req.namespace,
            path: req.path,
            metadata: None,
            tags: None,
            ownership_policy: Some(resource_type.default_ownership_policy()),
        },
    )
    .await?;
    Ok(Artifact::from_resource(&resource))
}

pub async fn get_artifact_by_id(pool: &SqlitePool, id: &str) -> Result<Artifact, NousError> {
    let resource = crate::resources::get_resource_by_id(pool, id).await?;
    Ok(Artifact::from_resource(&resource))
}

pub async fn list_artifacts(
    pool: &SqlitePool,
    filter: &ListArtifactsFilter,
) -> Result<Vec<Artifact>, NousError> {
    let resource_type = filter
        .artifact_type
        .as_ref()
        .map(|t| t.as_str().parse::<crate::resources::ResourceType>())
        .transpose()?;
    let status = filter
        .status
        .as_ref()
        .map(|s| s.as_str().parse::<crate::resources::ResourceStatus>())
        .transpose()?;

    let resources = crate::resources::list_resources(
        pool,
        &crate::resources::ListResourcesFilter {
            resource_type,
            status,
            owner_agent_id: filter.agent_id.clone(),
            namespace: filter.namespace.clone(),
            limit: filter.limit,
            offset: filter.offset,
            ..Default::default()
        },
    )
    .await?;
    Ok(resources.iter().map(Artifact::from_resource).collect())
}

pub async fn deregister_artifact(pool: &SqlitePool, id: &str) -> Result<(), NousError> {
    crate::resources::deregister_resource(pool, id, true).await
}

pub async fn update_artifact(
    pool: &SqlitePool,
    id: &str,
    name: Option<&str>,
    path: Option<&str>,
) -> Result<Artifact, NousError> {
    let resource = crate::resources::update_resource(
        pool,
        crate::resources::UpdateResourceRequest {
            id: id.to_string(),
            name: name.map(String::from),
            path: path.map(String::from),
            metadata: None,
            tags: None,
            status: None,
            ownership_policy: None,
        },
    )
    .await?;
    Ok(Artifact::from_resource(&resource))
}

// --- P7: Agent lifecycle, versioning, templates ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentVersion {
    pub id: String,
    pub agent_id: String,
    pub skill_hash: String,
    pub config_hash: String,
    pub skills_json: String,
    pub created_at: String,
}

impl AgentVersion {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            skill_hash: row.try_get("skill_hash")?,
            config_hash: row.try_get("config_hash")?,
            skills_json: row.try_get("skills_json")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub template_type: String,
    pub default_config: String,
    pub skill_refs: String,
    pub created_at: String,
    pub updated_at: String,
}

impl AgentTemplate {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            template_type: row.try_get("template_type")?,
            default_config: row.try_get("default_config")?,
            skill_refs: row.try_get("skill_refs")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInspection {
    #[serde(flatten)]
    pub agent: Agent,
    pub current_version: Option<AgentVersion>,
    pub template: Option<AgentTemplate>,
    pub version_count: i64,
    pub active_process: Option<processes::Process>,
    pub recent_invocations: Vec<processes::Invocation>,
}

#[derive(Debug, Clone)]
pub struct RecordVersionRequest {
    pub agent_id: String,
    pub skill_hash: String,
    pub config_hash: String,
    pub skills_json: Option<String>,
}

pub async fn record_version(
    pool: &SqlitePool,
    req: RecordVersionRequest,
) -> Result<AgentVersion, NousError> {
    let _agent = get_agent_by_id(pool, &req.agent_id).await?;
    let id = Uuid::now_v7().to_string();
    let skills = req.skills_json.unwrap_or_else(|| "[]".to_string());

    sqlx::query(
        "INSERT INTO agent_versions (id, agent_id, skill_hash, config_hash, skills_json) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.agent_id)
    .bind(&req.skill_hash)
    .bind(&req.config_hash)
    .bind(&skills)
    .execute(pool)
    .await?;

    sqlx::query("UPDATE agents SET current_version_id = ?, upgrade_available = 0 WHERE id = ?")
        .bind(&id)
        .bind(&req.agent_id)
        .execute(pool)
        .await?;

    get_version_by_id(pool, &id).await
}

pub async fn get_version_by_id(pool: &SqlitePool, id: &str) -> Result<AgentVersion, NousError> {
    let row = sqlx::query("SELECT * FROM agent_versions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("agent version '{id}' not found")))?;
    AgentVersion::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_versions(
    pool: &SqlitePool,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<AgentVersion>, NousError> {
    let _agent = get_agent_by_id(pool, agent_id).await?;
    let limit = limit.unwrap_or(20).min(100);

    let rows = sqlx::query(
        "SELECT * FROM agent_versions WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(agent_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(AgentVersion::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn inspect_agent(pool: &SqlitePool, id: &str) -> Result<AgentInspection, NousError> {
    let agent = get_agent_by_id(pool, id).await?;

    let current_version: Option<AgentVersion> = {
        let row = sqlx::query(
            "SELECT v.* FROM agent_versions v \
             INNER JOIN agents a ON a.current_version_id = v.id \
             WHERE a.id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        row.map(|r| AgentVersion::from_row(&r))
            .transpose()
            .map_err(NousError::Sqlite)?
    };

    let template: Option<AgentTemplate> = {
        let row = sqlx::query(
            "SELECT t.* FROM agent_templates t \
             INNER JOIN agents a ON a.template_id = t.id \
             WHERE a.id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;
        row.map(|r| AgentTemplate::from_row(&r))
            .transpose()
            .map_err(NousError::Sqlite)?
    };

    let version_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_versions WHERE agent_id = ?")
            .bind(id)
            .fetch_one(pool)
            .await?;

    let active_process = processes::get_active_process(pool, id).await?;
    let recent_invocations = processes::list_invocations(pool, id, None, Some(5)).await?;

    Ok(AgentInspection {
        agent,
        current_version,
        template,
        version_count,
        active_process,
        recent_invocations,
    })
}

pub async fn rollback_agent(
    pool: &SqlitePool,
    agent_id: &str,
    version_id: &str,
) -> Result<AgentVersion, NousError> {
    let version = get_version_by_id(pool, version_id).await?;
    if version.agent_id != agent_id {
        return Err(NousError::Validation(
            "version does not belong to this agent".into(),
        ));
    }

    sqlx::query("UPDATE agents SET current_version_id = ?, upgrade_available = 0 WHERE id = ?")
        .bind(version_id)
        .bind(agent_id)
        .execute(pool)
        .await?;

    Ok(version)
}

pub async fn set_upgrade_available(
    pool: &SqlitePool,
    agent_id: &str,
    available: bool,
) -> Result<(), NousError> {
    let flag = if available { 1 } else { 0 };
    let result = sqlx::query("UPDATE agents SET upgrade_available = ? WHERE id = ?")
        .bind(flag)
        .bind(agent_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("agent '{agent_id}' not found")));
    }
    Ok(())
}

pub async fn list_outdated_agents(
    pool: &SqlitePool,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Agent>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    if let Some(ns) = namespace {
        let rows = sqlx::query(
            "SELECT * FROM agents WHERE upgrade_available = 1 AND namespace = ? \
             ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(ns)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows.iter()
            .map(Agent::from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(NousError::Sqlite)
    } else {
        let rows = sqlx::query(
            "SELECT * FROM agents WHERE upgrade_available = 1 \
             ORDER BY updated_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows.iter()
            .map(Agent::from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(NousError::Sqlite)
    }
}

// --- Template operations ---

#[derive(Debug, Clone)]
pub struct CreateTemplateRequest {
    pub name: String,
    pub template_type: String,
    pub default_config: Option<String>,
    pub skill_refs: Option<String>,
}

pub async fn create_template(
    pool: &SqlitePool,
    req: CreateTemplateRequest,
) -> Result<AgentTemplate, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation(
            "template name cannot be empty".into(),
        ));
    }

    let id = Uuid::now_v7().to_string();
    let config = req.default_config.unwrap_or_else(|| "{}".to_string());
    let skills = req.skill_refs.unwrap_or_else(|| "[]".to_string());

    sqlx::query(
        "INSERT INTO agent_templates (id, name, template_type, default_config, skill_refs) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(req.name.trim())
    .bind(&req.template_type)
    .bind(&config)
    .bind(&skills)
    .execute(pool)
    .await?;

    get_template_by_id(pool, &id).await
}

pub async fn get_template_by_id(pool: &SqlitePool, id: &str) -> Result<AgentTemplate, NousError> {
    let row = sqlx::query("SELECT * FROM agent_templates WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("agent template '{id}' not found")))?;
    AgentTemplate::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_templates(
    pool: &SqlitePool,
    template_type: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<AgentTemplate>, NousError> {
    let limit = limit.unwrap_or(50).min(200);

    if let Some(t) = template_type {
        let rows = sqlx::query(
            "SELECT * FROM agent_templates WHERE template_type = ? \
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(t)
        .bind(limit)
        .fetch_all(pool)
        .await?;
        rows.iter()
            .map(AgentTemplate::from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(NousError::Sqlite)
    } else {
        let rows = sqlx::query("SELECT * FROM agent_templates ORDER BY created_at DESC LIMIT ?")
            .bind(limit)
            .fetch_all(pool)
            .await?;
        rows.iter()
            .map(AgentTemplate::from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(NousError::Sqlite)
    }
}

#[derive(Debug, Clone)]
pub struct InstantiateRequest {
    pub template_id: String,
    pub name: Option<String>,
    pub namespace: Option<String>,
    pub parent_id: Option<String>,
    pub config_overrides: Option<String>,
}

pub async fn instantiate_from_template(
    pool: &SqlitePool,
    req: InstantiateRequest,
) -> Result<Agent, NousError> {
    let template = get_template_by_id(pool, &req.template_id).await?;

    let agent_type: AgentType = template.template_type.parse()?;
    let name = req
        .name
        .unwrap_or_else(|| format!("{}-{}", template.name, &Uuid::now_v7().to_string()[..8]));

    let agent = register_agent(
        pool,
        RegisterAgentRequest {
            name,
            agent_type,
            parent_id: req.parent_id,
            namespace: req.namespace,
            room: None,
            metadata: Some(merge_config(
                &template.default_config,
                req.config_overrides.as_deref(),
            )),
            status: Some(AgentStatus::Active),
        },
    )
    .await?;

    sqlx::query("UPDATE agents SET template_id = ? WHERE id = ?")
        .bind(&req.template_id)
        .bind(&agent.id)
        .execute(pool)
        .await?;

    // Copy spawn config from template's default_config if present
    if let Ok(config_val) = serde_json::from_str::<serde_json::Value>(&template.default_config) {
        let process_type = config_val.get("process_type").and_then(|v| v.as_str());
        let spawn_command = config_val.get("spawn_command").and_then(|v| v.as_str());
        let working_dir = config_val.get("working_dir").and_then(|v| v.as_str());
        let auto_restart = config_val.get("auto_restart").and_then(|v| v.as_bool());

        if process_type.is_some()
            || spawn_command.is_some()
            || working_dir.is_some()
            || auto_restart.is_some()
        {
            processes::update_agent(
                pool,
                &agent.id,
                process_type,
                spawn_command,
                working_dir,
                auto_restart,
                None,
            )
            .await?;
        }
    }

    get_agent_by_id(pool, &agent.id).await
}

fn merge_config(base: &str, overrides: Option<&str>) -> String {
    let Some(overrides) = overrides else {
        return base.to_string();
    };

    let Ok(mut base_val) = serde_json::from_str::<serde_json::Value>(base) else {
        return base.to_string();
    };

    let Ok(over_val) = serde_json::from_str::<serde_json::Value>(overrides) else {
        return base.to_string();
    };

    if let (Some(base_obj), Some(over_obj)) = (base_val.as_object_mut(), over_val.as_object()) {
        for (k, v) in over_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }

    serde_json::to_string(&base_val).unwrap_or_else(|_| base.to_string())
}

pub async fn list_stale_agents(
    pool: &SqlitePool,
    threshold_secs: u64,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(threshold_secs as i64);
    let cutoff_str = cutoff.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    // Skip agents with running processes — they are alive even if heartbeat is stale
    let rows = sqlx::query(
        "SELECT a.* FROM agents a WHERE a.namespace = ? AND a.last_seen_at IS NOT NULL \
         AND a.last_seen_at < ? AND a.status NOT IN ('archived', 'inactive', 'done') \
         AND NOT EXISTS (SELECT 1 FROM agent_processes p WHERE p.agent_id = a.id AND p.status IN ('running','starting')) \
         ORDER BY a.last_seen_at",
    )
    .bind(ns)
    .bind(&cutoff_str)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(Agent::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}
