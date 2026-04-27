use nous_core::db::MemoryDb;

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None, 384).expect("failed to open in-memory db")
}

#[test]
fn tables_exist() {
    let db = open_test_db();
    let conn = db.connection();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let expected = [
        "access_log",
        "categories",
        "memories",
        "memory_chunks",
        "memory_embeddings",
        "memory_tags",
        "models",
        "relationships",
        "tags",
        "workspaces",
    ];
    for table in &expected {
        assert!(
            tables.contains(&table.to_string()),
            "missing table: {table}"
        );
    }
}

#[test]
fn fts5_table_exists() {
    let db = open_test_db();
    let conn = db.connection();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='memories_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "memories_fts FTS5 table should exist");
}

#[test]
fn triggers_exist() {
    let db = open_test_db();
    let conn = db.connection();
    let triggers: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let expected = ["memories_ad", "memories_ai", "memories_au", "tags_cleanup"];
    for trigger in &expected {
        assert!(
            triggers.contains(&trigger.to_string()),
            "missing trigger: {trigger}"
        );
    }
}

#[test]
fn indexes_exist() {
    let db = open_test_db();
    let conn = db.connection();
    let indexes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let expected = [
        "idx_memories_archived",
        "idx_memories_category",
        "idx_memories_created",
        "idx_memories_importance",
        "idx_memories_type",
        "idx_memories_workspace",
        "idx_memory_chunks_memory",
        "idx_memory_tags_memory",
        "idx_memory_tags_tag",
        "idx_relationships_source",
        "idx_relationships_target",
    ];
    for idx in &expected {
        assert!(
            indexes.contains(&idx.to_string()),
            "missing index: {idx}, found: {indexes:?}"
        );
    }
}

#[test]
fn seed_categories_top_level_count() {
    let db = open_test_db();
    let conn = db.connection();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM categories WHERE parent_id IS NULL AND source='system'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        count >= 15,
        "expected at least 15 top-level categories, got {count}"
    );
}

#[test]
fn seed_categories_infrastructure_children() {
    let db = open_test_db();
    let conn = db.connection();

    let parent_id: i64 = conn
        .query_row(
            "SELECT id FROM categories WHERE name='infrastructure' AND parent_id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let children: Vec<String> = conn
        .prepare("SELECT name FROM categories WHERE parent_id=? ORDER BY name")
        .unwrap()
        .query_map([parent_id], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let expected = ["compute", "k8s", "networking", "storage"];
    for child in &expected {
        assert!(
            children.contains(&child.to_string()),
            "infrastructure missing child: {child}, found: {children:?}"
        );
    }
}

#[test]
fn seed_categories_parent_links_correct() {
    let db = open_test_db();
    let conn = db.connection();

    let pairs = [
        ("data-platform", "etl"),
        ("ci-cd", "pipelines"),
        ("security", "auth"),
        ("tooling", "dev-environment"),
        ("observability", "monitoring"),
    ];

    for (parent_name, child_name) in &pairs {
        let parent_id: i64 = conn
            .query_row(
                "SELECT id FROM categories WHERE name=? AND parent_id IS NULL",
                [parent_name],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| panic!("parent category '{parent_name}' not found"));

        let child_parent: i64 = conn
            .query_row(
                "SELECT parent_id FROM categories WHERE name=?",
                [child_name],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| panic!("child category '{child_name}' not found"));

        assert_eq!(
            child_parent, parent_id,
            "{child_name} should be child of {parent_name}"
        );
    }
}

#[test]
fn seed_categories_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let path_str = path.to_str().unwrap();

    let db = MemoryDb::open(path_str, None, 384).unwrap();
    let count_before: i64 = db
        .connection()
        .query_row("SELECT count(*) FROM categories", [], |row| row.get(0))
        .unwrap();
    drop(db);

    let db2 = MemoryDb::open(path_str, None, 384).unwrap();
    let count_after: i64 = db2
        .connection()
        .query_row("SELECT count(*) FROM categories", [], |row| row.get(0))
        .unwrap();

    assert_eq!(
        count_before, count_after,
        "re-opening should not duplicate categories"
    );
}

#[test]
fn all_source_system() {
    let db = open_test_db();
    let conn = db.connection();
    let non_system: i64 = conn
        .query_row(
            "SELECT count(*) FROM categories WHERE source != 'system'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        non_system, 0,
        "all seeded categories should have source='system'"
    );
}
