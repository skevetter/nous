use std::sync::Arc;

use nous_core::channel::ReadPool;
use nous_core::db::MemoryDb;
use rusqlite::params;
use tempfile::NamedTempFile;

fn temp_db_with_data() -> NamedTempFile {
    let file = NamedTempFile::new().expect("failed to create temp file");
    let _db = MemoryDb::open(file.path().to_str().unwrap(), None, 384).expect("failed to open db");
    file
}

#[tokio::test]
async fn concurrent_reads() {
    let file = temp_db_with_data();
    let pool = Arc::new(ReadPool::new(file.path().to_str().unwrap(), None, 2).unwrap());

    let pool1 = Arc::clone(&pool);
    let h1 = tokio::spawn(async move {
        pool1
            .with_conn(|conn| {
                let count: i64 =
                    conn.query_row("SELECT count(*) FROM categories", [], |r| r.get(0))?;
                Ok(count)
            })
            .await
    });

    let pool2 = Arc::clone(&pool);
    let h2 = tokio::spawn(async move {
        pool2
            .with_conn(|conn| {
                let count: i64 =
                    conn.query_row("SELECT count(*) FROM categories", [], |r| r.get(0))?;
                Ok(count)
            })
            .await
    });

    let c1 = h1.await.unwrap().unwrap();
    let c2 = h2.await.unwrap().unwrap();
    assert!(c1 > 0);
    assert!(c2 > 0);
}

#[tokio::test]
async fn write_rejected() {
    let file = temp_db_with_data();
    let pool = ReadPool::new(file.path().to_str().unwrap(), None, 1).unwrap();

    let result = pool
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO categories (name, source) VALUES (?1, 'user')",
                params!["should_fail"],
            )?;
            Ok(())
        })
        .await;

    assert!(
        result.is_err(),
        "write should be rejected on read-only connection"
    );
}

#[tokio::test]
async fn excess_readers_queue() {
    let file = temp_db_with_data();
    let pool = Arc::new(ReadPool::new(file.path().to_str().unwrap(), None, 2).unwrap());

    let mut handles = Vec::new();
    for _ in 0..5 {
        let pool = Arc::clone(&pool);
        handles.push(tokio::spawn(async move {
            pool.with_conn(|conn| {
                let count: i64 =
                    conn.query_row("SELECT count(*) FROM categories", [], |r| r.get(0))?;
                Ok(count)
            })
            .await
        }));
    }

    for h in handles {
        let count = h.await.unwrap().unwrap();
        assert!(count > 0);
    }
}

#[tokio::test]
async fn query_only_pragma_set() {
    let file = temp_db_with_data();
    let pool = ReadPool::new(file.path().to_str().unwrap(), None, 2).unwrap();

    let value = pool
        .with_conn(|conn| {
            let val: i64 = conn.query_row("PRAGMA query_only", [], |r| r.get(0))?;
            Ok(val)
        })
        .await
        .unwrap();

    assert_eq!(value, 1, "expected query_only to be 1 (ON), got: {value}");
}
