use sea_orm::entity::prelude::*;
use sea_orm::{ActiveValue, ConnectionTrait, DatabaseConnection, NotSet, Set, Statement};
use serde::Serialize;

use crate::entities::search_events as se_entity;
use crate::error::NousError;

#[derive(Debug, Clone, Serialize)]
pub struct SearchEvent {
    pub query_text: String,
    pub search_type: String,
    pub result_count: i64,
    pub latency_ms: i64,
    pub workspace_id: Option<String>,
    pub agent_id: Option<String>,
}

pub async fn record_search_event(
    db: &DatabaseConnection,
    event: &SearchEvent,
) -> Result<(), NousError> {
    let model = se_entity::ActiveModel {
        id: ActiveValue::default(),
        query_text: Set(event.query_text.clone()),
        search_type: Set(event.search_type.clone()),
        result_count: Set(event.result_count as i32),
        latency_ms: Set(event.latency_ms as i32),
        workspace_id: Set(event.workspace_id.clone()),
        agent_id: Set(event.agent_id.clone()),
        created_at: NotSet,
    };

    se_entity::Entity::insert(model).exec(db).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchStats {
    pub total_searches: i64,
    pub fts_count: i64,
    pub vector_count: i64,
    pub hybrid_count: i64,
    pub zero_result_rate: f64,
    pub avg_latency_ms: f64,
    pub top_queries: Vec<TopQuery>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopQuery {
    pub query_text: String,
    pub count: i64,
}

fn maybe_since_values(since: Option<&str>) -> Vec<sea_orm::Value> {
    since.map(|v| vec![v.into()]).unwrap_or_default()
}

async fn query_total_count(
    db: &DatabaseConnection,
    since_clause: &str,
    since: Option<&str>,
) -> Result<i64, NousError> {
    let sql = format!("SELECT COUNT(*) as cnt FROM search_events{since_clause}");
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        maybe_since_values(since),
    );
    let row = db.query_one(stmt).await?;
    Ok(row
        .map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
        .unwrap_or(0))
}

async fn query_type_counts(
    db: &DatabaseConnection,
    since_clause: &str,
    since: Option<&str>,
) -> Result<(i64, i64, i64), NousError> {
    let sql = format!(
        "SELECT \
         COALESCE(SUM(CASE WHEN search_type = 'fts' THEN 1 ELSE 0 END), 0) as fts_cnt, \
         COALESCE(SUM(CASE WHEN search_type = 'vector' THEN 1 ELSE 0 END), 0) as vec_cnt, \
         COALESCE(SUM(CASE WHEN search_type = 'hybrid' THEN 1 ELSE 0 END), 0) as hyb_cnt \
         FROM search_events{since_clause}"
    );
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        maybe_since_values(since),
    );
    let row = db.query_one(stmt).await?;
    Ok(match row {
        Some(r) => (
            r.try_get_by::<i64, _>("fts_cnt").unwrap_or(0),
            r.try_get_by::<i64, _>("vec_cnt").unwrap_or(0),
            r.try_get_by::<i64, _>("hyb_cnt").unwrap_or(0),
        ),
        None => (0, 0, 0),
    })
}

async fn query_zero_result_count(
    db: &DatabaseConnection,
    since: Option<&str>,
) -> Result<i64, NousError> {
    let since_filter = if since.is_some() {
        " AND created_at >= ?"
    } else {
        ""
    };
    let sql = format!(
        "SELECT COUNT(*) as cnt FROM search_events WHERE result_count = 0{since_filter}"
    );
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        maybe_since_values(since),
    );
    let row = db.query_one(stmt).await?;
    Ok(row
        .map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
        .unwrap_or(0))
}

async fn query_avg_latency(
    db: &DatabaseConnection,
    since_clause: &str,
    since: Option<&str>,
) -> Result<f64, NousError> {
    let sql = format!(
        "SELECT CAST(COALESCE(AVG(latency_ms * 1.0), 0.0) AS REAL) as avg_lat FROM search_events{since_clause}"
    );
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        maybe_since_values(since),
    );
    let row = db.query_one(stmt).await?;
    Ok(row
        .map(|r| r.try_get_by::<f64, _>("avg_lat").unwrap_or(0.0))
        .unwrap_or(0.0))
}

async fn query_top_queries(
    db: &DatabaseConnection,
    since_clause: &str,
    since: Option<&str>,
) -> Result<Vec<TopQuery>, NousError> {
    let sql = format!(
        "SELECT query_text, COUNT(*) as cnt FROM search_events{since_clause} \
         GROUP BY query_text ORDER BY cnt DESC LIMIT 10"
    );
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        &sql,
        maybe_since_values(since),
    );
    let rows = db.query_all(stmt).await?;
    Ok(rows
        .iter()
        .filter_map(|r| {
            let query_text: String = r.try_get_by("query_text").ok()?;
            let count: i64 = r.try_get_by("cnt").ok()?;
            Some(TopQuery { query_text, count })
        })
        .collect())
}

pub async fn get_search_stats(
    db: &DatabaseConnection,
    since: Option<&str>,
) -> Result<SearchStats, NousError> {
    let since_clause = if since.is_some() {
        " WHERE created_at >= ?"
    } else {
        ""
    };

    let total_searches = query_total_count(db, since_clause, since).await?;
    let (fts_count, vector_count, hybrid_count) =
        query_type_counts(db, since_clause, since).await?;
    let zero_count = query_zero_result_count(db, since).await?;
    let zero_result_rate = if total_searches > 0 {
        let z = f64::from(i32::try_from(zero_count).unwrap_or(i32::MAX));
        let t = f64::from(i32::try_from(total_searches).unwrap_or(i32::MAX));
        z / t * 100.0
    } else {
        0.0
    };
    let avg_latency_ms = query_avg_latency(db, since_clause, since).await?;
    let top_queries = query_top_queries(db, since_clause, since).await?;

    Ok(SearchStats {
        total_searches,
        fts_count,
        vector_count,
        hybrid_count,
        zero_result_rate,
        avg_latency_ms,
        top_queries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use tempfile::TempDir;

    async fn setup() -> (DatabaseConnection, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        pools
            .fts
            .execute_unprepared(
                "INSERT OR IGNORE INTO agents (id, name, namespace, status) \
                 VALUES ('agent-1', 'agent-1', 'default', 'active')",
            )
            .await
            .unwrap();
        (pools.fts, tmp)
    }

    fn fts_event(query: &str, results: i64, latency: i64) -> SearchEvent {
        SearchEvent {
            query_text: query.to_string(),
            search_type: "fts".to_string(),
            result_count: results,
            latency_ms: latency,
            workspace_id: None,
            agent_id: None,
        }
    }

    #[tokio::test]
    async fn record_search_event_inserts_row() {
        let (db, _tmp) = setup().await;
        record_search_event(&db, &fts_event("hello", 5, 12)).await.unwrap();

        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.total_searches, 1);
    }

    #[tokio::test]
    async fn record_multiple_events() {
        let (db, _tmp) = setup().await;
        for i in 0..5 {
            record_search_event(&db, &fts_event(&format!("q{i}"), i, 10))
                .await
                .unwrap();
        }
        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.total_searches, 5);
    }

    #[tokio::test]
    async fn record_event_with_workspace_and_agent() {
        let (db, _tmp) = setup().await;
        let event = SearchEvent {
            query_text: "test".to_string(),
            search_type: "vector".to_string(),
            result_count: 3,
            latency_ms: 50,
            workspace_id: Some("ws-1".to_string()),
            agent_id: Some("agent-1".to_string()),
        };
        record_search_event(&db, &event).await.unwrap();
        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.vector_count, 1);
    }

    #[tokio::test]
    async fn get_stats_empty_db() {
        let (db, _tmp) = setup().await;
        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.total_searches, 0);
        assert_eq!(stats.fts_count, 0);
        assert_eq!(stats.vector_count, 0);
        assert_eq!(stats.hybrid_count, 0);
        assert_eq!(stats.zero_result_rate, 0.0);
        assert_eq!(stats.avg_latency_ms, 0.0);
        assert!(stats.top_queries.is_empty());
    }

    #[tokio::test]
    async fn get_stats_counts_by_type() {
        let (db, _tmp) = setup().await;
        record_search_event(&db, &fts_event("a", 1, 10)).await.unwrap();
        record_search_event(&db, &fts_event("b", 2, 20)).await.unwrap();
        record_search_event(
            &db,
            &SearchEvent {
                query_text: "c".to_string(),
                search_type: "vector".to_string(),
                result_count: 3,
                latency_ms: 30,
                workspace_id: None,
                agent_id: None,
            },
        )
        .await
        .unwrap();
        record_search_event(
            &db,
            &SearchEvent {
                query_text: "d".to_string(),
                search_type: "hybrid".to_string(),
                result_count: 4,
                latency_ms: 40,
                workspace_id: None,
                agent_id: None,
            },
        )
        .await
        .unwrap();

        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.total_searches, 4);
        assert_eq!(stats.fts_count, 2);
        assert_eq!(stats.vector_count, 1);
        assert_eq!(stats.hybrid_count, 1);
    }

    #[tokio::test]
    async fn zero_result_rate_all_zero() {
        let (db, _tmp) = setup().await;
        record_search_event(&db, &fts_event("a", 0, 10)).await.unwrap();
        record_search_event(&db, &fts_event("b", 0, 20)).await.unwrap();

        let stats = get_search_stats(&db, None).await.unwrap();
        assert!((stats.zero_result_rate - 100.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn zero_result_rate_partial() {
        let (db, _tmp) = setup().await;
        record_search_event(&db, &fts_event("a", 0, 10)).await.unwrap();
        record_search_event(&db, &fts_event("b", 5, 10)).await.unwrap();
        record_search_event(&db, &fts_event("c", 3, 10)).await.unwrap();
        record_search_event(&db, &fts_event("d", 0, 10)).await.unwrap();

        let stats = get_search_stats(&db, None).await.unwrap();
        assert!((stats.zero_result_rate - 50.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn avg_latency_calculation() {
        let (db, _tmp) = setup().await;
        record_search_event(&db, &fts_event("a", 1, 10)).await.unwrap();
        record_search_event(&db, &fts_event("b", 1, 20)).await.unwrap();
        record_search_event(&db, &fts_event("c", 1, 30)).await.unwrap();

        let stats = get_search_stats(&db, None).await.unwrap();
        assert!((stats.avg_latency_ms - 20.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn top_queries_ordered_by_frequency() {
        let (db, _tmp) = setup().await;
        for _ in 0..5 {
            record_search_event(&db, &fts_event("popular", 1, 10)).await.unwrap();
        }
        for _ in 0..3 {
            record_search_event(&db, &fts_event("medium", 1, 10)).await.unwrap();
        }
        record_search_event(&db, &fts_event("rare", 1, 10)).await.unwrap();

        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.top_queries.len(), 3);
        assert_eq!(stats.top_queries[0].query_text, "popular");
        assert_eq!(stats.top_queries[0].count, 5);
        assert_eq!(stats.top_queries[1].query_text, "medium");
        assert_eq!(stats.top_queries[1].count, 3);
        assert_eq!(stats.top_queries[2].query_text, "rare");
        assert_eq!(stats.top_queries[2].count, 1);
    }

    #[tokio::test]
    async fn top_queries_limited_to_10() {
        let (db, _tmp) = setup().await;
        for i in 0..15 {
            record_search_event(&db, &fts_event(&format!("query-{i}"), 1, 10))
                .await
                .unwrap();
        }

        let stats = get_search_stats(&db, None).await.unwrap();
        assert_eq!(stats.top_queries.len(), 10);
    }

    #[tokio::test]
    async fn temporal_filter_since_excludes_old_events() {
        let (db, _tmp) = setup().await;
        // Insert an event with an old timestamp
        db.execute_unprepared(
            "INSERT INTO search_events (query_text, search_type, result_count, latency_ms, created_at) \
             VALUES ('old-query', 'fts', 1, 10, '2020-01-01T00:00:00')",
        )
        .await
        .unwrap();
        // Insert a recent event
        record_search_event(&db, &fts_event("new-query", 2, 20)).await.unwrap();

        let stats = get_search_stats(&db, Some("2025-01-01T00:00:00")).await.unwrap();
        assert_eq!(stats.total_searches, 1);
        assert_eq!(stats.top_queries[0].query_text, "new-query");
    }

    #[tokio::test]
    async fn temporal_filter_since_includes_all_when_old() {
        let (db, _tmp) = setup().await;
        record_search_event(&db, &fts_event("a", 1, 10)).await.unwrap();
        record_search_event(&db, &fts_event("b", 2, 20)).await.unwrap();

        let stats = get_search_stats(&db, Some("2000-01-01T00:00:00")).await.unwrap();
        assert_eq!(stats.total_searches, 2);
    }

    #[tokio::test]
    async fn temporal_filter_affects_type_counts() {
        let (db, _tmp) = setup().await;
        db.execute_unprepared(
            "INSERT INTO search_events (query_text, search_type, result_count, latency_ms, created_at) \
             VALUES ('old', 'vector', 5, 50, '2020-01-01T00:00:00')",
        )
        .await
        .unwrap();
        record_search_event(
            &db,
            &SearchEvent {
                query_text: "new".to_string(),
                search_type: "hybrid".to_string(),
                result_count: 3,
                latency_ms: 30,
                workspace_id: None,
                agent_id: None,
            },
        )
        .await
        .unwrap();

        let stats = get_search_stats(&db, Some("2025-01-01T00:00:00")).await.unwrap();
        assert_eq!(stats.vector_count, 0);
        assert_eq!(stats.hybrid_count, 1);
    }

    #[tokio::test]
    async fn temporal_filter_affects_zero_result_rate() {
        let (db, _tmp) = setup().await;
        db.execute_unprepared(
            "INSERT INTO search_events (query_text, search_type, result_count, latency_ms, created_at) \
             VALUES ('old-zero', 'fts', 0, 10, '2020-01-01T00:00:00')",
        )
        .await
        .unwrap();
        record_search_event(&db, &fts_event("new-hit", 5, 10)).await.unwrap();

        let stats = get_search_stats(&db, Some("2025-01-01T00:00:00")).await.unwrap();
        assert_eq!(stats.zero_result_rate, 0.0);
    }

    #[tokio::test]
    async fn temporal_filter_affects_avg_latency() {
        let (db, _tmp) = setup().await;
        db.execute_unprepared(
            "INSERT INTO search_events (query_text, search_type, result_count, latency_ms, created_at) \
             VALUES ('old', 'fts', 1, 1000, '2020-01-01T00:00:00')",
        )
        .await
        .unwrap();
        record_search_event(&db, &fts_event("new", 1, 50)).await.unwrap();

        let stats = get_search_stats(&db, Some("2025-01-01T00:00:00")).await.unwrap();
        assert!((stats.avg_latency_ms - 50.0).abs() < 0.01);
    }
}
