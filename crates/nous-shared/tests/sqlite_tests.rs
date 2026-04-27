use std::sync::Mutex;

use nous_shared::sqlite;
use nous_shared::{NousError, Result};

static ENV_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn open_in_memory_sets_pragmas() {
    let conn = sqlite::open_connection(":memory:", None).unwrap();

    let journal: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(journal, "memory");

    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fk, 1);

    let sync: i64 = conn
        .query_row("PRAGMA synchronous", [], |r| r.get(0))
        .unwrap();
    assert_eq!(sync, 1); // NORMAL = 1

    let busy: i64 = conn
        .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
        .unwrap();
    assert_eq!(busy, 5000);

    let cache: i64 = conn
        .query_row("PRAGMA cache_size", [], |r| r.get(0))
        .unwrap();
    assert_eq!(cache, -64000);
}

#[test]
fn run_migrations_creates_table() {
    let conn = sqlite::open_connection(":memory:", None).unwrap();
    sqlite::run_migrations(
        &conn,
        &["CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL);"],
    )
    .unwrap();

    let count: i64 = conn
        .query_row("SELECT count(*) FROM items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn run_migrations_rolls_back_on_failure() {
    let conn = sqlite::open_connection(":memory:", None).unwrap();
    let result = sqlite::run_migrations(
        &conn,
        &[
            "CREATE TABLE good_table (id INTEGER PRIMARY KEY);",
            "THIS IS NOT VALID SQL;",
        ],
    );
    assert!(result.is_err());

    let exists: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='good_table'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 0, "first migration should have been rolled back");
}

#[tokio::test]
async fn spawn_blocking_returns_value() {
    let val: i32 = sqlite::spawn_blocking(|| Ok(42)).await.unwrap();
    assert_eq!(val, 42);
}

#[tokio::test]
async fn spawn_blocking_propagates_panic() {
    let result: Result<()> = sqlite::spawn_blocking(|| panic!("boom")).await;
    assert!(matches!(
        result.unwrap_err(),
        nous_shared::NousError::Internal(_)
    ));
}

#[test]
fn resolve_key_from_env_var() {
    let _lock = ENV_MUTEX.lock().unwrap();
    unsafe {
        std::env::set_var("NOUS_DB_KEY", "env-secret-key");
    }
    let key = sqlite::resolve_key().unwrap();
    assert_eq!(key, "env-secret-key");
    unsafe {
        std::env::remove_var("NOUS_DB_KEY");
    }
}

#[test]
fn resolve_key_from_file() {
    let _lock = ENV_MUTEX.lock().unwrap();
    unsafe {
        std::env::remove_var("NOUS_DB_KEY");
    }

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("db.key");
    std::fs::write(&key_path, "  file-secret-key  \n").unwrap();

    let key = sqlite::resolve_key_with_path(&key_path).unwrap();
    assert_eq!(key, "file-secret-key");
}

#[test]
fn resolve_key_auto_generates() {
    let _lock = ENV_MUTEX.lock().unwrap();
    unsafe {
        std::env::remove_var("NOUS_DB_KEY");
    }

    let tmp = tempfile::tempdir().unwrap();
    let key_path = tmp.path().join("nous").join("db.key");

    let key = sqlite::resolve_key_with_path(&key_path).unwrap();
    assert_eq!(key.len(), 64); // 32 bytes hex = 64 chars

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::metadata(&key_path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    let key2 = sqlite::resolve_key_with_path(&key_path).unwrap();
    assert_eq!(key2, key);
}

#[test]
fn open_connection_with_encryption() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("encrypted.db");
    let db_str = db_path.to_str().unwrap();
    let key = "test-encryption-key";

    {
        let conn = sqlite::open_connection(db_str, Some(key)).unwrap();
        conn.execute(
            "CREATE TABLE secrets (id INTEGER PRIMARY KEY, val TEXT)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO secrets (val) VALUES ('hidden')", [])
            .unwrap();
    }

    {
        let conn = sqlite::open_connection(db_str, Some(key)).unwrap();
        let val: String = conn
            .query_row("SELECT val FROM secrets WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, "hidden");
    }
}

#[test]
fn open_connection_migrates_plaintext_to_encrypted() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("plaintext.db");
    let db_str = db_path.to_str().unwrap();
    let key = "migration-test-key";

    {
        let conn = sqlite::open_connection(db_str, None).unwrap();
        conn.execute("CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        conn.execute("INSERT INTO data (val) VALUES ('preserved')", [])
            .unwrap();
    }

    let header = std::fs::read(db_path.as_path()).unwrap();
    assert_eq!(&header[..16], b"SQLite format 3\0");

    {
        let conn = sqlite::open_connection(db_str, Some(key)).unwrap();
        let val: String = conn
            .query_row("SELECT val FROM data WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(val, "preserved");
    }

    let header = std::fs::read(db_path.as_path()).unwrap();
    assert_ne!(
        &header[..16],
        b"SQLite format 3\0",
        "file should be encrypted after migration"
    );

    let backup = tmp.path().join("plaintext.db.pre-encryption.bak");
    assert!(backup.exists(), "backup should exist after migration");

    let result = sqlite::open_connection(db_str, Some("wrong-key"));
    assert!(result.is_err(), "wrong key should fail on migrated DB");
}

#[test]
fn open_connection_wrong_key_returns_encryption_error() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("encrypted2.db");
    let db_str = db_path.to_str().unwrap();

    {
        let conn = sqlite::open_connection(db_str, Some("correct-key")).unwrap();
        conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])
            .unwrap();
    }

    let result = sqlite::open_connection(db_str, Some("wrong-key"));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, NousError::Encryption(_)),
        "expected Encryption error, got: {err:?}"
    );
}
