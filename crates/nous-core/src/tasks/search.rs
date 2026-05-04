use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::entities::tasks as task_entity;
use crate::error::NousError;

use super::types::Task;

pub async fn search_tasks(
    db: &DatabaseConnection,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<Task>, NousError> {
    if query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = limit.unwrap_or(20).min(100);

    let rows = db
        .query_all(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT t.* FROM tasks t \
             JOIN tasks_fts fts ON t.rowid = fts.rowid \
             WHERE tasks_fts MATCH ?1 \
             LIMIT ?2",
            [query.into(), (limit as i64).into()],
        ))
        .await?;

    let mut tasks = Vec::new();
    for row in rows {
        let m = <task_entity::Model as sea_orm::FromQueryResult>::from_query_result(&row, "")?;
        tasks.push(Task::from_model(m));
    }
    Ok(tasks)
}
