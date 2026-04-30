# 07: Cross-Cutting Concerns

**Initiative:** INI-076  
**Status:** Draft  
**Author:** Technical Writer (paseo agent)  
**Date:** 2026-04-29  
**Relates to:** 01-system-architecture, 02-data-layer, 03-api-interfaces, 04-features-p0-p1, 05-features-p2-p4

Concerns that apply across every feature: how callers authenticate, how the system emits and ingests telemetry, how errors propagate from SQLite through MCP to the caller, how the test suite is structured, and how code reaches production.

---

## 1. Auth Model

### 1.1 Current State

Nous uses two identity mechanisms: API keys for external callers that reach the HTTP transport, and UUIDv7 agent IDs embedded in every memory row and room message. Both exist today. Namespace-based isolation scopes all memory queries to a `workspace_id` integer FK in the `memories` table (`crates/nous-core/src/db.rs:47`). The MCP transport runs over stdio, which establishes implicit trust — no credential exchange occurs because the calling process owns the file descriptor.

Authorization enforcement is currently absent at the HTTP daemon layer (`crates/nous-cli/src/daemon_api.rs`). The `/status`, `/rooms`, and `/memories/search` endpoints accept any request without token validation. This is acceptable for the single-user, single-machine deployment model but must change before multi-tenant or networked deployments.

### 1.2 Architecture

**Identity layers:**

| Layer | Identity carrier | Enforcement point | Status |
|-------|-----------------|-------------------|--------|
| MCP over stdio | None — implicit trust via process ownership | `crates/nous-cli/src/server.rs` | Active |
| HTTP daemon API | API key (header) | `daemon_router` middleware (not yet implemented) | Planned |
| Agent identity | UUIDv7 `agent_id` column in `memories`, `room_participants`, `room_messages` | SQL schema FK | Active |
| Namespace isolation | `workspace_id` INTEGER FK in `memories` | `WHERE workspace_id = ?` in all read queries | Active |

**Agent-to-agent trust model:**

Parent agents hold a UUIDv7 agent ID. Child agents inherit the parent's `namespace` scope. The parent can query child resources because both share the same `workspace_id`; child agents cannot query sibling namespaces because no cross-namespace `workspace_id` is issued. This is enforced structurally — no runtime capability check is needed.

```
  Namespace boundary (workspace_id = 42)
  ┌──────────────────────────────────────────────────────┐
  │                                                      │
  │  parent-agent (agent_id = 019...)                    │
  │        │                                             │
  │        ├──── memory row (workspace_id = 42)          │
  │        │                                             │
  │        └──── child-agent (agent_id = 01a...)         │
  │                    │                                 │
  │                    └──── memory row (workspace_id = 42)
  │                                                      │
  └──────────────────────────────────────────────────────┘

  Namespace boundary (workspace_id = 99)        ← not accessible
  ┌──────────────────────────────────────────────┐
  │  other-agent (agent_id = 01b...)             │
  └──────────────────────────────────────────────┘
```

**Namespace query enforcement:**

Every read query in `crates/nous-core/src/search.rs` and `crates/nous-core/src/db.rs` appends `m.workspace_id = ?N` when a workspace filter is present. The filter is assembled in a `conditions` vector and then joined into the WHERE clause at runtime:

```rust
// crates/nous-core/src/search.rs:462-464
if let Some(ws_id) = filters.workspace_id {
    conditions.push(format!("m.workspace_id = ?{}", params.len() + 1));
    params.push(Box::new(ws_id));
}
```

A query without `workspace_id` returns results across all workspaces — this is the superuser/admin path used by `admin re-embed` and `admin re-classify`. Normal agent calls always supply the resolved `workspace_id`.

BM25 search also respects this boundary. The FTS5 virtual table join adds the workspace filter before the `bm25(memories_fts, 10.0, 1.0, 0.5)` ranking expression runs, so relevance scoring never crosses namespace lines.

**MCP session authentication:**

The rmcp server runs on stdio. When Claude or another agent spawns `nous serve`, the stdio transport is exclusive to that process pair. No token exchange occurs because the OS enforces process ownership. Future HTTP transport (planned) will require `Authorization: Bearer <api-key>` on every request, validated in an Axum middleware layer before the request reaches any handler.

**Future roadmap:**

| Milestone | Mechanism | Blocked on |
|-----------|-----------|------------|
| HTTP API key validation | Axum middleware layer, `X-Api-Key` header | HTTP transport stabilization |
| JWT for cross-node requests | HS256 JWTs, shared secret per org | Multi-node deployment mode |
| Role-based access control | `api_keys` table with `role` column (`admin`, `read`, `write`) | API key infrastructure |
| Per-resource ACL | `resource_permissions` table, agent-ID-to-resource mapping | RBAC milestone |

### 1.3 Integration Points

- **Data layer** (`02-data-layer.md`): `workspace_id` is resolved from `WorkspacePath` before every `WriteChannel` write. The `ReadPool` passes the resolved integer through all SQL WHERE clauses. No caller can omit it; the query builder panics at construction time if `workspace_id` is missing for a namespaced query.
- **MCP API** (`03-api-interfaces.md`): `agent_id` is an optional parameter on `store_memory`, `post_message`, and task-assignment tools. The server accepts `None` for anonymous callers.
- **Room/task features** (`04-features-p0-p1.md`): `room_participants` table records `(room_id, agent_id, role)`. Task `assignee_id` stores the UUIDv7 of the agent responsible.
- **Scheduler** (`05-features-p2-p4.md`): Scheduled actions fire with the `workspace_id` of the schedule creator, ensuring cron-triggered writes land in the correct namespace.

### 1.4 Open Questions

1. **JWT vs. API key for HTTP transport**: API keys are simpler to issue and rotate; JWTs enable expiry without server-side revocation. Which properties matter more for the initial multi-user deployment?
2. **Role-based access control scope**: Should role enforcement (read-only vs. read-write) apply per namespace or per resource type (memories, rooms, tasks separately)?
3. **Key rotation impact on encrypted DB**: SQLCipher re-encryption (`admin rotate-key`) changes the database key. Does the API key derive from or relate to the SQLCipher key, or are they independent secrets?
4. **Child agent trust elevation**: Can a child agent explicitly request access to a sibling namespace? If so, what approval mechanism applies?

---

## 2. Observability (OTLP Integration)

### 2.1 Current State

`nous-otlp` is a standalone binary in the workspace (`crates/nous-otlp/`). It accepts OTLP HTTP/protobuf and OTLP HTTP/JSON on port 4318, decodes the payloads, and writes them to a dedicated SQLite database (`~/.cache/nous/otlp.db` by default). It is **not embedded** in the main `nous-cli` process — operators run it separately alongside the Nous daemon.

The main Nous process emits structured logs via the `tracing` crate with JSON output. It does not yet push spans or metrics to its own OTLP endpoint at runtime, though the `trace_id` and `session_id` columns in the `memories` table and the `log_events` table create a correlation join path.

### 2.2 Architecture

**OTLP ingestion pipeline:**

```
  External emitter (agent, SDK, test harness)
        │
        │  HTTP POST /v1/traces   (protobuf or JSON)
        │  HTTP POST /v1/logs
        │  HTTP POST /v1/metrics
        ▼
  nous-otlp (port 4318)
  ┌────────────────────────────────────────────────────────┐
  │                                                        │
  │  Axum router  (crates/nous-otlp/src/server.rs)         │
  │        │                                               │
  │        │  Content-Type: application/x-protobuf         │
  │        │      → prost decode                           │
  │        │  Content-Type: application/json               │
  │        │      → serde_json decode                      │
  │        ▼                                               │
  │  decode.rs  →  LogEvent / Span / Metric structs        │
  │        │                                               │
  │        ▼                                               │
  │  OtlpDb::store_logs / store_spans / store_metrics      │
  │  (crates/nous-otlp/src/db.rs)                         │
  │                                                        │
  └────────────────────────────────────────────────────────┘
        │
        ▼
  ~/.cache/nous/otlp.db  (SQLite, WAL mode)
  ┌─────────────────────────────────────────────────────────┐
  │  log_events (timestamp, severity, body, session_id,     │
  │              trace_id, span_id, resource_attrs, ...)    │
  │                                                         │
  │  spans      (trace_id, span_id, parent_span_id, name,   │
  │              start_time, end_time, span_attrs, ...)     │
  │                                                         │
  │  metrics    (name, type, data_points_json,              │
  │              resource_attrs, timestamp)                 │
  └─────────────────────────────────────────────────────────┘
```

**Index strategy in `otlp.db`:**

| Index | Column(s) | Query pattern |
|-------|-----------|---------------|
| `idx_log_events_session_id` | `log_events.session_id` | Fetch all logs for a session |
| `idx_log_events_trace_id` | `log_events.trace_id` | Join logs to spans by trace |
| `idx_log_events_timestamp` | `log_events.timestamp` | Time-range scans |
| `idx_spans_trace_id` | `spans.trace_id` | Reconstruct a full trace |
| `idx_spans_start_time` | `spans.start_time` | Latency histogram queries |
| `idx_metrics_name` | `metrics.name` | Fetch a named counter or gauge |
| `idx_metrics_timestamp` | `metrics.timestamp` | Time-range aggregations |

**Structured logging in the main process:**

The `tracing` crate instruments key operations. `RUST_LOG=info` (set in `systemd/nous-cli.service`) enables info-level output to the journal. Span contexts propagate through async tasks via `tracing::Span::enter()`, so log lines for a single operation share a parent span ID.

**Structured logging in the main process (expanded):**

`RUST_LOG=info` controls `tracing` subscriber output. The subscriber is initialized in `nous-cli/src/main.rs` before the daemon router starts. Useful log levels by component:

| Component | Recommended level | Key events logged |
|-----------|-------------------|-------------------|
| WriteChannel | `debug` | Batch size, flush latency |
| ReadPool | `trace` | Connection acquire time |
| Embedding pipeline | `info` | Model load, inference duration |
| Scheduler | `info` | Schedule fire, action result |
| OTLP server | `warn` (default) | Decode errors, store failures |
| Daemon API | `info` | Request path, status code |

Setting `RUST_LOG=nous_core=debug,nous_cli=info` reduces noise from the HTTP layer while retaining write-path visibility.

**`nous-otlp` CLI subcommands:**

Beyond `serve`, `nous-otlp` ships two query subcommands for local introspection:

- `nous-otlp logs <session_id> [--limit 100]` — fetch log events for a session, output as human-readable text or JSON/CSV
- `nous-otlp spans <trace_id> [--limit 100]` — fetch spans for a trace, enabling manual flamegraph reconstruction

Both subcommands resolve `db` path via `nous_shared::xdg::cache_dir()` when `--db` is omitted.

### 2.3 Integration Points

- **Shared `db_path` config**: Both `nous-cli` and `nous-otlp` resolve their database path through `nous_shared::xdg::cache_dir()`. The main DB lands at `~/.cache/nous/memory.db`; the OTLP DB at `~/.cache/nous/otlp.db`. The paths are independent — no file-locking conflicts occur.
- **`trace_id` / `session_id` correlation**: The `memories` table stores `trace_id TEXT` and `session_id TEXT` (`crates/nous-core/src/db.rs:50`). These match the `trace_id` and `session_id` columns in `otlp.db.log_events`, enabling a cross-DB join: given a memory row, retrieve all OTLP log lines from the same session.
- **Scheduler observability** (`05-features-p2-p4.md §5`): The scheduler can query `otlp.db` to surface latency data for schedule health checks. No implementation exists today; this is the planned integration path.
- **Daemon `/status` endpoint** (`crates/nous-cli/src/daemon_api.rs`): Returns `pid`, `uptime_secs`, and `version`. A health check endpoint at `/health` returning HTTP 200 is the next planned addition, enabling load-balancer and systemd watchdog integration.
- **Data layer** (`02-data-layer.md`): The `WriteChannel` batch size is an observable metric. Planned: emit a `write_channel.batch_size` histogram data point to `nous-otlp` when the write worker drains each batch.

### 2.4 Planned Metrics

The main Nous process does not yet emit metrics to `nous-otlp`. The following counters and histograms are planned, emitted via OTLP to `localhost:4318` when `nous-otlp` is running:

| Metric name | Type | Labels | What it measures |
|-------------|------|--------|-----------------|
| `nous.write_channel.batch_size` | Histogram | — | Number of ops drained per write worker wakeup |
| `nous.write_channel.flush_latency_ms` | Histogram | — | Time from batch dequeue to SQLite commit completion |
| `nous.memory.store.total` | Counter | `status: ok|error` | Total `store_memory` calls |
| `nous.memory.search.total` | Counter | `status: ok|error` | Total `search_memories` calls |
| `nous.embed.inference_ms` | Histogram | — | ONNX inference time per chunk |
| `nous.schedule.fires.total` | Counter | `action_type` | Total scheduler fires |
| `nous.schedule.action_errors.total` | Counter | `action_type` | Total failed scheduled actions |
| `nous.daemon.http.requests.total` | Counter | `path, status` | HTTP daemon API request volume |

### 2.5 Open Questions

1. **Push vs. pull for internal metrics**: Should the main Nous process push spans to `nous-otlp` at localhost:4318, or should `nous-otlp` expose a Prometheus scrape endpoint that pulls from `otlp.db`?
2. **Retention policy for `otlp.db`**: OTLP data accumulates without bound. What TTL applies — 7 days, 30 days, or configurable? Who triggers cleanup (daemon cron, `nous-otlp` itself, manual CLI)?
3. **Encryption**: `otlp.db` uses `open_connection` from `nous_shared::sqlite`, which supports SQLCipher. Should the OTLP database be encrypted by default, or is plaintext acceptable given that it contains only telemetry, not user content?
4. **Multi-node**: In a multi-node deployment (`01-system-architecture.md §multi-node`), do all nodes push to a single `nous-otlp` instance, or does each node run its own? If centralized, what port conflicts arise?
5. **Health endpoint**: The daemon API exposes `/status` but not `/health`. A `/health` endpoint returning `200 OK` / `503 Service Unavailable` based on DB connectivity would enable container health checks and systemd watchdog integration. Should it also report `nous-otlp` reachability?

---

## 3. Error Handling

### 3.1 Current State

`NousError` is a `thiserror`-derived enum in `crates/nous-shared/src/error.rs`. It covers the full failure surface of the system: SQLite errors, I/O, configuration, encryption, embedding pipeline failures, validation, not-found, and conflict. A `Result<T>` type alias (`pub type Result<T> = std::result::Result<T, NousError>`) propagates through every crate that imports `nous-shared`.

CLI entry points use `anyhow` for contextual error chains. The `NousError::exit_code()` method maps error variants to OS exit codes: 2 for validation/invalid-input, 3 for not-found, 4 for conflict, 1 for everything else.

### 3.2 Architecture

**`NousError` variant table:**

| Variant | Trigger | Exit code |
|---------|---------|-----------|
| `Sqlite(rusqlite::Error)` | Any SQLite operation failure | 1 |
| `Io(std::io::Error)` | File read/write, socket, process | 1 |
| `Config(String)` | Missing or malformed config file | 1 |
| `Encryption(String)` | SQLCipher key mismatch, re-key failure | 1 |
| `Internal(String)` | Invariant violation, unexpected state | 1 |
| `Embedding(String)` | ONNX inference failure, model load error | 1 |
| `Validation(String)` | Input fails schema constraints | 2 |
| `InvalidInput(String)` | Malformed argument, unsupported value | 2 |
| `NotFound(String)` | Resource absent in DB | 3 |
| `Conflict(String)` | Duplicate key, concurrent write collision | 4 |

**Error propagation flow:**

```
  nous-core handler (e.g. store_memory)
        │
        │  Result<T, NousError>  ← ? operator chains
        ▼
  WriteChannel / ReadPool
        │
        │  NousError::Sqlite(_) if rusqlite fails
        │  NousError::Embedding(_) if ort inference fails
        ▼
  crates/nous-cli/src/server.rs  (MCP tool handler)
        │
        │  On Err(e):
        │    → CallToolResult { is_error: true,
        │        content: [JSON string of e.to_string()] }
        ▼
  rmcp serializes → MCP client receives error content

  CLI path (clap handler in main.rs):
        │
        │  anyhow::Context wraps NousError
        ▼
  eprintln! + process::exit(e.exit_code())
```

**Graceful degradation rules:**

- **Embedding failure**: If ONNX inference fails when storing a memory, `Embedding(String)` is returned, but the calling code catches it and stores the row without a vector. The memory is text-searchable via FTS5 but not vector-searchable until re-embed runs.
- **Classification failure**: If the classifier cannot assign a category, the memory stores with `category_id = NULL`. `admin re-classify` can backfill later.
- **OTLP ingestion failure**: `nous-otlp` returns `HTTP 500` to the emitter but does not crash. The emitter retries at its own schedule; the main process is unaffected.

**Error serialization at the MCP boundary (detail):**

`handle_store` and other tool handlers in `crates/nous-cli/src/tools.rs` return `CallToolResult`. The pattern used throughout is:

```
match operation_result {
    Ok(payload) => CallToolResult {
        is_error: Some(false),
        content: vec![RawContent::Text { text: serde_json::to_string(&payload)? }],
    },
    Err(e) => CallToolResult {
        is_error: Some(true),
        content: vec![RawContent::Text { text: e.to_string() }],
    },
}
```

The `thiserror` `#[error("...")]` attribute defines the string form. Agents must check `is_error` before parsing `content[0]` as structured JSON, otherwise a not-found error message will fail JSON parsing and produce a misleading parse error.

### 3.3 Integration Points

- **MCP boundary** (`03-api-interfaces.md`): Every MCP tool handler wraps its result in `CallToolResult`. On success, `is_error = false` and content holds the JSON payload. On `Err(e)`, `is_error = true` and content holds `e.to_string()`. Agents read `is_error` to distinguish operational results from protocol errors.
- **Data layer** (`02-data-layer.md`): `WriteChannel` propagates `NousError` back to the sender through a `oneshot::Sender<Result<T>>`. Callers await the oneshot and surface the error to the MCP or CLI layer unchanged.
- **Rooms and tasks** (`04-features-p0-p1.md`): `NousError::NotFound` fires when a `room_id` or `task_id` FK constraint fails. The MCP tool returns this as `is_error: true` with the message `"not found: room {id}"`.
- **Scheduler** (`05-features-p2-p4.md`): If a scheduled action fails, the error is logged via `tracing::error!` and the schedule remains active. The next fire time proceeds regardless. Failed action results are not persisted (planned: write to a `schedule_errors` table).

### 3.4 Future Error Handling Work

**Retryable error classification:**

The current `NousError` enum has no `is_retryable()` predicate. Adding one would let the `WriteChannel` retry transient `Sqlite(SQLITE_BUSY)` errors without surfacing them to the caller:

| Variant | Retryable? | Rationale |
|---------|-----------|-----------|
| `Sqlite(SQLITE_BUSY)` | Yes, up to 3× | WAL reader-writer contention, resolves in milliseconds |
| `Embedding(_)` | Yes, once | ONNX model may fail on first load due to race on model cache |
| `NotFound(_)` | No | Retry cannot create the missing resource |
| `Conflict(_)` | No | Retry would hit the same unique constraint |
| `Validation(_)` | No | Input is invariantly bad |
| `Encryption(_)` | No | Key mismatch does not self-heal |

**`schedule_errors` table (planned):**

A `schedule_errors(id, schedule_id, fired_at, error_message, attempt)` table would persist each failed action execution. `nous schedule errors <id>` would expose this as a CLI command and MCP tool. Without this table, operators can only inspect failures in the systemd journal, which has no structured query interface.

### 3.5 Open Questions

1. **Retryable vs. terminal errors**: Should `NousError` carry a `is_retryable()` predicate? Embedding failures are retryable; conflict errors are not. Callers currently treat all errors as terminal.
2. **MCP error code taxonomy**: The MCP protocol supports structured error codes beyond `is_error`. Should Nous map `NousError` variants to MCP error codes for richer client-side handling?
3. **Scheduled action error persistence**: Failed scheduled actions currently log and continue. A `schedule_errors` table would enable `nous schedule errors <id>` introspection. Worth the schema cost?
4. **`anyhow` vs. `NousError` in `nous-otlp`**: `nous-otlp/src/main.rs` uses `anyhow::Result` throughout. Should it migrate to `NousError` for consistency, or is `anyhow` appropriate for a CLI-only binary?

---

## 4. Testing Strategy

### 4.1 Current State

The test suite runs with `cargo test --workspace` and covers four crates: `nous-shared`, `nous-core`, `nous-cli`, and `nous-otlp`. 31 test files exist across three test categories: unit tests (per-module, in `crates/*/tests/`), integration tests (full CLI flows in `crates/nous-cli/tests/`), and a standalone `tests/` workspace member with correlation and e2e tests.

All database tests create isolated SQLite instances via `tempdir`, so parallel test execution produces no file conflicts. The CI job runs `cargo test --workspace` as a single step without parallelism flags.

### 4.2 Test Architecture

**Test file inventory:**

| File | Crate | Category | What it verifies |
|------|-------|----------|-----------------|
| `nous-shared/tests/error_tests.rs` | nous-shared | Unit | `NousError` variants, exit codes |
| `nous-shared/tests/ids_tests.rs` | nous-shared | Unit | UUIDv7 generation, ID type round-trips |
| `nous-shared/tests/key_rotation_tests.rs` | nous-shared | Unit | SQLCipher key rotation |
| `nous-shared/tests/sqlite_tests.rs` | nous-shared | Unit | Migration runner, WAL mode flag |
| `nous-shared/tests/xdg_tests.rs` | nous-shared | Unit | XDG cache path resolution |
| `nous-core/tests/db_tests.rs` | nous-core | Unit | Schema migrations, CRUD on memories |
| `nous-core/tests/channel_tests.rs` | nous-core | Unit | `WriteChannel` bounded queue, backpressure |
| `nous-core/tests/chunk_tests.rs` | nous-core | Unit | Text chunking boundary conditions |
| `nous-core/tests/chunk_storage_tests.rs` | nous-core | Unit | Chunk read/write round-trips |
| `nous-core/tests/classify_tests.rs` | nous-core | Unit | Category assignment, null fallback |
| `nous-core/tests/embed_tests.rs` | nous-core | Unit | ONNX embedding pipeline |
| `nous-core/tests/embed_unit_tests.rs` | nous-core | Unit | Embedding math, cosine distance |
| `nous-core/tests/crud_tests.rs` | nous-core | Unit | Store/recall/forget lifecycle |
| `nous-core/tests/search_tests.rs` | nous-core | Unit | FTS5 query construction |
| `nous-core/tests/room_tests.rs` | nous-core | Unit | Room create/post/read |
| `nous-core/tests/scheduler_integration.rs` | nous-core | Integration | Scheduler fires at correct time |
| `nous-core/tests/pool_tests.rs` | nous-core | Unit | `ReadPool` concurrent access |
| `nous-core/tests/model_tests.rs` | nous-core | Unit | Serde round-trips for model types |
| `nous-core/tests/types_tests.rs` | nous-core | Unit | Type conversion, display impls |
| `nous-core/tests/db_vec0_tests.rs` | nous-core | Unit | `sqlite-vec` extension loading |
| `nous-core/tests/sqlite_vec.rs` | nous-core | Unit | Vector similarity query |
| `nous-core/tests/decoder_round_trip_tests.rs` | nous-core | Unit | Encode → decode → match |
| `nous-core/tests/encoder_round_trip_tests.rs` | nous-core | Unit | Encoder determinism |
| `nous-core/tests/access_tests.rs` | nous-core | Unit | Namespace isolation (cross-workspace reject) |
| `nous-cli/tests/integration.rs` | nous-cli | Integration | Full CLI flow: store → search → recall |
| `nous-cli/tests/concurrent.rs` | nous-cli | Concurrency | Parallel reads/writes, WAL mode verification |
| `nous-cli/tests/export_import.rs` | nous-cli | Integration | Export JSON → import → verify round-trip |
| `nous-cli/tests/room_e2e.rs` | nous-cli | E2E | Room create → post → read → subscribe |
| `nous-cli/tests/schedule_e2e.rs` | nous-cli | E2E | Schedule create → fire → verify |
| `nous-cli/tests/search_ranking.rs` | nous-cli | Integration | BM25 score order verification |
| `nous-otlp/tests/ingestion.rs` | nous-otlp | Integration | HTTP POST → decode → store → query |

**Isolation pattern:**

Every test that touches SQLite calls:
```rust
let dir = tempfile::tempdir().unwrap();
let db_path = dir.path().join("test.db");
```
The `tempdir` drop guard deletes the directory when the test exits. No global database state leaks between tests.

**Concurrency test approach (`concurrent.rs`):**

The test `ten_concurrent_writes_no_data_loss` uses `tokio::task::JoinSet` to spawn 10 concurrent async tasks against a shared `Arc<NousServer>`. Each task calls `handle_store` with a unique `title` and `content`. After all tasks complete, the test calls `MemorySqlParams` to count rows and asserts all 10 writes landed. The DB path is unique per test process using a `(pid, timestamp_nanos, atomic_counter)` tuple, preventing cross-test file conflicts when the suite runs in parallel threads.

**Test fixture pattern:**

Unit tests in `nous-core` use `NamedTempFile` from the `tempfile` crate:

```rust
fn temp_db() -> (MemoryDb, NamedTempFile) {
    let file = NamedTempFile::new().expect("failed to create temp file");
    let db = MemoryDb::open(file.path().to_str().unwrap(), None, 384)
        .expect("failed to open db");
    (db, file)
}
```

The `NamedTempFile` drop guard deletes the file when the tuple goes out of scope. The embedding dimension argument (`384`) matches the MiniLM-L6-v2 model used in production — tests exercise the same schema without requiring the ONNX model binary.

**Workspace-level test crates:**

The workspace includes two additional test crates not under `crates/`:

| Crate | Path | Purpose |
|-------|------|---------|
| `correlation` | `tests/correlation/` | Cross-crate correlation tests linking `trace_id` from memories to OTLP spans |
| `e2e` | `tests/e2e/` | End-to-end tests that spawn `nous serve` as a subprocess and drive it via MCP tool calls |

Both are workspace members (`[workspace].members` in root `Cargo.toml`). `cargo test --workspace` includes them automatically.

### 4.3 Integration Points

- **Data layer** (`02-data-layer.md`): `channel_tests.rs` and `pool_tests.rs` directly exercise the `WriteChannel` and `ReadPool` structs. Any change to channel capacity or pool size must not break these tests.
- **MCP API** (`03-api-interfaces.md`): `integration.rs` calls the CLI binary as a subprocess, so it exercises the full MCP tool dispatch path including rmcp serialization. This makes it the highest-confidence test for MCP correctness.
- **Search ranking** (`03-api-interfaces.md §BM25`): `search_ranking.rs` constructs known memory corpora, runs FTS5 queries, and asserts that BM25 score ordering matches expectations. Any change to the FTS5 tokenizer or ranking expression must re-validate these tests.
- **OTLP** (`crates/nous-otlp/tests/ingestion.rs`): Sends raw protobuf to the Axum server in-process and queries the resulting `otlp.db` rows. Tests both protobuf and JSON encoding paths.
- **Schedule e2e** (`05-features-p2-p4.md §5`): `schedule_e2e.rs` creates a schedule with a 1-second interval and polls for the action to fire. This test is time-sensitive; it may flake under high CI load.

### 4.4 Future Test Additions

| Test | Mechanism | Benefit |
|------|-----------|---------|
| Property-based cron parser tests | `proptest` — generate arbitrary cron strings | Catches edge cases in the 5-field parser (day-of-week, month abbreviations, L/W/# syntax) |
| Fuzz target for MCP input dispatch | `cargo-fuzz` on `handle_store` / `handle_recall` entry points | Finds panics on malformed JSON before attackers do |
| Mock clock for schedule_e2e | Inject `Arc<dyn Clock>` trait, advance by tick in tests | Removes wall-clock dependency, eliminates CI flakiness |
| Embedding pipeline snapshot tests | Store expected 384-dim vectors for fixed inputs; assert cosine similarity > 0.99 | Detects model version drift |
| SQLCipher re-key round-trip tests | Open DB, rotate key, re-open with new key, verify all rows readable | Already tested in `key_rotation_tests.rs`; worth extending to encrypted + vector extension |

### 4.5 Open Questions

1. **Property-based tests for cron parser**: The cron expression parser (`crates/nous-core/src/cron_parser.rs`) has no property-based coverage. `proptest` or `quickcheck` could generate arbitrary cron strings and verify the parser never panics. Is this worth the dependency?
2. **Fuzz testing for MCP input**: MCP tool inputs arrive as unvalidated JSON. A `cargo-fuzz` target on the tool dispatch layer would surface panics on malformed input. Blocked on: fuzz infrastructure in CI.
3. **Time-sensitive schedule tests**: `schedule_e2e.rs` uses real wall-clock time. Should it mock the clock or use a deterministic tick injector to remove CI flakiness?
4. **Test coverage reporting**: No coverage gate exists in CI. Should a minimum line coverage threshold (e.g., 70%) gate merges to `main`?

---

## 5. CI/CD

### 5.1 Current State

A single GitHub Actions workflow (`.github/workflows/ci.yml`) runs on every push to `main` and on every pull request. It runs on `ubuntu-latest` with a single job: format check → lint → test → release build. The job sets `CARGO_BUILD_JOBS: 1` to avoid OOM kills on the hosted runner, at the cost of slower parallel compilation.

The workspace uses Rust toolchain `1.88` (pinned via `dtolnay/rust-toolchain`) with `rustfmt` and `clippy` components. Cargo cache is keyed on `Cargo.lock` hash to get fast incremental builds on cache hit.

Deployment artifacts — Homebrew formula, Docker image, systemd service — are not yet automated in CI. The formula and service file exist in the repo but require manual publishing steps.

### 5.2 Architecture

**CI pipeline (current):**

```
  git push / pull_request event
        │
        ▼
  GitHub Actions: CI job (ubuntu-latest)
  ┌─────────────────────────────────────────────────────┐
  │                                                     │
  │  1. checkout@v4                                     │
  │  2. free-disk-space (reclaim ~20GB for build cache) │
  │  3. rust-toolchain@1.88 (rustfmt + clippy)          │
  │  4. cache@v4 (~/.cargo/registry + target/)          │
  │     key: $OS-cargo-${{ hashFiles('Cargo.lock') }}   │
  │                                                     │
  │  5. cargo fmt --check           (fails on diff)     │
  │  6. cargo clippy --workspace -- -D warnings         │
  │  7. cargo test --workspace                          │
  │  8. cargo clean --profile dev   (reclaim disk)      │
  │  9. cargo build --release                           │
  │                                                     │
  └─────────────────────────────────────────────────────┘
```

**Release distribution (planned):**

| Artifact | Target | Tool | Status |
|----------|--------|------|--------|
| Binary — macOS arm64 | `aarch64-apple-darwin` | `cross` | Planned |
| Binary — macOS x86_64 | `x86_64-apple-darwin` | `cross` | Planned |
| Binary — Linux x86_64 | `x86_64-unknown-linux-gnu` | `cross` | Planned |
| Homebrew formula | `skevetter/homebrew-tap` | `cargo-release` | Planned |
| Docker image | multi-stage, Debian slim | `docker buildx` | Planned |
| systemd service | `systemd/nous-cli.service` | install script | Manual |

**systemd service configuration:**

`systemd/nous-cli.service` installs to `~/.config/systemd/user/nous-cli.service`. Key settings:

```ini
ExecStart=%h/.cargo/bin/nous serve --transport http
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info
StandardOutput=journal
StandardError=journal
```

`%h` expands to the installing user's home directory. `Restart=on-failure` ensures the daemon recovers from crashes. `RUST_LOG=info` controls `tracing` subscriber verbosity.

### 5.3 Integration Points

- **System architecture** (`01-system-architecture.md`): The daemon mode (`nous serve --transport http`) is what the systemd service runs. CI must build and test the daemon binary; `cargo build --release` in step 9 produces it.
- **Data layer** (`02-data-layer.md`): The release build includes bundled SQLCipher (`rusqlite = { features = ["bundled-sqlcipher"] }`). Cross-compilation for macOS requires the host to provide an OpenSSL-compatible build environment or use `cross` with a Docker image that includes it.
- **Testing** (§4 above): `cargo test --workspace` in CI step 7 runs all 31 test files. The `schedule_e2e.rs` time-sensitive tests run here; wall-clock variance under GitHub Actions load is a known flakiness source.
- **nous-otlp binary**: The workspace includes `crates/nous-otlp` as a member. `cargo build --release` produces both `nous-cli` and `nous-otlp` binaries. The release workflow must distribute both.

**Docker multi-stage build design (planned):**

```dockerfile
# Stage 1: Builder
FROM rust:1.88-slim AS builder
WORKDIR /src
COPY . .
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/nous-cli /usr/local/bin/nous
COPY --from=builder /src/target/release/nous-otlp /usr/local/bin/nous-otlp
EXPOSE 3000 4318
CMD ["nous", "serve", "--transport", "http"]
```

Port 3000 is the HTTP daemon API port. Port 4318 is the OTLP ingest port from `nous-otlp`. Both processes can be managed via a process supervisor (s6, supervisord) inside the container, or split into separate containers behind a shared volume for `memory.db` and `otlp.db`.

**Version bump and release flow (planned, `cargo-release`):**

```
  1. cargo release [major|minor|patch]
       → bumps Cargo.toml versions across workspace
       → creates git tag vX.Y.Z
       → pushes tag to origin

  2. GitHub Actions release workflow (triggered by tag push)
       → builds release binaries for all targets via cross
       → creates GitHub Release with attached binaries
       → updates Homebrew formula in skevetter/homebrew-tap
         (SHA256 hashes + new download URL)
```

The `tests/correlation` and `tests/e2e` workspace members participate in the version bump because they are workspace members, even though they are test-only crates. They must be excluded from the Homebrew formula and GitHub Release artifacts.

**Disk space management in CI:**

The `jlumbroso/free-disk-space@v1.3.1` step reclaims ~20GB by removing Android SDK, .NET SDK, Haskell GHC, large pre-installed packages, Docker images, and swap storage. This is necessary because bundled SQLCipher (`rusqlite = { features = ["bundled-sqlcipher"] }`) and ONNX Runtime (`ort = "2.0.0-rc.12"`) produce large build artifacts that can exhaust the default 14GB free space on the GitHub-hosted `ubuntu-latest` runner.

### 5.4 Open Questions

1. **`CARGO_BUILD_JOBS: 1` tax**: Single-job compilation avoids OOM on the free GitHub runner (7GB RAM) but significantly slows builds. Would a larger runner (16GB) justify the cost to restore parallel compilation?
2. **Cross-compilation strategy**: `cross` uses Docker-in-Docker for macOS targets on Linux hosts, which may not work on GitHub Actions. `cargo-zigbuild` with Zig as the cross-linker is an alternative. Which is more maintainable?
3. **Homebrew formula automation**: `cargo-release` can tag versions and push to `skevetter/homebrew-tap` in one command. Should the release workflow trigger on git tags (`v*`) or on a manual dispatch?
4. **Docker base image**: The systemd integration requires a base image that ships systemd. Should the Docker image target a systemd-enabled distribution (Debian with systemd), or should it run `nous serve` directly without systemd?
5. **`cargo clean` between steps**: Step 8 (`cargo clean --profile dev`) removes debug artifacts to free disk before the release build. This means a CI failure in the release step cannot reuse debug build artifacts for diagnosis. Is this trade-off acceptable?

---

## 6. Decisions Log

Key decisions made during the design of these cross-cutting concerns, recorded so future contributors understand the rationale rather than re-litigating them.

| Decision | Chosen approach | Rejected alternative | Reason |
|----------|----------------|---------------------|--------|
| Error library | `thiserror` for library crates, `anyhow` for binaries | `anyhow` everywhere | Library callers need to pattern-match on error variants; anyhow erases the type |
| Namespace isolation mechanism | Integer `workspace_id` FK with SQL WHERE filtering | Separate SQLite databases per namespace | Separate databases require connection pool duplication and complicate FTS5 cross-memory search |
| OTLP as a separate binary | `nous-otlp` is its own process and crate | Embed OTLP ingest in `nous-cli` | Keeps the main process dependency surface small; operators who don't need observability don't run it |
| MCP transport default | stdio (implicit trust) | HTTP with auth tokens | Stdio matches the usage pattern of LLM agent frameworks; HTTP adds deployment complexity for no benefit in the single-user case |
| Test DB isolation | `NamedTempFile` / unique path per test | Shared in-memory SQLite | In-memory mode doesn't load the `sqlite-vec` and `fts5` extensions; file-backed mode exercises the real schema |
| CI single-job build | `CARGO_BUILD_JOBS: 1` | Full parallel compilation | GitHub-hosted `ubuntu-latest` runner has 7GB RAM; bundled SQLCipher + ONNX Runtime exhaust it under parallel compilation |

## 7. Cross-Reference Index

| Topic | This document | Referenced doc | Section |
|-------|---------------|----------------|---------|
| Daemon model, single/multi-node | §1 Auth, §5 CI/CD | `01-system-architecture.md` | Deployment modes |
| `WriteChannel`, `ReadPool`, `workspace_id` FK | §1.3, §3.3, §5.3 | `02-data-layer.md` | Backend trait |
| MCP tool dispatch, rmcp, `CallToolResult` | §3.2, §3.3, §4.3 | `03-api-interfaces.md` | MCP server |
| BM25 ranking, FTS5 virtual table | §1.2 (namespace scoping), §4.3 | `03-api-interfaces.md` | Search ranking |
| `agent_id` in rooms/tasks, `room_participants` | §1.3, §3.3 | `04-features-p0-p1.md` | §3.1, §3.2 |
| Room notification subscriber model | §3.3 | `04-features-p0-p1.md` | §3.1 Notification flow |
| Task `assignee_id`, schedule action dispatch | §1.3, §3.3 | `04-features-p0-p1.md` | §3.2 |
| Scheduler namespace, schedule_e2e flakiness | §1.3, §4.3, §4.5 | `05-features-p2-p4.md` | §5 Schedule Engine |
| Org hierarchy, namespace per org unit | §1.2 (future RBAC) | `05-features-p2-p4.md` | §4 Org Hierarchy |
| `NousError` source | §3 all | `crates/nous-shared/src/error.rs` | — |
| OTLP DB schema | §2 all | `crates/nous-otlp/src/db.rs` | `OTLP_MIGRATIONS` |
| OTLP HTTP server routing | §2.2 | `crates/nous-otlp/src/server.rs` | `router()` |
| CI workflow | §5 all | `.github/workflows/ci.yml` | — |
| systemd service | §5.2 | `systemd/nous-cli.service` | — |
| Workspace member list | §4.2, §5.3 | `Cargo.toml` (root) | `[workspace].members` |
