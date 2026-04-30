use nous_core::agents::{
    self, AgentStatus, AgentType, ArtifactType, ListAgentsFilter, ListArtifactsFilter,
    RegisterAgentRequest, RegisterArtifactRequest,
};
use nous_core::db::DbPools;
use tempfile::TempDir;

async fn setup() -> (DbPools, TempDir) {
    let db_dir = TempDir::new().unwrap();
    let pools = DbPools::connect(db_dir.path()).await.unwrap();
    pools.run_migrations().await.unwrap();
    (pools, db_dir)
}

#[tokio::test]
async fn test_register_agent() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "test-director".into(),
            agent_type: AgentType::Director,
            parent_id: None,
            namespace: Some("test-ns".into()),
            room: Some("test-room".into()),
            metadata: Some(r#"{"org":"6204"}"#.into()),
            status: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(agent.name, "test-director");
    assert_eq!(agent.agent_type, "director");
    assert_eq!(agent.namespace, "test-ns");
    assert_eq!(agent.status, "active");
    assert_eq!(agent.room.as_deref(), Some("test-room"));
    assert!(!agent.id.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_register_agent_with_parent() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let director = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "director".into(),
            agent_type: AgentType::Director,
            parent_id: None,
            namespace: Some("ns1".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let manager = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "manager".into(),
            agent_type: AgentType::Manager,
            parent_id: Some(director.id.clone()),
            namespace: Some("ns1".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(manager.parent_agent_id.as_deref(), Some(director.id.as_str()));

    pools.close().await;
}

#[tokio::test]
async fn test_register_agent_cross_namespace_parent_rejected() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let parent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "parent".into(),
            agent_type: AgentType::Director,
            parent_id: None,
            namespace: Some("ns-a".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let result = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "child".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(parent.id.clone()),
            namespace: Some("ns-b".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("same namespace"));

    pools.close().await;
}

#[tokio::test]
async fn test_lookup_agent() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "lookup-me".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: Some("default".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let found = agents::lookup_agent(pool, "lookup-me", Some("default"))
        .await
        .unwrap();
    assert_eq!(found.name, "lookup-me");

    let not_found = agents::lookup_agent(pool, "nonexistent", Some("default")).await;
    assert!(not_found.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_list_agents_with_filters() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "eng-1".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: Some("ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "mgr-1".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: Some("ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let all = agents::list_agents(
        pool,
        &ListAgentsFilter {
            namespace: Some("ns".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 2);

    let engineers = agents::list_agents(
        pool,
        &ListAgentsFilter {
            namespace: Some("ns".into()),
            agent_type: Some(AgentType::Engineer),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(engineers.len(), 1);
    assert_eq!(engineers[0].name, "eng-1");

    pools.close().await;
}

#[tokio::test]
async fn test_deregister_agent_no_children() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "solo".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let result = agents::deregister_agent(pool, &agent.id, false).await.unwrap();
    assert_eq!(result, "deleted");

    let lookup = agents::get_agent_by_id(pool, &agent.id).await;
    assert!(lookup.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_deregister_agent_with_children_no_cascade_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let parent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "parent".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "child".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(parent.id.clone()),
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let result = agents::deregister_agent(pool, &parent.id, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("children"));

    pools.close().await;
}

#[tokio::test]
async fn test_deregister_agent_cascade() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let parent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "cascade-parent".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: None,
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
            name: "cascade-child".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(parent.id.clone()),
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let result = agents::deregister_agent(pool, &parent.id, true).await.unwrap();
    assert_eq!(result, "cascaded");

    assert!(agents::get_agent_by_id(pool, &parent.id).await.is_err());
    assert!(agents::get_agent_by_id(pool, &child.id).await.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_heartbeat() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "heartbeat-agent".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    assert!(agent.last_seen_at.is_none());

    agents::heartbeat(pool, &agent.id, Some(AgentStatus::Running))
        .await
        .unwrap();

    let updated = agents::get_agent_by_id(pool, &agent.id).await.unwrap();
    assert!(updated.last_seen_at.is_some());
    assert_eq!(updated.status, "running");

    pools.close().await;
}

#[tokio::test]
async fn test_list_children_and_ancestors() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let director = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "dir".into(),
            agent_type: AgentType::Director,
            parent_id: None,
            namespace: Some("tree-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let manager = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "mgr".into(),
            agent_type: AgentType::Manager,
            parent_id: Some(director.id.clone()),
            namespace: Some("tree-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let engineer = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "eng".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(manager.id.clone()),
            namespace: Some("tree-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let children = agents::list_children(pool, &director.id, Some("tree-ns"))
        .await
        .unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "mgr");

    let ancestors = agents::list_ancestors(pool, &engineer.id, Some("tree-ns"))
        .await
        .unwrap();
    assert_eq!(ancestors.len(), 2);
    assert_eq!(ancestors[0].name, "dir");
    assert_eq!(ancestors[1].name, "mgr");

    pools.close().await;
}

#[tokio::test]
async fn test_get_tree() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let root = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "root".into(),
            agent_type: AgentType::Director,
            parent_id: None,
            namespace: Some("tree".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "leaf-a".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(root.id.clone()),
            namespace: Some("tree".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "leaf-b".into(),
            agent_type: AgentType::Engineer,
            parent_id: Some(root.id.clone()),
            namespace: Some("tree".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let tree = agents::get_tree(pool, None, Some("tree")).await.unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].agent.name, "root");
    assert_eq!(tree[0].children.len(), 2);

    pools.close().await;
}

#[tokio::test]
async fn test_search_agents() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "alpha-engineer".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: Some("search-ns".into()),
            room: None,
            metadata: Some(r#"{"skill":"rust"}"#.into()),
            status: None,
        },
    )
    .await
    .unwrap();

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "beta-manager".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: Some("search-ns".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let results = agents::search_agents(pool, "alpha", Some("search-ns"), None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "alpha-engineer");

    let results = agents::search_agents(pool, "engineer", Some("search-ns"), None)
        .await
        .unwrap();
    assert!(results.len() >= 1);

    pools.close().await;
}

#[tokio::test]
async fn test_register_artifact() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "artifact-owner".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let artifact = agents::register_artifact(
        pool,
        RegisterArtifactRequest {
            agent_id: agent.id.clone(),
            artifact_type: ArtifactType::Worktree,
            name: "my-worktree".into(),
            path: Some("/tmp/wt/my-worktree".into()),
            namespace: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(artifact.agent_id, agent.id);
    assert_eq!(artifact.artifact_type, "worktree");
    assert_eq!(artifact.name, "my-worktree");
    assert_eq!(artifact.path.as_deref(), Some("/tmp/wt/my-worktree"));
    assert_eq!(artifact.status, "active");

    pools.close().await;
}

#[tokio::test]
async fn test_list_artifacts() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "art-lister".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    agents::register_artifact(
        pool,
        RegisterArtifactRequest {
            agent_id: agent.id.clone(),
            artifact_type: ArtifactType::Room,
            name: "work-room".into(),
            path: None,
            namespace: None,
        },
    )
    .await
    .unwrap();

    agents::register_artifact(
        pool,
        RegisterArtifactRequest {
            agent_id: agent.id.clone(),
            artifact_type: ArtifactType::Branch,
            name: "feat/test".into(),
            path: Some("/repo".into()),
            namespace: None,
        },
    )
    .await
    .unwrap();

    let all = agents::list_artifacts(
        pool,
        &ListArtifactsFilter {
            agent_id: Some(agent.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 2);

    let rooms = agents::list_artifacts(
        pool,
        &ListArtifactsFilter {
            agent_id: Some(agent.id.clone()),
            artifact_type: Some(ArtifactType::Room),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0].name, "work-room");

    pools.close().await;
}

#[tokio::test]
async fn test_deregister_artifact() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "art-remover".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let artifact = agents::register_artifact(
        pool,
        RegisterArtifactRequest {
            agent_id: agent.id.clone(),
            artifact_type: ArtifactType::Schedule,
            name: "cron-job".into(),
            path: None,
            namespace: None,
        },
    )
    .await
    .unwrap();

    agents::deregister_artifact(pool, &artifact.id).await.unwrap();

    let result = agents::get_artifact_by_id(pool, &artifact.id).await;
    assert!(result.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_artifacts_cascade_on_agent_delete() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "cascade-art".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let artifact = agents::register_artifact(
        pool,
        RegisterArtifactRequest {
            agent_id: agent.id.clone(),
            artifact_type: ArtifactType::Worktree,
            name: "cascade-wt".into(),
            path: None,
            namespace: None,
        },
    )
    .await
    .unwrap();

    agents::deregister_agent(pool, &agent.id, false).await.unwrap();

    let result = agents::get_artifact_by_id(pool, &artifact.id).await;
    assert!(result.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_unique_name_namespace_constraint() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "unique-agent".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: Some("uniq".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let result = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "unique-agent".into(),
            agent_type: AgentType::Manager,
            parent_id: None,
            namespace: Some("uniq".into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await;

    assert!(result.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_update_agent_status() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "status-agent".into(),
            agent_type: AgentType::Engineer,
            parent_id: None,
            namespace: None,
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();

    let updated = agents::update_agent_status(pool, &agent.id, AgentStatus::Blocked)
        .await
        .unwrap();
    assert_eq!(updated.status, "blocked");

    pools.close().await;
}
