pub mod coordination;
pub mod definition;
pub mod processes;
pub mod sandbox;

use sea_orm::entity::prelude::*;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, NotSet, QueryOrder, QuerySelect, Set, Statement,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::agent_relationships as rel_entity;
use crate::entities::agent_templates as tmpl_entity;
use crate::entities::agent_versions as ver_entity;
use crate::entities::agents as agent_entity;
use crate::error::NousError;
use crate::fts::sanitize_fts5_query;
use crate::notifications::subscribe_to_room;
use crate::rooms;

// --- Types ---

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
                "invalid agent status: '{other}'. Valid values: active, inactive, archived, running, idle, blocked, done"
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
    pub(crate) fn from_model(m: agent_entity::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            agent_type: m.agent_type,
            parent_agent_id: m.parent_agent_id,
            namespace: m.namespace,
            status: m.status,
            room: m.room,
            last_seen_at: m.last_seen_at,
            metadata_json: m.metadata_json,
            current_version_id: m.current_version_id,
            upgrade_available: m.upgrade_available,
            template_id: m.template_id,
            process_type: m.process_type,
            spawn_command: m.spawn_command,
            working_dir: m.working_dir,
            auto_restart: m.auto_restart,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }

    fn from_query_result(row: &sea_orm::QueryResult) -> Result<Self, sea_orm::DbErr> {
        Ok(Self {
            id: row.try_get_by("id")?,
            name: row.try_get_by("name")?,
            agent_type: row.try_get_by("agent_type")?,
            parent_agent_id: row.try_get_by("parent_agent_id")?,
            namespace: row.try_get_by("namespace")?,
            status: row.try_get_by("status")?,
            room: row.try_get_by("room")?,
            last_seen_at: row.try_get_by("last_seen_at")?,
            metadata_json: row.try_get_by("metadata_json")?,
            current_version_id: row.try_get_by("current_version_id")?,
            upgrade_available: row.try_get_by("upgrade_available")?,
            template_id: row.try_get_by("template_id")?,
            process_type: row.try_get_by("process_type")?,
            spawn_command: row.try_get_by("spawn_command")?,
            working_dir: row.try_get_by("working_dir")?,
            auto_restart: row.try_get_by("auto_restart")?,
            created_at: row.try_get_by("created_at")?,
            updated_at: row.try_get_by("updated_at")?,
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
    pub parent_id: Option<String>,
    pub agent_type: Option<String>,
    pub namespace: Option<String>,
    pub room: Option<String>,
    pub metadata: Option<String>,
    pub status: Option<AgentStatus>,
}

#[derive(Debug, Clone, Default)]
pub struct ListAgentsFilter {
    pub namespace: Option<String>,
    pub status: Option<AgentStatus>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// --- Agent operations ---

pub async fn register_agent(
    db: &DatabaseConnection,
    req: RegisterAgentRequest,
) -> Result<Agent, NousError> {
    if req.name.trim().is_empty() {
        return Err(NousError::Validation("agent name cannot be empty".into()));
    }

    let namespace = req.namespace.unwrap_or_else(|| "default".to_string());
    let agent_type = req.agent_type.unwrap_or_else(|| "engineer".to_string());
    let status = req.status.unwrap_or(AgentStatus::Active);
    let id = Uuid::now_v7().to_string();

    if let Some(ref parent_id) = req.parent_id {
        let parent = get_agent_by_id(db, parent_id).await?;
        if parent.namespace != namespace {
            return Err(NousError::Validation(
                "parent agent must be in the same namespace".into(),
            ));
        }
    }

    let model = agent_entity::ActiveModel {
        id: Set(id.clone()),
        name: Set(req.name.trim().to_string()),
        agent_type: Set(agent_type),
        parent_agent_id: Set(req.parent_id.clone()),
        namespace: Set(namespace.clone()),
        status: Set(status.as_str().to_string()),
        room: Set(req.room.clone()),
        last_seen_at: Set(None),
        metadata_json: Set(req.metadata.clone()),
        current_version_id: Set(None),
        upgrade_available: Set(false),
        template_id: Set(None),
        process_type: Set(None),
        spawn_command: Set(None),
        working_dir: Set(None),
        auto_restart: Set(false),
        created_at: NotSet,
        updated_at: NotSet,
    };

    agent_entity::Entity::insert(model).exec(db).await?;

    if let Some(ref parent_id) = req.parent_id {
        let rel_model = rel_entity::ActiveModel {
            parent_id: Set(parent_id.clone()),
            child_id: Set(id.clone()),
            relationship_type: Set("parent-child".to_string()),
            namespace: Set(namespace.clone()),
            created_at: NotSet,
        };

        rel_entity::Entity::insert(rel_model).exec(db).await?;

        let parent = get_agent_by_id(db, parent_id).await?;
        let coord_room_name = format!("coord-{}-{}", namespace, parent.name);
        match rooms::create_room(
            db,
            &coord_room_name,
            Some(&format!("Coordination room for {}", parent.name)),
            None,
        )
        .await
        {
            Ok(room) => {
                subscribe_to_room(db, &room.id, parent_id, None).await?;
                subscribe_to_room(db, &room.id, &id, None).await?;
            }
            Err(NousError::Conflict(_)) => {
                let room = rooms::get_room(db, &coord_room_name).await?;
                subscribe_to_room(db, &room.id, &id, None).await?;
            }
            Err(e) => return Err(e),
        }
    }

    get_agent_by_id(db, &id).await
}

pub async fn get_agent_by_id(db: &DatabaseConnection, id: &str) -> Result<Agent, NousError> {
    let model = agent_entity::Entity::find_by_id(id).one(db).await?;

    let model = model.ok_or_else(|| NousError::NotFound(format!("agent '{id}' not found")))?;
    Ok(Agent::from_model(model))
}

pub async fn lookup_agent(
    db: &DatabaseConnection,
    name: &str,
    namespace: Option<&str>,
) -> Result<Agent, NousError> {
    let ns = namespace.unwrap_or("default");
    let model = agent_entity::Entity::find()
        .filter(agent_entity::Column::Name.eq(name))
        .filter(agent_entity::Column::Namespace.eq(ns))
        .one(db)
        .await?;

    let model = model.ok_or_else(|| {
        NousError::NotFound(format!("agent '{name}' not found in namespace '{ns}'"))
    })?;
    Ok(Agent::from_model(model))
}

pub async fn list_agents(
    db: &DatabaseConnection,
    filter: &ListAgentsFilter,
) -> Result<Vec<Agent>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200) as u64;
    let offset = filter.offset.unwrap_or(0) as u64;

    let mut query = agent_entity::Entity::find();

    if let Some(ref ns) = filter.namespace {
        query = query.filter(agent_entity::Column::Namespace.eq(ns.as_str()));
    }

    if let Some(ref s) = filter.status {
        query = query.filter(agent_entity::Column::Status.eq(s.as_str()));
    }

    let models = query
        .order_by_desc(agent_entity::Column::CreatedAt)
        .limit(limit)
        .offset(offset)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Agent::from_model).collect())
}

pub fn deregister_agent<'a>(
    db: &'a DatabaseConnection,
    id: &'a str,
    cascade: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, NousError>> + Send + 'a>> {
    let id = id.to_string();
    Box::pin(async move {
        let _agent = get_agent_by_id(db, &id).await?;

        let children: Vec<Agent> = {
            let rows = db
                .query_all(Statement::from_sql_and_values(
                    sea_orm::DbBackend::Sqlite,
                    "SELECT a.* FROM agents a \
                     INNER JOIN agent_relationships r ON r.child_id = a.id \
                     WHERE r.parent_id = ? AND r.namespace = a.namespace",
                    [id.clone().into()],
                ))
                .await?;

            rows.iter()
                .map(Agent::from_query_result)
                .collect::<Result<Vec<_>, _>>()?
        };

        if !children.is_empty() && !cascade {
            return Err(NousError::Conflict(format!(
                "agent '{id}' has {} children; use cascade=true to delete",
                children.len()
            )));
        }

        if cascade {
            for child in &children {
                deregister_agent(db, &child.id, true).await?;
            }
        }

        // Handle resource ownership policies before deleting
        crate::resources::handle_agent_deregistration(db, &id).await?;

        // Clean up process records before deleting (NO CASCADE on agent_processes FK)
        processes::cleanup_agent_processes(db, &id).await?;

        agent_entity::Entity::delete_by_id(&id).exec(db).await?;

        if cascade && !children.is_empty() {
            Ok("cascaded".to_string())
        } else {
            Ok("deleted".to_string())
        }
    })
}

pub async fn update_agent_status(
    db: &DatabaseConnection,
    id: &str,
    status: AgentStatus,
) -> Result<Agent, NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE agents SET status = ? WHERE id = ?",
            [status.as_str().into(), id.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("agent '{id}' not found")));
    }

    get_agent_by_id(db, id).await
}

pub async fn heartbeat(
    db: &DatabaseConnection,
    id: &str,
    status: Option<AgentStatus>,
) -> Result<(), NousError> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let new_status = status.unwrap_or(AgentStatus::Active);

    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE agents SET last_seen_at = ?, status = ? WHERE id = ?",
            [now.into(), new_status.as_str().into(), id.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("agent '{id}' not found")));
    }

    Ok(())
}

// --- Tree traversal ---

pub async fn list_children(
    db: &DatabaseConnection,
    parent_id: &str,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");
    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT a.* FROM agents a \
             INNER JOIN agent_relationships r ON r.child_id = a.id \
             WHERE r.parent_id = ? AND r.namespace = ? \
             ORDER BY a.created_at",
            [parent_id.into(), ns.into()],
        ))
        .await?;

    rows.iter()
        .map(Agent::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn list_ancestors(
    db: &DatabaseConnection,
    agent_id: &str,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "WITH RECURSIVE ancestor_chain(agent_id, depth) AS ( \
               SELECT r.parent_id, 1 FROM agent_relationships r \
               WHERE r.child_id = ? AND r.namespace = ? \
             UNION ALL \
               SELECT r.parent_id, ac.depth + 1 FROM agent_relationships r \
               INNER JOIN ancestor_chain ac ON ac.agent_id = r.child_id \
               WHERE r.namespace = ? \
             ) \
             SELECT a.* FROM ancestor_chain ac \
             INNER JOIN agents a ON a.id = ac.agent_id \
             ORDER BY ac.depth DESC",
            [agent_id.into(), ns.into(), ns.into()],
        ))
        .await?;

    rows.iter()
        .map(Agent::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn get_tree(
    db: &DatabaseConnection,
    root_id: Option<&str>,
    namespace: Option<&str>,
) -> Result<Vec<TreeNode>, NousError> {
    let ns = namespace.unwrap_or("default");

    let all_agents: Vec<Agent> = if let Some(id) = root_id {
        let rows = db
            .query_all(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "WITH RECURSIVE subtree(agent_id) AS ( \
                   SELECT ? \
                 UNION ALL \
                   SELECT r.child_id FROM agent_relationships r \
                   INNER JOIN subtree s ON s.agent_id = r.parent_id \
                   WHERE r.namespace = ? \
                 ) \
                 SELECT a.* FROM subtree s \
                 INNER JOIN agents a ON a.id = s.agent_id \
                 ORDER BY a.created_at",
                [id.into(), ns.into()],
            ))
            .await?;

        rows.iter()
            .map(Agent::from_query_result)
            .collect::<Result<Vec<_>, _>>()
            .map_err(NousError::SeaOrm)?
    } else {
        let models = agent_entity::Entity::find()
            .filter(agent_entity::Column::Namespace.eq(ns))
            .order_by_asc(agent_entity::Column::CreatedAt)
            .all(db)
            .await?;

        models.into_iter().map(Agent::from_model).collect()
    };

    let mut children_map: std::collections::HashMap<String, Vec<Agent>> =
        std::collections::HashMap::new();
    let mut roots: Vec<Agent> = Vec::new();

    for agent in all_agents {
        match (&root_id, &agent.parent_agent_id) {
            (Some(id), _) if agent.id == *id => roots.push(agent),
            (None, None) => roots.push(agent),
            _ => {
                if let Some(ref pid) = agent.parent_agent_id {
                    children_map.entry(pid.clone()).or_default().push(agent);
                }
            }
        }
    }

    fn build_node(
        agent: Agent,
        children_map: &mut std::collections::HashMap<String, Vec<Agent>>,
    ) -> TreeNode {
        let child_agents = children_map.remove(&agent.id).unwrap_or_default();
        let children = child_agents
            .into_iter()
            .map(|child| build_node(child, children_map))
            .collect();
        TreeNode { agent, children }
    }

    Ok(roots
        .into_iter()
        .map(|root| build_node(root, &mut children_map))
        .collect())
}

// --- Search ---

pub async fn search_agents(
    db: &DatabaseConnection,
    query: &str,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Agent>, NousError> {
    if query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);
    let sanitized = sanitize_fts5_query(query);

    let rows = if let Some(ns) = namespace {
        db.query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT a.* FROM agents a \
             INNER JOIN agents_fts f ON f.rowid = a.rowid \
             WHERE agents_fts MATCH ? AND a.namespace = ? \
             LIMIT ?",
            [sanitized.clone().into(), ns.into(), (limit as i32).into()],
        ))
        .await?
    } else {
        db.query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT a.* FROM agents a \
             INNER JOIN agents_fts f ON f.rowid = a.rowid \
             WHERE agents_fts MATCH ? \
             LIMIT ?",
            [sanitized.into(), (limit as i32).into()],
        ))
        .await?
    };

    rows.iter()
        .map(Agent::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
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
    fn from_model(m: ver_entity::Model) -> Self {
        Self {
            id: m.id,
            agent_id: m.agent_id,
            skill_hash: m.skill_hash,
            config_hash: m.config_hash,
            skills_json: m.skills_json,
            created_at: m.created_at,
        }
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
    fn from_model(m: tmpl_entity::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            template_type: m.template_type,
            default_config: m.default_config,
            skill_refs: m.skill_refs,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
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
    db: &DatabaseConnection,
    req: RecordVersionRequest,
) -> Result<AgentVersion, NousError> {
    let _agent = get_agent_by_id(db, &req.agent_id).await?;
    let id = Uuid::now_v7().to_string();
    let skills = req.skills_json.unwrap_or_else(|| "[]".to_string());

    let model = ver_entity::ActiveModel {
        id: Set(id.clone()),
        agent_id: Set(req.agent_id.clone()),
        skill_hash: Set(req.skill_hash),
        config_hash: Set(req.config_hash),
        skills_json: Set(skills),
        created_at: NotSet,
    };

    ver_entity::Entity::insert(model).exec(db).await?;

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE agents SET current_version_id = ?, upgrade_available = 0 WHERE id = ?",
        [id.clone().into(), req.agent_id.into()],
    ))
    .await?;

    get_version_by_id(db, &id).await
}

pub async fn get_version_by_id(
    db: &DatabaseConnection,
    id: &str,
) -> Result<AgentVersion, NousError> {
    let model = ver_entity::Entity::find_by_id(id).one(db).await?;

    let model =
        model.ok_or_else(|| NousError::NotFound(format!("agent version '{id}' not found")))?;
    Ok(AgentVersion::from_model(model))
}

pub async fn list_versions(
    db: &DatabaseConnection,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<AgentVersion>, NousError> {
    let _agent = get_agent_by_id(db, agent_id).await?;
    let limit = limit.unwrap_or(20).min(100) as u64;

    let models = ver_entity::Entity::find()
        .filter(ver_entity::Column::AgentId.eq(agent_id))
        .order_by_desc(ver_entity::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;

    Ok(models.into_iter().map(AgentVersion::from_model).collect())
}

pub async fn inspect_agent(
    db: &DatabaseConnection,
    id: &str,
) -> Result<AgentInspection, NousError> {
    let agent = get_agent_by_id(db, id).await?;

    let current_version: Option<AgentVersion> = {
        let row = db
            .query_one(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT v.* FROM agent_versions v \
                 INNER JOIN agents a ON a.current_version_id = v.id \
                 WHERE a.id = ?",
                [id.into()],
            ))
            .await?;
        row.map(|r| {
            Ok::<_, sea_orm::DbErr>(AgentVersion {
                id: r.try_get_by("id")?,
                agent_id: r.try_get_by("agent_id")?,
                skill_hash: r.try_get_by("skill_hash")?,
                config_hash: r.try_get_by("config_hash")?,
                skills_json: r.try_get_by("skills_json")?,
                created_at: r.try_get_by("created_at")?,
            })
        })
        .transpose()?
    };

    let template: Option<AgentTemplate> = {
        let row = db
            .query_one(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT t.* FROM agent_templates t \
                 INNER JOIN agents a ON a.template_id = t.id \
                 WHERE a.id = ?",
                [id.into()],
            ))
            .await?;
        row.map(|r| {
            Ok::<_, sea_orm::DbErr>(AgentTemplate {
                id: r.try_get_by("id")?,
                name: r.try_get_by("name")?,
                template_type: r.try_get_by("template_type")?,
                default_config: r.try_get_by("default_config")?,
                skill_refs: r.try_get_by("skill_refs")?,
                created_at: r.try_get_by("created_at")?,
                updated_at: r.try_get_by("updated_at")?,
            })
        })
        .transpose()?
    };

    let version_count = ver_entity::Entity::find()
        .filter(ver_entity::Column::AgentId.eq(id))
        .count(db)
        .await? as i64;

    let active_process = processes::get_active_process(db, id).await?;
    let recent_invocations = processes::list_invocations(db, id, None, Some(5)).await?;

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
    db: &DatabaseConnection,
    agent_id: &str,
    version_id: &str,
) -> Result<AgentVersion, NousError> {
    let version = get_version_by_id(db, version_id).await?;
    if version.agent_id != agent_id {
        return Err(NousError::Validation(
            "version does not belong to this agent".into(),
        ));
    }

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE agents SET current_version_id = ?, upgrade_available = 0 WHERE id = ?",
        [version_id.into(), agent_id.into()],
    ))
    .await?;

    Ok(version)
}

pub async fn set_upgrade_available(
    db: &DatabaseConnection,
    agent_id: &str,
    available: bool,
) -> Result<(), NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE agents SET upgrade_available = ? WHERE id = ?",
            [available.into(), agent_id.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("agent '{agent_id}' not found")));
    }
    Ok(())
}

pub async fn list_outdated_agents(
    db: &DatabaseConnection,
    namespace: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Agent>, NousError> {
    let limit = limit.unwrap_or(50).min(200) as u64;

    let mut query =
        agent_entity::Entity::find().filter(agent_entity::Column::UpgradeAvailable.eq(true));

    if let Some(ns) = namespace {
        query = query.filter(agent_entity::Column::Namespace.eq(ns));
    }

    let models = query
        .order_by_desc(agent_entity::Column::UpdatedAt)
        .limit(limit)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Agent::from_model).collect())
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
    db: &DatabaseConnection,
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

    let model = tmpl_entity::ActiveModel {
        id: Set(id.clone()),
        name: Set(req.name.trim().to_string()),
        template_type: Set(req.template_type),
        default_config: Set(config),
        skill_refs: Set(skills),
        created_at: NotSet,
        updated_at: NotSet,
    };

    tmpl_entity::Entity::insert(model).exec(db).await?;

    get_template_by_id(db, &id).await
}

pub async fn get_template_by_id(
    db: &DatabaseConnection,
    id: &str,
) -> Result<AgentTemplate, NousError> {
    let model = tmpl_entity::Entity::find_by_id(id).one(db).await?;

    let model =
        model.ok_or_else(|| NousError::NotFound(format!("agent template '{id}' not found")))?;
    Ok(AgentTemplate::from_model(model))
}

pub async fn list_templates(
    db: &DatabaseConnection,
    template_type: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<AgentTemplate>, NousError> {
    let limit = limit.unwrap_or(50).min(200) as u64;

    let mut query = tmpl_entity::Entity::find();

    if let Some(t) = template_type {
        query = query.filter(tmpl_entity::Column::TemplateType.eq(t));
    }

    let models = query
        .order_by_desc(tmpl_entity::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;

    Ok(models.into_iter().map(AgentTemplate::from_model).collect())
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
    db: &DatabaseConnection,
    req: InstantiateRequest,
) -> Result<Agent, NousError> {
    let template = get_template_by_id(db, &req.template_id).await?;

    let name = req
        .name
        .unwrap_or_else(|| format!("{}-{}", template.name, &Uuid::now_v7().to_string()[..8]));

    let agent = register_agent(
        db,
        RegisterAgentRequest {
            name,
            agent_type: None,
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

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE agents SET template_id = ? WHERE id = ?",
        [req.template_id.into(), agent.id.clone().into()],
    ))
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
                db,
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

    get_agent_by_id(db, &agent.id).await
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
    db: &DatabaseConnection,
    threshold_secs: u64,
    namespace: Option<&str>,
) -> Result<Vec<Agent>, NousError> {
    let ns = namespace.unwrap_or("default");
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(threshold_secs as i64);
    let cutoff_str = cutoff.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();

    // Skip agents with running processes — they are alive even if heartbeat is stale
    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT a.* FROM agents a WHERE a.namespace = ? AND a.last_seen_at IS NOT NULL \
             AND a.last_seen_at < ? AND a.status NOT IN ('archived', 'inactive', 'done') \
             AND NOT EXISTS (SELECT 1 FROM agent_processes p WHERE p.agent_id = a.id AND p.status IN ('running','starting')) \
             ORDER BY a.last_seen_at",
            [ns.into(), cutoff_str.into()],
        ))
        .await?;

    rows.iter()
        .map(Agent::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}
