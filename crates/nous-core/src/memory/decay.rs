use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::error::NousError;

pub async fn run_importance_decay(
    db: &DatabaseConnection,
    high_to_moderate_days: u32,
    moderate_to_low_days: u32,
) -> Result<u64, NousError> {
    let now = chrono::Utc::now();
    let high_cutoff = (now - chrono::Duration::days(high_to_moderate_days as i64))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    let moderate_cutoff = (now - chrono::Duration::days(moderate_to_low_days as i64))
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();

    // Run moderate->low first so that high->moderate in the same sweep
    // doesn't immediately cascade to low in one call.
    let r1 = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE memories SET importance = 'low' \
             WHERE importance = 'moderate' AND archived = 0 \
             AND id NOT IN (\
                 SELECT memory_id FROM memory_access_log WHERE accessed_at > ?\
             )",
            [moderate_cutoff.into()],
        ))
        .await?;

    let r2 = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE memories SET importance = 'moderate' \
             WHERE importance = 'high' AND archived = 0 \
             AND id NOT IN (\
                 SELECT memory_id FROM memory_access_log WHERE accessed_at > ?\
             )",
            [high_cutoff.into()],
        ))
        .await?;

    Ok(r1.rows_affected() + r2.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use crate::memory::store::{get_memory_by_id, log_access, save_memory};
    use crate::memory::types::*;
    use tempfile::TempDir;

    async fn setup() -> (DatabaseConnection, crate::db::VecPool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        for agent_id in ["agent-1", "agent-2", "agent-3", "test-agent"] {
            pools.fts.execute_unprepared(
                &format!("INSERT OR IGNORE INTO agents (id, name, namespace, status) VALUES ('{agent_id}', '{agent_id}', 'default', 'active')")
            ).await.unwrap();
        }
        (pools.fts, pools.vec, tmp)
    }

    #[tokio::test]
    async fn importance_decay() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "High importance".into(),
                content: "Should decay".into(),
                memory_type: MemoryType::Fact,
                importance: Some(Importance::High),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(mem.importance, "high");

        let affected = run_importance_decay(&db, 0, 0).await.unwrap();
        assert!(affected > 0);

        let refreshed = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(refreshed.importance, "moderate");

        let affected2 = run_importance_decay(&db, 0, 0).await.unwrap();
        assert!(affected2 > 0);

        let refreshed2 = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(refreshed2.importance, "low");
    }

    #[tokio::test]
    async fn access_log_prevents_decay() {
        let (db, _vec_pool, _tmp) = setup().await;

        let mem = save_memory(
            &db,
            SaveMemoryRequest {
                workspace_id: None,
                agent_id: None,
                title: "Accessed memory".into(),
                content: "Should not decay".into(),
                memory_type: MemoryType::Fact,
                importance: Some(Importance::High),
                topic_key: None,
                valid_from: None,
                valid_until: None,
            },
        )
        .await
        .unwrap();

        log_access(&db, &mem.id, "recall", None).await.unwrap();

        run_importance_decay(&db, 30, 60).await.unwrap();

        let refreshed = get_memory_by_id(&db, &mem.id).await.unwrap();
        assert_eq!(refreshed.importance, "high");
    }
}
