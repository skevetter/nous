use nous_core::classify::CategoryClassifier;
use nous_core::db::MemoryDb;
use nous_core::embed::{EmbeddingBackend, MockEmbedding};
use nous_core::types::{CategorySource, MemoryType, NewMemory};

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None).expect("failed to open in-memory db")
}

fn mock_embedder() -> MockEmbedding {
    MockEmbedding::new(64)
}

#[test]
fn all_categories_have_embeddings_after_construction() {
    let db = open_test_db();
    let embedder = mock_embedder();
    let classifier = CategoryClassifier::new(&db, &embedder).unwrap();

    for (cat, emb) in classifier.cache().values() {
        assert!(
            !emb.is_empty(),
            "category '{}' should have a non-empty embedding",
            cat.name
        );
    }

    let conn = db.connection();
    let null_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM categories WHERE embedding IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(null_count, 0, "all categories should have embeddings in DB");
}

#[test]
fn refresh_picks_up_new_category() {
    let db = open_test_db();
    let embedder = mock_embedder();
    let mut classifier = CategoryClassifier::new(&db, &embedder).unwrap();

    let initial_count = classifier.cache().len();
    db.category_add(
        "custom-cat",
        None,
        Some("a custom category"),
        CategorySource::User,
    )
    .unwrap();

    classifier.refresh(&db, &embedder).unwrap();
    assert_eq!(
        classifier.cache().len(),
        initial_count + 1,
        "cache should include the new category after refresh"
    );

    let has_custom = classifier
        .cache()
        .values()
        .any(|(cat, emb)| cat.name == "custom-cat" && !emb.is_empty());
    assert!(has_custom, "new category should have an embedding");
}

#[test]
fn classify_returns_some_for_known_category() {
    let db = open_test_db();
    let embedder = mock_embedder();
    let classifier = CategoryClassifier::new(&db, &embedder).unwrap();

    let emb = embedder.embed_one("infrastructure networking").unwrap();
    let result = classifier.classify(&emb);
    assert!(
        result.is_some(),
        "classify should return Some for a relevant embedding"
    );
}

#[test]
fn classify_returns_none_for_zero_vector() {
    let db = open_test_db();
    let embedder = mock_embedder();
    let classifier = CategoryClassifier::new(&db, &embedder).unwrap();

    let zero = vec![0.0f32; 64];
    let result = classifier.classify(&zero);
    assert!(
        result.is_none(),
        "zero vector should not match any category"
    );
}

#[test]
fn category_add_top_level_appears_in_list() {
    let db = open_test_db();
    let id = db
        .category_add("my-category", None, None, CategorySource::User)
        .unwrap();
    assert!(id > 0);

    let trees = db.category_list(None).unwrap();
    let found = trees.iter().any(|t| t.category.name == "my-category");
    assert!(found, "new top-level category should appear in list");
}

#[test]
fn category_add_child_links_to_parent() {
    let db = open_test_db();
    let parent_id = db
        .category_add("parent-cat", None, None, CategorySource::User)
        .unwrap();
    let child_id = db
        .category_add("child-cat", Some(parent_id), None, CategorySource::User)
        .unwrap();
    assert_ne!(parent_id, child_id);

    let trees = db.category_list(None).unwrap();
    let parent_tree = trees.iter().find(|t| t.category.id == parent_id).unwrap();
    assert_eq!(parent_tree.children.len(), 1);
    assert_eq!(parent_tree.children[0].category.name, "child-cat");
}

#[test]
fn category_suggest_creates_agent_category_and_assigns() {
    let db = open_test_db();

    let mem_id = db
        .store(&NewMemory {
            title: "test memory".into(),
            content: "some content".into(),
            memory_type: MemoryType::Fact,
            source: None,
            importance: Default::default(),
            confidence: Default::default(),
            tags: vec![],
            workspace_path: None,
            session_id: None,
            trace_id: None,
            agent_id: None,
            agent_model: None,
            valid_from: None,
            category_id: None,
        })
        .unwrap();

    let cat_id = db
        .category_suggest("suggested-cat", "an agent-suggested category", &mem_id)
        .unwrap();

    let recalled = db.recall(&mem_id).unwrap().unwrap();
    assert_eq!(recalled.memory.category_id, Some(cat_id));
    assert_eq!(recalled.category.unwrap().source, CategorySource::Agent);
}

#[test]
fn category_list_with_source_filter() {
    let db = open_test_db();
    db.category_add("agent-cat", None, None, CategorySource::Agent)
        .unwrap();

    let agent_trees = db.category_list(Some(CategorySource::Agent)).unwrap();
    assert!(!agent_trees.is_empty(), "should have agent categories");
    for tree in &agent_trees {
        assert_eq!(tree.category.source, CategorySource::Agent);
    }

    let system_trees = db.category_list(Some(CategorySource::System)).unwrap();
    let has_agent = system_trees
        .iter()
        .any(|t| t.category.source == CategorySource::Agent);
    assert!(
        !has_agent,
        "system filter should not include agent categories"
    );
}

#[test]
fn category_tree_nesting_correct() {
    let db = open_test_db();
    let trees = db.category_list(None).unwrap();

    let infra = trees
        .iter()
        .find(|t| t.category.name == "infrastructure")
        .expect("infrastructure should be a top-level category");

    assert!(
        !infra.children.is_empty(),
        "infrastructure should have children"
    );
    let child_names: Vec<&str> = infra
        .children
        .iter()
        .map(|c| c.category.name.as_str())
        .collect();
    assert!(
        child_names.contains(&"k8s"),
        "infrastructure should contain k8s"
    );
    assert!(
        child_names.contains(&"networking"),
        "infrastructure should contain networking"
    );

    for child in &infra.children {
        assert_eq!(child.category.parent_id, Some(infra.category.id));
    }
}
