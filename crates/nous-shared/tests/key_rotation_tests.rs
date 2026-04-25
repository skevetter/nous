use std::path::Path;

use nous_shared::NousError;
use nous_shared::sqlite;

fn create_encrypted_db(path: &Path, key: &str) {
    let conn = sqlite::open_connection(path.to_str().unwrap(), Some(key)).unwrap();
    conn.execute(
        "CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT NOT NULL)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO data (val) VALUES ('secret')", [])
        .unwrap();
}

#[test]
fn rotate_key_makes_data_readable_with_new_key() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("rotate.db");
    create_encrypted_db(&db_path, "old");

    sqlite::rotate_key(&db_path, "old", "new").unwrap();

    let conn = sqlite::open_connection(db_path.to_str().unwrap(), Some("new")).unwrap();
    let val: String = conn
        .query_row("SELECT val FROM data WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(val, "secret");
}

#[test]
fn rotate_key_rejects_old_key() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("rotate2.db");
    create_encrypted_db(&db_path, "old");

    sqlite::rotate_key(&db_path, "old", "new").unwrap();

    let result = sqlite::open_connection(db_path.to_str().unwrap(), Some("old"));
    assert!(
        matches!(result.unwrap_err(), NousError::Encryption(_)),
        "old key should be rejected after rotation"
    );
}

#[test]
fn rotate_key_creates_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("rotate3.db");
    create_encrypted_db(&db_path, "old");

    sqlite::rotate_key(&db_path, "old", "new").unwrap();

    let backup_path = db_path.with_extension("db.bak");
    assert!(
        backup_path.exists(),
        "backup file should exist after rotation"
    );
}

#[test]
fn rotate_key_wrong_current_key_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("rotate4.db");
    create_encrypted_db(&db_path, "correct");

    let result = sqlite::rotate_key(&db_path, "wrong", "new");
    assert!(result.is_err(), "wrong current key should fail");

    let conn = sqlite::open_connection(db_path.to_str().unwrap(), Some("correct")).unwrap();
    let val: String = conn
        .query_row("SELECT val FROM data WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        val, "secret",
        "database should be unmodified after failed rotation"
    );
}

#[test]
fn rotate_key_passes_integrity_check() {
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("rotate5.db");
    create_encrypted_db(&db_path, "old");

    sqlite::rotate_key(&db_path, "old", "new").unwrap();

    let conn = sqlite::open_connection(db_path.to_str().unwrap(), Some("new")).unwrap();
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}
