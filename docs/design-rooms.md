# Design: Stable Room/Conversation System

**Initiative:** INI-032  
**Status:** Draft  
**Author:** Technical Writer (paseo agent)  
**Date:** 2026-04-27

## 1. Overview & Motivation

Paseo's current room/conversation system uses JSON files (`~/.paseo/rooms/{org_id}-{room_slug}.json`) to persist messages. This approach degrades at scale: loading and writing thousands of messages requires full-file deserialization and serialization on every read/write operation. Performance suffers with large rooms, concurrent access is unsafe, and there is no indexing or search capability. Nous already maintains a SQLite-backed memory system with proven concurrency patterns (WriteChannel/ReadPool, WAL mode, FTS5, vec0 semantic search). This design extends Nous to support persistent rooms and messages using the same infrastructure, replacing Paseo's JSON-backed storage with a scalable, indexed, and searchable solution.

**Goals:**
- Store rooms and messages in the same SQLite database as memories (shared connection pool, shared WriteChannel)
- Support thousands of messages per room, hundreds of rooms
- Fast reads (indexed queries), batched writes (channel-based)
- Full-text search (FTS5) for message content in MVP
- Optional semantic search (vec0) in future phase
- MCP tools for room CRUD and message operations
- CLI commands for room management

**Non-goals (deferred to future work):**
- Message editing (MVP is append-only)
- Semantic search over messages (FTS5 is sufficient for MVP)
- Per-workspace room scoping (rooms are globally unique by name)
- Retention policies or auto-archival

## 2. Data Model

Four new tables extend the existing schema:

**`rooms`** — room metadata and lifecycle

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUIDv7 (matches `memories.id` pattern) |
| `name` | TEXT | NOT NULL | Human-readable room name (e.g., "pr-123-review") |
| `purpose` | TEXT | nullable | Room description/purpose |
| `metadata` | TEXT | nullable | JSON blob for extensible fields (labels, timestamps, etc.) |
| `archived` | INTEGER | NOT NULL DEFAULT 0 | 0 = active, 1 = archived |
| `created_at` | TEXT | NOT NULL DEFAULT now | ISO8601 timestamp |
| `updated_at` | TEXT | NOT NULL DEFAULT now | ISO8601 timestamp |

**Unique constraint:** `UNIQUE INDEX idx_rooms_name ON rooms(name) WHERE archived = 0` — ensures active room names are unique, but archived rooms with the same name can exist.

**`room_participants`** — participant roles per room

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `room_id` | TEXT | FK rooms(id) CASCADE | Room reference |
| `agent_id` | TEXT | NOT NULL | Participant agent ID (paseo agent ID, "system", or user ID) |
| `role` | TEXT | NOT NULL DEFAULT 'member' | CHECK IN ('owner','member','observer') |
| `joined_at` | TEXT | NOT NULL DEFAULT now | ISO8601 timestamp |

**Primary key:** `(room_id, agent_id)`

**`room_messages`** — message storage

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUIDv7 (provides lexicographic ordering + uniqueness) |
| `room_id` | TEXT | FK rooms(id) CASCADE | Room reference |
| `sender_id` | TEXT | NOT NULL | Agent ID, "system", or user ID |
| `content` | TEXT | NOT NULL | Message body |
| `reply_to` | TEXT | nullable, FK room_messages(id) | Parent message ID for threads |
| `metadata` | TEXT | nullable | JSON blob (mentions, labels, attachments, etc.) |
| `created_at` | TEXT | NOT NULL DEFAULT now | ISO8601 timestamp |

**Indexes:**
- `idx_messages_room_created ON room_messages(room_id, created_at)` — fast room-scoped chronological queries
- `idx_messages_sender ON room_messages(sender_id)` — filter by sender

**`room_messages_fts`** — FTS5 virtual table for full-text search

Synchronized with `room_messages` via triggers (same pattern as `memories_fts`).

**Why UUIDv7 for room and message IDs:**
- Lexicographic ordering (no separate sequence needed for message chronology)
- Globally unique across rooms
- Matches existing `memories.id` pattern in codebase (`crates/nous-shared/src/ids.rs:40` uses `uuid::Uuid::now_v7()`)

**Why no `message_embeddings` in MVP:**
- Embedding every message is expensive (compute + storage)
- Most conversations are recalled by room + time range, not semantic similarity
- FTS5 handles keyword search adequately for MVP
- Can be added later as a separate vec0 table without schema changes

**Note on room_participants:** This table is schema-ready infrastructure for Phase 2. Participant tracking is deferred in MVP — participants are auto-tracked based on message senders.

## 3. Schema DDL

Complete, copy-pasteable SQL for the `MIGRATIONS` array at `crates/nous-core/src/db.rs:12-138` (currently 28 statements, will become 38).

**Note:** Timestamp defaults use `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` matching existing schema convention at `db.rs:19,26,34,56`.

```sql
-- rooms table
CREATE TABLE IF NOT EXISTS rooms (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    purpose TEXT,
    metadata TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)

-- room_participants table
CREATE TABLE IF NOT EXISTS room_participants (
    room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member' CHECK(role IN ('owner','member','observer')),
    joined_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (room_id, agent_id)
)

-- room_messages table
CREATE TABLE IF NOT EXISTS room_messages (
    id TEXT PRIMARY KEY,
    room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    sender_id TEXT NOT NULL,
    content TEXT NOT NULL,
    reply_to TEXT REFERENCES room_messages(id),
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)

-- FTS5 virtual table for message search
CREATE VIRTUAL TABLE IF NOT EXISTS room_messages_fts USING fts5(
    content,
    content='room_messages',
    content_rowid='rowid'
)

-- FTS sync trigger: insert
CREATE TRIGGER IF NOT EXISTS room_messages_ai AFTER INSERT ON room_messages
BEGIN
    INSERT INTO room_messages_fts(rowid, content)
    VALUES (new.rowid, new.content);
END

-- FTS sync trigger: update
CREATE TRIGGER IF NOT EXISTS room_messages_au AFTER UPDATE ON room_messages
BEGIN
    INSERT INTO room_messages_fts(room_messages_fts, rowid, content)
    VALUES ('delete', old.rowid, old.content);
    INSERT INTO room_messages_fts(rowid, content)
    VALUES (new.rowid, new.content);
END

-- FTS sync trigger: delete
CREATE TRIGGER IF NOT EXISTS room_messages_ad AFTER DELETE ON room_messages
BEGIN
    INSERT INTO room_messages_fts(room_messages_fts, rowid, content)
    VALUES ('delete', old.rowid, old.content);
END

-- Unique index: non-archived room names
CREATE UNIQUE INDEX IF NOT EXISTS idx_rooms_name ON rooms(name) WHERE archived = 0

-- Index: messages by room + created_at
CREATE INDEX IF NOT EXISTS idx_messages_room_created ON room_messages(room_id, created_at)

-- Index: messages by sender
CREATE INDEX IF NOT EXISTS idx_messages_sender ON room_messages(sender_id)
```

**Migration order:** Append these 10 statements to the existing `MIGRATIONS` array. They must come after the existing schema (models, workspaces, categories, memories, tags, relationships, memory_chunks, FTS triggers, indexes) to ensure dependency order.

## 4. WriteChannel Extensions

Add four new `WriteOp` enum variants to `crates/nous-core/src/channel.rs:18-51`:

```rust
pub enum WriteOp {
    // ... existing variants (Store, Update, Forget, Relate, Unrelate, Unarchive,
    //     CategorySuggest, StoreChunks, DeleteChunks, LogAccess) ...
    
    CreateRoom {
        id: String,              // UUIDv7
        name: String,
        purpose: Option<String>,
        metadata: Option<String>,  // JSON
        resp: oneshot::Sender<Result<String>>,
    },
    PostMessage {
        id: String,              // UUIDv7
        room_id: String,
        sender_id: String,
        content: String,
        reply_to: Option<String>,
        metadata: Option<String>,  // JSON
        resp: oneshot::Sender<Result<String>>,
    },
    DeleteRoom(String, bool, oneshot::Sender<Result<bool>>),  // (id, hard, resp)
    ArchiveRoom(String, oneshot::Sender<Result<bool>>),       // (id, resp)
}
```

**WriteChannel methods** (add to `crates/nous-core/src/channel.rs:59-203`):

```rust
impl WriteChannel {
    pub async fn create_room(
        &self,
        id: String,
        name: String,
        purpose: Option<String>,
        metadata: Option<String>,
    ) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::CreateRoom { id, name, purpose, metadata, resp: resp_tx })
            .await
            .map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await
            .map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn post_message(
        &self,
        id: String,
        room_id: String,
        sender_id: String,
        content: String,
        reply_to: Option<String>,
        metadata: Option<String>,
    ) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::PostMessage { id, room_id, sender_id, content, reply_to, metadata, resp: resp_tx })
            .await
            .map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await
            .map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn delete_room(&self, id: String, hard: bool) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::DeleteRoom(id, hard, resp_tx))
            .await
            .map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await
            .map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn archive_room(&self, id: String) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::ArchiveRoom(id, resp_tx))
            .await
            .map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await
            .map_err(|_| NousError::Internal("response channel dropped".into()))?
    }
}
```

**write_worker match arms** (add to `crates/nous-core/src/channel.rs:205-291`):

```rust
async fn write_worker(mut rx: mpsc::Receiver<WriteOp>, db: MemoryDb, batch_count: Arc<AtomicUsize>) {
    // ... existing batching logic ...
    
    // Inside the blocking task transaction:
    for op in batch {
        match op {
            // ... existing match arms (Store, Update, Forget, ...) ...
            
            WriteOp::CreateRoom { id, name, purpose, metadata, resp } => {
                let result = MemoryDb::create_room_on(&tx, &id, &name, purpose.as_deref(), metadata.as_deref());
                let _ = resp.send(result.map(|_| id.clone()));
            }
            WriteOp::PostMessage { id, room_id, sender_id, content, reply_to, metadata, resp } => {
                let result = MemoryDb::post_message_on(&tx, &id, &room_id, &sender_id, &content, 
                    reply_to.as_deref(), metadata.as_deref());
                let _ = resp.send(result.map(|_| id.clone()));
            }
            WriteOp::DeleteRoom(id, hard, resp) => {
                let result = if hard {
                    MemoryDb::hard_delete_room_on(&tx, &id)
                } else {
                    MemoryDb::archive_room_on(&tx, &id)
                };
                let _ = resp.send(result);
            }
            WriteOp::ArchiveRoom(id, resp) => {
                let result = MemoryDb::archive_room_on(&tx, &id);
                let _ = resp.send(result);
            }
        }
    }
}
```

**MemoryDb methods** (add to `crates/nous-core/src/db.rs`):

```rust
impl MemoryDb {
    pub(crate) fn create_room_on(
        conn: &Connection,
        id: &str,
        name: &str,
        purpose: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO rooms (id, name, purpose, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, purpose, metadata],
        )?;
        Ok(())
    }

    pub(crate) fn post_message_on(
        conn: &Connection,
        id: &str,
        room_id: &str,
        sender_id: &str,
        content: &str,
        reply_to: Option<&str>,
        metadata: Option<&str>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO room_messages (id, room_id, sender_id, content, reply_to, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, room_id, sender_id, content, reply_to, metadata],
        )?;
        Ok(())
    }

    pub(crate) fn archive_room_on(conn: &Connection, id: &str) -> Result<bool> {
        let rows = conn.execute("UPDATE rooms SET archived = 1 WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    pub(crate) fn hard_delete_room_on(conn: &Connection, id: &str) -> Result<bool> {
        let rows = conn.execute("DELETE FROM rooms WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }
}
```

**Batching behavior:** Room operations batch alongside memory operations using the same `BATCH_LIMIT=32` and `CHANNEL_CAPACITY=256` from `channel.rs:15-16`. A batch may contain a mix of `Store`, `PostMessage`, `CreateRoom`, etc. All commit atomically in a single transaction.

## 5. ReadPool Extensions

Add query methods to `crates/nous-core/src/channel.rs:383+` or `crates/nous-core/src/db.rs` (pattern: similar to `search_memories`, `get_workspaces`).

**ID generation note:** Room and message IDs reuse the existing `MemoryId::new()` generator (UUIDv7 via `uuid::Uuid::now_v7()` at `ids.rs:40`) for consistency. No separate `RoomId` or `MessageId` type is introduced.

**ReadPool methods:**

```rust
impl ReadPool {
    pub async fn list_rooms(&self, archived: bool, limit: Option<usize>) -> Result<Vec<Room>> {
        let limit = limit.unwrap_or(100);
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, purpose, metadata, archived, created_at, updated_at
                 FROM rooms
                 WHERE archived = ?1
                 ORDER BY created_at DESC
                 LIMIT ?2"
            )?;
            let rows = stmt.query_map(params![archived as i64, limit], |row| {
                Ok(Room {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    purpose: row.get(2)?,
                    metadata: row.get(3)?,
                    archived: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?;
            rows.collect()
        }).await
    }

    pub async fn get_room(&self, id: &str) -> Result<Option<Room>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            conn.query_row(
                "SELECT id, name, purpose, metadata, archived, created_at, updated_at
                 FROM rooms WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        purpose: row.get(2)?,
                        metadata: row.get(3)?,
                        archived: row.get::<_, i64>(4)? != 0,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            ).optional()
        }).await
    }

    pub async fn list_messages(
        &self,
        room_id: &str,
        limit: Option<usize>,
        before: Option<String>,  // UUIDv7 or ISO timestamp
        since: Option<String>,
    ) -> Result<Vec<Message>> {
        let room_id = room_id.to_string();
        let limit = limit.unwrap_or(100);
        self.with_conn(move |conn| {
            let mut sql = "SELECT id, room_id, sender_id, content, reply_to, metadata, created_at
                           FROM room_messages
                           WHERE room_id = ?1".to_string();
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(room_id)];
            
            if let Some(b) = before {
                sql.push_str(" AND created_at < ?");
                params.push(Box::new(b));
            }
            if let Some(s) = since {
                sql.push_str(" AND created_at > ?");
                params.push(Box::new(s));
            }
            
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params.push(Box::new(limit));
            
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| {
                    Ok(Message {
                        id: row.get(0)?,
                        room_id: row.get(1)?,
                        sender_id: row.get(2)?,
                        content: row.get(3)?,
                        reply_to: row.get(4)?,
                        metadata: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                }
            )?;
            rows.collect()
        }).await
    }

    pub async fn search_messages(
        &self,
        room_id: &str,
        query: &str,
        limit: Option<usize>,
    ) -> Result<Vec<Message>> {
        let room_id = room_id.to_string();
        let query = query.to_string();
        let limit = limit.unwrap_or(50);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.room_id, m.sender_id, m.content, m.reply_to, m.metadata, m.created_at
                 FROM room_messages m
                 JOIN room_messages_fts fts ON m.rowid = fts.rowid
                 WHERE fts MATCH ?1 AND m.room_id = ?2
                 ORDER BY m.created_at DESC
                 LIMIT ?3"
            )?;
            let rows = stmt.query_map(params![query, room_id, limit], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    room_id: row.get(1)?,
                    sender_id: row.get(2)?,
                    content: row.get(3)?,
                    reply_to: row.get(4)?,
                    metadata: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;
            rows.collect()
        }).await
    }
}
```

**New types** (add to `crates/nous-core/src/types.rs`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub purpose: Option<String>,
    pub metadata: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub room_id: String,
    pub sender_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub metadata: Option<String>,
    pub created_at: String,
}
```

## 6. MCP Tool Specifications

Seven new MCP tools following the registration pattern at `crates/nous-cli/src/server.rs:63-199`.


### `room_create`

Create a new room.

Seven new tools extend the existing 21 tools registered in `crates/nous-cli/src/server.rs:87-293`.

**Params struct** (`crates/nous-cli/src/tools.rs`):
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomCreateParams {
    pub name: String,
    pub purpose: Option<String>,
    pub metadata: Option<String>,  // JSON string
}
```

**Handler signature:**
```rust
pub async fn handle_room_create(
    params: RoomCreateParams,
    write_channel: &WriteChannel,
) -> CallToolResult
```

**Return shape (JSON):**
```json
{
  "id": "018f3a5e-...",
  "name": "pr-123-review",
  "created_at": "2026-04-27T04:30:00.123Z"
}
```

**Registration** (`server.rs`):
```rust
#[tool(name = "room_create", description = "Create a new conversation room")]
async fn room_create(&self, params: Parameters<RoomCreateParams>) -> CallToolResult {
    handle_room_create(params.0, &self.write_channel).await
}
```

---

### `room_list`

List rooms (active or archived).

**Params struct:**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomListParams {
    #[serde(default)]
    pub archived: bool,
    pub limit: Option<usize>,
}
```

**Handler signature:**
```rust
pub async fn handle_room_list(
    params: RoomListParams,
    read_pool: &ReadPool,
) -> CallToolResult
```

**Return shape:**
```json
{
  "rooms": [
    {
      "id": "018f3a5e-...",
      "name": "pr-123-review",
      "purpose": "Code review coordination",
      "archived": false,
      "created_at": "2026-04-27T04:30:00.123Z",
      "updated_at": "2026-04-27T04:30:00.123Z"
    }
  ]
}
```

---

### `room_get`

Get a single room by ID or name.

**Params struct:**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomGetParams {
    pub id: String,  // UUIDv7 or room name
}
```

**Handler signature:**
```rust
pub async fn handle_room_get(
    params: RoomGetParams,
    read_pool: &ReadPool,
) -> CallToolResult
```

**Return shape:**
```json
{
  "id": "018f3a5e-...",
  "name": "pr-123-review",
  "purpose": "Code review coordination",
  "metadata": "{\"labels\":[\"urgent\"]}",
  "archived": false,
  "created_at": "2026-04-27T04:30:00.123Z",
  "updated_at": "2026-04-27T04:30:00.123Z"
}
```

---

### `room_delete`

Delete or archive a room.

**Params struct:**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomDeleteParams {
    pub id: String,
    #[serde(default)]
    pub hard: bool,  // true = DELETE, false = SET archived=1
}
```

**Handler signature:**
```rust
pub async fn handle_room_delete(
    params: RoomDeleteParams,
    write_channel: &WriteChannel,
) -> CallToolResult
```

**Return shape:**
```json
{
  "success": true,
  "deleted": true  // or "archived": true if hard=false
}
```

---

### `room_post_message`

Post a message to a room.

**Params struct:**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomPostMessageParams {
    pub room_id: String,
    pub content: String,
    pub sender_id: Option<String>,  // defaults to "system" if not provided
    pub reply_to: Option<String>,   // parent message ID
    pub metadata: Option<String>,   // JSON string
}
```

**Handler signature:**
```rust
pub async fn handle_room_post_message(
    params: RoomPostMessageParams,
    write_channel: &WriteChannel,
) -> CallToolResult
```

**Return shape:**
```json
{
  "id": "018f3a5f-...",
  "room_id": "018f3a5e-...",
  "sender_id": "agent-abc",
  "created_at": "2026-04-27T04:31:00.456Z"
}
```

---

### `room_read_messages`

Read messages from a room (chronological, with pagination).

**Params struct:**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomReadMessagesParams {
    pub room_id: String,
    pub limit: Option<usize>,
    pub before: Option<String>,  // UUIDv7 or ISO timestamp
    pub since: Option<String>,   // UUIDv7 or ISO timestamp
}
```

**Handler signature:**
```rust
pub async fn handle_room_read_messages(
    params: RoomReadMessagesParams,
    read_pool: &ReadPool,
) -> CallToolResult
```

**Return shape:**
```json
{
  "messages": [
    {
      "id": "018f3a5f-...",
      "room_id": "018f3a5e-...",
      "sender_id": "agent-abc",
      "content": "PR looks good, merging now",
      "reply_to": null,
      "metadata": null,
      "created_at": "2026-04-27T04:31:00.456Z"
    }
  ]
}
```

---

### `room_search`

Search messages within a room using FTS5.

**Params struct:**
```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RoomSearchParams {
    pub room_id: String,
    pub query: String,  // FTS5 match expression
    pub limit: Option<usize>,
}
```

**Handler signature:**
```rust
pub async fn handle_room_search(
    params: RoomSearchParams,
    read_pool: &ReadPool,
) -> CallToolResult
```

**Return shape:**
```json
{
  "messages": [
    {
      "id": "018f3a5f-...",
      "room_id": "018f3a5e-...",
      "sender_id": "agent-abc",
      "content": "The linter failed on line 42",
      "created_at": "2026-04-27T04:28:00.123Z"
    }
  ]
}
```

## 7. CLI Command Specifications

Add `room` subcommand to `crates/nous-cli/src/main.rs` following the `category` pattern at `main.rs:48-82`.

**Command enum variant** (add to `Command` at `main.rs:26`):
```rust
Room(RoomCmd),
```

**Subcommand structs:**
```rust
#[derive(Debug, Parser)]
struct RoomCmd {
    #[command(subcommand)]
    command: RoomSubcommand,
}

#[derive(Debug, Subcommand)]
enum RoomSubcommand {
    List {
        #[arg(long)]
        archived: bool,
        #[arg(long)]
        limit: Option<usize>,
    },
    Create {
        name: String,
        #[arg(long)]
        purpose: Option<String>,
    },
    Get {
        id: String,
    },
    Delete {
        id: String,
        #[arg(long)]
        hard: bool,
    },
    Post {
        room_id: String,
        content: String,
        #[arg(long)]
        sender: Option<String>,
        #[arg(long)]
        reply_to: Option<String>,
    },
    Read {
        room_id: String,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        before: Option<String>,
    },
    Search {
        room_id: String,
        query: String,
        #[arg(long)]
        limit: Option<usize>,
    },
}
```

**Match arm** (add to `main()` at `main.rs:108`):
```rust
Command::Room(cmd) => match cmd.command {
    RoomSubcommand::List { archived, limit } => {
        commands::run_room_list(&config, archived, limit)?
    }
    RoomSubcommand::Create { name, purpose } => {
        commands::run_room_create(&config, &name, purpose.as_deref())?
    }
    RoomSubcommand::Get { id } => {
        commands::run_room_get(&config, &id)?
    }
    RoomSubcommand::Delete { id, hard } => {
        commands::run_room_delete(&config, &id, hard)?
    }
    RoomSubcommand::Post { room_id, content, sender, reply_to } => {
        commands::run_room_post(&config, &room_id, &content, sender.as_deref(), reply_to.as_deref())?
    }
    RoomSubcommand::Read { room_id, limit, since, before } => {
        commands::run_room_read(&config, &room_id, limit, since.as_deref(), before.as_deref())?
    }
    RoomSubcommand::Search { room_id, query, limit } => {
        commands::run_room_search(&config, &room_id, &query, limit)?
    }
}
```

**Handler implementations** (add to `crates/nous-cli/src/commands.rs`):

```rust
pub fn run_room_list(config: &Config, archived: bool, limit: Option<usize>) -> Result<(), Box<dyn std::error::Error>> {
    let db = MemoryDb::open(&config.memory.db_path, None, 384)?;
    let read_pool = ReadPool::new(&config.memory.db_path, None, 1)?;
    let rt = tokio::runtime::Runtime::new()?;
    let rooms = rt.block_on(read_pool.list_rooms(archived, limit))?;
    for room in rooms {
        println!("{} | {} | {}", room.id, room.name, room.purpose.as_deref().unwrap_or(""));
    }
    Ok(())
}

pub fn run_room_create(config: &Config, name: &str, purpose: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let db = MemoryDb::open(&config.memory.db_path, None, 384)?;
    let (write_channel, _handle) = WriteChannel::new(db);
    let rt = tokio::runtime::Runtime::new()?;
    let id = nous_shared::ids::MemoryId::new().to_string();  // Generate UUIDv7
    rt.block_on(write_channel.create_room(id.clone(), name.to_string(), purpose.map(String::from), None))?;
    println!("Created room: {} ({})", name, id);
    Ok(())
}

// ... similar handlers for run_room_get, run_room_delete, run_room_post, run_room_read, run_room_search
```

**Usage examples:**

```bash
# List active rooms
nous-cli room list

# List archived rooms
nous-cli room list --archived

# Create a room
nous-cli room create pr-123-review --purpose "Code review coordination"

# Get room details
nous-cli room get 018f3a5e-...

# Post a message
nous-cli room post 018f3a5e-... "Starting review" --sender agent-abc

# Read recent messages
nous-cli room read 018f3a5e-... --limit 50

# Search messages
nous-cli room search 018f3a5e-... "linter failed"

# Delete room (soft)
nous-cli room delete 018f3a5e-...

# Delete room (hard)
nous-cli room delete 018f3a5e-... --hard
```

## 8. Config Additions

Add `[rooms]` section to `crates/nous-cli/src/config.rs` following existing patterns.

**Config struct** (add field at `config.rs:22`):
```rust
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub memory: MemoryConfig,
    pub embedding: EmbeddingConfig,
    pub otlp: OtlpConfig,
    pub classification: ClassificationConfig,
    pub encryption: EncryptionConfig,
    pub rooms: RoomsConfig,  // <-- NEW
}
```

**RoomsConfig struct** (add after `EncryptionConfig` at `config.rs:58+`):
```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RoomsConfig {
    pub max_rooms: usize,
    pub max_messages_per_room: usize,
}

impl Default for RoomsConfig {
    fn default() -> Self {
        Self {
            max_rooms: 1000,
            max_messages_per_room: 10000,
        }
    }
}
```

**DEFAULT_CONFIG_TOML** (update at `config.rs:108-126`):
```toml
[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "BAAI/bge-small-en-v1.5"
variant = "onnx/model.onnx"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "~/.config/nous/db.key"

[rooms]
max_rooms = 1000
max_messages_per_room = 10000
```

**Environment variable overrides** (optional, add to `apply_env_overrides` at `config.rs:160-176`):
```rust
if let Ok(val) = std::env::var("NOUS_ROOMS_MAX") {
    if let Ok(n) = val.parse::<usize>() {
        cfg.rooms.max_rooms = n;
    }
}

if let Ok(val) = std::env::var("NOUS_ROOMS_MAX_MESSAGES") {
    if let Ok(n) = val.parse::<usize>() {
        cfg.rooms.max_messages_per_room = n;
    }
}
```

**Note:** These config values are for future validation/enforcement. MVP does not enforce limits, but the config structure is ready for it.

## 9. Migration Strategy

The `MIGRATIONS` array at `crates/nous-core/src/db.rs:12-138` currently contains 28 SQL statements. Append the 10 new statements from Section 3 to this array.

**Current migration count:** 28  
**New migration count:** 38

**Insertion point:** After line 138 (after the last existing index creation).

**Order matters:** The new statements must come after the existing schema because:
1. FTS5 virtual tables reference `room_messages` via `content='room_messages'`
2. Triggers reference `room_messages` and `room_messages_fts`
3. Foreign key constraints reference `rooms(id)`

**Migration execution:** The `run_migrations()` function at `crates/nous-shared/src/sqlite.rs` executes all statements in a single transaction. If any statement fails, the entire transaction rolls back. On successful schema creation, a `schema_version` pragma records the last applied migration.

**Backward compatibility:** Old Nous binaries (without room schema) will continue to work with databases that already have the room tables. The `CREATE TABLE IF NOT EXISTS` guards prevent errors.

**Forward compatibility:** New Nous binaries will detect missing room tables on first open and create them via migrations.

**No data migration needed:** This is a green-field addition. No existing data needs transformation.

**Testing:** Unit tests should verify:
1. Fresh database opens successfully with all 38 migrations applied
2. Existing database (28 migrations) upgrades to 38 migrations without errors
3. All indexes and triggers are created
4. FTS5 sync triggers fire correctly on INSERT/UPDATE/DELETE

**Rollback strategy:** If a bug is discovered post-deployment, the room tables can be dropped manually via SQL without affecting memory operations. The two schemas are independent (no foreign keys between memories and rooms).

## 10. Future Work

**Semantic message search (Phase 2):**
- Add `message_embeddings` vec0 table (analogous to `memory_embeddings`)
- On message insert: call `embedding.embed_one(content)` and store in vec0
- New MCP tool: `room_search_semantic(room_id, query, k)` for KNN queries
- Decision needed: embed all messages (expensive) vs. on-demand / selective embedding

**Message editing:**
- Add `edited_at` column to `room_messages`
- New WriteOp: `EditMessage { id, new_content, resp }`
- MCP tool: `room_edit_message(message_id, content)`
- Maintain edit history in `metadata` JSON field or separate `message_edits` table

**Retention policies:**
- Config: `rooms.message_ttl_days`
- Background task: periodically DELETE messages older than TTL
- Per-room override: `rooms.metadata` JSON field with `{"ttl_days": 30}`

**Workspace-scoped rooms:**
- Add `workspace_id` FK to `rooms` table
- Update unique constraint: `UNIQUE(name, workspace_id) WHERE archived = 0`
- Allows different workspaces to have rooms with the same name

**Participant permissions:**
- Enforce `role` field: observers cannot post, only owners can delete
- Add `room_permissions` table for fine-grained ACLs
- MCP tools: `room_add_participant`, `room_remove_participant`, `room_set_role`

**Message reactions:**
- Add `message_reactions` table: `(message_id, agent_id, emoji, created_at)`
- MCP tools: `room_add_reaction`, `room_list_reactions`

**Message attachments:**
- Store attachment metadata in `metadata` JSON field
- Reference blobs in separate `attachments` table or external storage (S3, local files)

**Cross-entity queries:**
- JOIN rooms with memories: "find memories related to this conversation"
- Add `room_id` column to `memories` table (nullable FK)
- MCP tool: `memory_link_room(memory_id, room_id)`

**Notification system:**
- Track unread messages per participant
- Add `room_read_cursors` table: `(room_id, agent_id, last_read_message_id)`
- MCP tool: `room_mark_read(room_id, message_id)`

**Export/import:**
- CLI: `nous-cli room export <room_id> --format json`
- CLI: `nous-cli room import <file>`
- Include messages, participants, metadata in JSON export

**Room templates:**
- Predefined room structures (e.g., incident response, code review)
- Seed new rooms with pinned messages, default participants, metadata schema
