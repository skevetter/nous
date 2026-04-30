use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::error::NousError;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: "001",
        name: "schema_version",
        sql: "CREATE TABLE IF NOT EXISTS schema_version (\
              id INTEGER PRIMARY KEY, \
              version TEXT NOT NULL, \
              applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
              );",
    },
    Migration {
        version: "002",
        name: "rooms",
        sql: "CREATE TABLE IF NOT EXISTS rooms (\
              id TEXT PRIMARY KEY, \
              name TEXT NOT NULL, \
              purpose TEXT, \
              metadata TEXT, \
              archived INTEGER NOT NULL DEFAULT 0, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              ); \
              CREATE UNIQUE INDEX IF NOT EXISTS idx_rooms_name_active ON rooms(name) WHERE archived = 0;",
    },
    Migration {
        version: "003",
        name: "room_messages",
        sql: "CREATE TABLE IF NOT EXISTS room_messages (\
              id TEXT PRIMARY KEY, \
              room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
              sender_id TEXT NOT NULL, \
              content TEXT NOT NULL, \
              reply_to TEXT, \
              metadata TEXT, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))\
              );",
    },
    Migration {
        version: "004",
        name: "room_messages_fts",
        sql: "CREATE VIRTUAL TABLE IF NOT EXISTS room_messages_fts USING fts5(content, content_rowid='rowid', tokenize='porter unicode61'); \
              CREATE TRIGGER IF NOT EXISTS room_messages_fts_insert AFTER INSERT ON room_messages BEGIN INSERT INTO room_messages_fts(rowid, content) VALUES (NEW.rowid, NEW.content); END; \
              CREATE TRIGGER IF NOT EXISTS room_messages_fts_delete AFTER DELETE ON room_messages BEGIN INSERT INTO room_messages_fts(room_messages_fts, rowid, content) VALUES('delete', OLD.rowid, OLD.content); END;",
    },
    Migration {
        version: "005",
        name: "room_subscriptions",
        sql: "CREATE TABLE IF NOT EXISTS room_subscriptions (\
              room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE, \
              agent_id TEXT NOT NULL, \
              topics TEXT, \
              created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
              PRIMARY KEY (room_id, agent_id)\
              );",
    },
];

struct Migration {
    version: &'static str,
    name: &'static str,
    sql: &'static str,
}

/// Holds connection pools for both SQLite databases.
/// Call `close()` before application exit to ensure WAL checkpointing completes.
pub struct DbPools {
    pub fts: SqlitePool,
    pub vec: SqlitePool,
}

impl DbPools {
    pub async fn connect(data_dir: &Path) -> Result<Self, NousError> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| NousError::Internal(format!("failed to create data dir: {e}")))?;

        let fts_path = data_dir.join("memory-fts.db");
        let vec_path = data_dir.join("memory-vec.db");

        let fts = create_pool(&fts_path).await?;
        let vec = create_pool(&vec_path).await?;

        Ok(Self { fts, vec })
    }

    pub async fn run_migrations(&self) -> Result<(), NousError> {
        run_migrations_on_pool(&self.fts).await?;
        run_migrations_on_pool(&self.vec).await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.fts.close().await;
        self.vec.close().await;
    }
}

async fn run_migrations_on_pool(pool: &SqlitePool) -> Result<(), NousError> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_version (\
         id INTEGER PRIMARY KEY, \
         version TEXT NOT NULL, \
         applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
         );",
    )
    .execute(pool)
    .await?;

    for migration in MIGRATIONS {
        let already_applied: bool =
            sqlx::query("SELECT EXISTS(SELECT 1 FROM schema_version WHERE version = ?)")
                .bind(migration.version)
                .fetch_one(pool)
                .await?
                .get(0);

        if !already_applied {
            sqlx::raw_sql(migration.sql).execute(pool).await?;
            sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
                .bind(migration.version)
                .execute(pool)
                .await?;
            tracing::info!(
                version = migration.version,
                name = migration.name,
                "applied migration"
            );
        }
    }

    Ok(())
}

async fn create_pool(path: &Path) -> Result<SqlitePool, NousError> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;

    Ok(pool)
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
    async fn run_migrations_creates_schema_version() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();

        let expected = MIGRATIONS.len() as i64;

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, expected);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.vec)
            .await
            .unwrap();
        assert_eq!(row.0, expected);

        pools.close().await;
    }

    #[tokio::test]
    async fn migrations_are_idempotent() {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        pools.run_migrations().await.unwrap();

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, MIGRATIONS.len() as i64);

        pools.close().await;
    }
}
