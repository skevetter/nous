mod common;

use nous_core::agents::{self, RegisterAgentRequest};
use nous_core::db::DbPools;
use nous_core::inventory::{
    self, InventoryStatus, InventoryType, ListItemsFilter, RegisterItemRequest, SearchItemsRequest,
    UpdateItemRequest,
};

async fn setup() -> (DbPools, tempfile::TempDir) {
    common::setup_test_db().await
}

async fn create_test_agent(
    pool: &nous_core::db::DatabaseConnection,
    name: &str,
    namespace: &str,
) -> String {
    let agent = agents::register_agent(
        pool,
        RegisterAgentRequest {
            name: name.into(),
            parent_id: None,
            namespace: Some(namespace.into()),
            room: None,
            metadata: None,
            status: None,
        },
    )
    .await
    .unwrap();
    agent.id
}

// --- Register tests ---

#[tokio::test]
async fn test_register_item_minimal() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "my-worktree".into(),
            artifact_type: InventoryType::Worktree,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(item.name, "my-worktree");
    assert_eq!(item.artifact_type, "worktree");
    assert_eq!(item.namespace, "default");
    assert_eq!(item.status, "active");
    assert!(item.owner_agent_id.is_none());
    assert_eq!(item.tags, "[]");
    assert!(!item.id.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_register_item_full() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent_id = create_test_agent(pool, "worker-1", "default").await;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "api-server".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: Some(agent_id.clone()),
            namespace: Some("default".into()),
            path: Some("/images/api-server:latest".into()),
            metadata: Some(r#"{"digest":"sha256:abc123","size":42000}"#.into()),
            tags: Some(vec!["production".into(), "api".into(), "team-a".into()]),
        },
    )
    .await
    .unwrap();

    assert_eq!(item.name, "api-server");
    assert_eq!(item.artifact_type, "docker-image");
    assert_eq!(item.owner_agent_id.as_deref(), Some(agent_id.as_str()));
    assert_eq!(item.path.as_deref(), Some("/images/api-server:latest"));
    assert!(item.metadata.as_deref().unwrap().contains("sha256:abc123"));
    let tags: Vec<String> = serde_json::from_str(&item.tags).unwrap();
    assert_eq!(tags, vec!["production", "api", "team-a"]);

    pools.close().await;
}

#[tokio::test]
async fn test_register_item_empty_name_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let result = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "  ".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cannot be empty"));

    pools.close().await;
}

#[tokio::test]
async fn test_register_item_invalid_metadata_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let result = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "bad-item".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: Some("not valid json".into()),
            tags: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("valid JSON"));

    pools.close().await;
}

#[tokio::test]
async fn test_register_item_namespace_mismatch_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent_id = create_test_agent(pool, "agent-ns-a", "ns-a").await;

    let result = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "item-in-b".into(),
            artifact_type: InventoryType::Branch,
            owner_agent_id: Some(agent_id),
            namespace: Some("ns-b".into()),
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("namespace"));

    pools.close().await;
}

#[tokio::test]
async fn test_register_item_nonexistent_owner_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let result = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "orphan-item".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: Some("nonexistent-agent-id".into()),
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));

    pools.close().await;
}

// --- Get tests ---

#[tokio::test]
async fn test_get_item_by_id() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "get-test".into(),
            artifact_type: InventoryType::Room,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let fetched = inventory::get_item_by_id(pool, &item.id).await.unwrap();
    assert_eq!(fetched.id, item.id);
    assert_eq!(fetched.name, "get-test");

    pools.close().await;
}

#[tokio::test]
async fn test_get_item_not_found() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let result = inventory::get_item_by_id(pool, "does-not-exist").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));

    pools.close().await;
}

// --- List tests ---

#[tokio::test]
async fn test_list_items_empty() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let items = inventory::list_items(pool, &ListItemsFilter::default())
        .await
        .unwrap();
    assert!(items.is_empty());

    pools.close().await;
}

#[tokio::test]
async fn test_list_items_with_filters() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent_id = create_test_agent(pool, "list-agent", "default").await;

    for i in 0..5 {
        inventory::register_item(
            pool,
            RegisterItemRequest {
                name: format!("file-{i}"),
                artifact_type: InventoryType::File,
                owner_agent_id: Some(agent_id.clone()),
                namespace: Some("default".into()),
                path: None,
                metadata: None,
                tags: None,
            },
        )
        .await
        .unwrap();
    }

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "a-branch".into(),
            artifact_type: InventoryType::Branch,
            owner_agent_id: Some(agent_id.clone()),
            namespace: Some("default".into()),
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let all = inventory::list_items(pool, &ListItemsFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 6);

    let files = inventory::list_items(
        pool,
        &ListItemsFilter {
            artifact_type: Some(InventoryType::File),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(files.len(), 5);

    let by_owner = inventory::list_items(
        pool,
        &ListItemsFilter {
            owner_agent_id: Some(agent_id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(by_owner.len(), 6);

    let limited = inventory::list_items(
        pool,
        &ListItemsFilter {
            limit: Some(3),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(limited.len(), 3);

    pools.close().await;
}

#[tokio::test]
async fn test_list_orphaned_items() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "orphan-1".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let agent_id = create_test_agent(pool, "owned-agent", "default").await;
    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "owned-1".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: Some(agent_id),
            namespace: Some("default".into()),
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let orphans = inventory::list_items(
        pool,
        &ListItemsFilter {
            orphaned: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].name, "orphan-1");

    pools.close().await;
}

// --- Update tests ---

#[tokio::test]
async fn test_update_item_name() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "original-name".into(),
            artifact_type: InventoryType::Binary,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let updated = inventory::update_item(
        pool,
        UpdateItemRequest {
            id: item.id.clone(),
            name: Some("new-name".into()),
            path: None,
            metadata: None,
            tags: None,
            status: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name, "new-name");
    assert_eq!(updated.id, item.id);

    pools.close().await;
}

#[tokio::test]
async fn test_update_item_tags() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "tagged-item".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["old-tag".into()]),
        },
    )
    .await
    .unwrap();

    let updated = inventory::update_item(
        pool,
        UpdateItemRequest {
            id: item.id.clone(),
            name: None,
            path: None,
            metadata: None,
            tags: Some(vec!["new-tag-1".into(), "new-tag-2".into()]),
            status: None,
        },
    )
    .await
    .unwrap();

    let tags: Vec<String> = serde_json::from_str(&updated.tags).unwrap();
    assert_eq!(tags, vec!["new-tag-1", "new-tag-2"]);

    pools.close().await;
}

#[tokio::test]
async fn test_update_item_metadata() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "meta-item".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: Some(r#"{"v":"1"}"#.into()),
            tags: None,
        },
    )
    .await
    .unwrap();

    let updated = inventory::update_item(
        pool,
        UpdateItemRequest {
            id: item.id,
            name: None,
            path: None,
            metadata: Some(r#"{"v":"2","extra":"field"}"#.into()),
            tags: None,
            status: None,
        },
    )
    .await
    .unwrap();

    assert!(updated.metadata.as_deref().unwrap().contains("\"v\":\"2\""));

    pools.close().await;
}

#[tokio::test]
async fn test_update_deleted_item_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "to-delete".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    inventory::archive_item(pool, &item.id).await.unwrap();
    inventory::deregister_item(pool, &item.id, false)
        .await
        .unwrap();

    let result = inventory::update_item(
        pool,
        UpdateItemRequest {
            id: item.id,
            name: Some("cant-update".into()),
            path: None,
            metadata: None,
            tags: None,
            status: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("deleted"));

    pools.close().await;
}

// --- Archive tests ---

#[tokio::test]
async fn test_archive_item() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "to-archive".into(),
            artifact_type: InventoryType::Schedule,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let archived = inventory::archive_item(pool, &item.id).await.unwrap();
    assert_eq!(archived.status, "archived");
    assert!(archived.archived_at.is_some());

    pools.close().await;
}

#[tokio::test]
async fn test_archive_non_active_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "already-archived".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    inventory::archive_item(pool, &item.id).await.unwrap();
    let result = inventory::archive_item(pool, &item.id).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("active"));

    pools.close().await;
}

// --- Deregister tests ---

#[tokio::test]
async fn test_deregister_soft() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "soft-del".into(),
            artifact_type: InventoryType::Branch,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    inventory::deregister_item(pool, &item.id, false)
        .await
        .unwrap();

    let fetched = inventory::get_item_by_id(pool, &item.id).await.unwrap();
    assert_eq!(fetched.status, "deleted");

    pools.close().await;
}

#[tokio::test]
async fn test_deregister_hard() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "hard-del".into(),
            artifact_type: InventoryType::Binary,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    inventory::deregister_item(pool, &item.id, true)
        .await
        .unwrap();

    let result = inventory::get_item_by_id(pool, &item.id).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));

    pools.close().await;
}

// --- Search by tags tests ---

#[tokio::test]
async fn test_search_by_single_tag() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "prod-api".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["production".into(), "api".into()]),
        },
    )
    .await
    .unwrap();

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "staging-api".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["staging".into(), "api".into()]),
        },
    )
    .await
    .unwrap();

    let results = inventory::search_by_tags(
        pool,
        &SearchItemsRequest {
            tags: vec!["production".into()],
            artifact_type: None,
            status: None,
            namespace: None,
            limit: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "prod-api");

    pools.close().await;
}

#[tokio::test]
async fn test_search_by_multiple_tags_and_semantics() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "prod-api-team-a".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["production".into(), "api".into(), "team-a".into()]),
        },
    )
    .await
    .unwrap();

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "prod-worker-team-a".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["production".into(), "worker".into(), "team-a".into()]),
        },
    )
    .await
    .unwrap();

    let results = inventory::search_by_tags(
        pool,
        &SearchItemsRequest {
            tags: vec!["production".into(), "api".into()],
            artifact_type: None,
            status: None,
            namespace: None,
            limit: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "prod-api-team-a");

    pools.close().await;
}

#[tokio::test]
async fn test_search_by_tags_with_type_filter() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "prod-image".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["production".into()]),
        },
    )
    .await
    .unwrap();

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "prod-binary".into(),
            artifact_type: InventoryType::Binary,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["production".into()]),
        },
    )
    .await
    .unwrap();

    let results = inventory::search_by_tags(
        pool,
        &SearchItemsRequest {
            tags: vec!["production".into()],
            artifact_type: Some(InventoryType::Binary),
            status: None,
            namespace: None,
            limit: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "prod-binary");

    pools.close().await;
}

#[tokio::test]
async fn test_search_empty_tags_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let result = inventory::search_by_tags(
        pool,
        &SearchItemsRequest {
            tags: vec![],
            artifact_type: None,
            status: None,
            namespace: None,
            limit: None,
        },
    )
    .await;

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("at least one tag"));

    pools.close().await;
}

// --- FTS search tests ---

#[tokio::test]
async fn test_fts_search_by_name() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "kubernetes-deployment".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "docker-compose".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let results = inventory::search_fts(pool, "kubernetes", None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "kubernetes-deployment");

    pools.close().await;
}

#[tokio::test]
async fn test_fts_search_by_type() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "my-image".into(),
            artifact_type: InventoryType::DockerImage,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let results = inventory::search_fts(pool, "docker", None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "my-image");

    pools.close().await;
}

#[tokio::test]
async fn test_fts_search_empty_query_fails() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let result = inventory::search_fts(pool, "  ", None, None).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cannot be empty"));

    pools.close().await;
}

// --- Transfer ownership tests ---

#[tokio::test]
async fn test_transfer_ownership_to_agent() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent_a = create_test_agent(pool, "agent-a", "default").await;
    let agent_b = create_test_agent(pool, "agent-b", "default").await;

    for i in 0..3 {
        inventory::register_item(
            pool,
            RegisterItemRequest {
                name: format!("item-{i}"),
                artifact_type: InventoryType::File,
                owner_agent_id: Some(agent_a.clone()),
                namespace: Some("default".into()),
                path: None,
                metadata: None,
                tags: None,
            },
        )
        .await
        .unwrap();
    }

    let transferred = inventory::transfer_ownership(pool, &agent_a, Some(&agent_b))
        .await
        .unwrap();
    assert_eq!(transferred, 3);

    let items_b = inventory::list_items(
        pool,
        &ListItemsFilter {
            owner_agent_id: Some(agent_b.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(items_b.len(), 3);

    let items_a = inventory::list_items(
        pool,
        &ListItemsFilter {
            owner_agent_id: Some(agent_a),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(items_a.len(), 0);

    pools.close().await;
}

#[tokio::test]
async fn test_transfer_ownership_orphan() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let agent_id = create_test_agent(pool, "leaving-agent", "default").await;

    for i in 0..2 {
        inventory::register_item(
            pool,
            RegisterItemRequest {
                name: format!("artifact-{i}"),
                artifact_type: InventoryType::Branch,
                owner_agent_id: Some(agent_id.clone()),
                namespace: Some("default".into()),
                path: None,
                metadata: None,
                tags: None,
            },
        )
        .await
        .unwrap();
    }

    let transferred = inventory::transfer_ownership(pool, &agent_id, None)
        .await
        .unwrap();
    assert_eq!(transferred, 2);

    let orphans = inventory::list_items(
        pool,
        &ListItemsFilter {
            orphaned: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(orphans.len(), 2);
    assert_eq!(orphans[0].status, "archived");
    assert!(orphans[0].archived_at.is_some());

    pools.close().await;
}

// --- Lifecycle tests ---

#[tokio::test]
async fn test_full_lifecycle() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "lifecycle-item".into(),
            artifact_type: InventoryType::Worktree,
            owner_agent_id: None,
            namespace: None,
            path: Some("/tmp/wt-1".into()),
            metadata: None,
            tags: Some(vec!["ephemeral".into()]),
        },
    )
    .await
    .unwrap();
    assert_eq!(item.status, "active");

    let archived = inventory::archive_item(pool, &item.id).await.unwrap();
    assert_eq!(archived.status, "archived");
    assert!(archived.archived_at.is_some());

    inventory::deregister_item(pool, &item.id, false)
        .await
        .unwrap();
    let deleted = inventory::get_item_by_id(pool, &item.id).await.unwrap();
    assert_eq!(deleted.status, "deleted");

    inventory::deregister_item(pool, &item.id, true)
        .await
        .unwrap();
    let result = inventory::get_item_by_id(pool, &item.id).await;
    assert!(result.is_err());

    pools.close().await;
}

// --- Tags normalization tests ---

#[tokio::test]
async fn test_tags_are_lowercased() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    let item = inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "caps-tags".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: None,
            path: None,
            metadata: None,
            tags: Some(vec!["Production".into(), "Team-A".into()]),
        },
    )
    .await
    .unwrap();

    let tags: Vec<String> = serde_json::from_str(&item.tags).unwrap();
    assert_eq!(tags, vec!["production", "team-a"]);

    let results = inventory::search_by_tags(
        pool,
        &SearchItemsRequest {
            tags: vec!["PRODUCTION".into()],
            artifact_type: None,
            status: None,
            namespace: None,
            limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(results.len(), 1);

    pools.close().await;
}

// --- All inventory types ---

#[tokio::test]
async fn test_all_inventory_types_parse() {
    let types = [
        ("worktree", InventoryType::Worktree),
        ("room", InventoryType::Room),
        ("schedule", InventoryType::Schedule),
        ("branch", InventoryType::Branch),
        ("file", InventoryType::File),
        ("docker-image", InventoryType::DockerImage),
        ("binary", InventoryType::Binary),
    ];

    for (s, expected) in types {
        let parsed: InventoryType = s.parse().unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_str(), s);
    }
}

#[tokio::test]
async fn test_all_inventory_statuses_parse() {
    let statuses = [
        ("active", InventoryStatus::Active),
        ("archived", InventoryStatus::Archived),
        ("deleted", InventoryStatus::Deleted),
    ];

    for (s, expected) in statuses {
        let parsed: InventoryStatus = s.parse().unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_str(), s);
    }
}

#[tokio::test]
async fn test_invalid_type_parse_fails() {
    let result = "foobar".parse::<InventoryType>();
    assert!(result.is_err());
}

// --- Namespace scoping test ---

#[tokio::test]
async fn test_namespace_scoping() {
    let (pools, _dir) = setup().await;
    let pool = &pools.fts;

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "ns-a-item".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: Some("ns-a".into()),
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    inventory::register_item(
        pool,
        RegisterItemRequest {
            name: "ns-b-item".into(),
            artifact_type: InventoryType::File,
            owner_agent_id: None,
            namespace: Some("ns-b".into()),
            path: None,
            metadata: None,
            tags: None,
        },
    )
    .await
    .unwrap();

    let ns_a_items = inventory::list_items(
        pool,
        &ListItemsFilter {
            namespace: Some("ns-a".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ns_a_items.len(), 1);
    assert_eq!(ns_a_items[0].name, "ns-a-item");

    pools.close().await;
}
