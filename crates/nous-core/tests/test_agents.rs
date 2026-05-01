use nous_core::agents::processes;
use nous_core::agents::{
    self, AgentStatus, AgentType, ArtifactType, CreateTemplateRequest, InstantiateRequest,
    ListAgentsFilter, ListArtifactsFilter, RecordVersionRequest, RegisterAgentRequest,
    RegisterArtifactRequest,
};
use nous_core::db::DbPools;
use tempfile::TempDir;

async fn setup() -> (DbPools, TempDir) {
    let db_dir = TempDir::new().unwrap();
    let pools = DbPools::connect(db_dir.path()).await.unwrap();
    pools.run_migrations("porter unicode61").await.unwrap();
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

    assert_eq!(
        manager.parent_agent_id.as_deref(),
        Some(director.id.as_str())
    );

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

    let result = agents::deregister_agent(pool, &agent.id, false)
        .await
        .unwrap();
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

    let result = agents::deregister_agent(pool, &parent.id, true)
        .await
        .unwrap();
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
    assert!(!results.is_empty());

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

    agents::deregister_artifact(pool, &artifact.id)
        .await
        .unwrap();

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

    agents::deregister_agent(pool, &agent.id, false)
        .await
        .unwrap();

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

// --- P7: Agent lifecycle, versioning, templates ---

#[tokio::test]
async fn test_record_and_list_versions() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "versioned-agent".into(),
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

    let v1 = agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "abc123".into(),
            config_hash: "cfg001".into(),
            skills_json: Some(r#"[{"name":"skill-a","path":"a.md","hash":"aaa"}]"#.into()),
        },
    )
    .await
    .unwrap();

    assert_eq!(v1.agent_id, agent.id);
    assert_eq!(v1.skill_hash, "abc123");
    assert!(!v1.id.is_empty());

    let refreshed = agents::get_agent_by_id(pool, &agent.id).await.unwrap();
    assert_eq!(
        refreshed.current_version_id.as_deref(),
        Some(v1.id.as_str())
    );
    assert!(!refreshed.upgrade_available);

    let v2 = agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "def456".into(),
            config_hash: "cfg002".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    let versions = agents::list_versions(pool, &agent.id, None).await.unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].id, v2.id);
    assert_eq!(versions[1].id, v1.id);

    pools.close().await;
}

#[tokio::test]
async fn test_inspect_agent() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "inspectable".into(),
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

    let inspection = agents::inspect_agent(pool, &agent.id).await.unwrap();
    assert_eq!(inspection.agent.name, "inspectable");
    assert!(inspection.current_version.is_none());
    assert!(inspection.template.is_none());
    assert_eq!(inspection.version_count, 0);

    agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "h1".into(),
            config_hash: "c1".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    let inspection = agents::inspect_agent(pool, &agent.id).await.unwrap();
    assert!(inspection.current_version.is_some());
    assert_eq!(inspection.version_count, 1);
    assert_eq!(inspection.current_version.unwrap().skill_hash, "h1");

    pools.close().await;
}

#[tokio::test]
async fn test_rollback_agent() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "rollback-agent".into(),
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

    let v1 = agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "old-hash".into(),
            config_hash: "old-cfg".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    let _v2 = agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "new-hash".into(),
            config_hash: "new-cfg".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    let rolled = agents::rollback_agent(pool, &agent.id, &v1.id)
        .await
        .unwrap();
    assert_eq!(rolled.skill_hash, "old-hash");

    let refreshed = agents::get_agent_by_id(pool, &agent.id).await.unwrap();
    assert_eq!(
        refreshed.current_version_id.as_deref(),
        Some(v1.id.as_str())
    );

    pools.close().await;
}

#[tokio::test]
async fn test_rollback_wrong_agent_rejected() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let a1 = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "agent-a".into(),
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

    let a2 = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "agent-b".into(),
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

    let v1 = agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: a1.id.clone(),
            skill_hash: "h1".into(),
            config_hash: "c1".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    let result = agents::rollback_agent(pool, &a2.id, &v1.id).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("does not belong"));

    pools.close().await;
}

#[tokio::test]
async fn test_upgrade_available_flag() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "upgrade-agent".into(),
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

    assert!(!agent.upgrade_available);

    agents::set_upgrade_available(pool, &agent.id, true)
        .await
        .unwrap();
    let refreshed = agents::get_agent_by_id(pool, &agent.id).await.unwrap();
    assert!(refreshed.upgrade_available);

    let outdated = agents::list_outdated_agents(pool, None, None)
        .await
        .unwrap();
    assert_eq!(outdated.len(), 1);
    assert_eq!(outdated[0].id, agent.id);

    agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "new".into(),
            config_hash: "new".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    let refreshed = agents::get_agent_by_id(pool, &agent.id).await.unwrap();
    assert!(!refreshed.upgrade_available);

    let outdated = agents::list_outdated_agents(pool, None, None)
        .await
        .unwrap();
    assert!(outdated.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_create_and_list_templates() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let template = agents::create_template(
        pool,
        CreateTemplateRequest {
            name: "code-reviewer".into(),
            template_type: "engineer".into(),
            default_config: Some(
                r#"{"provider":"claude/sonnet","mode":"bypassPermissions"}"#.into(),
            ),
            skill_refs: Some(r#"["org:code-reviewer","superpowers:debugging"]"#.into()),
        },
    )
    .await
    .unwrap();

    assert_eq!(template.name, "code-reviewer");
    assert_eq!(template.template_type, "engineer");
    assert!(!template.id.is_empty());

    let all = agents::list_templates(pool, None, None).await.unwrap();
    assert_eq!(all.len(), 1);

    let by_type = agents::list_templates(pool, Some("engineer"), None)
        .await
        .unwrap();
    assert_eq!(by_type.len(), 1);

    let empty = agents::list_templates(pool, Some("nonexistent"), None)
        .await
        .unwrap();
    assert!(empty.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_template_unique_name() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    agents::create_template(
        pool,
        CreateTemplateRequest {
            name: "unique-template".into(),
            template_type: "engineer".into(),
            default_config: None,
            skill_refs: None,
        },
    )
    .await
    .unwrap();

    let result = agents::create_template(
        pool,
        CreateTemplateRequest {
            name: "unique-template".into(),
            template_type: "manager".into(),
            default_config: None,
            skill_refs: None,
        },
    )
    .await;

    assert!(result.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_instantiate_from_template() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let template = agents::create_template(
        pool,
        CreateTemplateRequest {
            name: "worker-template".into(),
            template_type: "engineer".into(),
            default_config: Some(r#"{"provider":"claude/sonnet","retries":3}"#.into()),
            skill_refs: Some(r#"["skill-a"]"#.into()),
        },
    )
    .await
    .unwrap();

    let agent = agents::instantiate_from_template(
        pool,
        InstantiateRequest {
            template_id: template.id.clone(),
            name: Some("my-worker-1".into()),
            namespace: None,
            parent_id: None,
            config_overrides: Some(r#"{"retries":5,"debug":true}"#.into()),
        },
    )
    .await
    .unwrap();

    assert_eq!(agent.name, "my-worker-1");
    assert_eq!(agent.agent_type, "engineer");
    assert_eq!(agent.template_id.as_deref(), Some(template.id.as_str()));

    let meta: serde_json::Value =
        serde_json::from_str(agent.metadata_json.as_deref().unwrap()).unwrap();
    assert_eq!(meta["provider"], "claude/sonnet");
    assert_eq!(meta["retries"], 5);
    assert_eq!(meta["debug"], true);

    pools.close().await;
}

#[tokio::test]
async fn test_instantiate_auto_name() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let template = agents::create_template(
        pool,
        CreateTemplateRequest {
            name: "auto-named".into(),
            template_type: "engineer".into(),
            default_config: None,
            skill_refs: None,
        },
    )
    .await
    .unwrap();

    let agent = agents::instantiate_from_template(
        pool,
        InstantiateRequest {
            template_id: template.id.clone(),
            name: None,
            namespace: None,
            parent_id: None,
            config_overrides: None,
        },
    )
    .await
    .unwrap();

    assert!(agent.name.starts_with("auto-named-"));

    pools.close().await;
}

#[tokio::test]
async fn test_versions_cascade_on_agent_delete() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "cascade-v".into(),
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

    let v = agents::record_version(
        pool,
        RecordVersionRequest {
            agent_id: agent.id.clone(),
            skill_hash: "h".into(),
            config_hash: "c".into(),
            skills_json: None,
        },
    )
    .await
    .unwrap();

    agents::deregister_agent(pool, &agent.id, false)
        .await
        .unwrap();

    let result = agents::get_version_by_id(pool, &v.id).await;
    assert!(result.is_err());

    pools.close().await;
}

#[tokio::test]
async fn test_agent_new_fields_default() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "default-fields".into(),
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

    assert!(agent.current_version_id.is_none());
    assert!(!agent.upgrade_available);
    assert!(agent.template_id.is_none());

    pools.close().await;
}

#[tokio::test]
async fn test_process_type_claude_round_trip() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: "claude-rt".into(),
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

    assert!(agent.process_type.is_none());

    let updated = processes::update_agent(pool, &agent.id, Some("claude"), None, None, None, None)
        .await
        .unwrap();

    assert_eq!(updated.process_type.as_deref(), Some("claude"));

    let fetched = agents::get_agent_by_id(pool, &agent.id).await.unwrap();
    assert_eq!(fetched.process_type.as_deref(), Some("claude"));

    pools.close().await;
}
