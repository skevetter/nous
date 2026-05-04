// Integration tests for tasks module

mod common;

use nous_core::error::NousError;
use nous_core::tasks;
use sea_orm::ConnectionTrait;

async fn seed_agents(pool: &nous_core::db::DatabaseConnection) {
    for agent_id in [
        "agent-1", "agent-42", "creator-1", "actor-1", "actor", "closer",
        "alice", "bob", "new-agent",
    ] {
        pool.execute_unprepared(
            &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
        ).await.unwrap();
    }
}

#[tokio::test]
async fn test_direct_cycle_detection_blocked_by() {
    let (pools, _tmp) = common::setup_test_db().await;

    let task_a = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task A", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();
    let task_b = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task B", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();

    // Create A -> B
    tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_a.id, target_id: &task_b.id, link_type: "blocked_by", actor_id: None })
        .await
        .unwrap();

    // Attempt B -> A (should fail with CyclicLink)
    let result = tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_b.id, target_id: &task_a.id, link_type: "blocked_by", actor_id: None }).await;
    assert!(matches!(
        result,
        Err(nous_core::error::NousError::CyclicLink(_))
    ));

    pools.close().await;
}

#[tokio::test]
async fn test_indirect_cycle_detection_parent() {
    let (pools, _tmp) = common::setup_test_db().await;

    let task_a = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task A", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();
    let task_b = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task B", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();
    let task_c = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task C", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();

    // Create A -> B -> C
    tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_a.id, target_id: &task_b.id, link_type: "parent", actor_id: None })
        .await
        .unwrap();
    tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_b.id, target_id: &task_c.id, link_type: "parent", actor_id: None })
        .await
        .unwrap();

    // Attempt C -> A (should fail with CyclicLink)
    let result = tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_c.id, target_id: &task_a.id, link_type: "parent", actor_id: None }).await;
    assert!(matches!(
        result,
        Err(nous_core::error::NousError::CyclicLink(_))
    ));

    pools.close().await;
}

#[tokio::test]
async fn test_related_to_allows_bidirectional() {
    let (pools, _tmp) = common::setup_test_db().await;

    let task_a = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task A", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();
    let task_b = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task B", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();

    // related_to should allow bidirectional links (no cycle detection)
    tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_a.id, target_id: &task_b.id, link_type: "related_to", actor_id: None })
        .await
        .unwrap();
    tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &task_b.id, target_id: &task_a.id, link_type: "related_to", actor_id: None })
        .await
        .unwrap();

    pools.close().await;
}

#[tokio::test]
async fn test_fts_trigger_on_insert() {
    let (pools, _tmp) = common::setup_test_db().await;

    let _task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Find authentication bug",
        description: Some("The OAuth2 flow is failing intermittently"),
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    // Query FTS5 table to verify content was indexed
    use sea_orm::{ConnectionTrait, Statement, TryGetable};
    let row = pools
        .fts
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'authentication'",
        ))
        .await
        .unwrap()
        .unwrap();
    let count: i64 = i64::try_get_by(&row, 0usize).unwrap();
    assert_eq!(count, 1, "FTS5 should have indexed the task title");

    let row = pools
        .fts
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'OAuth2'",
        ))
        .await
        .unwrap()
        .unwrap();
    let count: i64 = i64::try_get_by(&row, 0usize).unwrap();
    assert_eq!(count, 1, "FTS5 should have indexed the task description");

    pools.close().await;
}

#[tokio::test]
async fn test_fts_trigger_on_update() {
    let (pools, _tmp) = common::setup_test_db().await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Original title",
        description: Some("Original description"),
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    // Update the task description (status, priority, assignee_id, description, labels, actor_id)
    tasks::update_task(tasks::UpdateTaskParams {
        db: &pools.fts,
        id: &task.id,
        status: None,
        priority: None,
        assignee_id: None,
        description: Some("Updated description with unique word"),
        labels: None,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    // Verify FTS5 reflects the updated content
    use sea_orm::{ConnectionTrait, Statement, TryGetable};
    let row = pools
        .fts
        .query_one(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "SELECT COUNT(*) FROM tasks_fts WHERE content MATCH 'unique'",
        ))
        .await
        .unwrap()
        .unwrap();
    let count: i64 = i64::try_get_by(&row, 0usize).unwrap();
    assert_eq!(count, 1, "FTS5 should have updated with new description");

    pools.close().await;
}

#[tokio::test]
async fn test_tasks_au_trigger_updates_timestamp() {
    let (pools, _tmp) = common::setup_test_db().await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Test task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    let created_at = task.created_at.clone();
    let updated_at_initial = task.updated_at.clone();

    // Wait a bit to ensure timestamp changes
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Update the task (status, priority, assignee_id, description, labels, actor_id)
    tasks::update_task(tasks::UpdateTaskParams {
        db: &pools.fts,
        id: &task.id,
        status: Some("in_progress"),
        priority: None,
        assignee_id: None,
        description: None,
        labels: None,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Minimal task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let labels = vec!["bug".to_string(), "urgent".to_string()];
    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Full task",
        description: Some("A detailed description"),
        priority: Some("high"),
        assignee_id: Some("agent-42"),
        labels: Some(&labels),
        room_id: None,
        create_room: false,
        actor_id: Some("creator-1"),
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;

    let result = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "  ", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await;
    assert!(matches!(result, Err(NousError::Validation(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_create_task_with_room_creation() {
    let (pools, _tmp) = common::setup_test_db().await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Task with room",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: true,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;

    let t1 = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Open task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Another open",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    tasks::close_task(&pools.fts, &t1.id, None).await.unwrap();

    let open_tasks =
        tasks::list_tasks(tasks::ListTasksParams {
            db: &pools.fts, status: Some("open"), assignee_id: None, label: None, limit: None, offset: None, order_by: None, order_dir: None,
        })
            .await
            .unwrap();
    assert_eq!(open_tasks.len(), 1);
    assert_eq!(open_tasks[0].title, "Another open");

    let closed_tasks = tasks::list_tasks(tasks::ListTasksParams {
        db: &pools.fts,
        status: Some("closed"),
        assignee_id: None,
        label: None,
        limit: None,
        offset: None,
        order_by: None,
        order_dir: None,
    })
    .await
    .unwrap();
    assert_eq!(closed_tasks.len(), 1);
    assert_eq!(closed_tasks[0].title, "Open task");

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_filter_by_assignee() {
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Alice task",
        description: None,
        priority: None,
        assignee_id: Some("alice"),
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Bob task",
        description: None,
        priority: None,
        assignee_id: Some("bob"),
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    let alice_tasks = tasks::list_tasks(tasks::ListTasksParams {
        db: &pools.fts,
        status: None,
        assignee_id: Some("alice"),
        label: None,
        limit: None,
        offset: None,
        order_by: None,
        order_dir: None,
    })
    .await
    .unwrap();
    assert_eq!(alice_tasks.len(), 1);
    assert_eq!(alice_tasks[0].title, "Alice task");

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_filter_by_label() {
    let (pools, _tmp) = common::setup_test_db().await;

    let labels_bug = vec!["bug".to_string()];
    let labels_feat = vec!["feature".to_string()];
    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Bug task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: Some(&labels_bug),
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Feature task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: Some(&labels_feat),
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    let bug_tasks = tasks::list_tasks(tasks::ListTasksParams {
        db: &pools.fts, status: None, assignee_id: None, label: Some("bug"), limit: None, offset: None, order_by: None, order_dir: None,
    })
        .await
        .unwrap();
    assert_eq!(bug_tasks.len(), 1);
    assert_eq!(bug_tasks[0].title, "Bug task");

    pools.close().await;
}

#[tokio::test]
async fn test_list_tasks_pagination() {
    let (pools, _tmp) = common::setup_test_db().await;

    for i in 0..5 {
        tasks::create_task(tasks::CreateTaskParams {
            db: &pools.fts,
            title: &format!("Task {i}"),
            description: None,
            priority: None,
            assignee_id: None,
            labels: None,
            room_id: None,
            create_room: false,
            actor_id: None,
            registry: None,
        })
        .await
        .unwrap();
    }

    let page1 = tasks::list_tasks(tasks::ListTasksParams {
        db: &pools.fts, status: None, assignee_id: None, label: None, limit: Some(2), offset: Some(0), order_by: None, order_dir: None,
    })
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);

    let page2 = tasks::list_tasks(tasks::ListTasksParams {
        db: &pools.fts, status: None, assignee_id: None, label: None, limit: Some(2), offset: Some(2), order_by: None, order_dir: None,
    })
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);

    let page3 = tasks::list_tasks(tasks::ListTasksParams {
        db: &pools.fts, status: None, assignee_id: None, label: None, limit: Some(2), offset: Some(4), order_by: None, order_dir: None,
    })
        .await
        .unwrap();
    assert_eq!(page3.len(), 1);

    pools.close().await;
}

#[tokio::test]
async fn test_update_task_status() {
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Status test",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    let updated = tasks::update_task(tasks::UpdateTaskParams {
        db: &pools.fts,
        id: &task.id,
        status: Some("in_progress"),
        priority: None,
        assignee_id: None,
        description: None,
        labels: None,
        actor_id: Some("actor-1"),
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Priority test",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    let updated = tasks::update_task(tasks::UpdateTaskParams {
        db: &pools.fts,
        id: &task.id,
        status: None,
        priority: Some("critical"),
        assignee_id: None,
        description: None,
        labels: None,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Assignee test",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    let updated = tasks::update_task(tasks::UpdateTaskParams {
        db: &pools.fts,
        id: &task.id,
        status: None,
        priority: None,
        assignee_id: Some("new-agent"),
        description: None,
        labels: None,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Close me", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;

    let result = tasks::get_task(&pools.fts, "nonexistent-id").await;
    assert!(matches!(result, Err(NousError::NotFound(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_link_then_unlink() {
    let (pools, _tmp) = common::setup_test_db().await;

    let t1 = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task 1", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();
    let t2 = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task 2", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();

    let link = tasks::link_tasks(tasks::LinkTasksParams { db: &pools.fts, source_id: &t1.id, target_id: &t2.id, link_type: "blocked_by", actor_id: None })
        .await
        .unwrap();
    assert_eq!(link.source_id, t1.id);
    assert_eq!(link.target_id, t2.id);
    assert_eq!(link.link_type, "blocked_by");

    let links = tasks::list_links(&pools.fts, &t1.id).await.unwrap();
    assert_eq!(links.blocked_by.len(), 1);

    tasks::unlink_tasks(tasks::UnlinkTasksParams { db: &pools.fts, source_id: &t1.id, target_id: &t2.id, link_type: "blocked_by", actor_id: None })
        .await
        .unwrap();

    let links = tasks::list_links(&pools.fts, &t1.id).await.unwrap();
    assert!(links.blocked_by.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_unlink_nonexistent_fails() {
    let (pools, _tmp) = common::setup_test_db().await;

    let t1 = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task 1", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();
    let t2 = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts, title: "Task 2", description: None, priority: None, assignee_id: None, labels: None, room_id: None, create_room: false, actor_id: None, registry: None,
    })
    .await
    .unwrap();

    let result = tasks::unlink_tasks(tasks::UnlinkTasksParams { db: &pools.fts, source_id: &t1.id, target_id: &t2.id, link_type: "blocked_by", actor_id: None }).await;
    assert!(matches!(result, Err(NousError::NotFound(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_add_note_auto_creates_room() {
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "No room task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Room task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: true,
        actor_id: None,
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;
    seed_agents(&pools.fts).await;

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "History task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: Some("actor"),
        registry: None,
    })
    .await
    .unwrap();
    tasks::update_task(tasks::UpdateTaskParams {
        db: &pools.fts,
        id: &task.id,
        status: Some("in_progress"),
        priority: None,
        assignee_id: None,
        description: None,
        labels: None,
        actor_id: Some("actor"),
        registry: None,
    })
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
    let (pools, _tmp) = common::setup_test_db().await;

    let result = tasks::search_tasks(&pools.fts, "  ", None).await;
    assert!(matches!(result, Err(NousError::Validation(_))));

    pools.close().await;
}

#[tokio::test]
async fn test_search_tasks_finds_by_title() {
    let (pools, _tmp) = common::setup_test_db().await;

    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Optimize database queries",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();
    tasks::create_task(tasks::CreateTaskParams {
        db: &pools.fts,
        title: "Fix login page",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: None,
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    let results = tasks::search_tasks(&pools.fts, "database", None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Optimize database queries");

    pools.close().await;
}
