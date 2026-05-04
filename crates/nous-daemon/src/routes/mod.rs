pub mod agents;
pub mod health;
pub mod mcp;
pub mod memory;
pub mod messages;
pub mod resources;
pub mod rooms;
pub mod schedules;
pub mod search;
pub mod tasks;
pub mod worktrees;

use crate::error::AppError;
use sea_orm::{ConnectionTrait, Statement};

pub(crate) async fn count_total(
    db: &nous_core::db::DatabaseConnection,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Result<usize, AppError> {
    let row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            sql,
            values,
        ))
        .await
        .map_err(nous_core::error::NousError::SeaOrm)?;
    let count: i32 = row
        .map(|r| r.try_get_by::<i32, _>("cnt"))
        .transpose()
        .map_err(nous_core::error::NousError::SeaOrm)?
        .unwrap_or(0);
    // count is a non-negative DB row count; safe to cast
    Ok(usize::try_from(count).unwrap_or(0))
}
