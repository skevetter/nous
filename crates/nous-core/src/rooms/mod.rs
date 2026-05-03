use sea_orm::entity::prelude::*;
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryOrder, Set, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entities::rooms as rooms_entity;
use crate::error::NousError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl Room {
    fn from_model(m: rooms_entity::Model) -> Self {
        let metadata = m
            .metadata
            .as_deref()
            .and_then(|s| match serde_json::from_str(s) {
                Ok(val) => Some(val),
                Err(e) => {
                    tracing::warn!(error = %e, "malformed JSON in room metadata column, treating as null");
                    None
                }
            });
        Self {
            id: m.id,
            name: m.name,
            purpose: m.purpose,
            metadata,
            archived: m.archived,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

pub async fn create_room(
    db: &DatabaseConnection,
    name: &str,
    purpose: Option<&str>,
    metadata: Option<&serde_json::Value>,
) -> Result<Room, NousError> {
    if name.trim().is_empty() {
        return Err(NousError::Validation("room name cannot be empty".into()));
    }

    let id = Uuid::now_v7().to_string();
    let metadata_json = metadata.map(|m| m.to_string());

    let model = rooms_entity::ActiveModel {
        id: Set(id.clone()),
        name: Set(name.to_string()),
        purpose: Set(purpose.map(String::from)),
        metadata: Set(metadata_json),
        archived: Set(false),
        created_at: Set(String::new()),
        updated_at: Set(String::new()),
    };

    let result = rooms_entity::Entity::insert(model).exec(db).await;

    match result {
        Ok(_) => {}
        Err(ref e) if e.to_string().contains("2067") || e.to_string().contains("UNIQUE") => {
            return Err(NousError::Conflict(format!(
                "room with name '{name}' already exists"
            )));
        }
        Err(e) => return Err(NousError::SeaOrm(e)),
    }

    get_room(db, &id).await
}

pub async fn list_rooms(
    db: &DatabaseConnection,
    include_archived: bool,
) -> Result<Vec<Room>, NousError> {
    let mut query = rooms_entity::Entity::find();

    if !include_archived {
        query = query.filter(rooms_entity::Column::Archived.eq(false));
    }

    let models = query
        .order_by_desc(rooms_entity::Column::CreatedAt)
        .all(db)
        .await?;

    Ok(models.into_iter().map(Room::from_model).collect())
}

pub async fn get_room(db: &DatabaseConnection, id_or_name: &str) -> Result<Room, NousError> {
    let model = rooms_entity::Entity::find_by_id(id_or_name)
        .one(db)
        .await?;

    if let Some(m) = model {
        return Ok(Room::from_model(m));
    }

    let model = rooms_entity::Entity::find()
        .filter(rooms_entity::Column::Name.eq(id_or_name))
        .filter(rooms_entity::Column::Archived.eq(false))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(Room::from_model(m)),
        None => Err(NousError::NotFound(format!(
            "room '{id_or_name}' not found"
        ))),
    }
}

pub async fn delete_room(db: &DatabaseConnection, id: &str, hard: bool) -> Result<(), NousError> {
    if hard {
        let result = rooms_entity::Entity::delete_by_id(id).exec(db).await?;

        if result.rows_affected == 0 {
            return Err(NousError::NotFound(format!("room '{id}' not found")));
        }
    } else {
        archive_room(db, id).await?;
    }

    Ok(())
}

pub async fn archive_room(db: &DatabaseConnection, id: &str) -> Result<(), NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE rooms SET archived = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [id.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("room '{id}' not found")));
    }

    Ok(())
}

pub async fn unarchive_room(db: &DatabaseConnection, id: &str) -> Result<Room, NousError> {
    let result = db
        .execute(Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "UPDATE rooms SET archived = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            [id.into()],
        ))
        .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("room '{id}' not found")));
    }

    get_room(db, id).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomStats {
    pub room_id: String,
    pub message_count: i64,
    pub last_message_at: Option<String>,
    pub subscriber_count: i64,
}

pub async fn inspect_room(db: &DatabaseConnection, id: &str) -> Result<RoomStats, NousError> {
    let _room = get_room(db, id).await?;

    let message_count = crate::entities::room_messages::Entity::find()
        .filter(crate::entities::room_messages::Column::RoomId.eq(id))
        .count(db)
        .await? as i64;

    let last_message_at: Option<String> = {
        let row = db
            .query_one(Statement::from_sql_and_values(
                sea_orm::DbBackend::Sqlite,
                "SELECT MAX(created_at) as val FROM room_messages WHERE room_id = ?",
                [id.into()],
            ))
            .await?;
        row.and_then(|r| r.try_get_by::<Option<String>, _>("val").ok().flatten())
    };

    let subscriber_count = crate::entities::room_subscriptions::Entity::find()
        .filter(crate::entities::room_subscriptions::Column::RoomId.eq(id))
        .count(db)
        .await? as i64;

    Ok(RoomStats {
        room_id: id.to_string(),
        message_count,
        last_message_at,
        subscriber_count,
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
        pools.run_migrations("porter unicode61").await.unwrap();
        let db = pools.fts.clone();
        (db, tmp)
    }

    #[tokio::test]
    async fn test_create_room() {
        let (db, _tmp) = setup().await;

        let room = create_room(&db, "general", Some("General discussion"), None)
            .await
            .unwrap();

        assert_eq!(room.name, "general");
        assert_eq!(room.purpose.as_deref(), Some("General discussion"));
        assert!(!room.archived);
        assert!(!room.id.is_empty());
    }

    #[tokio::test]
    async fn test_list_rooms_excludes_archived() {
        let (db, _tmp) = setup().await;

        let room1 = create_room(&db, "active-room", None, None).await.unwrap();
        let room2 = create_room(&db, "archived-room", None, None)
            .await
            .unwrap();
        archive_room(&db, &room2.id).await.unwrap();

        let active = list_rooms(&db, false).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, room1.id);

        let all = list_rooms(&db, true).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_get_room_by_id_and_name() {
        let (db, _tmp) = setup().await;

        let room = create_room(&db, "lookup-test", Some("test"), None)
            .await
            .unwrap();

        let by_id = get_room(&db, &room.id).await.unwrap();
        assert_eq!(by_id.name, "lookup-test");

        let by_name = get_room(&db, "lookup-test").await.unwrap();
        assert_eq!(by_name.id, room.id);

        let not_found = get_room(&db, "nonexistent").await;
        assert!(matches!(not_found, Err(NousError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_delete_room_soft_and_hard() {
        let (db, _tmp) = setup().await;

        let room = create_room(&db, "soft-delete", None, None).await.unwrap();
        delete_room(&db, &room.id, false).await.unwrap();

        let fetched = get_room(&db, &room.id).await.unwrap();
        assert!(fetched.archived);

        let room2 = create_room(&db, "hard-delete", None, None).await.unwrap();
        delete_room(&db, &room2.id, true).await.unwrap();

        let not_found = get_room(&db, &room2.id).await;
        assert!(matches!(not_found, Err(NousError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_create_duplicate_name_conflict() {
        let (db, _tmp) = setup().await;

        create_room(&db, "unique-name", None, None).await.unwrap();
        let duplicate = create_room(&db, "unique-name", None, None).await;

        assert!(matches!(duplicate, Err(NousError::Conflict(_))));
    }
}
