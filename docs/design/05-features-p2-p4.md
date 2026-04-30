# 05: P2-P4 Features — Worktree, Org Hierarchy, Schedule

**Initiative:** INI-076  
**Status:** Draft  
**Author:** Technical Writer (paseo agent)  
**Date:** 2026-04-29

---

## 1. Goals

**P2 — Worktree Management:**

- Git worktrees created, tracked, and archived inside Nous — persisted in SQLite alongside memory, rooms, and tasks
- Worktrees associated with both a task ID and an agent ID so ownership and purpose are always queryable
- Automatic staleness detection: worktrees with no new commits for a configurable threshold (default 24 h) transition to `stale`
- Cleanup automation: worktrees for merged branches archive without manual intervention
- Paseo currently writes worktrees to `~/.paseo/worktrees/<slug>/`; this design formalises that contract with a database record per worktree

**P3 — Org Hierarchy:**

- Agent registry embedded in Nous, ported from the standalone `org-management` binary (v0.2.0)
- Parent–child relationships between agents with namespace scoping so queries never cross team boundaries
- Heartbeat tracking (`last_seen_at`) with configurable staleness threshold to detect dead agents
- Artifact ownership model: worktrees, rooms, and schedules carry an `agent_id` FK so the owning agent is always known
- Queries for ancestors, descendants, and subtree roots all scoped by namespace

**P4 — Schedule Engine:**

- Cron-based scheduling is already implemented (see §5.1); this section documents the existing architecture and planned extensions
- Planned: one-shot timers (`max_runs=1`), event-driven triggers on room messages and task status changes, and schedule templates
- Planned: integration between the task system and the scheduler so recurring tasks are created and closed automatically

---

## 2. Non-Goals

- No distributed or multi-host scheduling: the Schedule Engine targets single-node Nous deployments
- No Git hosting or PR automation: worktree management tracks local filesystem paths only; PR creation remains a CLI concern
- No cross-namespace agent queries: namespace boundaries are hard — agents in namespace `A` cannot enumerate agents in namespace `B`
- No real-time agent monitoring dashboard: org hierarchy exposes data via MCP tools and CLI; rendering is out of scope
- No container or VM lifecycle management: agents are logical entities in the registry; their process management is handled by the Paseo runtime
- No external cron daemon integration (e.g. systemd timers, AWS EventBridge): the built-in `Scheduler` struct handles all scheduling

---

## 3. Worktree Management (P2)

### 3.1 Architecture

Worktree management adds a `worktrees` table to the Nous SQLite database (see `docs/design/02-data-layer.md` for the WriteChannel/ReadPool pattern). Every worktree write goes through `WriteChannel`; reads hit a `ReadPool` connection. The MCP server (see `docs/design/03-api-interfaces.md`) exposes three tools; the `nous` CLI mirrors them.

```
┌─────────────────────────────────────────────────────────────────┐
│ Nous Process                                                    │
│                                                                 │
│  ┌────────────────────┐        ┌──────────────────────────────┐ │
│  │  MCP Server        │        │  CLI (nous worktree …)       │ │
│  │  worktree_create   │        │  nous worktree create        │ │
│  │  worktree_list     │        │  nous worktree list          │ │
│  │  worktree_archive  │        │  nous worktree archive       │ │
│  └────────┬───────────┘        └──────────────┬───────────────┘ │
│           │                                    │                 │
│           └──────────────┬─────────────────────┘                │
│                          ▼                                       │
│              ┌───────────────────────┐                          │
│              │  WorktreeService      │                          │
│              │  create()             │                          │
│              │  list()               │                          │
│              │  archive()            │                          │
│              │  detect_stale()       │  ◄── background task    │
│              │  cleanup_merged()     │  ◄── background task    │
│              └──────────┬────────────┘                          │
│                         │                                        │
│         ┌───────────────┼──────────────────────┐                │
│         ▼               ▼                      ▼                │
│  WriteChannel     ReadPool              Git subprocess           │
│  (INSERT/UPDATE)  (SELECT)              (git worktree add/prune) │
└─────────────────────────────────────────────────────────────────┘
                          │
                          ▼
                  ~/.paseo/worktrees/<slug>/
                  (filesystem, managed by git)
```

The `WorktreeService` shells out to `git worktree add` on creation and `git worktree prune` on archive/delete. The database record is the source of truth for metadata (agent owner, task, status); the filesystem is the source of truth for the actual working tree.

### 3.2 Data Model

IDs follow the UUIDv7 convention (see `docs/design/01-system-architecture.md`). The `slug` column stores the short identifier used in the filesystem path (`~/.paseo/worktrees/<slug>/`).

```sql
CREATE TABLE IF NOT EXISTS worktrees (
    id         TEXT NOT NULL PRIMARY KEY,           -- UUIDv7
    slug       TEXT NOT NULL,                       -- e.g. "2y8hcni0"
    path       TEXT NOT NULL,                       -- absolute path on disk
    branch     TEXT NOT NULL,                       -- git branch name
    repo_root  TEXT NOT NULL,                       -- parent repo path
    agent_id   TEXT REFERENCES agents(id)
                   ON DELETE SET NULL,
    task_id    TEXT REFERENCES tasks(id)
                   ON DELETE SET NULL,
    status     TEXT NOT NULL DEFAULT 'active'
                   CHECK (status IN ('active', 'stale', 'archived', 'deleted')),
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    UNIQUE(slug, repo_root)
);

CREATE INDEX IF NOT EXISTS idx_worktrees_agent   ON worktrees(agent_id);
CREATE INDEX IF NOT EXISTS idx_worktrees_task    ON worktrees(task_id);
CREATE INDEX IF NOT EXISTS idx_worktrees_status  ON worktrees(status);
CREATE INDEX IF NOT EXISTS idx_worktrees_branch  ON worktrees(branch);
```

| Column | Type | Notes |
|--------|------|-------|
| `id` | `TEXT` (UUIDv7) | Globally unique; sortable by creation time |
| `slug` | `TEXT` | Short path component, e.g. `2y8hcni0`; unique per `repo_root` |
| `path` | `TEXT` | Absolute path, e.g. `~/.paseo/worktrees/2y8hcni0/` |
| `branch` | `TEXT` | The git branch checked out in this worktree |
| `repo_root` | `TEXT` | Absolute path to the parent git repository |
| `agent_id` | `TEXT` FK | Agent that created / owns this worktree; `NULL` if agent deleted |
| `task_id` | `TEXT` FK | Task this worktree was created for; `NULL` if unassigned |
| `status` | `TEXT` enum | `active` → `stale` → `archived` / `deleted` |

### 3.3 Lifecycle

```
create ──► active ──► stale ──► archived
                  \                 │
                   \                ▼
                    └──────────► deleted
```

| Transition | Trigger | Action |
|------------|---------|--------|
| `→ active` | `worktree_create` call | `git worktree add <path> <branch>` + DB insert |
| `active → stale` | No commits in last N hours (configurable, default 24 h) | DB UPDATE only; filesystem unchanged |
| `stale → archived` | Manual `worktree_archive` or branch merged into main | `git worktree prune` + DB UPDATE |
| `active → archived` | Manual `worktree_archive` | `git worktree prune` + DB UPDATE |
| `archived → deleted` | Manual `worktree_delete` | Filesystem removal + DB UPDATE |

A stale worktree remains usable — the `stale` status is informational, not a lock. The only behavioral difference is that the background cleanup job will archive stale worktrees whose branches have been merged.

**Branch-merge detection** runs on a configurable schedule (default: every 6 hours). For each `active` or `stale` worktree, the service checks whether `git merge-base --is-ancestor <branch> <main>` exits 0. When it does, the worktree transitions to `archived` automatically.

### 3.4 Staleness Detection and Cleanup Automation

**Staleness detection** runs as a background Tokio task alongside the Scheduler. Every `stale_check_interval_secs` (default: 3600) it:

1. Queries all `active` worktrees.
2. For each, shells out `git -C <path> log --oneline --since=<threshold>` to count commits since the threshold.
3. Records with zero commits transition to `stale` via `WriteChannel`.

The staleness threshold is configured in `config.toml`:

```toml
[worktrees]
stale_threshold_hours = 24      # hours without commits → stale
stale_check_interval_secs = 3600
cleanup_check_interval_secs = 21600  # merged-branch check every 6 h
main_branch = "main"
```

**Cleanup automation** runs every `cleanup_check_interval_secs` and archives worktrees where:

- `status IN ('active', 'stale')`, AND
- `git merge-base --is-ancestor <branch> <main_branch>` exits 0 (branch merged)

This requires the `repo_root` column so the check runs in the correct git repository. Archived worktrees are not deleted automatically — `nous worktree delete` requires explicit invocation to remove files from disk.

### 3.5 API Surface

**MCP Tools** (exposed via `rmcp`, consistent with `docs/design/03-api-interfaces.md`):

| Tool | Input | Output |
|------|-------|--------|
| `worktree_create` | `slug`, `branch`, `repo_root`, `agent_id?`, `task_id?` | `{ id, slug, path, branch, status }` |
| `worktree_list` | `status?`, `agent_id?`, `task_id?`, `limit?` | `{ worktrees: [...], total }` |
| `worktree_archive` | `id` or `slug` | `{ archived: true, id }` |

**CLI** (sous `nous worktree`, consistent with clap patterns in `docs/design/03-api-interfaces.md`):

```
nous worktree create --branch <branch> [--slug <slug>] [--task <task-id>]
nous worktree list [--status active|stale|archived] [--agent <id>] [--task <id>]
nous worktree archive <id-or-slug>
nous worktree delete <id-or-slug>
```

The `create` subcommand derives the `slug` from the last 8 characters of the UUIDv7 ID when `--slug` is not supplied, matching Paseo's existing convention (`~/.paseo/worktrees/2y8hcni0/`).

---

## 4. Org Hierarchy (P3)

### 4.1 Architecture

The org hierarchy is already implemented in the standalone `org-management` binary (v0.2.0, `~/ws/org-management`). For Nous integration, the same SQLite schema and operations are embedded directly into the Nous database — no separate process or IPC required.

```
┌──────────────────────────────────────────────────────────────────┐
│ Namespace: "bf91"                                                │
│                                                                  │
│               Director                                           │
│             (86872aaf-…)                                         │
│              /        \                                          │
│      Manager-A       Manager-B                                   │
│      /      \             \                                      │
│  Eng-1     Eng-2          Eng-3                                  │
│                                                                  │
│  Artifacts owned by Eng-1:                                       │
│    worktree: 2y8hcni0  (status: active)                         │
│    room:     bf91-59e2-eng1-work                                 │
└──────────────────────────────────────────────────────────────────┘
```

The `agents` table stores the hierarchy via a self-referencing `parent_agent_id` FK. The `relationships` table provides an explicit edge list for efficient ancestor/descendant traversals. Both tables carry a `namespace` column; all queries filter on it.

Data writes go through `WriteChannel`; reads use `ReadPool`. The `AgentService` exposes the same operations as the `org-management` binary's `DbHandle`: `register`, `deregister`, `lookup`, `list_children`, `list_ancestors`, `get_tree`, `heartbeat`.

### 4.2 Data Model

The following DDL is ported directly from `~/ws/org-management/src/db.rs` with UUIDv7 IDs replacing the prior `TEXT PRIMARY KEY` without type constraint.

```sql
CREATE TABLE IF NOT EXISTS agents (
    id              TEXT NOT NULL PRIMARY KEY,       -- UUIDv7
    name            TEXT NOT NULL,
    agent_type      TEXT NOT NULL
                        CHECK (agent_type IN ('engineer','manager',
                                              'director','senior-manager')),
    parent_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL,
    namespace       TEXT NOT NULL DEFAULT 'default',
    status          TEXT NOT NULL DEFAULT 'active'
                        CHECK (status IN ('active','inactive','archived',
                                          'running','idle','blocked','done')),
    room            TEXT,
    last_seen_at    TEXT,
    metadata_json   TEXT,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE (name, namespace)
);

CREATE TABLE IF NOT EXISTS agent_relationships (
    parent_id         TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    child_id          TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    relationship_type TEXT NOT NULL DEFAULT 'reports_to',
    namespace         TEXT NOT NULL DEFAULT 'default',
    created_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (parent_id, child_id, namespace)
);

CREATE TABLE IF NOT EXISTS artifacts (
    id            TEXT NOT NULL PRIMARY KEY,         -- UUIDv7
    agent_id      TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    artifact_type TEXT NOT NULL
                      CHECK (artifact_type IN ('worktree','room','schedule','branch')),
    name          TEXT NOT NULL,
    path          TEXT,
    status        TEXT NOT NULL DEFAULT 'active'
                      CHECK (status IN ('active','archived','deleted')),
    namespace     TEXT NOT NULL DEFAULT 'default',
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_seen_at  TEXT,
    UNIQUE (agent_id, artifact_type, name, namespace)
);

CREATE INDEX IF NOT EXISTS idx_agents_namespace  ON agents(namespace);
CREATE INDEX IF NOT EXISTS idx_agents_parent     ON agents(parent_agent_id);
CREATE INDEX IF NOT EXISTS idx_agents_status     ON agents(namespace, status);
CREATE INDEX IF NOT EXISTS idx_rel_parent        ON agent_relationships(parent_id, namespace);
CREATE INDEX IF NOT EXISTS idx_rel_child         ON agent_relationships(child_id, namespace);
CREATE INDEX IF NOT EXISTS idx_artifacts_agent   ON artifacts(agent_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_ns      ON artifacts(namespace);
CREATE INDEX IF NOT EXISTS idx_artifacts_type    ON artifacts(agent_id, artifact_type, namespace);
```

An FTS5 virtual table over `agents(name, agent_type, namespace, metadata_json)` enables name-based discovery, mirroring the implementation in `org-management/src/db.rs:104-128`.

### 4.3 Namespace Isolation

Every read query adds `AND namespace = ?` before executing. No cross-namespace query path exists — this is enforced at the service layer, not just by convention.

Namespace values map to Nous instance identifiers (e.g. team ID `59e2`, org ID `bf91`). A Nous instance typically operates in a single namespace configured at startup via `config.toml`:

```toml
[org]
namespace = "bf91"
```

The `org-management` MCP tools accept an explicit `namespace` parameter. When absent, they default to the configured namespace — callers in a different namespace cannot accidentally read each other's agent registries.

Ancestor and descendant traversals use the `agent_relationships` edge table rather than recursive CTEs on `parent_agent_id`, which lets the query planner use the `idx_rel_parent` and `idx_rel_child` indexes directly instead of a full table scan.

### 4.4 Heartbeat and Staleness

Agents call `agent_heartbeat` (MCP tool) or `nous agent heartbeat` (CLI) periodically, which updates `last_seen_at = now()` on the agent row. A background task in Nous sweeps agents every `heartbeat_check_interval_secs` (default: 300) and marks as `inactive` any agent whose `last_seen_at` is older than `stale_threshold_secs` (default: 900 — 15 minutes).

```toml
[org]
heartbeat_check_interval_secs = 300
stale_threshold_secs = 900
```

The staleness check does not cascade: if a Manager goes `inactive`, its Engineers remain at their current status. Callers can detect dead subtrees by querying `list_ancestors` and checking whether any ancestor has `status = 'inactive'`.

Status transitions driven by heartbeat:

| Event | Transition |
|-------|------------|
| `heartbeat` received | any → `active` (or `running` / `idle` if payload specifies) |
| `last_seen_at` > threshold | `active` / `running` / `idle` → `inactive` |
| Explicit `deregister` | any → `archived` |

### 4.5 Artifact Ownership

The `artifacts` table (see §4.2) links any owned resource back to its agent. `artifact_type` is an enum with four values:

| Type | What it represents | `name` convention | `path` |
|------|--------------------|-------------------|--------|
| `worktree` | A git worktree row from §3 | slug (e.g. `2y8hcni0`) | absolute filesystem path |
| `room` | A chat room from `docs/design/04-features-p0-p1.md` | room name | `NULL` |
| `schedule` | A schedule from §5 | schedule name | `NULL` |
| `branch` | A git branch (no worktree) | branch name | repo root |

When an agent is deregistered, its artifacts are cascade-deleted (FK `ON DELETE CASCADE`). When a worktree or room is archived, `artifact_status` transitions to `archived` via a join update.

The tasks system (P1, `docs/design/04-features-p0-p1.md`) stores the assigning agent in a `assigned_to TEXT` column on the `tasks` table — this is a logical reference, not a FK, since tasks outlive individual agents. The `artifacts` table covers resources *owned* by an agent at creation time.

### 4.6 API Surface

**MCP Tools** (ported from `org-management/src/mcp/tools.rs`):

| Tool | Key inputs | Output |
|------|-----------|--------|
| `agent_register` | `name`, `agent_type`, `parent_id?`, `namespace?`, `room?`, `metadata?` | `{ agent, created }` |
| `agent_deregister` | `id` or `name`, `namespace?`, `cascade?` | `{ result: "deleted" \| "cascaded" \| "has_children" }` |
| `agent_lookup` | `name`, `namespace?` | `Agent` object |
| `agent_list` | `namespace?`, `status?`, `agent_type?`, `limit?` | `{ agents, total }` |
| `agent_list_children` | `id`, `namespace?` | `[{ id, name }]` |
| `agent_list_ancestors` | `id`, `namespace?` | `[Agent]` ordered root → leaf |
| `agent_tree` | `root_id?`, `namespace?` | `TreeNode` (recursive) |
| `agent_heartbeat` | `id`, `status?` | `{ ok: true }` |
| `artifact_register` | `agent_id`, `artifact_type`, `name`, `path?`, `namespace?` | `Artifact` |
| `artifact_deregister` | `id` | `{ ok: true }` |
| `artifact_list` | `agent_id?`, `artifact_type?`, `namespace?` | `{ artifacts, total }` |

**CLI** (sous `nous agent` and `nous artifact`):

```
nous agent register --name <name> --type engineer|manager|director [--parent <id>]
nous agent deregister <id-or-name> [--cascade]
nous agent lookup <name>
nous agent list [--status <status>] [--type <type>]
nous agent tree [--root <id>]
nous agent heartbeat <id> [--status running|idle|blocked|done]
nous artifact register --agent <id> --type worktree|room|schedule|branch --name <name>
nous artifact list --agent <id> [--type <type>]
```

---

## 5. Schedule Engine (P4)

### 5.1 Current Architecture

The Schedule Engine is fully implemented as of the current branch. Its components are:

| Component | File | Responsibility |
|-----------|------|----------------|
| `CronExpr` | `crates/nous-core/src/cron_parser.rs` | Parse and evaluate 5-field cron expressions |
| `Scheduler` | `crates/nous-core/src/scheduler.rs` | Run loop, concurrency limiting, startup recovery |
| `ScheduleDb` | `crates/nous-core/src/schedule_db.rs` | CRUD, next-run computation, run recording |
| `Schedule` / `ScheduleRun` | `crates/nous-core/src/types.rs:383-434` | Data types |
| `ScheduleConfig` | `crates/nous-core/src/scheduler.rs:16-35` | Runtime configuration |

```
┌─────────────────────────────────────────────────────────────────┐
│ Nous Process (startup)                                          │
│                                                                 │
│   startup_recovery()                                            │
│   ├── SELECT id FROM schedules WHERE enabled = 1               │
│   └── compute_next_run(id) for each                             │
│                                                                 │
│   Scheduler::run() — Tokio task                                 │
│   ├── next_pending() ──► earliest next_run_at                  │
│   ├── sleep(delay)  ◄── woken by Notify on schedule change     │
│   └── tokio::spawn(execute_schedule) per due schedule          │
│       ├── semaphore.acquire()  (max_concurrent = 4 default)    │
│       ├── running HashSet — skip if already running            │
│       ├── dispatch_action(schedule)                             │
│       │   ├── McpTool ──► dispatch_mcp_tool()                  │
│       │   ├── Shell   ──► dispatch_shell()  [allow_shell=false] │
│       │   └── Http    ──► dispatch_http()                       │
│       ├── evaluate_desired_outcome() — substring or /regex/    │
│       ├── RunPatch → WriteChannel → schedule_runs              │
│       └── emit_otlp_span() ──► nous-otlp SQLite                │
└─────────────────────────────────────────────────────────────────┘
```

The `Notify` token lets the run loop react immediately when a new schedule is created or an existing one is updated, without polling.

### 5.2 CronExpr Parser

`CronExpr` (`cron_parser.rs:7-15`) represents a parsed 5-field cron expression. All field values are pre-expanded into `BTreeSet<u32>` at parse time, so `next_run` is a pure set-membership walk — no re-parsing on every tick.

```
CronExpr {
    minutes:        BTreeSet<u32>  // 0–59
    hours:          BTreeSet<u32>  // 0–23
    days_of_month:  BTreeSet<u32>  // 1–31
    months:         BTreeSet<u32>  // 1–12
    days_of_week:   BTreeSet<u32>  // 0–6 (Sunday = 0)
    dom_is_wildcard: bool
    dow_is_wildcard: bool
}
```

**Field syntax** (`parse_field`, `cron_parser.rs:35-109`):

| Syntax | Example | Meaning |
|--------|---------|---------|
| Wildcard | `*` | All values in range |
| Single | `5` | Only value 5 |
| Range | `1-5` | Values 1 through 5 inclusive |
| Step | `*/15` | Every 15th value starting at min |
| Step on range | `1-10/2` | Odd values 1, 3, 5, 7, 9 |
| List | `1,3,5` | Values 1, 3, 5 |
| Mixed | `1-5,3-7` | Union: values 1 through 7 |

**DOM/DOW semantics** (`day_matches`, `cron_parser.rs:245-260`): when both `days_of_month` and `days_of_week` are constrained (neither is the literal `*` wildcard), the POSIX OR rule applies — a day matches if it satisfies *either* the DOM or the DOW constraint.

**DST handling** (`next_run`, `cron_parser.rs:144-243`): the iterator advances via UTC to avoid ambiguous local times during fall-back transitions. When a computed local time falls in a DST spring-forward gap, the algorithm skips to the next day rather than silently firing at the wrong hour. The `earliest()` disambiguation strategy is used for fall-back duplicates, so `30 1 * * *` in US/Eastern fires at the first occurrence of 1:30 AM on fall-back night.

**Safety limit**: `next_run` searches at most 4 years (366 × 4 + 1 days) ahead. Expressions with no valid future date (e.g. `0 0 30 2 *`) return `None`.

### 5.3 Scheduler Runtime

`Scheduler` (`scheduler.rs:37-160`) is a single Tokio task spawned at process startup via `Scheduler::spawn(wc, rp, config)`. It holds:

| Field | Type | Role |
|-------|------|------|
| `write_channel` | `WriteChannel` | Writes `next_run_at` updates and run records |
| `read_pool` | `ReadPool` | Reads `next_pending()` schedule |
| `notify` | `Arc<Notify>` | Wakes the run loop immediately on schedule change |
| `config` | `ScheduleConfig` | Feature flags and limits |
| `otlp_db_path` | `Option<String>` | Path to OTLP SQLite for span emission |

**`ScheduleConfig` defaults** (`scheduler.rs:26-35`):

| Field | Default | Meaning |
|-------|---------|---------|
| `enabled` | `true` | Master switch; when `false`, run loop exits immediately |
| `allow_shell` | `false` | Shell actions are disabled unless explicitly opted in |
| `allow_http` | `true` | HTTP actions enabled |
| `max_concurrent` | `4` | Semaphore capacity — at most 4 schedules run in parallel |
| `default_timeout_secs` | `300` | Per-run wall-clock timeout; schedule can override |

**Startup recovery** (`startup_recovery`, `scheduler.rs:141-159`): on boot, re-computes `next_run_at` for all enabled schedules. This handles the case where the process was down during a cron window — it does not fire missed runs, it only advances the clock to the next future slot.

**Concurrency control**: a `HashSet<schedule_id>` guards against overlap — if a schedule fires while a prior run for the same ID is still in progress, a `Skipped` run record is written and the new invocation aborts. The semaphore enforces the `max_concurrent` ceiling across all schedules.

**Retry backoff**: on `ActionType::Shell` or `ActionType::Http` failures, the executor retries up to `max_retries` times with exponential backoff `2^attempt` seconds (capped to prevent overflow via `.saturating_pow`).

### 5.4 Data Model

The `schedules` and `schedule_runs` tables are production code in `crates/nous-core/src/db.rs:199-233`. Reproduced here for reference:

```sql
CREATE TABLE IF NOT EXISTS schedules (
    id              TEXT NOT NULL PRIMARY KEY,       -- UUIDv7
    name            TEXT NOT NULL,
    cron_expr       TEXT NOT NULL,
    timezone        TEXT NOT NULL DEFAULT 'UTC',
    enabled         INTEGER NOT NULL DEFAULT 1,
    action_type     TEXT NOT NULL
                        CHECK (action_type IN ('mcp_tool', 'shell', 'http')),
    action_payload  TEXT NOT NULL,
    desired_outcome TEXT DEFAULT NULL,
    max_retries     INTEGER NOT NULL DEFAULT 3,
    timeout_secs    INTEGER,
    max_output_bytes INTEGER NOT NULL DEFAULT 65536,
    max_runs        INTEGER NOT NULL DEFAULT 100,
    next_run_at     INTEGER,                         -- Unix timestamp (seconds)
    created_at      INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at      INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_schedules_enabled_next ON schedules(enabled, next_run_at);
CREATE INDEX IF NOT EXISTS idx_schedules_name         ON schedules(name);

CREATE TABLE IF NOT EXISTS schedule_runs (
    id          TEXT NOT NULL PRIMARY KEY,           -- UUIDv7
    schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    status      TEXT NOT NULL DEFAULT 'running'
                    CHECK (status IN ('running','completed','failed','timeout','skipped')),
    exit_code   INTEGER,
    output      TEXT,
    error       TEXT,
    attempt     INTEGER NOT NULL DEFAULT 1,
    duration_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_runs_schedule_started ON schedule_runs(schedule_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_runs_status           ON schedule_runs(status);
```

`max_runs` controls both the run-count retention limit and — when set to 1 — implements one-shot timers (see §5.6). When `record_run_on` inserts a new run, it immediately purges the oldest rows exceeding `max_runs` for that schedule (`schedule_db.rs:151-168`).

> **Note:** The `schedules` and `schedule_runs` tables currently use INTEGER (Unix epoch seconds) for timestamps. A future migration will convert these to ISO-8601 TEXT format to match the rest of the system (memories, rooms, tasks, agents all use TEXT timestamps). The INTEGER format is retained for now because the scheduler performs arithmetic on `next_run_at`.

### 5.5 Action Dispatch

Three action types are supported (`dispatch_action`, `scheduler.rs:321-342`):

**`McpTool`** (`action_payload`: `{ "tool": "<name>", "args": {...} }`)

Currently dispatches to in-process handlers only (`dispatch_mcp_tool`, `scheduler.rs:351-431`). Supported tools:

| Tool | Args | Returns |
|------|------|---------|
| `memory_stats` | none | statistics JSON |
| `memory_search` | `{ "query": "…" }` | `{ results: [{id, title, memory_type}] }` |
| `memory_forget` | `{ "id": "…", "hard": bool }` | `{ forgotten: … }` |

The set of dispatchable tools will expand when the MCP server exposes a local call path (planned, not yet implemented).

**`Shell`** (`action_payload`: `{ "command": "…", "working_dir": "…" }`)

Disabled by default (`allow_shell = false`). When enabled, spawns `sh -c <command>` with `kill_on_drop` and captures stdout + stderr. Non-zero exit codes fail the run. Stderr is appended to stdout with a `--- stderr ---` separator.

**`Http`** (`action_payload`: `{ "method": "GET", "url": "…", "headers": {}, "body": "…" }`)

Uses `reqwest`. Non-2xx responses fail the run. No authentication helpers beyond static headers — OAuth flows are not supported.

**Desired outcome** (`evaluate_desired_outcome`, `scheduler.rs:612-643`): the `desired_outcome` column accepts either a plain substring (match if output contains it) or a `/regex/`-delimited pattern. A mismatch writes a `Failed` run with `error = "outcome mismatch: expected …, got …"` even when the action itself returned exit code 0.

### 5.6 Planned Extensions

**One-shot timers**

Set `max_runs = 1`. When `record_run_on` fires and the schedule has `max_runs = 1`, `execute_schedule` checks the completed run count after recording and disables the schedule (`enabled = 0`). No schema change required — this is a behavioral extension of the existing `max_runs` column.

For ergonomics, the MCP tool and CLI will accept a `--once` or `--at <timestamp>` flag that sets `max_runs = 1` and computes a one-time `next_run_at` (bypassing the normal `cron_expr` path). The `cron_expr` for one-shot schedules is stored as `@once` (a non-standard sentinel that `CronExpr::parse` rejects gracefully today — a `@once` path will be added).

**Event-driven triggers**

Three trigger sources are planned:

| Trigger | Config field | Fire condition |
|---------|-------------|----------------|
| Room message | `on_room_message: { room_id, pattern? }` | New message in room; optional regex filter on content |
| Task status change | `on_task_status: { task_id?, status }` | Any task (or specific task) transitions to given status |
| Memory create | `on_memory_create: { memory_type? }` | A memory record is inserted, optionally filtered by type |

Each trigger source is an alternative to `cron_expr`. A new column `trigger_type TEXT CHECK(trigger_type IN ('cron', 'room_message', 'task_status', 'memory_create'))` will be added to `schedules`. The `Scheduler` run loop handles `cron` triggers; a new `EventDispatcher` Tokio task handles the other three by subscribing to internal broadcast channels that the room, task, and memory write paths publish to.

**Schedule templates**

Named presets stored in `config.toml` under `[schedule_templates]`. The CLI exposes them as `nous schedule create --template <name>`. Example:

```toml
[[schedule_templates]]
name = "hourly-memory-stats"
cron_expr = "0 * * * *"
action_type = "mcp_tool"
action_payload = '{"tool":"memory_stats","args":{}}'
```

Templates are expanded at creation time — the stored schedule row contains the resolved values, not a reference to the template name.

**Task integration**

When `action_type = "mcp_tool"` and the tool is `task_create`, the scheduler creates a recurring task on each fire. The task ID is stored in the run's `output` column for traceability. When `max_runs` fires complete, the schedule disables itself as with one-shot timers.

### 5.7 API Surface

**MCP Tools:**

| Tool | Key inputs | Output |
|------|-----------|--------|
| `schedule_create` | `name`, `cron_expr`, `action_type`, `action_payload`, `timezone?`, `desired_outcome?`, `max_retries?`, `timeout_secs?`, `max_output_bytes?`, `max_runs?` | `Schedule` |
| `schedule_get` | `id` | `Schedule` |
| `schedule_list` | `enabled?`, `action_type?`, `limit?` | `{ schedules, total }` |
| `schedule_update` | `id`, any updatable field | `{ updated: true }` |
| `schedule_delete` | `id` | `{ deleted: true }` |
| `schedule_runs_list` | `schedule_id`, `status?`, `limit?` | `[ScheduleRun]` |
| `schedule_health` | none | `{ total, active, failing, outcome_mismatches, next_upcoming }` |

**CLI:**

```
nous schedule create --name <name> --cron "*/5 * * * *" --action mcp_tool \
    --payload '{"tool":"memory_stats"}' [--tz UTC] [--timeout 60]
nous schedule list [--enabled] [--action mcp_tool|shell|http]
nous schedule get <id>
nous schedule update <id> [--cron "…"] [--enabled true|false]
nous schedule delete <id>
nous schedule runs <id> [--status completed|failed|timeout|skipped]
nous schedule health
```

---

## 6. Feature Dependencies

```
P0 (Chat/Rooms) ◄── P2 (Worktrees) depends on rooms for artifact ownership FK
       │                             (room artifact_type references rooms.id logically)
       │
P1 (Tasks) ◄──────── P2 (Worktrees) worktrees carry task_id FK
       │    ◄──────── P3 (Org Hierarchy) tasks have assigned_to agent reference
       │
P3 (Org) ◄─────────── P2 (Worktrees) worktrees carry agent_id FK into agents table
       │  ◄─────────── P4 (Schedules) schedule ownership via artifacts table
       │
P4 (Schedules) ──────► P1 (Tasks)  planned: schedule_create action creates tasks
                 ──────► P3 (Org)  planned: event triggers subscribe to agent heartbeat
```

**Delivery order constraints:**

| Blocker | Dependent | Reason |
|---------|-----------|--------|
| P3 `agents` table must exist | P2 `worktrees.agent_id` FK | FK requires the referenced table |
| P1 `tasks` table must exist | P2 `worktrees.task_id` FK | FK requires the referenced table |
| P0 rooms table must exist | P3 `artifacts` room type | Logical reference (not FK), but rooms must exist to register them |
| Schedule Engine (P4, current) | Event triggers (P4 extension) | Event dispatchers are layered on top of the existing `Scheduler` |

P4 is already shipped as cron-only. The event-trigger and one-shot extensions are additive and do not require P2 or P3 to be complete. Template support requires no dependencies beyond the config infrastructure.

---

## 7. Open Questions

**Worktree Management**

1. **Slug uniqueness scope**: ~~the current Paseo convention derives the slug from the last 8 hex characters of the UUIDv7. Collisions are astronomically unlikely but possible across repos. Should the `UNIQUE` constraint on `slug` be repo-scoped (`UNIQUE(slug, repo_root)`) instead of global?~~ **Resolved:** change UNIQUE constraint to `UNIQUE(slug, repo_root)` instead of global `UNIQUE(slug)`. This allows the same slug in different repositories without collision. See updated DDL in §3.2.

2. **Orphaned worktrees**: ~~when an agent is deregistered, its `agent_id` FK is `SET NULL`. Should the cleanup job archive all worktrees where `agent_id IS NULL AND status = 'active'`, or leave them for manual resolution?~~ **Resolved:** a background cleanup job archives worktrees where `agent_id IS NULL AND status = 'active' AND updated_at < (now - 7 days)`. This prevents indefinite accumulation of orphaned worktrees while giving operators a week to manually reassign them.

3. **Remote worktrees**: the current design assumes `path` is a local filesystem path. If Nous runs in a container, the path may not be accessible from the MCP caller's environment. Does the API need to return an SSH URI or mount path in addition to the local path?

**Org Hierarchy**

4. **Namespace bootstrap**: who creates the initial namespace? The first `agent_register` call implicitly creates any namespace by inserting a row with that namespace value. Should there be an explicit `namespace_create` operation with access control?

5. **Deregister cascade semantics**: the existing `org-management` binary returns `HasChildren` when deregistering an agent with children unless `--cascade` is set. For Nous integration, should `cascade=true` be the default for Manager/Director agents (which expect to own children) and `false` for Engineer agents?

6. **`last_seen_at` timestamp format**: `agents` uses `TEXT` ISO-8601 (inherited from `org-management`); `schedules` and `schedule_runs` use `INTEGER` Unix seconds. Should the agents table be migrated to Unix seconds for consistency across Nous tables?

**Schedule Engine**

7. **`@once` cron sentinel**: ~~the one-shot design proposes a `@once` literal in `cron_expr`. An alternative is a separate `trigger_at INTEGER` column that takes precedence over `cron_expr`. Which approach avoids more parser complexity?~~ **Resolved:** add a `trigger_at INTEGER` column to the `schedules` table that takes precedence over `cron_expr` when non-NULL. This avoids parser complexity from a `@once` sentinel. When `trigger_at` is set, the scheduler fires at that timestamp and disables the schedule (`enabled=0`) after the run completes.

8. **McpTool dispatch expansion**: ~~`dispatch_mcp_tool` currently handles only `memory_stats`, `memory_search`, and `memory_forget`. Exposing all registered MCP tools requires a local call path into the MCP server. What is the interface boundary — a function call within the same Tokio runtime, or a loopback HTTP/stdio connection?~~ **Resolved:** `dispatch_mcp_tool` uses direct Rust function pointers within the same Tokio runtime. No loopback HTTP/stdio connection. The scheduler holds an `Arc<NousServer>` and calls tool handler methods directly.

9. **Event trigger delivery guarantees**: if the Nous process restarts while a room-message trigger is mid-flight, the trigger event is lost. Should event triggers write a durable event record to SQLite before firing so they survive restarts, or is at-most-once acceptable for event triggers?
