use std::path::Path;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::SqlitePool;

use crate::error::NousError;

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
        sqlx::raw_sql(include_str!("migrations/fts/001_schema_version.sql"))
            .execute(&self.fts)
            .await?;

        sqlx::raw_sql(include_str!("migrations/vec/001_schema_version.sql"))
            .execute(&self.vec)
            .await?;

        Ok(())
    }

    pub async fn close(&self) {
        self.fts.close().await;
        self.vec.close().await;
    }
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

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.fts)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM schema_version")
            .fetch_one(&pools.vec)
            .await
            .unwrap();
        assert_eq!(row.0, 0);

        pools.close().await;
    }
}
