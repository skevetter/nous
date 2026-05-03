# System Architecture

## Table of Contents

1. [Goals](#1-goals)
2. [Non-Goals](#2-non-goals)
3. [Architecture Overview](#3-architecture-overview)
4. [Daemon / Service Model](#4-daemon--service-model)
5. [Horizontal Scaling](#5-horizontal-scaling)
6. [Deployment Modes](#6-deployment-modes)
7. [Service Topology](#7-service-topology)
8. [Configuration Management](#8-configuration-management)
9. [Health Checks](#9-health-checks)
10. [Graceful Shutdown](#10-graceful-shutdown)
11. [Dependencies Between Documents](#11-dependencies-between-documents)
12. [Open Questions](#12-open-questions)

---

## 1. Goals

- **Single-node simplicity.** `nous daemon start` on a developer laptop starts a fully functional instance in under two seconds with zero infrastructure dependencies — no external database, no message broker, no service registry.
- **Multi-node scalability.** The coordinator + worker topology (§5) lets operators add worker processes without changing the coordinator. Workers share the same SQLite database on a shared volume, or switch to a Postgres backend for cloud deployments.
- **Uniform API surface.** Every deployment mode exposes the same MCP protocol (stdio or HTTP) and the same daemon REST API over a Unix socket. Application code does not change between dev and production.
- **Observability from day one.** Every scheduled action emits an OTLP span pair (`schedule.run` → `schedule.action`) into the local OTLP database. The daemon `/status` endpoint exposes PID and uptime. No external collector required.
- **Predictable shutdown.** SIGTERM/SIGINT triggers a drain sequence that flushes the write channel before closing database connections, preventing partial transactions on restart.

## 2. Non-Goals

- **High-availability coordinator.** Leader election, failover, and coordinator redundancy are deferred. The coordinator is a single process; losing it stops scheduling until it restarts.
- **Distributed transactions.** Workers share state through SQLite (single-file) or Postgres. Cross-node atomic writes are not supported; each write is serialized through the write channel on the node that owns it.
- **Authentication and authorization.** The daemon API socket (`~/.cache/nous/daemon.sock`) relies on Unix filesystem permissions. Network-accessible deployments (Docker/k8s) must front the daemon with a proxy that handles authn.
- **Schema migrations in this document.** Migration strategy is covered in `02-data-layer.md`.
- **MCP protocol internals.** Tool definitions, parameter schemas, and call routing are covered in `03-api-interfaces.md`.

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│  Client Layer                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────┐    │
│  │  MCP Host    │  │  CLI (nous)  │  │  HTTP REST Client  │    │
│  │  (stdio/HTTP)│  │              │  │  (daemon API)      │    │
│  └──────┬───────┘  └──────┬───────┘  └─────────┬──────────┘    │
└─────────┼────────────────┼────────────────────┼────────────────┘
          │ MCP protocol   │ CLI commands        │ Unix socket HTTP
          ▼                ▼                     ▼
┌─────────────────────────────────────────────────────────────────┐
│  nous binary (single process)                                   │
│                                                                 │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  NousServer                                             │   │
│  │  ┌─────────────┐  ┌──────────┐  ┌───────────────────┐  │   │
│  │  │ MCP Router  │  │Scheduler │  │  Daemon API        │  │   │
│  │  │ (rmcp)      │  │          │  │  (axum/Unix sock)  │  │   │
│  │  └──────┬──────┘  └────┬─────┘  └────────┬──────────┘  │   │
│  │         │              │                  │              │   │
│  │         └──────────────┴──────────────────┘              │   │
│  │                        │                                  │   │
│  │         ┌──────────────▼──────────────┐                  │   │
│  │         │       WriteChannel          │                  │   │
│  │         │  (cap=256, batch=32 ops)    │                  │   │
│  │         └──────────────┬──────────────┘                  │   │
│  │                        │                                  │   │
│  │         ┌──────────────▼──────────────┐                  │   │
│  │         │  EmbeddingBackend           │                  │   │
│  │         │  (ONNX: Qwen3-0.6B, 1024d) │                  │   │
│  │         └─────────────────────────────┘                  │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
│  ┌───────────────────────┐  ┌────────────────────────────┐     │
│  │  SQLite (memory.db)   │  │  SQLite (otlp.db)          │     │
│  │  WAL mode             │  │  OTLP spans                │     │
│  │                       │  └────────────────────────────┘     │
│  │  SQLite (memory-fts.db)  SQLite (memory-vec.db)        │     │
│  │  FTS5 tables             vec0 tables                   │     │
│  └───────────────────────┘                                     │
└─────────────────────────────────────────────────────────────────┘
```

The `nous` binary is a single Rust binary containing all subsystems. `nous serve` exposes only the MCP server (stdio or HTTP). `nous daemon start` runs the MCP server plus the daemon API on a Unix socket, enabling `nous daemon status` and `nous daemon stop` from other processes.

## 4. Daemon / Service Model

`nous daemon start` forks a background process that owns two resources:

| Resource | Default path | Purpose |
|---|---|---|
| PID file | `~/.cache/nous/daemon.pid` | Mutual-exclusion guard — second start fails if file exists and process is alive |
| Unix socket | `~/.cache/nous/daemon.sock` | axum HTTP server for the daemon REST API |

### Startup sequence

1. `Daemon::new` checks `~/.cache/nous/daemon.pid`. If the file exists and `/proc/<pid>` is present, it returns `DaemonError::AlreadyRunning(pid)`. If the file exists but the process is gone (stale PID), it removes the file and continues.
2. Writes the current PID to the PID file.
3. Binds a `UnixListener` on the socket path (removes a stale socket if present).
4. Creates a `watch::channel(false)` for shutdown signaling.
5. Installs signal handlers for SIGTERM and SIGINT; both send `true` on the watch channel. SIGHUP is caught but currently a no-op (reserved for future config reload).
6. Calls `axum::serve(listener, router).with_graceful_shutdown(...)`, which drains in-flight requests before returning.
7. On exit (clean or signal), removes the PID file and socket.

### Daemon REST API routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/status` | Returns `{ pid, uptime_secs, version }` |
| `POST` | `/shutdown` | Sends shutdown signal; returns `{ ok: true }` |
| `POST` | `/rooms` | Create a room |
| `GET` | `/rooms` | List rooms |
| `GET` | `/rooms/{id}` | Get room by UUID or name |
| `POST` | `/rooms/{id}/messages` | Post a message |
| `GET` | `/rooms/{id}/messages` | Read messages (supports `limit`, `since`, `before`) |
| `POST` | `/memories/search` | Search memories |
| `POST` | `/memories/store` | Store a memory |
| `GET` | `/categories` | List categories |
| `POST` | `/export` | Export all memories as JSON |
| `POST` | `/import` | Import memories from JSON |

The daemon API and the MCP server share the same `NousServer` instance — the same `WriteChannel`, `ReadPool`, and `EmbeddingBackend`. There is no internal message passing between them; they call the same Rust methods directly.

## 5. Horizontal Scaling

> **Post-MVP.** The coordinator/worker multi-node topology described in this section is deferred. MVP is single-node only. This section documents the target architecture for reference.

The current codebase is a single-node design. The horizontal scaling model described here is the target architecture.

### Coordinator + Worker pattern

```
                    ┌──────────────────────┐
                    │     Coordinator      │
                    │                      │
                    │  - Task dispatcher   │
                    │  - WriteChannel owner│
                    │  - Scheduler         │
                    │  - Daemon API        │
                    └──────┬───────────────┘
                           │  Shared database
             ┌─────────────┼─────────────┐
             ▼             ▼             ▼
      ┌────────────┐ ┌────────────┐ ┌────────────┐
      │  Worker 1  │ │  Worker 2  │ │  Worker N  │
      │            │ │            │ │            │
      │  - MCP srv │ │  - MCP srv │ │  - MCP srv │
      │  - Embed   │ │  - Embed   │ │  - Embed   │
      │  - ReadPool│ │  - ReadPool│ │  - ReadPool│
      └────────────┘ └────────────┘ └────────────┘
```

**Coordinator responsibilities:**
- Owns the single `WriteChannel` (serializes all writes into the SQLite WAL).
- Runs the `Scheduler` loop (`max_concurrent = 4` scheduled tasks, configurable).
- Exposes the daemon REST API for management operations.

**Worker responsibilities:**
- Runs an MCP server instance (stdio or HTTP).
- Owns a local `ReadPool` (4 connections, `PRAGMA query_only = ON`).
- Owns a local `EmbeddingBackend` (CPU-bound ONNX inference; no sharing required).
- Forwards write operations to the coordinator via an IPC channel (mechanism TBD — see §12).

**Task distribution:** MCP clients connect directly to any worker. Read operations (search, recall) are served from the worker's local `ReadPool`. Write operations (store, update, forget) are forwarded to the coordinator's `WriteChannel` and batched into transactions of up to 32 operations.

**Internal feature communication:** Components within a single process communicate through Rust method calls on shared `Arc<NousServer>`. Across processes (coordinator ↔ worker), the write-path IPC replaces this with an explicit protocol (mechanism under design).

## 6. Deployment Modes

### 6a. systemd (Linux) — Primary

Linux with systemd is the primary deployment target. A single binary installed from a `.deb`/`.rpm` package or direct download.

```ini
# /etc/systemd/system/nous.service
[Unit]
Description=Nous memory daemon
After=network.target

[Service]
Type=simple
User=nous
ExecStart=/usr/local/bin/nous daemon start
Restart=on-failure
RestartSec=5s
Environment=NOUS_MEMORY_DB=/var/lib/nous/memory.db
Environment=NOUS_OTLP_DB=/var/lib/nous/otlp.db

[Install]
WantedBy=multi-user.target
```

```bash
systemctl enable --now nous
systemctl status nous
journalctl -u nous -f
```

The daemon socket lands at `/home/nous/.cache/nous/daemon.sock` (or override with `NOUS_DAEMON_SOCKET`). If the socket needs to be world-accessible, mount a `tmpfs` at a shared path and set `NOUS_DAEMON_SOCKET=/run/nous/daemon.sock`, adjusting the unit's `RuntimeDirectory=nous`.

---

### 6b. Homebrew service (macOS) — Secondary/Future

A single binary installed by `brew install nous`. `brew services start nous` installs a launchd plist that keeps the daemon alive across reboots.

**Example launchd plist** (`~/Library/LaunchAgents/com.nous.daemon.plist`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>         <string>com.nous.daemon</string>
  <key>ProgramArguments</key>
  <array>
    <string>/opt/homebrew/bin/nous</string>
    <string>daemon</string>
    <string>start</string>
  </array>
  <key>KeepAlive</key>     <true/>
  <key>RunAtLoad</key>     <true/>
  <key>StandardOutPath</key>
  <string>/Users/me/.cache/nous/daemon.log</string>
  <key>StandardErrorPath</key>
  <string>/Users/me/.cache/nous/daemon.log</string>
</dict>
</plist>
```

Config lives at `~/.config/nous/config.toml` (XDG base dir). The databases at `~/.cache/nous/` (memory.db, memory-fts.db, memory-vec.db, otlp.db) are user-owned files. No additional ports are opened.

---

### 6c. Docker / Kubernetes (multi-node)

The Docker image runs `nous serve` directly. No systemd inside the container.

```dockerfile
FROM debian:bookworm-slim
COPY nous /usr/local/bin/nous
RUN useradd -m nous
USER nous
ENTRYPOINT ["nous", "serve", "--transport", "http", "--port", "8377"]
```

```yaml
# coordinator deployment
apiVersion: apps/v1
kind: Deployment
metadata:
  name: nous-coordinator
spec:
  replicas: 1
  template:
    spec:
      containers:
      - name: nous
        image: nous:latest
        env:
        - name: NOUS_MEMORY_DB
          value: /data/memory.db
        - name: NOUS_OTLP_DB
          value: /data/otlp.db
        volumeMounts:
        - name: data
          mountPath: /data
      volumes:
      - name: data
        persistentVolumeClaim:
          claimName: nous-data
```

In multi-node deployments, the coordinator owns the write path and runs the scheduler. Worker pods mount the same PVC (or point to a shared Postgres backend — see §12). The daemon socket inside each pod is not accessible from outside; expose the MCP HTTP port (`mcp_port = 8377`) through a `Service` instead.

## 7. Service Topology

```
  MCP client (stdio)           MCP client (HTTP :8377)
        │                               │
        ▼                               ▼
  ┌─────────────────────────────────────────────┐
  │               rmcp MCP Server               │
  │  tool_router! macro dispatches to NousServer│
  └──────────────────────┬──────────────────────┘
                         │ Arc<NousServer>
          ┌──────────────┼──────────────────┐
          │              │                  │
          ▼              ▼                  ▼
  ┌──────────────┐ ┌──────────────┐ ┌──────────────────┐
  │WriteChannel  │ │  ReadPool    │ │EmbeddingBackend   │
  │              │ │              │ │                   │
  │ cap=256 ops  │ │ 4 conns      │ │ Qwen3-0.6B ONNX   │
  │ batch≤32     │ │ query_only   │ │ 1024-dim vectors  │
  └──────┬───────┘ └──────┬───────┘ └──────────────────┘
         │                │
         ▼                ▼
  ┌──────────────────────────────────┐
  │   SQLite memory.db (WAL mode)    │
  │   memories, chunks, embeddings   │
  │   categories, rooms, messages    │
  │   schedules, schedule_runs       │
  └──────────────────────────────────┘
         │
         ▼
  ┌──────────────────────────────────┐
  │   Scheduler (tokio::spawn loop)  │
  │   polls next_run_at from DB      │
  │   max_concurrent semaphore = 4   │
  │   emits OTLP spans on each run   │
  └──────────────┬───────────────────┘
                 │
                 ▼
  ┌──────────────────────────────────┐
  │   SQLite otlp.db                 │
  │   schedule.run / schedule.action │
  │   spans (trace_id = UUIDv7)      │
  └──────────────────────────────────┘
```

**Write path:** All mutations (store memory, post message, create room, create schedule) go through `WriteChannel`. The channel queues up to 256 `WriteOp` variants and drains them in batches of up to 32 inside a single SQLite transaction. A single write-worker goroutine serializes all writes, eliminating WAL conflicts.

**Read path:** `ReadPool` holds 4 connections opened with `PRAGMA query_only = ON`. A `Semaphore(4)` ensures at most 4 concurrent read operations. Reads bypass the write channel entirely.

**Embedding path:** When storing a memory, `NousServer` calls `EmbeddingBackend.embed()` synchronously before sending to the write channel, so the chunk vectors arrive with the write batch. Inference runs on CPU via ONNX Runtime.

## 8. Configuration Management

### config.toml structure

The config file lives at `~/.config/nous/config.toml` (XDG). If absent, `Config::load` creates it with all defaults.

```toml
[memory]
db_path     = "~/.cache/nous/memory.db"
fts_db_path = "~/.cache/nous/memory-fts.db"
vec_db_path = "~/.cache/nous/memory-vec.db"

[embedding]
model   = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "onnx/model_q4.onnx"
dimensions   = 1024
chunk_size   = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port    = 4318

[classification]
confidence_threshold = 0.3

[rooms]
max_rooms              = 1000
max_messages_per_room  = 10000

[daemon]
socket_path          = "~/.cache/nous/daemon.sock"
pid_file             = "~/.cache/nous/daemon.pid"
log_file             = "~/.cache/nous/daemon.log"
mcp_transport        = "stdio"          # "stdio" | "http"
mcp_port             = 8377
shutdown_timeout_secs = 30

[schedule]
enabled              = true
allow_shell          = true             # shell actions enabled by default
allow_http           = true
max_concurrent       = 4
default_timeout_secs = 300
```

### Environment variable overrides

Environment variables override the file and take effect at startup. Empty string values are ignored.

| Variable | Overrides | Type |
|---|---|---|
| `NOUS_MEMORY_DB` | `memory.db_path` | string (path) |
| `NOUS_OTLP_DB` | `otlp.db_path` | string (path) |
| `NOUS_ROOMS_MAX` | `rooms.max_rooms` | integer |
| `NOUS_ROOMS_MAX_MESSAGES` | `rooms.max_messages_per_room` | integer |
| `NOUS_DAEMON_SOCKET` | `daemon.socket_path` | string (path) |
| `NOUS_SCHEDULE_ENABLED` | `schedule.enabled` | `"0"` or `"false"` disables |
| `NOUS_SCHEDULE_ALLOW_SHELL` | `schedule.allow_shell` | `"1"` or `"true"` enables |

### Precedence

```
environment variable  >  config.toml  >  compiled defaults
```

The CLI flags `--config` and `--db` override the config-file path and database path respectively, taking precedence over all other sources for those fields.

### XDG Directory Handling

Nous creates XDG directories (`$XDG_CONFIG_HOME/nous`, `$XDG_CACHE_HOME/nous`) if missing on first run. Falls back to `~/.nous` only if XDG environment variables are explicitly unset AND the platform is not Linux. On Linux, the XDG Base Directory Specification defaults apply even without the environment variables (`~/.config/nous`, `~/.cache/nous`).

## 9. Health Checks

Both probes go to the daemon API socket via Unix HTTP.

```bash
curl --unix-socket ~/.cache/nous/daemon.sock http://localhost/status
# { "pid": 12345, "uptime_secs": 3607, "version": "0.1.0" }
```

| Probe | Endpoint | Pass condition | Fail condition |
|---|---|---|---|
| **Liveness** | `GET /status` | HTTP 200 | Connection refused (daemon down) or no response within 5 s |
| **Readiness** | `GET /status` (uptime check) | `uptime_secs > 0` | `uptime_secs == 0` (startup not complete) |

Kubernetes liveness/readiness probes should use `exec` to call the daemon API via the Unix socket, since k8s HTTP probes target TCP ports:

```yaml
livenessProbe:
  exec:
    command:
    - sh
    - -c
    - >
      curl -sf --unix-socket /root/.cache/nous/daemon.sock
      http://localhost/status > /dev/null
  initialDelaySeconds: 5
  periodSeconds: 10

readinessProbe:
  exec:
    command:
    - sh
    - -c
    - >
      curl -sf --unix-socket /root/.cache/nous/daemon.sock
      http://localhost/status | grep -q '"uptime_secs"'
  initialDelaySeconds: 2
  periodSeconds: 5
```

For the MCP HTTP transport, a separate TCP liveness probe on `mcp_port` (default 8377) is appropriate once that transport is enabled.

**What `/status` checks:** The axum handler returns synchronously using an `Instant` captured at server startup. It does not query the database, so it does not validate storage health. A separate storage health endpoint is a candidate for a future addition (see §12).

## 10. Graceful Shutdown

### Signal handling

`Daemon::install_signal_handlers` spawns a Tokio task that waits for any of three signals:

| Signal | Action |
|---|---|
| `SIGTERM` | Sends `true` on the `watch::Sender<bool>` shutdown channel |
| `SIGINT` | Same as SIGTERM |
| `SIGHUP` | No-op — reserved for future live config reload |

The same shutdown channel is available via `POST /shutdown` on the daemon API, which sends `true` directly on the watch sender.

### Drain sequence

`axum::serve(...).with_graceful_shutdown(...)` waits for `shutdown_rx.wait_for(|&v| v)` before returning. The drain order is:

```
1. Stop accepting new connections on the Unix socket
2. Serve in-flight HTTP requests to completion
3. axum returns from serve()
4. NousServer is dropped:
   - WriteChannel tx is dropped → write-worker loop exits after draining queued ops
   - ReadPool connections are dropped → SQLite WAL checkpoint runs on close
   - EmbeddingBackend dropped → ONNX session released
5. Daemon::run removes PID file and socket file
6. Process exits
```

The configured `shutdown_timeout_secs` (default 30 s) is the ceiling for step 2. If in-flight requests do not complete within this window, axum forces close. This timeout is read from `DaemonConfig.shutdown_timeout_secs` and passed to `Daemon::shutdown_timeout_secs()`.

### Partial-write recovery

On startup, the write worker checks for any `schedule_runs` rows with `status = 'running'` and marks them `'failed'` with `error = 'process restarted'`. This handles runs interrupted by a crash or hard kill before graceful shutdown completes.

## 11. Dependencies Between Documents

| Document | Relationship to this document |
|---|---|
| `02-data-layer.md` | Covers the SQLite schema, WAL configuration, vector index (`sqlite-vec`), and migration strategy. This document assumes the data layer exists and references `WriteChannel` / `ReadPool` as the access abstraction. |
| `03-api-interfaces.md` | Covers the MCP tool definitions (`memory_store`, `memory_search`, etc.), parameter schemas, and the `rmcp` transport layer (stdio vs HTTP). This document treats the MCP server as a black box that calls `NousServer` methods. |

**Naming conventions (cross-document):**
- IDs use UUIDv7 throughout (`uuid::Uuid::now_v7()`), providing time-ordered sortability. `MemoryId`, `SessionId`, `TraceId`, `SpanId` are all newtype wrappers over the same string representation.
- Error handling uses `nous_shared::NousError` as the canonical error type; all internal results are `nous_shared::Result<T>`.
- Events use the OTLP span model — no bespoke event bus. The `schedule.run` / `schedule.action` parent-child span pair is the established pattern for structured events.

## 12. Open Questions

| # | Question | Resolution |
|---|---|---|
| 1 | **Worker → coordinator write IPC mechanism.** | **Resolved:** reuse the existing Axum HTTP/JSON daemon API. Workers forward `WriteOp` variants as JSON POST requests to the coordinator. No gRPC or custom IPC protocol. |
| 2 | **Coordinator HA strategy.** If the coordinator crashes, scheduled tasks stop firing until it restarts. | Accept single-coordinator limitation (YAGNI). Revisit only if availability SLA requires it. |
| 3 | **Storage backend for multi-node.** SQLite + shared PVC works for single-AZ k8s. Cross-AZ or multi-write requires a different backend. | Postgres backend via `sqlx` (deferred post-MVP). See `02-data-layer.md`. |
| 4 | **Primary packaging target.** | **Resolved:** Linux-first (systemd/deb/rpm). macOS launchd is secondary/future. |
| 5 | **Storage health in `/status`.** The liveness probe currently returns without touching the database. A failed `ReadPool` connection would go undetected. | **Resolved:** Add `GET /health` endpoint with `SELECT 1` DB check. See `03-api-interfaces.md`. |
