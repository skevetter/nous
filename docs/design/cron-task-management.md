# Cron Task Management for Nous

## 1. Summary

Nous currently has zero scheduling capability. This design adds cron-based scheduling and task management so agents can automate recurring work: memory cleanup, re-embedding, health checks, webhook integrations, and arbitrary shell commands.

**Library choice:** A hand-written 5-field cron expression parser (minute, hour, day-of-month, month, day-of-week) with standard syntax support (`*`, ranges, lists, steps) + a custom tokio executor. This avoids the SQLx/rusqlite conflict that apalis would introduce, the missing SQLite backend in tokio-cron-scheduler, and the external dependency on the `cron` crate. The custom parser handles expression validation and next-fire-time computation; the executor is ~50 lines of tokio timer logic.

**Scope estimate:**

| Metric | Estimate |
|--------|----------|
| New Rust files | 2 in nous-core (`schedule_db.rs`, `scheduler.rs`), tool additions in nous-mcp |
| New lines of code | ~1,200 across all 3 phases |
| New SQLite tables | 2 (`schedules`, `schedule_runs`) |
| New MCP tools | 14 (7 in Phase 1, 4 in Phase 2, 3 in Phase 3) |
| New dependencies | (none — custom cron parser) |
| Migration count | 2 new entries in the `MIGRATIONS` array |

## 2. Current Architecture

Nous is a Rust MCP memory server organized into 4 crates:

```
nous-shared ── types, SQLite helpers (open_connection, run_migrations), XDG paths, error types
    |
nous-core  ── MemoryDb, WriteChannel (mpsc + oneshot), ReadPool (semaphore + connection vec),
    |          EmbeddingBackend trait, Chunker, CategoryClassifier
    |
nous-mcp   ── NousServer struct, 15 MCP tool handlers via rmcp #[tool_router], stdio + HTTP
    |          transport, Config (TOML with env overrides), CLI entry point
    |
nous-otlp  ── OTLP gRPC/HTTP receiver, separate SQLite DB for logs/traces/metrics
```

**Single-writer pattern.** All mutations flow through `WriteChannel`, which wraps an `mpsc::channel<WriteOp>` (capacity 256). A background `write_worker` task drains up to `BATCH_LIMIT` (32) operations per iteration, executes them inside a single SQLite transaction via `unchecked_transaction()`, and sends results back through per-operation `oneshot::Sender` channels. The `WriteOp` enum currently has 10 variants (Store, Update, Forget, Relate, Unrelate, Unarchive, CategorySuggest, StoreChunks, DeleteChunks, LogAccess).

**Read pool.** `ReadPool` maintains a fixed-size pool (default 4) of read-only SQLite connections (`PRAGMA query_only = ON`), guarded by an `Arc<Semaphore>`. Each read borrows a connection, runs the closure on `spawn_blocking`, and returns the connection to the pool. `ReadPool` does not currently implement `Clone`; since all its fields are `Arc`-wrapped, adding `#[derive(Clone)]` is required for the scheduler to share a pool handle with spawned execution tasks.

**MCP tools.** `NousServer` holds `WriteChannel`, `ReadPool`, `Arc<dyn EmbeddingBackend>`, `CategoryClassifier`, `Chunker`, and `Config`. The `#[tool_router]` macro on the impl block registers 15 tools (memory_store, memory_recall, memory_search, memory_context, memory_forget, memory_unarchive, memory_update, memory_relate, memory_unrelate, memory_category_suggest, memory_workspaces, memory_tags, memory_stats, memory_schema, memory_sql).

**Migrations.** The `MIGRATIONS` array in `crates/nous-core/src/db.rs` contains ordered DDL strings. `run_migrations()` from nous-shared applies them sequentially. Current tables: models, workspaces, categories, memories, tags, memory_tags, relationships, access_log, memories_fts (FTS5), memory_chunks, memory_embeddings (vec0).

## 3. Proposed Changes

### 3a. SQLite Schema

Two new tables added as entries in the `MIGRATIONS` array in `crates/nous-core/src/db.rs`:

```sql
-- Migration N+1: schedules
CREATE TABLE IF NOT EXISTS schedules (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    cron_expr TEXT NOT NULL,
    timezone TEXT NOT NULL DEFAULT 'UTC',
    enabled INTEGER NOT NULL DEFAULT 1,
    action_type TEXT NOT NULL CHECK (action_type IN ('mcp_tool', 'shell', 'http')),
    action_payload TEXT NOT NULL,  -- JSON blob
    desired_outcome TEXT DEFAULT NULL,
    max_retries INTEGER NOT NULL DEFAULT 3,
    timeout_secs INTEGER,
    max_output_bytes INTEGER NOT NULL DEFAULT 65536,
    max_runs INTEGER NOT NULL DEFAULT 100,
    next_run_at INTEGER,  -- epoch seconds, recomputed after each execution
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_schedules_enabled_next ON schedules(enabled, next_run_at);
CREATE INDEX IF NOT EXISTS idx_schedules_name ON schedules(name);
```

```sql
-- Migration N+2: schedule_runs
CREATE TABLE IF NOT EXISTS schedule_runs (
    id TEXT NOT NULL PRIMARY KEY,
    schedule_id TEXT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    status TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'completed', 'failed', 'timeout', 'skipped')),
    exit_code INTEGER,
    output TEXT,
    error TEXT,
    attempt INTEGER NOT NULL DEFAULT 1,
    duration_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_runs_schedule_started
    ON schedule_runs(schedule_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_runs_status ON schedule_runs(status);
```

**Timestamp convention.** The schedule tables use epoch seconds (`INTEGER`) rather than the ISO 8601 strings (`TEXT`) used in existing tables (memories, categories, etc.). This is deliberate: the scheduler performs arithmetic on `next_run_at` (comparison, subtraction for sleep duration) on every loop iteration. Epoch seconds avoid repeated parsing overhead and simplify `ORDER BY` queries. User-facing MCP tool responses format these as ISO 8601 for consistency with the rest of the API.

**`action_payload` JSON formats by action_type:**

| action_type | Payload shape |
|-------------|---------------|
| `mcp_tool` | `{"tool": "memory_search", "args": {"query": "stale", "since": "-30d"}}` |
| `shell` | `{"command": "df -h", "working_dir": "/tmp"}` |
| `http` | `{"method": "POST", "url": "https://...", "headers": {}, "body": "..."}` |

**WriteOp extensions** in `crates/nous-core/src/channel.rs`:

```rust
pub enum WriteOp {
    // ... existing 10 variants ...
    CreateSchedule(Schedule, oneshot::Sender<Result<String>>),
    UpdateSchedule(String, SchedulePatch, oneshot::Sender<Result<bool>>),
    DeleteSchedule(String, oneshot::Sender<Result<bool>>),
    RecordRun(ScheduleRun, oneshot::Sender<Result<String>>),
    UpdateRun(String, RunPatch, oneshot::Sender<Result<bool>>),
    ComputeNextRun(String, oneshot::Sender<Result<()>>),
}
```

The `write_worker` match block gains 6 new arms that delegate to `ScheduleDb::*_on(&tx, ...)` static methods, following the same pattern as `MemoryDb::store_on`, `MemoryDb::update_on`, etc.

### 3b. Scheduler Executor

A single tokio task spawned during `NousServer::new()`. The loop queries the soonest enabled schedule, sleeps until its `next_run_at`, executes, records the run, and recomputes.

```rust
// crates/nous-core/src/scheduler.rs (pseudocode)
pub struct Scheduler {
    write_channel: WriteChannel,
    read_pool: ReadPool,
    notify: Arc<tokio::sync::Notify>,  // wake on schedule create/update/delete
}

impl Scheduler {
    pub fn spawn(wc: WriteChannel, rp: ReadPool) -> (Arc<Notify>, JoinHandle<()>) {
        let notify = Arc::new(Notify::new());
        let scheduler = Scheduler { write_channel: wc, read_pool: rp, notify: notify.clone() };
        let handle = tokio::spawn(scheduler.run());
        (notify, handle)
    }

    async fn run(self) {
        loop {
            // 1. Query soonest schedule
            let next = self.read_pool.with_conn(|conn| {
                ScheduleDb::next_pending(conn)  // SELECT ... WHERE enabled=1 ORDER BY next_run_at LIMIT 1
            }).await;

            match next {
                Ok(Some(schedule)) => {
                    let now = Utc::now().timestamp();
                    let delay = (schedule.next_run_at - now).max(0) as u64;

                    // 2. Sleep until fire time, or wake early on notify
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(delay)) => {
                            // 3. Advance next_run_at BEFORE spawning to prevent re-trigger
                            self.write_channel.compute_next_run(schedule.id.clone()).await;

                            // 4. Execute in a spawned task (non-blocking)
                            let wc = self.write_channel.clone();
                            let rp = self.read_pool.clone();
                            tokio::spawn(async move {
                                execute_schedule(&schedule, &wc, &rp).await;
                            });
                        }
                        _ = self.notify.notified() => {
                            // Schedule changed — re-query
                            continue;
                        }
                    }
                }
                Ok(None) => {
                    // No enabled schedules — park until notified
                    self.notify.notified().await;
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
}
```

**Startup recovery:** On boot, the scheduler recomputes `next_run_at` for all enabled schedules from their `cron_expr`. Any schedule whose `next_run_at` is in the past fires immediately (catch-up for missed ticks during downtime).

### 3c. Action Execution

Each action type dispatches through a dedicated async function inside `execute_schedule()`:

**`mcp_tool`** -- Deserialize `action_payload` into `{"tool": String, "args": Value}`. Call the corresponding tool handler function directly (these are free functions that accept `WriteChannel` and `ReadPool` as parameters), bypassing MCP protocol overhead. The tool executes in the same process. Output is the JSON-serialized `CallToolResult`.

**`shell`** -- Deserialize `action_payload` into `{"command": String, "working_dir": Option<String>}`. Spawn via `tokio::process::Command` with stdout/stderr captured. Enforce `timeout_secs` via `tokio::time::timeout`. Truncate output to `max_output_bytes`. Shell actions are gated behind the `allow_shell` config flag (see Section 5); if disabled, the scheduler records a `failed` run with error `"shell actions disabled by configuration"`.

**`http`** -- Deserialize `action_payload` into `{"method": String, "url": String, "headers": Map, "body": Option<String>}`. Execute via `reqwest::Client` (already a transitive dependency). Enforce `timeout_secs`. Store response status code as `exit_code` (0 for 2xx, 1 otherwise) and response body as `output`.

**Concurrency policy:** If a schedule fires while a previous execution is still running, the new execution is skipped with `status = 'skipped'` in `schedule_runs`. A future `allow_overlap` flag on the schedule can relax this.

**`mcp_tool` self-invocation safety:** Tool calls execute in a spawned task that receives cloned `WriteChannel` and `ReadPool` handles, not a reference to `NousServer` itself. This prevents deadlock from re-entering the MCP handler's dispatch loop.

### 3d. Error Handling and Retry Strategy

**Retry logic.** On failure, the scheduler re-executes up to `max_retries` times (default 3) with exponential backoff: 2^attempt seconds (2s, 4s, 8s). Each attempt is a separate `schedule_runs` row with incrementing `attempt` number. After exhausting retries, the schedule remains enabled -- it will fire again at its next cron tick.

**Timeout enforcement.** Every action runs inside `tokio::time::timeout(Duration::from_secs(schedule.timeout_secs))`. On timeout, the run records `status = 'timeout'` and the action task is dropped (shell processes are killed via `Child::kill()`).

**Run retention.** After inserting a new run, the scheduler deletes the oldest rows exceeding `max_runs` per schedule: `DELETE FROM schedule_runs WHERE schedule_id = ? AND id NOT IN (SELECT id FROM schedule_runs WHERE schedule_id = ? ORDER BY started_at DESC LIMIT ?)`. Default `max_runs` is 100.

**Error recording.** Every run captures: `status` (running/completed/failed/timeout/skipped), `exit_code` (process exit code for shell, HTTP status for http, 0/1 for mcp_tool), `output` (truncated to `max_output_bytes`), `error` (exception/panic message), and `duration_ms`.

### 3e. Desired Outcome Field

The `desired_outcome` column on `schedules` (TEXT, nullable) defines what success looks like beyond exit code 0. When NULL, success = exit code 0 only. When set, the scheduler evaluates the outcome after execution completes.

**Evaluation strategies**, inferred from the `desired_outcome` string format:

| Format | Strategy | Example |
|--------|----------|---------|
| Plain string | `output.contains(desired_outcome)` | `"memory count below 10000"` |
| `/regex/` | `Regex::new(pattern).is_match(output)` | `"/^HTTP 200/"` |
| `llm:description` | Pass output + description to LLM for boolean judgment (future) | `"llm:no anomalies in the health check output"` |

**Evaluation flow:**
1. Action executes, producing `output` and `exit_code`.
2. If `exit_code != 0`, status is `failed` regardless of `desired_outcome`.
3. If `exit_code == 0` and `desired_outcome` is NULL, status is `completed`.
4. If `exit_code == 0` and `desired_outcome` is set, evaluate. Match = `completed`; mismatch = `failed` with `error = "outcome mismatch: expected <desired_outcome>, got <output summary>"`.

**MCP exposure:** `schedule_create` and `schedule_update` accept an optional `desired_outcome` parameter. `schedule_get` and `schedule_runs` return it in responses. `schedule_health` reports schedules where exit code 0 but outcome mismatched as `outcome_mismatch` status.

### 3f. Custom Cron Parser

A hand-written cron expression parser in `crates/nous-core/src/cron_parser.rs` (new file) that handles 5-field standard cron syntax: minute (0–59), hour (0–23), day-of-month (1–31), month (1–12), day-of-week (0–6, Sunday = 0).

**Parser architecture:** A two-stage tokenizer + field validator. Stage 1 splits the expression into 5 whitespace-delimited fields and tokenizes each field into atoms (literal values, ranges, lists, steps). Stage 2 validates each atom against the field's legal range and expands it into a set of matching values.

**Supported syntax:**

| Syntax | Example | Meaning |
|--------|---------|---------|
| `*` | `* * * * *` | Every valid value for the field |
| Specific value | `30 * * * *` | Minute 30 only |
| Range | `1-5 * * * *` | Minutes 1 through 5 |
| List | `1,15,30 * * * *` | Minutes 1, 15, and 30 |
| Step (wildcard) | `*/5 * * * *` | Every 5th value from field minimum |
| Step (range) | `1-10/2 * * * *` | Values 1, 3, 5, 7, 9 |

**`next_run()` algorithm:** Given a reference time, iterate forward by incrementing the smallest non-matching field until all 5 fields match. The algorithm walks: minute → hour → day-of-month → month, resetting smaller fields when a larger field advances. Day-of-week is checked as a filter on the computed day-of-month (standard cron OR-semantics when both day-of-month and day-of-week are specified).

**Missed-run handling:** On startup, if `next_run_at < now` for any enabled schedule, fire immediately (catch-up for missed ticks during downtime). This is the same behavior described in Section 3b (Scheduler Executor, startup recovery).

**Module location:** `crates/nous-core/src/cron_parser.rs` (new file). Public API: `CronExpr::parse(expr: &str) -> Result<CronExpr, CronParseError>` and `CronExpr::next_run(&self, after: DateTime<Tz>) -> Option<DateTime<Tz>>`.

### 3g. Custom Cron Parser Testing Strategy

Dedicated test suite for the custom cron parser, estimated at ~25 unit tests. These are separate from the integration tests in Section 7, which test the scheduler-level behavior.

**Standard expression tests (~5 tests):**

| Test | Validates |
|------|----------|
| `parse_every_minute` | `* * * * *` parses; `next_run` returns the next minute boundary |
| `parse_hourly` | `0 * * * *` fires at minute 0 of the next hour |
| `parse_daily` | `0 0 * * *` fires at midnight |
| `parse_monthly` | `0 0 1 * *` fires on the 1st of the next month |
| `parse_yearly` | `0 0 1 1 *` fires on Jan 1 of the next year |

**Edge case tests (~10 tests):**

| Test | Validates |
|------|----------|
| `dst_spring_forward` | 2:30 AM expression skips to 3:00 AM on spring-forward day (2:00–3:00 doesn't exist) |
| `dst_fall_back` | 1:30 AM expression fires once (not twice) during fall-back overlap |
| `leap_year_feb_29` | Day-of-month 29 + month 2 fires on Feb 29 in leap years |
| `non_leap_year_feb_29` | Day-of-month 29 + month 2 skips to next leap year's Feb 29 |
| `month_end_31st` | Day 31 skips months with 30 days (April, June, September, November) |
| `month_end_30th_feb` | Day 30 skips February entirely |
| `midnight_boundary` | `59 23 * * *` at 23:58 returns 23:59 same day, not next day |
| `year_boundary` | `0 0 1 1 *` at Dec 31 23:59 returns Jan 1 00:00 next year |
| `dow_and_dom_or_semantics` | Both day-of-month and day-of-week specified: fires on either match (OR logic) |
| `all_fields_constrained` | `30 14 15 6 3` — fires only when minute=30, hour=14, day=15, month=June, AND it's a Wednesday (or the OR-semantics interpretation) |

**Boundary condition tests (~5 tests):**

| Test | Validates |
|------|----------|
| `field_min_values` | `0 0 1 1 0` (all field minimums) parses correctly |
| `field_max_values` | `59 23 31 12 6` (all field maximums) parses correctly |
| `overlapping_range_and_list` | `1-5,3-7 * * * *` merges into `{1,2,3,4,5,6,7}` |
| `step_exceeds_range` | `*/60 * * * *` for minute field (0-59) — fires once at 0 only |
| `single_value_range` | `5-5 * * * *` equivalent to `5 * * * *` |

**Rejection tests (~3 tests):**

| Test | Validates |
|------|----------|
| `reject_empty_expr` | Empty string returns `CronParseError` |
| `reject_too_few_fields` | `* * *` (3 fields) returns `CronParseError` |
| `reject_out_of_range` | `60 * * * *` (minute 60) returns `CronParseError` |

**Property-based tests (~2 tests):**

| Test | Validates |
|------|----------|
| `next_run_always_future` | For any valid expression and reference time, `next_run()` returns a time strictly after the reference |
| `next_run_matches_expr` | For any valid expression, the returned time's fields all satisfy the expression's constraints |

## 4. MCP Tool Surface

### Phase 1: Core Schedule CRUD (7 tools)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_create` | `name`, `cron_expr`, `action_type`, `action_payload`, `timezone?`, `max_retries?`, `timeout_secs?`, `desired_outcome?` | Register a new cron schedule. Validates `cron_expr` via the built-in parser and rejects invalid expressions. Returns `{id, next_run_at}`. |
| `schedule_list` | `enabled?`, `action_type?`, `limit?` | List schedules with `next_run_at`, ordered by next fire time. Default limit 50. |
| `schedule_get` | `id` | Return full schedule detail including last 10 runs. |
| `schedule_update` | `id`, `name?`, `cron_expr?`, `action_payload?`, `enabled?`, `max_retries?`, `timeout_secs?`, `desired_outcome?` | Modify a schedule. Recomputes `next_run_at` if `cron_expr` changes. Notifies the scheduler loop. |
| `schedule_delete` | `id` | Remove schedule and all its run history (CASCADE). |
| `schedule_pause` | `id`, `duration_secs?`, `reason?` | Set `enabled = 0`. If `duration_secs` is provided, spawn a delayed re-enable task. |
| `schedule_resume` | `id` | Set `enabled = 1` and recompute `next_run_at`. |

### Phase 2: Execution and History (4 tools)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_runs` | `schedule_id?`, `status?`, `since?`, `limit?` | Query execution history across all or one schedule. Returns `[{id, schedule_id, started_at, finished_at, status, exit_code, duration_ms}]`. |
| `schedule_run_get` | `run_id` | Return full run detail including `output` and `error` fields. |
| `schedule_trigger` | `id` | Execute a schedule immediately, outside its cron cadence. Records a normal run. |
| `schedule_health` | -- | Summary: total schedules, active count, failing count (consecutive failures > 0), outcome mismatches, next 5 upcoming fires. |

### Phase 3: Advanced (3 tools)

| Tool | Parameters | Description |
|------|-----------|-------------|
| `schedule_discover` | -- | Scan system crontabs (`crontab -l`), return found entries as structured data for agent-driven registration. |
| `schedule_export` | `format?` | Export all schedules as JSON (default) or TOML. |
| `schedule_import` | `data` | Import schedules from JSON. Validates all `cron_expr` values before inserting. |

## 5. Configuration

**config.toml additions:**

```toml
[schedule]
enabled = true              # master switch for the scheduler loop
allow_shell = false          # gate shell action_type (security-sensitive)
allow_http = true            # gate http action_type
max_concurrent = 4           # max simultaneous action executions
default_timeout_secs = 300   # fallback when schedule.timeout_secs is NULL
```

**Config struct** in `crates/nous-mcp/src/config.rs`:

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub allow_shell: bool,
    pub allow_http: bool,
    pub max_concurrent: usize,
    pub default_timeout_secs: u64,
}
```

**CLI flags** on the `serve` command:

| Flag | Effect |
|------|--------|
| `--allow-shell-schedules` | Override `schedule.allow_shell = true` at runtime |
| `--no-scheduler` | Override `schedule.enabled = false` at runtime |

**Environment variable overrides:**

| Variable | Config field |
|----------|-------------|
| `NOUS_SCHEDULE_ALLOW_SHELL` | `schedule.allow_shell` |
| `NOUS_SCHEDULE_ENABLED` | `schedule.enabled` |

## 6. OTLP Integration

Each schedule execution emits telemetry to the existing nous-otlp receiver:

**Trace spans:**

| Span name | Attributes | When |
|-----------|-----------|------|
| `schedule.run` | `schedule.id`, `schedule.name`, `schedule.cron_expr`, `action_type`, `attempt` | Wraps the full execution including retries |
| `schedule.action` | `action_type`, `exit_code`, `duration_ms` | Wraps a single action attempt |

**Span events:** `start`, `retry` (with `attempt` and `backoff_secs`), `complete`, `fail` (with `error`), `timeout`, `outcome_mismatch`.

**Metrics:**

| Metric | Type | Labels |
|--------|------|--------|
| `schedule.run.duration_ms` | Histogram | `schedule_id`, `action_type`, `status` |
| `schedule.run.total` | Counter | `schedule_id`, `action_type`, `status` |
| `schedule.active.count` | Gauge | -- |

**Correlation.** The existing `trace_id` and `session_id` fields on memories enable linking: a scheduled `memory_search` + `memory_forget` pipeline records the `trace_id` from its span, which appears on any memories it modifies. Querying OTLP traces by `schedule.id` shows execution history; querying memories by `trace_id` shows what the schedule changed.

## 7. Testing Strategy

**Unit tests** in `crates/nous-core/src/schedule_db.rs`:

| Test | Validates |
|------|----------|
| `create_and_get_schedule` | Round-trip: insert schedule, read back, verify all fields match |
| `update_schedule_recomputes_next_run` | Changing `cron_expr` updates `next_run_at` |
| `delete_cascades_runs` | Deleting a schedule removes its `schedule_runs` rows |
| `record_run_enforces_max_runs` | Inserting run N+1 deletes the oldest when `max_runs = N` |
| `cron_expr_validation_rejects_invalid` | Invalid expressions (empty, too few fields, out-of-range values) rejected at `schedule_create` time |
| `cron_expr_standard_patterns` | Every-minute (`* * * * *`), hourly (`0 * * * *`), daily (`0 0 * * *`), monthly (`0 0 1 * *`), yearly (`0 0 1 1 *`) expressions parse and compute correct next-run times |
| `cron_expr_ranges_lists_steps` | Range (`1-5`), list (`1,3,5`), step (`*/5`, `1-10/2`) syntax parses correctly across all 5 fields |
| `cron_expr_dst_transitions` | Spring-forward (2:30 AM doesn't exist) and fall-back (1:30 AM occurs twice) produce correct next-run times |
| `cron_expr_leap_year` | Feb 29 schedule fires on leap years, skips gracefully on non-leap years |
| `cron_expr_month_boundaries` | 31st-of-month schedule handles months with 30 days (skips to next valid month) |
| `desired_outcome_exact_match` | Plain string outcome evaluated via `contains()` |
| `desired_outcome_regex_match` | `/pattern/` outcome evaluated via `Regex::is_match()` |
| `next_pending_returns_soonest` | `next_pending()` returns the schedule with the lowest `next_run_at` among enabled schedules |

**Integration tests** in `crates/nous-core/tests/scheduler_integration.rs`:

| Test | Validates |
|------|----------|
| `scheduler_fires_on_time` | Create schedule with 1-second cron, assert run record appears within 2 seconds |
| `scheduler_skips_overlap` | Schedule with long-running action; second tick produces `status = 'skipped'` |
| `scheduler_retries_on_failure` | Action that fails once then succeeds; verify `attempt = 2` on the successful run |
| `scheduler_respects_timeout` | Action that sleeps beyond `timeout_secs`; verify `status = 'timeout'` |
| `notify_wakes_scheduler` | Create schedule while scheduler is parked; verify it fires without waiting for poll |
| `shell_action_gated` | With `allow_shell = false`, shell action records `failed` with config error |

**End-to-end tests** in `crates/nous-mcp/tests/schedule_e2e.rs`:

| Test | Validates |
|------|----------|
| `create_list_get_delete_lifecycle` | Full CRUD through MCP tool handlers |
| `trigger_and_check_runs` | `schedule_trigger` → `schedule_runs` → verify output |
| `health_reports_failures` | Create failing schedule, trigger, verify `schedule_health` reports it |
| `pause_resume_cycle` | Pause with duration, verify re-enable after expiry |

## 8. Risks and Open Questions

### Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Missed ticks on process restart | Medium | Low | Recompute `next_run_at` from `cron_expr` on startup; fire immediately if overdue |
| Long-running action blocks scheduler | Medium | Medium | Execute actions in spawned tasks with `max_concurrent` semaphore; timeout enforcement via `tokio::time::timeout` |
| `mcp_tool` self-invocation deadlock | Low | High | Execute tool calls through cloned `WriteChannel`/`ReadPool` in a spawned task, never re-enter the MCP dispatch loop |
| SQLite write contention from run records | Low | Low | All writes flow through `WriteChannel` (existing batching pattern, limit 32). Run inserts are small rows. |
| Shell action privilege escalation | Medium | High | Gated behind `allow_shell = false` default + `--allow-shell-schedules` CLI flag. Shell actions run as the nous process user with no sandboxing. |
| Cron expression parsing edge cases (DST, leap seconds) | Low | Medium | Custom parser tested with dedicated DST transition tests (spring-forward/fall-back), leap-year boundary tests (Feb 29), and month-end boundary tests (31st of 30-day months). Property-based tests verify round-trip correctness against a reference set. Timezone stored per-schedule. |
| Schema migration on encrypted DB | Low | Medium | New migrations follow the existing `run_migrations()` path in nous-shared, which handles SQLCipher transparently |
| Run retention bloat | Low | Low | `max_runs` (default 100) enforced on insert; oldest runs deleted per-schedule |

### Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| Should `mcp_tool` actions have access to the full `NousServer` or only `WriteChannel` + `ReadPool`? | Full server (enables embedding, classification) vs. channel-only (simpler, safer) | Start with channel-only for Phase 1; expand to full server access if needed |
| Should `schedule_pause` with `duration_secs` use a separate tokio timer or a DB-level `resume_at` column? | Timer (precise, lost on restart) vs. column (persisted, checked on scheduler loop) | DB column -- survives restarts without additional timer bookkeeping |
| Should the `desired_outcome` regex strategy support capture groups for structured extraction? | Simple `is_match` vs. capture-based extraction | Start with `is_match`; capture groups add complexity with unclear use cases |
| How should the scheduler handle clock skew (e.g., NTP adjustments)? | Ignore (accept occasional double-fire or skip) vs. monotonic clock checks | Ignore -- nous is single-node, local-first; NTP skew is rare and bounded |

## 9. Phased Implementation Plan

### Phase 1: Core (target: ~500 lines)

| Task | File(s) | Effort |
|------|---------|--------|
| Implement custom cron expression parser (`cron_parser.rs`) | `nous-core/src/cron_parser.rs` (new) | 6 hr |
| Unit tests for cron parser (~25 tests) | `nous-core/src/cron_parser.rs`, `nous-core/tests/cron_parser_tests.rs` | 4 hr |
| Add `schedules` and `schedule_runs` migrations to `MIGRATIONS` | `nous-core/src/db.rs` | 30 min |
| Implement `ScheduleDb` with CRUD + `next_pending()` | `nous-core/src/schedule_db.rs` (new) | 3 hr |
| Extend `WriteOp` with 6 schedule variants + `write_worker` match arms | `nous-core/src/channel.rs` | 1 hr |
| Implement `Scheduler::spawn()` and timer loop | `nous-core/src/scheduler.rs` (new) | 2 hr |
| Implement `mcp_tool` action dispatch | `nous-core/src/scheduler.rs` | 1 hr |
| Add `ScheduleConfig` to `Config` | `nous-mcp/src/config.rs` | 30 min |
| Implement 7 MCP tool handlers (`schedule_create` through `schedule_resume`) | `nous-mcp/src/tools.rs` | 3 hr |
| Register tools in `NousServer` via `#[tool_router]`, spawn scheduler on startup | `nous-mcp/src/server.rs` | 1 hr |
| Unit tests for `ScheduleDb` (8 tests) | `nous-core/src/schedule_db.rs` | 2 hr |
| Integration tests for scheduler loop (6 tests) | `nous-core/tests/scheduler_integration.rs` | 3 hr |
| **Phase 1 total** | | **~27 hr** |

### Phase 2: Execution History (target: ~400 lines)

| Task | File(s) | Effort |
|------|---------|--------|
| Implement `schedule_runs`, `schedule_run_get`, `schedule_trigger`, `schedule_health` handlers | `nous-mcp/src/tools.rs` | 3 hr |
| Register Phase 2 tools in `NousServer` | `nous-mcp/src/server.rs` | 30 min |
| Implement desired outcome evaluation (exact match + regex) | `nous-core/src/scheduler.rs` | 2 hr |
| Add OTLP span emission per execution | `nous-core/src/scheduler.rs` | 2 hr |
| E2E tests (4 tests) | `nous-mcp/tests/schedule_e2e.rs` | 2 hr |
| **Phase 2 total** | | **~10 hr** |

### Phase 3: Advanced (target: ~300 lines)

| Task | File(s) | Effort |
|------|---------|--------|
| Implement `shell` action dispatch with `allow_shell` gate | `nous-core/src/scheduler.rs` | 2 hr |
| Implement `http` action dispatch via `reqwest` | `nous-core/src/scheduler.rs` | 1.5 hr |
| Add `--allow-shell-schedules` and `--no-scheduler` CLI flags | `nous-mcp/src/main.rs` (arg defs), `nous-mcp/src/server.rs` (override wiring) | 30 min |
| Implement `schedule_discover`, `schedule_export`, `schedule_import` handlers | `nous-mcp/src/tools.rs` | 3 hr |
| Register Phase 3 tools in `NousServer` | `nous-mcp/src/server.rs` | 30 min |
| Tests for shell/http actions + discovery | `nous-core/tests/`, `nous-mcp/tests/` | 2 hr |
| **Phase 3 total** | | **~10 hr** |

## 10. Demo Deliverable

A live demonstration of the full cron schedule lifecycle, proving end-to-end functionality before handoff.

**Demo format:** CLI walkthrough script (automated bash script that runs through the lifecycle with commentary) plus a recorded terminal session (asciinema or similar). The script is self-contained and can be re-run by any developer to verify functionality.

**Demo scenarios:**

| Scenario | Action type | What it demonstrates |
|----------|------------|---------------------|
| Memory cleanup schedule | `mcp_tool` | Create a recurring `memory_search` + `memory_forget` pipeline for stale entries (>30 days, no access). Watch it fire, verify entries are cleaned up, check `schedule_runs` history. |
| Health check schedule | `mcp_tool` | Create a periodic `memory_stats` schedule with `desired_outcome` validation (e.g., `"memory count below 10000"`). Trigger it, verify outcome evaluation, inspect `schedule_health` output. |
| System disk monitoring | `shell` | Create a `df -h` schedule (requires `--allow-shell-schedules`). Watch it fire, inspect output in `schedule_run_get`, verify truncation at `max_output_bytes`. |

**Full lifecycle per scenario:**
1. `schedule_create` — register the schedule with cron expression
2. `schedule_list` — verify it appears with correct `next_run_at`
3. `schedule_trigger` — fire immediately (don't wait for cron tick)
4. `schedule_runs` — verify run recorded with expected status
5. `schedule_run_get` — inspect full output and duration
6. `schedule_pause` / `schedule_resume` — toggle enabled state
7. `schedule_health` — dashboard shows correct active/failing counts
8. `schedule_delete` — cleanup

**Success criteria:**
- All 3 demo scenarios run end-to-end without errors
- Output is self-explanatory (a developer unfamiliar with the codebase can follow the demo)
- Demo completes in under 5 minutes total
- Recorded terminal session is committed to the repository (`docs/demo/cron-demo.cast` or equivalent)

**Total estimated effort: ~47 hours across 3 phases.**
