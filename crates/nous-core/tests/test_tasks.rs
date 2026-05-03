// Integration tests for tasks module

use nous_core::db::DbPools;
use nous_core::error::NousError;
use nous_core::tasks;
use tempfile::TempDir;

#[tokio::test]
async fn test_direct_cycle_detection_blocked_by() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task_a = tasks::create_task(
        &pools.fts, "Task A", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let task_b = tasks::create_task(
        &pools.fts, "Task B", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();

    // Create A -> B
    tasks::link_tasks(&pools.fts, &task_a.id, &task_b.id, "blocked_by", None)
        .await
        .unwrap();

    // Attempt B -> A (should fail with CyclicLink)
    let result = tasks::link_tasks(&pools.fts, &task_b.id, &task_a.id, "blocked_by", None).await;
    assert!(matches!(
        result,
        Err(nous_core::error::NousError::CyclicLink(_))
    ));

    pools.close().await;
}

#[tokio::test]
async fn test_indirect_cycle_detection_parent() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task_a = tasks::create_task(
        &pools.fts, "Task A", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let task_b = tasks::create_task(
        &pools.fts, "Task B", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let task_c = tasks::create_task(
        &pools.fts, "Task C", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();

    // Create A -> B -> C
    tasks::link_tasks(&pools.fts, &task_a.id, &task_b.id, "parent", None)
        .await
        .unwrap();
    tasks::link_tasks(&pools.fts, &task_b.id, &task_c.id, "parent", None)
        .await
        .unwrap();

    // Attempt C -> A (should fail with CyclicLink)
    let result = tasks::link_tasks(&pools.fts, &task_c.id, &task_a.id, "parent", None).await;
    assert!(matches!(
        result,
        Err(nous_core::error::NousError::CyclicLink(_))
    ));

    pools.close().await;
}

#[tokio::test]
async fn test_related_to_allows_bidirectional() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task_a = tasks::create_task(
        &pools.fts, "Task A", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let task_b = tasks::create_task(
        &pools.fts, "Task B", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();

    // related_to should allow bidirectional links (no cycle detection)
    tasks::link_tasks(&pools.fts, &task_a.id, &task_b.id, "related_to", None)
        .await
        .unwrap();
    tasks::link_tasks(&pools.fts, &task_b.id, &task_a.id, "related_to", None)
        .await
        .unwrap();

    pools.close().await;
}

#[tokio::test]
async fn test_fts_trigger_on_insert() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let _task = tasks::create_task(
        &pools.fts,
        "Find authentication bug",
        Some("The OAuth2 flow is failing intermittently"),
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    // Query FTS5 table to verify content was indexed
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'authentication'")
            .fetch_one(&pools.fts)
            .await
            .unwrap();

    assert_eq!(row.0, 1, "FTS5 should have indexed the task title");

    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'OAuth2'")
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
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Original title",
        Some("Original description"),
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    // Update the task description (status, priority, assignee_id, description, labels, actor_id)
    tasks::update_task(
        &pools.fts,
        &task.id,
        None,                                         // status
        None,                                         // priority
        None,                                         // assignee_id
        Some("Updated description with unique word"), // description
        None,                                         // labels
        None,                                         // actor_id
        None,
    )
    .await
    .unwrap();

    // Verify FTS5 reflects the updated content
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'unique'")
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
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Test task",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

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
        None,
    )
    .await
    .unwrap();

    // Fetch the task again to check timestamps
    let updated_task = tasks::get_task(&pools.fts, &task.id).await.unwrap();

    assert_eq!(
        updated_task.created_at, created_at,
        "created_at should not change"
    );
    assert_ne!(
        updated_task.updated_at, updated_at_initial,
        "updated_at should have changed"
    );
    assert!(
        updated_task.updated_at > updated_at_initial,
        "updated_at should be newer"
    );

    pools.close().await;
}

#[tokio::test]
async fn test_create_task_minimal() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Minimal task",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(task.title, "Minimal task");
    assert_eq!(task.status, "open");
    assert_eq!(task.priority, "medium");
    assert!(task.description.is_none());
    assert!(task.assignee_id.is_none());
    assert!(task.labels.is_none());
    assert!(task.room_id.is_none());
    assert!(task.closed_at.is_none());

    pools.close().await;
}

#[tokio::test]
async fn test_create_task_full() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let labels = vec!["bug".to_string(), "urgent".to_string()];
    let task = tasks::create_task(
        &pools.fts,
        "Full task",
        Some("A detailed description"),
        Some("high"),
        Some("agent-42"),
        Some(&labels),
        None,
        false,
        Some("creator-1"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(task.title, "Full task");
    assert_eq!(task.description.as_deref(), Some("A detailed description"));
    assert_eq!(task.priority, "high");
    assert_eq!(task.assignee_id.as_deref(), Some("agent-42"));
    assert_eq!(task.labels.as_ref().unwrap(), &labels);

    pools.close().await;
}

#[tokio::test]
async fn test_create_task_empty_title_fails() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let result = tasks::create_task(
        &pools.fts, "  ", None, None, None, None, None, false, None, None,
    )
    .await;
    assert!(matches!(result, Err(NousError::Validation(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_create_task_with_room_creation() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Task with room",
        None,
        None,
        None,
        None,
        None,
        true,
        None,
        None,
    )
    .await
    .unwrap();

    assert!(
        task.room_id.is_some(),
        "room_id should be set when create_room=true"
    );

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_filter_by_status() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let t1 = tasks::create_task(
        &pools.fts,
        "Open task",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    tasks::create_task(
        &pools.fts,
        "Another open",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    tasks::close_task(&pools.fts, &t1.id, None).await.unwrap();

    let open_tasks =
        tasks::list_tasks(&pools.fts, Some("open"), None, None, None, None, None, None)
            .await
            .unwrap();
    assert_eq!(open_tasks.len(), 1);
    assert_eq!(open_tasks[0].title, "Another open");

    let closed_tasks = tasks::list_tasks(
        &pools.fts,
        Some("closed"),
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(closed_tasks.len(), 1);
    assert_eq!(closed_tasks[0].title, "Open task");

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_filter_by_assignee() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    tasks::create_task(
        &pools.fts,
        "Alice task",
        None,
        None,
        Some("alice"),
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    tasks::create_task(
        &pools.fts,
        "Bob task",
        None,
        None,
        Some("bob"),
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    let alice_tasks = tasks::list_tasks(
        &pools.fts,
        None,
        Some("alice"),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(alice_tasks.len(), 1);
    assert_eq!(alice_tasks[0].title, "Alice task");

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_filter_by_label() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let labels_bug = vec!["bug".to_string()];
    let labels_feat = vec!["feature".to_string()];
    tasks::create_task(
        &pools.fts,
        "Bug task",
        None,
        None,
        None,
        Some(&labels_bug),
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    tasks::create_task(
        &pools.fts,
        "Feature task",
        None,
        None,
        None,
        Some(&labels_feat),
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    let bug_tasks = tasks::list_tasks(&pools.fts, None, None, Some("bug"), None, None, None, None)
        .await
        .unwrap();
    assert_eq!(bug_tasks.len(), 1);
    assert_eq!(bug_tasks[0].title, "Bug task");

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_pagination() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    for i in 0..5 {
        tasks::create_task(
            &pools.fts,
            &format!("Task {i}"),
            None,
            None,
            None,
            None,
            None,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let page1 = tasks::list_tasks(&pools.fts, None, None, None, Some(2), Some(0), None, None)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);

    let page2 = tasks::list_tasks(&pools.fts, None, None, None, Some(2), Some(2), None, None)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);

    let page3 = tasks::list_tasks(&pools.fts, None, None, None, Some(2), Some(4), None, None)
        .await
        .unwrap();
    assert_eq!(page3.len(), 1);

    pools.close().await;
}

#[tokio::test]
async fn test_update_task_status() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Status test",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    let updated = tasks::update_task(
        &pools.fts,
        &task.id,
        Some("in_progress"),
        None,
        None,
        None,
        None,
        Some("actor-1"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.status, "in_progress");

    let events = tasks::task_history(&pools.fts, &task.id, None)
        .await
        .unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event_type == "status_changed"
                && e.new_value.as_deref() == Some("in_progress"))
    );

    pools.close().await;
}

#[tokio::test]
async fn test_update_task_priority() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Priority test",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    let updated = tasks::update_task(
        &pools.fts,
        &task.id,
        None,
        Some("critical"),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.priority, "critical");

    let events = tasks::task_history(&pools.fts, &task.id, None)
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == "priority_changed" && e.new_value.as_deref() == Some("critical")));

    pools.close().await;
}

#[tokio::test]
async fn test_update_task_assignee() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Assignee test",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    let updated = tasks::update_task(
        &pools.fts,
        &task.id,
        None,
        None,
        Some("new-agent"),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.assignee_id.as_deref(), Some("new-agent"));

    let events = tasks::task_history(&pools.fts, &task.id, None)
        .await
        .unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == "assigned" && e.new_value.as_deref() == Some("new-agent")));

    pools.close().await;
}

#[tokio::test]
async fn test_close_task() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts, "Close me", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let closed = tasks::close_task(&pools.fts, &task.id, Some("closer"))
        .await
        .unwrap();

    assert_eq!(closed.status, "closed");
    assert!(closed.closed_at.is_some());

    pools.close().await;
}

#[tokio::test]
async fn test_get_task_not_found() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let result = tasks::get_task(&pools.fts, "nonexistent-id").await;
    assert!(matches!(result, Err(NousError::NotFound(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_link_then_unlink() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let t1 = tasks::create_task(
        &pools.fts, "Task 1", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let t2 = tasks::create_task(
        &pools.fts, "Task 2", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();

    let link = tasks::link_tasks(&pools.fts, &t1.id, &t2.id, "blocked_by", None)
        .await
        .unwrap();
    assert_eq!(link.source_id, t1.id);
    assert_eq!(link.target_id, t2.id);
    assert_eq!(link.link_type, "blocked_by");

    let links = tasks::list_links(&pools.fts, &t1.id).await.unwrap();
    assert_eq!(links.blocked_by.len(), 1);

    tasks::unlink_tasks(&pools.fts, &t1.id, &t2.id, "blocked_by", None)
        .await
        .unwrap();

    let links = tasks::list_links(&pools.fts, &t1.id).await.unwrap();
    assert!(links.blocked_by.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_unlink_nonexistent_fails() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let t1 = tasks::create_task(
        &pools.fts, "Task 1", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();
    let t2 = tasks::create_task(
        &pools.fts, "Task 2", None, None, None, None, None, false, None, None,
    )
    .await
    .unwrap();

    let result = tasks::unlink_tasks(&pools.fts, &t1.id, &t2.id, "blocked_by", None).await;
    assert!(matches!(result, Err(NousError::NotFound(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_add_note_auto_creates_room() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "No room task",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(task.room_id.is_none());

    // add_note should auto-create a room and succeed
    let result = tasks::add_note(&pools.fts, &task.id, "agent-1", "Hello").await;
    assert!(result.is_ok());

    // Verify the task now has a room_id
    let updated_task = tasks::get_task(&pools.fts, &task.id).await.unwrap();
    assert!(updated_task.room_id.is_some());

    pools.close().await;
}

#[tokio::test]
async fn test_add_note_with_room() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "Room task",
        None,
        None,
        None,
        None,
        None,
        true,
        None,
        None,
    )
    .await
    .unwrap();
    let note = tasks::add_note(&pools.fts, &task.id, "agent-1", "This is a note")
        .await
        .unwrap();

    assert_eq!(note["content"], "This is a note");
    assert_eq!(note["sender_id"], "agent-1");

    pools.close().await;
}

#[tokio::test]
async fn test_task_history() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let task = tasks::create_task(
        &pools.fts,
        "History task",
        None,
        None,
        None,
        None,
        None,
        false,
        Some("actor"),
        None,
    )
    .await
    .unwrap();
    tasks::update_task(
        &pools.fts,
        &task.id,
        Some("in_progress"),
        None,
        None,
        None,
        None,
        Some("actor"),
        None,
    )
    .await
    .unwrap();
    tasks::close_task(&pools.fts, &task.id, Some("actor"))
        .await
        .unwrap();

    let events = tasks::task_history(&pools.fts, &task.id, None)
        .await
        .unwrap();
    assert_eq!(events.len(), 3);

    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(types.contains(&"created"));
    assert_eq!(types.iter().filter(|&&t| t == "status_changed").count(), 2);

    pools.close().await;
}

#[tokio::test]
async fn test_search_tasks_empty_query_fails() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    let result = tasks::search_tasks(&pools.fts, "  ", None).await;
    assert!(matches!(result, Err(NousError::Validation(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_search_tasks_finds_by_title() {
    let tmp = TempDir::new().unwrap();
    let pools = DbPools::connect(tmp.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();

    tasks::create_task(
        &pools.fts,
        "Optimize database queries",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();
    tasks::create_task(
        &pools.fts,
        "Fix login page",
        None,
        None,
        None,
        None,
        None,
        false,
        None,
        None,
    )
    .await
    .unwrap();

    let results = tasks::search_tasks(&pools.fts, "database", None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Optimize database queries");

    pools.close().await;
}
