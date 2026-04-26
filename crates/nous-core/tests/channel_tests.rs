use std::collections::HashSet;

use nous_core::channel::{WriteChannel, WriteOp};
use nous_core::db::MemoryDb;
use nous_core::types::{Importance, MemoryPatch, MemoryType, NewMemory, RelationType};
use tempfile::NamedTempFile;
use tokio::sync::oneshot;

fn temp_db() -> (MemoryDb, NamedTempFile) {
    let file = NamedTempFile::new().expect("failed to create temp file");
    let db = MemoryDb::open(file.path().to_str().unwrap(), None).expect("failed to open db");
    (db, file)
}

fn minimal_memory() -> NewMemory {
    NewMemory {
        title: "test title".into(),
        content: "test content".into(),
        memory_type: MemoryType::Decision,
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
    }
}

#[tokio::test]
async fn store_via_channel() {
    let (db, _file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    let id = ch.store(minimal_memory()).await.unwrap();
    assert!(!id.to_string().is_empty());

    drop(ch);
    handle.await.unwrap();
}

#[tokio::test]
async fn concurrent_stores() {
    let (db, _file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let ch = ch.clone();
        handles.push(tokio::spawn(async move {
            ch.store(minimal_memory()).await.unwrap()
        }));
    }

    let mut ids = HashSet::new();
    for h in handles {
        let id = h.await.unwrap();
        ids.insert(id.to_string());
    }
    assert_eq!(ids.len(), 10);

    drop(ch);
    handle.await.unwrap();
}

#[tokio::test]
async fn batching_under_load() {
    let (db, _file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let ch = ch.clone();
        handles.push(tokio::spawn(async move {
            ch.store(minimal_memory()).await.unwrap()
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    let batches = ch.batch_count();
    assert!(
        batches < 10,
        "expected fewer than 10 batches, got {batches}"
    );

    drop(ch);
    handle.await.unwrap();
}

#[tokio::test]
async fn backpressure_at_capacity() {
    let (db, _file) = temp_db();
    let (ch, _handle) = WriteChannel::new(db);
    let tx = ch.sender().clone();

    for _ in 0..256 {
        let (resp_tx, _) = oneshot::channel();
        tx.try_send(WriteOp::Store(minimal_memory(), resp_tx))
            .expect("should accept up to 256 ops");
    }

    let (resp_tx, _) = oneshot::channel();
    let result = tx.try_send(WriteOp::Store(minimal_memory(), resp_tx));
    assert!(result.is_err(), "257th send should fail (channel full)");
}

#[tokio::test]
async fn clean_shutdown() {
    let (db, _file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    ch.store(minimal_memory()).await.unwrap();

    drop(ch);
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;
    assert!(
        result.is_ok(),
        "worker should exit after channel is dropped"
    );
    result.unwrap().unwrap();
}

#[tokio::test]
async fn update_via_channel() {
    let (db, file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    let id = ch.store(minimal_memory()).await.unwrap();

    let patch = MemoryPatch {
        title: Some("updated title".into()),
        content: None,
        tags: None,
        importance: Some(Importance::High),
        confidence: None,
        valid_until: None,
    };
    let updated = ch.update(id.clone(), patch).await.unwrap();
    assert!(updated);

    drop(ch);
    handle.await.unwrap();

    let verify_db = MemoryDb::open(file.path().to_str().unwrap(), None).unwrap();
    let recalled = verify_db.recall(&id).unwrap().expect("should find memory");
    assert_eq!(recalled.memory.title, "updated title");
    assert_eq!(recalled.memory.importance, Importance::High);
}

#[tokio::test]
async fn forget_via_channel() {
    let (db, file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    let id = ch.store(minimal_memory()).await.unwrap();
    let forgot = ch.forget(id.clone(), false).await.unwrap();
    assert!(forgot);

    drop(ch);
    handle.await.unwrap();

    let verify_db = MemoryDb::open(file.path().to_str().unwrap(), None).unwrap();
    let recalled = verify_db
        .recall(&id)
        .unwrap()
        .expect("should find archived memory");
    assert!(recalled.memory.archived);
}

#[tokio::test]
async fn relate_via_channel() {
    let (db, file) = temp_db();
    let (ch, handle) = WriteChannel::new(db);

    let id1 = ch.store(minimal_memory()).await.unwrap();
    let id2 = ch.store(minimal_memory()).await.unwrap();
    ch.relate(id1.clone(), id2.clone(), RelationType::Related)
        .await
        .unwrap();

    drop(ch);
    handle.await.unwrap();

    let verify_db = MemoryDb::open(file.path().to_str().unwrap(), None).unwrap();
    let recalled = verify_db.recall(&id1).unwrap().expect("should find memory");
    assert!(
        recalled
            .relationships
            .iter()
            .any(|r| r.source_id == id1.to_string()
                && r.target_id == id2.to_string()
                && r.relation_type == RelationType::Related),
        "relationship should exist between id1 and id2"
    );
}
