use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

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
    fn from_row(row: &sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        let metadata_str: Option<String> = row.try_get("metadata")?;
        let metadata = metadata_str
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .unwrap_or(None);
        let archived: i32 = row.try_get("archived")?;

        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            purpose: row.try_get("purpose")?,
            metadata,
            archived: archived != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

pub async fn create_room(
    pool: &SqlitePool,
    name: &str,
    purpose: Option<&str>,
    metadata: Option<&serde_json::Value>,
) -> Result<Room, NousError> {
    let id = Uuid::now_v7().to_string();
    let metadata_json = metadata.map(|m| m.to_string());

    let result = sqlx::query("INSERT INTO rooms (id, name, purpose, metadata) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(name)
        .bind(purpose)
        .bind(&metadata_json)
        .execute(pool)
        .await;

    match result {
        Ok(_) => {}
        Err(sqlx::Error::Database(ref db_err)) if db_err.code().as_deref() == Some("2067") => {
            return Err(NousError::Conflict(format!(
                "room with name '{name}' already exists"
            )));
        }
        Err(e) => return Err(NousError::Sqlite(e)),
    }

    get_room(pool, &id).await
}

pub async fn list_rooms(pool: &SqlitePool, include_archived: bool) -> Result<Vec<Room>, NousError> {
    let query = if include_archived {
        "SELECT * FROM rooms ORDER BY created_at DESC"
    } else {
        "SELECT * FROM rooms WHERE archived = 0 ORDER BY created_at DESC"
    };

    let rows = sqlx::query(query).fetch_all(pool).await?;

    rows.iter()
        .map(Room::from_row)
        .collect::<Result<Vec<_>, _>>()
        .map_err(NousError::Sqlite)
}

pub async fn get_room(pool: &SqlitePool, id_or_name: &str) -> Result<Room, NousError> {
    let row = sqlx::query("SELECT * FROM rooms WHERE id = ?")
        .bind(id_or_name)
        .fetch_optional(pool)
        .await?;

    if let Some(row) = row {
        return Room::from_row(&row).map_err(NousError::Sqlite);
    }

    let row = sqlx::query("SELECT * FROM rooms WHERE name = ? AND archived = 0")
        .bind(id_or_name)
        .fetch_optional(pool)
        .await?;

    match row {
        Some(row) => Room::from_row(&row).map_err(NousError::Sqlite),
        None => Err(NousError::NotFound(format!(
            "room '{id_or_name}' not found"
        ))),
    }
}

pub async fn delete_room(pool: &SqlitePool, id: &str, hard: bool) -> Result<(), NousError> {
    if hard {
        let result = sqlx::query("DELETE FROM rooms WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(NousError::NotFound(format!("room '{id}' not found")));
        }
    } else {
        archive_room(pool, id).await?;
    }

    Ok(())
}

pub async fn archive_room(pool: &SqlitePool, id: &str) -> Result<(), NousError> {
    let result = sqlx::query(
        "UPDATE rooms SET archived = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(NousError::NotFound(format!("room '{id}' not found")));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPools;
    use tempfile::TempDir;

    async fn setup() -> (SqlitePool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let pools = DbPools::connect(tmp.path()).await.unwrap();
        pools.run_migrations().await.unwrap();
        let pool = pools.fts.clone();
        (pool, tmp)
    }

    #[tokio::test]
    async fn test_create_room() {
        let (pool, _tmp) = setup().await;

        let room = create_room(&pool, "general", Some("General discussion"), None)
            .await
            .unwrap();

        assert_eq!(room.name, "general");
        assert_eq!(room.purpose.as_deref(), Some("General discussion"));
        assert!(!room.archived);
        assert!(!room.id.is_empty());
    }

    #[tokio::test]
    async fn test_list_rooms_excludes_archived() {
        let (pool, _tmp) = setup().await;

        let room1 = create_room(&pool, "active-room", None, None).await.unwrap();
        let room2 = create_room(&pool, "archived-room", None, None)
            .await
            .unwrap();
        archive_room(&pool, &room2.id).await.unwrap();

        let active = list_rooms(&pool, false).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, room1.id);

        let all = list_rooms(&pool, true).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_get_room_by_id_and_name() {
        let (pool, _tmp) = setup().await;

        let room = create_room(&pool, "lookup-test", Some("test"), None)
            .await
            .unwrap();

        let by_id = get_room(&pool, &room.id).await.unwrap();
        assert_eq!(by_id.name, "lookup-test");

        let by_name = get_room(&pool, "lookup-test").await.unwrap();
        assert_eq!(by_name.id, room.id);

        let not_found = get_room(&pool, "nonexistent").await;
        assert!(matches!(not_found, Err(NousError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_delete_room_soft_and_hard() {
        let (pool, _tmp) = setup().await;

        let room = create_room(&pool, "soft-delete", None, None).await.unwrap();
        delete_room(&pool, &room.id, false).await.unwrap();

        let fetched = get_room(&pool, &room.id).await.unwrap();
        assert!(fetched.archived);

        let room2 = create_room(&pool, "hard-delete", None, None).await.unwrap();
        delete_room(&pool, &room2.id, true).await.unwrap();

        let not_found = get_room(&pool, &room2.id).await;
        assert!(matches!(not_found, Err(NousError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_create_duplicate_name_conflict() {
        let (pool, _tmp) = setup().await;

        create_room(&pool, "unique-name", None, None).await.unwrap();
        let duplicate = create_room(&pool, "unique-name", None, None).await;

        assert!(matches!(duplicate, Err(NousError::Conflict(_))));
    }
}
