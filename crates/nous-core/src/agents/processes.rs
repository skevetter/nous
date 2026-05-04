use sea_orm::entity::prelude::*;
use sea_orm::{
    ConnectionTrait, DatabaseConnection, NotSet, QueryOrder, QuerySelect, Set, Statement,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::agent_invocations as inv_entity;
use crate::entities::agent_processes as proc_entity;
use crate::error::NousError;

// --- Domain objects ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Process {
    pub id: String,
    pub agent_id: String,
    pub process_type: String,
    pub command: String,
    pub working_dir: Option<String>,
    pub env_json: Option<String>,
    pub pid: Option<i64>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
    pub last_output: Option<String>,
    pub max_output_bytes: i64,
    pub timeout_secs: Option<i64>,
    pub sandbox_image: Option<String>,
    pub sandbox_cpus: Option<i64>,
    pub sandbox_memory_mib: Option<i64>,
    pub sandbox_network_policy: Option<String>,
    pub sandbox_volumes_json: Option<String>,
    pub sandbox_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Process {
    pub fn from_model(m: proc_entity::Model) -> Self {
        Self {
            id: m.id,
            agent_id: m.agent_id,
            process_type: m.process_type,
            command: m.command,
            working_dir: m.working_dir,
            env_json: m.env_json,
            pid: m.pid.map(i64::from),
            status: m.status,
            exit_code: m.exit_code,
            started_at: m.started_at,
            stopped_at: m.stopped_at,
            last_output: m.last_output,
            max_output_bytes: i64::from(m.max_output_bytes),
            timeout_secs: m.timeout_secs.map(i64::from),
            sandbox_image: m.sandbox_image,
            sandbox_cpus: m.sandbox_cpus.map(i64::from),
            sandbox_memory_mib: m.sandbox_memory_mib.map(i64::from),
            sandbox_network_policy: m.sandbox_network_policy,
            sandbox_volumes_json: m.sandbox_volumes_json,
            sandbox_name: m.sandbox_name,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invocation {
    pub id: String,
    pub agent_id: String,
    pub process_id: Option<String>,
    pub prompt: String,
    pub result: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub duration_ms: Option<i64>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

impl Invocation {
    pub fn from_model(m: inv_entity::Model) -> Self {
        Self {
            id: m.id,
            agent_id: m.agent_id,
            process_id: m.process_id,
            prompt: m.prompt,
            result: m.result,
            status: m.status,
            error: m.error,
            started_at: m.started_at,
            completed_at: m.completed_at,
            duration_ms: m.duration_ms.map(i64::from),
            metadata_json: m.metadata_json,
            created_at: m.created_at,
        }
    }
}

// --- Process CRUD ---

pub struct CreateProcessParams<'a> {
    pub db: &'a DatabaseConnection,
    pub agent_id: &'a str,
    pub process_type: &'a str,
    pub command: &'a str,
    pub working_dir: Option<&'a str>,
    pub env_json: Option<&'a str>,
    pub timeout_secs: Option<i64>,
}

pub async fn create_process(params: CreateProcessParams<'_>) -> Result<Process, NousError> {
    let CreateProcessParams {
        db,
        agent_id,
        process_type,
        command,
        working_dir,
        env_json,
        timeout_secs,
    } = params;
    // Verify agent exists
    super::get_agent_by_id(db, agent_id).await?;

    let id = Uuid::now_v7().to_string();

    let model = proc_entity::ActiveModel {
        id: Set(id.clone()),
        agent_id: Set(agent_id.to_string()),
        process_type: Set(process_type.to_string()),
        command: Set(command.to_string()),
        working_dir: Set(working_dir.map(String::from)),
        env_json: Set(Some(env_json.unwrap_or("{}").to_string())),
        timeout_secs: Set(timeout_secs.map(|v| v as i32)),
        status: Set("pending".to_string()),
        pid: Set(None),
        exit_code: Set(None),
        started_at: Set(None),
        stopped_at: Set(None),
        last_output: Set(None),
        max_output_bytes: Set(1_048_576),
        sandbox_image: Set(None),
        sandbox_cpus: Set(None),
        sandbox_memory_mib: Set(None),
        sandbox_network_policy: Set(None),
        sandbox_volumes_json: Set(None),
        sandbox_name: Set(None),
        created_at: NotSet,
        updated_at: NotSet,
    };

    proc_entity::Entity::insert(model).exec(db).await?;

    get_process_by_id(db, &id).await
}

pub struct CreateSandboxProcessParams<'a> {
    pub db: &'a DatabaseConnection,
    pub agent_id: &'a str,
    pub sandbox_image: &'a str,
    pub sandbox_cpus: Option<u32>,
    pub sandbox_memory_mib: Option<u32>,
    pub sandbox_network_policy: Option<&'a str>,
    pub sandbox_volumes_json: Option<&'a str>,
    pub sandbox_name: Option<&'a str>,
    pub timeout_secs: Option<i64>,
}

pub async fn create_sandbox_process(
    params: CreateSandboxProcessParams<'_>,
) -> Result<Process, NousError> {
    let CreateSandboxProcessParams {
        db,
        agent_id,
        sandbox_image,
        sandbox_cpus,
        sandbox_memory_mib,
        sandbox_network_policy,
        sandbox_volumes_json,
        sandbox_name,
        timeout_secs,
    } = params;
    super::get_agent_by_id(db, agent_id).await?;

    let id = Uuid::now_v7().to_string();
    let name = sandbox_name.unwrap_or(agent_id);

    let model = proc_entity::ActiveModel {
        id: Set(id.clone()),
        agent_id: Set(agent_id.to_string()),
        process_type: Set("sandbox".to_string()),
        command: Set(format!("sandbox:{sandbox_image}")),
        working_dir: Set(None),
        env_json: Set(None),
        timeout_secs: Set(timeout_secs.map(|v| v as i32)),
        status: Set("pending".to_string()),
        pid: Set(None),
        exit_code: Set(None),
        started_at: Set(None),
        stopped_at: Set(None),
        last_output: Set(None),
        max_output_bytes: Set(1_048_576),
        sandbox_image: Set(Some(sandbox_image.to_string())),
        // sandbox_cpus and sandbox_memory_mib are user-supplied values bounded by
        // reasonable CPU/memory limits (well within i32 range), safe to cast.
        sandbox_cpus: Set(sandbox_cpus.map(u32::cast_signed)),
        sandbox_memory_mib: Set(sandbox_memory_mib.map(u32::cast_signed)),
        sandbox_network_policy: Set(sandbox_network_policy.map(String::from)),
        sandbox_volumes_json: Set(sandbox_volumes_json.map(String::from)),
        sandbox_name: Set(Some(name.to_string())),
        created_at: NotSet,
        updated_at: NotSet,
    };

    proc_entity::Entity::insert(model).exec(db).await?;

    get_process_by_id(db, &id).await
}

pub async fn get_process_by_id(db: &DatabaseConnection, id: &str) -> Result<Process, NousError> {
    let model = proc_entity::Entity::find_by_id(id).one(db).await?;

    let model = model.ok_or_else(|| NousError::NotFound(format!("process '{id}' not found")))?;
    Ok(Process::from_model(model))
}

pub async fn get_active_process(
    db: &DatabaseConnection,
    agent_id: &str,
) -> Result<Option<Process>, NousError> {
    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT * FROM agent_processes WHERE agent_id = ? AND status IN ('pending','starting','running','stopping') LIMIT 1",
            [agent_id.into()],
        ))
        .await?;

    if let Some(row) = rows.first() {
        let model = proc_entity::Model {
            id: row.try_get_by("id")?,
            agent_id: row.try_get_by("agent_id")?,
            process_type: row.try_get_by("process_type")?,
            command: row.try_get_by("command")?,
            working_dir: row.try_get_by("working_dir")?,
            env_json: row.try_get_by("env_json")?,
            pid: row.try_get_by("pid")?,
            status: row.try_get_by("status")?,
            exit_code: row.try_get_by("exit_code")?,
            started_at: row.try_get_by("started_at")?,
            stopped_at: row.try_get_by("stopped_at")?,
            last_output: row.try_get_by("last_output")?,
            max_output_bytes: row.try_get_by("max_output_bytes")?,
            timeout_secs: row.try_get_by("timeout_secs")?,
            sandbox_image: row.try_get_by("sandbox_image")?,
            sandbox_cpus: row.try_get_by("sandbox_cpus")?,
            sandbox_memory_mib: row.try_get_by("sandbox_memory_mib")?,
            sandbox_network_policy: row.try_get_by("sandbox_network_policy")?,
            sandbox_volumes_json: row.try_get_by("sandbox_volumes_json")?,
            sandbox_name: row.try_get_by("sandbox_name")?,
            created_at: row.try_get_by("created_at")?,
            updated_at: row.try_get_by("updated_at")?,
        };
        Ok(Some(Process::from_model(model)))
    } else {
        Ok(None)
    }
}

pub async fn get_latest_process(
    db: &DatabaseConnection,
    agent_id: &str,
) -> Result<Option<Process>, NousError> {
    let model = proc_entity::Entity::find()
        .filter(proc_entity::Column::AgentId.eq(agent_id))
        .order_by_desc(proc_entity::Column::CreatedAt)
        .one(db)
        .await?;

    Ok(model.map(Process::from_model))
}

pub struct UpdateProcessStatusRequest<'a> {
    pub process_id: &'a str,
    pub status: &'a str,
    pub exit_code: Option<i32>,
    pub output: Option<&'a str>,
    pub pid: Option<i64>,
}

fn process_status_timestamps(status: &str, now: &str) -> (Option<String>, Option<String>) {
    let stopped_at = if matches!(status, "stopped" | "failed" | "crashed") {
        Some(now.to_string())
    } else {
        None
    };
    let started_at = if status == "running" {
        Some(now.to_string())
    } else {
        None
    };
    (started_at, stopped_at)
}

pub async fn update_process_status(
    db: &DatabaseConnection,
    req: UpdateProcessStatusRequest<'_>,
) -> Result<Process, NousError> {
    let UpdateProcessStatusRequest {
        process_id,
        status,
        exit_code,
        output,
        pid,
    } = req;

    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let (started_at, stopped_at) = process_status_timestamps(status, &now);

    // Build update dynamically
    let mut sets = vec!["status = ?".to_string(), "updated_at = ?".to_string()];
    let mut bind_values: Vec<sea_orm::Value> = vec![status.into(), now.into()];

    if let Some(code) = exit_code {
        sets.push("exit_code = ?".to_string());
        bind_values.push(code.into());
    }
    if let Some(out) = output {
        sets.push("last_output = ?".to_string());
        bind_values.push(out.into());
    }
    if let Some(p) = pid {
        sets.push("pid = ?".to_string());
        bind_values.push((p as i32).into());
    }
    if let Some(sa) = started_at {
        sets.push("started_at = ?".to_string());
        bind_values.push(sa.into());
    }
    if let Some(sa) = stopped_at {
        sets.push("stopped_at = ?".to_string());
        bind_values.push(sa.into());
    }

    let sql = format!(
        "UPDATE agent_processes SET {} WHERE id = ?",
        sets.join(", ")
    );
    bind_values.push(process_id.into());

    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            &sql,
            bind_values,
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!(
            "process '{process_id}' not found"
        )));
    }

    get_process_by_id(db, process_id).await
}

pub async fn list_processes(
    db: &DatabaseConnection,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Process>, NousError> {
    let limit = u64::from(limit.unwrap_or(20).min(100));
    let models = proc_entity::Entity::find()
        .filter(proc_entity::Column::AgentId.eq(agent_id))
        .order_by_desc(proc_entity::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Process::from_model).collect())
}

pub async fn list_all_active_processes(db: &DatabaseConnection) -> Result<Vec<Process>, NousError> {
    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT * FROM agent_processes WHERE status IN ('pending','starting','running','stopping') ORDER BY created_at DESC",
            [],
        ))
        .await?;

    let mut result = Vec::new();
    for row in &rows {
        let model = proc_entity::Model {
            id: row.try_get_by("id")?,
            agent_id: row.try_get_by("agent_id")?,
            process_type: row.try_get_by("process_type")?,
            command: row.try_get_by("command")?,
            working_dir: row.try_get_by("working_dir")?,
            env_json: row.try_get_by("env_json")?,
            pid: row.try_get_by("pid")?,
            status: row.try_get_by("status")?,
            exit_code: row.try_get_by("exit_code")?,
            started_at: row.try_get_by("started_at")?,
            stopped_at: row.try_get_by("stopped_at")?,
            last_output: row.try_get_by("last_output")?,
            max_output_bytes: row.try_get_by("max_output_bytes")?,
            timeout_secs: row.try_get_by("timeout_secs")?,
            sandbox_image: row.try_get_by("sandbox_image")?,
            sandbox_cpus: row.try_get_by("sandbox_cpus")?,
            sandbox_memory_mib: row.try_get_by("sandbox_memory_mib")?,
            sandbox_network_policy: row.try_get_by("sandbox_network_policy")?,
            sandbox_volumes_json: row.try_get_by("sandbox_volumes_json")?,
            sandbox_name: row.try_get_by("sandbox_name")?,
            created_at: row.try_get_by("created_at")?,
            updated_at: row.try_get_by("updated_at")?,
        };
        result.push(Process::from_model(model));
    }

    Ok(result)
}

// --- Invocation CRUD ---

pub async fn create_invocation(
    db: &DatabaseConnection,
    agent_id: &str,
    prompt: &str,
    metadata: Option<&str>,
) -> Result<Invocation, NousError> {
    super::get_agent_by_id(db, agent_id).await?;
    let id = Uuid::now_v7().to_string();

    // Link to active process if one exists
    let active = get_active_process(db, agent_id).await?;
    let process_id = active.map(|p| p.id);

    let model = inv_entity::ActiveModel {
        id: Set(id.clone()),
        agent_id: Set(agent_id.to_string()),
        process_id: Set(process_id),
        prompt: Set(prompt.to_string()),
        result: Set(None),
        status: Set("pending".to_string()),
        error: Set(None),
        started_at: Set(None),
        completed_at: Set(None),
        duration_ms: Set(None),
        metadata_json: Set(metadata.map(String::from)),
        created_at: NotSet,
    };

    inv_entity::Entity::insert(model).exec(db).await?;

    get_invocation(db, &id).await
}

pub struct UpdateInvocationRequest<'a> {
    pub invocation_id: &'a str,
    pub status: &'a str,
    pub result: Option<&'a str>,
    pub error: Option<&'a str>,
    pub duration_ms: Option<i64>,
}

pub async fn update_invocation(
    db: &DatabaseConnection,
    req: UpdateInvocationRequest<'_>,
) -> Result<Invocation, NousError> {
    let UpdateInvocationRequest {
        invocation_id,
        status,
        result,
        error,
        duration_ms,
    } = req;
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let started_at = if status == "running" {
        Some(now.clone())
    } else {
        None
    };

    let completed_at = if matches!(status, "completed" | "failed" | "timeout" | "cancelled") {
        Some(now)
    } else {
        None
    };

    let bind_values: Vec<sea_orm::Value> = vec![
        status.into(),
        result.map(std::string::ToString::to_string).into(),
        error.map(std::string::ToString::to_string).into(),
        duration_ms.map(|v| v as i32).into(),
        started_at.into(),
        completed_at.into(),
        invocation_id.into(),
    ];

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE agent_invocations SET status = ?, result = COALESCE(?, result), error = COALESCE(?, error), \
         duration_ms = COALESCE(?, duration_ms), started_at = COALESCE(?, started_at), completed_at = COALESCE(?, completed_at) \
         WHERE id = ?",
        bind_values,
    ))
    .await?;

    get_invocation(db, invocation_id).await
}

pub async fn get_invocation(db: &DatabaseConnection, id: &str) -> Result<Invocation, NousError> {
    let model = inv_entity::Entity::find_by_id(id).one(db).await?;

    let model = model.ok_or_else(|| NousError::NotFound(format!("invocation '{id}' not found")))?;
    Ok(Invocation::from_model(model))
}

pub async fn list_invocations(
    db: &DatabaseConnection,
    agent_id: &str,
    status_filter: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Invocation>, NousError> {
    let limit = u64::from(limit.unwrap_or(20).min(100));

    let mut query = inv_entity::Entity::find().filter(inv_entity::Column::AgentId.eq(agent_id));

    if let Some(status) = status_filter {
        query = query.filter(inv_entity::Column::Status.eq(status));
    }

    let models = query
        .order_by_desc(inv_entity::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Invocation::from_model).collect())
}

pub async fn cleanup_agent_processes(
    db: &DatabaseConnection,
    agent_id: &str,
) -> Result<(), NousError> {
    // Mark any active processes as stopped
    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "UPDATE agent_processes SET status = 'stopped', stopped_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE agent_id = ? AND status IN ('pending','starting','running','stopping')",
        [agent_id.into()],
    ))
    .await?;

    // Delete all process records for this agent
    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "DELETE FROM agent_processes WHERE agent_id = ?",
        [agent_id.into()],
    ))
    .await?;

    Ok(())
}

// --- Agent update (config fields) ---

pub struct UpdateAgentRequest<'a> {
    pub id: &'a str,
    pub process_type: Option<&'a str>,
    pub spawn_command: Option<&'a str>,
    pub working_dir: Option<&'a str>,
    pub auto_restart: Option<bool>,
    pub metadata_json: Option<&'a str>,
}

pub async fn update_agent(
    db: &DatabaseConnection,
    req: UpdateAgentRequest<'_>,
) -> Result<super::Agent, NousError> {
    let UpdateAgentRequest {
        id,
        process_type,
        spawn_command,
        working_dir,
        auto_restart,
        metadata_json,
    } = req;
    let _existing = super::get_agent_by_id(db, id).await?;

    let mut sets: Vec<String> = Vec::new();
    let mut binds: Vec<sea_orm::Value> = Vec::new();

    if let Some(pt) = process_type {
        sets.push("process_type = ?".to_string());
        binds.push(pt.into());
    }
    if let Some(sc) = spawn_command {
        sets.push("spawn_command = ?".to_string());
        binds.push(sc.into());
    }
    if let Some(wd) = working_dir {
        sets.push("working_dir = ?".to_string());
        binds.push(wd.into());
    }
    if let Some(ar) = auto_restart {
        sets.push("auto_restart = ?".to_string());
        binds.push(ar.into());
    }
    if let Some(md) = metadata_json {
        sets.push("metadata_json = ?".to_string());
        binds.push(md.into());
    }

    if sets.is_empty() {
        return super::get_agent_by_id(db, id).await;
    }

    let sql = format!("UPDATE agents SET {} WHERE id = ?", sets.join(", "));
    binds.push(id.into());

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        binds,
    ))
    .await?;

    super::get_agent_by_id(db, id).await
}
