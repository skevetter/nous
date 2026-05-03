use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, NotSet, Set, Statement};
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
        id: Default::default(),
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

pub async fn get_search_stats(
    db: &DatabaseConnection,
    since: Option<&str>,
) -> Result<SearchStats, NousError> {
    let since_clause = if since.is_some() {
        " WHERE created_at >= ?"
    } else {
        ""
    };

    // Total count
    let total_sql = format!("SELECT COUNT(*) as cnt FROM search_events{since_clause}");
    let total_searches: i64 = {
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(since_val) = since {
            values.push(since_val.into());
        }
        let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &total_sql, values);
        let row = db.query_one(stmt).await?;
        row.map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
            .unwrap_or(0)
    };

    // Type counts
    let type_count_sql = format!(
        "SELECT \
         COALESCE(SUM(CASE WHEN search_type = 'fts' THEN 1 ELSE 0 END), 0) as fts_cnt, \
         COALESCE(SUM(CASE WHEN search_type = 'vector' THEN 1 ELSE 0 END), 0) as vec_cnt, \
         COALESCE(SUM(CASE WHEN search_type = 'hybrid' THEN 1 ELSE 0 END), 0) as hyb_cnt \
         FROM search_events{since_clause}"
    );
    let (fts_count, vector_count, hybrid_count): (i64, i64, i64) = {
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(since_val) = since {
            values.push(since_val.into());
        }
        let stmt =
            Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &type_count_sql, values);
        let row = db.query_one(stmt).await?;
        match row {
            Some(r) => (
                r.try_get_by::<i64, _>("fts_cnt").unwrap_or(0),
                r.try_get_by::<i64, _>("vec_cnt").unwrap_or(0),
                r.try_get_by::<i64, _>("hyb_cnt").unwrap_or(0),
            ),
            None => (0, 0, 0),
        }
    };

    // Zero-result count
    let zero_sql = format!(
        "SELECT COUNT(*) as cnt FROM search_events WHERE result_count = 0{}",
        if since.is_some() {
            " AND created_at >= ?"
        } else {
            ""
        }
    );
    let zero_count: i64 = {
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(since_val) = since {
            values.push(since_val.into());
        }
        let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &zero_sql, values);
        let row = db.query_one(stmt).await?;
        row.map(|r| r.try_get_by::<i64, _>("cnt").unwrap_or(0))
            .unwrap_or(0)
    };

    let zero_result_rate = if total_searches > 0 {
        zero_count as f64 / total_searches as f64 * 100.0
    } else {
        0.0
    };

    // Avg latency
    let avg_sql = format!(
        "SELECT CAST(COALESCE(AVG(latency_ms * 1.0), 0.0) AS REAL) as avg_lat FROM search_events{since_clause}"
    );
    let avg_latency_ms: f64 = {
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(since_val) = since {
            values.push(since_val.into());
        }
        let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &avg_sql, values);
        let row = db.query_one(stmt).await?;
        row.map(|r| r.try_get_by::<f64, _>("avg_lat").unwrap_or(0.0))
            .unwrap_or(0.0)
    };

    // Top queries
    let top_sql = format!(
        "SELECT query_text, COUNT(*) as cnt FROM search_events{since_clause} \
         GROUP BY query_text ORDER BY cnt DESC LIMIT 10"
    );
    let top_queries: Vec<TopQuery> = {
        let mut values: Vec<sea_orm::Value> = Vec::new();
        if let Some(since_val) = since {
            values.push(since_val.into());
        }
        let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &top_sql, values);
        let rows = db.query_all(stmt).await?;
        rows.iter()
            .filter_map(|r| {
                let query_text: String = r.try_get_by("query_text").ok()?;
                let count: i64 = r.try_get_by("cnt").ok()?;
                Some(TopQuery { query_text, count })
            })
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
