use nous_core::types::{
    AccessLogEntry, Category, CategorySource, CategoryTree, Confidence, ContextEntry, Importance,
    Memory, MemoryChunk, MemoryPatch, MemoryTag, MemoryType, MemoryWithRelations, Model, NewMemory,
    RelationType, Relationship, SearchFilters, SearchMode, SearchResult, Tag, Workspace,
};

#[test]
fn memory_type_display_and_from_str() {
    let variants = [
        (MemoryType::Decision, "decision"),
        (MemoryType::Convention, "convention"),
        (MemoryType::Bugfix, "bugfix"),
        (MemoryType::Architecture, "architecture"),
        (MemoryType::Fact, "fact"),
        (MemoryType::Observation, "observation"),
    ];
    for (variant, expected) in &variants {
        assert_eq!(variant.to_string(), *expected);
        let parsed: MemoryType = expected.parse().unwrap();
        assert_eq!(&parsed, variant);
    }
    assert!("invalid".parse::<MemoryType>().is_err());
}

#[test]
fn importance_display_and_from_str() {
    let variants = [
        (Importance::Low, "low"),
        (Importance::Moderate, "moderate"),
        (Importance::High, "high"),
    ];
    for (variant, expected) in &variants {
        assert_eq!(variant.to_string(), *expected);
        let parsed: Importance = expected.parse().unwrap();
        assert_eq!(&parsed, variant);
    }
    assert!("invalid".parse::<Importance>().is_err());
}

#[test]
fn confidence_display_and_from_str() {
    let variants = [
        (Confidence::Low, "low"),
        (Confidence::Moderate, "moderate"),
        (Confidence::High, "high"),
    ];
    for (variant, expected) in &variants {
        assert_eq!(variant.to_string(), *expected);
        let parsed: Confidence = expected.parse().unwrap();
        assert_eq!(&parsed, variant);
    }
    assert!("invalid".parse::<Confidence>().is_err());
}

#[test]
fn relation_type_display_and_from_str() {
    let variants = [
        (RelationType::Related, "related"),
        (RelationType::Supersedes, "supersedes"),
        (RelationType::Contradicts, "contradicts"),
        (RelationType::DependsOn, "depends_on"),
    ];
    for (variant, expected) in &variants {
        assert_eq!(variant.to_string(), *expected);
        let parsed: RelationType = expected.parse().unwrap();
        assert_eq!(&parsed, variant);
    }
    assert!("invalid".parse::<RelationType>().is_err());
}

#[test]
fn category_source_display_and_from_str() {
    let variants = [
        (CategorySource::System, "system"),
        (CategorySource::User, "user"),
        (CategorySource::Agent, "agent"),
    ];
    for (variant, expected) in &variants {
        assert_eq!(variant.to_string(), *expected);
        let parsed: CategorySource = expected.parse().unwrap();
        assert_eq!(&parsed, variant);
    }
    assert!("invalid".parse::<CategorySource>().is_err());
}

#[test]
fn search_mode_display_and_from_str() {
    let variants = [
        (SearchMode::Fts, "fts"),
        (SearchMode::Semantic, "semantic"),
        (SearchMode::Hybrid, "hybrid"),
    ];
    for (variant, expected) in &variants {
        assert_eq!(variant.to_string(), *expected);
        let parsed: SearchMode = expected.parse().unwrap();
        assert_eq!(&parsed, variant);
    }
    assert!("invalid".parse::<SearchMode>().is_err());
}

#[test]
fn memory_roundtrip_json() {
    let mem = Memory {
        id: "mem_01".into(),
        title: "Test memory".into(),
        content: "Some content".into(),
        memory_type: MemoryType::Decision,
        source: Some("agent".into()),
        importance: Importance::High,
        confidence: Confidence::Moderate,
        workspace_id: Some(1),
        session_id: None,
        trace_id: None,
        agent_id: Some("claude".into()),
        agent_model: Some("opus".into()),
        valid_from: None,
        valid_until: None,
        archived: false,
        category_id: Some(3),
        created_at: "2025-01-01T00:00:00Z".into(),
        updated_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&mem).unwrap();
    let back: Memory = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, mem.id);
    assert_eq!(back.title, mem.title);
    assert_eq!(back.memory_type, mem.memory_type);
    assert_eq!(back.importance, mem.importance);
}

#[test]
fn new_memory_defaults() {
    let nm = NewMemory {
        title: "test".into(),
        content: "content".into(),
        memory_type: MemoryType::Fact,
        source: None,
        importance: Importance::default(),
        confidence: Confidence::default(),
        tags: vec![],
        workspace_path: None,
        session_id: None,
        trace_id: None,
        agent_id: None,
        agent_model: None,
        valid_from: None,
        category_id: None,
    };
    assert_eq!(nm.importance, Importance::Moderate);
    assert_eq!(nm.confidence, Confidence::Moderate);
}

#[test]
fn memory_patch_roundtrip_json() {
    let patch = MemoryPatch {
        title: Some("updated".into()),
        content: None,
        tags: Some(vec!["new-tag".into()]),
        importance: Some(Importance::High),
        confidence: None,
        valid_until: None,
    };
    let json = serde_json::to_string(&patch).unwrap();
    let back: MemoryPatch = serde_json::from_str(&json).unwrap();
    assert_eq!(back.title.as_deref(), Some("updated"));
    assert!(back.content.is_none());
}

#[test]
fn tag_roundtrip_json() {
    let tag = Tag {
        id: 1,
        name: "rust".into(),
    };
    let json = serde_json::to_string(&tag).unwrap();
    let back: Tag = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, 1);
    assert_eq!(back.name, "rust");
}

#[test]
fn memory_tag_roundtrip_json() {
    let mt = MemoryTag {
        memory_id: "m1".into(),
        tag_id: 5,
    };
    let json = serde_json::to_string(&mt).unwrap();
    let back: MemoryTag = serde_json::from_str(&json).unwrap();
    assert_eq!(back.memory_id, "m1");
    assert_eq!(back.tag_id, 5);
}

#[test]
fn relationship_roundtrip_json() {
    let rel = Relationship {
        id: 1,
        source_id: "m1".into(),
        target_id: "m2".into(),
        relation_type: RelationType::Supersedes,
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&rel).unwrap();
    let back: Relationship = serde_json::from_str(&json).unwrap();
    assert_eq!(back.relation_type, RelationType::Supersedes);
}

#[test]
fn workspace_roundtrip_json() {
    let ws = Workspace {
        id: 1,
        path: "/home/user/project".into(),
        name: Some("myproject".into()),
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&ws).unwrap();
    let back: Workspace = serde_json::from_str(&json).unwrap();
    assert_eq!(back.path, "/home/user/project");
}

#[test]
fn category_roundtrip_json() {
    let cat = Category {
        id: 1,
        name: "infrastructure".into(),
        parent_id: None,
        source: CategorySource::System,
        description: None,
        embedding: None,
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&cat).unwrap();
    let back: Category = serde_json::from_str(&json).unwrap();
    assert_eq!(back.name, "infrastructure");
    assert!(back.parent_id.is_none());
}

#[test]
fn access_log_entry_roundtrip_json() {
    let entry = AccessLogEntry {
        id: 1,
        memory_id: "m1".into(),
        accessed_at: "2025-01-01T00:00:00Z".into(),
        access_type: "search".into(),
        session_id: None,
    };
    let json = serde_json::to_string(&entry).unwrap();
    let back: AccessLogEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.access_type, "search");
}

#[test]
fn model_roundtrip_json() {
    let m = Model {
        id: 1,
        name: "all-MiniLM-L6-v2".into(),
        dimensions: 384,
        max_tokens: 256,
        variant: Some("fp16".into()),
        chunk_size: 512,
        chunk_overlap: 64,
        active: false,
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&m).unwrap();
    let back: Model = serde_json::from_str(&json).unwrap();
    assert_eq!(back.dimensions, 384);
}

#[test]
fn memory_chunk_roundtrip_json() {
    let chunk = MemoryChunk {
        id: "c1".into(),
        memory_id: "m1".into(),
        chunk_index: 0,
        content: "chunk text".into(),
        token_count: 50,
        model_id: 1,
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&chunk).unwrap();
    let back: MemoryChunk = serde_json::from_str(&json).unwrap();
    assert_eq!(back.chunk_index, 0);
}

#[test]
fn memory_with_relations_roundtrip_json() {
    let mwr = MemoryWithRelations {
        memory: Memory {
            id: "m1".into(),
            title: "test".into(),
            content: "content".into(),
            memory_type: MemoryType::Fact,
            source: None,
            importance: Importance::Moderate,
            confidence: Confidence::Moderate,
            workspace_id: None,
            session_id: None,
            trace_id: None,
            agent_id: None,
            agent_model: None,
            valid_from: None,
            valid_until: None,
            archived: false,
            category_id: None,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
        },
        tags: vec!["rust".into()],
        relationships: vec![],
        category: None,
        access_count: 5,
    };
    let json = serde_json::to_string(&mwr).unwrap();
    let back: MemoryWithRelations = serde_json::from_str(&json).unwrap();
    assert_eq!(back.tags, vec!["rust"]);
    assert_eq!(back.access_count, 5);
}

#[test]
fn search_result_roundtrip_json() {
    let sr = SearchResult {
        memory: Memory {
            id: "m1".into(),
            title: "test".into(),
            content: "content".into(),
            memory_type: MemoryType::Convention,
            source: None,
            importance: Importance::Low,
            confidence: Confidence::High,
            workspace_id: None,
            session_id: None,
            trace_id: None,
            agent_id: None,
            agent_model: None,
            valid_from: None,
            valid_until: None,
            archived: false,
            category_id: None,
            created_at: "2025-01-01T00:00:00Z".into(),
            updated_at: "2025-01-01T00:00:00Z".into(),
        },
        tags: vec![],
        rank: 0.95,
    };
    let json = serde_json::to_string(&sr).unwrap();
    let back: SearchResult = serde_json::from_str(&json).unwrap();
    assert!((back.rank - 0.95).abs() < f64::EPSILON);
}

#[test]
fn search_filters_defaults() {
    let sf = SearchFilters::default();
    assert!(sf.memory_type.is_none());
    assert_eq!(sf.archived, Some(false));
    assert_eq!(sf.limit, Some(20));
}

#[test]
fn context_entry_roundtrip_json() {
    let ce = ContextEntry {
        id: "m1".into(),
        title: "test".into(),
        content: Some("content".into()),
        memory_type: MemoryType::Observation,
        importance: Importance::High,
        created_at: "2025-01-01T00:00:00Z".into(),
    };
    let json = serde_json::to_string(&ce).unwrap();
    let back: ContextEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.memory_type, MemoryType::Observation);
}

#[test]
fn category_tree_roundtrip_json() {
    let tree = CategoryTree {
        category: Category {
            id: 1,
            name: "infrastructure".into(),
            parent_id: None,
            source: CategorySource::System,
            description: None,
            embedding: None,
            created_at: "2025-01-01T00:00:00Z".into(),
        },
        children: vec![CategoryTree {
            category: Category {
                id: 2,
                name: "k8s".into(),
                parent_id: Some(1),
                source: CategorySource::System,
                description: None,
                embedding: None,
                created_at: "2025-01-01T00:00:00Z".into(),
            },
            children: vec![],
        }],
    };
    let json = serde_json::to_string(&tree).unwrap();
    let back: CategoryTree = serde_json::from_str(&json).unwrap();
    assert_eq!(back.children.len(), 1);
    assert_eq!(back.children[0].category.name, "k8s");
}

#[test]
fn new_memory_roundtrip_json() {
    let nm = NewMemory {
        title: "use uv for packages".into(),
        content: "always use uv, not pip".into(),
        memory_type: MemoryType::Convention,
        source: Some("agent".into()),
        importance: Importance::High,
        confidence: Confidence::Moderate,
        tags: vec!["python".into(), "tooling".into()],
        workspace_path: Some("/home/user/project".into()),
        session_id: Some("sess-1".into()),
        trace_id: None,
        agent_id: Some("claude".into()),
        agent_model: Some("opus".into()),
        valid_from: Some("2025-01-01".into()),
        category_id: Some(5),
    };
    let json = serde_json::to_string(&nm).unwrap();
    let back: NewMemory = serde_json::from_str(&json).unwrap();
    assert_eq!(back, nm);
}

#[test]
fn search_filters_roundtrip_json() {
    let sf = SearchFilters {
        memory_type: Some(MemoryType::Decision),
        category_id: Some(3),
        workspace_id: Some(1),
        importance: Some(Importance::High),
        confidence: Some(Confidence::Low),
        tags: Some(vec!["rust".into()]),
        archived: Some(false),
        since: Some("2025-01-01".into()),
        until: Some("2025-12-31".into()),
        valid_only: Some(true),
        limit: Some(10),
    };
    let json = serde_json::to_string(&sf).unwrap();
    let back: SearchFilters = serde_json::from_str(&json).unwrap();
    assert_eq!(back, sf);
}

#[test]
fn enum_serde_lowercase() {
    let val = serde_json::to_value(MemoryType::Bugfix).unwrap();
    assert_eq!(val, "bugfix");

    let val = serde_json::to_value(Importance::High).unwrap();
    assert_eq!(val, "high");

    let val = serde_json::to_value(RelationType::DependsOn).unwrap();
    assert_eq!(val, "depends_on");

    let val = serde_json::to_value(CategorySource::Agent).unwrap();
    assert_eq!(val, "agent");

    let val = serde_json::to_value(SearchMode::Hybrid).unwrap();
    assert_eq!(val, "hybrid");
}
