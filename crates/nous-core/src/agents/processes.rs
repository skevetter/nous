use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

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
    pub restart_policy: String,
    pub restart_count: i32,
    pub max_restarts: i32,
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
    pub fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            process_type: row.try_get("process_type")?,
            command: row.try_get("command")?,
            working_dir: row.try_get("working_dir")?,
            env_json: row.try_get("env_json")?,
            pid: row.try_get("pid")?,
            status: row.try_get("status")?,
            exit_code: row.try_get("exit_code")?,
            started_at: row.try_get("started_at")?,
            stopped_at: row.try_get("stopped_at")?,
            last_output: row.try_get("last_output")?,
            max_output_bytes: row.try_get("max_output_bytes")?,
            restart_policy: row.try_get("restart_policy")?,
            restart_count: row.try_get("restart_count")?,
            max_restarts: row.try_get("max_restarts")?,
            timeout_secs: row.try_get("timeout_secs")?,
            sandbox_image: row.try_get("sandbox_image")?,
            sandbox_cpus: row.try_get("sandbox_cpus")?,
            sandbox_memory_mib: row.try_get("sandbox_memory_mib")?,
            sandbox_network_policy: row.try_get("sandbox_network_policy")?,
            sandbox_volumes_json: row.try_get("sandbox_volumes_json")?,
            sandbox_name: row.try_get("sandbox_name")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
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
    pub fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            process_id: row.try_get("process_id")?,
            prompt: row.try_get("prompt")?,
            result: row.try_get("result")?,
            status: row.try_get("status")?,
            error: row.try_get("error")?,
            started_at: row.try_get("started_at")?,
            completed_at: row.try_get("completed_at")?,
            duration_ms: row.try_get("duration_ms")?,
            metadata_json: row.try_get("metadata_json")?,
            created_at: row.try_get("created_at")?,
        })
    }
}

// --- Process CRUD ---

#[allow(clippy::too_many_arguments)]
pub async fn create_process(
    pool: &SqlitePool,
    agent_id: &str,
    process_type: &str,
    command: &str,
    working_dir: Option<&str>,
    env_json: Option<&str>,
    timeout_secs: Option<i64>,
    restart_policy: Option<&str>,
    max_restarts: Option<i32>,
) -> Result<Process, NousError> {
    // Verify agent exists
    super::get_agent_by_id(pool, agent_id).await?;

    let id = Uuid::now_v7().to_string();
    let policy = restart_policy.unwrap_or("never");
    let max_r = max_restarts.unwrap_or(3);

    sqlx::query(
        "INSERT INTO agent_processes (id, agent_id, process_type, command, working_dir, env_json, timeout_secs, restart_policy, max_restarts) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(agent_id)
    .bind(process_type)
    .bind(command)
    .bind(working_dir)
    .bind(env_json.unwrap_or("{}"))
    .bind(timeout_secs)
    .bind(policy)
    .bind(max_r)
    .execute(pool)
    .await?;

    get_process_by_id(pool, &id).await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_sandbox_process(
    pool: &SqlitePool,
    agent_id: &str,
    sandbox_image: &str,
    sandbox_cpus: Option<u32>,
    sandbox_memory_mib: Option<u32>,
    sandbox_network_policy: Option<&str>,
    sandbox_volumes_json: Option<&str>,
    sandbox_name: Option<&str>,
    timeout_secs: Option<i64>,
    restart_policy: Option<&str>,
) -> Result<Process, NousError> {
    super::get_agent_by_id(pool, agent_id).await?;

    let id = Uuid::now_v7().to_string();
    let policy = restart_policy.unwrap_or("never");
    let name = sandbox_name.unwrap_or(agent_id);

    sqlx::query(
        "INSERT INTO agent_processes (id, agent_id, process_type, command, status, restart_policy, timeout_secs, \
         sandbox_image, sandbox_cpus, sandbox_memory_mib, sandbox_network_policy, sandbox_volumes_json, sandbox_name) \
         VALUES (?, ?, 'sandbox', ?, 'pending', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(agent_id)
    .bind(format!("sandbox:{sandbox_image}"))
    .bind(policy)
    .bind(timeout_secs)
    .bind(sandbox_image)
    .bind(sandbox_cpus.map(|v| v as i64))
    .bind(sandbox_memory_mib.map(|v| v as i64))
    .bind(sandbox_network_policy)
    .bind(sandbox_volumes_json)
    .bind(name)
    .execute(pool)
    .await?;

    get_process_by_id(pool, &id).await
}

pub async fn get_process_by_id(pool: &SqlitePool, id: &str) -> Result<Process, NousError> {
    let row = sqlx::query("SELECT * FROM agent_processes WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("process '{id}' not found")))?;
    Process::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn get_active_process(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<Option<Process>, NousError> {
    let row = sqlx::query(
        "SELECT * FROM agent_processes WHERE agent_id = ? AND status IN ('pending','starting','running','stopping') LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;

    row.map(|r| Process::from_row(&r))
        .transpose()
        .map_err(NousError::Sqlite)
}

pub async fn get_latest_process(
    pool: &SqlitePool,
    agent_id: &str,
) -> Result<Option<Process>, NousError> {
    let row = sqlx::query(
        "SELECT * FROM agent_processes WHERE agent_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;

    row.map(|r| Process::from_row(&r))
        .transpose()
        .map_err(NousError::Sqlite)
}

pub async fn update_process_status(
    pool: &SqlitePool,
    process_id: &str,
    status: &str,
    exit_code: Option<i32>,
    output: Option<&str>,
    pid: Option<i64>,
) -> Result<Process, NousError> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    let stopped_at = if matches!(status, "stopped" | "failed" | "crashed") {
        Some(now.clone())
    } else {
        None
    };

    let started_at = if status == "running" {
        Some(now.clone())
    } else {
        None
    };

    // Build update dynamically
    let mut sets = vec!["status = ?".to_string(), "updated_at = ?".to_string()];
    let mut bind_values: Vec<Option<String>> = vec![Some(status.to_string()), Some(now)];

    if let Some(code) = exit_code {
        sets.push("exit_code = ?".to_string());
        bind_values.push(Some(code.to_string()));
    }
    if let Some(out) = output {
        sets.push("last_output = ?".to_string());
        bind_values.push(Some(out.to_string()));
    }
    if let Some(p) = pid {
        sets.push("pid = ?".to_string());
        bind_values.push(Some(p.to_string()));
    }
    if let Some(sa) = started_at {
        sets.push("started_at = ?".to_string());
        bind_values.push(Some(sa));
    }
    if let Some(sa) = stopped_at {
        sets.push("stopped_at = ?".to_string());
        bind_values.push(Some(sa));
    }

    let sql = format!(
        "UPDATE agent_processes SET {} WHERE id = ?",
        sets.join(", ")
    );
    bind_values.push(Some(process_id.to_string()));

    let mut query = sqlx::query(&sql);
    for val in &bind_values {
        match val {
            Some(v) => query = query.bind(v),
            None => query = query.bind(Option::<String>::None),
        }
    }

    let result = query.execute(pool).await?;
    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!(
            "process '{process_id}' not found"
        )));
    }

    get_process_by_id(pool, process_id).await
}

pub async fn increment_restart_count(pool: &SqlitePool, process_id: &str) -> Result<(), NousError> {
    sqlx::query("UPDATE agent_processes SET restart_count = restart_count + 1 WHERE id = ?")
        .bind(process_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_processes(
    pool: &SqlitePool,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Process>, NousError> {
    let limit = limit.unwrap_or(20).min(100);
    let rows = sqlx::query(
        "SELECT * FROM agent_processes WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(agent_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(Process::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn list_all_active_processes(pool: &SqlitePool) -> Result<Vec<Process>, NousError> {
    let rows = sqlx::query(
        "SELECT * FROM agent_processes WHERE status IN ('pending','starting','running','stopping') ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;

    rows.iter()
        .map(Process::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

// --- Invocation CRUD ---

pub async fn create_invocation(
    pool: &SqlitePool,
    agent_id: &str,
    prompt: &str,
    metadata: Option<&str>,
) -> Result<Invocation, NousError> {
    super::get_agent_by_id(pool, agent_id).await?;
    let id = Uuid::now_v7().to_string();

    // Link to active process if one exists
    let active = get_active_process(pool, agent_id).await?;
    let process_id = active.map(|p| p.id);

    sqlx::query(
        "INSERT INTO agent_invocations (id, agent_id, process_id, prompt, metadata_json) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(agent_id)
    .bind(&process_id)
    .bind(prompt)
    .bind(metadata)
    .execute(pool)
    .await?;

    get_invocation(pool, &id).await
}

pub async fn update_invocation(
    pool: &SqlitePool,
    invocation_id: &str,
    status: &str,
    result: Option<&str>,
    error: Option<&str>,
    duration_ms: Option<i64>,
) -> Result<Invocation, NousError> {
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

    sqlx::query(
        "UPDATE agent_invocations SET status = ?, result = COALESCE(?, result), error = COALESCE(?, error), \
         duration_ms = COALESCE(?, duration_ms), started_at = COALESCE(?, started_at), completed_at = COALESCE(?, completed_at) \
         WHERE id = ?",
    )
    .bind(status)
    .bind(result)
    .bind(error)
    .bind(duration_ms)
    .bind(&started_at)
    .bind(&completed_at)
    .bind(invocation_id)
    .execute(pool)
    .await?;

    get_invocation(pool, invocation_id).await
}

pub async fn get_invocation(pool: &SqlitePool, id: &str) -> Result<Invocation, NousError> {
    let row = sqlx::query("SELECT * FROM agent_invocations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("invocation '{id}' not found")))?;
    Invocation::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_invocations(
    pool: &SqlitePool,
    agent_id: &str,
    status_filter: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<Invocation>, NousError> {
    let limit = limit.unwrap_or(20).min(100);

    let rows = if let Some(status) = status_filter {
        sqlx::query(
            "SELECT * FROM agent_invocations WHERE agent_id = ? AND status = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(agent_id)
        .bind(status)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT * FROM agent_invocations WHERE agent_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(agent_id)
        .bind(limit)
        .fetch_all(pool)
        .await?
    };

    rows.iter()
        .map(Invocation::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn cleanup_agent_processes(pool: &SqlitePool, agent_id: &str) -> Result<(), NousError> {
    // Mark any active processes as stopped
    sqlx::query(
        "UPDATE agent_processes SET status = 'stopped', stopped_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE agent_id = ? AND status IN ('pending','starting','running','stopping')",
    )
    .bind(agent_id)
    .execute(pool)
    .await?;

    // Delete all process records for this agent
    sqlx::query("DELETE FROM agent_processes WHERE agent_id = ?")
        .bind(agent_id)
        .execute(pool)
        .await?;

    Ok(())
}

// --- Agent update (config fields) ---

pub async fn update_agent(
    pool: &SqlitePool,
    id: &str,
    process_type: Option<&str>,
    spawn_command: Option<&str>,
    working_dir: Option<&str>,
    auto_restart: Option<bool>,
    metadata_json: Option<&str>,
) -> Result<super::Agent, NousError> {
    let _existing = super::get_agent_by_id(pool, id).await?;

    let mut sets: Vec<String> = Vec::new();
    let mut binds: Vec<Option<String>> = Vec::new();

    if let Some(pt) = process_type {
        sets.push("process_type = ?".to_string());
        binds.push(Some(pt.to_string()));
    }
    if let Some(sc) = spawn_command {
        sets.push("spawn_command = ?".to_string());
        binds.push(Some(sc.to_string()));
    }
    if let Some(wd) = working_dir {
        sets.push("working_dir = ?".to_string());
        binds.push(Some(wd.to_string()));
    }
    if let Some(ar) = auto_restart {
        sets.push("auto_restart = ?".to_string());
        binds.push(Some(if ar { "1" } else { "0" }.to_string()));
    }
    if let Some(md) = metadata_json {
        sets.push("metadata_json = ?".to_string());
        binds.push(Some(md.to_string()));
    }

    if sets.is_empty() {
        return super::get_agent_by_id(pool, id).await;
    }

    let sql = format!("UPDATE agents SET {} WHERE id = ?", sets.join(", "));
    binds.push(Some(id.to_string()));

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        match bind {
            Some(v) => query = query.bind(v),
            None => query = query.bind(Option::<String>::None),
        }
    }
    query.execute(pool).await?;

    super::get_agent_by_id(pool, id).await
}
