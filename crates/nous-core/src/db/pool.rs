use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::Connection;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};

use crate::error::NousError;

pub type VecPool = Arc<Mutex<Connection>>;

pub struct DbPools {
    pub fts: DatabaseConnection,
    pub vec: VecPool,
}

impl DbPools {
    pub async fn connect(data_dir: &Path) -> Result<Self, NousError> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| NousError::Internal(format!("failed to create data dir: {e}")))?;

        let fts_path = data_dir.join("memory-fts.db");
        let vec_path = data_dir.join("memory-vec.db");

        let fts = create_fts_connection(&fts_path).await?;
        let vec = create_vec_pool(&vec_path)?;

        Ok(Self { fts, vec })
    }

    pub async fn run_migrations(&self) -> Result<(), NousError> {
        use sea_orm_migration::MigratorTrait;
        nous_migration::Migrator::up(&self.fts, None).await?;
        run_vec_migrations(&self.vec)?;
        Ok(())
    }

    pub async fn close(self) {
        let _ = self.fts.close().await;
    }

    pub fn fts(&self) -> &DatabaseConnection {
        &self.fts
    }
}

async fn create_fts_connection(path: &Path) -> Result<DatabaseConnection, NousError> {
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let mut opts = ConnectOptions::new(url);
    opts.sqlx_logging(false)
        .connect_timeout(Duration::from_secs(5))
        .max_connections(5);

    let db = Database::connect(opts).await?;

    let pool = db.get_sqlite_connection_pool();
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;")
        .execute(pool)
        .await
        .map_err(|e| NousError::Internal(format!("failed to set pragmas: {e}")))?;

    Ok(db)
}

pub fn create_vec_pool(path: &Path) -> Result<VecPool, NousError> {
    let conn = Connection::open(path)
        .map_err(|e| NousError::Internal(format!("failed to open vec db: {e}")))?;

    // SAFETY: sqlite-vec requires raw FFI init before the connection is used.
    unsafe {
        let db = conn.handle();
        let rc = sqlite3_vec_init(db, std::ptr::null_mut(), std::ptr::null());
        if rc != rusqlite::ffi::SQLITE_OK {
            return Err(NousError::Internal(format!(
                "failed to load sqlite-vec: rc={rc}"
            )));
        }
    }

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .map_err(|e| NousError::Internal(format!("failed to set vec db pragmas: {e}")))?;

    Ok(Arc::new(Mutex::new(conn)))
}

#[link(name = "sqlite_vec0")]
extern "C" {
    fn sqlite3_vec_init(
        db: *mut rusqlite::ffi::sqlite3,
        pz_err_msg: *mut *mut std::ffi::c_char,
        p_api: *const rusqlite::ffi::sqlite3_api_routines,
    ) -> std::ffi::c_int;
}

struct Migration {
    version: &'static str,
    name: &'static str,
    sql: &'static str,
}

const VEC_MIGRATIONS: &[Migration] = &[
    Migration {
        version: "vec_001",
        name: "memory_embeddings_vec0",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(\
              memory_id TEXT PRIMARY KEY, \
              embedding float[1024]\
              );",
    },
    Migration {
        version: "vec_002",
        name: "memory_chunks",
        sql: "CREATE TABLE IF NOT EXISTS memory_chunks (\
              id TEXT PRIMARY KEY, \
              memory_id TEXT NOT NULL, \
              content TEXT NOT NULL, \
              chunk_index INTEGER NOT NULL, \
              start_offset INTEGER NOT NULL, \
              end_offset INTEGER NOT NULL, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE INDEX IF NOT EXISTS idx_chunks_memory_id ON memory_chunks(memory_id);",
    },
];

/// Returns the dimension of the `embedding` column in the `memory_embeddings` vec0 table,
/// or `None` if the table does not yet exist (fresh DB, pre-migration).
pub fn read_vec_dimension(vec_pool: &VecPool) -> Result<Option<usize>, NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    let table_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='memory_embeddings')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| NousError::Internal(format!("failed to check memory_embeddings table: {e}")))?;

    if !table_exists {
        return Ok(None);
    }

    // vec0 exposes column metadata via vec_column_distance_metric / vec_each; the simplest
    // approach is to parse the CREATE TABLE SQL from sqlite_master.
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='memory_embeddings'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| NousError::Internal(format!("failed to read memory_embeddings DDL: {e}")))?;

    // Extract the dimension from patterns like `float[1024]` or `float[ 1024 ]`.
    let dim = sql
        .split("float[")
        .nth(1)
        .and_then(|s| s.split(']').next())
        .and_then(|s| s.trim().parse::<usize>().ok());

    Ok(dim)
}

fn run_vec_migrations(vec_pool: &VecPool) -> Result<(), NousError> {
    let conn = vec_pool
        .lock()
        .map_err(|e| NousError::Internal(format!("vec pool lock poisoned: {e}")))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS vec_schema_version (\
         id INTEGER PRIMARY KEY, \
         version TEXT NOT NULL, \
         applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
         );",
    )
    .map_err(|e| NousError::Internal(format!("failed to create vec schema_version: {e}")))?;

    for migration in VEC_MIGRATIONS {
        let already_applied: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM vec_schema_version WHERE version = ?1)",
                rusqlite::params![migration.version],
                |row| row.get(0),
            )
            .map_err(|e| NousError::Internal(format!("failed to check vec migration: {e}")))?;

        if !already_applied {
            conn.execute_batch(migration.sql).map_err(|e| {
                NousError::Internal(format!(
                    "failed to run vec migration {}: {e}",
                    migration.version
                ))
            })?;
            conn.execute(
                "INSERT INTO vec_schema_version (version) VALUES (?1)",
                rusqlite::params![migration.version],
            )
            .map_err(|e| NousError::Internal(format!("failed to record vec migration: {e}")))?;
            tracing::info!(
                version = migration.version,
                name = migration.name,
                "applied vec migration"
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn connect_creates_db_files() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();

        assert!(tmp.path().join("memory-fts.db").exists());
        assert!(tmp.path().join("memory-vec.db").exists());

        pools.close().await;
    }

    #[tokio::test]
    async fn run_migrations_creates_tables() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();

        let pool = pools.fts.get_sqlite_connection_pool();
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM seaql_migrations")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(
            row.0 >= 33,
            "expected at least 33 migrations, got {}",
            row.0
        );

        let vec_count: i64 = pools
            .vec
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM vec_schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(vec_count, VEC_MIGRATIONS.len() as i64);

        pools.close().await;
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        pools.run_migrations().await.unwrap();

        let pool = pools.fts.get_sqlite_connection_pool();
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM seaql_migrations")
            .fetch_one(pool)
            .await
            .unwrap();
        assert!(row.0 >= 33);

        pools.close().await;
    }
}
