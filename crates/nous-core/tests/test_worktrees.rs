use std::process::Command;

use nous_core::db::DbPools;
use nous_core::error::NousError;
use nous_core::worktrees::{self, CreateWorktreeRequest, ListWorktreesFilter, WorktreeStatus};
use tempfile::TempDir;

fn init_git_repo(dir: &std::path::Path) {
    Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .expect("git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .output()
        .expect("git config email failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir)
        .output()
        .expect("git config name failed");

    std::fs::write(dir.join("README.md"), "# test").expect("write readme failed");

    Command::new("git")
        .args(["add", "."])
        .current_dir(dir)
        .output()
        .expect("git add failed");

    Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(dir)
        .output()
        .expect("git commit failed");
}

async fn setup() -> (DbPools, TempDir, TempDir) {
    let db_dir = TempDir::new().unwrap();
    let repo_dir = TempDir::new().unwrap();
    let pools = DbPools::connect(db_dir.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();
    init_git_repo(repo_dir.path());
    (pools, db_dir, repo_dir)
}

#[tokio::test]
async fn test_create_worktree() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("test-wt".into()),
            branch: "feat/test".into(),
            repo_root: repo_root.clone(),
            agent_id: Some("agent-1".into()),
            task_id: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(wt.slug, "test-wt");
    assert_eq!(wt.branch, "feat/test");
    assert_eq!(wt.repo_root, repo_root);
    assert_eq!(wt.agent_id.as_deref(), Some("agent-1"));
    assert_eq!(wt.status, "active");
    assert!(std::path::Path::new(&wt.path).exists());

    pools.close().await;
}

#[tokio::test]
async fn test_create_worktree_auto_slug() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: None,
            branch: "feat/auto-slug".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(wt.slug.len(), 8);

    pools.close().await;
}

#[tokio::test]
async fn test_create_worktree_empty_branch_fails() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let result = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: None,
            branch: "  ".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await;

    assert!(matches!(result, Err(NousError::Validation(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_get_by_id() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let created = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("get-test".into()),
            branch: "feat/get-test".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    let fetched = worktrees::get(&pools.fts, &created.id).await.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.slug, "get-test");

    pools.close().await;
}

#[tokio::test]
async fn test_get_by_slug() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("slug-test".into()),
            branch: "feat/slug-test".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    let fetched = worktrees::get(&pools.fts, "slug-test").await.unwrap();
    assert_eq!(fetched.slug, "slug-test");

    pools.close().await;
}

#[tokio::test]
async fn test_get_not_found() {
    let (pools, _db_dir, _repo_dir) = setup().await;

    let result = worktrees::get(&pools.fts, "nonexistent").await;
    assert!(matches!(result, Err(NousError::NotFound(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_list_worktrees_all() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    for i in 0..3 {
        worktrees::create(
            &pools.fts,
            CreateWorktreeRequest {
                slug: Some(format!("list-{i}")),
                branch: format!("feat/list-{i}"),
                repo_root: repo_root.clone(),
                agent_id: None,
                task_id: None,
            },
        )
        .await
        .unwrap();
    }

    let all = worktrees::list(&pools.fts, ListWorktreesFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    pools.close().await;
}

#[tokio::test]
async fn test_list_worktrees_filter_by_status() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("filter-status".into()),
            branch: "feat/filter-status".into(),
            repo_root: repo_root.clone(),
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("filter-status-2".into()),
            branch: "feat/filter-status-2".into(),
            repo_root: repo_root.clone(),
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    worktrees::archive(&pools.fts, &wt.id).await.unwrap();

    let active = worktrees::list(
        &pools.fts,
        ListWorktreesFilter {
            status: Some(WorktreeStatus::Active),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(active.len(), 1);

    let archived = worktrees::list(
        &pools.fts,
        ListWorktreesFilter {
            status: Some(WorktreeStatus::Archived),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(archived.len(), 1);

    pools.close().await;
}

#[tokio::test]
async fn test_list_worktrees_filter_by_agent() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("agent-a".into()),
            branch: "feat/agent-a".into(),
            repo_root: repo_root.clone(),
            agent_id: Some("alpha".into()),
            task_id: None,
        },
    )
    .await
    .unwrap();

    worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("agent-b".into()),
            branch: "feat/agent-b".into(),
            repo_root: repo_root.clone(),
            agent_id: Some("beta".into()),
            task_id: None,
        },
    )
    .await
    .unwrap();

    let alpha = worktrees::list(
        &pools.fts,
        ListWorktreesFilter {
            agent_id: Some("alpha".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(alpha.len(), 1);
    assert_eq!(alpha[0].slug, "agent-a");

    pools.close().await;
}

#[tokio::test]
async fn test_list_worktrees_pagination() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    for i in 0..5 {
        worktrees::create(
            &pools.fts,
            CreateWorktreeRequest {
                slug: Some(format!("page-{i}")),
                branch: format!("feat/page-{i}"),
                repo_root: repo_root.clone(),
                agent_id: None,
                task_id: None,
            },
        )
        .await
        .unwrap();
    }

    let page1 = worktrees::list(
        &pools.fts,
        ListWorktreesFilter {
            limit: Some(2),
            offset: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page1.len(), 2);

    let page3 = worktrees::list(
        &pools.fts,
        ListWorktreesFilter {
            limit: Some(2),
            offset: Some(4),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page3.len(), 1);

    pools.close().await;
}

#[tokio::test]
async fn test_archive_worktree() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("archive-me".into()),
            branch: "feat/archive".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    assert!(std::path::Path::new(&wt.path).exists());

    let archived = worktrees::archive(&pools.fts, &wt.id).await.unwrap();
    assert_eq!(archived.status, "archived");
    assert!(!std::path::Path::new(&wt.path).exists());

    pools.close().await;
}

#[tokio::test]
async fn test_delete_worktree() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("delete-me".into()),
            branch: "feat/delete".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    worktrees::delete(&pools.fts, &wt.id).await.unwrap();

    let fetched = worktrees::get(&pools.fts, &wt.id).await.unwrap();
    assert_eq!(fetched.status, "deleted");
    assert!(!std::path::Path::new(&wt.path).exists());

    pools.close().await;
}

#[tokio::test]
async fn test_update_status() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("status-test".into()),
            branch: "feat/status".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    let updated = worktrees::update_status(&pools.fts, &wt.id, WorktreeStatus::Stale)
        .await
        .unwrap();
    assert_eq!(updated.status, "stale");

    pools.close().await;
}

#[tokio::test]
async fn test_full_lifecycle() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    // Create
    let wt = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("lifecycle".into()),
            branch: "feat/lifecycle".into(),
            repo_root,
            agent_id: Some("agent-lifecycle".into()),
            task_id: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(wt.status, "active");
    assert!(std::path::Path::new(&wt.path).exists());

    // List
    let all = worktrees::list(&pools.fts, ListWorktreesFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 1);

    // Get
    let fetched = worktrees::get(&pools.fts, &wt.id).await.unwrap();
    assert_eq!(fetched.slug, "lifecycle");

    // Update status to stale
    let stale = worktrees::update_status(&pools.fts, &wt.id, WorktreeStatus::Stale)
        .await
        .unwrap();
    assert_eq!(stale.status, "stale");

    // Archive
    let archived = worktrees::archive(&pools.fts, &wt.id).await.unwrap();
    assert_eq!(archived.status, "archived");

    // Delete (already archived, path is gone — just marks as deleted)
    worktrees::delete(&pools.fts, &wt.id).await.unwrap();
    let deleted = worktrees::get(&pools.fts, &wt.id).await.unwrap();
    assert_eq!(deleted.status, "deleted");

    pools.close().await;
}

#[tokio::test]
async fn test_duplicate_slug_same_repo_fails() {
    let (pools, _db_dir, repo_dir) = setup().await;
    let repo_root = repo_dir.path().to_string_lossy().to_string();

    worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("dupe-slug".into()),
            branch: "feat/dupe-1".into(),
            repo_root: repo_root.clone(),
            agent_id: None,
            task_id: None,
        },
    )
    .await
    .unwrap();

    let result = worktrees::create(
        &pools.fts,
        CreateWorktreeRequest {
            slug: Some("dupe-slug".into()),
            branch: "feat/dupe-2".into(),
            repo_root,
            agent_id: None,
            task_id: None,
        },
    )
    .await;

    assert!(result.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_worktree_status_parse() {
    assert_eq!(
        "active".parse::<WorktreeStatus>().unwrap(),
        WorktreeStatus::Active
    );
    assert_eq!(
        "stale".parse::<WorktreeStatus>().unwrap(),
        WorktreeStatus::Stale
    );
    assert_eq!(
        "archived".parse::<WorktreeStatus>().unwrap(),
        WorktreeStatus::Archived
    );
    assert_eq!(
        "deleted".parse::<WorktreeStatus>().unwrap(),
        WorktreeStatus::Deleted
    );
    assert!("invalid".parse::<WorktreeStatus>().is_err());
}
