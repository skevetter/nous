# Vector Memory Pipeline

## 1. Overview

`search_similar` (`crates/nous-core/src/memory/mod.rs:757`) loaded every row with a non-NULL `embedding` BLOB from the `memories` table and computed cosine similarity in Rust. On a database with N memories the query touches all N rows — O(N) I/O plus O(N) CPU before the first result is returned.

sqlite-vec replaces this with a vec0 virtual table that stores embeddings in a dedicated file (`memory-vec.db`) and answers K-nearest-neighbour queries with a single indexed lookup. Response time becomes O(log N + K) instead of O(N), and the main FTS database is never touched during similarity search.

Implementation follows the approach specified in `docs/design/06-features-p5-p7.md:483`.

## 2. Architecture

The platform maintains two SQLite databases. Their roles and connection types differ:

| Database | File | Connection | Purpose |
|----------|------|------------|---------|
| FTS | `memory-fts.db` | `SqlitePool` (SQLx, async, up to 5 connections) | All relational data: memories, rooms, messages, schedules, sessions |
| Vec | `memory-vec.db` | `Arc<Mutex<Connection>>` (rusqlite, single connection) | vec0 virtual table for KNN embedding search |

`DbPools` (`crates/nous-core/src/db/pool.rs:413`) holds both:

```rust
pub struct DbPools {
    pub fts: SqlitePool,
    pub vec: VecPool,          // type alias: Arc<Mutex<Connection>>
}
```

SQLx cannot load native extensions at runtime, so vec0 requires rusqlite. `create_vec_pool` (`pool.rs:443`) opens the connection and calls `sqlite_vec::sqlite3_vec_init(conn.handle())` to register the extension before any queries run. The `sqlite-vec = "0.1.9"` and `rusqlite = { version = "0.32", features = ["bundled"] }` crate dependencies are declared at workspace level (`Cargo.toml:19-20`), giving a fully static distribution with no system SQLite dependency.

The FTS pool runs its own migration runner (`run_migrations_on_pool`, `pool.rs:527`). The vec pool runs a separate runner (`run_vec_migrations`, `pool.rs:481`). Both are called from `DbPools::run_migrations` (`pool.rs:432`).

## 3. Schema

The vec database holds two tables:

```sql
-- migration tracker (applied by run_vec_migrations before any vec migration)
CREATE TABLE IF NOT EXISTS vec_schema_version (
    id         INTEGER PRIMARY KEY,
    version    TEXT    NOT NULL,
    applied_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- vec0 virtual table for KNN search
CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(
    memory_id TEXT PRIMARY KEY,
    embedding float[384]
);
```

`memory_id` is the same UUID string used as the primary key in `memories.id` on the FTS side. The dimension constant 384 matches `EMBEDDING_DIMENSION` (`pool.rs:11`) and the all-MiniLM-L6-v2 output size. This is the only place the dimension is hardcoded; changing the model requires updating `EMBEDDING_DIMENSION` and running a fresh vec0 migration.

The FTS `memories` table retains its `embedding BLOB` column but it is no longer the authoritative store. See §10 for the deprecation approach.

## 4. Storage Flow

`store_embedding` (`memory/mod.rs:720`) performs a dual write: first to the FTS pool's legacy BLOB column, then to the vec pool's `memory_embeddings` table.

The FTS write updates the `memories.embedding` BLOB column for backwards compatibility:

```sql
UPDATE memories SET embedding = ? WHERE id = ?
```

The vec write acquires the `Arc<Mutex<Connection>>` lock, deletes any prior row for the same `memory_id`, and inserts the new embedding:

```sql
DELETE FROM memory_embeddings WHERE memory_id = ?1;
INSERT INTO memory_embeddings(memory_id, embedding) VALUES (?1, ?2);
```

The embedding bytes passed to both writes are the same float32 LE representation produced by `embedding_to_bytes` (`mod.rs:832`). The two writes are not wrapped in a distributed transaction. If the vec write fails, the BLOB in `memories` still holds the current embedding and remains the fallback (see §10).

## 5. Search Flow

`search_similar` (`memory/mod.rs:757`) issues a KNN query against the vec pool:

```sql
SELECT memory_id, distance
FROM memory_embeddings
WHERE embedding MATCH ?1
ORDER BY distance
LIMIT ?2
```

`?1` is the query embedding serialized as float32 LE bytes (same format as storage). `?2` is the limit (capped at 100). vec0 returns rows sorted by ascending L2 distance; the function maps `memory_id` values to full `Memory` records via a follow-up query on the FTS pool.

The workspace filter (`workspace_id = ?`) and threshold filter applied in the current O(N) scan move to the post-KNN join step: vec0 returns the K nearest IDs, then the FTS query applies workspace and archival filters. This means the effective result set may be smaller than K when filters remove candidates; callers should over-fetch (e.g. `k = limit * 3`) if exact-limit results are required.

## 6. Migration Strategy

Migrations are split across two independent runners:

| Runner | Applies to | Tracking table |
|--------|-----------|----------------|
| `run_migrations_on_pool` (`pool.rs:527`) | FTS pool (`SqlitePool`) | `schema_version` |
| `run_vec_migrations` (`pool.rs:481`) | Vec pool (`VecPool`) | `vec_schema_version` |

`DbPools::run_migrations` (`pool.rs:432`) calls both in sequence at startup. The `VEC_MIGRATIONS` slice (`pool.rs:472`) currently contains one entry:

```
vec_001  memory_embeddings_vec0  — CREATE VIRTUAL TABLE IF NOT EXISTS memory_embeddings USING vec0(…)
```

Vec migrations never run on the FTS pool. FTS migrations never run on the vec pool. Future schema changes to `memory_embeddings` (e.g. dimension change, adding a second vector column) are added to `VEC_MIGRATIONS` only.

The existing `MIGRATIONS` slice in `pool.rs` still includes all FTS migrations and is unchanged. No existing migration SQL was modified.

Existing `embedding` BLOBs in the `memories` table are not automatically migrated to `memory_embeddings` at startup. A one-time migration tool is needed to read each BLOB, insert it into vec0, and mark it migrated. Until that tool runs, memories whose embeddings exist only as BLOBs will not appear in KNN results (see §10 for the fallback behaviour and §11 for the deferred scope).

## 7. Embedding Model

The platform targets **all-MiniLM-L6-v2**, which produces 384-dimensional float32 vectors. The dimension is the only model-specific constant in the codebase (`EMBEDDING_DIMENSION = 384`, `pool.rs:11`); it appears once in the `CREATE VIRTUAL TABLE` statement.

Embedding generation is **client-side** in this workstream. Callers pass pre-computed `&[f32]` slices to `store_embedding` and `search_similar`; the platform stores and queries them but does not invoke an inference backend. Server-side embedding generation (loading the ONNX model in the daemon) is deferred to WS2.

Changing the model requires:
1. Updating `EMBEDDING_DIMENSION` in `pool.rs:11`.
2. Adding a new vec migration that drops and recreates `memory_embeddings` with the new dimension.
3. Re-embedding all existing memories (see §11 — batch migration deferred).

## 8. Chunking Strategy

Deferred to WS2. The current design stores one embedding per memory row. `memory_embeddings.memory_id` maps 1:1 to `memories.id`. There is no `memory_chunks` table and no sliding-window splitter in this workstream. Chunked retrieval (multiple embeddings per memory, retrieved by chunk and re-assembled) is out of scope here.

## 9. Reranking

Deferred to WS2. vec0 KNN returns results sorted by L2 distance, reducing query complexity from O(N) to O(log N + K) compared to the current full-table cosine scan. No cross-encoder reranking or reciprocal rank fusion with the FTS BM25 score is applied in this workstream. The hybrid search path described in `docs/design/06-features-p5-p7.md:271` (FTS5 + vec0 + RRF) is a WS2 scope item.

## 10. Backwards Compatibility

Three scenarios must work without errors after the migration:

| Scenario | Behaviour |
|----------|-----------|
| Memory with no embedding | `search_similar` returns no row for it (neither BLOB nor vec entry exists); all non-semantic queries are unaffected |
| Memory with BLOB but no vec entry | The vec KNN query silently omits the memory; the BLOB remains in place as a fallback for any caller still using the old codepath |
| Vec pool fails to open | `DbPools::connect` returns an error today; a future hardening pass can make it degrade to BLOB-only mode |

The `embedding BLOB` column on `memories` is **not dropped** in this workstream. Dropping it is a separate migration that should only run after all existing BLOBs have been migrated to vec0 and the old `search_similar` codepath is removed.

## 11. Open Decisions

| Decision | Current stance | Notes |
|----------|---------------|-------|
| Embedding dimensions | 384, set via `EMBEDDING_DIMENSION` (`pool.rs:11`) | Changing requires a vec migration drop/recreate and full re-embed |
| Static distribution | `rusqlite = { features = ["bundled"] }` + `sqlite-vec` crate | No system SQLite dependency; binary size increases ~2 MB |
| Migration separation | FTS and vec runners are independent (`pool.rs:432`) | Prevents SQLx from trying to manage vec0 DDL it cannot parse |
| Batch migration of existing BLOBs | Deferred | Existing `embedding` BLOBs are not copied to vec0 at startup; a one-time migration tool is needed before the BLOB column can be dropped |
| Vec pool concurrency | Single `Arc<Mutex<Connection>>` | Sufficient while writes are infrequent; revisit if contention appears under load |
