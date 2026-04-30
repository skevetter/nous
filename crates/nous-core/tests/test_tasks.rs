// Integration tests for tasks module

use nous_core::tasks;
use nous_core::db::DbPools;
use tempfile::TempDir;

#[tokio::test]
async fn test_direct_cycle_detection_blocked_by() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let task_a = tasks::create_task(&pools.fts, "Task A", None, None, None, None, None, false, None).await.unwrap();
    let task_b = tasks::create_task(&pools.fts, "Task B", None, None, None, None, None, false, None).await.unwrap();

    // Create A -> B
    tasks::link_tasks(&pools.fts, &task_a.id, &task_b.id, "blocked_by", None).await.unwrap();

    // Attempt B -> A (should fail with CyclicLink)
    let result = tasks::link_tasks(&pools.fts, &task_b.id, &task_a.id, "blocked_by", None).await;
    assert!(matches!(result, Err(nous_core::error::NousError::CyclicLink(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_indirect_cycle_detection_parent() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let task_a = tasks::create_task(&pools.fts, "Task A", None, None, None, None, None, false, None).await.unwrap();
    let task_b = tasks::create_task(&pools.fts, "Task B", None, None, None, None, None, false, None).await.unwrap();
    let task_c = tasks::create_task(&pools.fts, "Task C", None, None, None, None, None, false, None).await.unwrap();

    // Create A -> B -> C
    tasks::link_tasks(&pools.fts, &task_a.id, &task_b.id, "parent", None).await.unwrap();
    tasks::link_tasks(&pools.fts, &task_b.id, &task_c.id, "parent", None).await.unwrap();

    // Attempt C -> A (should fail with CyclicLink)
    let result = tasks::link_tasks(&pools.fts, &task_c.id, &task_a.id, "parent", None).await;
    assert!(matches!(result, Err(nous_core::error::NousError::CyclicLink(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_related_to_allows_bidirectional() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let task_a = tasks::create_task(&pools.fts, "Task A", None, None, None, None, None, false, None).await.unwrap();
    let task_b = tasks::create_task(&pools.fts, "Task B", None, None, None, None, None, false, None).await.unwrap();

    // related_to should allow bidirectional links (no cycle detection)
    tasks::link_tasks(&pools.fts, &task_a.id, &task_b.id, "related_to", None).await.unwrap();
    tasks::link_tasks(&pools.fts, &task_b.id, &task_a.id, "related_to", None).await.unwrap();

    pools.close().await;
}

#[tokio::test]
async fn test_fts_trigger_on_insert() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let _task = tasks::create_task(
        &pools.fts,
        "Find authentication bug",
        Some("The OAuth2 flow is failing intermittently"),
        None,
        None,
        None,
        None,
        false,
        None
    ).await.unwrap();

    // Query FTS5 table to verify content was indexed
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'authentication'"
    )
    .fetch_one(&pools.fts)
    .await
    .unwrap();

    assert_eq!(row.0, 1, "FTS5 should have indexed the task title");

    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'OAuth2'"
    )
    .fetch_one(&pools.fts)
    .await
    .unwrap();

    assert_eq!(row.0, 1, "FTS5 should have indexed the task description");

    pools.close().await;
}

#[tokio::test]
async fn test_fts_trigger_on_update() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Original title",
        Some("Original description"),
        None,
        None,
        None,
        None,
        false,
        None
    ).await.unwrap();

    // Update the task description (status, priority, assignee_id, description, labels, actor_id)
    tasks::update_task(
        &pools.fts,
        &task.id,
        None, // status
        None, // priority
        None, // assignee_id
        Some("Updated description with unique word"), // description
        None, // labels
        None, // actor_id
    ).await.unwrap();

    // Verify FTS5 reflects the updated content
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'unique'"
    )
    .fetch_one(&pools.fts)
    .await
    .unwrap();

    assert_eq!(row.0, 1, "FTS5 should have updated with new description");

    pools.close().await;
}

#[tokio::test]
async fn test_tasks_au_trigger_updates_timestamp() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations().await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Test task",
        None,
        None,
        None,
        None,
        None,
        false,
        None
    ).await.unwrap();

    let created_at = task.created_at.clone();
    let updated_at_initial = task.updated_at.clone();

    // Wait a bit to ensure timestamp changes
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Update the task (status, priority, assignee_id, description, labels, actor_id)
    tasks::update_task(
        &pools.fts,
        &task.id,
        Some("in_progress"), // status
        None,
        None,
        None,
        None,
        None,
    ).await.unwrap();

    // Fetch the task again to check timestamps
    let updated_task = tasks::get_task(&pools.fts, &task.id).await.unwrap();

    assert_eq!(updated_task.created_at, created_at, "created_at should not change");
    assert_ne!(updated_task.updated_at, updated_at_initial, "updated_at should have changed");
    assert!(updated_task.updated_at > updated_at_initial, "updated_at should be newer");

    pools.close().await;
}
