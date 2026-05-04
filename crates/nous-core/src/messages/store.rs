use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, NotSet, Set, Statement};
use uuid::Uuid;

use crate::entities::message_cursors as cursor_entity;
use crate::entities::room_messages as msg_entity;
use crate::error::NousError;
use crate::notifications::{Notification, NotificationRegistry};

use super::{
    ChatCursor, GetThreadRequest, MarkReadRequest, Message, PostMessageRequest,
    ReadMessagesRequest, ThreadView, UnreadCountRequest,
};

pub async fn post_message(
    db: &DatabaseConnection,
    request: PostMessageRequest,
    registry: Option<&NotificationRegistry>,
) -> Result<Message, NousError> {
    if request.content.trim().is_empty() {
        return Err(NousError::Validation(
            "message content cannot be empty".into(),
        ));
    }

    let room_exists = crate::entities::rooms::Entity::find_by_id(&request.room_id)
        .count(db)
        .await?
        > 0;

    if !room_exists {
        return Err(NousError::NotFound(format!(
            "room '{}' not found",
            request.room_id
        )));
    }

    let id = Uuid::now_v7().to_string();
    let metadata_json = request.metadata.as_ref().map(std::string::ToString::to_string);
    let msg_type = request.message_type.unwrap_or_default().to_string();

    let model = msg_entity::ActiveModel {
        id: Set(id.clone()),
        room_id: Set(request.room_id.clone()),
        sender_id: Set(request.sender_id.clone()),
        content: Set(request.content.clone()),
        reply_to: Set(request.reply_to.clone()),
        metadata: Set(metadata_json),
        message_type: Set(msg_type),
        created_at: NotSet,
    };

    msg_entity::Entity::insert(model).exec(db).await?;

    let row = msg_entity::Entity::find_by_id(&id).one(db).await?;

    let message = Message::from_model(
        row.ok_or_else(|| NousError::Internal("inserted message not found".into()))?,
    );

    if let Some(registry) = registry {
        let (topics, mentions) = extract_topics_mentions(request.metadata.as_ref());
        registry
            .notify(Notification {
                room_id: message.room_id.clone(),
                message_id: message.id.clone(),
                sender_id: message.sender_id.clone(),
                priority: crate::notifications::NotificationPriority::Normal,
                topics,
                mentions,
            })
            .await;
    }

    Ok(message)
}

fn extract_topics_mentions(metadata: Option<&serde_json::Value>) -> (Vec<String>, Vec<String>) {
    let Some(meta) = metadata else {
        return (vec![], vec![]);
    };

    let topics = meta
        .get("topics")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mentions = meta
        .get("mentions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    (topics, mentions)
}

pub async fn read_messages(
    db: &DatabaseConnection,
    request: ReadMessagesRequest,
) -> Result<Vec<Message>, NousError> {
    let room_exists = crate::entities::rooms::Entity::find_by_id(&request.room_id)
        .count(db)
        .await?
        > 0;

    if !room_exists {
        return Err(NousError::NotFound(format!(
            "room '{}' not found",
            request.room_id
        )));
    }

    let limit = request.limit.unwrap_or(50).min(200);
    let mut sql = String::from("SELECT * FROM room_messages WHERE room_id = ?");
    let mut binds: Vec<sea_orm::Value> = vec![request.room_id.clone().into()];

    if let Some(ref since) = request.since {
        sql.push_str(" AND created_at > ?");
        binds.push(since.clone().into());
    }

    if let Some(ref before) = request.before {
        sql.push_str(" AND created_at < ?");
        binds.push(before.clone().into());
    }

    sql.push_str(" ORDER BY created_at ASC LIMIT ?");
    binds.push(i64::from(limit).into());

    let stmt = Statement::from_sql_and_values(sea_orm::DbBackend::Sqlite, &sql, binds);
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn list_mentions(
    db: &DatabaseConnection,
    room_id: &str,
    agent_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Message>, NousError> {
    let limit = limit.unwrap_or(50).min(200);
    let pattern = format!("%@{agent_id}%");

    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "SELECT * FROM room_messages WHERE room_id = ? AND content LIKE ? ORDER BY created_at DESC LIMIT ?",
        [room_id.into(), pattern.into(), i64::from(limit).into()],
    );
    let rows = db.query_all(stmt).await?;

    rows.iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)
}

pub async fn get_thread(
    db: &DatabaseConnection,
    req: GetThreadRequest,
) -> Result<ThreadView, NousError> {
    let stmt = Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "SELECT * FROM room_messages WHERE id = ? OR reply_to = ? ORDER BY created_at ASC",
        [
            req.root_message_id.clone().into(),
            req.root_message_id.clone().into(),
        ],
    );
    let rows = db.query_all(stmt).await?;

    let messages: Vec<Message> = rows
        .iter()
        .map(Message::from_query_result)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::SeaOrm)?;

    let root = messages
        .iter()
        .find(|m| m.id == req.root_message_id)
        .cloned()
        .ok_or_else(|| {
            NousError::NotFound(format!("message '{}' not found", req.root_message_id))
        })?;

    let replies: Vec<Message> = messages
        .into_iter()
        .filter(|m| m.id != req.root_message_id)
        .collect();
    let reply_count = replies.len();

    Ok(ThreadView {
        root,
        replies,
        reply_count,
    })
}

pub async fn mark_read(
    db: &DatabaseConnection,
    req: MarkReadRequest,
) -> Result<ChatCursor, NousError> {
    let msg_model = msg_entity::Entity::find_by_id(&req.message_id)
        .one(db)
        .await?
        .ok_or_else(|| NousError::NotFound(format!("message '{}' not found", req.message_id)))?;

    let msg_created_at = msg_model.created_at;

    db.execute(Statement::from_sql_and_values(
        sea_orm::DbBackend::Sqlite,
        "INSERT INTO message_cursors (room_id, agent_id, last_read_message_id, last_read_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT (room_id, agent_id) DO UPDATE SET \
         last_read_message_id = excluded.last_read_message_id, \
         last_read_at = excluded.last_read_at",
        [
            req.room_id.clone().into(),
            req.agent_id.clone().into(),
            req.message_id.clone().into(),
            msg_created_at.clone().into(),
        ],
    ))
    .await?;

    let unread_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ? AND created_at > ?",
            [req.room_id.clone().into(), msg_created_at.clone().into()],
        ))
        .await?;

    let unread: i64 = unread_row
        .map_or(0, |r| r.try_get_by::<i64, _>("cnt").unwrap_or(0));

    Ok(ChatCursor {
        room_id: req.room_id,
        agent_id: req.agent_id,
        last_read_message_id: req.message_id,
        last_read_at: msg_created_at,
        unread_count: unread,
    })
}

pub async fn unread_count(
    db: &DatabaseConnection,
    req: UnreadCountRequest,
) -> Result<ChatCursor, NousError> {
    let cursor_model =
        cursor_entity::Entity::find_by_id((req.room_id.clone(), req.agent_id.clone()))
            .one(db)
            .await?;

    let (last_read_message_id, last_read_at) = if let Some(m) = cursor_model { (m.last_read_message_id, m.last_read_at) } else {
        let count_row = db
            .query_one(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ?",
                [req.room_id.clone().into()],
            ))
            .await?;

        let count: i64 = count_row
            .map_or(0, |r| r.try_get_by::<i64, _>("cnt").unwrap_or(0));

        return Ok(ChatCursor {
            room_id: req.room_id,
            agent_id: req.agent_id,
            last_read_message_id: String::new(),
            last_read_at: String::new(),
            unread_count: count,
        });
    };

    let unread_row = db
        .query_one(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT COUNT(*) as cnt FROM room_messages WHERE room_id = ? AND created_at > ?",
            [req.room_id.clone().into(), last_read_at.clone().into()],
        ))
        .await?;

    let unread: i64 = unread_row
        .map_or(0, |r| r.try_get_by::<i64, _>("cnt").unwrap_or(0));

    Ok(ChatCursor {
        room_id: req.room_id,
        agent_id: req.agent_id,
        last_read_message_id,
        last_read_at,
        unread_count: unread,
    })
}
