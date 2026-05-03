use std::path::PathBuf;
use std::process::Command;

use sea_orm::entity::prelude::*;
use sea_orm::{
    Condition, ConnectionTrait, DatabaseConnection, QueryOrder, QuerySelect, Set, Statement,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::worktrees as wt_entity;
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
                "invalid worktree status: '{other}'. Valid values: active, stale, archived, deleted"
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
    fn from_model(m: wt_entity::Model) -> Self {
        Self {
            id: m.id,
            slug: m.slug,
            path: m.path,
            branch: m.branch,
            repo_root: m.repo_root,
            agent_id: m.agent_id,
            task_id: m.task_id,
            status: m.status,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
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

pub async fn insert_worktree(db: &DatabaseConnection, wt: &Worktree) -> Result<(), NousError> {
    let model = wt_entity::ActiveModel {
        id: Set(wt.id.clone()),
        slug: Set(wt.slug.clone()),
        path: Set(wt.path.clone()),
        branch: Set(wt.branch.clone()),
        repo_root: Set(wt.repo_root.clone()),
        agent_id: Set(wt.agent_id.clone()),
        task_id: Set(wt.task_id.clone()),
        status: Set(wt.status.clone()),
        created_at: Set(wt.created_at.clone()),
        updated_at: Set(wt.updated_at.clone()),
    };

    wt_entity::Entity::insert(model).exec(db).await?;

    Ok(())
}

pub async fn get_worktree_by_id(db: &DatabaseConnection, id: &str) -> Result<Worktree, NousError> {
    let model = wt_entity::Entity::find_by_id(id).one(db).await?;

    match model {
        Some(m) => Ok(Worktree::from_model(m)),
        None => Err(NousError::NotFound(format!("worktree '{id}' not found"))),
    }
}

pub async fn get_worktree_by_slug(
    db: &DatabaseConnection,
    slug: &str,
    repo_root: Option<&str>,
) -> Result<Worktree, NousError> {
    let mut query = wt_entity::Entity::find().filter(wt_entity::Column::Slug.eq(slug));

    if let Some(root) = repo_root {
        query = query.filter(wt_entity::Column::RepoRoot.eq(root));
    }

    let model = query.one(db).await?;

    match model {
        Some(m) => Ok(Worktree::from_model(m)),
        None => Err(NousError::NotFound(format!(
            "worktree slug '{slug}' not found"
        ))),
    }
}

pub async fn list_worktrees(
    db: &DatabaseConnection,
    filter: &ListWorktreesFilter,
) -> Result<Vec<Worktree>, NousError> {
    let limit = filter.limit.unwrap_or(50).min(200);
    let offset = filter.offset.unwrap_or(0);

    let mut condition = Condition::all();

    if let Some(ref s) = filter.status {
        condition = condition.add(wt_entity::Column::Status.eq(s.as_str()));
    }

    if let Some(ref a) = filter.agent_id {
        condition = condition.add(wt_entity::Column::AgentId.eq(a.as_str()));
    }

    if let Some(ref t) = filter.task_id {
        condition = condition.add(wt_entity::Column::TaskId.eq(t.as_str()));
    }

    if let Some(ref r) = filter.repo_root {
        condition = condition.add(wt_entity::Column::RepoRoot.eq(r.as_str()));
    }

    let models = wt_entity::Entity::find()
        .filter(condition)
        .order_by_desc(wt_entity::Column::CreatedAt)
        .limit(limit as u64)
        .offset(offset as u64)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Worktree::from_model).collect())
}

pub async fn update_worktree_status(
    db: &DatabaseConnection,
    id: &str,
    status: WorktreeStatus,
) -> Result<Worktree, NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE worktrees SET status = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [status.as_str().into(), id.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("worktree '{id}' not found")));
    }

    get_worktree_by_id(db, id).await
}

pub async fn update_worktree(
    db: &DatabaseConnection,
    id: &str,
    req: &UpdateWorktreeRequest,
) -> Result<Worktree, NousError> {
    let _existing = get_worktree_by_id(db, id).await?;

    if let Some(ref agent_id) = req.agent_id {
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE worktrees SET agent_id = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [agent_id.as_str().into(), id.into()],
        ))
        .await?;
    }

    if let Some(ref task_id) = req.task_id {
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE worktrees SET task_id = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [task_id.as_str().into(), id.into()],
        ))
        .await?;
    }

    if let Some(status) = req.status {
        db.execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE worktrees SET status = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [status.as_str().into(), id.into()],
        ))
        .await?;
    }

    get_worktree_by_id(db, id).await
}

pub async fn delete_worktree(db: &DatabaseConnection, id: &str) -> Result<(), NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE worktrees SET status = 'deleted', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [id.into()],
        ))
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

async fn get_by_id_or_slug(
    db: &DatabaseConnection,
    id_or_slug: &str,
) -> Result<Worktree, NousError> {
    if resolve_worktree_id_or_slug(id_or_slug) {
        get_worktree_by_id(db, id_or_slug).await
    } else {
        get_worktree_by_slug(db, id_or_slug, None).await
    }
}

pub async fn create(
    db: &DatabaseConnection,
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
    let wt_path = PathBuf::from(&req.repo_root).join(".worktrees").join(&slug);
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

    insert_worktree(db, &wt).await?;
    get_worktree_by_id(db, &wt.id).await
}

pub async fn list(
    db: &DatabaseConnection,
    filter: ListWorktreesFilter,
) -> Result<Vec<Worktree>, NousError> {
    list_worktrees(db, &filter).await
}

pub async fn get(db: &DatabaseConnection, id_or_slug: &str) -> Result<Worktree, NousError> {
    get_by_id_or_slug(db, id_or_slug).await
}

pub async fn archive(db: &DatabaseConnection, id_or_slug: &str) -> Result<Worktree, NousError> {
    let wt = get_by_id_or_slug(db, id_or_slug).await?;

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

    update_worktree_status(db, &wt.id, WorktreeStatus::Archived).await
}

pub async fn delete(db: &DatabaseConnection, id_or_slug: &str) -> Result<(), NousError> {
    let wt = get_by_id_or_slug(db, id_or_slug).await?;

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

    update_worktree_status(db, &wt.id, WorktreeStatus::Deleted).await?;
    Ok(())
}

pub async fn update_status(
    db: &DatabaseConnection,
    id_or_slug: &str,
    status: WorktreeStatus,
) -> Result<Worktree, NousError> {
    let wt = get_by_id_or_slug(db, id_or_slug).await?;
    update_worktree_status(db, &wt.id, status).await
}
