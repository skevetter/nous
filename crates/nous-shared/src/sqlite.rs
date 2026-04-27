use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::{NousError, Result};

const WAL_PRAGMAS: &[&str] = &[
    "PRAGMA journal_mode = WAL",
    "PRAGMA wal_autocheckpoint = 1000",
    "PRAGMA synchronous = NORMAL",
    "PRAGMA busy_timeout = 5000",
    "PRAGMA cache_size = -64000",
    "PRAGMA foreign_keys = ON",
];

const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";

fn is_plaintext_sqlite(path: &str) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut header = [0u8; 16];
    matches!(file.read_exact(&mut header), Ok(()) if header == *SQLITE_HEADER)
}

fn migrate_plaintext_to_encrypted(path: &str, key: &str) -> Result<()> {
    let backup_path = format!("{path}.pre-encryption.bak");
    std::fs::copy(path, &backup_path)?;
    eprintln!(
        "warning: plaintext database detected at {path}, migrating to encrypted format (backup at {backup_path})"
    );

    let conn = Connection::open(path)?;
    for pragma in WAL_PRAGMAS {
        conn.execute_batch(pragma)?;
    }

    conn.execute_batch("PRAGMA writable_schema = ON")?;
    conn.execute_batch(
        "DELETE FROM sqlite_master WHERE type='table' AND name LIKE 'memory_embeddings%'",
    )?;
    conn.execute_batch("PRAGMA writable_schema = OFF")?;
    conn.execute_batch("VACUUM")?;

    let encrypted_path = format!("{path}.encrypted");
    conn.execute_batch(&format!(
        "ATTACH DATABASE '{}' AS encrypted KEY '{}'",
        encrypted_path,
        key.replace('\'', "''")
    ))?;
    conn.execute_batch("SELECT sqlcipher_export('encrypted')")?;
    conn.execute_batch("DETACH DATABASE encrypted")?;
    drop(conn);

    std::fs::rename(&encrypted_path, path)?;
    eprintln!("info: database successfully migrated to encrypted format");
    Ok(())
}

pub fn open_connection(path: &str, key: Option<&str>) -> Result<Connection> {
    if path != ":memory:"
        && let Some(k) = key
        && Path::new(path).exists()
        && is_plaintext_sqlite(path)
    {
        migrate_plaintext_to_encrypted(path, k)?;
    }

    let conn = if path == ":memory:" {
        Connection::open_in_memory()?
    } else {
        Connection::open(path)?
    };

    if let Some(k) = key {
        conn.pragma_update(None, "key", k)?;
    }

    for pragma in WAL_PRAGMAS {
        if let Err(e) = conn.execute_batch(pragma) {
            if is_not_a_database(&e) {
                return Err(NousError::Encryption(e.to_string()));
            }
            return Err(e.into());
        }
    }

    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(|e| {
        if is_not_a_database(&e) {
            return NousError::Encryption(e.to_string());
        }
        NousError::Sqlite(e)
    })?;

    Ok(conn)
}

fn is_not_a_database(e: &rusqlite::Error) -> bool {
    if let rusqlite::Error::SqliteFailure(ffi_err, _) = e {
        return ffi_err.code == rusqlite::ErrorCode::NotADatabase;
    }
    false
}

pub fn run_migrations(conn: &Connection, migrations: &[&str]) -> Result<()> {
    conn.execute_batch("BEGIN")?;
    for migration in migrations {
        if let Err(e) = conn.execute_batch(migration) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e.into());
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(())
}

pub async fn spawn_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| NousError::Internal(format!("task panicked: {e}")))?
}

fn default_key_path() -> PathBuf {
    crate::xdg::config_dir()
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("db.key")
}

pub fn resolve_key() -> Result<String> {
    if let Ok(val) = std::env::var("NOUS_DB_KEY")
        && !val.is_empty()
    {
        return Ok(val);
    }
    resolve_key_with_path(&default_key_path())
}

pub fn resolve_key_with_path(key_path: &Path) -> Result<String> {
    if let Ok(val) = std::env::var("NOUS_DB_KEY")
        && !val.is_empty()
    {
        return Ok(val);
    }

    if key_path.exists() {
        let contents = std::fs::read_to_string(key_path)?;
        let trimmed = contents.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let key = generate_hex_key();
    std::fs::write(key_path, &key)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(key)
}

fn generate_hex_key() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

pub fn rotate_key(db_path: &Path, current_key: &str, new_key: &str) -> Result<()> {
    let db_str = db_path
        .to_str()
        .ok_or_else(|| NousError::Config("non-UTF-8 database path".into()))?;

    let backup_path = db_path.with_extension(format!(
        "{}.bak",
        db_path
            .extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
    ));

    // Verify current key works before making any changes
    {
        let conn = open_connection(db_str, Some(current_key))?;
        drop(conn);
    }

    std::fs::copy(db_path, &backup_path)?;

    let rekey_result = (|| -> Result<()> {
        let conn = open_connection(db_str, Some(current_key))?;
        conn.pragma_update(None, "rekey", new_key)?;
        drop(conn);

        let conn = open_connection(db_str, Some(new_key))?;
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| NousError::Encryption(e.to_string()))?;
        if integrity != "ok" {
            return Err(NousError::Encryption(format!(
                "integrity check failed after rekey: {integrity}"
            )));
        }
        Ok(())
    })();

    if let Err(e) = rekey_result {
        if let Err(restore_err) = std::fs::copy(&backup_path, db_path) {
            return Err(NousError::Internal(format!(
                "rekey failed: {e}; backup restore also failed: {restore_err}"
            )));
        }
        return Err(e);
    }

    let key_path = default_key_path();
    if key_path.exists()
        && let Ok(contents) = std::fs::read_to_string(&key_path)
        && contents.trim() == current_key
    {
        std::fs::write(&key_path, new_key)?;
    }

    Ok(())
}
