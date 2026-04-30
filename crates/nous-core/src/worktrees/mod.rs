use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::NousError;

// --- Types ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeStatus {
    Active,
    Stale,
    Archived,
    Deleted,
}

impl WorktreeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Archived => "archived",
            Self::Deleted => "deleted",
        }
    }
}

impl std::fmt::Display for WorktreeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for WorktreeStatus {
    type Err = NousError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "stale" => Ok(Self::Stale),
            "archived" => Ok(Self::Archived),
            "deleted" => Ok(Self::Deleted),
            other => Err(NousError::Validation(format!(
                "invalid worktree status: '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    pub id: String,
    pub slug: String,
    pub path: String,
    pub branch: String,
    pub repo_root: String,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl Worktree {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            slug: row.try_get("slug")?,
            path: row.try_get("path")?,
            branch: row.try_get("branch")?,
            repo_root: row.try_get("repo_root")?,
            agent_id: row.try_get("agent_id")?,
            task_id: row.try_get("task_id")?,
            status: row.try_get("status")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CreateWorktreeRequest {
    pub slug: Option<String>,
    pub branch: String,
    pub repo_root: String,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateWorktreeRequest {
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub status: Option<WorktreeStatus>,
}

#[derive(Debug, Clone, Default)]
pub struct ListWorktreesFilter {
    pub status: Option<WorktreeStatus>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub repo_root: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

// --- DB operations ---

pub async fn insert_worktree(pool: &SqlitePool, wt: &Worktree) -> Result<(), NousError> {
    sqlx::query(
        "INSERT INTO worktrees (id, slug, path, branch, repo_root, agent_id, task_id, status) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&wt.id)
    .bind(&wt.slug)
    .bind(&wt.path)
    .bind(&wt.branch)
    .bind(&wt.repo_root)
    .bind(&wt.agent_id)
    .bind(&wt.task_id)
    .bind(&wt.status)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_worktree_by_id(pool: &SqlitePool, id: &str) -> Result<Worktree, NousError> {
    let row = sqlx::query("SELECT * FROM worktrees WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;

    let row = row.ok_or_else(|| NousError::NotFound(format!("worktree '{id}' not found")))?;
    Worktree::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn get_worktree_by_slug(
    pool: &SqlitePool,
    slug: &str,
    repo_root: Option<&str>,
) -> Result<Worktree, NousError> {
    let row = if let Some(root) = repo_root {
        sqlx::query("SELECT * FROM worktrees WHERE slug = ? AND repo_root = ?")
            .bind(slug)
            .bind(root)
            .fetch_optional(pool)
            .await?
    } else {
        sqlx::query("SELECT * FROM worktrees WHERE slug = ?")
            .bind(slug)
            .fetch_optional(pool)
            .await?
    };

    let row = row.ok_or_else(|| NousError::NotFound(format!("worktree slug '{slug}' not found")))?;
    Worktree::from_row(&row).map_err(NousError::Sqlite)
}

pub async fn list_worktrees(
    pool: &SqlitePool,
    filter: &ListWorktreesFilter,
) -> Result<Vec<Worktree>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut sql = String::from("SELECT * FROM worktrees");
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(ref s) = filter.status {
        conditions.push("status = ?".to_string());
        binds.push(s.as_str().to_string());
    }

    if let Some(ref a) = filter.agent_id {
        conditions.push("agent_id = ?".to_string());
        binds.push(a.clone());
    }

    if let Some(ref t) = filter.task_id {
        conditions.push("task_id = ?".to_string());
        binds.push(t.clone());
    }

    if let Some(ref r) = filter.repo_root {
        conditions.push("repo_root = ?".to_string());
        binds.push(r.clone());
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
        .map(Worktree::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn update_worktree_status(
    pool: &SqlitePool,
    id: &str,
    status: WorktreeStatus,
) -> Result<Worktree, NousError> {
    let result = sqlx::query("UPDATE worktrees SET status = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("worktree '{id}' not found")));
    }

    get_worktree_by_id(pool, id).await
}

pub async fn update_worktree(
    pool: &SqlitePool,
    id: &str,
    req: &UpdateWorktreeRequest,
) -> Result<Worktree, NousError> {
    let _existing = get_worktree_by_id(pool, id).await?;

    if let Some(ref agent_id) = req.agent_id {
        sqlx::query("UPDATE worktrees SET agent_id = ? WHERE id = ?")
            .bind(agent_id)
            .bind(id)
            .execute(pool)
            .await?;
    }

    if let Some(ref task_id) = req.task_id {
        sqlx::query("UPDATE worktrees SET task_id = ? WHERE id = ?")
            .bind(task_id)
            .bind(id)
            .execute(pool)
            .await?;
    }

    if let Some(status) = req.status {
        sqlx::query("UPDATE worktrees SET status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(id)
            .execute(pool)
            .await?;
    }

    get_worktree_by_id(pool, id).await
}

pub async fn delete_worktree(pool: &SqlitePool, id: &str) -> Result<(), NousError> {
    let result = sqlx::query("UPDATE worktrees SET status = 'deleted' WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("worktree '{id}' not found")));
    }

    Ok(())
}

// --- Service-level operations (git integration) ---

fn resolve_worktree_id_or_slug(id_or_slug: &str) -> bool {
    id_or_slug.len() == 36 && id_or_slug.chars().filter(|c| *c == '-').count() == 4
}

async fn get_by_id_or_slug(pool: &SqlitePool, id_or_slug: &str) -> Result<Worktree, NousError> {
    if resolve_worktree_id_or_slug(id_or_slug) {
        get_worktree_by_id(pool, id_or_slug).await
    } else {
        get_worktree_by_slug(pool, id_or_slug, None).await
    }
}

pub async fn create(
    pool: &SqlitePool,
    req: CreateWorktreeRequest,
) -> Result<Worktree, NousError> {
    if req.branch.trim().is_empty() {
        return Err(NousError::Validation("branch cannot be empty".into()));
    }
    if req.repo_root.trim().is_empty() {
        return Err(NousError::Validation("repo_root cannot be empty".into()));
    }

    let id = Uuid::now_v7().to_string();
    let slug = req.slug.unwrap_or_else(|| id[id.len() - 8..].to_string());
    let wt_path = PathBuf::from(&req.repo_root)
        .join(".worktrees")
        .join(&slug);
    let path_str = wt_path.to_string_lossy().to_string();

    let output = Command::new("git")
        .args(["worktree", "add", &path_str, "-b", &req.branch])
        .current_dir(&req.repo_root)
        .output()
        .map_err(|e| NousError::Internal(format!("failed to spawn git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NousError::Internal(format!(
            "git worktree add failed: {stderr}"
        )));
    }

    let wt = Worktree {
        id,
        slug,
        path: path_str,
        branch: req.branch,
        repo_root: req.repo_root,
        agent_id: req.agent_id,
        task_id: req.task_id,
        status: WorktreeStatus::Active.as_str().to_string(),
        created_at: String::new(),
        updated_at: String::new(),
    };

    insert_worktree(pool, &wt).await?;
    get_worktree_by_id(pool, &wt.id).await
}

pub async fn list(
    pool: &SqlitePool,
    filter: ListWorktreesFilter,
) -> Result<Vec<Worktree>, NousError> {
    list_worktrees(pool, &filter).await
}

pub async fn get(pool: &SqlitePool, id_or_slug: &str) -> Result<Worktree, NousError> {
    get_by_id_or_slug(pool, id_or_slug).await
}

pub async fn archive(pool: &SqlitePool, id_or_slug: &str) -> Result<Worktree, NousError> {
    let wt = get_by_id_or_slug(pool, id_or_slug).await?;

    let output = Command::new("git")
        .args(["worktree", "remove", &wt.path])
        .current_dir(&wt.repo_root)
        .output()
        .map_err(|e| NousError::Internal(format!("failed to spawn git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("is not a working tree") {
            return Err(NousError::Internal(format!(
                "git worktree remove failed: {stderr}"
            )));
        }
    }

    update_worktree_status(pool, &wt.id, WorktreeStatus::Archived).await
}

pub async fn delete(pool: &SqlitePool, id_or_slug: &str) -> Result<(), NousError> {
    let wt = get_by_id_or_slug(pool, id_or_slug).await?;

    let path = PathBuf::from(&wt.path);
    if path.exists() {
        std::fs::remove_dir_all(&path).map_err(|e| {
            NousError::Internal(format!("failed to remove worktree dir {}: {e}", wt.path))
        })?;
    }

    // Also try git worktree remove to clean up git's internal tracking
    let _ = Command::new("git")
        .args(["worktree", "remove", "--force", &wt.path])
        .current_dir(&wt.repo_root)
        .output();

    update_worktree_status(pool, &wt.id, WorktreeStatus::Deleted).await?;
    Ok(())
}

pub async fn update_status(
    pool: &SqlitePool,
    id_or_slug: &str,
    status: WorktreeStatus,
) -> Result<Worktree, NousError> {
    let wt = get_by_id_or_slug(pool, id_or_slug).await?;
    update_worktree_status(pool, &wt.id, status).await
}
