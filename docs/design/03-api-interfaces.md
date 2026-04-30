# API Interfaces

## Table of Contents

1. [Goals](#1-goals)
2. [Non-Goals](#2-non-goals)
3. [Architecture Overview](#3-architecture-overview)
4. [MCP Protocol Design](#4-mcp-protocol-design)
5. [CLI Interface Design](#5-cli-interface-design)
6. [Internal Service APIs](#6-internal-service-apis)
7. [Event Bus](#7-event-bus)
8. [Error Handling Strategy](#8-error-handling-strategy)
9. [ID Generation](#9-id-generation)
10. [Dependencies Between Documents](#10-dependencies-between-documents)
11. [Open Questions](#11-open-questions)
12. [Cross-Reference Index](#12-cross-reference-index)

---

## 1. Goals

Three interface layers expose nous to different consumers:

| Layer | Consumer | Transport |
|-------|----------|-----------|
| MCP server | AI agents (Claude Code, LLM toolchains) | stdio (in-process) or streamable HTTP on port 8377 |
| CLI (`nous`) | Human operators, shell scripts, CI pipelines | subprocess |
| Daemon API | Internal daemon ↔ CLI coordination | Unix socket at `~/.cache/nous/daemon.sock` |

**MCP** is the primary runtime surface. Every AI agent interaction — storing memories, searching, managing rooms and schedules — flows through MCP tools. The protocol must be stable, well-typed (JSON Schema per tool), and discoverable by any MCP-compatible client.

**CLI** exposes the same capabilities to humans and scripts. Its tiered namespace (`nous memory store`, `nous room create`) provides discoverability without sacrificing the flat-command muscle memory of existing users via backward-compatible aliases.

**Internal APIs** let the CLI control a running daemon (check status, trigger shutdown) without re-implementing MCP logic. The daemon's Unix socket is local-only, keeping the IPC surface minimal and avoiding network auth complexity.

## 2. Non-Goals

- **REST API for external consumers.** No public HTTP API is planned. The daemon's HTTP routes (`daemon_api.rs`) are Unix-socket-only and serve internal coordination, not external clients.
- **GraphQL.** The query surface is well-defined and tool-centric; a graph query language adds schema complexity without a clear benefit for the current use cases.
- **WebSocket streaming.** Streamable HTTP (MCP transport) handles streaming for the remote-access case. A separate WebSocket protocol is out of scope.
- **Multi-tenant auth.** The daemon API assumes a single local user. Token-based authentication or per-tenant isolation is not in scope for this document.
- **Plugin system for third-party MCP tools.** Tool registration is compile-time via `#[tool_router]`. A runtime plugin loader is not planned.

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     External Clients                        │
│                                                             │
│   Claude Code / LLM agent        Human / shell script      │
│         (MCP client)                  (nous CLI)            │
└──────────┬──────────────────────────────┬───────────────────┘
           │ stdio or HTTP :8377           │ subprocess
           ▼                              ▼
┌──────────────────────┐    ┌─────────────────────────────┐
│    MCP Server        │    │          CLI Layer           │
│  (NousServer /       │    │  clap Command enum           │
│   rmcp #[tool_router])    │  nous memory / room /        │
│                      │    │  schedule / admin / query    │
└──────────┬───────────┘    └──────────────┬──────────────┘
           │                               │ Unix socket
           │         ┌─────────────────────┤
           │         ▼                     ▼
           │  ┌─────────────────────────────────────┐
           │  │        Daemon API (axum Router)      │
           │  │  GET  /status                        │
           │  │  POST /shutdown                      │
           │  │  POST /rooms   GET /rooms/{id}       │
           │  │  POST /memories/store  /search       │
           │  └──────────────────────────────────────┘
           │                    │
           └────────────────────┘
                       │
                       ▼
       ┌───────────────────────────────────┐
       │         Internal Service Layer    │
       │                                   │
       │  WriteChannel  ──►  MemoryDb      │
       │  ReadPool      ──►  MemoryDb      │
       │  EmbeddingBackend (ONNX)          │
       │  CategoryClassifier               │
       │  Scheduler                        │
       └───────────────────────────────────┘
                       │
                       ▼
       ┌───────────────────────────────────┐
       │           Data Layer              │
       │  SQLite + sqlite-vec + FTS5       │
       │  ~/.cache/nous/memory.db          │
       │  ~/.cache/nous/memory-fts.db      │
       │  ~/.cache/nous/memory-vec.db      │
       │  ~/.cache/nous/otlp.db            │
       └───────────────────────────────────┘
```

All three entry points — MCP, CLI, and daemon API — funnel writes through `WriteChannel` and reads through `ReadPool`. Neither the MCP server nor the daemon API bypass this abstraction. See [02-data-layer.md](02-data-layer.md) for the channel contract.

## 4. MCP Protocol Design

### Tool Registration

`NousServer` (`crates/nous-cli/src/server.rs`) is the MCP server struct. It holds `WriteChannel`, `ReadPool`, `EmbeddingBackend`, `CategoryClassifier`, `Chunker`, and `Scheduler` as fields. Tool handlers are registered with two rmcp macros:

- `#[tool_router]` on the `impl NousServer` block generates the compile-time tool dispatch table (`ToolRouter<NousServer>`).
- `#[tool(name = "...", description = "...")]` on each async method registers one tool.
- `#[tool_handler(name = "nous-cli", version = "0.1.0")]` on `impl ServerHandler for NousServer {}` wires the struct into the MCP protocol handshake.

```rust
// crates/nous-cli/src/server.rs

use rmcp::{ServerHandler, tool, tool_handler, tool_router};

#[tool_router]
impl NousServer {
    #[tool(name = "memory_store", description = "Store a new memory")]
    async fn memory_store(&self, params: Parameters<MemoryStoreParams>) -> CallToolResult {
        handle_store(params.0, &self.write_channel, &self.embedding,
                     &self.classifier, &self.chunker).await
    }

    #[tool(name = "memory_search",
           description = "Search memories using FTS, semantic, or hybrid search")]
    async fn memory_search(&self, params: Parameters<MemorySearchParams>) -> CallToolResult {
        handle_search(params.0, &self.db_path, self.config.embedding.dimensions,
                      &self.embedding).await
    }
    // ... 30+ further tools
}

#[tool_handler(name = "nous-cli", version = "0.1.0")]
impl ServerHandler for NousServer {}
```

Tool parameter structs in `crates/nous-cli/src/tools.rs` derive both `serde::Deserialize` and `schemars::JsonSchema`. The `JsonSchema` derive causes rmcp to emit a JSON Schema description for each parameter in the MCP tool manifest, allowing Claude Code and other clients to validate inputs before sending them.

```rust
// crates/nous-cli/src/tools.rs

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySearchParams {
    pub query: String,
    pub mode: Option<String>,          // "fts" | "semantic" | "hybrid"
    pub memory_type: Option<String>,
    pub workspace_path: Option<String>,
    pub tags: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub since: Option<String>,         // ISO 8601
    pub limit: Option<usize>,          // default: 20
    // ... additional filters
}
```

### Transport Modes

| Mode | When | Config key | Default |
|------|------|-----------|---------|
| `stdio` | Claude Code integration, daemon-managed server | `daemon.mcp_transport = "stdio"` | Yes |
| `http` | Remote access, multi-client scenarios | `daemon.mcp_transport = "http"` | No |

`stdio` transport: `nous serve --transport stdio` reads JSON-RPC from stdin and writes to stdout. The daemon spawns this process and manages its lifecycle.

`http` transport: `nous serve --transport http --port 8377` starts an axum-backed streamable HTTP server (rmcp `transport-streamable-http-server` feature) on `0.0.0.0:8377`. Each request establishes a session managed by `LocalSessionManager`.

```rust
// crates/nous-cli/src/main.rs (serve command)
match transport {
    Transport::Stdio => {
        let service = server.serve(rmcp::transport::stdio());
        service.waiting().await?;
    }
    Transport::Http => {
        let config = StreamableHttpServerConfig {
            bind: format!("0.0.0.0:{port}").parse()?,
            ..Default::default()
        };
        let service = StreamableHttpService::new(
            move || Ok(NousServer::new(...)),
            LocalSessionManager::default(),
            config,
        );
        axum::serve(listener, service).await?;
    }
}
```

### Tool Catalog

**Memory CRUD** — core operations on the `memories` table:

| Tool | Description |
|------|-------------|
| `memory_store` | Create a memory with title, content, type, tags, workspace, and embedding |
| `memory_recall` | Fetch a memory by UUIDv7 ID with all fields and relationships |
| `memory_update` | Patch title, content, tags, importance, confidence, or `valid_until` |
| `memory_forget` | Archive (soft) or hard-delete a memory |
| `memory_unarchive` | Restore an archived memory and re-embed it |
| `memory_relate` | Create a typed edge (`related`, `supersedes`, `contradicts`, `depends_on`) |
| `memory_unrelate` | Remove a typed edge between two memories |
| `memory_search` | FTS, semantic, or hybrid search with tag/workspace/date filters |
| `memory_context` | Return workspace-scoped memories paginated for context injection |

**Memory introspection:**

| Tool | Description |
|------|-------------|
| `memory_workspaces` | List all workspaces with memory counts |
| `memory_tags` | List all tags ordered by frequency, with optional prefix filter |
| `memory_stats` | Database-level counts by type, category, importance, and workspace |
| `memory_schema` | Return the SQLite DDL schema text |
| `memory_sql` | Execute a read-only SQL query (SELECT, EXPLAIN, read-only PRAGMA) |

**Category management:**

| Tool | Description |
|------|-------------|
| `memory_category_list` | List categories as a tree, filtered by source (`system`, `user`, `agent`) |
| `memory_category_add` | Create a user category with optional parent and classification threshold |
| `memory_category_delete` | Delete a category; refuses if it has children |
| `memory_category_update` | Rename a category or change its threshold |
| `memory_category_suggest` | Compute an embedding for a proposed category and assign it to a memory |

**Room operations:**

| Tool | Description |
|------|-------------|
| `room_create` | Create a conversation room with optional purpose metadata |
| `room_list` | List active or archived rooms |
| `room_get` | Fetch a room by UUIDv7 ID or name |
| `room_delete` | Archive or hard-delete a room |
| `room_post_message` | Append a message to a room |
| `room_read_messages` | Read messages with optional pagination and time filters |
| `room_search` | Full-text search over messages within a room |
| `room_info` | Room metadata including participant list and message count |
| `room_join` | Add a participant with a role (`owner`, `member`, `observer`) |

**Schedule operations:**

| Tool | Description |
|------|-------------|
| `schedule_create` | Register a cron schedule with an action type and payload |
| `schedule_list` | List schedules ordered by next fire time |
| `schedule_get` | Fetch full schedule detail including last 10 run records |
| `schedule_update` | Modify a schedule; recomputes `next_run_at` if the cron expression changes |
| `schedule_delete` | Remove a schedule |
| `schedule_pause` | Suspend a schedule without deleting it |
| `schedule_resume` | Re-activate a paused schedule |

**OTLP correlation:**

| Tool | Description |
|------|-------------|
| `otlp_trace_context` | Fetch memories, spans, and logs correlated to a trace ID |
| `otlp_memory_context` | Fetch spans and logs correlated to a memory's `trace_id` and `session_id` |

### Naming Convention

Tools follow the pattern `<resource>_<verb>` where resource is the primary entity (`memory`, `room`, `schedule`) and verb describes the operation (`store`, `recall`, `update`, `forget`, `search`, `context`). Category tools are nested: `memory_category_<verb>`. OTLP tools use `otlp_<noun>_context`. No tool name is a bare verb — every name is self-contained and unambiguous in a flat tool list.

### Error Response Format

All tool handlers return `CallToolResult`. On error, the result sets `is_error = Some(true)` and `content[0]` holds a JSON string with an `"error"` key:

```json
{
  "isError": true,
  "content": [{ "type": "text", "text": "{\"error\": \"not found: memory 019123...\"}" }]
}
```

On success, `is_error` is absent or `false` and `content[0]` holds the serialized result as a JSON string. The daemon API's `call_tool_result_to_response` helper (`daemon_api.rs:458`) translates this convention to HTTP status codes: error → `400 Bad Request`, success → `200 OK`.

## 5. CLI Interface Design

### Binary and Namespace Structure

The `nous` binary (`crates/nous-cli/src/main.rs`) uses clap 4 with the `derive` feature. Commands organize into a tiered namespace per [docs/cli-restructuring.md](../cli-restructuring.md):

```
nous
├── memory        store | recall | update | forget | unarchive
│                 search | context | relate | unrelate
├── model         list | info | register | activate | deactivate | switch | setup
├── embedding     inspect | reset
├── category      list | add | delete | rename | update | suggest
├── room          create | list | get | post | read | search | delete
├── schedule      list | get | create | delete | pause | resume
├── daemon        start | stop | restart | status
├── doctor        (top-level — system health check)
├── admin         status | re-embed | re-classify | import | export
├── query         sql | schema | workspaces | tags | trace
└── serve         (top-level — starts the MCP server)
```

`serve` stays at the top level because it is referenced by name in systemd units and Docker `CMD` directives (`nous serve --transport stdio`). Moving it into a namespace would silently break all deployment configurations.

### Global Flags

Inherited by every subcommand:

| Flag | Type | Default | Purpose |
|------|------|---------|---------|
| `--config <PATH>` | PathBuf | `~/.config/nous/config.toml` | Config file path |
| `--db <PATH>` | PathBuf | from config | Database path override |
| `-v / --verbose` | bool | false | Verbose output |
| `-q / --quiet` | bool | false | Errors only |
| `--format <FMT>` | enum | `human` | Output format: `human \| json \| csv` |

### Output Formats

`--format human` (default) renders tabular output with aligned columns and colored headers for terminal use. `--format json` emits newline-delimited JSON, one object per result row, suitable for `jq` pipelines. `--format csv` emits RFC 4180 CSV with a header row. All three formats receive the same underlying data; only rendering differs. Deprecation warnings always go to stderr and never contaminate `--format json` output.

### Backward Compatibility

The restructuring introduces 20 previously flat commands (those now under `memory`, `admin`, and `query`) as new namespaced paths while preserving the old flat paths as hidden clap aliases:

```rust
// Phase 1: alias with no warning (v0.2.0)
#[derive(Debug, Subcommand)]
enum Command {
    Memory(MemoryCmd),

    #[command(hide = true)]   // hidden from --help
    Store { /* same fields as MemorySubcommand::Store */ },
}
// The Store match arm forwards to the same handler as Memory { Store { .. } }

// Phase 2: deprecation warning on stderr (v0.3.0)
fn deprecated_alias(old: &str, new: &str) {
    eprintln!("⚠ `{old}` is deprecated. Use `{new}` instead.");
    eprintln!("  This alias will be removed in v0.5.0.");
}
```

| Version | Behavior |
|---------|----------|
| v0.2.0 | Old and new paths both work; no output change |
| v0.3.0 | Old paths emit a deprecation warning to stderr |
| v0.5.0 | Old paths removed; clap emits "unknown subcommand: store — did you mean `memory store`?" |

### serve Command Flags

```
nous serve
  --transport <stdio|http>   Transport mode [default: stdio]
  --port <PORT>              HTTP listen port [default: 8377]
  --model <NAME>             Override embedding model name
  --variant <VARIANT>        Override embedding model file variant
  --allow-shell-schedules    Enable shell-based schedule actions
  --no-scheduler             Disable the scheduler loop
```

### Key Namespace Details

**`nous memory search`** accepts `--mode fts|semantic|hybrid` (default: `hybrid`), `--limit` (default: 20), and a free-text `<QUERY>` positional argument. The same parameters map 1:1 to `MemorySearchParams` in the MCP `memory_search` tool.

**`nous admin re-embed`** accepts `--model <NAME>` as optional (not required). When omitted, the command resolves the active model from the database — the user no longer needs to run `nous model list` first to find the model name (see `docs/cli-restructuring.md §6.2`).

**`nous model list`** hides models whose names contain `placeholder`, `mock`, or `test` by default. `--all` shows all registered models.

### `nous doctor` — System Health Check

`nous doctor` validates the local installation and reports issues. It runs without a daemon connection and is suitable for debugging setup problems.

**Checks performed:**

| Check | Pass condition | Fail output |
|-------|---------------|-------------|
| DB connectivity | Can open all 3 SQLite files (memory.db, memory-fts.db, memory-vec.db) | `FAIL: cannot open <path>: <error>` |
| Config validity | TOML parses, required fields present | `FAIL: config parse error: <detail>` |
| Port availability | MCP port 8377 not in use | `WARN: port 8377 in use by PID <pid>` |
| Storage permissions | Read/write access to data directories | `FAIL: no write access to <path>` |
| Version compatibility | Binary version matches schema version | `WARN: schema version mismatch (binary: X, db: Y)` |

**Example output:**

```
$ nous doctor
✓ Config valid                 ~/.config/nous/config.toml
✓ memory.db                    ~/.cache/nous/memory.db (WAL mode, 2.4 MB)
✓ memory-fts.db                ~/.cache/nous/memory-fts.db (WAL mode, 512 KB)
✓ memory-vec.db                ~/.cache/nous/memory-vec.db (WAL mode, 1.8 MB)
✓ otlp.db                     ~/.cache/nous/otlp.db (WAL mode, 128 KB)
✓ Port 8377                    available
✓ Storage permissions          read/write OK
✓ Schema version               v34 (matches binary)

All checks passed.
```

On failure, exit code is 1 and failing checks are listed first.

## 6. Internal Service APIs

### Daemon Architecture

`Daemon` (`crates/nous-cli/src/daemon.rs`) owns the Unix socket lifecycle:

1. On `Daemon::new(config)`, it writes the current PID to `~/.cache/nous/daemon.pid`, cleans up any stale socket file, and binds a `tokio::net::UnixListener` to `~/.cache/nous/daemon.sock`.
2. `Daemon::run(router)` passes the listener to `axum::serve`, which handles HTTP/1.1 over Unix domain sockets. A `tokio::sync::watch` channel carries the shutdown signal.
3. `Drop` removes both the PID file and socket file, so abrupt termination leaves no stale files.

```rust
// crates/nous-cli/src/daemon.rs

pub struct Daemon {
    pid_file: PathBuf,            // ~/.cache/nous/daemon.pid
    socket_path: PathBuf,         // ~/.cache/nous/daemon.sock
    listener: Option<UnixListener>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    shutdown_timeout_secs: u64,   // default: 30
}
```

A second daemon start is rejected immediately: `Daemon::new` checks whether the PID in the existing PID file corresponds to a live process (`/proc/{pid}` exists) and returns `DaemonError::AlreadyRunning(pid)` if so. Stale PID files (from crashed processes) are cleaned up automatically.

### Daemon HTTP Routes

`daemon_router` (`crates/nous-cli/src/daemon_api.rs`) mounts an axum `Router` with these routes:

| Method | Path | Purpose | Response codes |
|--------|------|---------|----------------|
| `GET` | `/health` | DB connectivity check via `SELECT 1`. Returns `{status: "healthy", db_ok: bool, uptime_secs: u64}` | 200 OK, 503 Service Unavailable |
| `GET` | `/status` | Returns `{ pid, uptime_secs, version }` | 200 OK |
| `POST` | `/shutdown` | Sends `true` on the shutdown watch channel; returns `{ ok: true }` | 200 OK |
| `POST` | `/rooms` | Create a room by name | 201 Created, 409 Conflict |
| `GET` | `/rooms` | List rooms (`?archived=true&limit=N`) | 200 OK |
| `GET` | `/rooms/{id}` | Get a room by UUID or name | 200 OK, 404 Not Found |
| `POST` | `/rooms/{id}/messages` | Post a message to a room | 201 Created, 404 Not Found |
| `GET` | `/rooms/{id}/messages` | Read messages (`?limit=N&since=<ts>&before=<ts>`) | 200 OK, 404 Not Found |
| `POST` | `/memories/search` | Search memories (same params as MCP `memory_search`) | 200 OK, 400 Bad Request |
| `POST` | `/memories/store` | Store a memory (same params as MCP `memory_store`) | 201 Created, 400 Bad Request |
| `GET` | `/categories` | List all categories | 200 OK |
| `POST` | `/export` | Export all memories as JSON | 200 OK |
| `POST` | `/import` | Import memories from JSON body | 200 OK, 400 Bad Request |

These routes exist to let the CLI manage data in a running daemon without spawning a separate MCP session. They are not intended as a general-purpose API.

**Schedule errors table:** Deferred. Schedule execution errors are logged in `schedule_runs.error` column. A separate `schedule_errors` table for detailed error history will be added when the scheduling system matures.

### DaemonClient

`DaemonClient` (`crates/nous-cli/src/daemon_client.rs`) connects to the Unix socket using `hyper` over `tokio::net::UnixStream`. Each call opens a fresh connection — no persistent connection pool — which keeps the implementation simple for the low-frequency CLI use case.

```rust
pub struct DaemonClient {
    socket_path: PathBuf,   // ~/.cache/nous/daemon.sock
}

impl DaemonClient {
    pub async fn status(&self) -> Result<StatusResponse, ClientError>;
    pub async fn shutdown(&self) -> Result<ShutdownResponse, ClientError>;
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError>;
    pub async fn post_json<T, B: Serialize>(&self, path: &str, body: &B) -> Result<T, ClientError>;
}
```

`ClientError::ConnectionRefused` is returned when the socket file does not exist or connection is refused, giving the CLI a clear diagnostic: "is the daemon running?"

### Future: Worker Coordination

The current single-node model has no worker registry. Future multi-node deployments will need:

1. **Worker registration** — workers announce their capabilities (tool subset, resource limits) to a coordinator on startup.
2. **Heartbeat** — workers ping the coordinator every N seconds; the coordinator marks them unhealthy after 3 missed heartbeats.
3. **Task assignment** — the coordinator assigns incoming tool calls to available workers based on capability and load.

Workers will forward write operations as JSON POST requests to the coordinator's existing Axum HTTP/JSON daemon API (see §11 Q1 — resolved). The current daemon API structure — axum `Router` over `UnixListener` — is extensible: adding coordinator routes requires only new `route()` registrations in `daemon_router`.

## 7. Event Bus

### Current State

Nous has no explicit event bus today. Cross-feature notifications happen through direct calls: the scheduler fires `Scheduler::spawn_with_otlp`, `WriteChannel` persists writes synchronously, and the MCP server returns results without broadcasting to other features.

### Proposed In-Process Design

For single-node deployments, a `tokio::sync::broadcast` channel delivers events to multiple subscribers with zero external dependencies. The channel sits inside `NousServer` or a shared `Arc<EventBus>` accessible to all features.

```rust
// Proposed: crates/nous-core/src/events.rs

#[derive(Clone, Debug)]
pub enum NousEvent {
    MemoryStored { id: MemoryId, workspace_path: Option<String> },
    MemoryUpdated { id: MemoryId },
    MemoryForgotten { id: MemoryId, hard: bool },
    RoomMessagePosted { room_id: String, message_id: String, sender_id: String },
    ScheduleTriggered { schedule_id: String, action_type: String },
    EmbeddingComplete { memory_id: MemoryId },
}

pub struct EventBus {
    tx: broadcast::Sender<NousEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: NousEvent) {
        // Receivers lagging behind are dropped; no blocking on slow subscribers.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<NousEvent> {
        self.tx.subscribe()
    }
}
```

`broadcast::Sender::send` returns `Err` only when there are no active receivers, which is harmless — the event is discarded rather than blocking the publisher. Slow subscribers that fall behind `capacity` lose events (`RecvError::Lagged`); feature code that cannot tolerate lag should use `try_recv` and handle the lagged case explicitly.

### Subscriber Pattern

```rust
// Example subscriber — cache invalidation on memory update
let mut rx = event_bus.subscribe();
tokio::spawn(async move {
    loop {
        match rx.recv().await {
            Ok(NousEvent::MemoryUpdated { id }) => {
                cache.invalidate(&id);
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("event bus: dropped {n} events, invalidating full cache");
                cache.invalidate_all();
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
});
```

### Multi-Node Considerations

`tokio::sync::broadcast` is in-process only. For multi-node deployments, Postgres `LISTEN/NOTIFY` will be used when the Postgres backend ships. No additional infrastructure (NATS, Redis) is required.

| Option | Decision |
|--------|----------|
| **Postgres LISTEN/NOTIFY** | **Chosen.** No new infrastructure if Postgres is already present. Delivery limited to 8 KB payload per notification — sufficient for event envelopes with IDs. |
| ~~NATS~~ | Rejected — requires an additional process, violates zero-external-deps goal. |
| ~~Redis pub/sub~~ | Rejected — same infrastructure concern as NATS. |

The `EventBus` abstraction above can be backed by `LISTEN/NOTIFY` by changing the `publish` and `subscribe` implementations. The event enum stays the same; only the transport changes. For the current SQLite-only deployment, the in-process broadcast channel is sufficient.

## 8. Error Handling Strategy

### Unified Error Type

`NousError` (`crates/nous-shared/src/error.rs`) is the single error type used across all crates. It uses `thiserror` for `Display` and `From` implementations:

```rust
#[derive(Debug, thiserror::Error)]
pub enum NousError {
    #[error("sqlite error: {0}")]      Sqlite(#[from] sqlx::Error),
    #[error("io error: {0}")]          Io(#[from] std::io::Error),
    #[error("config error: {0}")]      Config(String),
    #[error("internal error: {0}")]   Internal(String),
    #[error("embedding error: {0}")] Embedding(String),
    #[error("validation error: {0}")] Validation(String),
    #[error("not found: {0}")]        NotFound(String),
    #[error("conflict: {0}")]         Conflict(String),
    #[error("invalid input: {0}")]   InvalidInput(String),
}
```

### Error Surface by Interface

**MCP tools** — `CallToolResult` carries the error as text content with `is_error = Some(true)`. Tool handlers serialize the `NousError` message into the JSON error body:

```
{ "isError": true, "content": [{ "type": "text", "text": "{\"error\": \"not found: memory 01964...\"}" }] }
```

The MCP client (Claude Code) reads `isError` and presents the error message to the model as a tool failure rather than crashing the session.

**CLI** — Errors print to stderr using `eprintln!` and the process exits with a code from `NousError::exit_code()`:

| Error variant | Exit code |
|---------------|-----------|
| `Validation`, `InvalidInput` | 2 |
| `NotFound` | 3 |
| `Conflict` | 4 |
| All others (`Sqlite`, `Io`, `Internal`, etc.) | 1 |

When `--format json` is active, error output is still written to stderr as plain text so that the stdout JSON stream remains parseable by downstream tools.

**Daemon API** — `call_tool_result_to_response` (`daemon_api.rs:458`) maps `CallToolResult` to HTTP status codes: `is_error = true` → `400 Bad Request`; otherwise `200 OK`. Axum's unprocessable-entity handling (`422`) covers malformed request bodies before handlers are reached.

**Internal Result types** — All internal functions return `nous_shared::Result<T>` (aliased `std::result::Result<T, NousError>`). The `?` operator propagates errors up to the interface boundary where they are translated to the appropriate surface representation.

### HTTP Status Code Mapping

| NousError variant | HTTP Status | Code |
|-------------------|-------------|------|
| `NotFound` | Not Found | 404 |
| `Validation` / `InvalidInput` | Bad Request | 400 |
| `Conflict` | Conflict | 409 |
| `Sqlite` / `Io` / `Internal` / `Embedding` | Internal Server Error | 500 |
| `Config` | Internal Server Error | 500 |

### Retryable Errors

Only `SQLITE_BUSY` is retried (3 attempts, 100ms backoff between each). All other errors are terminal and propagated immediately to the caller.

### anyhow Migration

`nous-otlp` will migrate from `anyhow` to `NousError` for consistency across the codebase. This is planned but not blocking MVP.

## 9. ID Generation

### UUIDv7 Everywhere

All entity IDs — memories, rooms, messages, schedules — are UUIDv7 strings. `MemoryId::new()` (`crates/nous-shared/src/ids.rs:39`) calls `uuid::Uuid::now_v7().to_string()`:

```rust
// crates/nous-shared/src/ids.rs

impl MemoryId {
    pub fn new() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }
}
```

`SessionId`, `TraceId`, and `SpanId` use the same `define_id!` macro pattern but do not implement `new()` — they are created externally (by the OTLP ingestion path or by the calling agent) and parsed via `FromStr`.

### Why UUIDv7

UUIDv7 encodes a millisecond-precision Unix timestamp in the high 48 bits. Three properties follow from this:

1. **Lexicographic ordering** — sorting UUID strings by byte value produces creation-time order. SQLite `ORDER BY id` on a text column gives correct chronological order without a separate `created_at` index.
2. **Globally unique** — the remaining 74 bits include a random component, making collisions negligible even across concurrent writers.
3. **Embedded timestamp** — the creation time is recoverable from the ID without a database lookup, which simplifies log correlation and debugging.

### ID Flow

```
MCP tool call arrives
        │
        ▼
Tool handler generates MemoryId::new()
        │  (at the API boundary, before any DB call)
        ▼
MemoryId passed to WriteChannel::store_memory(id, ...)
        │
        ▼
WriteChannel sends DbWrite::StoreMemory to the writer task
        │
        ▼
MemoryDb writes the ID as the primary key TEXT column
        │
        ▼
ID returned in CallToolResult to the MCP client
```

IDs are generated once at the API boundary (MCP tool handler or daemon API handler) and never regenerated. `WriteChannel` accepts a caller-supplied ID rather than generating one internally, which means the ID is known to the caller before the write completes — enabling the caller to return it immediately without waiting for the writer task to flush.

## 10. Dependencies Between Documents

**[01-system-architecture.md](01-system-architecture.md)** defines the daemon as the runtime that owns the MCP server process, the Unix socket, and the PID file. This document assumes that runtime model: the MCP server runs inside the daemon's process space, and the daemon API routes share the same `NousServer` instance via `Arc<NousServer>`.

**[02-data-layer.md](02-data-layer.md)** defines `WriteChannel` and `ReadPool` — the two abstractions that all interfaces in this document use for data access. MCP tool handlers receive both by reference from `NousServer`. Daemon API handlers access them via `Arc<AppState>` which holds `Arc<NousServer>`. The CLI opens its own `MemoryDb` connection (bypassing `WriteChannel`) only for batch operations (`admin import`, `admin export`) that run outside a live daemon session.

## 11. Open Questions

| # | Question | Resolution |
|---|----------|-----------|
| 1 | **Worker coordination protocol.** | **Resolved:** reuse the existing Axum HTTP/JSON daemon API. Workers forward `WriteOp` variants as JSON POST requests to the coordinator. No gRPC or custom IPC protocol. |
| 2 | **Event bus technology for multi-node.** | **Resolved:** Postgres `LISTEN/NOTIFY` when the Postgres backend ships. No NATS or Redis. See §7. |
| 3 | **MCP tool versioning strategy.** | **Resolved:** additive-only evolution. Tool parameters are only added, never removed or type-changed. No breaking changes post-launch. Per-tool versioning is unnecessary given this constraint. |
| 4 | **MCP error codes.** | **Resolved:** deferred. Standard `is_error` boolean is sufficient for MVP. Structured MCP error codes will be added when client-side handling requires them. |
| 5 | **CLI plugin system for extensions.** | Open. The `#[tool_router]` macro is compile-time. The simpler alternative is a subprocess-based plugin model: the MCP server proxies tool calls to a child process that speaks MCP on stdio. |

## 12. Cross-Reference Index

| Topic | Primary doc | Related sections |
|-------|-------------|-----------------|
| WriteChannel / ReadPool | 02-data-layer.md §6 | 01-system-architecture.md §7 (Service Topology) |
| StorageBackend trait | 02-data-layer.md §4 | This doc §6 (Internal Service APIs) |
| Deployment modes | 01-system-architecture.md §6 | This doc §6 (Daemon Architecture) |
| SQLite schema & tables | 02-data-layer.md §5 | This doc §4 (MCP Tool Catalog) |
| Error handling | This doc §8 | 02-data-layer.md §9 (Transaction Semantics) |
| Event bus | This doc §7 | 01-system-architecture.md §5 (Horizontal Scaling) |
| Configuration | 01-system-architecture.md §8 | 02-data-layer.md §8 (Connection Pooling) |
| Health checks | This doc §6 (`GET /health`) | 01-system-architecture.md §9 |
| `nous doctor` | This doc §5.5 | 01-system-architecture.md §8 (Config) |
| Multi-node coordination | 01-system-architecture.md §5 | This doc §11 Q1 |
