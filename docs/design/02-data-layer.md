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
9. [SQLCipher Encryption](#9-sqlcipher-encryption)
10. [Transaction Semantics](#10-transaction-semantics)
11. [Dependencies Between Documents](#11-dependencies-between-documents)
12. [Open Questions](#12-open-questions)

---

## 1. Goals

The data layer supports two deployment targets from a single codebase:

| Target | Backend | Deployment |
|--------|---------|------------|
| Single-node (developer machine, brew service) | SQLite + SQLCipher | One process, one file |
| Multi-node (k8s, Docker Compose, managed cloud) | PostgreSQL | N stateless app replicas, shared DB |

Concrete goals:

- **Unified read/write API** — application code calls `WriteChannel` and `ReadPool` without knowing which backend is active. No SQL leaks above the data layer boundary.
- **Feature parity** — full-text search, semantic vector search, room messaging, and schedules work on both backends. The mechanism differs (FTS5 vs `tsvector`; sqlite-vec vs `pgvector`), but the query interface is identical.
- **Atomic migrations** — schema is applied once at startup, inside a transaction, from a versioned array embedded in the binary. No external migration tool is required.
- **Encryption at rest on SQLite** — SQLCipher key resolved from `NOUS_DB_KEY` env var or `~/.config/nous/db.key` file, generated on first run if absent.
- **Transparent concurrency** — single-writer / multi-reader on SQLite (WAL + `ReadPool`); connection-pool-based concurrency on Postgres with serializable transactions for write batches.

## 2. Non-Goals

- **ORM / query builder** — SQL is written by hand. The abstraction boundary is a Rust trait, not a DSL. No Diesel, SeaORM, or SQLx query macros.
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
   │  rusqlite   │          │tokio-postgres│
   │ + SQLCipher │          │  (planned)  │
   │  + sqlite-  │          │  pgvector + │
   │  vec (vec0) │          │  tsvector   │
   │  + FTS5     │          │             │
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

### Library choices and trade-offs

| Library | Async | Compile-time SQL checks | Notes |
|---------|-------|------------------------|-------|
| `rusqlite` (current) | No — requires `spawn_blocking` | No | Bundled SQLCipher; sqlite-vec extension; FTS5. Stays for SQLite backend. |
| `tokio-postgres` | Yes — native async | No | Low-level; close to raw SQL; easy to co-locate with the trait implementation. **Recommended for Postgres backend.** |
| `sqlx` | Yes | Yes (macro) | Compile-time checks require a live DB at build time or offline snapshots. Attractive long-term but adds build complexity. |
| `diesel` | No (sync) | Yes (schema.rs) | Schema-first codegen; excellent for pure Postgres shops. Sync model is awkward in an async runtime. Not recommended. |

**Recommendation:** keep `rusqlite` for the SQLite backend unchanged. Add `tokio-postgres` for the Postgres backend behind the `StorageBackend` trait. Migrate to `sqlx` only if compile-time SQL checks become a priority and the build infrastructure can supply a live database.

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

The batch commit logic in `write_worker` calls `backend.begin_transaction()` → execute ops → `backend.commit()`. Each backend implements transaction semantics appropriately (rusqlite `unchecked_transaction`; tokio-postgres `client.transaction()`).

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

Schedules use Unix epoch seconds (`INTEGER`) rather than ISO 8601 strings because the scheduler computes `next_run_at` arithmetic on integers.

### Core tables

| Table | Primary Key | Purpose |
|-------|-------------|---------|
| `memories` | `id TEXT` (UUIDv7) | Core knowledge store |
| `memory_chunks` | `id TEXT` (`{memory_id}:{chunk_index}`) | Text chunks for embedding |
| `memory_embeddings` | `chunk_id TEXT` | vec0 (SQLite) / pgvector (Postgres) |
| `memories_fts` | rowid | FTS5 virtual table (SQLite) / `tsvector` column (Postgres) |
| `rooms` | `id TEXT` (UUIDv7) | Chat rooms |
| `room_messages` | `id TEXT` (UUIDv7) | Messages within rooms |
| `room_messages_fts` | rowid | FTS5 virtual table for message search |
| `room_participants` | `(room_id, agent_id)` | Room membership |
| `schedules` | `id TEXT` (UUIDv7) | Cron-style schedule definitions |
| `schedule_runs` | `id TEXT` (UUIDv7) | Execution history per schedule |
| `models` | `id INTEGER AUTOINCREMENT` | Embedding model registry |
| `categories` | `id INTEGER AUTOINCREMENT` | Hierarchical memory categories |
| `tags` / `memory_tags` | `id INTEGER` / composite | Tag many-to-many |
| `relationships` | `id INTEGER AUTOINCREMENT` | Memory-to-memory graph edges |
| `access_log` | `id INTEGER AUTOINCREMENT` | Read access audit trail |
| `workspaces` | `id INTEGER AUTOINCREMENT` | Workspace path registry |

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
    pub fn new(path: &str, key: Option<&str>, size: usize) -> Result<Self> {
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

On Postgres, `WriteChannel` and `ReadPool` map to a single connection pool (e.g., `deadpool-postgres` or `bb8-postgres`):

| SQLite concept | Postgres equivalent |
|----------------|---------------------|
| `WriteChannel` (single writer) | Connection pool; each batch acquires a connection and runs `BEGIN … COMMIT` |
| `ReadPool` (N read-only conns) | Same connection pool; reads run outside a transaction or in `READ COMMITTED` |
| `spawn_blocking` wrapper | Native `async` calls via `tokio-postgres` |
| `BATCH_LIMIT = 32` | Unchanged — still batches up to 32 ops per transaction |
| `PRAGMA query_only` | No equivalent needed — app code separates reads from writes at the trait level |

The `WriteChannel` worker on Postgres acquires a pool connection, calls `client.transaction()`, executes the batch, and commits. The `oneshot` result-return pattern is unchanged.

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
| `refinery` | Embedded migrations with a version table. Supports both SQLite (via rusqlite) and Postgres (via tokio-postgres). Low external dep surface. Good candidate for multi-backend. |

The current embedded approach works for SQLite. For Postgres multi-backend, `refinery` is the lowest-friction path to adding a migration version table while keeping migrations embedded in the binary.

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
| Write connection | 1 | Owned by the write worker behind `Arc<Mutex<MemoryDb>>` |
| Read pool | 4 (default) | `ReadPool::new(path, key, 4)` |

Total: 5 open file handles to the same SQLite database. WAL mode allows all 4 readers to run concurrently with the writer.

### Postgres configuration (planned)

```toml
[database.postgres]
url = "postgres://nous:password@localhost:5432/nous"
pool_min = 2
pool_max = 10
connect_timeout_secs = 5
idle_timeout_secs = 300
```

Candidate pool crates:

| Crate | Notes |
|-------|-------|
| `deadpool-postgres` | Zero-cost async pool for `tokio-postgres`; widely used; configurable via `deadpool` config types |
| `bb8` + `bb8-postgres` | Alternative; similar API |

The write worker acquires a connection from the pool, starts a transaction, runs the batch, and releases the connection back to the pool on commit or rollback. Read calls each acquire a separate connection for the duration of the query.

Pool sizing heuristic: `pool_max = 2 × CPU cores + 1` for OLTP workloads is a common starting point. The default of 10 covers most k8s deployments with 2–4 core pods.

## 9. SQLCipher Encryption

### SQLite: SQLCipher

The workspace depends on `rusqlite = { version = "0.39", features = ["bundled-sqlcipher"] }`. SQLCipher is compiled in; no separate shared library is required.

**Key resolution** (implemented in `nous-shared/src/sqlite.rs`):

1. Check `NOUS_DB_KEY` environment variable (non-empty string wins).
2. Read `~/.config/nous/db.key` (trimmed, non-empty string wins).
3. Generate a 32-byte random hex key, write it to `db.key` with `chmod 0600`, and use it.

The key is applied immediately after `Connection::open()` via `PRAGMA key = '…'`. All subsequent operations on that connection are transparently encrypted.

**Plaintext migration:** If an existing database file has the SQLite plaintext header (`SQLite format 3\0`), `open_connection` calls `migrate_plaintext_to_encrypted`. It copies the database, uses `sqlcipher_export` to re-encrypt, and atomically renames the encrypted file back into place. The vec0 virtual table is dropped before export (SQLCipher cannot export it; `MemoryDb::open` recreates it).

**Key rotation:** `rotate_key(db_path, current_key, new_key)` in `sqlite.rs` uses `PRAGMA rekey` followed by an `integrity_check` verification. On success, it rewrites `db.key`.

### Postgres: encryption options

Postgres does not offer transparent database-file encryption at the SQLite/SQLCipher level. The available options are:

| Option | Scope | Trade-off |
|--------|-------|-----------|
| TLS in transit (`sslmode=require`) | Connection | Mandatory for all deployments. Configured in the connection URL. |
| OS/filesystem encryption (LUKS, dm-crypt, EBS encryption) | Full disk | Transparent to Postgres; managed by infrastructure. Recommended for k8s PVC. |
| `pgcrypto` column encryption | Selected columns | Application encrypts before insert; performance cost per column; complicates FTS and vector search on encrypted fields. |

For the planned Postgres backend, TLS in transit plus disk-level encryption (managed by the cloud provider or k8s storage class) covers the threat model. Column encryption with `pgcrypto` is not planned.

## 10. Transaction Semantics

### Batch atomicity

Every batch of up to 32 `WriteOp`s commits as a single transaction. If any op in the batch fails (e.g., a unique constraint violation on `relationships`), the entire batch rolls back.

**Consequence for callers:** an operation that succeeds from the caller's perspective (its `oneshot` channel has been written to) may still not reach disk if the commit fails after the results are dispatched. The current SQLite implementation dispatches results *before* calling `tx.commit()` — strictly, results sent on the oneshot channels are provisional until the commit completes. In practice this is safe because:

- Write ops succeed or fail individually (rusqlite returns errors per-statement before commit).
- The commit is a WAL flush, not a re-validation. If individual ops succeeded in the transaction, the commit succeeds unless there is an I/O error.

For Postgres, the same pattern applies: results are dispatched after each statement executes, and `client.transaction().commit().await` finalises all. A failing `pgvector` insert (e.g., dimension mismatch) returns an error before the commit, which rolls back the batch.

### Isolation levels

| Backend | Write batch isolation | Read isolation |
|---------|----------------------|----------------|
| SQLite | `DEFERRED` transaction (default); serialized by the single writer | Snapshot isolation via WAL; readers see the last committed state |
| Postgres (planned) | `BEGIN … COMMIT` with default `READ COMMITTED` | `READ COMMITTED` per query |

Postgres `SERIALIZABLE` is not required for the write batch because all writes are already serialized through the single `WriteChannel` worker. Using `READ COMMITTED` avoids the overhead of predicate locking.

### Error handling in batches

The `write_worker` in `channel.rs` calls `spawn_blocking` on the entire batch. If `tx.commit()` returns an error, the `spawn_blocking` future resolves to `Err`, and the worker logs the error but continues processing the next batch. Callers that sent ops in the failed batch see their `oneshot` receiver dropped (the `Sender` is dropped when the batch `Vec` goes out of scope on rollback), which surfaces as `NousError::Internal("response channel dropped")`.

This means **at most one batch** is lost per commit failure. Callers that need durability guarantees should await the write result and retry on error. The `WriteChannel` helper methods (`store`, `update`, etc.) propagate the error to callers rather than silently discarding it.

## 11. Dependencies Between Documents

| Document | Relationship |
|----------|-------------|
| `01-system-architecture.md` | Defines deployment modes (single-node vs. multi-node). The deployment mode determines which `StorageBackend` implementation is instantiated at startup. The data layer is the storage substrate for all services described in the system architecture. |
| `03-api-interfaces.md` | MCP tools and CLI commands are the primary callers of `WriteChannel` and `ReadPool`. The API layer owns the request boundary; the data layer owns the storage boundary. Changes to the `StorageBackend` trait surface require corresponding updates to the API layer if method signatures change. |

## 12. Open Questions

| Question | Options | Blocking |
|----------|---------|---------|
| **Backend selection mechanism** | (a) `[database] backend = "sqlite" \| "postgres"` in config; (b) auto-detect from `DATABASE_URL` env var (postgres scheme → Postgres, else SQLite); (c) compile-time feature flag | Not blocking for SQLite-only work; needed before Postgres backend ships |
| **Migration tooling for Postgres** | (a) Keep embedded array with `IF NOT EXISTS` guards (no version table); (b) add `refinery` for a version table and rollback awareness; (c) `sqlx migrate` with external `.sql` files | Decision needed before first Postgres deploy to avoid manual schema repairs |
| **Postgres FTS approach** | (a) `tsvector` generated column + GIN index (built-in, no extension); (b) `pg_trgm` extension for fuzzy matching; (c) both | `tsvector` is the default recommendation; `pg_trgm` adds fuzzy prefix matching. Decide based on search quality requirements. |
| **Vector search extension** | (a) `pgvector` (most widely deployed, AWS RDS / Cloud SQL support); (b) `pg_embedding` (Neon); (c) `pgvecto.rs` (Rust, faster indexing) | `pgvector` is the safe default; revisit if IVFFlat recall or indexing speed is insufficient |
| **Connection pool sizing heuristics** | (a) Fixed default (e.g., `pool_max = 10`); (b) runtime-configurable; (c) auto-tuned from `max_connections` Postgres setting | Configurable in `[database.postgres]` config is sufficient; auto-tuning is a nice-to-have |
| **Write-result dispatch timing** | Results are currently dispatched before `tx.commit()`. Should they wait for commit confirmation? | Low risk today (commit rarely fails); strict guarantee requires moving `resp.send()` calls after commit, which complicates the batch loop |
