use nous_core::db::MemoryDb;
use nous_shared::ids::MemoryId;

use crate::config::Config;

pub fn run_room_create(
    config: &Config,
    name: &str,
    purpose: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let id = MemoryId::new().to_string();
    MemoryDb::create_room_on(db.connection(), &id, name, purpose, None)?;
    println!("{id}");
    Ok(())
}

pub fn run_room_list(
    config: &Config,
    archived: bool,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let limit = limit.unwrap_or(100) as i64;
    let mut stmt = conn.prepare(
        "SELECT id, name, purpose, archived, created_at FROM rooms WHERE archived = ?1 ORDER BY created_at DESC LIMIT ?2",
    )?;
    let rows: Vec<(String, String, Option<String>, i64, String)> = stmt
        .query_map(rusqlite::params![archived as i64, limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    if rows.is_empty() {
        println!("No rooms found.");
        return Ok(());
    }

    for (id, name, purpose, _archived, created_at) in &rows {
        let p = purpose.as_deref().unwrap_or("");
        println!("{id}  {name}  {p}  {created_at}");
    }
    Ok(())
}

pub fn run_room_get(config: &Config, id_or_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();

    let room_id = resolve_room_id_sync(conn, id_or_name)?;

    let (name, purpose, archived, created_at): (String, Option<String>, i64, String) = conn
        .query_row(
            "SELECT name, purpose, archived, created_at FROM rooms WHERE id = ?1",
            rusqlite::params![room_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

    let msg_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM room_messages WHERE room_id = ?1",
        rusqlite::params![room_id],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(
        "SELECT agent_id, role FROM room_participants WHERE room_id = ?1 ORDER BY joined_at",
    )?;
    let participants: Vec<(String, String)> = stmt
        .query_map(rusqlite::params![room_id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<Result<_, _>>()?;

    println!("id: {room_id}");
    println!("name: {name}");
    if let Some(p) = purpose {
        println!("purpose: {p}");
    }
    println!("archived: {}", archived != 0);
    println!("created: {created_at}");
    println!("messages: {msg_count}");
    if !participants.is_empty() {
        println!("participants:");
        for (agent_id, role) in &participants {
            println!("  {agent_id} ({role})");
        }
    }
    Ok(())
}

pub fn run_room_post(
    config: &Config,
    room: &str,
    content: &str,
    sender: Option<&str>,
    reply_to: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, room)?;
    let msg_id = MemoryId::new().to_string();
    let sender_id = sender.unwrap_or("cli");
    MemoryDb::post_message_on(conn, &msg_id, &room_id, sender_id, content, reply_to, None)?;
    println!("{msg_id}");
    Ok(())
}

pub fn run_room_read(
    config: &Config,
    room: &str,
    limit: Option<usize>,
    since: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, room)?;
    let limit = limit.unwrap_or(50) as i64;

    let (sql, params): (String, Vec<Box<dyn rusqlite::ToSql>>) = match since {
        Some(s) => (
            "SELECT sender_id, content, created_at FROM room_messages WHERE room_id = ?1 AND created_at > ?2 ORDER BY created_at ASC LIMIT ?3".into(),
            vec![Box::new(room_id), Box::new(s.to_string()), Box::new(limit)],
        ),
        None => (
            "SELECT sender_id, content, created_at FROM room_messages WHERE room_id = ?1 ORDER BY created_at ASC LIMIT ?2".into(),
            vec![Box::new(room_id), Box::new(limit)],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows: Vec<(String, String, String)> = stmt
        .query_map(params_ref.as_slice(), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    for (sender_id, content, created_at) in &rows {
        println!("[{created_at}] {sender_id}: {content}");
    }
    Ok(())
}

pub fn run_room_search(
    config: &Config,
    room: &str,
    query: &str,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, room)?;
    let limit = limit.unwrap_or(50) as i64;

    let mut stmt = conn.prepare(
        "SELECT m.sender_id, m.content, m.created_at
         FROM room_messages m
         JOIN room_messages_fts ON m.rowid = room_messages_fts.rowid
         WHERE room_messages_fts MATCH ?1 AND m.room_id = ?2
         ORDER BY m.created_at DESC LIMIT ?3",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(rusqlite::params![query, room_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;

    for (sender_id, content, created_at) in &rows {
        println!("[{created_at}] {sender_id}: {content}");
    }
    Ok(())
}

pub fn run_room_delete(
    config: &Config,
    id_or_name: &str,
    hard: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let db_key = config.resolve_db_key().ok();
    let db = MemoryDb::open(&config.memory.db_path, db_key.as_deref(), 384)?;
    let conn = db.connection();
    let room_id = resolve_room_id_sync(conn, id_or_name)?;

    let result = if hard {
        MemoryDb::hard_delete_room_on(conn, &room_id)?
    } else {
        MemoryDb::archive_room_on(conn, &room_id)?
    };

    if result {
        if hard {
            println!("Deleted room {room_id}");
        } else {
            println!("Archived room {room_id}");
        }
    } else {
        return Err(format!("room not found: {id_or_name}").into());
    }
    Ok(())
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36 && s.contains('-')
}

fn resolve_room_id_sync(
    conn: &rusqlite::Connection,
    id_or_name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    if looks_like_uuid(id_or_name) {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?1)",
            rusqlite::params![id_or_name],
            |row| row.get(0),
        )?;
        if exists {
            return Ok(id_or_name.to_string());
        }
    }
    let id: String = conn
        .query_row(
            "SELECT id FROM rooms WHERE name = ?1 AND archived = 0",
            rusqlite::params![id_or_name],
            |row| row.get(0),
        )
        .map_err(|_| format!("room not found: {id_or_name}"))?;
    Ok(id)
}
