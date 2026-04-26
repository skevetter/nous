# Cron & Task Management for nous

## Status

Draft — 2026-04-26

## Context

nous is a Rust MCP memory server (4 crates: shared, core, mcp, otlp). It stores, searches, and relates memories in SQLite with FTS5 + sqlite-vec. It has zero scheduling or task management capability. Adding cron/task management enables agents to schedule recurring actions (memory cleanup, re-embedding, health checks) and manage background work through MCP tools.

## Decision

Use the **`cron` crate** (v0.16, 11.5M downloads) for expression parsing + a **custom executor** built on tokio timers, backed by nous's existing rusqlite database. Avoid adopting apalis or tokio-cron-scheduler wholesale — both bring more machinery than nous needs and introduce dependency/compatibility costs that outweigh their benefits for this use case.

---

## 1. Library Comparison

### Candidates

| Library | Version | Downloads | Last Updated | Async | SQLite | Cron Parsing | Full Scheduler |
|---------|---------|-----------|-------------|-------|--------|-------------|----------------|
| **apalis** | 1.0.0-rc.7 | 718K | 2026-04-13 | Yes | Yes (separate crate, SQLx) | Via `apalis-cron` | Yes |
| **tokio-cron-scheduler** | 0.15.1 | 3.4M | 2025-10-28 | Yes | No (Postgres, NATS) | Via `cron` crate | Yes |
| **cron** | 0.16.0 | 11.5M | 2026-03-25 | N/A | N/A | Yes (7-field) | No — parser only |
| **clokwerk** | 0.4.0 | 964K | 2022-11-23 | Optional | No | No (fluent DSL) | Interval only |
| **job_scheduler** | 1.2.1 | 182K | 2020-04-01 | No | No | Via `cron` | Basic sync |

### apalis Deep-Dive

**Architecture.** Four layers: Backend (trait-based storage), Worker (async stream polling + Monitor coordination), Middleware (Tower Layer/Service), Task (generic `Task<Args, Context, IdType>`). Workers poll backends, maintain heartbeats, process tasks with dependency injection via `Data<T>` extractors.

**SQLite backend.** Separate `apalis-sqlite` crate (7K downloads) built on SQLx. Two tables: `Jobs` (BLOB payload, text status, priority, advisory locking via worker ID) and `Workers`. WAL mode, 7 migrations, 12 indexes. Polling modes: interval-based and hook-based (SQLite update hooks).

**Cron.** `apalis-cron` wraps the `cron` crate v0.16. `CronStream` implements `Backend`, yields `Tick` events on schedule. `pipe_to()` sends ticks into durable storage for crash recovery.

**Lifecycle.** Pending → Queued → Running → Done/Failed/Killed. Default 25 retries. Heartbeat-based orphan recovery. No DLQ — `vacuum()` deletes terminal tasks.

**Observability.** Feature flags for tracing, sentry, prometheus, OpenTelemetry. Worker lifecycle events: Start, Engage, Idle, Error, Stop.

**Risks.**

| Risk | Impact | Detail |
|------|--------|--------|
| Not 1.0 | Medium | 7 RCs in 4 months — API still shifting |
| Bus factor ~1 | Medium | One primary author (Njuguna Mureithi), 28 minor contributors |
| SQLite backend immature | High | 7K downloads, first alpha Oct 2025. Uses SQLx — nous uses rusqlite |
| SQLx vs rusqlite conflict | High | apalis-sqlite requires SQLx; nous uses rusqlite 0.39 + bundled-sqlcipher. Running two SQLite drivers on one DB is unsupported |
| Advisory locking | Low | Worker ID-based, no `FOR UPDATE SKIP LOCKED`. Acceptable for single-node |
| No cluster rate limiting | Low | Irrelevant — nous is single-node |

### tokio-cron-scheduler Deep-Dive

Full async scheduler on tokio. One-shot + recurring + cron jobs. Lifecycle callbacks (on_start, on_done, on_removed). Persistence via PostgreSQL and NATS — **no SQLite backend**. Requires multi-threaded tokio runtime. 0.x version.

**Why not.** No SQLite support. Adding it means writing a custom `MetaDataStorage` + `NotificationStore` implementation against their traits — more work than building a scheduler from scratch with the `cron` crate.

### cron Crate (Parser Only)

Pure parser. Zero runtime deps (chrono only). 7-field format: `sec min hour dom month dow year`. Returns `DateTime<Tz>` iterator of upcoming fire times. 11.5M downloads, actively maintained (Mar 2026).

**Why this.** nous needs to: (1) parse cron expressions, (2) compute next fire time, (3) sleep until then, (4) execute. The `cron` crate handles (1) and (2). Building (3) and (4) on tokio is ~50 lines. No foreign ORM, no trait zoo, no second SQLite driver.

### clokwerk and job_scheduler

Eliminated. clokwerk: no cron expression support, stale since 2022. job_scheduler: abandoned since 2020, sync-only.

### Evaluation

| Criterion | apalis | tokio-cron-scheduler | cron + custom |
|-----------|--------|---------------------|---------------|
| **Correctness** | Yes | Yes | Yes |
| **Simplicity** | Low — 4 layers, Tower middleware, separate crate per backend | Medium — full scheduler but heavy | High — parser + tokio timer loop |
| **DB compatibility** | Conflict — SQLx vs rusqlite | No SQLite at all | Native — reuse existing rusqlite |
| **Reversibility** | Low — deep integration across traits | Medium | High — thin layer, easy to swap |
| **Incremental delivery** | Difficult — need full Backend impl upfront | Medium | Easy — start with in-memory, add persistence later |
| **Consistency with nous** | Poor — different ORM, different patterns | Poor — different persistence model | Good — same rusqlite, same WriteOp/ReadPool pattern |

---

## 2. cronitor-cli Feature Analysis

### Command Surface (40 subcommands)

**Core operations:** discover (scan crontabs), exec (wrap command with telemetry), list (show jobs), status (health snapshot), sync (push to service).

**Monitor CRUD:** list, get, search, create, update, delete, clone, export, pause, unpause. Filterable by type/state/tag/env/group. Output as json/yaml/table.

**Issues/Incidents:** 5 severity levels (outage → operational), 5 states (unresolved → resolved). Bulk operations on issues.

**Maintenance windows:** First-class concept — suppress alerts during scheduled work. Tracks state: upcoming → ongoing → past.

### UX Patterns Worth Adopting

| Pattern | cronitor Implementation | MCP Adaptation |
|---------|------------------------|----------------|
| **Execution wrapping** | 3-signal telemetry: run/complete/fail pings, stdout capture, duration tracking | Task execution records stored in SQLite with start/end/exit_code/output |
| **Health status** | Table with passing/failing/muted per monitor | Structured JSON: `{schedule_id, last_run, next_run, status, consecutive_failures}` |
| **Discovery** | Scan system crontabs, interactive selection | `cron_discover` returns found crontab entries as structured data for agent to register |
| **Maintenance windows** | Named windows with start/end, target monitors | `schedule_pause` with optional duration and reason |
| **Issue lifecycle** | 5-state progression with severity | Simplified: schedule fires → execution record → success/failure with error detail |
| **Output truncation** | 1,000 chars for notifications, 100MB for logs | Configurable `max_output_bytes` per schedule, default 64KB |

### What NOT to Adopt

- **API-key auth / SaaS sync** — nous is local-first, no external service
- **Status pages** — presentation concern, not MCP's job
- **Notification channels** — defer to agent-level notification (MCP client decides)
- **Interactive discovery UX** — MCP tools return data; the agent decides

---

## 3. Architecture Fit

### Current nous Architecture

```
nous-shared  ─── types, SQLite helpers, XDG paths
    │
nous-core   ─── MemoryDb, WriteChannel, ReadPool, EmbeddingBackend, Chunker, Classifier
    │
nous-mcp    ─── 15 MCP tools via rmcp, stdio + HTTP transport, CLI
    │
nous-otlp   ─── OTLP receiver, separate SQLite DB, logs/traces/metrics
```

### Where Scheduling Fits

**Option A: Extend nous-core + nous-mcp** (recommended)

Add `ScheduleDb` alongside `MemoryDb` in nous-core. New tables in the same SQLite database. New MCP tools in nous-mcp. The scheduler loop runs as a tokio task inside the MCP server process.

```
nous-core
├── db.rs          ← add schedule migrations
├── memory_db.rs   ← existing
├── schedule_db.rs ← new: CRUD for schedules + execution records
├── scheduler.rs   ← new: tokio timer loop, drives execution
└── channel.rs     ← extend WriteOp enum with schedule variants
```

Advantages: single process, single database, reuses WriteChannel/ReadPool concurrency model, no new crate to maintain.

**Option B: New `nous-schedule` crate** (following nous-otlp pattern)

Separate crate with its own DB. Linked to nous-mcp via shared IDs.

Disadvantages: second database file, cross-DB queries impossible, more crate coordination. The otlp crate is separate because it receives external data (OTLP protocol) — scheduling is internal to nous and belongs in-process.

**Option C: Adopt apalis wholesale**

Bring in apalis + apalis-sqlite + apalis-cron as dependencies. Implement the Backend trait against nous's existing DB or use apalis-sqlite's separate DB.

Disadvantages: SQLx/rusqlite conflict, opinionated task model (25 retries, vacuum-based cleanup), 1.0 instability, large API surface to learn.

### SQLite Schema Additions

```sql
-- Migration N+1: schedules table
CREATE TABLE schedules (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    cron_expr TEXT NOT NULL,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    enabled INTEGER NOT NULL DEFAULT 1,
    action_type TEXT NOT NULL CHECK (action_type IN ('mcp_tool', 'shell', 'http')),
    action_payload TEXT NOT NULL,  -- JSON: tool name + args, or shell command, or HTTP request
    max_retries INTEGER NOT NULL DEFAULT 3,
    timeout_secs INTEGER,
    max_output_bytes INTEGER NOT NULL DEFAULT 65536,
    next_run_at INTEGER,  -- epoch seconds, computed from cron_expr
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX idx_schedules_enabled_next ON schedules(enabled, next_run_at);
CREATE INDEX idx_schedules_name ON schedules(name);

-- Migration N+2: execution records
CREATE TABLE schedule_runs (
    id TEXT NOT NULL PRIMARY KEY,
    schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    status TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'completed', 'failed', 'timeout', 'skipped')),
    exit_code INTEGER,
    output TEXT,
    error TEXT,
    attempt INTEGER NOT NULL DEFAULT 1,
    duration_ms INTEGER
);

CREATE INDEX idx_runs_schedule_started ON schedule_runs(schedule_id, started_at DESC);
CREATE INDEX idx_runs_status ON schedule_runs(status);
```

### WriteOp Extensions

```rust
enum WriteOp {
    // ... existing variants
    CreateSchedule(Schedule),
    UpdateSchedule(ScheduleId, SchedulePatch),
    DeleteSchedule(ScheduleId),
    RecordRun(ScheduleRun),
    UpdateRun(RunId, RunPatch),
    ComputeNextRun(ScheduleId),
}
```

### OTLP Integration

Each schedule execution can emit trace data to nous-otlp:

- **Span per execution**: `schedule.run` span with `schedule.id`, `schedule.name`, `cron_expr` as attributes
- **Events**: start, retry, complete, fail, timeout
- **Metrics**: `schedule.run.duration_ms` histogram, `schedule.run.count` counter (by status), `schedule.active.count` gauge

The existing `trace_id` and `session_id` fields on memories enable correlation: a scheduled memory-cleanup job can record which memories it archived, linking back via trace_id.

---

## 4. Proposed MCP Tool Surface

### Core Schedule Tools (Phase 1)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_create` | name, cron_expr, action_type, action_payload, timezone?, max_retries?, timeout_secs? | Register a new cron schedule |
| `schedule_list` | enabled?, action_type?, limit? | List schedules with next_run_at |
| `schedule_get` | id | Get schedule detail + last N runs |
| `schedule_update` | id, name?, cron_expr?, action_payload?, enabled?, max_retries?, timeout_secs? | Modify a schedule |
| `schedule_delete` | id | Remove schedule and its run history |
| `schedule_pause` | id, duration_secs?, reason? | Disable temporarily (re-enables after duration if set) |
| `schedule_resume` | id | Re-enable a paused schedule |

### Execution & History Tools (Phase 2)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_runs` | schedule_id?, status?, since?, limit? | Query execution history |
| `schedule_run_get` | run_id | Get run detail including output and error |
| `schedule_trigger` | id | Execute immediately, outside normal schedule |
| `schedule_health` | — | Summary: total schedules, active, failing, next 5 upcoming runs |

### Advanced Tools (Phase 3)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_discover` | — | Scan system crontabs, return found entries |
| `schedule_export` | format? | Export all schedules as JSON |
| `schedule_import` | data | Import schedules from JSON |

### Action Types

**`mcp_tool`** — invoke another nous MCP tool. Payload: `{"tool": "recall", "args": {"query": "stale memories", "since": "-30d"}}`. Enables self-referential automation: schedule `recall` → `forget` pipelines for memory hygiene.

**`shell`** — run a shell command. Payload: `{"command": "df -h", "working_dir": "/tmp"}`. Output captured and stored in `schedule_runs.output`.

**`http`** — make an HTTP request. Payload: `{"method": "POST", "url": "...", "headers": {}, "body": "..."}`. For webhook integrations.

---

## 5. Recommendation

### Library Choice

**`cron` crate (v0.16) + custom executor.**

The `cron` crate provides battle-tested expression parsing (11.5M downloads). The executor is a tokio task that:

1. Queries `SELECT * FROM schedules WHERE enabled = 1 ORDER BY next_run_at ASC LIMIT 1` to find the soonest schedule.
2. Computes sleep duration: `next_run_at - now()`.
3. Sleeps with `tokio::time::sleep_until`, or wakes early on channel notification (new/updated schedule).
4. Executes the action, records the run, computes next_run_at, loops.

This fits naturally into nous's existing tokio runtime and single-writer channel.

### Crate Structure

Keep it in nous-core and nous-mcp. No new crate.

```
crates/nous-core/src/
├── schedule_db.rs   — ScheduleDb: CRUD, run recording, next-run computation
├── scheduler.rs     — Scheduler: tokio task, timer loop, action dispatch
└── db.rs            — add 2 migrations to MIGRATIONS array

crates/nous-mcp/src/
├── tools.rs         — add 11 schedule_* tool handlers
└── server.rs        — spawn Scheduler on startup, pass WriteChannel + ReadPool
```

### Implementation Phases

| Phase | Scope | New Tools | Depends On |
|-------|-------|-----------|------------|
| **1: Core** | Schema + CRUD + timer loop + `mcp_tool` action | schedule_create, schedule_list, schedule_get, schedule_update, schedule_delete, schedule_pause, schedule_resume | Nothing |
| **2: History** | Run recording, query, manual trigger | schedule_runs, schedule_run_get, schedule_trigger, schedule_health | Phase 1 |
| **3: Advanced** | Discovery, export/import, shell + http actions | schedule_discover, schedule_export, schedule_import | Phase 2 |

Each phase ships independently. Phase 1 is ~500 lines of Rust (schema + db + scheduler + 7 tools).

### Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Missed ticks on process restart | Medium | Low | Compute `next_run_at` from cron expression on startup; run overdue schedules immediately |
| Long-running action blocks scheduler | Medium | Medium | Execute actions in spawned tasks, not the timer loop. Timeout enforcement via `tokio::time::timeout` |
| `mcp_tool` self-invocation deadlocks | Low | High | Execute tool calls through a separate channel/task, not re-entering the MCP handler |
| SQLite write contention from execution records | Low | Low | Batch-insert via WriteChannel (existing pattern, batch limit 32) |
| Cron expression parsing edge cases | Low | Low | `cron` crate is well-tested; validate on schedule_create, reject invalid expressions |
| Schema migration on encrypted DB | Low | Medium | Existing `run_migrations()` in nous-shared handles SQLCipher; new migrations follow the same path |

### Open Questions

1. **Should `mcp_tool` actions invoke tools locally or through the MCP protocol?** Local dispatch avoids network overhead but bypasses any client-side middleware. Recommendation: local dispatch via direct function calls on `NousServer`.

2. **Concurrent execution policy.** If a schedule fires while a previous execution is still running: skip, queue, or run in parallel? Recommendation: skip with `status = 'skipped'` in the run record. Add an optional `allow_overlap` flag later.

3. **Timezone handling.** The `cron` crate supports chrono-tz. Store timezone per schedule or force UTC? Recommendation: store per-schedule, default UTC.

4. **Run retention.** How long to keep execution records? Recommendation: configurable `max_runs_per_schedule` (default 100), oldest deleted on insert.

5. **Should `shell` actions be enabled by default?** Shell execution from a memory server is a privilege escalation vector. Recommendation: gate behind a config flag (`[schedule] allow_shell = false`) and a `--allow-shell-schedules` CLI flag.
