# 04: P0/P1 Features — Chat & Tasks

**Initiative:** INI-076 / INI-089  
**Status:** Draft  
**Author:** Technical Writer (paseo agent)  
**Date:** 2026-04-29

---

## 1. Goals

**P0 — Chat & Notifications:**

- Persistent, SQLite-backed chat rooms shared across agents in the same Nous instance
- FTS5 full-text search over message history (see `docs/design-rooms.md` for schema)
- Subscriber-based notification delivery so agents receive `@mention` events without polling
- Topic labels on messages, enabling agents to filter room traffic by subject
- Defined room lifecycle: `pending → active → archived → deleted`

**P1 — Tasks / Issues:**

- Issue tracking native to Nous, backed by the same SQLite database as memory and rooms
- UUIDv7-identified tasks with status (`open → in_progress → done → closed`), priority, assignee, and labels
- Task-to-task linking with typed edges (`blocked_by`, `parent`, `related_to`)
- Each task carries an optional `room_id` reference to its discussion thread
- Tasks assignable to agent IDs so the scheduler can route work automatically
- MCP tools and CLI commands consistent with the patterns in `docs/design/03-api-interfaces.md`

---

## 2. Non-Goals

- Message editing (MVP is append-only; `edited_at` is a Phase 2 concern in `docs/design-rooms.md §10`)
- Semantic search over messages (FTS5 is sufficient for MVP; vec0 extension deferred)
- Per-workspace room scoping (rooms are globally unique by name in MVP)
- Retention policies or auto-archival of old messages
- Real-time push via WebSocket or SSE (agents poll or use `chat wait`; server-push is Phase 2)
- Gantt charts, sprints, or velocity tracking (this is lightweight issue tracking, not a project-management suite)
- Task comments as a separate entity (comments go in the linked room; no `task_comments` table)
- GitHub/GitLab issue sync (integration is a future plugin concern)

---

## 3. Architecture

Both features live in the same SQLite database (`~/.cache/nous/memory.db`) that stores memories. All writes flow through a single `WriteChannel` (a bounded `mpsc` channel with a serial `write_worker`); all reads use a `ReadPool` (multiple read-only connections). This is the data layer defined in `docs/design/02-data-layer.md`.

MCP tools are registered in `crates/nous-cli/src/server.rs` via `rmcp`. CLI subcommands are defined with `clap` in `crates/nous-cli/src/main.rs`. Both delegate to shared handler functions. IDs are UUIDv7 everywhere (see `docs/design/01-system-architecture.md`).

```
  ┌─────────────────────────────────────────────────────────────────┐
  │                        Nous process                             │
  │                                                                 │
  │   MCP Client (agent)           CLI (user / script)             │
  │        │                              │                         │
  │        ▼                              ▼                         │
  │   ┌─────────┐                  ┌───────────┐                   │
  │   │  rmcp   │                  │   clap    │                   │
  │   │ server  │                  │  parser   │                   │
  │   └────┬────┘                  └─────┬─────┘                   │
  │        │                             │                          │
  │        └─────────────┬───────────────┘                         │
  │                      ▼                                          │
  │              handler functions                                  │
  │              (tools.rs / commands.rs)                           │
  │                      │                                          │
  │           ┌──────────┴──────────┐                              │
  │           ▼                     ▼                               │
  │     WriteChannel            ReadPool                            │
  │     (serial writes)         (parallel reads)                    │
  │           │                     │                               │
  │           └──────────┬──────────┘                              │
  │                      ▼                                          │
  │               SQLite (WAL mode)                                 │
  │     ┌──────────┬──────────────┬────────────────┐               │
  │     │ memories │    rooms     │     tasks       │               │
  │     │  (P2)    │  (P0 chat)   │   (P1 issues)   │               │
  │     └──────────┴──────────────┴────────────────┘               │
  └─────────────────────────────────────────────────────────────────┘
```

### 3.1 Chat & Rooms (P0)

Four tables (`rooms`, `room_participants`, `room_messages`, `room_messages_fts`) extend the existing schema. A `room_subscriptions` table (new in this document) records which agents subscribe to which rooms for notification delivery.

```
  rooms
  ┌────────────────────────────────────────────────────┐
  │ id (UUIDv7)  name  purpose  metadata  archived     │
  └──────────────────────┬─────────────────────────────┘
                         │ 1:N
  room_messages          ▼
  ┌──────────────────────────────────────────────────────────┐
  │ id (UUIDv7)  room_id  sender_id  content                 │
  │ reply_to  metadata (mentions[], topic)  created_at       │
  └──────────────────────┬───────────────────────────────────┘
                         │ triggers
                         ▼
  room_messages_fts (FTS5 virtual table — BM25 ranking)

  room_subscriptions
  ┌───────────────────────────────────────────────────────────┐
  │ room_id  agent_id  topics[]  created_at                   │
  │ (FK rooms.id)                                             │
  └───────────────────────────────────────────────────────────┘
```

Notification flow: when `post_message` writes a row, the `write_worker` calls `notify_subscribers(room_id, message_id)` before returning. `notify_subscribers` reads `room_subscriptions` and enqueues a `Notification` struct into each subscriber's in-memory `broadcast::Sender`. Agents waiting via `room_wait` hold the corresponding `Receiver`.

```
  write_worker (serial)
        │
        ├─ INSERT INTO room_messages
        │
        └─ notify_subscribers(room_id, msg_id)
               │
               ▼
        room_subscriptions (ReadPool lookup)
               │
        for each subscriber:
               ▼
        broadcast::Sender<Notification>
               │
        ┌──────┴──────────────────┐
        ▼                         ▼
  agent A (waiting)         agent B (waiting)
  room_wait MCP call        room_wait MCP call
  unblocks immediately      unblocks immediately
```

### 3.2 Tasks (P1)

Tasks live in two tables: `tasks` (core row) and `task_links` (typed edges). A task optionally points at a room (for discussion) and an agent (for assignment).

```
  tasks
  ┌─────────────────────────────────────────────────────────────┐
  │ id (UUIDv7)  title  description  status  priority           │
  │ assignee_id  labels (JSON)  room_id (FK)  created_at        │
  │ updated_at  closed_at                                        │
  └────────────────────────┬────────────────────────────────────┘
                            │ 1:N (both directions)
  task_links                ▼
  ┌─────────────────────────────────────────────────────────────┐
  │ id (UUIDv7)  source_id (FK tasks)  target_id (FK tasks)     │
  │ link_type  ('blocked_by'|'parent'|'related_to')             │
  │ created_at                                                   │
  └─────────────────────────────────────────────────────────────┘

  Status transitions:

  open ──► in_progress ──► done ──► closed
   │                               ▲
   └──────── (skip done) ──────────┘   (closed_at set on → closed)
```

The `room_id` FK is nullable — tasks without a discussion room are valid. When set, MCP tools can navigate from a task to its room and vice versa.

---

## 4. P0: Chat & Notifications

### 4.1 Rooms Design Summary

The full rooms schema, `WriteChannel` extensions, `ReadPool` query methods, MCP tool specifications, and CLI command definitions are in **`docs/design-rooms.md`**. This section summarises the key decisions; refer to that document for DDL, handler signatures, and return shapes.

**Tables (4):**

| Table | Purpose |
|-------|---------|
| `rooms` | Room metadata: `id` (UUIDv7), `name`, `purpose`, `metadata` (JSON), `archived` |
| `room_participants` | `(room_id, agent_id)` with `role` (`owner`/`member`/`observer`); schema-ready, enforcement deferred to Phase 2 |
| `room_messages` | Append-only messages: `id` (UUIDv7), `room_id`, `sender_id`, `content`, `reply_to`, `metadata` (JSON), `created_at` |
| `room_messages_fts` | FTS5 virtual table, content-tracked against `room_messages`, synced via three triggers |

**Uniqueness:** Active room names are unique via `CREATE UNIQUE INDEX ... WHERE archived = 0`. Archived rooms with a recycled name are allowed.

**Ordering:** UUIDv7 primary keys are lexicographically monotone; chronological message order requires no separate sequence column.

**Seven MCP tools:** `room_create`, `room_list`, `room_get`, `room_delete`, `room_post_message`, `room_read_messages`, `room_search`.

**Seven CLI subcommands:** `nous-cli room {list,create,get,delete,post,read,search}`.

**BM25 ranking** (`room_search`) uses FTS5's built-in `bm25()` scoring function — the same approach used for memory search (commit `3c153ec`).

### 4.2 Notification System

#### Subscriber model

An agent subscribes to a room by calling `room_subscribe`. The subscription is persisted in `room_subscriptions` and also registers an in-memory `broadcast::Receiver<Notification>`.

**`room_subscriptions` table:**

```sql
CREATE TABLE IF NOT EXISTS room_subscriptions (
    room_id    TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    agent_id   TEXT NOT NULL,
    topics     TEXT,         -- JSON array of topic strings; NULL = all topics
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (room_id, agent_id)
)
```

The in-memory `broadcast::Sender<Notification>` per room lives in a `DashMap<RoomId, broadcast::Sender<Notification>>` held by the `NotificationRegistry` struct in `crates/nous-core`. Channel capacity is 256 per room (`broadcast::channel(256)`). On overflow (lagged receiver), the oldest notification is dropped and the receiver receives a `RecvError::Lagged` — the agent must re-read messages from the last known cursor.

#### @mention routing

`@<agent_id>` tokens in message content trigger targeted notifications. The `post_message` handler parses `content` for `@`-prefixed agent IDs before writing the row. Any mentioned agent that is not subscribed to the room receives a one-shot notification regardless (the message is still the source of truth — the notification is advisory).

Parsing rule: `@` followed by a UUID (with or without hyphens) or a registered agent name. Names are resolved against the `org-management` agent directory at dispatch time; unresolvable names produce a warning in the `metadata` JSON field of the message row (`{"unresolved_mentions": ["@nobody"]}`).

#### Delivery guarantees

| Scenario | Behaviour |
|----------|-----------|
| Subscriber is waiting (`room_wait`) | `broadcast::Receiver` unblocks within the same `write_worker` transaction commit |
| Subscriber is not connected | Notification is lost; agent reads missed messages by polling `room_read_messages` with `since=<last_cursor>` on reconnect |
| Broadcast channel full (256 messages) | Oldest notification dropped; agent receives `Lagged` error, must re-read from last cursor |
| Room archived mid-subscribe | Subscribers receive a synthetic `{event: "room_archived"}` notification before the subscription is dropped |

#### `room_wait` MCP tool

Blocks the calling MCP request until a new message arrives or `timeout_ms` elapses.

```json
{
  "name": "room_wait",
  "params": {
    "room_id": "string (required)",
    "timeout_ms": "integer (optional, default 30000, max 120000)"
  },
  "returns": {
    "message": { "...": "Message object or null on timeout" },
    "timed_out": "boolean"
  }
}
```

Implementation: acquires a `broadcast::Receiver` from `NotificationRegistry`, then calls `tokio::time::timeout(timeout_ms, receiver.recv())`. On timeout returns `{"timed_out": true, "message": null}`.

#### `room_subscribe` / `room_unsubscribe` MCP tools

```json
{
  "name": "room_subscribe",
  "params": {
    "room_id": "string",
    "agent_id": "string",
    "topics": ["optional", "array", "of", "topic", "strings"]
  }
}
```

```json
{
  "name": "room_unsubscribe",
  "params": {
    "room_id": "string",
    "agent_id": "string"
  }
}
```

Both operations are synchronous writes through `WriteChannel` (`WriteOp::Subscribe` / `WriteOp::Unsubscribe`).

### 4.3 Topic / Threading Model

#### Reply threads

The `reply_to` column in `room_messages` stores the parent message ID. This is already in the schema (`docs/design-rooms.md §2`). A reply chain is a simple linked list — there is no nesting depth limit, but UI clients should flatten at depth 1 (direct replies only) for readability.

Reading a thread: `room_read_messages` with `reply_to=<parent_id>` returns all direct replies in chronological order. There is no recursive CTE in MVP; deeper nesting requires repeated calls.

#### Topic labels

Topics are string tags stored in two places:

1. **Per-message** — the `metadata` JSON field contains a `"topics"` key: `{"topics": ["deploy", "incident"], "mentions": ["agent-abc"]}`. This is set by the `post_message` caller.
2. **Per-subscription** — `room_subscriptions.topics` is a JSON array. When set, the notification registry only delivers notifications for messages whose `metadata.topics` intersects the subscription topics. A NULL topics list means all messages.

Topic strings are free-form lowercase identifiers. Suggested conventions:

| Topic | Usage |
|-------|-------|
| `deploy` | Deployment events |
| `incident` | Incident coordination |
| `review` | Code review discussion |
| `task:<id>` | Messages scoped to a specific task (links room ↔ task) |
| `decision` | Architecture or process decisions |

The `task:<id>` convention lets agents listen only for messages relevant to a given task without reading the whole room.

### 4.4 Room Lifecycle

```
  ┌─────────┐   room_create    ┌────────┐
  │ pending │ ───────────────► │ active │
  └─────────┘                  └───┬────┘
       (not used in MVP —          │  room_delete (hard=false)
        room_create goes           │  or room_archive
        directly to active)        ▼
                              ┌──────────┐   room_delete (hard=true)
                              │ archived │ ──────────────────────────► deleted
                              └──────────┘                             (row gone)
```

**State transitions:**

| Transition | Trigger | Effect |
|------------|---------|--------|
| `→ active` | `room_create` | INSERT into `rooms` with `archived=0` |
| `active → archived` | `room_delete(hard=false)` or `room_archive` | `UPDATE rooms SET archived=1`; unique index allows name reuse; subscribers receive `room_archived` notification |
| `archived → deleted` | `room_delete(hard=true)` | `DELETE FROM rooms`; CASCADE deletes `room_messages`, `room_participants`, `room_subscriptions`; FTS triggers clean up `room_messages_fts` |
| `active → deleted` | `room_delete(hard=true)` on active room | Same as above; no intermediate archived state required |

**Name reuse after archive:** Once a room is archived, a new active room with the same name can be created. The partial unique index (`WHERE archived = 0`) permits this. Clients that store room IDs (UUIDv7) rather than names are unaffected; name-based lookups will find the new active room.

**Participant cleanup:** When a room is archived, existing subscriptions remain in `room_subscriptions` but notification delivery stops (the `write_worker` skips delivery for archived rooms). When a room is hard-deleted, subscriptions CASCADE delete.

### 4.5 Integration with Tasks and Memory

#### Rooms ↔ Tasks

A task row carries an optional `room_id` (FK to `rooms.id`). When set:
- `task_get` includes a `room_url` field in the response: the MCP tool path the caller can use to read the discussion (`room_read_messages(room_id=...)`).
- `task_create` accepts an optional `room_id`; alternatively it accepts `create_room=true`, which auto-creates a room named `task-<task_id>` and stores the resulting ID in `tasks.room_id`.
- Messages in the linked room tagged with topic `task:<task_id>` are surfaced by `task_get` as `recent_discussion` (last 5 messages) to give agents context without a separate MCP call.

#### Rooms ↔ Memory

Memories (see `docs/design/02-data-layer.md`) can reference a room via a relationship edge (`memory_relate(memory_id, room_id, "discussed_in")`). This is a soft link — no FK constraint — to avoid coupling the memory schema to the rooms schema. When `memory_search` returns a result with a `discussed_in` relationship, agents can follow the link to read the originating conversation.

The inverse direction (`room_search` finding memories) is not implemented in MVP. Future: a `room_to_memory` tool that embeds a room's message history into a summary memory.

---

## 5. P1: Tasks / Issues

### 5.1 Data Model

#### `tasks` table

```sql
CREATE TABLE IF NOT EXISTS tasks (
    id          TEXT PRIMARY KEY,  -- UUIDv7
    title       TEXT NOT NULL,
    description TEXT,              -- Markdown body; nullable
    status      TEXT NOT NULL DEFAULT 'open'
                    CHECK(status IN ('open','in_progress','done','closed')),
    priority    TEXT NOT NULL DEFAULT 'medium'
                    CHECK(priority IN ('critical','high','medium','low')),
    assignee_id TEXT,              -- Nous agent ID or NULL
    labels      TEXT,              -- JSON array, e.g. '["bug","backend"]'
    room_id     TEXT REFERENCES rooms(id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    closed_at   TEXT               -- set when status → 'closed'
)
```

```sql
CREATE INDEX IF NOT EXISTS idx_tasks_status    ON tasks(status)
CREATE INDEX IF NOT EXISTS idx_tasks_assignee  ON tasks(assignee_id)
CREATE INDEX IF NOT EXISTS idx_tasks_room      ON tasks(room_id)
CREATE INDEX IF NOT EXISTS idx_tasks_created   ON tasks(created_at)
```

#### `task_links` table

```sql
CREATE TABLE IF NOT EXISTS task_links (
    id          TEXT PRIMARY KEY,  -- UUIDv7
    source_id   TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    target_id   TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    link_type   TEXT NOT NULL
                    CHECK(link_type IN ('blocked_by','parent','related_to')),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(source_id, target_id, link_type)
)
```

```sql
CREATE INDEX IF NOT EXISTS idx_task_links_source ON task_links(source_id)
CREATE INDEX IF NOT EXISTS idx_task_links_target ON task_links(target_id)
```

**`labels` encoding:** A JSON array stored as TEXT. The query layer uses `json_each(labels)` for set-membership filtering: `WHERE EXISTS (SELECT 1 FROM json_each(labels) WHERE value = ?1)`. This avoids a separate `task_labels` junction table at the cost of no foreign key enforcement on label names.

**`assignee_id`:** Stores the Paseo agent ID (`<org_id>-<team_id>-<role>-<suffix>` format). No foreign key to an agents table — agent identity is managed by `org-management`, not Nous. The field is `NULL` for unassigned tasks.

### 5.2 Task Lifecycle State Machine

```
         ┌────────────────────────────────────────────────────┐
         │                                                    │
  ──────►│  open  ──────────────────────────────────────────►│ closed
         │   │                                    ▲           │
         │   │ task_update(status=in_progress)    │           │
         │   ▼                                    │           │
         │ in_progress ──────────────────────────►│           │
         │   │                                    │           │
         │   │ task_update(status=done)            │           │
         │   ▼                                    │           │
         │  done ─────────────────────────────────┘           │
         └────────────────────────────────────────────────────┘
```

**Allowed transitions:**

| From | To | Notes |
|------|----|-------|
| `open` | `in_progress` | Assignment usually happens here |
| `open` | `closed` | Rejected / won't-fix without starting work |
| `in_progress` | `done` | Work complete, pending verification |
| `in_progress` | `open` | Unassigned / blocked, kicked back |
| `in_progress` | `closed` | Abandoned |
| `done` | `closed` | Verified and accepted |
| `done` | `in_progress` | Verification failed, re-opened |
| `closed` | (none) | Terminal state; re-open by creating a linked follow-up task |

**`closed_at`:** Set by `task_update` when `status` transitions to `closed`. Never cleared. If a task moves `closed → in_progress` (should not happen per the table above, but the DB does not enforce direction), `closed_at` retains the original value — the application layer must not allow the backwards transition.

**`updated_at`:** Refreshed by a SQLite trigger on any UPDATE to the `tasks` row:

```sql
CREATE TRIGGER IF NOT EXISTS tasks_au AFTER UPDATE ON tasks
BEGIN
    UPDATE tasks SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = new.id;
END
```

### 5.3 Linking Model

The `task_links` table mirrors the API from `github.com/skevetter/task-management` (`link_tasks`, `list_links`). Three link types:

| `link_type` | Semantics | Direction |
|-------------|-----------|-----------|
| `blocked_by` | `source` cannot progress until `target` is `done`/`closed` | Directed |
| `parent` | `source` is a subtask of `target` | Directed |
| `related_to` | Loose association | Undirected (stored once; query both directions) |

**Cycle prevention:** The application layer checks for cycles before inserting a `blocked_by` or `parent` link. The check runs a DFS from `target_id` following edges of the same type; if it reaches `source_id`, the insert is rejected with `NousError::CyclicLink`. SQLite does not enforce this — the check is in Rust.

**`related_to` queries:** Because `related_to` is undirected, `list_links` returns rows where `source_id = ?1 OR target_id = ?1` for this type. `blocked_by` and `parent` only return rows where `source_id = ?1` (following the directed meaning).

**Cascade behaviour:** Deleting a task cascades to all `task_links` rows where it appears as either `source_id` or `target_id` (two FK constraints on `task_links`, both `ON DELETE CASCADE`). The second FK (`target_id`) is covered by the `idx_task_links_target` index.

### 5.4 MCP Tools

Nine MCP tools, following the registration pattern in `crates/nous-cli/src/server.rs`. Tools that mutate state use `WriteChannel`; read-only tools use `ReadPool`.

| Tool | Operation | Channel |
|------|-----------|---------|
| `task_create` | INSERT tasks row | `WriteChannel` |
| `task_update` | UPDATE status, priority, assignee, labels, description | `WriteChannel` |
| `task_close` | Shorthand: UPDATE status=closed, set closed_at | `WriteChannel` |
| `task_get` | SELECT by id; includes linked tasks and last 5 room messages | `ReadPool` |
| `task_list` | SELECT with filters; returns paginated list | `ReadPool` |
| `task_link` | INSERT task_links row | `WriteChannel` |
| `task_unlink` | DELETE task_links row by (source, target, type) | `WriteChannel` |
| `task_list_links` | SELECT links for a task id | `ReadPool` |
| `task_add_note` | POST a message to the task's linked room; returns message id | `WriteChannel` |

**`task_create` params:**
```json
{
  "title": "string (required)",
  "description": "string (optional)",
  "priority": "'critical'|'high'|'medium'|'low' (default: medium)",
  "assignee_id": "string (optional)",
  "labels": ["optional", "array"],
  "room_id": "string (optional, existing room)",
  "create_room": "boolean (optional, creates task-<id> room if true)"
}
```

**`task_create` return:**
```json
{
  "id": "018f4a00-...",
  "title": "Fix embeddings regression",
  "status": "open",
  "priority": "high",
  "room_id": "018f4a01-...",
  "created_at": "2026-04-29T10:00:00.000Z"
}
```

**`task_list` params:**
```json
{
  "status": "open|in_progress|done|closed (optional)",
  "assignee_id": "string (optional)",
  "label": "string (optional, single label filter)",
  "limit": "integer (default 50)",
  "offset": "integer (default 0)",
  "order_by": "'created_at'|'updated_at'|'priority' (default: created_at)",
  "order_dir": "'asc'|'desc' (default: desc)"
}
```

**`task_get` return** (abbreviated):
```json
{
  "id": "018f4a00-...",
  "title": "Fix embeddings regression",
  "description": "BM25 scores drift after re-embed...",
  "status": "in_progress",
  "priority": "high",
  "assignee_id": "bf91-59e2-eng-abc1",
  "labels": ["bug", "embeddings"],
  "room_id": "018f4a01-...",
  "links": {
    "blocked_by": [],
    "parent": [],
    "related_to": ["018f3f00-..."]
  },
  "recent_discussion": [
    {
      "sender_id": "bf91-59e2-eng-abc1",
      "content": "Reproducing locally now",
      "created_at": "2026-04-29T10:05:00.000Z"
    }
  ],
  "created_at": "2026-04-29T10:00:00.000Z",
  "updated_at": "2026-04-29T10:05:00.000Z"
}
```

**`task_add_note` behaviour:** Requires `room_id` to be set on the task. Posts `content` to the linked room as the caller's `sender_id` with `metadata.topics = ["task:<task_id>"]`. Returns `{"message_id": "..."}`. Callers without a room linked receive `NousError::NoLinkedRoom`.

### 5.5 CLI Commands

Add `task` subcommand to `crates/nous-cli/src/main.rs` following the `room` pattern.

```bash
# Create a task
nous-cli task create "Fix embeddings regression" \
    --priority high \
    --assignee bf91-59e2-eng-abc1 \
    --label bug --label embeddings \
    --create-room

# List open tasks
nous-cli task list --status open

# List tasks assigned to me
nous-cli task list --assignee bf91-59e2-eng-abc1 --status in_progress

# Show task detail
nous-cli task get 018f4a00-...

# Update status
nous-cli task update 018f4a00-... --status in_progress

# Close a task
nous-cli task close 018f4a00-...

# Link tasks
nous-cli task link 018f4a00-... --blocks 018f3f00-...
nous-cli task link 018f4a00-... --parent 018f3e00-...
nous-cli task link 018f4a00-... --related-to 018f3d00-...

# List links for a task
nous-cli task links 018f4a00-...

# Add a note (posts to linked room)
nous-cli task note 018f4a00-... "Reproducing locally now"
```

**Clap struct (abbreviated):**

```rust
#[derive(Debug, Subcommand)]
enum TaskSubcommand {
    Create {
        title: String,
        #[arg(long)] description: Option<String>,
        #[arg(long, default_value = "medium")] priority: String,
        #[arg(long)] assignee: Option<String>,
        #[arg(long = "label", action = clap::ArgAction::Append)] labels: Vec<String>,
        #[arg(long)] room_id: Option<String>,
        #[arg(long)] create_room: bool,
    },
    List {
        #[arg(long)] status: Option<String>,
        #[arg(long)] assignee: Option<String>,
        #[arg(long)] label: Option<String>,
        #[arg(long, default_value = "50")] limit: usize,
        #[arg(long, default_value = "0")] offset: usize,
    },
    Get { id: String },
    Update {
        id: String,
        #[arg(long)] status: Option<String>,
        #[arg(long)] priority: Option<String>,
        #[arg(long)] assignee: Option<String>,
        #[arg(long)] description: Option<String>,
    },
    Close { id: String },
    Link {
        id: String,
        #[arg(long)] blocks: Option<String>,
        #[arg(long)] parent: Option<String>,
        #[arg(long = "related-to")] related_to: Option<String>,
    },
    Links { id: String },
    Note { id: String, content: String },
}
```

### 5.6 Integration with Rooms and Agents

#### Tasks ↔ Rooms

- `tasks.room_id` is a nullable FK to `rooms.id` (`ON DELETE SET NULL`). Deleting a room sets `tasks.room_id = NULL`; the task survives.
- `task_create(create_room=true)` auto-creates a room named `task-<task_id>` and subscribes the creating agent as owner.
- `task_get` returns the last 5 messages from `tasks.room_id` under `recent_discussion`, scoped to topic `task:<task_id>`.
- `task_add_note` calls `post_message` with `topics=["task:<task_id>"]`. Agents subscribed to the room with `topics=["task:<task_id>"]` receive targeted notifications without reading unrelated traffic.

#### Tasks ↔ Agents

- `tasks.assignee_id` stores a Paseo agent ID. The scheduler (outside Nous scope) reads open tasks assigned to a given agent and routes them.
- `task_list(assignee_id=self)` is the standard agent startup query: "what tasks am I responsible for?"
- `task_update(id, status=in_progress)` is the agent's signal that it has picked up the task.
- `task_close` or `task_update(status=done)` signals completion.

The sequence for an orchestrator assigning work:

```
  orchestrator                        worker agent
       │                                  │
       │─ task_create(title, assignee) ──►│  (task row created)
       │                                  │
       │                              task_list(assignee=self, status=open)
       │                                  │◄── task appears
       │                              task_update(status=in_progress)
       │                                  │
       │                              ... work ...
       │                                  │
       │                              task_close(id)
       │◄── room message notification ───│  (if subscribed to room)
```

#### Tasks ↔ Memory

A task can be linked to a memory via the existing `memory_relate` operation (`docs/design/02-data-layer.md`). Relationship type: `"derived_from"` (memory → task) or `"implements"` (task → memory). No FK constraint — the link lives in the `relationships` table, not in `tasks`.

---

## 6. Dependencies

| Dependency | Version / Reference | Required by |
|------------|--------------------|----|
| SQLite WAL mode + FTS5 | System SQLite (≥3.35) | Rooms FTS, task indexes |
| `rusqlite` | existing in `Cargo.toml` | All DB operations |
| `tokio::sync::broadcast` | existing `tokio` dep | Notification delivery |
| `dashmap` | add if not present | `NotificationRegistry` (concurrent room→sender map) |
| `uuid` (v7) | `crates/nous-shared/src/ids.rs:40` | Task and room IDs |
| `rmcp` | existing | MCP tool registration |
| `clap` | existing | CLI subcommands |
| `docs/design-rooms.md` | this branch | Full rooms schema — must be merged before P0 ships |
| `docs/design/01-system-architecture.md` | this branch | UUIDv7 ID conventions |
| `docs/design/02-data-layer.md` | this branch | WriteChannel/ReadPool pattern |
| `docs/design/03-api-interfaces.md` | this branch | MCP tool + CLI patterns |
| `org-management` MCP server | running in Paseo org | `@mention` name resolution |
| `github.com/skevetter/task-management` | reference implementation | Tool API shape compatibility |

---

## 7. Open Questions

1. **Notification durability**: In-memory `broadcast::Sender` does not survive process restart. Agents that were offline miss notifications and must re-read from cursor. Should we persist a `last_delivered_at` cursor per subscriber so reconnecting agents get an accurate "you missed N messages" count, rather than requiring a full `room_read_messages` scan?

2. **`room_wait` and MCP timeout**: The MCP protocol has no standard long-poll mechanism. `room_wait` blocks the HTTP connection for up to 120 seconds. Some MCP clients enforce shorter timeouts. Should `room_wait` return immediately with a poll token, and a separate `room_poll` call checks delivery? Or is 120s acceptable given that Nous agents are the primary consumers (not browser clients)?

3. **Topic enforcement**: Topics are free-form strings today. Unregistered or misspelled topics silently produce no matches. Should we add a `topics` table with FK enforcement, or keep topics as advisory metadata and document the conventions?

4. **Task history / audit log**: The reference `task-management` API exposes `task_history`. This design has no `task_history` table. Should we add an `task_events` table (`(id, task_id, event_type, payload, created_at)`) to record status transitions and assignment changes, or delegate history to the linked room (where agents post notes on transitions)?

5. **Priority as integer vs enum**: `priority TEXT CHECK(...)` is human-readable but requires mapping to sort order (`critical=0, high=1, medium=2, low=3`) in `ORDER BY` queries. `priority INTEGER` with a display mapping is more sortable. Worth changing before migration is committed?

6. **`task_list` performance at scale**: The `labels` JSON filter uses `json_each`, which scans the full table for the label membership check. At 10k+ tasks, this will be slow. A `task_labels` junction table with a composite index `(label, status, assignee_id)` would fix this but adds schema complexity. Defer until benchmarks show the need?

7. **Cross-task search**: There is no FTS5 table for task titles and descriptions. Free-text search over task content is not supported in MVP. Add `tasks_fts` (covering `title || ' ' || coalesce(description, '')`) as part of P1, or defer to a follow-up?
