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

pub fn open_connection(path: &str, key: Option<&str>) -> Result<Connection> {
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
        .map_err(|e| NousError::Config(format!("task panicked: {e}")))?
}

fn default_key_path() -> PathBuf {
    dirs_next_or_fallback().join("nous").join("db.key")
}

fn dirs_next_or_fallback() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

pub fn resolve_key() -> Result<Option<String>> {
    if let Ok(val) = std::env::var("NOUS_DB_KEY") {
        if !val.is_empty() {
            return Ok(Some(val));
        }
    }
    resolve_key_with_path(&default_key_path())
}

pub fn resolve_key_with_path(key_path: &Path) -> Result<Option<String>> {
    if let Ok(val) = std::env::var("NOUS_DB_KEY") {
        if !val.is_empty() {
            return Ok(Some(val));
        }
    }

    if key_path.exists() {
        let contents = std::fs::read_to_string(key_path)?;
        let trimmed = contents.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed));
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

    Ok(Some(key))
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
