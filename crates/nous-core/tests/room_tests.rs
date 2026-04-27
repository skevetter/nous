use nous_core::channel::{ReadPool, WriteChannel};
use nous_core::db::MemoryDb;

fn open_test_db() -> MemoryDb {
    MemoryDb::open(":memory:", None, 384).expect("failed to open in-memory db")
}

#[test]
fn fresh_db_has_room_schema() {
    let db = open_test_db();
    let conn = db.connection();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('rooms', 'room_participants', 'room_messages', 'room_messages_fts')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 4,
        "expected 4 room-related objects (3 tables + 1 FTS), got {count}"
    );
}

#[test]
fn room_tables_exist() {
    let db = open_test_db();
    let conn = db.connection();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    for table in &["rooms", "room_participants", "room_messages"] {
        assert!(
            tables.contains(&table.to_string()),
            "missing table: {table}"
        );
    }
}

#[test]
fn room_fts_table_exists() {
    let db = open_test_db();
    let conn = db.connection();
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='room_messages_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "room_messages_fts FTS5 table should exist");
}

#[test]
fn room_triggers_exist() {
    let db = open_test_db();
    let conn = db.connection();
    let triggers: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    for trigger in &["room_messages_ai", "room_messages_au", "room_messages_ad"] {
        assert!(
            triggers.contains(&trigger.to_string()),
            "missing trigger: {trigger}"
        );
    }
}

#[test]
fn room_indexes_exist() {
    let db = open_test_db();
    let conn = db.connection();
    let indexes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    for idx in &[
        "idx_rooms_name",
        "idx_messages_room_created",
        "idx_messages_sender",
    ] {
        assert!(
            indexes.contains(&idx.to_string()),
            "missing index: {idx}, found: {indexes:?}"
        );
    }
}

#[tokio::test]
async fn room_create_and_read_round_trip() {
    let db = open_test_db();
    let _db_path = {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let path_str = path.to_str().unwrap().to_string();
        drop(db);

        let db = MemoryDb::open(&path_str, None, 384).unwrap();
        let (write_channel, _handle) = WriteChannel::new(db);
        let read_pool = ReadPool::new(&path_str, None, 1).unwrap();

        let room_id = nous_shared::ids::MemoryId::new().to_string();
        write_channel
            .create_room(
                room_id.clone(),
                "test-room".to_string(),
                Some("A test room".to_string()),
                None,
            )
            .await
            .unwrap();

        let room = read_pool.get_room(&room_id).await.unwrap();
        assert!(room.is_some());
        let room = room.unwrap();
        assert_eq!(room.name, "test-room");
        assert_eq!(room.purpose.as_deref(), Some("A test room"));
        assert!(!room.archived);

        let rooms = read_pool.list_rooms(false, None).await.unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].id, room_id);

        let room_by_name = read_pool.get_room_by_name("test-room").await.unwrap();
        assert!(room_by_name.is_some());
        assert_eq!(room_by_name.unwrap().id, room_id);

        let msg_id = nous_shared::ids::MemoryId::new().to_string();
        write_channel
            .post_message(
                msg_id.clone(),
                room_id.clone(),
                "agent-1".to_string(),
                "Hello, world!".to_string(),
                None,
                None,
            )
            .await
            .unwrap();

        let msg_id2 = nous_shared::ids::MemoryId::new().to_string();
        write_channel
            .post_message(
                msg_id2.clone(),
                room_id.clone(),
                "agent-2".to_string(),
                "The linter failed on line 42".to_string(),
                Some(msg_id.clone()),
                None,
            )
            .await
            .unwrap();

        let messages = read_pool
            .list_messages(&room_id, None, None, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "The linter failed on line 42");
        assert_eq!(messages[1].content, "Hello, world!");

        let search_results = read_pool
            .search_messages(&room_id, "linter", None)
            .await
            .unwrap();
        assert_eq!(search_results.len(), 1);
        assert_eq!(search_results[0].sender_id, "agent-2");

        let info = read_pool.room_info(&room_id).await.unwrap();
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info["message_count"], 2);

        path_str
    };
}

#[tokio::test]
async fn room_archive_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let path_str = path.to_str().unwrap();

    let db = MemoryDb::open(path_str, None, 384).unwrap();
    let (write_channel, _handle) = WriteChannel::new(db);
    let read_pool = ReadPool::new(path_str, None, 1).unwrap();

    let room_id = nous_shared::ids::MemoryId::new().to_string();
    write_channel
        .create_room(room_id.clone(), "archive-me".to_string(), None, None)
        .await
        .unwrap();

    let archived = write_channel.archive_room(room_id.clone()).await.unwrap();
    assert!(archived);

    let active_rooms = read_pool.list_rooms(false, None).await.unwrap();
    assert!(active_rooms.is_empty());

    let archived_rooms = read_pool.list_rooms(true, None).await.unwrap();
    assert_eq!(archived_rooms.len(), 1);

    let room_id2 = nous_shared::ids::MemoryId::new().to_string();
    write_channel
        .create_room(room_id2.clone(), "delete-me".to_string(), None, None)
        .await
        .unwrap();

    let deleted = write_channel
        .delete_room(room_id2.clone(), true)
        .await
        .unwrap();
    assert!(deleted);

    let room = read_pool.get_room(&room_id2).await.unwrap();
    assert!(room.is_none());
}
