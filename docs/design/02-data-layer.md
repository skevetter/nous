# Data Layer

## Table of Contents

1. [Goals](#1-goals)
2. [Non-Goals](#2-non-goals)
3. [Architecture Overview](#3-architecture-overview)
4. [Backend Abstraction](#4-backend-abstraction)
5. [Schema Design Patterns](#5-schema-design-patterns)
6. [WriteChannel / ReadPool Pattern](#6-writechannel--readpool-pattern)
7. [Migration Strategy](#7-migration-strategy)
8. [Connection Pooling](#8-connection-pooling)
9. [Transaction Semantics](#9-transaction-semantics)
10. [Dependencies Between Documents](#10-dependencies-between-documents)
11. [Open Questions](#11-open-questions)

---

## 1. Goals

The data layer supports two deployment targets from a single codebase:

| Target | Backend | Deployment |
|--------|---------|------------|
| Single-node (developer machine, systemd service) | SQLite (3 files: memory.db, memory-fts.db, memory-vec.db) | One process |
| Multi-node (k8s, Docker Compose, managed cloud) | PostgreSQL | N stateless app replicas, shared DB |

Concrete goals:

- **Unified read/write API** — application code calls `WriteChannel` and `ReadPool` without knowing which backend is active. No SQL leaks above the data layer boundary.
- **Feature parity** — full-text search, semantic vector search, room messaging, and schedules work on both backends. The mechanism differs (FTS5 vs `tsvector`; sqlite-vec vs `pgvector`), but the query interface is identical.
- **Atomic migrations** — schema is applied once at startup, inside a transaction, from a versioned array embedded in the binary. No external migration tool is required.
- **Transparent concurrency** — single-writer / multi-reader on SQLite (WAL + `ReadPool`); connection-pool-based concurrency on Postgres with serializable transactions for write batches.

## 2. Non-Goals

- **No ORM / query builder** — SQL is written by hand using `sqlx` with compile-time checked queries. The abstraction boundary is a Rust trait, not a DSL. No Diesel or SeaORM.
- **Multi-master replication** — Postgres target is single primary. Read replicas are a future concern, not addressed here.
- **Schema-per-tenant isolation** — all tenants share one schema. Row-level filtering via `agent_id` and `workspace_id` columns.
- **Automatic backend detection** — the backend is selected from config, not inferred from the environment. Auto-detection is an open question (see §12).
- **Online schema migrations** — migrations run at startup and are not designed for zero-downtime column drops. Additive migrations (new columns, new indexes) are safe; destructive changes require a maintenance window.
- **SQLite → Postgres data migration tooling** — exporting an existing SQLite database into Postgres is out of scope for this document.

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                     Application Layer                        │
│   MCP tools  ·  CLI commands  ·  Scheduler  ·  OTLP ingest  │
└────────────────────────┬────────────────────────────────────┘
                         │  calls
         ┌───────────────┴───────────────┐
         │                               │
    ┌────▼────────┐              ┌────────▼──────┐
    │WriteChannel │              │   ReadPool    │
    │ (mpsc tx)   │              │ (Semaphore +  │
    │ cap=256     │              │  N conns)     │
    │ batch ≤32   │              └───────┬───────┘
    └──────┬──────┘                      │
           │                             │
           └──────────┬──────────────────┘
                      │  implements
              ┌───────▼────────┐
              │ StorageBackend │  (trait — §4)
              └───────┬────────┘
                      │
          ┌───────────┴────────────┐
          │                        │
   ┌──────▼──────┐          ┌──────▼───────┐
   │   SQLite    │          │  PostgreSQL  │
   │   sqlx      │          │   sqlx       │
   │  + sqlite-  │          │  (planned)   │
   │  vec (vec0) │          │  pgvector +  │
   │  + FTS5     │          │  tsvector    │
   └─────────────┘          └─────────────┘
```

**Data flow — write path:**

1. Application calls `WriteChannel::store(memory)`.
2. A `WriteOp::Store` message is sent on the `mpsc` channel (capacity 256).
3. The write worker drains up to 32 ops into a batch, opens one transaction, executes all ops, and commits.
4. Each op carries a `oneshot::Sender`; the worker sends the result back after the commit.

**Data flow — read path:**

1. Application calls `ReadPool::with_conn(|conn| { … })`.
2. The pool acquires a semaphore permit, pops a connection, and runs the closure on a `spawn_blocking` thread.
3. The connection is returned to the pool after the closure completes (including on panic, via `catch_unwind`).

## 4. Backend Abstraction

### Proposed trait surface

Both backends implement a single `StorageBackend` trait. The trait covers the write operations dispatched from `WriteChannel` and the read operations currently on `MemoryDb` / `ReadPool`.

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync + 'static {
    // --- Memory CRUD ---
    async fn store(&self, memory: &NewMemory) -> Result<MemoryId>;
    async fn recall(&self, id: &MemoryId) -> Result<Option<MemoryWithRelations>>;
    async fn update(&self, id: &MemoryId, patch: &MemoryPatch) -> Result<bool>;
    async fn forget(&self, id: &MemoryId, hard: bool) -> Result<bool>;
    async fn unarchive(&self, id: &MemoryId) -> Result<bool>;

    // --- Relationships ---
    async fn relate(&self, src: &MemoryId, tgt: &MemoryId, rel: RelationType) -> Result<()>;
    async fn unrelate(&self, src: &MemoryId, tgt: &MemoryId, rel: RelationType) -> Result<bool>;

    // --- Chunks / embeddings ---
    async fn store_chunks(
        &self, id: &MemoryId, chunks: &[Chunk], embeddings: &[Vec<f32>],
    ) -> Result<()>;
    async fn delete_chunks(&self, id: &MemoryId) -> Result<()>;

    // --- Search ---
    async fn search_fts(&self, query: &str, filters: &SearchFilters) -> Result<Vec<SearchResult>>;
    async fn search_semantic(
        &self, embedding: &[f32], filters: &SearchFilters,
    ) -> Result<Vec<SearchResult>>;

    // --- Rooms ---
    async fn create_room(
        &self, id: &str, name: &str, purpose: Option<&str>, metadata: Option<&str>,
    ) -> Result<()>;
    async fn post_message(
        &self, id: &str, room_id: &str, sender_id: &str,
        content: &str, reply_to: Option<&str>, metadata: Option<&str>,
    ) -> Result<()>;
    async fn list_messages(
        &self, room_id: &str, limit: Option<usize>,
        before: Option<String>, since: Option<String>,
    ) -> Result<Vec<Message>>;
    async fn search_messages(
        &self, room_id: &str, query: &str, limit: Option<usize>,
    ) -> Result<Vec<Message>>;

    // --- Schedules ---
    async fn create_schedule(&self, schedule: &Schedule) -> Result<String>;
    async fn update_schedule(&self, id: &str, patch: &SchedulePatch) -> Result<bool>;
    async fn delete_schedule(&self, id: &str) -> Result<bool>;
    async fn record_run(&self, run: &ScheduleRun) -> Result<String>;

    // --- Migrations ---
    async fn run_migrations(&self) -> Result<()>;
}
```

### Library choice

| Library | Async | Compile-time SQL checks | Notes |
|---------|-------|------------------------|-------|
| **`sqlx`** (chosen) | Yes — native async | Yes (`sqlx::query!` macro with offline mode) | Unified backend for both SQLite and Postgres. Compile-time query checking via `sqlx prepare` (offline snapshots — no live DB required at build time). sqlite-vec and FTS5 extensions loaded at runtime. |
| `diesel` | No (sync) | Yes (schema.rs) | Schema-first codegen; sync model is awkward in an async runtime. Not used. |

**Decision:** `sqlx` is the unified database library from day one, providing compile-time checked queries for both SQLite and Postgres backends. SQL is written by hand (no query builder) — `sqlx` validates the SQL at compile time using offline `.sqlx` snapshots generated by `cargo sqlx prepare`. The `StorageBackend` trait implementations use `sqlx::SqlitePool` and `sqlx::PgPool` respectively.

### WriteChannel integration

`WriteChannel` does not change structurally. Its write worker currently takes `MemoryDb` (the SQLite implementation). After introducing the trait, the worker is parameterized over `Arc<dyn StorageBackend>`:

```rust
pub struct WriteChannel {
    tx: mpsc::Sender<WriteOp>,
    batch_count: Arc<AtomicUsize>,
}

impl WriteChannel {
    pub fn new(backend: Arc<dyn StorageBackend>) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        let batch_count = Arc::new(AtomicUsize::new(0));
        let handle = tokio::spawn(write_worker(rx, backend, Arc::clone(&batch_count)));
        (Self { tx, batch_count }, handle)
    }
}
```

The batch commit logic in `write_worker` calls `backend.begin_transaction()` → execute ops → `backend.commit()`. Each backend implements transaction semantics appropriately (`sqlx::SqlitePool` acquires a connection and begins a transaction; `sqlx::PgPool` does the same via `pool.begin()`).

## 5. Schema Design Patterns

### ID convention

All application-layer IDs are UUIDv7 strings generated by `MemoryId::new()` in `nous-shared/src/ids.rs`:

```rust
impl MemoryId::new() -> Self {
    Self(uuid::Uuid::now_v7().to_string())
}
```

UUIDv7 is time-ordered (first 48 bits = millisecond timestamp), so rows inserted in sequence cluster together on disk without a separate `created_at` index in most cases. On Postgres the column type is `TEXT` (same as SQLite) to avoid UUID-type casting at the application boundary.

### Timestamp convention

All timestamps are ISO 8601 strings at millisecond resolution in UTC, produced by SQLite's `strftime('%Y-%m-%dT%H:%M:%fZ', 'now')` — e.g. `2026-04-29T14:05:22.741Z`. On Postgres, the equivalent default is `to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')`. Application code compares timestamps as strings (ISO 8601 lexicographic order is chronological).

Schedules currently use Unix epoch seconds (`INTEGER`) rather than ISO 8601 strings because the scheduler computes `next_run_at` arithmetic on integers. **Planned migration:** `schedules` and `schedule_runs` timestamp columns will migrate from `INTEGER` (Unix epoch) to `TEXT` (ISO-8601) to match the rest of the system. Target column type: `TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))`.

### Database file layout (SQLite)

SQLite storage is split across three files to isolate concerns and allow independent vacuuming:

| File | Purpose | Key tables |
|------|---------|------------|
| `memory.db` | Core relational data | `memories`, `memory_chunks`, `rooms`, `room_messages`, `schedules`, `schedule_runs`, `categories`, `tags`, `relationships`, `workspaces` |
| `memory-fts.db` | Full-text search indexes | `memories_fts` (FTS5), `room_messages_fts` (FTS5) |
| `memory-vec.db` | Vector embeddings | `memory_embeddings` (vec0) |

All three files use WAL mode. The write worker opens connections to all three and coordinates writes within the same batch transaction (using `ATTACH` or separate connections depending on transaction requirements). Read connections open the appropriate file based on query type.

### Core tables

**memory.db** (core relational):

| Table | Primary Key | Purpose |
|-------|-------------|---------|
| `memories` | `id TEXT` (UUIDv7) | Core knowledge store |
| `memory_chunks` | `id TEXT` (`{memory_id}:{chunk_index}`) | Text chunks for embedding |
| `rooms` | `id TEXT` (UUIDv7) | Chat rooms |
| `room_messages` | `id TEXT` (UUIDv7) | Messages within rooms |
| `room_participants` | `(room_id, agent_id)` | Room membership |
| `schedules` | `id TEXT` (UUIDv7) | Cron-style schedule definitions |
| `schedule_runs` | `id TEXT` (UUIDv7) | Execution history per schedule |
| `models` | `id INTEGER AUTOINCREMENT` | Embedding model registry |
| `categories` | `id INTEGER AUTOINCREMENT` | Hierarchical memory categories |
| `tags` / `memory_tags` | `id INTEGER` / composite | Tag many-to-many |
| `relationships` | `id INTEGER AUTOINCREMENT` | Memory-to-memory graph edges (related, supersedes, contradicts, depends_on) |
| `access_log` | `id INTEGER AUTOINCREMENT` | Read access audit trail |

> ⚠️ **Side-effect:** Creating a `supersedes` relationship automatically sets `target.valid_until = now()`, marking the superseded memory as expired.
| `workspaces` | `id INTEGER AUTOINCREMENT` | Workspace path registry |

**memory-fts.db** (FTS5 virtual tables):

| Table | Primary Key | Purpose |
|-------|-------------|---------|
| `memories_fts` | rowid | FTS5 virtual table (SQLite) / `tsvector` column (Postgres) |
| `room_messages_fts` | rowid | FTS5 virtual table for message search |

**memory-vec.db** (vector storage):

| Table | Primary Key | Purpose |
|-------|-------------|---------|
| `memory_embeddings` | `chunk_id TEXT` | vec0 (SQLite) / pgvector (Postgres) |

### Agent ID column naming convention

All tables that reference an agent use standardized column names: `agent_id` (the owner or actor performing the operation) and `parent_agent_id` (the agent's position in the hierarchy). This convention ensures consistent join patterns and avoids ambiguity between actor and subject in queries.

### Namespace vs workspace_id

`namespace` (string, from org-management) identifies the team/org boundary. `workspace_id` (integer FK to `workspaces` table) identifies a project directory. An agent operates within one namespace and may access multiple workspaces within that namespace.

### JSON metadata columns

`rooms.metadata`, `room_messages.metadata`, `schedules.action_payload`, and `schedules.desired_outcome` store arbitrary JSON as `TEXT`. The schema does not enforce structure; the application deserializes with `serde_json`. On Postgres these can be typed as `JSONB` for index support, but `TEXT` is acceptable for initial parity.

### Full-text search

**SQLite:** FTS5 virtual tables (`memories_fts`, `room_messages_fts`) are kept in sync via `AFTER INSERT / UPDATE / DELETE` triggers. Queries use `bm25()` for ranking with column weights (title ×10.0, content ×1.0, memory_type ×0.5 in `search_fts`). Tokens containing FTS5 operator characters (`-`, `:`, `.`, etc.) are quoted by `sanitize_fts_query()` in `search.rs`.

**Postgres:** Add a `search_vector tsvector` generated column on `memories` and `room_messages`. Populate with `to_tsvector('english', title || ' ' || content)`. Create a GIN index. Queries use `ts_rank` or `ts_rank_cd`. `pg_trgm` is an alternative for fuzzy matching but requires the extension and a different index type.

### Semantic vector search

**SQLite:** `memory_embeddings` is a `vec0` virtual table (`CREATE VIRTUAL TABLE memory_embeddings USING vec0(chunk_id TEXT PRIMARY KEY, embedding float[N])`). The `sqlite-vec` extension is loaded at connection open via `crates/nous-core/src/sqlite_vec.rs`. KNN queries use `WHERE embedding MATCH ? AND k = ?`.

**Postgres:** Replace `vec0` with `pgvector`. Column type `vector(N)` on `memory_chunks` (or a separate `memory_embeddings` table). Index with `CREATE INDEX … USING ivfflat (embedding vector_cosine_ops)`. KNN query: `ORDER BY embedding <=> $1 LIMIT $2`.

## 6. WriteChannel / ReadPool Pattern

### WriteChannel

`WriteChannel` (in `crates/nous-core/src/channel.rs`) serializes all mutation through a single async worker. Constants:

```rust
const CHANNEL_CAPACITY: usize = 256;  // mpsc backpressure limit
const BATCH_LIMIT: usize = 32;        // max ops committed per transaction
```

**Worker loop:**

1. Block on `rx.recv()` until the first `WriteOp` arrives.
2. Drain additional ops via non-blocking `rx.try_recv()` until the batch reaches `BATCH_LIMIT` or the channel is empty.
3. Spawn a `blocking` task, take the `MemoryDb` mutex, open `unchecked_transaction()`.
4. Execute each op. Send the result back on its `oneshot::Sender` immediately — the sender does not wait for the commit.
5. Call `tx.commit()`. If commit fails, all ops in the batch receive the error (the `oneshot` channels are dropped and callers see a `RecvError`).

**WriteOp variants** (full list from `channel.rs`):

| Domain | Variants |
|--------|---------|
| Memory | `Store`, `Update`, `Forget`, `Unarchive` |
| Relationships | `Relate`, `Unrelate` |
| Categories | `CategorySuggest`, `CategoryDelete`, `CategoryRename`, `CategoryUpdate` |
| Chunks/Embeddings | `StoreChunks`, `DeleteChunks` |
| Access | `LogAccess` |
| Rooms | `CreateRoom`, `PostMessage`, `DeleteRoom`, `ArchiveRoom`, `JoinRoom` |
| Schedules | `CreateSchedule`, `UpdateSchedule`, `DeleteSchedule`, `RecordRun`, `UpdateRun`, `ComputeNextRun`, `ForceNextRunAt` |

Each variant carries a `oneshot::Sender<Result<T>>` as the last field. The caller awaits this sender to obtain the write result.

### ReadPool

`ReadPool` holds `N` read-only connections behind a `Semaphore`:

```rust
pub struct ReadPool {
    connections: Arc<Mutex<Vec<Connection>>>,
    semaphore: Arc<Semaphore>,
}

impl ReadPool {
    pub fn new(path: &str, size: usize) -> Result<Self> {
        // Opens `size` connections; sets PRAGMA query_only = ON on each.
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        // Acquires semaphore permit → pops connection → spawn_blocking(f) →
        // returns connection → drops permit.
    }
}
```

The `PRAGMA query_only = ON` guard prevents accidental writes through a read connection. Read connections are opened in WAL mode and share the WAL file with the write connection, so they see committed data without blocking writers.

### Postgres mapping

> **Post-MVP.** PostgreSQL backend is deferred. SQLite is the MVP backend. This design is retained for future reference.

On Postgres, `WriteChannel` and `ReadPool` map to a single `sqlx::PgPool`:

| SQLite concept | Postgres equivalent |
|----------------|---------------------|
| `WriteChannel` (single writer) | `PgPool`; each batch acquires a connection via `pool.begin()` and runs `BEGIN … COMMIT` |
| `ReadPool` (N read-only conns) | Same `PgPool`; reads run outside a transaction or in `READ COMMITTED` |
| `spawn_blocking` wrapper | Native `async` calls via `sqlx` |
| `BATCH_LIMIT = 32` | Unchanged — still batches up to 32 ops per transaction |
| `PRAGMA query_only` | No equivalent needed — app code separates reads from writes at the trait level |

The `WriteChannel` worker on Postgres acquires a pool connection, calls `pool.begin()`, executes the batch, and commits. The `oneshot` result-return pattern is unchanged.

### Batch limit tuning rationale

The batch limit of 32 ops per transaction balances write throughput (fewer fsync calls) against latency (callers wait for the batch to fill or the channel to drain). Empirical testing showed 32 as the sweet spot for typical workloads of 1-100 concurrent writers.

## 7. Migration Strategy

### Current approach (SQLite)

Migrations are a `const MIGRATIONS: &[&str]` array in `crates/nous-core/src/db.rs`. At startup, `run_migrations` (in `nous-shared/src/sqlite.rs`) executes each statement inside a single `BEGIN … COMMIT`:

```rust
pub fn run_migrations(conn: &Connection, migrations: &[&str]) -> Result<()> {
    conn.execute_batch("BEGIN")?;
    for migration in migrations {
        conn.execute_batch(migration)?; // rolls back on first error
    }
    conn.execute_batch("COMMIT")?;
    Ok(())
}
```

The array currently has 34 statements (as of this writing). All statements use `CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`, and `CREATE TRIGGER IF NOT EXISTS` guards, so re-running the array on an existing database is idempotent. There is no migration version table; idempotency replaces version tracking.

Additional schema evolution that cannot use `IF NOT EXISTS` (e.g., `ALTER TABLE … ADD COLUMN`) is handled by post-migration helper functions that call `PRAGMA table_info(…)` to detect missing columns before executing the `ALTER`. Examples: `migrate_models_columns`, `migrate_categories_columns` in `db.rs`.

### Multi-backend approach

For Postgres, the same logical migration array is maintained, with backend-specific statements where SQL dialects diverge:

```rust
pub enum MigrationStatement {
    Both(&'static str),           // identical SQL for both backends
    SqliteOnly(&'static str),     // e.g., FTS5 virtual table, vec0 table
    PostgresOnly(&'static str),   // e.g., tsvector column, pgvector extension
}

const MIGRATIONS: &[MigrationStatement] = &[
    Both("CREATE TABLE IF NOT EXISTS models ( … )"),
    Both("CREATE TABLE IF NOT EXISTS memories ( … )"),
    SqliteOnly("CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5( … )"),
    PostgresOnly("ALTER TABLE memories ADD COLUMN IF NOT EXISTS search_vector tsvector"),
    PostgresOnly("CREATE INDEX IF NOT EXISTS idx_memories_fts ON memories USING GIN(search_vector)"),
    // …
];
```

The migration runner receives the backend type and filters accordingly.

### Numbering and rollback

- Migrations are positional (array index). Adding a new migration appends to the array; existing indexes never change.
- Rollback is not supported at the migration level. Forward-only schema changes are required. Destructive changes (column drops, table renames) must be phased: add the new column, migrate data, then drop the old column in a later release.
- The `IF NOT EXISTS` / `IF NOT EXISTS` guard pattern means the migration runner is safe to call on every startup — no separate "has this run?" check is needed on SQLite. On Postgres, `CREATE TABLE IF NOT EXISTS` and `ADD COLUMN IF NOT EXISTS` (Postgres 9.6+) provide the same guarantee.

### Migration tooling options

| Option | Trade-off |
|--------|-----------|
| Embedded in binary (current) | Zero external deps; startup applies schema; safe for single-tenant. Hard to inspect without running the binary. |
| `sqlx migrate` | First-class Postgres support, version table, rollback scripts. Requires separate migration files and a build-time DB for `sqlx prepare`. |
| `refinery` | Embedded migrations with a version table. Supports both SQLite and Postgres via sqlx. Low external dep surface. Good candidate for multi-backend. |

The current embedded approach works for SQLite. For Postgres multi-backend, `sqlx migrate` is natural since we already use sqlx — it provides a version table and offline prepare support.

## 8. Connection Pooling

### SQLite configuration

Every connection (both write and read) is opened with the following pragmas (defined in `nous-shared/src/sqlite.rs`):

```sql
PRAGMA journal_mode = WAL;           -- write-ahead log; enables concurrent readers
PRAGMA wal_autocheckpoint = 1000;    -- checkpoint after 1000 pages
PRAGMA synchronous = NORMAL;         -- fsync on WAL checkpoint, not every write
PRAGMA busy_timeout = 5000;          -- wait up to 5 s before returning SQLITE_BUSY
PRAGMA cache_size = -64000;          -- 64 MB page cache per connection
PRAGMA foreign_keys = ON;
```

Read connections additionally set:

```sql
PRAGMA query_only = ON;              -- prevents accidental writes
```

Pool sizes:

| Connection | Count | Notes |
|------------|-------|-------|
| Write connection | 1 per DB file | Owned by the write worker (3 connections total: memory.db, memory-fts.db, memory-vec.db) |
| Read pool | 4 (default) per DB file | `ReadPool::new(path, 4)` |

Total: 15 open file handles across the three SQLite databases. WAL mode allows all 4 readers to run concurrently with the writer on each file.

### Postgres configuration (planned)

> **Post-MVP.** PostgreSQL backend is deferred. SQLite is the MVP backend. This configuration is retained for future reference.

```toml
[database.postgres]
url = "postgres://nous:password@localhost:5432/nous"
pool_min = 2
pool_max = 10
connect_timeout_secs = 5
idle_timeout_secs = 300
```

`sqlx::PgPool` is used directly — no additional pool crate is needed since sqlx includes connection pooling.

The write worker acquires a connection from the pool, starts a transaction, runs the batch, and releases the connection back to the pool on commit or rollback. Read calls each acquire a separate connection for the duration of the query.

Pool sizing: `pool_max = 10` (configurable via `[database.postgres]` config). No auto-tuning — the default covers most k8s deployments with 2–4 core pods.

## 9. Transaction Semantics

### Batch atomicity

Every batch of up to 32 `WriteOp`s commits as a single transaction. If any op in the batch fails (e.g., a unique constraint violation on `relationships`), the entire batch rolls back.

**Consequence for callers:** an operation that succeeds from the caller's perspective (its `oneshot` channel has been written to) may still not reach disk if the commit fails after the results are dispatched. The current SQLite implementation dispatches results *before* calling `tx.commit()` — strictly, results sent on the oneshot channels are provisional until the commit completes. In practice this is safe because:

- Write ops succeed or fail individually (sqlx returns errors per-statement before commit).
- The commit is a WAL flush, not a re-validation. If individual ops succeeded in the transaction, the commit succeeds unless there is an I/O error.

For Postgres, the same pattern applies: results are dispatched after each statement executes, and `tx.commit().await` finalises all. A failing `pgvector` insert (e.g., dimension mismatch) returns an error before the commit, which rolls back the batch.

### Isolation levels

| Backend | Write batch isolation | Read isolation |
|---------|----------------------|----------------|
| SQLite | `DEFERRED` transaction (default); serialized by the single writer | Snapshot isolation via WAL; readers see the last committed state |
| Postgres (planned, post-MVP) | `BEGIN … COMMIT` with default `READ COMMITTED` | `READ COMMITTED` per query |

> **Post-MVP.** PostgreSQL isolation levels apply only when the Postgres backend ships. SQLite is the MVP backend.

Postgres `SERIALIZABLE` is not required for the write batch because all writes are already serialized through the single `WriteChannel` worker. Using `READ COMMITTED` avoids the overhead of predicate locking.

### Error handling in batches

The `write_worker` in `channel.rs` calls `spawn_blocking` on the entire batch. If `tx.commit()` returns an error, the `spawn_blocking` future resolves to `Err`, and the worker logs the error but continues processing the next batch. Callers that sent ops in the failed batch see their `oneshot` receiver dropped (the `Sender` is dropped when the batch `Vec` goes out of scope on rollback), which surfaces as `NousError::Internal("response channel dropped")`.

This means **at most one batch** is lost per commit failure. Callers that need durability guarantees should await the write result and retry on error. The `WriteChannel` helper methods (`store`, `update`, etc.) propagate the error to callers rather than silently discarding it.

## 10. Dependencies Between Documents

| Document | Relationship |
|----------|-------------|
| `01-system-architecture.md` | Defines deployment modes (single-node vs. multi-node). The deployment mode determines which `StorageBackend` implementation is instantiated at startup. The data layer is the storage substrate for all services described in the system architecture. |
| `03-api-interfaces.md` | MCP tools and CLI commands are the primary callers of `WriteChannel` and `ReadPool`. The API layer owns the request boundary; the data layer owns the storage boundary. Changes to the `StorageBackend` trait surface require corresponding updates to the API layer if method signatures change. |

## 11. Open Questions

| Question | Resolution |
|----------|-----------|
| **Backend selection mechanism** | `[database] backend = "sqlite" \| "postgres"` in config. Not blocking for SQLite-only MVP work; needed before Postgres backend ships. |
| **Migration tooling for Postgres** | `sqlx migrate` with external `.sql` files, since sqlx is the unified backend. Decision needed before first Postgres deploy. |
| **Postgres FTS approach** | **Resolved:** `tsvector` (built-in PostgreSQL full-text search) with GIN index. No extension required. |
| **Vector search extension** | **Resolved:** `pgvector`. Widely deployed, supported by AWS RDS and Cloud SQL. |
| **Connection pool sizing** | **Resolved:** default `pool_max = 10`, configurable via `[database.postgres]` config. No auto-tuning. |
| **Write-result dispatch timing** | Low risk today (commit rarely fails); strict guarantee requires moving `resp.send()` calls after commit. Deferred — no change for MVP. |
