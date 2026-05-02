# Agent Chat & Task Management Integration

**Status:** Draft  
**Date:** 2026-05-02  
**Workstream:** NOUS-062

---

## Table of Contents

1. [Current State](#1-current-state)
2. [Problem Statement](#2-problem-statement)
3. [Target Architecture](#3-target-architecture)
4. [Chat Enhancement Layer](#4-chat-enhancement-layer)
5. [Task-Chat Integration](#5-task-chat-integration)
6. [Agent Coordination Patterns](#6-agent-coordination-patterns)
7. [Notification Enhancement](#7-notification-enhancement)
8. [MCP Tool Extensions](#8-mcp-tool-extensions)
9. [Agent Form Extensions](#9-agent-form-extensions)
10. [Migration Plan](#10-migration-plan)
11. [Testing Strategy](#11-testing-strategy)
12. [Open Decisions](#12-open-decisions)

---

## 1. Current State

### 1.1 Chat/Messaging Infrastructure

The chat system is built on three tables and one in-memory registry:

| Component | Location | Purpose |
|-----------|----------|---------|
| `rooms` | Migration 002 (`pool.rs:26-38`) | Room entity with name, purpose, metadata, archived flag |
| `room_messages` | Migration 003 (`pool.rs:40-51`) | Messages with sender_id, content, reply_to, metadata JSON |
| `room_messages_fts` | Migration 004/022 (`pool.rs:53-58`) | FTS5 full-text search on message content |
| `room_subscriptions` | Migration 005 (`pool.rs:60-69`) | Per-agent topic subscriptions (room_id, agent_id, topics JSON) |
| `NotificationRegistry` | `notifications/mod.rs:34-72` | In-memory `HashMap<String, broadcast::Sender<Notification>>` per room |

Core operations (`crates/nous-core/src/messages/mod.rs`):

| Function | Line | Description |
|----------|------|-------------|
| `post_message` | 68 | Insert message, extract topics/mentions from metadata, notify via registry |
| `read_messages` | 158 | Paginated read with `since`/`before` filters, ASC order, max 200 |
| `search_messages` | 205 | FTS5 MATCH query with optional room filter, BM25-ranked |
| `list_mentions` | 256 | Content LIKE `%@{agent_id}%` scan — no index support |

Room operations (`crates/nous-core/src/rooms/mod.rs`):

| Function | Line | Description |
|----------|------|-------------|
| `create_room` | 44 | UUIDv7 ID, unique name constraint (active only) |
| `get_room` | 93 | Lookup by ID first, then by name (non-archived) |
| `inspect_room` | 171 | Returns message_count, last_message_at, subscriber_count |

Notification layer (`crates/nous-core/src/notifications/mod.rs`):

| Function | Line | Description |
|----------|------|-------------|
| `room_wait` | 155 | Subscribe to broadcast channel, wait with timeout, optional topic filter |
| `subscribe_to_room` | 80 | Persist subscription to `room_subscriptions` table |
| `list_subscriptions` | 124 | List subscriptions for an agent |

### 1.2 Task Management Infrastructure

Tasks span four tables with comprehensive lifecycle tracking (`crates/nous-core/src/tasks/mod.rs`, 1052 lines):

| Table | Migration | Schema |
|-------|-----------|--------|
| `tasks` | 006 (`pool.rs:72-90`) | `id`, `title`, `description`, `status` (open/in_progress/done/closed), `priority` (critical/high/medium/low), `assignee_id TEXT`, `labels TEXT` (JSON array), `room_id TEXT → rooms(id)`, timestamps |
| `task_links` | 007 (`pool.rs:92-104`) | `source_id → tasks`, `target_id → tasks`, `link_type` (blocked_by/parent/related_to), unique triple |
| `task_events` | 008 (`pool.rs:106-118`) | `task_id → tasks`, `event_type` (created/status_changed/assigned/priority_changed/linked/unlinked/note_added), `old_value`, `new_value`, `actor_id` |
| `task_dependencies` | 019 (`pool.rs:366-387`) | `task_id → tasks`, `depends_on_task_id → tasks`, `dep_type` (blocked_by/blocks/waiting_on), cycle detection via BFS |
| `task_templates` | 019 (`pool.rs:375-387`) | `name UNIQUE`, `title_pattern`, `description_template`, `default_priority`, `default_labels JSON`, `checklist JSON` |
| `tasks_fts` | 009/022 | FTS5 on `title || description`, auto-synced triggers |

Key operations:

| Function | Line | Description |
|----------|------|-------------|
| `create_task` | 115 | Optional auto-create room, emit 'created' event |
| `update_task` | 288 | Per-field updates with event logging for status/priority/assignee changes |
| `add_note` | 554 | Auto-create room if none, post message with `task:{id}` topic, emit 'note_added' event |
| `link_tasks` | 413 | Cycle detection for blocked_by/parent, emit 'linked' event |
| `add_dependency` | 692 | BFS cycle detection via `would_create_cycle`, unique constraint on (task, dep, type) |
| `create_from_template` | 930 | Variable substitution `{{var}}` in title/description patterns |
| `batch_close` / `batch_update_status` / `batch_assign` | 994-1051 | Batch operations with per-task error collection |

### 1.3 Agent Process Management

Agents are registered in the `agents` table (migration 011, `pool.rs:152-171`) with hierarchy via `agent_relationships` (migration 012). Process management uses two additional tables:

| Table | Migration | Purpose |
|-------|-----------|---------|
| `agent_processes` | 023/026 (`pool.rs:423-514`) | Process lifecycle: type (claude/shell/http/sandbox), command, PID, status (pending→starting→running→stopping→stopped/failed/crashed), sandbox fields |
| `agent_invocations` | 024 (`pool.rs:449-468`) | Prompt/response tracking per invocation: status (pending/running/completed/failed/timeout/cancelled), duration_ms |

Agent forms are parsed by `crates/nous-core/src/agents/definition.rs:7-44`:

```rust
// crates/nous-core/src/agents/definition.rs
pub struct AgentDefinition {
    pub agent: AgentSection,         // name, type, version, namespace, description
    pub process: Option<ProcessSection>,    // type, spawn_command, working_dir, auto_restart
    pub skills: Option<SkillsSection>,      // refs: Vec<String>
    pub metadata: Option<MetadataSection>,  // model, timeout, tags
}
```

Runtime orchestration in `crates/nous-daemon/src/process_manager.rs`:

- `ProcessRegistry` holds `HashMap<String, ProcessHandle>` keyed by agent_id (`process_manager.rs:31`)
- `spawn()` creates DB record → starts OS process → background monitor task (`process_manager.rs:48-182`)
- `invoke()` dispatches to `invoke_claude` (rig Agent) or `invoke_shell` (sh -c) based on `process_type` (`process_manager.rs:671-742`)
- `AppState` (`crates/nous-daemon/src/state.rs:14-28`) composes: `pool`, `vec_pool`, `registry: Arc<NotificationRegistry>`, `process_registry: Arc<ProcessRegistry>`, `llm_client`, `schedule_notify`, `shutdown`

### 1.4 Integration Points

Current cross-subsystem integrations:

| Integration | Mechanism | Location |
|-------------|-----------|----------|
| Task → Room | `tasks.room_id` FK to `rooms(id)`, ON DELETE SET NULL | Migration 006 (`pool.rs:82`) |
| Task notes → Messages | `add_note` posts to task's room with `task:{id}` topic metadata | `tasks/mod.rs:554-619` |
| Task → Auto-create room | `create_task(..., create_room=true)` creates `task-{id}` room | `tasks/mod.rs:133-145` |
| Agent → Room | `agents.room` column (free-form TEXT, not FK) | Migration 011 (`pool.rs:162`) |
| Worktree → Task | `worktrees.task_id` FK to `tasks(id)`, ON DELETE SET NULL | Migration 010 (`pool.rs:141`) |
| Worktree → Agent | `worktrees.agent_id` (free-form TEXT, not FK) | Migration 010 (`pool.rs:138`) |
| Notification → Message | `post_message` calls `registry.notify()` with topics/mentions from metadata | `messages/mod.rs:114-125` |
| Subscription → Room | `room_subscriptions` persisted in DB, but NOT used for notification routing | `notifications/mod.rs:80-108` |

### 1.5 Gap Analysis

| Feature | Current State | Gap |
|---------|--------------|-----|
| Message indexing | No indexes on `room_messages` for room_id or created_at | Queries do full table scans for large rooms |
| Threading | `reply_to` field exists but no thread-fetch API | Cannot retrieve a reply chain or thread view |
| Read tracking | None | No way to track which messages an agent has seen |
| Message types | All messages are plain text, type inferred from sender | No system messages, task events, or structured commands |
| Message editing/deletion | Not supported | Messages are immutable once posted |
| Notification routing | `room_subscriptions` table populated but ignored by `NotificationRegistry` | Subscriptions persist but broadcast goes to ALL room listeners regardless of topics |
| Notification persistence | In-memory broadcast channels only | Lost on daemon restart; no replay of missed notifications |
| Task → Room events | `add_note` posts to room, but status/priority/assignment changes do not | Task lifecycle invisible in room unless manually posted |
| Task commands from chat | Not supported | Agents cannot issue task commands (close, assign, etc.) via chat messages |
| Agent assignment enforcement | `tasks.assignee_id` is free-form TEXT | No FK to `agents` table; any string accepted, no validation |
| Agent form → chat/tasks | No `[chat]` or `[tasks]` section in `AgentDefinition` | Forms cannot declare chat capabilities, auto-subscribe rooms, or task permissions |
| Agent-to-agent protocol | Agents communicate via raw room messages | No structured handoff format, no presence broadcasting |
| Coordination room patterns | Manually created per workflow | No auto-creation of coordination rooms for agent hierarchies |
| `room_wait` filtering | Topic filter loops over broadcast, no server-side subscription filtering | Agents receive and discard irrelevant messages before matching |
| `list_mentions` | Content LIKE scan (`messages/mod.rs:266`) | No index; O(n) scan over all messages in room |

---

## 2. Problem Statement

The nous platform has three mature subsystems — chat (rooms/messages), tasks (lifecycle/dependencies/templates), and agents (registration/processes/invocations) — that were designed independently and remain loosely coupled. This limits multi-agent coordination in several ways:

**Task lifecycle is invisible in chat.** When a task status changes from `open` to `in_progress` or a new assignee is set, the associated room receives no notification. Agents monitoring a room must poll `task_show` to detect changes. The only cross-subsystem bridge is `add_note`, which posts a message when an agent explicitly adds a note — but the seven other event types (status_changed, assigned, priority_changed, linked, unlinked, created) are silently logged to `task_events` with no room projection.

**Agents cannot coordinate through structured protocols.** Agent-to-agent communication happens via raw text messages in rooms. There is no structured handoff format — when a manager delegates to an engineer, the handoff context (task ID, branch, scope, acceptance criteria) is embedded in free-form prose that must be parsed by the receiving agent's LLM. This is fragile, context-expensive, and non-verifiable.

**Assignment is unenforced.** The `assignee_id` column on tasks accepts arbitrary text (`pool.rs:79`), not a validated reference to `agents(id)`. This means a task can be "assigned" to a non-existent agent, and there is no way to query "all tasks assigned to this agent" with referential integrity.

**Notifications don't survive restarts.** The `NotificationRegistry` is a pure in-memory `HashMap<String, broadcast::Sender>` (`notifications/mod.rs:35`). If the daemon restarts, all pending notifications are lost and agents blocking on `room_wait` receive a channel-closed error rather than a reconnectable stream. The `room_subscriptions` table exists but is never consulted for notification routing — subscriptions with topic filters are persisted and then ignored.

**Agent forms have no chat/task awareness.** The `AgentDefinition` struct (`definition.rs:7-44`) defines process, skills, and metadata sections but nothing about which rooms an agent should subscribe to, what task operations it is authorized to perform, or how it should handle incoming messages. This means all coordination behavior is encoded in skill markdown text rather than in structured, validatable configuration.

**The CLI wait command breaks with multi-process.** `room_wait` subscribes to a broadcast channel at call time (`notifications/mod.rs:163`). If multiple agents call `room_wait` on the same room, they each get independent receivers, but if a message arrives between subscription and the first `recv()`, it is lost (no replay buffer beyond the broadcast channel's 256-slot capacity).

---

## 3. Target Architecture

### 3.1 Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────┐
│                         MCP Tool Layer                               │
│  room_thread_view  task_command  agent_handoff  room_wait_filtered   │
└──────────┬──────────────┬──────────────┬──────────────┬──────────────┘
           │              │              │              │
┌──────────▼──────────────▼──────────────▼──────────────▼──────────────┐
│                      Coordination Layer                               │
│                                                                       │
│  ┌─────────────┐  ┌──────────────┐  ┌──────────────────────────┐     │
│  │ ChatEnhance │  │ TaskChat     │  │ AgentCoordination        │     │
│  │             │  │ Integration  │  │                          │     │
│  │ • threading │  │ • lifecycle  │  │ • structured handoffs    │     │
│  │ • msg types │  │   events     │  │ • presence broadcasting  │     │
│  │ • cursors   │  │ • commands   │  │ • coordination rooms     │     │
│  │ • indexing  │  │ • assignment │  │ • agent form extensions  │     │
│  └──────┬──────┘  └──────┬───────┘  └──────────┬───────────────┘     │
│         │                │                     │                      │
└─────────┼────────────────┼─────────────────────┼──────────────────────┘
          │                │                     │
┌─────────▼────────────────▼─────────────────────▼──────────────────────┐
│                     Notification Layer                                 │
│                                                                       │
│  NotificationRegistry (in-memory broadcast + persistent replay)       │
│  ┌─────────────────────────────────────────────────────────────┐      │
│  │ room_subscriptions → server-side topic + mention filtering  │      │
│  │ notification_queue  → persistent WAL-backed replay buffer   │      │
│  │ agent cursors       → per-agent read position tracking      │      │
│  └─────────────────────────────────────────────────────────────┘      │
└───────────────────────────────┬────────────────────────────────────────┘
                                │
┌───────────────────────────────▼────────────────────────────────────────┐
│                        Storage Layer (SQLite)                          │
│                                                                       │
│  rooms ─── room_messages ─── room_messages_fts                        │
│    │           │                                                      │
│    │       message_cursors (NEW)                                      │
│    │       notification_queue (NEW)                                   │
│    │                                                                  │
│  tasks ─── task_events ─── task_links ─── task_dependencies           │
│    │                                                                  │
│  agents ── agent_processes ── agent_invocations                       │
│    │                                                                  │
│  agent_relationships ── artifacts                                     │
└────────────────────────────────────────────────────────────────────────┘
```

### 3.2 Core Principles

1. **Agent-first.** Every enhancement is designed for programmatic agent consumption. Human-readable formatting is a secondary concern; structured metadata and typed enums are primary.

2. **Room-centric.** Rooms remain the fundamental coordination primitive. Task events flow into rooms. Agent handoffs are room messages. Coordination patterns auto-create rooms. All observable state changes produce room messages.

3. **Event-driven.** Task lifecycle changes, agent status transitions, and coordination events produce notifications that agents can subscribe to with server-side filtering. No polling required for state changes.

4. **Backwards-compatible schema evolution.** New tables and columns via additive migrations (027+). No breaking changes to existing `room_messages`, `tasks`, or `agents` tables. Existing MCP tools continue to work unchanged.

5. **SQLite-native.** All new features use the same patterns: UUIDv7 PKs, JSON-in-TEXT columns, FTS5 companion tables, CHECK constraints for enums, and triggers for updated_at.

---

## 4. Chat Enhancement Layer

### 4.1 Message Types

Add a `message_type` column to `room_messages` to distinguish structured message categories:

| Type | Description | Producer |
|------|-------------|----------|
| `user` | Standard agent-authored message (default, backwards-compatible) | `post_message` |
| `system` | Room lifecycle events (created, archived, agent joined/left) | Daemon internals |
| `task_event` | Task status change, assignment, priority change projected into room | `TaskChatBridge` |
| `command` | Parseable task/agent command embedded in a message | Agent via `task_command` tool |
| `handoff` | Structured agent-to-agent delegation with typed metadata | Agent via `agent_handoff` tool |

The column uses a CHECK constraint and defaults to `'user'` so existing messages require no backfill.

### 4.2 Threading Model

The existing `reply_to` field on `room_messages` already supports single-level replies. Extend this to support thread retrieval:

A **thread** is defined as a root message plus all messages where `reply_to` points to that root (flat threading, not nested). This matches the current data model — `reply_to` is already a single optional FK.

Thread fetch query:

```sql
-- Get thread rooted at :root_id
SELECT * FROM room_messages
WHERE id = :root_id OR reply_to = :root_id
ORDER BY created_at ASC;
```

No schema changes required for flat threading. The missing piece is an API endpoint and index (see 4.5).

### 4.3 Structured Message Metadata

The existing `metadata TEXT` column on `room_messages` holds arbitrary JSON. Standardize the schema for typed metadata:

```json
{
  "topics": ["task:abc123", "deploy"],
  "mentions": ["agent-uuid-1", "agent-uuid-2"],
  "message_type": "task_event",
  "task_event": {
    "task_id": "abc123",
    "event_type": "status_changed",
    "old_value": "open",
    "new_value": "in_progress",
    "actor_id": "agent-uuid-3"
  },
  "handoff": {
    "from_agent": "mgr-uuid",
    "to_agent": "eng-uuid",
    "task_id": "abc123",
    "context": { "branch": "feat/foo", "scope": "src/api/" }
  }
}
```

This remains a JSON-in-TEXT column — no schema change. The Rust layer validates the structure via serde deserialization.

### 4.4 Read Tracking / Cursors

New table to track per-agent read position in each room:

```sql
-- Migration 027
CREATE TABLE IF NOT EXISTS message_cursors (
    room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL,
    last_read_message_id TEXT NOT NULL,
    last_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (room_id, agent_id)
);
CREATE INDEX IF NOT EXISTS idx_cursors_agent ON message_cursors(agent_id);
```

Operations:
- **Advance cursor**: After reading messages, agent calls `room_mark_read(room_id, agent_id, message_id)` to update their position
- **Get unread count**: `SELECT COUNT(*) FROM room_messages WHERE room_id = ? AND created_at > (SELECT last_read_at FROM message_cursors WHERE room_id = ? AND agent_id = ?)`
- **Implicit advance**: `room_read_messages` can optionally auto-advance the cursor for the calling agent

### 4.5 Message Indexing

The `room_messages` table (migration 003) has NO indexes beyond the PK. Every query in `messages/mod.rs` scans the full table per room. Add:

```sql
-- Migration 027 (continued)
CREATE INDEX IF NOT EXISTS idx_room_messages_room_created
    ON room_messages(room_id, created_at);
CREATE INDEX IF NOT EXISTS idx_room_messages_reply_to
    ON room_messages(reply_to) WHERE reply_to IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_room_messages_sender
    ON room_messages(room_id, sender_id, created_at);
```

| Index | Covers | Current query |
|-------|--------|---------------|
| `idx_room_messages_room_created` | `read_messages` WHERE room_id + ORDER BY created_at | Full scan → index scan |
| `idx_room_messages_reply_to` | Thread fetch (WHERE reply_to = ?) | Full scan → index lookup |
| `idx_room_messages_sender` | `list_mentions` and per-sender queries | LIKE scan remains, but sender filter fast |

The `list_mentions` function (`messages/mod.rs:256`) currently uses `content LIKE %@{agent_id}%` which cannot use a B-tree index. Options: (a) keep LIKE for simplicity since mention volume is low, (b) add a `message_mentions` junction table. Recommendation: keep LIKE for now, add junction table if mention volume exceeds 10K per room.

### 4.6 New Rust Types

```rust
// crates/nous-core/src/messages/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    User,
    System,
    TaskEvent,
    Command,
    Handoff,
}

impl Default for MessageType {
    fn default() -> Self {
        Self::User
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadView {
    pub root: Message,
    pub replies: Vec<Message>,
    pub reply_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCursor {
    pub room_id: String,
    pub agent_id: String,
    pub last_read_message_id: String,
    pub last_read_at: String,
    pub unread_count: i64,
}

#[derive(Debug)]
pub struct GetThreadRequest {
    pub room_id: String,
    pub root_message_id: String,
}

#[derive(Debug)]
pub struct MarkReadRequest {
    pub room_id: String,
    pub agent_id: String,
    pub message_id: String,
}

#[derive(Debug)]
pub struct UnreadCountRequest {
    pub room_id: String,
    pub agent_id: String,
}
```

New functions:

| Function | Signature | Description |
|----------|-----------|-------------|
| `get_thread` | `(pool, req: GetThreadRequest) -> Result<ThreadView, NousError>` | Fetch root + all replies ordered by created_at |
| `mark_read` | `(pool, req: MarkReadRequest) -> Result<ChatCursor, NousError>` | Upsert cursor position |
| `unread_count` | `(pool, req: UnreadCountRequest) -> Result<ChatCursor, NousError>` | Count messages after cursor position |

---

## 5. Task-Chat Integration

### 5.1 Task Lifecycle Events in Rooms

When a task has an associated room (`tasks.room_id IS NOT NULL`), every `task_events` insert should auto-post a `task_event` typed message to that room. This bridges the gap identified in Section 1.5.

Events to project:

| Task Event Type | Room Message Content | Metadata |
|----------------|---------------------|----------|
| `status_changed` | `Task status: {old} → {new}` | `{"message_type": "task_event", "task_event": {"task_id", "event_type": "status_changed", "old_value", "new_value", "actor_id"}}` |
| `assigned` | `Task assigned: {old} → {new}` | Same structure with `event_type: "assigned"` |
| `priority_changed` | `Task priority: {old} → {new}` | Same structure with `event_type: "priority_changed"` |
| `linked` | `Task linked: {link_type}:{target_id}` | Same structure with `event_type: "linked"` |
| `created` | `Task created: {title}` | Same structure with `event_type: "created"` |

Implementation: Add a `post_task_event_to_room` helper called from `update_task` and `create_task` after each event insert:

```rust
// crates/nous-core/src/tasks/mod.rs
async fn post_task_event_to_room(
    pool: &SqlitePool,
    registry: Option<&NotificationRegistry>,
    task_id: &str,
    room_id: &str,
    event_type: &str,
    old_value: Option<&str>,
    new_value: Option<&str>,
    actor_id: Option<&str>,
) -> Result<(), NousError> {
    let content = match event_type {
        "status_changed" => format!(
            "Task status: {} → {}",
            old_value.unwrap_or("none"),
            new_value.unwrap_or("none")
        ),
        "assigned" => format!(
            "Task assigned: {} → {}",
            old_value.unwrap_or("unassigned"),
            new_value.unwrap_or("unassigned")
        ),
        "priority_changed" => format!(
            "Task priority: {} → {}",
            old_value.unwrap_or("none"),
            new_value.unwrap_or("none")
        ),
        _ => format!("Task event: {event_type}"),
    };

    let metadata = serde_json::json!({
        "message_type": "task_event",
        "topics": [format!("task:{task_id}")],
        "task_event": {
            "task_id": task_id,
            "event_type": event_type,
            "old_value": old_value,
            "new_value": new_value,
            "actor_id": actor_id
        }
    });

    post_message(
        pool,
        PostMessageRequest {
            room_id: room_id.to_string(),
            sender_id: "system".to_string(),
            content,
            reply_to: None,
            metadata: Some(metadata),
        },
        registry,
    )
    .await?;
    Ok(())
}
```

### 5.2 Task Commands from Chat

Allow agents to issue task operations via specially-formatted chat messages. The `task_command` MCP tool parses a command string and dispatches to the appropriate task operation:

| Command | Maps To | Example |
|---------|---------|---------|
| `/task close {task_id}` | `close_task(pool, task_id, actor_id)` | `/task close abc123` |
| `/task assign {task_id} {agent_id}` | `update_task(pool, id, assignee_id=agent_id)` | `/task assign abc123 eng-uuid` |
| `/task status {task_id} {status}` | `update_task(pool, id, status=status)` | `/task status abc123 in_progress` |
| `/task priority {task_id} {priority}` | `update_task(pool, id, priority=priority)` | `/task priority abc123 high` |
| `/task link {source} {target} {type}` | `link_tasks(pool, source, target, type)` | `/task link abc123 def456 blocked_by` |

Commands are posted as regular messages with `message_type: "command"` and parsed server-side by the `task_command` handler.

### 5.3 Task Discussion Threads

When a task event is posted to a room (5.1), agents can reply to that event message to create a discussion thread. The thread retrieval API (4.2) provides the full context.

Pattern: task event message becomes the thread root, agent discussion replies to it:

```
[system] Task status: open → in_progress          ← root (task_event)
  ├── [eng-1] Starting implementation now           ← reply
  ├── [mgr-1] Focus on the auth module first        ← reply
  └── [eng-1] Auth module done, moving to API       ← reply
```

### 5.4 Agent Assignment Flow

Strengthen the task assignment model by enforcing that `assignee_id` references a registered agent:

**Current state:** `tasks.assignee_id` is `TEXT` with no FK constraint (`pool.rs:79`). Any string is accepted.

**Target state:** Add a soft validation at the Rust layer (not a hard FK, since tasks may outlive agent registrations):

```rust
// crates/nous-core/src/tasks/mod.rs — in update_task()
if let Some(new_assignee) = assignee_id {
    // Validate agent exists (soft check — logs warning if not found)
    match agents::get_agent_by_id(pool, new_assignee).await {
        Ok(agent) => {
            tracing::info!(
                task_id = %id,
                assignee = %new_assignee,
                agent_name = %agent.name,
                "task assigned to registered agent"
            );
        }
        Err(_) => {
            tracing::warn!(
                task_id = %id,
                assignee = %new_assignee,
                "task assigned to unregistered agent ID"
            );
        }
    }
    // ... proceed with assignment
}
```

Additionally, emit an `assigned` task event to the room (via 5.1) and notify the assigned agent via mention metadata:

```json
{
  "message_type": "task_event",
  "topics": ["task:abc123", "assignment"],
  "mentions": ["eng-uuid"],
  "task_event": {
    "task_id": "abc123",
    "event_type": "assigned",
    "new_value": "eng-uuid"
  }
}
```

### 5.5 New Rust Types and SQL Migrations

```rust
// crates/nous-core/src/tasks/mod.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCommand {
    pub command: String,          // "close", "assign", "status", "priority", "link"
    pub task_id: String,
    pub args: Vec<String>,        // positional arguments after task_id
    pub actor_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCommandResult {
    pub command: String,
    pub task_id: String,
    pub success: bool,
    pub message: String,
    pub task: Option<Task>,
}

pub async fn execute_task_command(
    pool: &SqlitePool,
    cmd: TaskCommand,
    registry: Option<&NotificationRegistry>,
) -> Result<TaskCommandResult, NousError>;
```

SQL additions for migration 027 (task-chat bridge — no new tables, just new event types):

```sql
-- Extend task_events CHECK to include new event types
-- NOTE: SQLite CHECK constraints cannot be ALTER'd. Since we're in prototype
-- phase, we accept the existing CHECK and store new event types that pass
-- the constraint. The existing CHECK already covers: created, status_changed,
-- assigned, priority_changed, linked, unlinked, note_added — which covers
-- all events we need to project to rooms.
```

No schema migration needed for the task-chat bridge itself — the existing `task_events` schema covers all required event types. The new code is purely in the Rust layer.

---

## 6. Agent Coordination Patterns

### 6.1 Agent-to-Agent Communication Protocol

Define a structured message envelope for agent-to-agent communication that LLMs can both produce and consume reliably:

```rust
// crates/nous-core/src/agents/coordination.rs (new file)

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub from_agent: String,
    pub to_agent: Option<String>,         // None = broadcast to room
    pub message_kind: AgentMessageKind,
    pub correlation_id: Option<String>,   // links related messages across a workflow
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    Handoff,          // delegation of work
    StatusUpdate,     // progress report
    Question,         // request for clarification
    Answer,           // response to question
    Completion,       // work finished, results attached
    Escalation,       // blocked, escalating to parent
}
```

Messages are posted to rooms as regular `room_messages` with `message_type: "handoff"` and the `AgentMessage` serialized into the `metadata` JSON column.

### 6.2 Structured Handoff Messages

A handoff is the most critical coordination primitive — it's how a manager delegates to an engineer. Define a typed handoff payload:

```rust
// crates/nous-core/src/agents/coordination.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffPayload {
    pub task_id: Option<String>,
    pub branch: Option<String>,
    pub scope: Option<String>,           // file/module scope
    pub acceptance_criteria: Vec<String>,
    pub context: serde_json::Value,      // arbitrary context blob
    pub deadline: Option<String>,        // ISO 8601 timestamp
}

pub async fn post_handoff(
    pool: &SqlitePool,
    registry: Option<&NotificationRegistry>,
    room_id: &str,
    from_agent: &str,
    to_agent: &str,
    payload: HandoffPayload,
) -> Result<Message, NousError> {
    let metadata = serde_json::json!({
        "message_type": "handoff",
        "mentions": [to_agent],
        "topics": ["handoff"],
        "handoff": {
            "from_agent": from_agent,
            "to_agent": to_agent,
            "task_id": payload.task_id,
            "branch": payload.branch,
            "scope": payload.scope,
            "acceptance_criteria": payload.acceptance_criteria,
            "context": payload.context,
            "deadline": payload.deadline
        }
    });

    post_message(
        pool,
        PostMessageRequest {
            room_id: room_id.to_string(),
            sender_id: from_agent.to_string(),
            content: format!(
                "Handoff to @{to_agent}: {}",
                payload.acceptance_criteria.first().unwrap_or(&"See metadata".to_string())
            ),
            reply_to: None,
            metadata: Some(metadata),
        },
        registry,
    )
    .await
}
```

### 6.3 Agent Presence and Status Broadcasting

Extend the agent heartbeat mechanism to broadcast presence events to coordination rooms:

```rust
// crates/nous-core/src/agents/coordination.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceEvent {
    pub agent_id: String,
    pub agent_name: String,
    pub status: String,              // active, idle, blocked, done
    pub current_task_id: Option<String>,
    pub room_id: String,
}

pub async fn broadcast_presence(
    pool: &SqlitePool,
    registry: Option<&NotificationRegistry>,
    agent_id: &str,
    status: &str,
) -> Result<(), NousError> {
    let agent = get_agent_by_id(pool, agent_id).await?;
    if let Some(ref room) = agent.room {
        let metadata = serde_json::json!({
            "message_type": "system",
            "topics": ["presence"],
            "presence": {
                "agent_id": agent_id,
                "agent_name": agent.name,
                "status": status
            }
        });
        post_message(
            pool,
            PostMessageRequest {
                room_id: room.clone(),
                sender_id: "system".to_string(),
                content: format!("Agent {} is now {}", agent.name, status),
                reply_to: None,
                metadata: Some(metadata),
            },
            registry,
        )
        .await?;
    }
    Ok(())
}
```

Integrate with `heartbeat()` in `agents/mod.rs:518-540` — after updating `last_seen_at`, call `broadcast_presence` if the status changed.

### 6.4 Coordination Room Patterns

Auto-create coordination rooms when agent hierarchies are established:

| Pattern | Trigger | Room Name | Purpose |
|---------|---------|-----------|---------|
| Manager-team room | Manager registers with children | `coord-{manager_name}` | Team coordination, status updates, handoffs |
| Task room | `create_task(create_room=true)` | `task-{task_id}` | Task-specific discussion (already exists) |
| Agent pair room | `post_handoff` between two agents | `pair-{agent1}-{agent2}` | Direct agent-to-agent channel |
| Namespace room | First agent registers in namespace | `ns-{namespace}` | Namespace-wide announcements |

Implementation in `register_agent`:

```rust
// crates/nous-core/src/agents/mod.rs — in register_agent()
// After inserting agent + relationship:
if let Some(ref parent_id) = req.parent_id {
    let parent = get_agent_by_id(pool, parent_id).await?;
    let coord_room_name = format!("coord-{}", parent.name);
    // Ensure coordination room exists (idempotent)
    match rooms::create_room(
        pool,
        &coord_room_name,
        Some(&format!("Coordination room for {}", parent.name)),
        None,
    ).await {
        Ok(room) => {
            // Auto-subscribe both parent and child
            subscribe_to_room(pool, &room.id, parent_id, None).await?;
            subscribe_to_room(pool, &room.id, &id, None).await?;
        }
        Err(NousError::Conflict(_)) => {
            // Room already exists — just subscribe the new child
            if let Ok(room) = rooms::get_room(pool, &coord_room_name).await {
                subscribe_to_room(pool, &room.id, &id, None).await?;
            }
        }
        Err(e) => return Err(e),
    }
}
```

### 6.5 Integration with Agent Forms

Extend `AgentDefinition` to declare chat and task capabilities (see Section 9 for full TOML examples):

```rust
// crates/nous-core/src/agents/definition.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub agent: AgentSection,
    pub process: Option<ProcessSection>,
    pub skills: Option<SkillsSection>,
    pub metadata: Option<MetadataSection>,
    pub chat: Option<ChatSection>,      // NEW
    pub tasks: Option<TasksSection>,    // NEW
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSection {
    pub auto_subscribe: Option<Vec<String>>,    // room names to auto-join
    pub presence_broadcast: Option<bool>,        // emit presence events
    pub message_types: Option<Vec<String>>,      // types this agent handles
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSection {
    pub can_create: Option<bool>,               // allowed to create tasks
    pub can_assign: Option<bool>,               // allowed to assign tasks
    pub can_close: Option<bool>,                // allowed to close tasks
    pub auto_accept_assignments: Option<bool>,  // auto-accept when assigned
    pub labels: Option<Vec<String>>,            // default labels for created tasks
}
```

When an agent form is loaded and the agent is registered, `register_agent` consults these sections to auto-subscribe to rooms and set up task permissions in agent metadata.

---

## 7. Notification Enhancement

### 7.1 Server-Side Topic Filtering

The `room_subscriptions` table (migration 005) stores per-agent topic filters but `NotificationRegistry` (`notifications/mod.rs:34-72`) ignores them. Fix: consult subscriptions when routing notifications.

```rust
// crates/nous-core/src/notifications/mod.rs — enhanced notify()

pub async fn notify_filtered(
    &self,
    pool: &SqlitePool,
    notification: Notification,
) {
    let sender = self.get_sender(&notification.room_id).await;

    // Fast path: broadcast to all listeners (existing behavior)
    let _ = sender.send(notification.clone());

    // Enhanced path: check subscriptions for topic-filtered agents
    // This is used by the persistent notification queue (7.4)
    let subscriptions = list_room_subscriptions(pool, &notification.room_id)
        .await
        .unwrap_or_default();

    for sub in subscriptions {
        if let Some(ref topics) = sub.topics {
            // Only queue if notification topics intersect subscription topics
            let matches = topics.is_empty()
                || notification.topics.iter().any(|t| topics.contains(t))
                || notification.mentions.contains(&sub.agent_id);
            if matches {
                let _ = enqueue_notification(pool, &sub.agent_id, &notification).await;
            }
        } else {
            // No topic filter — queue everything
            let _ = enqueue_notification(pool, &sub.agent_id, &notification).await;
        }
    }
}
```

### 7.2 Mention-Based Routing

When a message contains `@{agent_id}` in content or metadata mentions array, the notification should be routed to that agent regardless of topic subscription:

```rust
// crates/nous-core/src/notifications/mod.rs

pub fn should_notify_agent(
    notification: &Notification,
    subscription: &Subscription,
) -> bool {
    // Always notify on direct mention
    if notification.mentions.contains(&subscription.agent_id) {
        return true;
    }
    // Topic-based filtering
    match &subscription.topics {
        None => true,  // no filter = receive all
        Some(topics) if topics.is_empty() => true,
        Some(topics) => notification.topics.iter().any(|t| topics.contains(t)),
    }
}
```

### 7.3 Priority Notifications

Add priority levels to notifications for agents that need to distinguish urgent signals:

```rust
// crates/nous-core/src/notifications/mod.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationPriority {
    Low,      // presence updates, system messages
    Normal,   // regular messages, task events
    High,     // direct mentions, handoffs
    Urgent,   // escalations, critical task events
}

// Extended notification struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub room_id: String,
    pub message_id: String,
    pub sender_id: String,
    pub topics: Vec<String>,
    pub mentions: Vec<String>,
    pub priority: NotificationPriority,  // NEW
}
```

Priority is derived from message metadata:
- `Urgent`: message_type = "handoff" with escalation, or task priority = "critical"
- `High`: direct @mention, or message_type = "handoff"
- `Normal`: regular messages, task events
- `Low`: presence updates, system messages

### 7.4 Notification Persistence

New table to persist notifications that survive daemon restart:

```sql
-- Migration 027 (continued)
CREATE TABLE IF NOT EXISTS notification_queue (
    id TEXT NOT NULL PRIMARY KEY,
    agent_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    priority TEXT NOT NULL DEFAULT 'normal'
        CHECK(priority IN ('low','normal','high','urgent')),
    topics TEXT NOT NULL DEFAULT '[]',
    mentions TEXT NOT NULL DEFAULT '[]',
    delivered INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_notif_queue_agent
    ON notification_queue(agent_id, delivered, created_at);
CREATE INDEX IF NOT EXISTS idx_notif_queue_room
    ON notification_queue(room_id, created_at);
```

On daemon startup, the `NotificationRegistry` replays undelivered notifications from the queue. On `room_wait`, check the persistent queue first before subscribing to the broadcast channel:

```rust
// crates/nous-core/src/notifications/mod.rs

pub async fn room_wait_persistent(
    pool: &SqlitePool,
    registry: &NotificationRegistry,
    room_id: &str,
    agent_id: &str,
    timeout_ms: Option<u64>,
    topics: Option<&[String]>,
) -> Result<WaitResult, NousError> {
    // 1. Check persistent queue for undelivered notifications
    if let Some(queued) = dequeue_notification(pool, agent_id, room_id, topics).await? {
        return Ok(WaitResult {
            notification: Some(queued),
            timed_out: false,
        });
    }
    // 2. Fall back to live broadcast subscription
    room_wait(registry, room_id, timeout_ms, topics).await
}
```

---

## 8. MCP Tool Extensions

### 8.1 New Tools

| Tool Name | Description | Input Schema |
|-----------|-------------|-------------|
| `room_thread_view` | Get a thread (root message + all replies) | `{ room_id: string, root_message_id: string }` |
| `room_mark_read` | Update agent's read cursor in a room | `{ room_id: string, agent_id: string, message_id: string }` |
| `room_unread_count` | Get unread message count for an agent | `{ room_id: string, agent_id: string }` |
| `task_command` | Execute a task command from chat context | `{ room_id: string, command: string, task_id: string, args: string[], actor_id: string }` |
| `agent_handoff` | Send a structured handoff message | `{ room_id: string, from_agent: string, to_agent: string, task_id?: string, branch?: string, scope?: string, acceptance_criteria: string[], context?: object }` |
| `agent_presence` | Broadcast agent presence/status | `{ agent_id: string, status: string }` |

Tool schemas (following `ToolSchema` pattern from `mcp.rs:24-29`):

```rust
// crates/nous-daemon/src/routes/mcp.rs — additions to get_tool_schemas()

ToolSchema {
    name: "room_thread_view",
    description: "Get a message thread — root message and all replies",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "room_id": { "type": "string", "description": "Room ID" },
            "root_message_id": { "type": "string", "description": "Root message ID" }
        },
        "required": ["room_id", "root_message_id"]
    }),
},
ToolSchema {
    name: "room_mark_read",
    description: "Mark messages as read up to a given message ID",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "room_id": { "type": "string" },
            "agent_id": { "type": "string" },
            "message_id": { "type": "string", "description": "Last read message ID" }
        },
        "required": ["room_id", "agent_id", "message_id"]
    }),
},
ToolSchema {
    name: "room_unread_count",
    description: "Get the number of unread messages for an agent in a room",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "room_id": { "type": "string" },
            "agent_id": { "type": "string" }
        },
        "required": ["room_id", "agent_id"]
    }),
},
ToolSchema {
    name: "task_command",
    description: "Execute a task operation from chat (close, assign, status, priority, link)",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "room_id": { "type": "string", "description": "Room where command is issued" },
            "command": { "type": "string", "enum": ["close", "assign", "status", "priority", "link"] },
            "task_id": { "type": "string" },
            "args": { "type": "array", "items": { "type": "string" } },
            "actor_id": { "type": "string" }
        },
        "required": ["command", "task_id", "actor_id"]
    }),
},
ToolSchema {
    name: "agent_handoff",
    description: "Send a structured work handoff from one agent to another",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "room_id": { "type": "string" },
            "from_agent": { "type": "string" },
            "to_agent": { "type": "string" },
            "task_id": { "type": "string" },
            "branch": { "type": "string" },
            "scope": { "type": "string" },
            "acceptance_criteria": { "type": "array", "items": { "type": "string" } },
            "context": { "type": "object" }
        },
        "required": ["room_id", "from_agent", "to_agent"]
    }),
},
ToolSchema {
    name: "agent_presence",
    description: "Broadcast agent presence/status to coordination room",
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "agent_id": { "type": "string" },
            "status": { "type": "string", "enum": ["active", "idle", "blocked", "done"] }
        },
        "required": ["agent_id", "status"]
    }),
},
```

### 8.2 Enhanced Existing Tools

| Tool | Enhancement |
|------|------------|
| `room_wait` | Add optional `agent_id` parameter for persistent queue check (7.4). Add `priority_min` filter. |
| `room_post_message` | Add optional `message_type` parameter (defaults to "user"). Validate against MessageType enum. |
| `room_subscribe` | Return subscription confirmation with current unread count. |
| `task_update` | Accept optional `notify_room` boolean (default true) to control event projection (5.1). |

### 8.3 Tool Schemas

The new tools follow the existing pattern in `crates/nous-daemon/src/routes/mcp.rs`:

1. Schema defined as static `ToolSchema` in `get_tool_schemas()` (line 51+)
2. Handler dispatched via `match request.name` in `call_tool()` 
3. Arguments extracted via `serde_json::from_value` on `request.arguments`
4. Core logic delegated to `nous_core` functions
5. Response wrapped in `ToolCallResponse` with `ToolContent` text

Total tool count after additions: 105 (existing) + 6 (new) = **111 tools**.

---

## 9. Agent Form Extensions

### 9.1 Chat Capabilities

New `[chat]` section in agent form TOML:

```toml
[chat]
auto_subscribe = ["coord-team-alpha", "ns-eng"]  # rooms to auto-join on registration
presence_broadcast = true                          # emit status changes to coordination room
message_types = ["user", "handoff", "command"]     # types this agent can produce/consume
```

When `presence_broadcast = true`, the daemon wraps `heartbeat()` to call `broadcast_presence()` on status transitions.

### 9.2 Task Capabilities

New `[tasks]` section in agent form TOML:

```toml
[tasks]
can_create = true         # allowed to create tasks
can_assign = true         # allowed to assign tasks to other agents
can_close = true          # allowed to close tasks
auto_accept = true        # automatically accept when assigned a task
labels = ["eng", "impl"]  # default labels applied to tasks this agent creates
```

These capabilities are stored in `agents.metadata_json` as a structured JSON block and checked by the `task_command` handler before executing operations.

### 9.3 TOML Examples

**Manager agent with full coordination capabilities:**

```toml
# examples/agents/coordinator.toml
[agent]
name        = "coordinator"
type        = "manager"
version     = "1.0.0"
namespace   = "eng"
description = "Coordinates engineering tasks and delegates to engineers"

[process]
type          = "claude"
spawn_command = "claude --model claude-opus-4-6"
working_dir   = "~"
auto_restart  = false

[skills]
refs = [
  "planning",
  "code-review",
]

[metadata]
model   = "global.anthropic.claude-opus-4-6-v1"
timeout = 7200
tags    = ["coordination", "management"]

[chat]
auto_subscribe      = ["ns-eng", "coord-coordinator"]
presence_broadcast  = true
message_types       = ["user", "handoff", "command", "task_event"]

[tasks]
can_create  = true
can_assign  = true
can_close   = true
auto_accept = false
labels      = ["managed"]
```

**Engineer agent with limited task capabilities:**

```toml
# examples/agents/implementer.toml
[agent]
name        = "implementer"
type        = "engineer"
version     = "1.0.0"
namespace   = "eng"
description = "Implements features on assigned tasks"

[process]
type          = "claude"
spawn_command = "claude --model claude-sonnet-4-6"
working_dir   = "~"
auto_restart  = false
restart_policy = "on-failure"

[skills]
refs = [
  "code-review",
  "git-workflow",
]

[metadata]
model   = "global.anthropic.claude-sonnet-4-6-v1"
timeout = 3600
tags    = ["implementation", "engineering"]

[chat]
auto_subscribe      = []                # rooms assigned dynamically via handoff
presence_broadcast  = true
message_types       = ["user", "handoff"]

[tasks]
can_create  = false
can_assign  = false
can_close   = true                       # can close own tasks when done
auto_accept = true                       # auto-accept assignments
labels      = ["eng", "implementation"]
```

**Reviewer agent (read-heavy, minimal task interaction):**

```toml
# examples/agents/code-reviewer.toml
[agent]
name        = "code-reviewer"
type        = "engineer"
version     = "1.0.0"
namespace   = "eng"
description = "Reviews PRs and provides feedback"

[process]
type          = "claude"
spawn_command = "claude --model claude-sonnet-4-6"

[skills]
refs = ["code-review"]

[metadata]
model   = "global.anthropic.claude-sonnet-4-6-v1"
timeout = 1800
tags    = ["review"]

[chat]
presence_broadcast = false
message_types      = ["user"]

[tasks]
can_create  = false
can_assign  = false
can_close   = false
auto_accept = true
```

---

## 10. Migration Plan

### 10.1 Phase 1: Schema Additions (Migration 027)

**Commit:** `feat: add chat-task integration schema (migration 027)`

New tables and indexes in a single migration:

```sql
-- Migration 027: chat_task_integration
-- message_cursors
CREATE TABLE IF NOT EXISTS message_cursors (
    room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
    agent_id TEXT NOT NULL,
    last_read_message_id TEXT NOT NULL,
    last_read_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (room_id, agent_id)
);
CREATE INDEX IF NOT EXISTS idx_cursors_agent ON message_cursors(agent_id);

-- notification_queue
CREATE TABLE IF NOT EXISTS notification_queue (
    id TEXT NOT NULL PRIMARY KEY,
    agent_id TEXT NOT NULL,
    room_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    sender_id TEXT NOT NULL,
    priority TEXT NOT NULL DEFAULT 'normal'
        CHECK(priority IN ('low','normal','high','urgent')),
    topics TEXT NOT NULL DEFAULT '[]',
    mentions TEXT NOT NULL DEFAULT '[]',
    delivered INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_notif_queue_agent
    ON notification_queue(agent_id, delivered, created_at);
CREATE INDEX IF NOT EXISTS idx_notif_queue_room
    ON notification_queue(room_id, created_at);

-- Missing indexes on room_messages
CREATE INDEX IF NOT EXISTS idx_room_messages_room_created
    ON room_messages(room_id, created_at);
CREATE INDEX IF NOT EXISTS idx_room_messages_reply_to
    ON room_messages(reply_to) WHERE reply_to IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_room_messages_sender
    ON room_messages(room_id, sender_id, created_at);

-- Add message_type column to room_messages
ALTER TABLE room_messages ADD COLUMN message_type TEXT NOT NULL DEFAULT 'user'
    CHECK(message_type IN ('user','system','task_event','command','handoff'));
```

Files created:
- None (migration is in `crates/nous-core/src/db/pool.rs`, appended to `MIGRATIONS` array)

Files modified:
- `crates/nous-core/src/db/pool.rs` — add migration 027 entry

### 10.2 Phase 2: Core Module Changes

**Commit:** `feat: chat enhancement layer — threading, cursors, message types`

| File | Change |
|------|--------|
| `crates/nous-core/src/messages/mod.rs` | Add `MessageType` enum, `ThreadView`, `ChatCursor` structs. Add `get_thread()`, `mark_read()`, `unread_count()` functions. Update `Message` struct with `message_type` field. Update `from_row` to handle new column. |
| `crates/nous-core/src/notifications/mod.rs` | Add `NotificationPriority` enum. Add `notify_filtered()`, `should_notify_agent()`, `enqueue_notification()`, `dequeue_notification()`, `room_wait_persistent()`. Update `Notification` with priority field. |
| `crates/nous-core/src/rooms/mod.rs` | No changes needed (room model unchanged). |

**Commit:** `feat: task-chat bridge — lifecycle events in rooms`

| File | Change |
|------|--------|
| `crates/nous-core/src/tasks/mod.rs` | Add `post_task_event_to_room()` helper. Add `TaskCommand`, `TaskCommandResult`, `execute_task_command()`. Modify `update_task()` and `create_task()` to accept optional `NotificationRegistry` and call event bridge. |

**Commit:** `feat: agent coordination — handoffs, presence, coordination rooms`

| File | Change |
|------|--------|
| `crates/nous-core/src/agents/coordination.rs` | **NEW FILE.** `AgentMessage`, `AgentMessageKind`, `HandoffPayload`, `PresenceEvent` structs. `post_handoff()`, `broadcast_presence()` functions. |
| `crates/nous-core/src/agents/mod.rs` | Add `pub mod coordination;`. Modify `register_agent()` to auto-create coordination rooms. Modify `heartbeat()` to broadcast presence on status change. |

### 10.3 Phase 3: MCP Tool Additions

**Commit:** `feat: new MCP tools — thread_view, mark_read, task_command, handoff, presence`

| File | Change |
|------|--------|
| `crates/nous-daemon/src/routes/mcp.rs` | Add 6 new `ToolSchema` entries to `get_tool_schemas()`. Add 6 new match arms in `call_tool()`. Update `room_wait` handler for persistent queue integration. Update `room_post_message` handler for `message_type` parameter. |

### 10.4 Phase 4: Agent Form Extensions

**Commit:** `feat: agent form chat/task sections`

| File | Change |
|------|--------|
| `crates/nous-core/src/agents/definition.rs` | Add `ChatSection`, `TaskSection` structs. Add `chat` and `tasks` fields to `AgentDefinition`. |
| `examples/agents/coordinator.toml` | **NEW FILE.** Example manager agent with chat + task sections. |
| `examples/agents/implementer.toml` | **NEW FILE.** Example engineer agent with limited capabilities. |
| `examples/agents/code-reviewer.toml` | **NEW FILE.** Example read-only reviewer agent. |

---

## 11. Testing Strategy

### 11.1 Unit Tests

Located alongside module code, following existing pattern (e.g., `messages/mod.rs:280-564`, `tasks/mod.rs` uses `setup()` with `TempDir` + `DbPools`).

| Module | Tests |
|--------|-------|
| `messages/mod.rs` | `test_get_thread_returns_root_and_replies`, `test_get_thread_empty_replies`, `test_mark_read_advances_cursor`, `test_unread_count_after_new_messages`, `test_message_type_defaults_to_user`, `test_post_system_message` |
| `notifications/mod.rs` | `test_should_notify_agent_mention_always`, `test_should_notify_agent_topic_match`, `test_should_notify_agent_topic_mismatch`, `test_enqueue_dequeue_notification`, `test_room_wait_persistent_checks_queue_first`, `test_notification_priority_derivation` |
| `tasks/mod.rs` | `test_task_event_posted_to_room_on_status_change`, `test_task_event_not_posted_when_no_room`, `test_execute_task_command_close`, `test_execute_task_command_assign`, `test_execute_task_command_invalid` |
| `agents/coordination.rs` | `test_post_handoff_creates_message_with_metadata`, `test_broadcast_presence_posts_to_agent_room`, `test_broadcast_presence_skips_no_room` |
| `agents/definition.rs` | `test_parse_definition_with_chat_section`, `test_parse_definition_with_tasks_section`, `test_parse_definition_minimal_no_chat_tasks` |

### 11.2 Integration Tests

Cross-module tests that verify the coordination layer works end-to-end:

| Test | Modules Under Test | Scenario |
|------|-------------------|----------|
| `test_task_lifecycle_room_projection` | tasks + messages + notifications | Create task with room → update status → verify room has task_event messages |
| `test_agent_handoff_e2e` | agents + messages + notifications | Register manager + engineer → post_handoff → verify engineer receives notification with handoff metadata |
| `test_coordination_room_auto_create` | agents + rooms + notifications | Register parent → register child → verify coordination room exists and both are subscribed |
| `test_persistent_notification_survives_restart` | notifications | Enqueue notification → recreate NotificationRegistry (simulates restart) → verify dequeue returns notification |
| `test_task_command_from_chat` | tasks + messages | Post command message → execute_task_command → verify task updated and room has event message |

### 11.3 End-to-End Scenarios

MCP-level integration tests using the daemon HTTP API:

| Scenario | Steps |
|----------|-------|
| **Manager delegates to engineer** | 1. `agent_register` manager → 2. `agent_register` engineer under manager → 3. `task_create` with room → 4. `agent_handoff` from manager to engineer → 5. Verify: coordination room created, handoff message in room, task assigned |
| **Task lifecycle tracking** | 1. `task_create` with room → 2. `task_update` status to in_progress → 3. `task_update` assign to agent → 4. `task_close` → 5. Verify: 4 task_event messages in room with correct metadata |
| **Notification persistence** | 1. `room_subscribe` with topics → 2. Post messages matching and non-matching topics → 3. `room_wait` with agent_id → 4. Verify: only matching notifications returned |
| **Read cursor workflow** | 1. Post 10 messages → 2. `room_mark_read` at message 5 → 3. `room_unread_count` → 4. Verify: count = 5 → 5. `room_mark_read` at message 10 → 6. Verify: count = 0 |

---

## 12. Open Decisions

### 12.1 Hard FK on assignee_id vs. Soft Validation

**Options:**
- **(A) Hard FK** — `ALTER TABLE tasks ADD CONSTRAINT fk_assignee REFERENCES agents(id)`. Guarantees referential integrity.
- **(B) Soft validation** — Rust-layer check with warning log (proposed in 5.4). Allows tasks to outlive agents.

**Trade-offs:** Hard FK prevents orphaned assignments but breaks if an agent is deregistered while tasks are still open. Soft validation is more resilient to agent lifecycle but allows invalid assignee strings.

**Recommendation:** (B) Soft validation for prototype phase. Add hard FK when agent lifecycle management is mature enough to cascade-update task assignments on deregister.

### 12.2 Threading Model: Flat vs. Nested

**Options:**
- **(A) Flat threading** — `reply_to` points to root message only. Simple, matches current data model.
- **(B) Nested threading** — `reply_to` can point to any message, forming a tree. More expressive but complex to render.

**Trade-offs:** Flat threading is simpler to implement and query. Nested threading supports richer conversations but adds complexity to thread retrieval (recursive CTE).

**Recommendation:** (A) Flat threading. Agent coordination is task-oriented; deeply nested conversations are uncommon. Revisit if user feedback indicates nested threading is needed.

### 12.3 Notification Queue Cleanup Strategy

**Options:**
- **(A) TTL-based** — Auto-delete notifications older than N hours (e.g., 24h).
- **(B) Acknowledgment-based** — Mark as delivered when dequeued, periodic cleanup of delivered records.
- **(C) Capacity-based** — Keep last N notifications per agent, delete oldest.

**Trade-offs:** TTL is simplest but may delete undelivered notifications. Acknowledgment needs explicit cleanup. Capacity bounds storage but may lose important notifications.

**Recommendation:** (B) Acknowledgment-based with a background cleanup job that deletes delivered notifications older than 1 hour and undelivered notifications older than 24 hours.

### 12.4 Coordination Room Naming

**Options:**
- **(A) Agent-name based** — `coord-{manager_name}` (proposed in 6.4). Human-readable.
- **(B) Agent-ID based** — `coord-{manager_id}`. Globally unique, no collisions across namespaces.
- **(C) Namespace-scoped** — `coord-{namespace}-{manager_name}`. Unique per namespace.

**Trade-offs:** Agent names are human-readable but may collide across namespaces. IDs are unique but opaque. Namespace-scoped is a middle ground.

**Recommendation:** (C) Namespace-scoped: `coord-{namespace}-{manager_name}`. Ensures uniqueness while remaining readable.

### 12.5 Agent Form `[chat]` Section: Metadata vs. Top-Level

**Options:**
- **(A) New `[chat]` section** — Top-level section in TOML (proposed in 6.5). Extends `AgentDefinition`.
- **(B) Nested under `[metadata]`** — Add chat/task config as JSON under existing metadata section. No struct changes.

**Trade-offs:** Top-level section is more explicit and validated by serde. Metadata approach is flexible but untyped.

**Recommendation:** (A) Top-level `[chat]` and `[tasks]` sections. The Agent Skills Specification (agentskills.io) favors explicit, typed capability declarations over free-form metadata. This aligns with the existing pattern of `[agent]`, `[process]`, `[skills]`, `[metadata]` as peer sections.

### 12.6 room_wait Upgrade Path

**Options:**
- **(A) New tool** — `room_wait_persistent` as separate tool, keep `room_wait` unchanged.
- **(B) In-place upgrade** — Add `agent_id` parameter to existing `room_wait`. When provided, check persistent queue first.

**Trade-offs:** New tool avoids breaking existing callers. In-place upgrade reduces tool proliferation and is backwards-compatible (agent_id is optional).

**Recommendation:** (B) In-place upgrade. The new parameter is optional, so existing callers are unaffected. This keeps the tool surface clean.

### 12.7 Message Type Enforcement

**Options:**
- **(A) SQL CHECK** — Enforce via column constraint (proposed in 10.1). Invalid types rejected at insert.
- **(B) Rust-only validation** — Validate in `post_message` via `MessageType` enum. SQL column is unconstrained TEXT.

**Trade-offs:** SQL CHECK is the strictest guarantee but requires migration to add new types. Rust-only validation is more flexible but allows direct SQL inserts to bypass validation.

**Recommendation:** (A) SQL CHECK. Follows the existing pattern (status/priority on tasks, action_type on schedules). New message types are infrequent enough that a migration is acceptable.
