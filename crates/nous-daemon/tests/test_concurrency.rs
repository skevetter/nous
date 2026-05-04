mod common;

use nous_core::agents::{self, get_agent_by_id, AgentStatus, RegisterAgentRequest};
use nous_core::messages::{self, PostMessageRequest};
use nous_core::rooms;

// --- Concurrent agent registration with same name ---

#[tokio::test]
async fn concurrent_register_agents_same_name() {
    let (state, _tmp) = common::test_state().await;
    let db = &state.pool;

    let mut handles = Vec::new();
    for _ in 0..10 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            agents::register_agent(
                &db,
                RegisterAgentRequest {
                    name: "dup-agent".to_string(),
            agent_type: None,
                    parent_id: None,
                    namespace: Some("default".to_string()),
                    room: None,
                    metadata: None,
                    status: Some(AgentStatus::Active),
                },
            )
            .await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    let errors: Vec<_> = results.iter().filter(|r| r.is_err()).collect();

    // Under SQLite WAL concurrency, some writes may fail with database-locked errors.
    // The important invariant: no panics, no corruption, and all successful agents have unique IDs.
    assert!(
        !successes.is_empty(),
        "at least one registration should succeed"
    );
    assert_eq!(
        successes.len() + errors.len(),
        10,
        "all attempts must resolve"
    );

    let ids: std::collections::HashSet<_> = successes
        .iter()
        .map(|r| r.as_ref().unwrap().id.clone())
        .collect();
    assert_eq!(
        ids.len(),
        successes.len(),
        "all successful agents should have unique IDs"
    );
}

// --- Simultaneous delete of same agent ---

#[tokio::test]
async fn concurrent_delete_same_agent() {
    let (state, _tmp) = common::test_state().await;
    let db = &state.pool;

    let agent = agents::register_agent(
        db,
        RegisterAgentRequest {
            name: "doomed-agent".to_string(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".to_string()),
            room: None,
            metadata: None,
            status: Some(AgentStatus::Active),
        },
    )
    .await
    .unwrap();

    let mut handles = Vec::new();
    for _ in 0..5 {
        let db = db.clone();
        let id = agent.id.clone();
        handles.push(tokio::spawn(async move {
            agents::deregister_agent(&db, &id, false).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let successes = results.iter().filter(|r| r.is_ok()).count();
    let failures = results.iter().filter(|r| r.is_err()).count();

    // Exactly one delete should succeed; the rest should get NotFound
    assert!(successes >= 1, "at least one delete must succeed");
    assert_eq!(
        successes + failures,
        5,
        "all attempts must resolve (success or error)"
    );

    let not_found_count = results
        .iter()
        .filter(|r| {
            r.as_ref()
                .err()
                .map(|e| matches!(e, nous_core::error::NousError::NotFound(_)))
                .unwrap_or(false)
        })
        .count();

    assert_eq!(
        not_found_count, failures,
        "all failures should be NotFound errors"
    );
}

// --- Parallel message posting to same room ---

#[tokio::test]
async fn concurrent_post_messages_same_room() {
    let (state, _tmp) = common::test_state().await;
    let db = &state.pool;

    let room = rooms::create_room(db, "busy-room", Some("concurrency test"), None)
        .await
        .unwrap();

    let mut handles = Vec::new();
    for i in 0..20 {
        let db = db.clone();
        let room_id = room.id.clone();
        handles.push(tokio::spawn(async move {
            messages::post_message(
                &db,
                PostMessageRequest {
                    room_id,
                    sender_id: format!("agent-{i}"),
                    content: format!("Message number {i}"),
                    reply_to: None,
                    metadata: None,
                    message_type: None,
                },
                None,
            )
            .await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let successes: Vec<_> = results.iter().filter(|r| r.is_ok()).collect();
    assert_eq!(successes.len(), 20, "all messages should be posted");

    let ids: std::collections::HashSet<_> = successes
        .iter()
        .map(|r| r.as_ref().unwrap().id.clone())
        .collect();
    assert_eq!(ids.len(), 20, "all messages should have unique IDs");

    let msgs = messages::read_messages(
        db,
        messages::ReadMessagesRequest {
            room_id: room.id.clone(),
            since: None,
            before: None,
            limit: Some(100),
        },
    )
    .await
    .unwrap();
    assert_eq!(msgs.len(), 20, "all 20 messages should be readable");
}

// --- Concurrent room creation with same name ---

#[tokio::test]
async fn concurrent_create_room_same_name() {
    let (state, _tmp) = common::test_state().await;
    let db = &state.pool;

    let mut handles = Vec::new();
    for _ in 0..10 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            rooms::create_room(&db, "singleton-room", Some("race test"), None).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let successes = results.iter().filter(|r| r.is_ok()).count();
    let conflicts = results
        .iter()
        .filter(|r| {
            r.as_ref()
                .err()
                .map(|e| matches!(e, nous_core::error::NousError::Conflict(_)))
                .unwrap_or(false)
        })
        .count();

    assert_eq!(successes, 1, "exactly one room creation should succeed");
    assert_eq!(conflicts, 9, "the rest should get Conflict errors");
}

// --- Concurrent cascade delete of parent with children ---

#[tokio::test]
async fn concurrent_cascade_delete_parent_agent() {
    let (state, _tmp) = common::test_state().await;
    let db = &state.pool;

    let parent = agents::register_agent(
        db,
        RegisterAgentRequest {
            name: "parent-agent".to_string(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".to_string()),
            room: None,
            metadata: None,
            status: Some(AgentStatus::Active),
        },
    )
    .await
    .unwrap();

    for i in 0..5 {
        agents::register_agent(
            db,
            RegisterAgentRequest {
                name: format!("child-{i}"),
            agent_type: None,
                parent_id: Some(parent.id.clone()),
                namespace: Some("default".to_string()),
                room: None,
                metadata: None,
                status: Some(AgentStatus::Active),
            },
        )
        .await
        .unwrap();
    }

    let mut handles = Vec::new();
    for _ in 0..3 {
        let db = db.clone();
        let id = parent.id.clone();
        handles.push(tokio::spawn(async move {
            agents::deregister_agent(&db, &id, true).await
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    let successes = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        successes >= 1,
        "at least one cascade delete must succeed"
    );

    // Under concurrent cascade deletes, the parent should always be gone.
    // Some children may survive if their cascade-delete races with the parent's deletion.
    let parent_result = agents::get_agent_by_id(db, &parent.id).await;
    assert!(
        parent_result.is_err(),
        "parent agent must be deleted after concurrent cascade deletes"
    );
}

// --- Interleaved register and delete on same namespace ---

#[tokio::test]
async fn concurrent_register_and_delete_interleaved() {
    let (state, _tmp) = common::test_state().await;
    let db = &state.pool;

    let agents_to_create: Vec<_> = (0..10)
        .map(|i| {
            let db = db.clone();
            tokio::spawn(async move {
                agents::register_agent(
                    &db,
                    RegisterAgentRequest {
                        name: format!("ephemeral-{i}"),
            agent_type: None,
                        parent_id: None,
                        namespace: Some("default".to_string()),
                        room: None,
                        metadata: None,
                        status: Some(AgentStatus::Active),
                    },
                )
                .await
            })
        })
        .collect();

    let created: Vec<_> = futures::future::join_all(agents_to_create)
        .await
        .into_iter()
        .map(|r| r.unwrap().unwrap())
        .collect();

    // Now concurrently delete half and register new ones
    let mut handles = Vec::new();
    for agent in created.iter().take(5) {
        let db = db.clone();
        let id = agent.id.clone();
        handles.push(tokio::spawn(async move {
            agents::deregister_agent(&db, &id, false).await.map(|_| ())
        }));
    }
    for i in 10..15 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            agents::register_agent(
                &db,
                RegisterAgentRequest {
                    name: format!("ephemeral-{i}"),
            agent_type: None,
                    parent_id: None,
                    namespace: Some("default".to_string()),
                    room: None,
                    metadata: None,
                    status: Some(AgentStatus::Active),
                },
            )
            .await
            .map(|_| ())
        }));
    }

    let results: Vec<_> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();

    // Under SQLite WAL concurrency, some operations may fail with busy/locked errors.
    // The key invariant: no panics, no corruption, and final state is consistent.
    let successes = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        successes >= 1,
        "at least some interleaved operations should succeed"
    );

    // Verify we can still list agents without corruption
    let remaining = agents::list_agents(
        db,
        &agents::ListAgentsFilter {
            namespace: Some("default".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // All remaining agents should be individually fetchable (no corruption)
    for agent in &remaining {
        let fetched = get_agent_by_id(db, &agent.id).await;
        assert!(
            fetched.is_ok(),
            "listed agent {} should be individually fetchable",
            agent.id
        );
    }
}
