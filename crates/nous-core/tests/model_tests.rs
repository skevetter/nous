use nous_core::db::MemoryDb;

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None).expect("failed to open in-memory db")
}

#[test]
fn register_model_defaults_inactive() {
    let db = open_test_db();
    let id = db
        .register_model("all-MiniLM-L6-v2", Some("fp16"), 384, 256, 512, 64)
        .unwrap();
    assert!(id > 0);

    let model = db
        .list_models()
        .unwrap()
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();
    assert!(!model.active);
    assert_eq!(model.name, "all-MiniLM-L6-v2");
    assert_eq!(model.variant.as_deref(), Some("fp16"));
    assert_eq!(model.dimensions, 384);
    assert_eq!(model.max_tokens, 256);
    assert_eq!(model.chunk_size, 512);
    assert_eq!(model.chunk_overlap, 64);
}

#[test]
fn activate_model_returns_it_as_active() {
    let db = open_test_db();
    let id = db
        .register_model("model-a", None, 768, 8192, 512, 64)
        .unwrap();
    db.activate_model(id).unwrap();

    let active = db
        .active_model()
        .unwrap()
        .expect("should have active model");
    assert_eq!(active.id, id);
    assert!(active.active);
}

#[test]
fn activating_b_deactivates_a() {
    let db = open_test_db();
    let a = db
        .register_model("model-a", None, 384, 8192, 512, 64)
        .unwrap();
    db.activate_model(a).unwrap();

    let b = db
        .register_model("model-b", Some("q8"), 768, 8192, 256, 32)
        .unwrap();
    db.activate_model(b).unwrap();

    let active = db
        .active_model()
        .unwrap()
        .expect("should have active model");
    assert_eq!(active.id, b);

    let models = db.list_models().unwrap();
    let model_a = models.iter().find(|m| m.id == a).unwrap();
    assert!(!model_a.active);
}

#[test]
fn deactivate_leaves_no_active() {
    let db = open_test_db();
    let id = db
        .register_model("model-a", None, 384, 8192, 512, 64)
        .unwrap();
    db.activate_model(id).unwrap();
    db.deactivate_model(id).unwrap();

    assert!(db.active_model().unwrap().is_none());
}

#[test]
fn list_models_returns_all() {
    let db = open_test_db();
    let a = db
        .register_model("model-a", None, 384, 8192, 512, 64)
        .unwrap();
    let b = db
        .register_model("model-b", Some("q4"), 768, 8192, 256, 32)
        .unwrap();

    let models = db.list_models().unwrap();
    assert!(models.len() >= 2);

    let ma = models.iter().find(|m| m.id == a).unwrap();
    assert_eq!(ma.dimensions, 384);
    assert_eq!(ma.chunk_size, 512);
    assert_eq!(ma.chunk_overlap, 64);
    assert!(ma.variant.is_none());

    let mb = models.iter().find(|m| m.id == b).unwrap();
    assert_eq!(mb.dimensions, 768);
    assert_eq!(mb.chunk_size, 256);
    assert_eq!(mb.chunk_overlap, 32);
    assert_eq!(mb.variant.as_deref(), Some("q4"));
}

#[test]
fn validation_rejects_bad_dimensions() {
    let db = open_test_db();
    let err = db.register_model("bad", None, 0, 8192, 512, 64);
    assert!(err.is_err());

    let err = db.register_model("bad", None, -1, 8192, 512, 64);
    assert!(err.is_err());
}

#[test]
fn validation_rejects_overlap_ge_chunk_size() {
    let db = open_test_db();
    let err = db.register_model("bad", None, 384, 8192, 512, 512);
    assert!(err.is_err());

    let err = db.register_model("bad", None, 384, 8192, 512, 600);
    assert!(err.is_err());
}

#[test]
fn activate_nonexistent_model_errors() {
    let db = open_test_db();
    let a = db
        .register_model("model-a", None, 384, 8192, 512, 64)
        .unwrap();
    db.activate_model(a).unwrap();

    let err = db.activate_model(9999);
    assert!(err.is_err());

    let active = db
        .active_model()
        .unwrap()
        .expect("original model still active");
    assert_eq!(active.id, a);
}

#[test]
fn register_model_stores_separate_max_tokens() {
    let db = open_test_db();
    let id = db
        .register_model("bge-small", Some("fp16"), 384, 512, 256, 32)
        .unwrap();

    let model = db
        .list_models()
        .unwrap()
        .into_iter()
        .find(|m| m.id == id)
        .unwrap();

    assert_eq!(model.dimensions, 384);
    assert_eq!(model.max_tokens, 512);
    assert_ne!(
        model.dimensions, model.max_tokens,
        "dimensions and max_tokens should be stored independently"
    );
}

#[test]
fn multi_model_isolation() {
    let db = open_test_db();
    let a = db
        .register_model("model-encoder", None, 384, 512, 256, 32)
        .unwrap();
    let b = db
        .register_model("model-decoder", Some("q4f16"), 1024, 8192, 512, 64)
        .unwrap();

    db.activate_model(a).unwrap();
    let active = db.active_model().unwrap().expect("should have active");
    assert_eq!(active.id, a);
    assert_eq!(active.name, "model-encoder");
    assert_eq!(active.dimensions, 384);

    db.activate_model(b).unwrap();
    let active = db.active_model().unwrap().expect("should have active");
    assert_eq!(active.id, b);
    assert_eq!(active.name, "model-decoder");
    assert_eq!(active.dimensions, 1024);
    assert_eq!(active.max_tokens, 8192);

    // Verify model A is no longer active
    let models = db.list_models().unwrap();
    let model_a = models.iter().find(|m| m.id == a).unwrap();
    assert!(!model_a.active, "model-encoder should be deactivated");
}
