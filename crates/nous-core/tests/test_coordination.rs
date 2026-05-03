use nous_core::agents::coordination::{post_handoff, HandoffPayload};
use nous_core::agents::{self, AgentType, RegisterAgentRequest};
use nous_core::db::DbPools;
use nous_core::messages::{read_messages, MessageType, ReadMessagesRequest};
use nous_core::notifications::list_subscriptions;
use nous_core::rooms;
use nous_core::tasks::{self, TaskCommand};
use tempfile::TempDir;

async fn setup() -> (DbPools, TempDir) {
    let db_dir = TempDir::new().unwrap();
    let pools = DbPools::connect(db_dir.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools, db_dir)
}

#[tokio::test]
async fn test_task_lifecycle_room_projection() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "lifecycle-room", None, None)
        .await
        .unwrap();

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: pool,
        title: "Lifecycle test task",
        description: None,
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: Some(&room.id),
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    tasks::update_task(tasks::UpdateTaskParams {
        db: pool,
        id: &task.id,
        status: Some("in_progress"),
        priority: None,
        assignee_id: None,
        description: None,
        labels: None,
        actor_id: Some("actor-lifecycle"),
        registry: None,
    })
    .await
    .unwrap();

    tasks::update_task(tasks::UpdateTaskParams {
        db: pool,
        id: &task.id,
        status: Some("closed"),
        priority: None,
        assignee_id: None,
        description: None,
        labels: None,
        actor_id: Some("actor-lifecycle"),
        registry: None,
    })
    .await
    .unwrap();

    let msgs = read_messages(
        pool,
        ReadMessagesRequest {
            room_id: room.id.clone(),
            since: None,
            before: None,
            limit: None,
        },
    )
    .await
    .unwrap();

    let task_events: Vec<_> = msgs
        .iter()
        .filter(|m| m.message_type == MessageType::TaskEvent)
        .collect();

    assert!(
        task_events.len() >= 3,
        "expected at least 3 task_event messages (created + 2 status changes), got {}",
        task_events.len()
    );

    let status_events: Vec<_> = task_events
        .iter()
        .filter(|m| m.content.contains("Task status:"))
        .collect();
    assert!(
        status_events.len() >= 2,
        "expected at least 2 status change events, got {}",
        status_events.len()
    );

    assert!(status_events
        .iter()
        .any(|m| m.content.contains("in_progress")));
    assert!(status_events.iter().any(|m| m.content.contains("closed")));

    for evt in &task_events {
        assert_eq!(evt.room_id, room.id);
        let meta = evt
            .metadata
            .as_ref()
            .expect("task_event should have metadata");
        assert_eq!(meta["task_event"]["task_id"], task.id);
    }

    pools.close().await;
}

#[tokio::test]
async fn test_agent_handoff_e2e() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "handoff-room", None, None)
        .await
        .unwrap();

    let _manager = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "handoff-manager".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: Some("handoff-ns".into()),
            room: Some(room.name.clone()),
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let _engineer = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "handoff-engineer".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(_manager.id.clone()),
            namespace: Some("handoff-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let payload = HandoffPayload {
        task_id: Some("TASK-HANDOFF-001".into()),
        branch: Some("feat/handoff-test".into()),
        scope: Some("integration test module".into()),
        acceptance_criteria: vec!["All 4 tests pass".into(), "No regressions".into()],
        context: serde_json::json!({"priority": "high", "sprint": "ws-b"}),
        deadline: Some("2026-05-04".into()),
    };

    let msg = post_handoff(pool, None, &room.id, &_manager.id, &_engineer.id, payload)
        .await
        .unwrap();

    assert_eq!(msg.message_type, MessageType::Handoff);
    assert_eq!(msg.room_id, room.id);
    assert_eq!(msg.sender_id, _manager.id);

    let meta = msg.metadata.expect("handoff message should have metadata");
    assert_eq!(meta["handoff"]["from_agent"], _manager.id);
    assert_eq!(meta["handoff"]["to_agent"], _engineer.id);
    assert_eq!(meta["handoff"]["task_id"], "TASK-HANDOFF-001");
    assert_eq!(meta["handoff"]["branch"], "feat/handoff-test");

    let criteria = meta["handoff"]["acceptance_criteria"]
        .as_array()
        .expect("acceptance_criteria should be an array");
    assert_eq!(criteria.len(), 2);
    assert_eq!(criteria[0], "All 4 tests pass");

    assert!(msg.content.contains(&_engineer.id));

    pools.close().await;
}

#[tokio::test]
async fn test_coordination_room_auto_create() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let parent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "coord-parent".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: Some("coord-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let child = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "coord-child".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(parent.id.clone()),
            namespace: Some("coord-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let coord_room_name = format!("coord-{}-{}", "coord-ns", "coord-parent");
    let coord_room = rooms::get_room(pool, &coord_room_name).await.unwrap();
    assert_eq!(coord_room.name, coord_room_name);

    let parent_subs = list_subscriptions(pool, &parent.id).await.unwrap();
    assert!(
        parent_subs.iter().any(|s| s.room_id == coord_room.id),
        "parent should be subscribed to coordination room"
    );

    let child_subs = list_subscriptions(pool, &child.id).await.unwrap();
    assert!(
        child_subs.iter().any(|s| s.room_id == coord_room.id),
        "child should be subscribed to coordination room"
    );

    let child2 = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "coord-child-2".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(parent.id.clone()),
            namespace: Some("coord-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let child2_subs = list_subscriptions(pool, &child2.id).await.unwrap();
    assert!(
        child2_subs.iter().any(|s| s.room_id == coord_room.id),
        "second child should also be subscribed to existing coordination room"
    );

    pools.close().await;
}

#[tokio::test]
async fn test_task_command_from_chat() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let room = rooms::create_room(pool, "cmd-room", None, None)
        .await
        .unwrap();

    let task = tasks::create_task(tasks::CreateTaskParams {
        db: pool,
        title: "Command test task",
        description: Some("A task to test chat-driven commands"),
        priority: None,
        assignee_id: None,
        labels: None,
        room_id: Some(&room.id),
        create_room: false,
        actor_id: None,
        registry: None,
    })
    .await
    .unwrap();

    assert_eq!(task.status, "open");

    let result = tasks::execute_task_command(
        pool,
        TaskCommand {
            command: "close".to_string(),
            task_id: task.id.clone(),
            args: vec![],
            actor_id: "chat-actor".to_string(),
        },
        None,
    )
    .await
    .unwrap();

    assert!(result.success);
    assert_eq!(result.command, "close");
    let closed_task = result.task.expect("close command should return task");
    assert_eq!(closed_task.status, "closed");

    let msgs = read_messages(
        pool,
        ReadMessagesRequest {
            room_id: room.id.clone(),
            since: None,
            before: None,
            limit: None,
        },
    )
    .await
    .unwrap();

    let task_events: Vec<_> = msgs
        .iter()
        .filter(|m| m.message_type == MessageType::TaskEvent)
        .collect();

    assert!(
        !task_events.is_empty(),
        "closing a task with a room should produce task_event messages"
    );

    let close_events: Vec<_> = task_events
        .iter()
        .filter(|m| m.content.contains("closed"))
        .collect();
    assert!(
        !close_events.is_empty(),
        "should have a task_event mentioning 'closed'"
    );

    for evt in &task_events {
        let meta = evt
            .metadata
            .as_ref()
            .expect("task_event should have metadata");
        assert_eq!(meta["task_event"]["task_id"], task.id);
    }

    pools.close().await;
}
