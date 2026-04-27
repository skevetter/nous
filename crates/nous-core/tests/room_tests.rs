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

#[test]
fn room_create_and_read_round_trip() {
    let db = open_test_db();
    let conn = db.connection();

    let room_id = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::create_room_on(conn, &room_id, "test-room", Some("A test room"), None).unwrap();

    let room: (String, String, Option<String>, i64) = conn
        .query_row(
            "SELECT id, name, purpose, archived FROM rooms WHERE id = ?1",
            rusqlite::params![room_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(room.0, room_id);
    assert_eq!(room.1, "test-room");
    assert_eq!(room.2.as_deref(), Some("A test room"));
    assert_eq!(room.3, 0);

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rooms WHERE archived = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    let name_lookup: String = conn
        .query_row(
            "SELECT id FROM rooms WHERE name = ?1 AND archived = 0",
            rusqlite::params!["test-room"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(name_lookup, room_id);

    let msg_id = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::post_message_on(conn, &msg_id, &room_id, "agent-1", "Hello, world!", None, None)
        .unwrap();

    let msg_id2 = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::post_message_on(
        conn,
        &msg_id2,
        &room_id,
        "agent-2",
        "The linter failed on line 42",
        Some(&msg_id),
        None,
    )
    .unwrap();

    let messages: Vec<(String, String)> = conn
        .prepare(
            "SELECT sender_id, content FROM room_messages WHERE room_id = ?1 ORDER BY created_at DESC",
        )
        .unwrap()
        .query_map(rusqlite::params![room_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].1, "The linter failed on line 42");
    assert_eq!(messages[1].1, "Hello, world!");

    let search_results: Vec<(String, String)> = conn
        .prepare(
            "SELECT m.sender_id, m.content FROM room_messages m
             JOIN room_messages_fts ON m.rowid = room_messages_fts.rowid
             WHERE room_messages_fts MATCH ?1 AND m.room_id = ?2",
        )
        .unwrap()
        .query_map(rusqlite::params!["linter", room_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(search_results.len(), 1);
    assert_eq!(search_results[0].0, "agent-2");

    let message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM room_messages WHERE room_id = ?1",
            rusqlite::params![room_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(message_count, 2);
}

#[test]
fn room_archive_and_delete() {
    let db = open_test_db();
    let conn = db.connection();

    let room_id = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::create_room_on(conn, &room_id, "archive-me", None, None).unwrap();

    let archived = MemoryDb::archive_room_on(conn, &room_id).unwrap();
    assert!(archived);

    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rooms WHERE archived = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);

    let archived_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM rooms WHERE archived = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived_count, 1);

    let room_id2 = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::create_room_on(conn, &room_id2, "delete-me", None, None).unwrap();

    let deleted = MemoryDb::hard_delete_room_on(conn, &room_id2).unwrap();
    assert!(deleted);

    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?1)",
            rusqlite::params![room_id2],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[test]
fn room_join_participant() {
    let db = open_test_db();
    let conn = db.connection();

    let room_id = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::create_room_on(conn, &room_id, "join-test", None, None).unwrap();

    MemoryDb::join_room_on(conn, &room_id, "agent-1", "owner").unwrap();
    MemoryDb::join_room_on(conn, &room_id, "agent-2", "member").unwrap();

    let participants: Vec<(String, String)> = conn
        .prepare("SELECT agent_id, role FROM room_participants WHERE room_id = ?1 ORDER BY agent_id")
        .unwrap()
        .query_map(rusqlite::params![room_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    assert_eq!(participants.len(), 2);
    assert_eq!(participants[0], ("agent-1".to_string(), "owner".to_string()));
    assert_eq!(participants[1], ("agent-2".to_string(), "member".to_string()));
}

#[test]
fn room_unique_name_constraint() {
    let db = open_test_db();
    let conn = db.connection();

    let room_id1 = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::create_room_on(conn, &room_id1, "unique-room", None, None).unwrap();

    let room_id2 = nous_shared::ids::MemoryId::new().to_string();
    let result = MemoryDb::create_room_on(conn, &room_id2, "unique-room", None, None);
    assert!(result.is_err(), "duplicate room name should fail");
}

#[test]
fn room_cascade_delete_messages() {
    let db = open_test_db();
    let conn = db.connection();

    let room_id = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::create_room_on(conn, &room_id, "cascade-test", None, None).unwrap();

    let msg_id = nous_shared::ids::MemoryId::new().to_string();
    MemoryDb::post_message_on(conn, &msg_id, &room_id, "agent-1", "message", None, None).unwrap();

    MemoryDb::hard_delete_room_on(conn, &room_id).unwrap();

    let msg_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM room_messages WHERE room_id = ?1",
            rusqlite::params![room_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(msg_count, 0, "messages should be cascade-deleted with room");
}
