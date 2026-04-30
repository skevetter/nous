use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;

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
    pub created_at: String,
    pub updated_at: String,
}

impl Agent {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
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
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            artifact_type: row.try_get("artifact_type")?,
            name: row.try_get("name")?,
            path: row.try_get("path")?,
            status: row.try_get("status")?,
            namespace: row.try_get("namespace")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            last_seen_at: row.try_get("last_seen_at")?,
        })
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
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let new_status = status.unwrap_or(AgentStatus::Active);

    let result =
        sqlx::query("UPDATE agents SET last_seen_at = ?, status = ? WHERE id = ?")
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

// --- Artifact operations ---

pub async fn register_artifact(
    pool: &SqlitePool,
    req: RegisterArtifactRequest,
) -> Result<Artifact, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation(
            "artifact name cannot be empty".into(),
        ));
    }

    let agent = get_agent_by_id(pool, &req.agent_id).await?;
    let namespace = req.namespace.unwrap_or_else(|| agent.namespace.clone());
    if agent.namespace != namespace {
        return Err(NousError::Validation(
            "artifact namespace must match owning agent's namespace".into(),
        ));
    }
    let id = Uuid::now_v7().to_string();

    sqlx::query(
        "INSERT INTO artifacts (id, agent_id, artifact_type, name, path, namespace) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.agent_id)
    .bind(req.artifact_type.as_str())
    .bind(req.name.trim())
    .bind(&req.path)
    .bind(&namespace)
    .execute(pool)
    .await?;

    get_artifact_by_id(pool, &id).await
}

pub async fn get_artifact_by_id(pool: &SqlitePool, id: &str) -> Result<Artifact, NousError> {
    let row = sqlx::query("SELECT * FROM artifacts WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("artifact '{id}' not found")))?;
    Artifact::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_artifacts(
    pool: &SqlitePool,
    filter: &ListArtifactsFilter,
) -> Result<Vec<Artifact>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut sql = String::from("SELECT * FROM artifacts");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref agent_id) = filter.agent_id {
        conditions.push("agent_id = ?".to_string());
        binds.push(agent_id.clone());
    }

    if let Some(ref t) = filter.artifact_type {
        conditions.push("artifact_type = ?".to_string());
        binds.push(t.as_str().to_string());
    }

    if let Some(ref ns) = filter.namespace {
        conditions.push("namespace = ?".to_string());
        binds.push(ns.clone());
    }

    if let Some(ref s) = filter.status {
        conditions.push("status = ?".to_string());
        binds.push(s.as_str().to_string());
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
        .map(Artifact::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn deregister_artifact(pool: &SqlitePool, id: &str) -> Result<(), NousError> {
    let result = sqlx::query("DELETE FROM artifacts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("artifact '{id}' not found")));
    }

    Ok(())
}

pub async fn list_stale_agents(
    pool: &SqlitePool,
    threshold_secs: u64,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");
    let cutoff = chrono::Utc::now()
        - chrono::Duration::seconds(threshold_secs as i64);
    let cutoff_str = cutoff.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    let rows = sqlx::query(
        "SELECT * FROM agents WHERE namespace = ? AND last_seen_at IS NOT NULL \
         AND last_seen_at < ? AND status NOT IN ('archived', 'inactive', 'done') \
         ORDER BY last_seen_at",
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
