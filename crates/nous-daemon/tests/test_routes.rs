mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use nous_daemon::app;
use nous_daemon::state::AppState;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

async fn test_state() -> (AppState, TempDir) {
    common::test_state().await
}

async fn json_body(response: axum::http::Response<Body>) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

// =============================================================================
// Agent Routes: search, stale, inspect, versioning, templates
// =============================================================================

#[tokio::test]
async fn agent_search_returns_matching_agents() {
    let (state, _tmp) = test_state().await;

    nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "uniquefindable".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("search-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // FTS5 MATCH: use a simple token that matches the agent name
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/agents/search?q=uniquefindable&namespace=search-ns")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body: Value = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "search failed: {body}");
    let agents = body["data"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["name"], "uniquefindable");
}

#[tokio::test]
async fn agent_search_no_match_returns_empty() {
    let (state, _tmp) = test_state().await;

    nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "existingagent".into(),
            agent_type: None,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Search for a term that won't match anything
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/agents/search?q=zzznomatchxyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body: Value = json_body(response).await;
    assert_eq!(status, StatusCode::OK, "search failed: {body}");
    let agents = body["data"].as_array().unwrap();
    assert!(agents.is_empty());
}

#[tokio::test]
async fn agent_stale_returns_agents_past_threshold() {
    let (state, _tmp) = test_state().await;

    // Register an agent and give it a heartbeat so last_seen_at is set
    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "stale-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("default".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Heartbeat sets last_seen_at
    nous_core::agents::heartbeat(&state.pool, &agent.id, None)
        .await
        .unwrap();

    // With threshold=0, any agent with a last_seen_at should appear stale
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/agents/stale?threshold=0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let agents = body["data"].as_array().unwrap();
    assert!(!agents.is_empty());
    assert!(agents.iter().any(|a| a["name"] == "stale-agent"));
}

#[tokio::test]
async fn agent_inspect_returns_details() {
    let (state, _tmp) = test_state().await;

    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "inspect-me".into(),
            agent_type: None,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/inspect", agent.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    // AgentInspection uses #[serde(flatten)] so agent fields are at top level
    assert_eq!(body["data"]["name"], "inspect-me");
}

#[tokio::test]
async fn agent_versioning_record_list_rollback() {
    let (state, _tmp) = test_state().await;

    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "version-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Record version
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents/versions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "agent_id": agent.id,
                        "skill_hash": "abc123",
                        "config_hash": "def456",
                        "skills_json": "[\"skill-a\"]"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    let version_id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["skill_hash"], "abc123");

    // Record second version
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents/versions")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "agent_id": agent.id,
                        "skill_hash": "xyz789",
                        "config_hash": "uvw012",
                        "skills_json": "[\"skill-b\"]"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // List versions
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/agents/{}/versions", agent.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let versions = body["data"].as_array().unwrap();
    assert_eq!(versions.len(), 2);

    // Rollback to first version
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/agents/{}/rollback", agent.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "version_id": version_id }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["skill_hash"], "abc123");
}

#[tokio::test]
async fn agent_notify_upgrade_and_list_outdated() {
    let (state, _tmp) = test_state().await;

    let agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "upgrade-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: Some("upgrade-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    // Notify upgrade
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/agents/{}/notify-upgrade", agent.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["notified"], true);

    // List outdated
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/agents/outdated?namespace=upgrade-ns")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let outdated = body["data"].as_array().unwrap();
    assert_eq!(outdated.len(), 1);
    assert_eq!(outdated[0]["name"], "upgrade-agent");
}

#[tokio::test]
async fn agent_template_create_list_get_instantiate() {
    let (state, _tmp) = test_state().await;

    // Create template
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/templates")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "worker-template",
                        "type": "worker",
                        "default_config": "{\"concurrency\": 4}",
                        "skill_refs": "[\"data-processing\"]"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    let template_id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["name"], "worker-template");
    assert_eq!(body["data"]["template_type"], "worker");

    // List templates
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/templates?type=worker")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let templates = body["data"].as_array().unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0]["name"], "worker-template");

    // Get template
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/templates/{template_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["name"], "worker-template");

    // Instantiate
    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/templates/instantiate")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "template_id": template_id,
                        "name": "instantiated-worker",
                        "namespace": "prod"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["name"], "instantiated-worker");
    assert_eq!(body["data"]["namespace"], "prod");
}

// =============================================================================
// Memory Routes: save, get, update, search, context, relate, list_relations
// =============================================================================

#[tokio::test]
async fn memory_save_returns_201() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memories")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "title": "Test decision",
                        "content": "We decided to use Rust for the backend",
                        "type": "decision",
                        "importance": "high",
                        "topic_key": "architecture"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["title"], "Test decision");
    assert_eq!(body["data"]["memory_type"], "decision");
    assert_eq!(body["data"]["importance"], "high");
    assert_eq!(body["data"]["topic_key"], "architecture");
}

#[tokio::test]
async fn memory_get_returns_200() {
    let (state, _tmp) = test_state().await;

    let mem = nous_core::memory::save_memory(
        &state.pool,
        nous_core::memory::SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Retrievable memory".into(),
            content: "This content can be fetched".into(),
            memory_type: nous_core::memory::MemoryType::Fact,
            importance: None,
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/memories/{}", mem.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["title"], "Retrievable memory");
    assert_eq!(body["data"]["content"], "This content can be fetched");
}

#[tokio::test]
async fn memory_get_not_found_returns_404() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/memories/nonexistent-mem-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn memory_update_returns_200() {
    let (state, _tmp) = test_state().await;

    let mem = nous_core::memory::save_memory(
        &state.pool,
        nous_core::memory::SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Original title".into(),
            content: "Original content".into(),
            memory_type: nous_core::memory::MemoryType::Observation,
            importance: Some(nous_core::memory::Importance::Moderate),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/memories/{}", mem.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "title": "Updated title",
                        "content": "Updated content",
                        "importance": "high",
                        "topic_key": "updated-topic"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["title"], "Updated title");
    assert_eq!(body["data"]["content"], "Updated content");
    assert_eq!(body["data"]["importance"], "high");
    assert_eq!(body["data"]["topic_key"], "updated-topic");
}

#[tokio::test]
async fn memory_search_returns_results() {
    let (state, _tmp) = test_state().await;

    nous_core::memory::save_memory(
        &state.pool,
        nous_core::memory::SaveMemoryRequest {
            workspace_id: Some("ws-1".into()),
            agent_id: None,
            title: "Searchable observation".into(),
            content: "The deployment pipeline uses GitHub Actions".into(),
            memory_type: nous_core::memory::MemoryType::Observation,
            importance: Some(nous_core::memory::Importance::High),
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/memories/search?q=deployment+pipeline&workspace_id=ws-1&type=observation&importance=high")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let results = body["data"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["title"], "Searchable observation");
}

#[tokio::test]
async fn memory_context_returns_recent() {
    let (state, _tmp) = test_state().await;

    for i in 0..3 {
        nous_core::memory::save_memory(
            &state.pool,
            nous_core::memory::SaveMemoryRequest {
                workspace_id: Some("ctx-ws".into()),
                agent_id: Some("ctx-agent".into()),
                title: format!("Context memory {i}"),
                content: format!("Content {i}"),
                memory_type: nous_core::memory::MemoryType::Fact,
                importance: None,
                topic_key: Some("ctx-topic".into()),
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();
    }

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/memories?workspace_id=ctx-ws&agent_id=ctx-agent&topic_key=ctx-topic&limit=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let results = body["data"].as_array().unwrap();
    assert!(results.len() >= 1, "context should return saved memories, got {}", results.len());
}

#[tokio::test]
async fn memory_relate_and_list_relations() {
    let (state, _tmp) = test_state().await;

    let mem1 = nous_core::memory::save_memory(
        &state.pool,
        nous_core::memory::SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Source memory".into(),
            content: "I am the source".into(),
            memory_type: nous_core::memory::MemoryType::Decision,
            importance: None,
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    let mem2 = nous_core::memory::save_memory(
        &state.pool,
        nous_core::memory::SaveMemoryRequest {
            workspace_id: None,
            agent_id: None,
            title: "Target memory".into(),
            content: "I am the target".into(),
            memory_type: nous_core::memory::MemoryType::Decision,
            importance: None,
            topic_key: None,
            valid_from: None,
            valid_until: None,
        },
    )
    .await
    .unwrap();

    // Create relation
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/memories/relate")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "source_id": mem1.id,
                        "target_id": mem2.id,
                        "relation_type": "supersedes"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["source_id"], mem1.id);
    assert_eq!(body["data"]["target_id"], mem2.id);
    assert_eq!(body["data"]["relation_type"], "supersedes");

    // List relations
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/memories/{}/relations", mem1.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let relations = body["data"].as_array().unwrap();
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0]["target_id"], mem2.id);
    assert_eq!(relations[0]["relation_type"], "supersedes");
}

// =============================================================================
// Resource Routes: update, transfer
// =============================================================================

#[tokio::test]
async fn resource_update_returns_200() {
    let (state, _tmp) = test_state().await;

    let resource = nous_core::resources::register_resource(
        &state.pool,
        nous_core::resources::RegisterResourceRequest {
            name: "updatable-res".into(),
            resource_type: nous_core::resources::ResourceType::File,
            owner_agent_id: None,
            namespace: None,
            path: Some("/old/path".into()),
            metadata: None,
            tags: Some(vec!["old-tag".into()]),
            ownership_policy: None,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/resources/{}", resource.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "updated-res",
                        "path": "/new/path",
                        "tags": ["new-tag", "extra"],
                        "status": "active"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["name"], "updated-res");
    assert_eq!(body["data"]["path"], "/new/path");
}

#[tokio::test]
async fn resource_transfer_ownership() {
    let (state, _tmp) = test_state().await;

    let from_agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "from-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let to_agent = nous_core::agents::register_agent(
        &state.pool,
        nous_core::agents::RegisterAgentRequest {
            name: "to-agent".into(),
            agent_type: None,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    nous_core::resources::register_resource(
        &state.pool,
        nous_core::resources::RegisterResourceRequest {
            name: "transferable".into(),
            resource_type: nous_core::resources::ResourceType::Worktree,
            owner_agent_id: Some(from_agent.id.clone()),
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
            ownership_policy: None,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/resources/transfer")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "from_agent_id": from_agent.id,
                        "to_agent_id": to_agent.id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["transferred"], 1);
}

// =============================================================================
// Schedule Routes: create, list, get, update, delete, list_runs, health
// =============================================================================

#[tokio::test]
async fn schedule_create_returns_201() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/schedules")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "daily-cleanup",
                        "cron_expr": "0 2 * * *",
                        "action_type": "shell",
                        "action_payload": "echo cleanup",
                        "desired_outcome": "Remove old files",
                        "max_retries": 3,
                        "timeout_secs": 60
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["name"], "daily-cleanup");
    assert_eq!(body["data"]["cron_expr"], "0 2 * * *");
    assert_eq!(body["data"]["action_type"], "shell");
    assert_eq!(body["data"]["enabled"], true);
    assert_eq!(body["data"]["max_retries"], 3);
    assert_eq!(body["data"]["timeout_secs"], 60);
}

#[tokio::test]
async fn schedule_list_returns_200() {
    let (state, _tmp) = test_state().await;

    nous_core::schedules::create_schedule(nous_core::schedules::CreateScheduleParams {
        db: &state.pool,
        name: "listed-schedule",
        cron_expr: "*/5 * * * *",
        trigger_at: None,
        timezone: None,
        action_type: "mcp_tool",
        action_payload: "{}",
        desired_outcome: None,
        max_retries: None,
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &nous_core::schedules::SystemClock,
    })
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/schedules")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let schedules = body["data"].as_array().unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0]["name"], "listed-schedule");
}

#[tokio::test]
async fn schedule_get_returns_200() {
    let (state, _tmp) = test_state().await;

    let schedule = nous_core::schedules::create_schedule(
        nous_core::schedules::CreateScheduleParams {
            db: &state.pool,
            name: "get-schedule",
            cron_expr: "0 * * * *",
            trigger_at: None,
            timezone: None,
            action_type: "shell",
            action_payload: "echo hi",
            desired_outcome: None,
            max_retries: None,
            timeout_secs: None,
            max_output_bytes: None,
            max_runs: None,
            clock: &nous_core::schedules::SystemClock,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/schedules/{}", schedule.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["name"], "get-schedule");
    assert_eq!(body["data"]["cron_expr"], "0 * * * *");
}

#[tokio::test]
async fn schedule_get_not_found_returns_404() {
    let (state, _tmp) = test_state().await;

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/schedules/nonexistent-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn schedule_update_returns_200() {
    let (state, _tmp) = test_state().await;

    let schedule = nous_core::schedules::create_schedule(
        nous_core::schedules::CreateScheduleParams {
            db: &state.pool,
            name: "update-schedule",
            cron_expr: "0 0 * * *",
            trigger_at: None,
            timezone: None,
            action_type: "shell",
            action_payload: "echo old",
            desired_outcome: None,
            max_retries: None,
            timeout_secs: None,
            max_output_bytes: None,
            max_runs: None,
            clock: &nous_core::schedules::SystemClock,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/schedules/{}", schedule.id))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "updated-schedule",
                        "cron_expr": "*/10 * * * *",
                        "enabled": false,
                        "action_payload": "echo new",
                        "max_retries": 5
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["name"], "updated-schedule");
    assert_eq!(body["data"]["cron_expr"], "*/10 * * * *");
    assert_eq!(body["data"]["enabled"], false);
    assert_eq!(body["data"]["max_retries"], 5);
}

#[tokio::test]
async fn schedule_delete_returns_204() {
    let (state, _tmp) = test_state().await;

    let schedule = nous_core::schedules::create_schedule(
        nous_core::schedules::CreateScheduleParams {
            db: &state.pool,
            name: "delete-me",
            cron_expr: "0 0 * * *",
            trigger_at: None,
            timezone: None,
            action_type: "shell",
            action_payload: "echo del",
            desired_outcome: None,
            max_retries: None,
            timeout_secs: None,
            max_output_bytes: None,
            max_runs: None,
            clock: &nous_core::schedules::SystemClock,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/schedules/{}", schedule.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn schedule_list_runs_returns_200() {
    let (state, _tmp) = test_state().await;

    let schedule = nous_core::schedules::create_schedule(
        nous_core::schedules::CreateScheduleParams {
            db: &state.pool,
            name: "runs-schedule",
            cron_expr: "0 0 * * *",
            trigger_at: None,
            timezone: None,
            action_type: "shell",
            action_payload: "echo runs",
            desired_outcome: None,
            max_retries: None,
            timeout_secs: None,
            max_output_bytes: None,
            max_runs: None,
            clock: &nous_core::schedules::SystemClock,
        },
    )
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/schedules/{}/runs", schedule.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let runs = body["data"].as_array().unwrap();
    assert!(runs.is_empty());
}

#[tokio::test]
async fn schedule_health_returns_200() {
    let (state, _tmp) = test_state().await;

    // Create a schedule so health has something to report
    nous_core::schedules::create_schedule(nous_core::schedules::CreateScheduleParams {
        db: &state.pool,
        name: "health-schedule",
        cron_expr: "0 0 * * *",
        trigger_at: None,
        timezone: None,
        action_type: "shell",
        action_payload: "echo health",
        desired_outcome: None,
        max_retries: None,
        timeout_secs: None,
        max_output_bytes: None,
        max_runs: None,
        clock: &nous_core::schedules::SystemClock,
    })
    .await
    .unwrap();

    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/schedules/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["total"], 1);
    assert_eq!(body["data"]["active"], 1);
}

// =============================================================================
// Schedule full lifecycle via HTTP
// =============================================================================

#[tokio::test]
async fn schedule_full_lifecycle() {
    let (state, _tmp) = test_state().await;

    // Create
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/schedules")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "lifecycle-schedule",
                        "cron_expr": "*/30 * * * *",
                        "action_type": "mcp_tool",
                        "action_payload": "{\"tool\": \"room_list\"}",
                        "max_runs": 10
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = json_body(response).await;
    let schedule_id = body["data"]["id"].as_str().unwrap().to_string();

    // Update to disable
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/schedules/{schedule_id}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "enabled": false }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    assert_eq!(body["data"]["enabled"], false);

    // List filtered by enabled=false
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/schedules?enabled=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = json_body(response).await;
    let schedules = body["data"].as_array().unwrap();
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0]["name"], "lifecycle-schedule");

    // Delete
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/schedules/{schedule_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify gone
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/schedules/{schedule_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
