use serde::Serialize;
use sqlx::SqlitePool;

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

pub async fn record_search_event(pool: &SqlitePool, event: &SearchEvent) -> Result<(), NousError> {
    sqlx::query(
        "INSERT INTO search_events (query_text, search_type, result_count, latency_ms, workspace_id, agent_id) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.query_text)
    .bind(&event.search_type)
    .bind(event.result_count)
    .bind(event.latency_ms)
    .bind(&event.workspace_id)
    .bind(&event.agent_id)
    .execute(pool)
    .await?;
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

pub async fn get_search_stats(
    pool: &SqlitePool,
    since: Option<&str>,
) -> Result<SearchStats, NousError> {
    let since_clause = if since.is_some() {
        " WHERE created_at >= ?"
    } else {
        ""
    };

    let total_sql = format!("SELECT COUNT(*) FROM search_events{since_clause}");
    let total_searches: i64 = if let Some(since_val) = since {
        sqlx::query_scalar(&total_sql)
            .bind(since_val)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_scalar(&total_sql).fetch_one(pool).await?
    };

    let type_count_sql = format!(
        "SELECT \
         COALESCE(SUM(CASE WHEN search_type = 'fts' THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN search_type = 'vector' THEN 1 ELSE 0 END), 0), \
         COALESCE(SUM(CASE WHEN search_type = 'hybrid' THEN 1 ELSE 0 END), 0) \
         FROM search_events{since_clause}"
    );

    let (fts_count, vector_count, hybrid_count): (i64, i64, i64) = if let Some(since_val) = since {
        sqlx::query_as::<_, (i64, i64, i64)>(&type_count_sql)
            .bind(since_val)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_as::<_, (i64, i64, i64)>(&type_count_sql)
            .fetch_one(pool)
            .await?
    };

    let zero_sql = format!(
        "SELECT COUNT(*) FROM search_events WHERE result_count = 0{}",
        if since.is_some() {
            " AND created_at >= ?"
        } else {
            ""
        }
    );
    let zero_count: i64 = if let Some(since_val) = since {
        sqlx::query_scalar(&zero_sql)
            .bind(since_val)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_scalar(&zero_sql).fetch_one(pool).await?
    };

    let zero_result_rate = if total_searches > 0 {
        zero_count as f64 / total_searches as f64 * 100.0
    } else {
        0.0
    };

    let avg_sql = format!(
        "SELECT CAST(COALESCE(AVG(latency_ms * 1.0), 0.0) AS REAL) FROM search_events{since_clause}"
    );
    let avg_latency_ms: f64 = if let Some(since_val) = since {
        sqlx::query_scalar(&avg_sql)
            .bind(since_val)
            .fetch_one(pool)
            .await?
    } else {
        sqlx::query_scalar(&avg_sql).fetch_one(pool).await?
    };

    let top_sql = format!(
        "SELECT query_text, COUNT(*) as cnt FROM search_events{since_clause} \
         GROUP BY query_text ORDER BY cnt DESC LIMIT 10"
    );
    let top_queries: Vec<TopQuery> = if let Some(since_val) = since {
        sqlx::query_as::<_, (String, i64)>(&top_sql)
            .bind(since_val)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(query_text, count)| TopQuery { query_text, count })
            .collect()
    } else {
        sqlx::query_as::<_, (String, i64)>(&top_sql)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|(query_text, count)| TopQuery { query_text, count })
            .collect()
    };

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
