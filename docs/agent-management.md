# Design: Agent Management

**Initiative:** INI-062  
**Status:** Draft  
**Author:** Technical Writer (paseo agent)  
**Date:** 2026-04-27

## 1. Overview & Motivation

Nous tracks agent metadata in freeform TEXT fields (`agent_id`, `agent_model`) scattered across three tables: `memories` (`crates/nous-core/src/db.rs:49-50`), `room_participants` (`db.rs:150`), and `room_messages` (`db.rs:160`). These fields have no validation, no foreign key enforcement, no registry. Agent lineage (which agent spawned which) is untracked. Performance metrics (memory counts, session durations, error rates) are unavailable. Debugging agent behavior requires manual log correlation.

This design introduces a proper agent registry with lifecycle tracking, spawn lineage, and performance metrics. The registry integrates with the existing SQLite schema (14 tables, UUIDv7 IDs, WriteChannel batching, ReadPool queries) and provides both MCP tools and CLI commands for agent management.

**Goals:**
- Agent CRUD: register, update, archive agents with metadata (name, type, model, workspace, status)
- Spawn lineage: track parent-child relationships via `parent_id` FK for recursive lineage queries
- Performance metrics: record memory counts, session durations, error rates in time-series format
- Backward compatibility: existing `agent_id` fields in memories and room_participants remain freeform TEXT, no FK enforcement
- MCP tools: 7+ tools for agent operations (register, list, get, update, archive, lineage, metrics)
- CLI commands: `nous agent {register,list,get,update,archive,lineage,metrics}` subcommands

**Non-goals (deferred to Future Work):**
- Agent orchestration: runtime task assignment, priority queues, scheduling
- Agent-to-agent communication: message passing, pub-sub, event streams
- Auto-discovery: scanning logs or process tables to auto-populate the registry
- Dashboard: web UI for agent visualization
- FK enforcement: existing agent_id columns remain TEXT (no ALTER TABLE, no breaking changes)

## 2. Data Model

Three new tables extend the existing 14-table schema at `crates/nous-core/src/db.rs:11-233`.

**`agents`** — agent registry and metadata

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUIDv7 (matches existing `memories.id` pattern from `crates/nous-shared/src/ids.rs:40`) |
| `name` | TEXT | NOT NULL | Human-readable name (e.g., "research-db-crud", "dab8-writer-agent-mgmt") |
| `agent_type` | TEXT | NOT NULL CHECK | Type: `claude_code`, `paseo`, `custom`, `system` — CHECK constraint matches `MemoryType` pattern at `types.rs:10-47` |
| `model` | TEXT | nullable | Model identifier (e.g., "claude/opus", "codex/gpt-5.4") — nullable for system agents |
| `parent_id` | TEXT | FK agents(id) nullable | Lineage: which agent spawned this one — recursive CTE queries supported |
| `workspace_id` | INTEGER | FK workspaces(id) nullable | Directory context — joins to existing `workspaces` table (`db.rs:19-21`) |
| `status` | TEXT | NOT NULL DEFAULT 'active' CHECK | Lifecycle: `active`, `idle`, `archived`, `terminated` — CHECK constraint |
| `metadata` | TEXT | nullable | JSON blob: capabilities, config, labels — TEXT storage matches existing `rooms.metadata` pattern (`db.rs:139`) |
| `created_at` | TEXT | NOT NULL DEFAULT now | ISO8601 via `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` — matches existing schema convention (`db.rs:26,34,56`) |
| `updated_at` | TEXT | NOT NULL DEFAULT now | ISO8601 — updated on status changes, metadata edits |

**`agent_sessions`** — activity windows for agents

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | TEXT | PRIMARY KEY | UUIDv7 |
| `agent_id` | TEXT | FK agents(id) CASCADE | Which agent — CASCADE matches existing FK pattern (`db.rs:150,159`) |
| `session_id` | TEXT | nullable | Links to `memories.session_id` (`db.rs:54`) for cross-referencing — nullable for sessions without memories |
| `started_at` | TEXT | NOT NULL DEFAULT now | Session start timestamp |
| `ended_at` | TEXT | nullable | Session end timestamp — NULL means session is active |
| `memory_count` | INTEGER | NOT NULL DEFAULT 0 | Memories created in this session — updated via WriteChannel |
| `metadata` | TEXT | nullable | JSON: tools used, tokens consumed, errors — extensible |

**`agent_metrics`** — performance tracking (time-series)

| Column | Type | Constraints | Description |
|--------|------|-------------|-------------|
| `id` | INTEGER | PRIMARY KEY AUTO | Row ID — INTEGER AUTOINCREMENT matches existing dictionary tables (`db.rs:14,19,28,61`) |
| `agent_id` | TEXT | FK agents(id) CASCADE | Which agent |
| `metric_type` | TEXT | NOT NULL CHECK | Type: `memory_created`, `session_duration_ms`, `error_count`, `tool_call_count` — CHECK constraint |
| `value` | REAL | NOT NULL | Metric value — REAL for floating-point durations, counters |
| `recorded_at` | TEXT | NOT NULL DEFAULT now | When recorded — ISO8601 |

**Indexes:**

- `idx_agents_name ON agents(name)` — name lookup for CLI/MCP tools
- `idx_agents_parent ON agents(parent_id)` — lineage queries (recursive CTEs)
- `idx_agents_status ON agents(status)` — filter by lifecycle (active vs. archived)
- `idx_agents_workspace ON agents(workspace_id)` — workspace-scoped queries
- `idx_agent_sessions_agent ON agent_sessions(agent_id, started_at DESC)` — session history
- `idx_agent_metrics_agent ON agent_metrics(agent_id, recorded_at DESC)` — time-series queries

**Relationship to existing schema:**

- `memories.agent_id` (`db.rs:49`) becomes a soft FK to `agents.id` — no DDL change, optional JOIN capability
- `room_participants.agent_id` (`db.rs:150`) similarly joins to `agents.id` for participant metadata
- `agent_sessions.session_id` joins to `memories.session_id` (`db.rs:54`) for cross-referencing
- `agents.workspace_id` FK to `workspaces(id)` (`db.rs:19`) for directory scoping

**Why no FTS5 for agents:**

Agent names are short (10-50 chars), full-text search adds no value. Simple `LIKE` queries suffice for name filtering.

**Why no vec0 for agents:**

No semantic search use case. Agents are retrieved by exact ID, name prefix, workspace, or lineage — all covered by indexes.

## 3. Schema DDL

Copy-pasteable SQL for appending to the `MIGRATIONS` array at `crates/nous-core/src/db.rs:11-233` (currently 44 statements, will become 53).

**Append position:** After index 43 (line 233).

```sql
-- agents table
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    agent_type TEXT NOT NULL CHECK(agent_type IN ('claude_code','paseo','custom','system')),
    model TEXT,
    parent_id TEXT REFERENCES agents(id),
    workspace_id INTEGER REFERENCES workspaces(id),
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','idle','archived','terminated')),
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)

-- agent_sessions table
CREATE TABLE IF NOT EXISTS agent_sessions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    session_id TEXT,
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ended_at TEXT,
    memory_count INTEGER NOT NULL DEFAULT 0,
    metadata TEXT
)

-- agent_metrics table
CREATE TABLE IF NOT EXISTS agent_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    metric_type TEXT NOT NULL CHECK(metric_type IN ('memory_created','session_duration_ms','error_count','tool_call_count')),
    value REAL NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
)

-- Index: agents by name
CREATE INDEX IF NOT EXISTS idx_agents_name ON agents(name)

-- Index: agents by parent_id (lineage queries)
CREATE INDEX IF NOT EXISTS idx_agents_parent ON agents(parent_id)

-- Index: agents by status (lifecycle filtering)
CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status)

-- Index: agents by workspace_id
CREATE INDEX IF NOT EXISTS idx_agents_workspace ON agents(workspace_id)

-- Index: agent_sessions by agent_id + started_at DESC
CREATE INDEX IF NOT EXISTS idx_agent_sessions_agent ON agent_sessions(agent_id, started_at DESC)

-- Index: agent_metrics by agent_id + recorded_at DESC
CREATE INDEX IF NOT EXISTS idx_agent_metrics_agent ON agent_metrics(agent_id, recorded_at DESC)
```

**Migration order:** These 9 statements depend on existing `workspaces` table (created in migration 2 at `db.rs:19-21`). The `agents.parent_id` self-FK is valid because SQLite allows deferred constraint checking.

**Migration execution:** The `run_migrations()` function at `crates/nous-shared/src/sqlite.rs` executes all statements in a single transaction. On successful schema creation, a `schema_version` pragma records the last applied migration.

**Backward compatibility:** Old Nous binaries (without agent schema) will continue to work with databases that already have the agent tables. The `CREATE TABLE IF NOT EXISTS` guards prevent errors.

**Forward compatibility:** New Nous binaries will detect missing agent tables on first open and create them via migrations.

## 4. WriteChannel Extensions

Add five new `WriteOp` enum variants to `crates/nous-core/src/channel.rs:18-51`.

**Enum variants:**

```rust
pub enum WriteOp {
    // ... existing variants (Store, Update, Forget, Relate, Unrelate, Unarchive,
    //     CategorySuggest, StoreChunks, DeleteChunks, LogAccess, CreateRoom,
    //     PostMessage, DeleteRoom, ArchiveRoom) ...
    
    RegisterAgent {
        id: String,              // UUIDv7
        name: String,
        agent_type: String,      // Parsed to AgentType enum in handler
        model: Option<String>,
        parent_id: Option<String>,
        workspace_id: Option<i64>,
        metadata: Option<String>,  // JSON
        resp: oneshot::Sender<Result<String>>,
    },
    UpdateAgent {
        id: String,
        patch: AgentPatch,       // Struct with optional fields for partial update
        resp: oneshot::Sender<Result<bool>>,
    },
    ArchiveAgent(String, oneshot::Sender<Result<bool>>),  // (id, resp)
    RecordSession {
        id: String,              // UUIDv7
        agent_id: String,
        session_id: Option<String>,
        started_at: Option<String>,
        ended_at: Option<String>,
        memory_count: i64,
        metadata: Option<String>,
        resp: oneshot::Sender<Result<String>>,
    },
    RecordMetric {
        agent_id: String,
        metric_type: String,     // Parsed to MetricType enum in handler
        value: f64,
        resp: oneshot::Sender<Result<()>>,
    },
}
```

**WriteChannel methods** (add to `crates/nous-core/src/channel.rs:59-203`):

```rust
impl WriteChannel {
    pub async fn register_agent(
        &self,
        id: String,
        name: String,
        agent_type: String,
        model: Option<String>,
        parent_id: Option<String>,
        workspace_id: Option<i64>,
        metadata: Option<String>,
    ) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::RegisterAgent {
            id, name, agent_type, model, parent_id, workspace_id, metadata, resp: resp_tx
        }).await.map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await.map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn update_agent(&self, id: String, patch: AgentPatch) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::UpdateAgent { id, patch, resp: resp_tx })
            .await.map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await.map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn archive_agent(&self, id: String) -> Result<bool> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::ArchiveAgent(id, resp_tx))
            .await.map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await.map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn record_session(
        &self,
        id: String,
        agent_id: String,
        session_id: Option<String>,
        started_at: Option<String>,
        ended_at: Option<String>,
        memory_count: i64,
        metadata: Option<String>,
    ) -> Result<String> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::RecordSession {
            id, agent_id, session_id, started_at, ended_at, memory_count, metadata, resp: resp_tx
        }).await.map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await.map_err(|_| NousError::Internal("response channel dropped".into()))?
    }

    pub async fn record_metric(
        &self,
        agent_id: String,
        metric_type: String,
        value: f64,
    ) -> Result<()> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx.send(WriteOp::RecordMetric { agent_id, metric_type, value, resp: resp_tx })
            .await.map_err(|_| NousError::Internal("write channel closed".into()))?;
        resp_rx.await.map_err(|_| NousError::Internal("response channel dropped".into()))?
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
            // ... existing match arms ...
            
            WriteOp::RegisterAgent { id, name, agent_type, model, parent_id, workspace_id, metadata, resp } => {
                let result = MemoryDb::register_agent_on(&tx, &id, &name, &agent_type,
                    model.as_deref(), parent_id.as_deref(), workspace_id, metadata.as_deref());
                let _ = resp.send(result.map(|_| id.clone()));
            }
            WriteOp::UpdateAgent { id, patch, resp } => {
                let result = MemoryDb::update_agent_on(&tx, &id, &patch);
                let _ = resp.send(result);
            }
            WriteOp::ArchiveAgent(id, resp) => {
                let result = MemoryDb::archive_agent_on(&tx, &id);
                let _ = resp.send(result);
            }
            WriteOp::RecordSession { id, agent_id, session_id, started_at, ended_at, memory_count, metadata, resp } => {
                let result = MemoryDb::record_session_on(&tx, &id, &agent_id,
                    session_id.as_deref(), started_at.as_deref(), ended_at.as_deref(), memory_count, metadata.as_deref());
                let _ = resp.send(result.map(|_| id.clone()));
            }
            WriteOp::RecordMetric { agent_id, metric_type, value, resp } => {
                let result = MemoryDb::record_metric_on(&tx, &agent_id, &metric_type, value);
                let _ = resp.send(result);
            }
        }
    }
}
```

**MemoryDb methods** (add to `crates/nous-core/src/db.rs`, in the `impl MemoryDb` block after existing write methods (after line 650)):

```rust
impl MemoryDb {
    pub(crate) fn register_agent_on(
        conn: &Connection,
        id: &str,
        name: &str,
        agent_type: &str,
        model: Option<&str>,
        parent_id: Option<&str>,
        workspace_id: Option<i64>,
        metadata: Option<&str>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO agents (id, name, agent_type, model, parent_id, workspace_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, name, agent_type, model, parent_id, workspace_id, metadata],
        )?;
        Ok(())
    }

    pub(crate) fn update_agent_on(conn: &Connection, id: &str, patch: &AgentPatch) -> Result<bool> {
        // Dynamic SQL builder pattern matches MemoryDb::update_on at db.rs:501-536
        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        
        if let Some(ref status) = patch.status {
            updates.push("status = ?");
            params.push(Box::new(status.clone()));
        }
        if let Some(ref metadata) = patch.metadata {
            updates.push("metadata = ?");
            params.push(Box::new(metadata.clone()));
        }
        updates.push("updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')");
        
        params.push(Box::new(id.to_string()));
        
        let sql = format!("UPDATE agents SET {} WHERE id = ?", updates.join(", "));
        let rows = conn.execute(&sql, rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())))?;
        Ok(rows > 0)
    }

    pub(crate) fn archive_agent_on(conn: &Connection, id: &str) -> Result<bool> {
        let rows = conn.execute(
            "UPDATE agents SET status = 'archived', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            params![id]
        )?;
        Ok(rows > 0)
    }

    pub(crate) fn record_session_on(
        conn: &Connection,
        id: &str,
        agent_id: &str,
        session_id: Option<&str>,
        started_at: Option<&str>,
        ended_at: Option<&str>,
        memory_count: i64,
        metadata: Option<&str>,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO agent_sessions (id, agent_id, session_id, started_at, ended_at, memory_count, metadata)
             VALUES (?1, ?2, ?3, COALESCE(?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?5, ?6, ?7)",
            params![id, agent_id, session_id, started_at, ended_at, memory_count, metadata],
        )?;
        Ok(())
    }

    pub(crate) fn record_metric_on(
        conn: &Connection,
        agent_id: &str,
        metric_type: &str,
        value: f64,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO agent_metrics (agent_id, metric_type, value) VALUES (?1, ?2, ?3)",
            params![agent_id, metric_type, value],
        )?;
        Ok(())
    }
}
```

**Batching behavior:** Agent operations batch alongside memory and room operations using the same `BATCH_LIMIT=32` and `CHANNEL_CAPACITY=256` from `channel.rs:15-16`. A batch may contain a mix of `Store`, `RegisterAgent`, `PostMessage`, `RecordMetric`. All commit atomically in a single transaction.

## 5. ReadPool Extensions

Add query methods to `crates/nous-core/src/channel.rs:383+` or `crates/nous-core/src/db.rs` (pattern matches `search_memories`, `get_workspaces`).

**New types** (add to `crates/nous-core/src/types.rs:128-434`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub parent_id: Option<String>,
    pub workspace_id: Option<i64>,
    pub status: String,
    pub metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPatch {
    pub status: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub memory_count: i64,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetric {
    pub id: i64,
    pub agent_id: String,
    pub metric_type: String,
    pub value: f64,
    pub recorded_at: String,
}
```

**ReadPool methods:**

```rust
impl ReadPool {
    pub async fn list_agents(
        &self,
        status: Option<&str>,
        workspace_id: Option<i64>,
        limit: Option<usize>,
    ) -> Result<Vec<Agent>> {
        let status = status.map(String::from);
        let limit = limit.unwrap_or(100);
        self.with_conn(move |conn| {
            let mut sql = "SELECT id, name, agent_type, model, parent_id, workspace_id, status, metadata, created_at, updated_at
                           FROM agents WHERE 1=1".to_string();
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
            
            if let Some(s) = status {
                sql.push_str(" AND status = ?");
                params.push(Box::new(s));
            }
            if let Some(ws) = workspace_id {
                sql.push_str(" AND workspace_id = ?");
                params.push(Box::new(ws));
            }
            
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params.push(Box::new(limit));
            
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| {
                    Ok(Agent {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        agent_type: row.get(2)?,
                        model: row.get(3)?,
                        parent_id: row.get(4)?,
                        workspace_id: row.get(5)?,
                        status: row.get(6)?,
                        metadata: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                }
            )?;
            rows.collect()
        }).await
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<Agent>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            conn.query_row(
                "SELECT id, name, agent_type, model, parent_id, workspace_id, status, metadata, created_at, updated_at
                 FROM agents WHERE id = ?1",
                params![id],
                |row| {
                    Ok(Agent {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        agent_type: row.get(2)?,
                        model: row.get(3)?,
                        parent_id: row.get(4)?,
                        workspace_id: row.get(5)?,
                        status: row.get(6)?,
                        metadata: row.get(7)?,
                        created_at: row.get(8)?,
                        updated_at: row.get(9)?,
                    })
                },
            ).optional()
        }).await
    }

    pub async fn get_agent_lineage(&self, id: &str) -> Result<Vec<Agent>> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            // Recursive CTE: walk parent_id chain from target agent to root
            let mut stmt = conn.prepare(
                "WITH RECURSIVE lineage(id, name, agent_type, model, parent_id, workspace_id, status, metadata, created_at, updated_at, depth) AS (
                    SELECT id, name, agent_type, model, parent_id, workspace_id, status, metadata, created_at, updated_at, 0
                    FROM agents WHERE id = ?1
                    UNION ALL
                    SELECT a.id, a.name, a.agent_type, a.model, a.parent_id, a.workspace_id, a.status, a.metadata, a.created_at, a.updated_at, l.depth + 1
                    FROM agents a JOIN lineage l ON a.id = l.parent_id
                )
                SELECT id, name, agent_type, model, parent_id, workspace_id, status, metadata, created_at, updated_at
                FROM lineage ORDER BY depth DESC"
            )?;
            let rows = stmt.query_map(params![id], |row| {
                Ok(Agent {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    agent_type: row.get(2)?,
                    model: row.get(3)?,
                    parent_id: row.get(4)?,
                    workspace_id: row.get(5)?,
                    status: row.get(6)?,
                    metadata: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            })?;
            rows.collect()
        }).await
    }

    pub async fn get_agent_sessions(
        &self,
        agent_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<AgentSession>> {
        let agent_id = agent_id.to_string();
        let limit = limit.unwrap_or(50);
        self.with_conn(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT id, agent_id, session_id, started_at, ended_at, memory_count, metadata
                 FROM agent_sessions
                 WHERE agent_id = ?1
                 ORDER BY started_at DESC
                 LIMIT ?2"
            )?;
            let rows = stmt.query_map(params![agent_id, limit], |row| {
                Ok(AgentSession {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    session_id: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    memory_count: row.get(5)?,
                    metadata: row.get(6)?,
                })
            })?;
            rows.collect()
        }).await
    }

    pub async fn get_agent_metrics(
        &self,
        agent_id: &str,
        metric_type: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<AgentMetric>> {
        let agent_id = agent_id.to_string();
        let metric_type = metric_type.map(String::from);
        let limit = limit.unwrap_or(100);
        self.with_conn(move |conn| {
            let mut sql = "SELECT id, agent_id, metric_type, value, recorded_at
                           FROM agent_metrics
                           WHERE agent_id = ?1".to_string();
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(agent_id)];
            
            if let Some(mt) = metric_type {
                sql.push_str(" AND metric_type = ?");
                params.push(Box::new(mt));
            }
            
            sql.push_str(" ORDER BY recorded_at DESC LIMIT ?");
            params.push(Box::new(limit));
            
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                |row| {
                    Ok(AgentMetric {
                        id: row.get(0)?,
                        agent_id: row.get(1)?,
                        metric_type: row.get(2)?,
                        value: row.get(3)?,
                        recorded_at: row.get(4)?,
                    })
                }
            )?;
            rows.collect()
        }).await
    }
}
```

**ReadPool::with_conn pattern:** Uses semaphore-gated pool of read connections (`channel.rs:383+`). Queries run on dedicated read connections, isolated from write transactions.

## 6. MCP Tool Specifications

Seven new MCP tools following the registration pattern at `crates/nous-mcp/src/server.rs:63-199`. Adds to existing 40+ tools across memory, category, workspace, tag, room, schedule, trace domains.

---

### `agent_register`

Register a new agent in the registry.

**Params struct** (`crates/nous-mcp/src/tools.rs`):

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `name` | String | Yes | Human-readable agent name (e.g., "dab8-writer-agent-mgmt") |
| `agent_type` | String | Yes | Type: "claude_code", "paseo", "custom", "system" — parsed to enum in handler |
| `model` | String | No | Model identifier (e.g., "claude/opus", "codex/gpt-5.4") |
| `parent_id` | String | No | Parent agent ID for lineage tracking |
| `workspace_id` | i64 | No | Workspace foreign key |
| `metadata` | String | No | JSON blob: capabilities, config, labels |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentRegisterParams {
    pub name: String,
    pub agent_type: String,
    pub model: Option<String>,
    pub parent_id: Option<String>,
    pub workspace_id: Option<i64>,
    pub metadata: Option<String>,
}
```

**Handler logic:**
1. Parse `agent_type` via `parse_enum::<AgentType>()` (matches `parse_enum` pattern at `tools.rs:285+`)
2. Generate UUIDv7 via `MemoryId::new()` (`ids.rs:40`)
3. Call `write_channel.register_agent()`
4. Return JSON with `id`, `name`, `created_at`

**Return JSON:**
```json
{
  "id": "018f3a5e-...",
  "name": "dab8-writer-agent-mgmt",
  "agent_type": "paseo",
  "created_at": "2026-04-27T04:30:00.123Z"
}
```

**Registration** (`server.rs`):
```rust
#[tool(name = "agent_register", description = "Register a new agent in the registry")]
async fn agent_register(&self, params: Parameters<AgentRegisterParams>) -> CallToolResult {
    handle_agent_register(params.0, &self.write_channel).await
}
```

---

### `agent_list`

List agents with optional filters.

**Params struct:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `status` | String | No | Filter by status: "active", "idle", "archived", "terminated" |
| `workspace_id` | i64 | No | Filter by workspace |
| `limit` | usize | No | Max results (default 100) |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentListParams {
    pub status: Option<String>,
    pub workspace_id: Option<i64>,
    pub limit: Option<usize>,
}
```

**Handler logic:**
1. Call `read_pool.list_agents(status, workspace_id, limit)`
2. Return JSON array of agents

**Return JSON:**
```json
{
  "agents": [
    {
      "id": "018f3a5e-...",
      "name": "dab8-writer-agent-mgmt",
      "agent_type": "paseo",
      "model": "claude/opus",
      "parent_id": "018f3a5d-...",
      "workspace_id": 5,
      "status": "active",
      "metadata": "{\"capabilities\":[\"write\",\"review\"]}",
      "created_at": "2026-04-27T04:30:00.123Z",
      "updated_at": "2026-04-27T04:30:00.123Z"
    }
  ]
}
```

---

### `agent_get`

Get a single agent by ID.

**Params struct:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `id` | String | Yes | Agent UUIDv7 |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentGetParams {
    pub id: String,
}
```

**Handler logic:**
1. Call `read_pool.get_agent(id)`
2. Return agent JSON or `{"error": "not found"}`

**Return JSON:**
```json
{
  "id": "018f3a5e-...",
  "name": "dab8-writer-agent-mgmt",
  "agent_type": "paseo",
  "model": "claude/opus",
  "parent_id": "018f3a5d-...",
  "workspace_id": 5,
  "status": "active",
  "metadata": "{\"capabilities\":[\"write\",\"review\"]}",
  "created_at": "2026-04-27T04:30:00.123Z",
  "updated_at": "2026-04-27T04:30:00.123Z"
}
```

---

### `agent_update`

Update agent metadata or status.

**Params struct:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `id` | String | Yes | Agent UUIDv7 |
| `status` | String | No | New status: "active", "idle", "archived", "terminated" |
| `metadata` | String | No | New metadata JSON (overwrites existing) |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentUpdateParams {
    pub id: String,
    pub status: Option<String>,
    pub metadata: Option<String>,
}
```

**Handler logic:**
1. Build `AgentPatch` from params
2. Call `write_channel.update_agent(id, patch)`
3. Return `{"success": true, "updated": true/false}`

**Return JSON:**
```json
{
  "success": true,
  "updated": true
}
```

---

### `agent_archive`

Archive an agent (sets status to "archived").

**Params struct:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `id` | String | Yes | Agent UUIDv7 |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentArchiveParams {
    pub id: String,
}
```

**Handler logic:**
1. Call `write_channel.archive_agent(id)`
2. Return `{"success": true, "archived": true/false}`

**Return JSON:**
```json
{
  "success": true,
  "archived": true
}
```

---

### `agent_lineage`

Get agent lineage (recursive parent chain).

**Params struct:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `id` | String | Yes | Agent UUIDv7 |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentLineageParams {
    pub id: String,
}
```

**Handler logic:**
1. Call `read_pool.get_agent_lineage(id)` — recursive CTE query
2. Return ordered list (root → target agent)

**Return JSON:**
```json
{
  "lineage": [
    {
      "id": "018f3a5c-...",
      "name": "root-manager",
      "agent_type": "paseo",
      "parent_id": null,
      "depth": 2
    },
    {
      "id": "018f3a5d-...",
      "name": "senior-manager",
      "agent_type": "paseo",
      "parent_id": "018f3a5c-...",
      "depth": 1
    },
    {
      "id": "018f3a5e-...",
      "name": "dab8-writer-agent-mgmt",
      "agent_type": "paseo",
      "parent_id": "018f3a5d-...",
      "depth": 0
    }
  ]
}
```

---

### `agent_metrics`

Get time-series metrics for an agent.

**Params struct:**

| Name | Type | Required | Description |
|------|------|----------|-------------|
| `agent_id` | String | Yes | Agent UUIDv7 |
| `metric_type` | String | No | Filter: "memory_created", "session_duration_ms", "error_count", "tool_call_count" |
| `limit` | usize | No | Max results (default 100) |

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentMetricsParams {
    pub agent_id: String,
    pub metric_type: Option<String>,
    pub limit: Option<usize>,
}
```

**Handler logic:**
1. Call `read_pool.get_agent_metrics(agent_id, metric_type, limit)`
2. Return JSON array ordered by `recorded_at DESC`

**Return JSON:**
```json
{
  "metrics": [
    {
      "id": 42,
      "agent_id": "018f3a5e-...",
      "metric_type": "memory_created",
      "value": 15.0,
      "recorded_at": "2026-04-27T04:35:00.456Z"
    },
    {
      "id": 41,
      "agent_id": "018f3a5e-...",
      "metric_type": "session_duration_ms",
      "value": 3240.5,
      "recorded_at": "2026-04-27T04:34:00.123Z"
    }
  ]
}
```

---

**Tool registration pattern** (add to `server.rs` impl block):

```rust
#[tool_router]
impl NousServer {
    // ... existing tools (memory_store, room_create, schedule_create, etc.) ...

    #[tool(name = "agent_register", description = "Register a new agent in the registry")]
    async fn agent_register(&self, params: Parameters<AgentRegisterParams>) -> CallToolResult {
        handle_agent_register(params.0, &self.write_channel).await
    }

    #[tool(name = "agent_list", description = "List agents with optional filters")]
    async fn agent_list(&self, params: Parameters<AgentListParams>) -> CallToolResult {
        handle_agent_list(params.0, &self.read_pool).await
    }

    #[tool(name = "agent_get", description = "Get agent by ID")]
    async fn agent_get(&self, params: Parameters<AgentGetParams>) -> CallToolResult {
        handle_agent_get(params.0, &self.read_pool).await
    }

    #[tool(name = "agent_update", description = "Update agent metadata or status")]
    async fn agent_update(&self, params: Parameters<AgentUpdateParams>) -> CallToolResult {
        handle_agent_update(params.0, &self.write_channel).await
    }

    #[tool(name = "agent_archive", description = "Archive an agent")]
    async fn agent_archive(&self, params: Parameters<AgentArchiveParams>) -> CallToolResult {
        handle_agent_archive(params.0, &self.write_channel).await
    }

    #[tool(name = "agent_lineage", description = "Get agent spawn lineage")]
    async fn agent_lineage(&self, params: Parameters<AgentLineageParams>) -> CallToolResult {
        handle_agent_lineage(params.0, &self.read_pool).await
    }

    #[tool(name = "agent_metrics", description = "Get agent performance metrics")]
    async fn agent_metrics(&self, params: Parameters<AgentMetricsParams>) -> CallToolResult {
        handle_agent_metrics(params.0, &self.read_pool).await
    }
}
```

## 7. CLI Command Specifications

Add `agent` subcommand to `crates/nous-mcp/src/main.rs` following the `room` pattern at `main.rs:48-82`.

**Command enum variant** (add to `Command` at `main.rs:26`):

```rust
Agent(AgentCmd),
```

**Subcommand structs:**

```rust
#[derive(Debug, Parser)]
struct AgentCmd {
    #[command(subcommand)]
    command: AgentSubcommand,
}

#[derive(Debug, Subcommand)]
enum AgentSubcommand {
    Register {
        name: String,
        #[arg(long)]
        agent_type: String,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long)]
        workspace_id: Option<i64>,
        #[arg(long)]
        metadata: Option<String>,
    },
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        workspace_id: Option<i64>,
        #[arg(long)]
        limit: Option<usize>,
    },
    Get {
        id: String,
    },
    Update {
        id: String,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        metadata: Option<String>,
    },
    Archive {
        id: String,
    },
    Lineage {
        id: String,
    },
    Metrics {
        agent_id: String,
        #[arg(long)]
        metric_type: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
}
```

**Match arm** (add to `main()` at `main.rs:108`):

```rust
Command::Agent(cmd) => match cmd.command {
    AgentSubcommand::Register { name, agent_type, model, parent_id, workspace_id, metadata } => {
        commands::run_agent_register(&config, &name, &agent_type, model.as_deref(),
            parent_id.as_deref(), workspace_id, metadata.as_deref())?
    }
    AgentSubcommand::List { status, workspace_id, limit } => {
        commands::run_agent_list(&config, status.as_deref(), workspace_id, limit)?
    }
    AgentSubcommand::Get { id } => {
        commands::run_agent_get(&config, &id)?
    }
    AgentSubcommand::Update { id, status, metadata } => {
        commands::run_agent_update(&config, &id, status.as_deref(), metadata.as_deref())?
    }
    AgentSubcommand::Archive { id } => {
        commands::run_agent_archive(&config, &id)?
    }
    AgentSubcommand::Lineage { id } => {
        commands::run_agent_lineage(&config, &id)?
    }
    AgentSubcommand::Metrics { agent_id, metric_type, limit } => {
        commands::run_agent_metrics(&config, &agent_id, metric_type.as_deref(), limit)?
    }
}
```

**Handler implementations** (add to `crates/nous-mcp/src/commands/agent.rs`):

```rust
use crate::config::Config;
use nous_core::{MemoryDb, ReadPool, WriteChannel, types::{Agent, AgentPatch}};
use nous_shared::ids::MemoryId;

pub fn run_agent_register(
    config: &Config,
    name: &str,
    agent_type: &str,
    model: Option<&str>,
    parent_id: Option<&str>,
    workspace_id: Option<i64>,
    metadata: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = MemoryDb::open(&config.memory.db_path, None, 384)?;
    let (write_channel, _handle) = WriteChannel::new(db);
    let rt = tokio::runtime::Runtime::new()?;
    let id = MemoryId::new().to_string();
    rt.block_on(write_channel.register_agent(
        id.clone(),
        name.to_string(),
        agent_type.to_string(),
        model.map(String::from),
        parent_id.map(String::from),
        workspace_id,
        metadata.map(String::from),
    ))?;
    println!("Registered agent: {} ({})", name, id);
    Ok(())
}

pub fn run_agent_list(
    config: &Config,
    status: Option<&str>,
    workspace_id: Option<i64>,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let read_pool = ReadPool::new(&config.memory.db_path, None, 1)?;
    let rt = tokio::runtime::Runtime::new()?;
    let agents = rt.block_on(read_pool.list_agents(status, workspace_id, limit))?;
    
    // Human-readable table output
    println!("{:<38} | {:<30} | {:<15} | {:<10}",
        "ID", "NAME", "TYPE", "STATUS");
    println!("{}", "-".repeat(110));
    for agent in agents {
        println!("{:<38} | {:<30} | {:<15} | {:<10}",
            agent.id, agent.name, agent.agent_type, agent.status);
    }
    Ok(())
}

pub fn run_agent_get(
    config: &Config,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let read_pool = ReadPool::new(&config.memory.db_path, None, 1)?;
    let rt = tokio::runtime::Runtime::new()?;
    if let Some(agent) = rt.block_on(read_pool.get_agent(id))? {
        println!("ID:          {}", agent.id);
        println!("Name:        {}", agent.name);
        println!("Type:        {}", agent.agent_type);
        println!("Model:       {}", agent.model.as_deref().unwrap_or("N/A"));
        println!("Parent ID:   {}", agent.parent_id.as_deref().unwrap_or("N/A"));
        println!("Workspace:   {}", agent.workspace_id.map_or("N/A".to_string(), |w| w.to_string()));
        println!("Status:      {}", agent.status);
        println!("Metadata:    {}", agent.metadata.as_deref().unwrap_or("N/A"));
        println!("Created:     {}", agent.created_at);
        println!("Updated:     {}", agent.updated_at);
    } else {
        eprintln!("Agent not found: {}", id);
    }
    Ok(())
}

pub fn run_agent_update(
    config: &Config,
    id: &str,
    status: Option<&str>,
    metadata: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = MemoryDb::open(&config.memory.db_path, None, 384)?;
    let (write_channel, _handle) = WriteChannel::new(db);
    let rt = tokio::runtime::Runtime::new()?;
    let patch = AgentPatch {
        status: status.map(String::from),
        metadata: metadata.map(String::from),
    };
    let updated = rt.block_on(write_channel.update_agent(id.to_string(), patch))?;
    if updated {
        println!("Updated agent: {}", id);
    } else {
        eprintln!("Agent not found: {}", id);
    }
    Ok(())
}

pub fn run_agent_archive(
    config: &Config,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let db = MemoryDb::open(&config.memory.db_path, None, 384)?;
    let (write_channel, _handle) = WriteChannel::new(db);
    let rt = tokio::runtime::Runtime::new()?;
    let archived = rt.block_on(write_channel.archive_agent(id.to_string()))?;
    if archived {
        println!("Archived agent: {}", id);
    } else {
        eprintln!("Agent not found: {}", id);
    }
    Ok(())
}

pub fn run_agent_lineage(
    config: &Config,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let read_pool = ReadPool::new(&config.memory.db_path, None, 1)?;
    let rt = tokio::runtime::Runtime::new()?;
    let lineage = rt.block_on(read_pool.get_agent_lineage(id))?;
    
    println!("Lineage for agent {}:", id);
    println!("{:<38} | {:<30} | {:<15} | {:<38}",
        "ID", "NAME", "TYPE", "PARENT_ID");
    println!("{}", "-".repeat(140));
    for agent in lineage {
        println!("{:<38} | {:<30} | {:<15} | {:<38}",
            agent.id, agent.name, agent.agent_type,
            agent.parent_id.as_deref().unwrap_or("(root)"));
    }
    Ok(())
}

pub fn run_agent_metrics(
    config: &Config,
    agent_id: &str,
    metric_type: Option<&str>,
    limit: Option<usize>,
) -> Result<(), Box<dyn std::error::Error>> {
    let read_pool = ReadPool::new(&config.memory.db_path, None, 1)?;
    let rt = tokio::runtime::Runtime::new()?;
    let metrics = rt.block_on(read_pool.get_agent_metrics(agent_id, metric_type, limit))?;
    
    println!("{:<10} | {:<30} | {:<15} | {:<30}",
        "ID", "METRIC_TYPE", "VALUE", "RECORDED_AT");
    println!("{}", "-".repeat(90));
    for metric in metrics {
        println!("{:<10} | {:<30} | {:<15.2} | {:<30}",
            metric.id, metric.metric_type, metric.value, metric.recorded_at);
    }
    Ok(())
}
```

**Module registration** (add to `crates/nous-mcp/src/commands/mod.rs`):

```rust
pub mod agent;
```

**Usage examples:**

```bash
# Register a new agent
nous agent register dab8-writer-agent-mgmt --agent-type paseo --model claude/opus --parent-id 018f3a5d-...

# List all active agents
nous agent list --status active

# List agents in workspace 5
nous agent list --workspace-id 5 --limit 50

# Get agent details
nous agent get 018f3a5e-...

# Update agent status
nous agent update 018f3a5e-... --status idle

# Archive agent
nous agent archive 018f3a5e-...

# Get agent lineage
nous agent lineage 018f3a5e-...

# Get agent metrics (all types)
nous agent metrics 018f3a5e-... --limit 100

# Get specific metric type
nous agent metrics 018f3a5e-... --metric-type memory_created --limit 20
```

**Output format support:**

Handlers use the existing `--format` flag from `main.rs:31`:

- `--format human` — table output (default, shown above)
- `--format json` — JSON array via `print_json()` (`commands/mod.rs:42-58`)
- `--format csv` — CSV via `print_csv()` (`commands/mod.rs:60-151`)

## 8. Config Additions

No new config sections required for MVP. Agent management uses the existing `[memory]` config for database path and connection pooling.

**Future consideration** (deferred to Phase 2):

If enforcement of agent limits or auto-cleanup policies is added, introduce an `[agents]` config section:

```toml
[agents]
max_agents = 10000
retention_days = 180  # Archive agents inactive for 180+ days
```

Add to `crates/nous-mcp/src/config.rs`:

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct AgentsConfig {
    pub max_agents: usize,
    pub retention_days: usize,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            max_agents: 10000,
            retention_days: 180,
        }
    }
}
```

This structure is ready for future use but omitted from MVP to avoid unused code.

## 9. Migration Strategy

The `MIGRATIONS` array at `crates/nous-core/src/db.rs:11-233` currently contains 44 SQL statements. Append the 9 new statements from Section 3 to this array.

**Current migration count:** 44  
**New migration count:** 53  
**Insertion point:** After line 233 (after the last existing index creation)

**Order matters:** The new statements must come after the existing schema because:
1. `agents.workspace_id` FK references `workspaces(id)` (created in migration 2 at `db.rs:19-21`)
2. `agent_sessions.agent_id` and `agent_metrics.agent_id` FK reference `agents(id)` (created in first new migration)
3. Indexes reference the tables they cover

**Migration execution:** The `run_migrations()` function at `crates/nous-shared/src/sqlite.rs` executes all statements in a single transaction. If any statement fails, the entire transaction rolls back. On successful schema creation, a `schema_version` pragma records the last applied migration.

**Backward compatibility:**

| Scenario | Behavior |
|----------|----------|
| Old Nous binary + new database (53 migrations) | Works. `CREATE TABLE IF NOT EXISTS` guards prevent errors. Agent tables are ignored. |
| New Nous binary + old database (44 migrations) | Works. On first open, runs migrations 45-53 to add agent tables. |
| Existing `agent_id` fields in `memories`, `room_participants`, `room_messages` | Remain freeform TEXT. No FK constraint added. Optional JOIN capability to `agents` table. |

**No breaking changes:** Existing queries and code continue to work. Agent registry is additive.

**Data backfill strategy:**

Existing `memories.agent_id` values (freeform TEXT) can optionally be migrated to the `agents` table via a post-deployment script:

```sql
-- Find unique agent_id values from memories
INSERT OR IGNORE INTO agents (id, name, agent_type, status)
SELECT DISTINCT agent_id, agent_id, 'custom', 'active'
FROM memories
WHERE agent_id IS NOT NULL;

-- Repeat for room_participants.agent_id and room_messages.sender_id
INSERT OR IGNORE INTO agents (id, name, agent_type, status)
SELECT DISTINCT agent_id, agent_id, 'custom', 'active'
FROM room_participants
WHERE agent_id NOT IN (SELECT id FROM agents);
```

This backfill is optional. The registry is designed to work alongside freeform agent_id fields indefinitely.

**Testing:**

Unit tests should verify:
1. Fresh database opens successfully with all 53 migrations applied
2. Existing database (44 migrations) upgrades to 53 migrations without errors
3. All indexes are created
4. Agent lineage recursive CTE returns correct parent chain
5. CASCADE deletes work (deleting an agent removes its sessions and metrics)
6. CHECK constraints reject invalid `agent_type` and `status` values

**Rollback strategy:**

If a bug is discovered post-deployment, the agent tables can be dropped manually via SQL without affecting memory or room operations. The three tables are independent (no FK from `memories` or `rooms` to `agents`).

```sql
DROP TABLE IF EXISTS agent_metrics;
DROP TABLE IF EXISTS agent_sessions;
DROP TABLE IF EXISTS agents;
```

This rollback does NOT require a code rollback. Old binaries will function normally after the DROP.

## 10. Future Work

**FK enforcement (Phase 2):**

Convert existing freeform `agent_id` columns to foreign keys:

```sql
-- Add FK constraint to memories.agent_id
ALTER TABLE memories ADD CONSTRAINT fk_memories_agent FOREIGN KEY (agent_id) REFERENCES agents(id);

-- Add FK constraint to room_participants.agent_id
ALTER TABLE room_participants ADD CONSTRAINT fk_room_participants_agent FOREIGN KEY (agent_id) REFERENCES agents(id);

-- Add FK constraint to room_messages.sender_id
ALTER TABLE room_messages ADD CONSTRAINT fk_room_messages_sender FOREIGN KEY (sender_id) REFERENCES agents(id);
```

Requires backfill (see Section 9) to ensure all existing `agent_id` values exist in `agents` table before adding constraints.

**Agent-to-agent communication (Phase 2):**

Add `agent_messages` table for inter-agent messaging (separate from room messages):

```sql
CREATE TABLE agent_messages (
    id TEXT PRIMARY KEY,
    from_agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    to_agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    metadata TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
```

MCP tools: `agent_send_message`, `agent_list_messages`.

**Auto-discovery (Phase 3):**

Scan process tables, logs, or runtime telemetry to auto-register agents:

- Parse Paseo CLI output (`paseo ls --json`)
- Parse OTLP traces (`traces.span_name LIKE 'agent:%'`)
- Parse memory store events (`memories.agent_id NOT IN agents.id`)

Cron job or background task to periodically sync discovered agents into registry.

**Dashboard (Phase 3):**

Web UI for agent visualization:

- Agent hierarchy graph (D3.js, Mermaid, or Graphviz)
- Metrics time-series charts (memory counts, session durations, error rates)
- Search and filter (by workspace, status, model, lineage)
- Live updates via WebSocket or SSE

Built on top of MCP tools — UI calls `agent_list`, `agent_lineage`, `agent_metrics`.

**Agent versioning (Phase 2):**

Track agent config snapshots (model, system prompt hash, tool list) in `agent_versions` table:

```sql
CREATE TABLE agent_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL,
    config_snapshot TEXT NOT NULL,  -- JSON: {model, system_prompt_hash, tools, ...}
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(agent_id, version_number)
);
```

MCP tools: `agent_create_version`, `agent_list_versions`, `agent_get_version`.

**Workspace-scoped queries (Phase 2):**

CLI flag: `nous agent list --workspace <path>` — resolves workspace path to ID, filters agents.

MCP tool: `agent_list` already supports `workspace_id` param.

**Agent capabilities registry (Phase 3):**

Structured `capabilities` field in `metadata` JSON:

```json
{
  "capabilities": {
    "tools": ["memory_store", "room_create", "schedule_create"],
    "model_context_window": 200000,
    "max_output_tokens": 8192,
    "supports_vision": true
  }
}
```

MCP tool: `agent_search_by_capability(capability_name, capability_value)` — JSON path query on metadata.

**Performance metrics aggregation (Phase 2):**

Add `agent_metrics_hourly` table for downsampled time-series (reduces row count for long-running agents):

```sql
CREATE TABLE agent_metrics_hourly (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    metric_type TEXT NOT NULL,
    hour_bucket TEXT NOT NULL,  -- ISO8601 hour: "2026-04-27T04:00:00Z"
    sum_value REAL NOT NULL,
    count INTEGER NOT NULL,
    avg_value REAL GENERATED ALWAYS AS (sum_value / count) STORED,
    UNIQUE(agent_id, metric_type, hour_bucket)
);
```

Background task: aggregate `agent_metrics` rows into hourly buckets, delete raw rows older than 7 days.

**Agent status transitions (Phase 2):**

Add `agent_status_history` table to track lifecycle changes:

```sql
CREATE TABLE agent_status_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    from_status TEXT NOT NULL,
    to_status TEXT NOT NULL,
    changed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
```

Trigger on `agents.status` UPDATE to log transitions.

**Implementation phases:**

| Phase | Features | Estimated LOE |
|-------|----------|---------------|
| **MVP (Phase 1)** | agents, agent_sessions, agent_metrics tables; 7 MCP tools; CLI commands; recursive lineage | 2-3 days |
| **Phase 2** | FK enforcement, agent versioning, workspace-scoped queries, metrics aggregation, status history | 3-5 days |
| **Phase 3** | Auto-discovery, dashboard, agent-to-agent messaging, capabilities registry | 5-8 days |

**Total estimated LOE:** 10-16 days (includes testing, documentation, PR review)

**Assumption:** Developer familiar with Rust, rusqlite, and the existing WriteChannel/ReadPool patterns.
