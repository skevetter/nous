use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::error::NousError;
use crate::fts::sanitize_fts5_query;

use super::{Message, SearchMessagesRequest};

pub async fn search_messages(
    db: &DatabaseConnection,
    request: SearchMessagesRequest,
) -> Result<Vec<Message>, NousError> {
    if request.query.trim().is_empty() {
        return Err(NousError::Validation("search query cannot be empty".into()));
    }

    let limit = request.limit.unwrap_or(20).min(100);

    let sanitized = sanitize_fts5_query(&request.query);

    let (sql, binds): (&str, Vec<sea_orm::Value>) = if let Some(ref room_id) = request.room_id {
        (
            "SELECT rm.* FROM room_messages rm \
             JOIN room_messages_fts fts ON rm.rowid = fts.rowid \
             WHERE room_messages_fts MATCH ? AND rm.room_id = ? \
             ORDER BY fts.rank \
             LIMIT ?",
            vec![
                sanitized.clone().into(),
                room_id.clone().into(),
                i64::from(limit).into(),
            ],
        )
    } else {
        (
            "SELECT rm.* FROM room_messages rm \
             JOIN room_messages_fts fts ON rm.rowid = fts.rowid \
             WHERE room_messages_fts MATCH ? \
             ORDER BY fts.rank \
             LIMIT ?",
            vec![sanitized.into(), i64::from(limit).into()],
        )
    };

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, sql, binds);
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}
