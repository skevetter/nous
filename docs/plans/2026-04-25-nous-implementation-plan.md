# Nous Implementation Plan

**Date**: 2026-04-25
**Spec**: `/home/skevetter/ws/h3/docs/superpowers/specs/2026-04-25-nous-design.md`
**Repo**: `~/ws/nous/`
**Approach**: TDD (red-green-refactor) for every task

## Known Issues

**sqlite-vec 0.1.10-alpha.3 build failure**: The only published version of `sqlite-vec` is missing `sqlite-vec-diskann.c` in the crate package. The dependency is declared in the workspace `Cargo.toml` but commented out in `nous-core/Cargo.toml`. All vector search tasks use `rusqlite` raw SQL for the `vec0` virtual table. When upstream publishes a fix, uncomment the dependency and replace raw SQL with the crate's helper API. Until then, load the sqlite-vec extension at runtime via `rusqlite::Connection::load_extension_enable()` + a vendored `.so`/`.dylib`, or compile `sqlite-vec.c` via a `build.rs` in `nous-core`. The plan marks vector-dependent tasks and provides a fallback path.

## Dependency Order

```
Phase 1: nous-shared  (foundation)
Phase 2: nous-core    (depends on nous-shared)
Phase 3: nous-mcp     (depends on nous-core, nous-shared)
Phase 4: nous-otlp    (depends on nous-shared)
Phase 5: Integration   (cross-crate, E2E)
```

---

## Phase 1: nous-shared

### Task 1.1: Error Types

**Files**: `crates/nous-shared/src/lib.rs`, `crates/nous-shared/src/error.rs`

**Implement**: Shared `NousError` enum via `thiserror`. Variants: `Sqlite(rusqlite::Error)`, `Io(std::io::Error)`, `Config(String)`, `Encryption(String)`, `Embedding(String)`. Re-export as `pub type Result<T> = std::result::Result<T, NousError>`.

**TDD sequence**:
1. **Red**: Write `crates/nous-shared/tests/error_tests.rs` — construct each variant, verify `Display` output contains expected substring, verify `From<rusqlite::Error>` conversion compiles
2. **Green**: Implement `NousError` with `#[derive(thiserror::Error)]`, add `From` impls
3. **Refactor**: Ensure all variants use `#[from]` where possible, remove manual `From` impls

**Acceptance**:
- `NousError` is `Send + Sync + 'static`
- Each variant round-trips through `Display` with a human-readable message
- `cargo test -p nous-shared` passes

**Complexity**: S

---

### Task 1.2: SQLite Helpers

**Files**: `crates/nous-shared/src/sqlite.rs`

**Implement**: `open_connection(path, key) -> Result<Connection>` that applies WAL pragmas from the spec (journal_mode, wal_autocheckpoint, synchronous, busy_timeout, cache_size, foreign_keys). `PRAGMA key` applied first if `key` is `Some`. `run_migrations(conn, migrations: &[&str]) -> Result<()>` executes SQL strings in a transaction. `spawn_blocking<F, T>(f: F) -> Result<T>` bridge for rusqlite calls on the tokio runtime.

**TDD sequence**:
1. **Red**: Test `open_connection` with in-memory DB (`":memory:"`), assert WAL mode is active via `PRAGMA journal_mode` query. Test `run_migrations` with a `CREATE TABLE` then `SELECT` to verify table exists. Test `spawn_blocking` returns value from closure
2. **Green**: Implement `open_connection` with pragma chain, `run_migrations` with `conn.execute_batch` inside `BEGIN/COMMIT`, `spawn_blocking` with `tokio::task::spawn_blocking`
3. **Refactor**: Extract pragma list to a const array, apply in a loop

**Acceptance**:
- In-memory DB opens with WAL mode confirmed
- Migrations apply atomically (partial failure rolls back)
- `spawn_blocking` propagates panics as errors

**Complexity**: M

---

### Task 1.3: XDG Path Resolution

**Files**: `crates/nous-shared/src/xdg.rs`

**Implement**: `cache_dir() -> PathBuf` resolves `$NOUS_CACHE_DIR` > `$XDG_CACHE_HOME/nous` > `~/.cache/nous`. `config_dir() -> PathBuf` resolves `$NOUS_CONFIG_DIR` > `$XDG_CONFIG_HOME/nous` > `~/.config/nous`. Both create the directory if absent. `db_path(name: &str) -> PathBuf` returns `cache_dir().join(name)`. `config_path(name: &str) -> PathBuf` returns `config_dir().join(name)`.

**TDD sequence**:
1. **Red**: In a temp dir, set `NOUS_CACHE_DIR` and `NOUS_CONFIG_DIR` env vars, call `cache_dir()` and `config_dir()`, assert paths match env vars. Unset env vars, set `XDG_CACHE_HOME` and `XDG_CONFIG_HOME`, assert fallback. Unset all, assert `~/.cache/nous` and `~/.config/nous` (use `$HOME` override in test)
2. **Green**: Implement with `std::env::var` chain, `std::fs::create_dir_all`
3. **Refactor**: Extract common resolution logic into a private helper `resolve_dir(app_var, xdg_var, default_subdir)`

**Acceptance**:
- Env var override takes precedence
- Directories are created on first call
- No panic on missing `$HOME` (return error)

**Complexity**: S

---

### Task 1.4: Typed ID Wrappers

**Files**: `crates/nous-shared/src/ids.rs`

**Implement**: Newtype wrappers `SessionId(String)`, `TraceId(String)`, `SpanId(String)`, `MemoryId(String)`. Each implements `Display`, `FromStr`, `Serialize`, `Deserialize`, `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`. `MemoryId::new()` generates a UUID v7 (time-ordered) as the string value. Add `uuid` dependency to `nous-shared/Cargo.toml` with `v7` feature.

**TDD sequence**:
1. **Red**: Construct `MemoryId::new()`, assert it parses as valid UUID. Round-trip `SessionId` through `Display` + `FromStr`. Serialize/deserialize `TraceId` through `serde_json`. Assert two `MemoryId::new()` calls produce different values. Assert `MemoryId::new()` values are lexicographically ordered (UUID v7 property)
2. **Green**: Implement newtype structs with derive macros, `MemoryId::new()` uses `uuid::Uuid::now_v7().to_string()`
3. **Refactor**: Use a macro to reduce boilerplate across the four ID types

**Acceptance**:
- `MemoryId::new()` produces time-ordered UUIDs
- All four types are `serde` round-trippable
- Types are distinct (cannot accidentally assign `SessionId` to `TraceId`)

**Complexity**: S

**Note — Spec Deviations**: `error.rs` and `MemoryId` are placed in `nous-shared` rather than in `nous-core` as implied by the spec layout. This is intentional: `nous-otlp` depends on `nous-shared` but not `nous-core`, and needs access to `NousError` and `MemoryId`. Placing them in the shared crate avoids a circular or heavyweight dependency.

---

## Phase 2: nous-core

### 2A: Schema, Types, and Migrations

#### Task 2A.1: Core Types

**Files**: `crates/nous-core/src/types.rs`

**Implement**: Structs matching the spec schema: `Memory`, `Tag`, `MemoryTag`, `Relationship`, `Workspace`, `Category`, `AccessLogEntry`, `Model`, `MemoryChunk`. Each struct derives `Debug, Clone, Serialize, Deserialize`. Enums: `MemoryType` (decision, convention, bugfix, architecture, fact, observation), `Importance` (low, moderate, high), `Confidence` (low, moderate, high), `RelationType` (related, supersedes, contradicts, depends_on), `CategorySource` (system, user, agent). All enums implement `Display`, `FromStr`, serialize as lowercase strings.

**TDD sequence**:
1. **Red**: Construct each struct with field values, serialize to JSON, deserialize back, assert equality. Parse each enum variant from its lowercase string. Assert `FromStr` fails on invalid input
2. **Green**: Define structs and enums with serde attributes (`#[serde(rename_all = "snake_case")]`)
3. **Refactor**: Add `impl TryFrom<&str>` for enums if `FromStr` is insufficient for SQLite text columns

**Acceptance**:
- All structs round-trip through `serde_json`
- Enum variants match the spec field reference (type: decision, convention, bugfix, architecture, fact, observation)
- Types are importable from `nous_core::types`

**Complexity**: M

---

#### Task 2A.2: Schema and Migrations

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `const MIGRATIONS: &[&str]` array containing the full schema SQL from the spec (models, workspaces, categories, memories, tags, memory_tags, relationships, access_log, memories_fts, memory_chunks). **Exclude** `memory_vecs` (vec0 virtual table) until sqlite-vec is resolved. `MemoryDb::open(path, key) -> Result<MemoryDb>` uses `nous_shared::sqlite::open_connection` + `run_migrations`. `MemoryDb` wraps `Connection` and provides typed query methods.

**TDD sequence**:
1. **Red**: Open in-memory DB, assert all tables exist via `SELECT name FROM sqlite_master WHERE type='table'`. Assert FTS5 table `memories_fts` exists. Assert triggers exist (memories_ai, memories_ad, memories_au, tags_cleanup). Assert indexes exist
2. **Green**: Write migration SQL strings, call `run_migrations` in `MemoryDb::open`
3. **Refactor**: Split migrations into numbered versions for future incremental migration support

**Acceptance**:
- All 10+ tables/virtual tables from spec are created
- FTS5 triggers fire on insert/update/delete (verified in Task 2B)
- `MemoryDb::open(":memory:", None)` works for all tests

**Complexity**: M

---

#### Task 2A.3: Seed Categories

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `seed_categories(conn)` inserts the 15 top-level categories and their children from the spec's starter hierarchy. Runs as part of migration (idempotent — `INSERT OR IGNORE`). Each category has `source = 'system'`.

**TDD sequence**:
1. **Red**: Open DB, query `SELECT COUNT(*) FROM categories WHERE source = 'system'`, assert >= 15 top-level. Query children of `infrastructure`, assert `k8s, networking, storage, compute` exist. Assert parent_id links are correct
2. **Green**: Generate INSERT statements for the hierarchy. Use two passes: top-level first, then children with subquery for parent_id
3. **Refactor**: Build hierarchy from a declarative Rust data structure (array of tuples) rather than raw SQL strings

**Acceptance**:
- 15 top-level categories seeded
- All subcategories linked to correct parents
- Re-running migration does not duplicate categories

**Complexity**: S

### 2B: CRUD Operations

#### Task 2B.1: Store Memory

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::store(memory: &NewMemory) -> Result<MemoryId>`. `NewMemory` is the input struct (no `id`, `created_at`, `updated_at`). Generates `MemoryId::new()`, inserts into `memories` table. Handles tags: find-or-create in `tags` table, insert into `memory_tags`. Handles workspace: find-or-create by path hash in `workspaces` table. Returns the generated ID. All within a single transaction.

**TDD sequence**:
1. **Red**: Store a memory with title/content/type, recall by ID, assert all fields match. Store a memory with ALL optional fields populated (importance, confidence, session_id, trace_id, agent_id, agent_model, valid_from), recall by ID, assert each optional field matches. Store with tags `["rust", "testing"]`, query `memory_tags`, assert 2 rows. Store two memories with same tag, assert tag table has 1 row for that tag. Assert FTS5 trigger fires: `SELECT * FROM memories_fts WHERE memories_fts MATCH 'title_word'` returns the stored memory
2. **Green**: Implement `store` with `INSERT INTO memories`, tag upsert loop, workspace upsert
3. **Refactor**: Extract tag upsert into `ensure_tags(conn, tags) -> Result<Vec<i64>>`

**Acceptance**:
- Stored memory gets a valid UUID v7 ID
- Tags are deduplicated across memories
- FTS5 index is populated via trigger
- Hash is SHA-256 of the absolute path, truncated to first 32 hex characters
- If workspace path doesn't match any hash, check aliases arrays before creating a new workspace

**Complexity**: M

---

#### Task 2B.2: Recall Memory

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::recall(id: &MemoryId) -> Result<Option<MemoryWithRelations>>`. Returns the memory with its tags (as `Vec<String>`), relationships (as `Vec<Relationship>`), category (as `Option<Category>`), and access count. Logs an access entry in `access_log`.

**TDD sequence**:
1. **Red**: Store a memory, recall it, assert all fields match. Recall a non-existent ID, assert `None`. Recall twice, query access_log, assert 2 entries for that memory
2. **Green**: Implement with a JOIN query across memories, categories, plus separate queries for tags and relationships. Insert into `access_log`
3. **Refactor**: Combine tag/relationship queries into the main query using GROUP_CONCAT or handle in Rust

**Acceptance**:
- Returns `None` for missing IDs (no error)
- Access log entry created on each recall
- Tags and relationships are fully populated

**Complexity**: M

---

#### Task 2B.3: Update Memory

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::update(id: &MemoryId, patch: &MemoryPatch) -> Result<bool>`. `MemoryPatch` has `Option` fields for title, content, tags, importance, confidence, valid_until. Only non-`None` fields are SET. Updates `updated_at`. Returns `false` if ID not found. If content changes, the caller is responsible for re-embedding (signaled by return type or a flag).

**TDD sequence**:
1. **Red**: Store a memory, update title only, recall, assert title changed and content unchanged. Update tags from `["a"]` to `["b","c"]`, assert old tag removed, new tags added. Update non-existent ID, assert returns `false`. Assert `updated_at` changes on update
2. **Green**: Build dynamic `UPDATE` SQL based on which `MemoryPatch` fields are `Some`. Handle tag replacement in transaction
3. **Refactor**: Use a query builder helper or parameter binding list to avoid SQL injection risk in dynamic queries

**Acceptance**:
- Partial updates leave untouched fields intact
- Tag replacement is atomic (old removed, new added in one transaction)
- `updated_at` is always refreshed

**Complexity**: M

---

#### Task 2B.4: Forget (Archive/Delete)

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::forget(id: &MemoryId, hard: bool) -> Result<bool>`. Soft delete: set `archived = 1`, delete from `memory_chunks` and `memory_vecs` (saves space, keeps text). Hard delete: `DELETE FROM memories` (cascades to tags, relationships, chunks, access_log via FK). Returns `false` if ID not found.

**TDD sequence**:
1. **Red**: Store, soft-forget, assert `archived = 1`. Recall soft-forgotten memory, assert it is returned with `archived = true`. Store, hard-forget, assert SELECT returns no row. Assert tags_cleanup trigger fires after hard delete (orphan tags removed). Assert chunks deleted on soft forget
2. **Green**: Implement with conditional `UPDATE` vs `DELETE`
3. **Refactor**: Add `MemoryDb::forget_batch(ids, hard)` if N > 1 is common

**Acceptance**:
- Soft delete preserves text, removes vectors
- Hard delete cascades to all related tables
- Orphan tags are cleaned up

**Complexity**: S

---

#### Task 2B.5: Unarchive

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::unarchive(id: &MemoryId) -> Result<bool>`. Sets `archived = 0`, `updated_at = now`. Returns `false` if not found or not archived. The caller must re-embed after unarchive (vectors were deleted on archive).

**TDD sequence**:
1. **Red**: Store, archive, unarchive, assert `archived = 0`. Unarchive a non-archived memory, assert `false`. Unarchive non-existent ID, assert `false`
2. **Green**: `UPDATE memories SET archived = 0, updated_at = datetime('now') WHERE id = ? AND archived = 1`
3. **Refactor**: Return a `UnarchiveResult` enum if the caller needs to distinguish not-found vs not-archived

**Acceptance**:
- Unarchived memory has `archived = 0`
- Returns false for non-archived or missing memories
- `updated_at` is refreshed

**Complexity**: S

---

#### Task 2B.6: Relationships

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::relate(from: &MemoryId, to: &MemoryId, relation: RelationType) -> Result<()>`. Inserts into `relationships`. If `relation == Supersedes`, also sets `valid_until = datetime('now')` on the `to` memory. `MemoryDb::unrelate(from: &MemoryId, to: &MemoryId, relation: RelationType) -> Result<bool>`. Deletes the relationship row.

**TDD sequence**:
1. **Red**: Create two memories, relate with `related`, query relationships table, assert row exists. Relate with `supersedes`, assert target memory's `valid_until` is set. Unrelate, assert row deleted. Relate same pair twice with same type, assert UNIQUE constraint doesn't duplicate (upsert or ignore)
2. **Green**: Implement `relate` with `INSERT OR IGNORE` + conditional `UPDATE` for supersedes. `unrelate` with `DELETE`
3. **Refactor**: Combine the supersedes side-effect into a trigger or keep it in application code with clear documentation

**Acceptance**:
- `supersedes` auto-sets `valid_until` on the target
- Duplicate relationships are idempotent
- Unrelate returns false if relationship didn't exist

**Complexity**: S

### 2C: Chunking

#### Task 2C.1: Token-Based Chunker

**Files**: `crates/nous-core/src/chunk.rs`

**Implement**: `Chunker` struct holding `chunk_size: usize`, `chunk_overlap: usize`, `min_chunk: usize` (default 32). `Chunker::chunk(text: &str, tokenizer: &Tokenizer) -> Vec<Chunk>` where `Chunk { idx: usize, start_char: usize, end_char: usize, text: String }`. Uses the tokenizer to count tokens. Short text (< chunk_size tokens) returns one chunk. Long text produces N chunks with `chunk_overlap` token overlap. Chunk boundaries align to token boundaries (no mid-token splits).

**TDD sequence**:
1. **Red**: Create a mock tokenizer (split on whitespace, 1 token per word). Chunk a 10-word string with chunk_size=100: assert 1 chunk, start_char=0, end_char=len. Chunk a 100-word string with chunk_size=30, overlap=5: assert ~4 chunks, verify each chunk has ~30 words, verify overlap region contains 5 shared words between adjacent chunks. Chunk a 20-token text with min_chunk=32: assert 1 chunk (below min splits). Verify `start_char`/`end_char` map back to correct substrings of the original text
2. **Green**: Implement tokenizer-driven chunking loop: encode full text, walk token offsets, slice at chunk boundaries, track char offsets from token offsets
3. **Refactor**: Extract overlap calculation into a helper. Ensure the last chunk isn't below `min_chunk` by merging it into the previous chunk

**Acceptance**:
- Single chunk for short text
- Overlap tokens are shared between adjacent chunks
- Char offsets reconstruct exact substrings from the original text

**Complexity**: M

### 2D: Embedding Trait and Backend

#### Task 2D.1: EmbeddingBackend Trait

**Files**: `crates/nous-core/src/embed.rs`

**Implement**: The `EmbeddingBackend` trait from the spec:

```rust
pub trait EmbeddingBackend: Send + Sync {
    fn model_id(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn max_tokens(&self) -> usize;
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed(&[text])?.remove(0))
    }
}
```

Also implement `MockEmbedding` for tests: returns deterministic vectors based on text hash (e.g., hash each char, normalize to unit vector of configurable dimensions). This mock is used in all Phase 2 and Phase 3 tests to avoid ONNX model downloads.

**TDD sequence**:
1. **Red**: Create `MockEmbedding` with 64 dimensions. Call `embed_one("hello")`, assert output is `Vec<f32>` of length 64. Call `embed(&["a", "b"])`, assert 2 vectors returned. Assert same input produces same output (deterministic). Assert different inputs produce different vectors. Assert all vectors are unit-normalized (L2 norm ~= 1.0)
2. **Green**: Implement trait and `MockEmbedding` with a hash-based vector generator
3. **Refactor**: Add `MockEmbedding::with_dimensions(n)` builder for test flexibility

**Acceptance**:
- Trait is object-safe (`dyn EmbeddingBackend` works)
- `MockEmbedding` is deterministic and produces normalized vectors
- `embed_one` default implementation delegates to `embed`

**Complexity**: S

---

#### Task 2D.2: OnnxBackend Implementation

**Files**: `crates/nous-core/src/embed.rs`

**Implement**: `OnnxBackend` struct implementing `EmbeddingBackend`. Uses `hf-hub` to download model files, `tokenizers` crate for text tokenization with left-padding, `ort` for ONNX inference session (Level3 optimization). Last-token extraction + L2 normalization (Qwen3-Embedding pattern). Builder pattern: `OnnxBackend::builder().model("repo").variant("model_q4f16.onnx").build() -> Result<OnnxBackend>`. Batched inference with configurable batch size.

**TDD sequence**:
1. **Red**: Test requires network + model download, mark `#[ignore]` for CI. Build `OnnxBackend` with Qwen3-Embedding-0.6B-ONNX q4f16 variant. Call `embed_one("test sentence")`, assert vector length matches model dimensions. Call `embed(&["a", "b", "c"])`, assert 3 vectors returned. Assert vectors are L2-normalized. Assert `model_id()` returns the HF repo name
2. **Green**: Implement model download with `hf_hub::api::sync::Api`, tokenizer loading, ONNX session creation, inference with ndarray, last-token pooling, L2 normalization
3. **Refactor**: Cache the tokenizer and session in the struct. Add `Semaphore` for concurrent inference limiting (permits = CPU count)

**Acceptance**:
- Model downloads and caches via hf-hub
- Vectors are unit-normalized (L2 norm within 0.001 of 1.0)
- Builder validates model exists before creating session

**Complexity**: L

---

#### Task 2D.3: Chunk Storage and Vector Indexing

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::store_chunks(memory_id, chunks: &[Chunk], embeddings: &[Vec<f32>]) -> Result<()>`. Inserts into `memory_chunks` table. **sqlite-vec workaround**: If sqlite-vec is available (loaded via `load_extension` or `build.rs` compiled C), create `memory_vecs` vec0 table and insert embeddings. If unavailable, store embeddings as BLOBs in a fallback `memory_embeddings` table (`chunk_id TEXT PK, embedding BLOB`) and implement brute-force cosine similarity in Rust. `MemoryDb::delete_chunks(memory_id) -> Result<()>` removes all chunks and vectors for a memory.

**TDD sequence**:
1. **Red**: Store chunks for a memory (3 chunks with mock embeddings), query `memory_chunks`, assert 3 rows with correct `chunk_idx`, `start_char`, `end_char`. Delete chunks, assert 0 rows. Store chunks for two memories, delete one, assert other's chunks intact
2. **Green**: Implement `INSERT INTO memory_chunks` and the embedding storage (fallback BLOB table initially). `DELETE FROM memory_chunks WHERE memory_id = ?`
3. **Refactor**: Add conditional compilation or runtime feature flag for sqlite-vec vs fallback path

**Acceptance**:
- Chunks and embeddings stored atomically
- Deletion removes both chunks and embeddings
- Fallback BLOB path works when sqlite-vec is unavailable
- chunk_id follows format {memory_id}:{chunk_idx} per spec

**Complexity**: M

### 2E: Search (FTS5 and Semantic)

#### Task 2E.1: FTS5 Search

**Files**: `crates/nous-core/src/search.rs`

**Implement**: `MemoryDb::search_fts(query: &str, filters: &SearchFilters) -> Result<Vec<SearchResult>>`. Queries `memories_fts` with FTS5 MATCH syntax. `SearchFilters` struct: `memory_type: Option<MemoryType>`, `category_id: Option<i64>`, `workspace_id: Option<i64>`, `importance: Option<Importance>`, `confidence: Option<Confidence>`, `tags: Option<Vec<String>>`, `archived: Option<bool>` (default false), `since: Option<DateTime>`, `until: Option<DateTime>`, `valid_only: Option<bool>` (filters out memories where `valid_until < now`), `limit: Option<usize>` (default 20). `SearchResult` contains the memory plus a `rank: f64` from FTS5's `rank` function. Results ordered by rank, then by importance weight (high=3, moderate=2, low=1), then by recency.

**TDD sequence**:
1. **Red**: Store 3 memories with distinct content. FTS search for a word in memory #2, assert it appears first. Search with `memory_type` filter, assert only matching type returned. Search with `importance` filter. Search with `tags` filter. Search with `since`/`until` date range. Search with `valid_only = true`, assert expired memories excluded. Search empty query, assert error or empty results
2. **Green**: Build dynamic SQL joining `memories_fts` with `memories` and applying WHERE clauses from filters. Use `bm25()` for ranking
3. **Refactor**: Extract filter SQL generation into a `FilterBuilder` to share with semantic search

**Acceptance**:
- FTS5 ranking (BM25) determines primary sort order
- All filter combinations compose correctly
- Default limit is 20, configurable

**Complexity**: M

---

#### Task 2E.2: Semantic (Vector) Search

**Files**: `crates/nous-core/src/search.rs`

**Implement**: `MemoryDb::search_semantic(query_embedding: &[f32], filters: &SearchFilters) -> Result<Vec<SearchResult>>`. With sqlite-vec: uses `SELECT * FROM memory_vecs WHERE embedding MATCH ? ORDER BY distance LIMIT ?` (KNN query). Without sqlite-vec (fallback): loads all embeddings from `memory_embeddings`, computes cosine similarity in Rust, sorts, returns top-K. Joins back to `memories` via `memory_chunks.memory_id`. Deduplicates: if multiple chunks of the same memory match, return the memory once with the best chunk's score.

**TDD sequence**:
1. **Red**: Store 3 memories with mock embeddings (deterministic vectors). Generate a query embedding close to memory #1's vector. Search, assert memory #1 ranks first. Apply workspace filter, assert only matching workspace returned. Assert deduplication: store a long memory with 3 chunks, search, assert memory appears once (not 3 times)
2. **Green**: Implement fallback cosine similarity search. Load all embeddings, compute dot product (vectors are unit-normalized, so cosine = dot product), sort descending, apply filters post-retrieval or in SQL join
3. **Refactor**: When sqlite-vec becomes available, add the vec0 KNN query path behind a feature flag or runtime check

**Acceptance**:
- Returns memories ranked by cosine similarity
- Multi-chunk memories deduplicated to best-chunk score
- Filters apply correctly to semantic results

**Complexity**: M

---

#### Task 2E.3: Hybrid Search

**Files**: `crates/nous-core/src/search.rs`

**Implement**: `MemoryDb::search(query: &str, embedding: &[f32], filters: &SearchFilters, mode: SearchMode) -> Result<Vec<SearchResult>>`. `SearchMode` enum: `Fts`, `Semantic`, `Hybrid`. Hybrid mode runs both FTS and semantic searches, combines results using reciprocal rank fusion (RRF): `score = sum(1 / (k + rank_i))` with `k = 60`. Deduplicates by memory ID, keeping the fused score. Applies importance weighting: `final_score = rrf_score * importance_weight`. Logs access for all returned results.

**TDD sequence**:
1. **Red**: Store 5 memories. One matches FTS well but not semantically, one matches semantically but not FTS, one matches both. Hybrid search, assert the dual-match memory ranks highest. Assert FTS-only mode ignores embeddings. Assert Semantic-only mode ignores FTS. Assert access_log entries created for returned results
2. **Green**: Implement RRF fusion. Run both searches, merge by memory_id, compute fused score, sort, apply limit
3. **Refactor**: Make RRF `k` parameter configurable. Consider weighting FTS vs semantic contributions

**Acceptance**:
- Hybrid search outranks single-signal matches for dual-signal matches
- Access logging for all returned results
- Mode selection works (FTS-only, semantic-only, hybrid)

**Complexity**: M

---

#### Task 2E.4: Context Query

**Files**: `crates/nous-core/src/search.rs`

**Implement**: `MemoryDb::context(workspace_id: i64, summary: bool) -> Result<Vec<ContextEntry>>`. Returns active (non-archived, valid) memories for the workspace, ordered by importance (high first), then recency. If `summary = true`, returns only `id` and `title` (token-efficient for session start). If `summary = false`, returns full content. Limits to 50 results.

**TDD sequence**:
1. **Red**: Store 5 memories in workspace A, 3 in workspace B. Context query for A, assert 5 results. Assert high-importance memories sort first. Assert archived memories excluded. Summary mode: assert content field is empty/None. Assert valid_until-expired memories excluded
2. **Green**: `SELECT` from `memories` with workspace and archived filters, ORDER BY importance DESC, created_at DESC, LIMIT 50
3. **Refactor**: Add pagination support if 50 is insufficient

**Acceptance**:
- Only active, valid memories for the specified workspace
- High-importance memories first
- Summary mode omits content

**Complexity**: S

### 2F: Category Classification

#### Task 2F.1: Category Embedding Cache

**Files**: `crates/nous-core/src/classify.rs`

**Implement**: `CategoryClassifier::new(db: &MemoryDb, embedder: &dyn EmbeddingBackend) -> Result<CategoryClassifier>`. On construction, loads all categories from DB. For any category with `embedding IS NULL`, computes embedding of `"{name} {description}"` and stores it back. Caches all category embeddings in a `HashMap<i64, (Category, Vec<f32>)>`. Provides `refresh()` to recompute after category changes.

**TDD sequence**:
1. **Red**: Open DB with seeded categories. Create classifier with `MockEmbedding`. Assert all categories now have non-NULL embedding in DB. Assert internal cache has entries for all categories. Add a new category, call `refresh()`, assert new category has embedding
2. **Green**: Query `SELECT id, name, description, embedding, parent_id FROM categories`. For NULL embeddings, call `embedder.embed_one()`, `UPDATE categories SET embedding = ? WHERE id = ?`. Build cache HashMap
3. **Refactor**: Batch-embed all NULL categories in one `embed()` call instead of N `embed_one()` calls

**Acceptance**:
- All categories have embeddings after construction
- Cache reflects current DB state after `refresh()`
- Batch embedding reduces calls to the backend

**Complexity**: M

---

#### Task 2F.2: Zero-Shot Classification

**Files**: `crates/nous-core/src/classify.rs`

**Implement**: `CategoryClassifier::classify(memory_embedding: &[f32]) -> Option<i64>`. Follows the spec algorithm: (1) compare against all top-level category embeddings (cosine similarity), (2) pick best match above threshold (0.3), (3) if best has children, repeat against children only, (4) return best leaf or top-level match. Returns `None` if no category scores above threshold.

**TDD sequence**:
1. **Red**: Using `MockEmbedding`, store a memory whose content is "kubernetes pod scheduling in production cluster". Classify it, assert category is `infrastructure` or its child `k8s`. Store a memory with gibberish content, classify, assert `None` (below threshold). Store memory about "python dependency upgrade", assert category is `languages/python` or `libraries`
2. **Green**: Implement cosine similarity comparison loop. Filter top-level (parent_id IS NULL), find best, check threshold, descend into children if available
3. **Refactor**: Make threshold configurable (read from config or constructor param). Pre-filter categories by source if needed

**Acceptance**:
- Returns correct category for domain-specific content
- Returns `None` for unclassifiable content
- Hierarchical descent picks leaf category when possible

**Complexity**: M

---

#### Task 2F.3: Category CRUD

**Files**: `crates/nous-core/src/classify.rs`, `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::category_add(name, parent_id, description, source) -> Result<i64>`. Inserts category, computes embedding if embedder is available. `MemoryDb::category_list(source_filter: Option<CategorySource>) -> Result<Vec<CategoryTree>>` returns hierarchical tree. `MemoryDb::category_suggest(name, description, memory_id) -> Result<i64>` creates a category with `source = 'agent'`, assigns it to the memory, computes embedding. `CategoryTree` is a recursive struct with `children: Vec<CategoryTree>`.

**TDD sequence**:
1. **Red**: Add a top-level category "testing", assert it appears in `category_list`. Add a child "unit-tests" under "testing", assert parent-child link. Suggest a category for a memory, assert memory's `category_id` updated. List with `source_filter = Some(Agent)`, assert only agent-suggested categories returned
2. **Green**: `INSERT INTO categories`, with parent_id lookup. Tree construction: query all, build parent-child map in Rust
3. **Refactor**: Add `category_rename` and `category_delete` (with reassignment of orphan memories) for completeness

**Acceptance**:
- Categories are unique by name
- Tree structure correctly nests children under parents
- Agent-suggested categories are tagged with `source = 'agent'`

**Complexity**: M

### 2G: Model Registry

#### Task 2G.1: Model Registry

**Files**: `crates/nous-core/src/model.rs`

**Implement**: `MemoryDb::register_model(model_id, variant, dimensions, chunk_size, chunk_overlap) -> Result<i64>`. Inserts into `models` table with `active = 0`. `MemoryDb::activate_model(id: i64) -> Result<()>` sets `active = 1` on the target, `active = 0` on all others (single active model invariant). `MemoryDb::active_model() -> Result<Option<Model>>` returns the row with `active = 1`. `MemoryDb::deactivate_model(id: i64) -> Result<()>` sets `active = 0`. `MemoryDb::list_models() -> Result<Vec<Model>>` returns all rows.

**TDD sequence**:
1. **Red**: Register model A, assert `active = 0`. Activate model A, assert `active_model()` returns A. Register model B, activate B, assert `active_model()` returns B and A is now `active = 0`. Deactivate B, assert `active_model()` returns `None`. List models, assert both A and B present with correct dimensions and chunk params
2. **Green**: Implement with `UPDATE models SET active = 0; UPDATE models SET active = 1 WHERE id = ?` in a transaction
3. **Refactor**: Add a check that dimensions > 0 and chunk_size > chunk_overlap on registration

**Acceptance**:
- Exactly zero or one model is active at any time
- Activating a model deactivates all others atomically
- Model metadata (dimensions, chunk params) is preserved

**Complexity**: S

### 2H: Access Log and Temporal Validity

#### Task 2H.1: Access Log

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `MemoryDb::log_access(memory_id: &MemoryId, tool_name: &str) -> Result<()>`. Inserts into `access_log`. `MemoryDb::most_accessed(since: Option<DateTime>, limit: usize) -> Result<Vec<(MemoryId, u64)>>` returns memory IDs ranked by access count in the given time window. `MemoryDb::access_count(memory_id: &MemoryId) -> Result<u64>`.

**TDD sequence**:
1. **Red**: Store a memory, log 5 accesses with tool_name "memory_recall". Query `access_count`, assert 5. Store another memory, log 2 accesses. Query `most_accessed(None, 10)`, assert first memory ranks above second. Query with `since = 1 hour ago`, assert correct counts. Assert `tool_name` is stored correctly
2. **Green**: `INSERT INTO access_log (memory_id, tool_name)`. `SELECT memory_id, COUNT(*) ... GROUP BY memory_id ORDER BY COUNT(*) DESC`
3. **Refactor**: Add index hint or ensure query uses `idx_access_log_time` for date-filtered queries

**Acceptance**:
- Append-only log (no updates or deletes by API)
- Time-windowed queries use the time index
- Correct ranking by access frequency

**Complexity**: S

---

#### Task 2H.2: Temporal Validity

**Files**: `crates/nous-core/src/db.rs`

**Implement**: Extend `MemoryDb::relate()` to auto-set `valid_until = datetime('now')` on the target memory when `relation = supersedes`. Extend `SearchFilters` with `valid_only: bool` (default true in `memory_search`, false in `memory_recall`). When `valid_only = true`, add `AND (valid_until IS NULL OR valid_until > datetime('now'))` to WHERE clause. `MemoryDb::update()` allows explicit `valid_until` setting.

**TDD sequence**:
1. **Red**: Create memory A, create memory B. Relate B supersedes A. Recall A, assert `valid_until` is set (not NULL). Search with `valid_only = true`, assert A excluded. Search with `valid_only = false`, assert A included. Explicitly set `valid_until` on a memory via update, assert it takes effect in filtered searches
2. **Green**: Add `UPDATE memories SET valid_until = datetime('now') WHERE id = ?` inside `relate()` for supersedes case. Add WHERE clause to search methods
3. **Refactor**: Consider a trigger for the supersedes side-effect instead of application code

**Acceptance**:
- `supersedes` relationship auto-expires the target
- Filtered searches exclude expired memories by default
- Explicit `valid_until` works independently of relationships

**Complexity**: S

### 2I: Encryption

#### Task 2I.1: Encryption Key Management

**Files**: `crates/nous-shared/src/sqlite.rs` (or new `crates/nous-shared/src/key.rs`)

**Implement**: Key resolution: (1) `NOUS_DB_KEY` env var, (2) `~/.config/nous/db.key` default path. `resolve_key() -> Result<Option<String>>`. The `db_key_file` config.toml override is deferred to Phase 3 when the config system exists. On first run with no key source, generate 32-byte random key, write to `~/.config/nous/db.key` with mode 0600. Extend `open_connection` to apply `PRAGMA key = ?` before any other pragma. Detect wrong key: catch `SQLITE_NOTADB` on first query, return `NousError::Encryption("database key is incorrect or database is not encrypted")`.

**TDD sequence**:
1. **Red**: In a temp dir, create an encrypted DB with a known key. Reopen with the same key, assert data readable. Reopen with a wrong key, assert `NousError::Encryption` returned. Set `NOUS_DB_KEY` env var, assert `resolve_key()` returns it. Unset env var, create `~/.config/nous/db.key`, assert `resolve_key()` reads it. No key anywhere + first run: assert key file created at `~/.config/nous/db.key` with 0600 permissions
2. **Green**: Implement key resolution chain. Modify `open_connection` to accept `Option<String>` key. After `PRAGMA key`, execute `SELECT count(*) FROM sqlite_master` as a canary query to detect wrong key
3. **Refactor**: Extract key file I/O into `KeyManager` struct for testability (inject paths instead of hardcoding)

**Acceptance**:
- Wrong key produces a clear error, not a panic or garbled data
- Auto-generated key file has restrictive permissions (0600)
- Key resolution order matches spec exactly

**Complexity**: M

---

#### Task 2I.2: Key Rotation

**Files**: `crates/nous-shared/src/sqlite.rs` (or `crates/nous-shared/src/key.rs`)

**Implement**: `rotate_key(db_path: &Path, current_key: &str, new_key: &str) -> Result<()>`. Applies to BOTH `memory.db` and `otlp.db`. Creates a backup of the DB file first (e.g., `memory.db.bak`, `otlp.db.bak`). Opens with current key, executes `PRAGMA rekey = ?` with the new key. Updates the key file if key was sourced from file. Verifies each rotated DB by reopening with the new key and running integrity check.

**TDD sequence**:
1. **Red**: Create two encrypted DBs (memory.db, otlp.db) with key "old". Rotate both to "new". Reopen each with "new", assert data intact. Reopen each with "old", assert `NousError::Encryption`. Assert backup files exist for both. Attempt rotation with wrong current key, assert error before any modification. Assert `PRAGMA integrity_check` passes on both after rotation
2. **Green**: Implement backup via `std::fs::copy`. For each DB: open with current key, `PRAGMA rekey`. Close, reopen with new key, `PRAGMA integrity_check`
3. **Refactor**: Add rollback: if post-rotation verification fails on either DB, restore both from backup

**Acceptance**:
- Backup created before rotation for both memory.db and otlp.db
- New key works, old key rejected after rotation on both databases
- Integrity check passes post-rotation on both databases

**Complexity**: M

### 2J: Write Channel

#### Task 2J.1: Write Channel

**Files**: `crates/nous-core/src/db.rs` (or new `crates/nous-core/src/channel.rs`)

**Implement**: `WriteChannel` struct wrapping `tokio::sync::mpsc::Sender<WriteOp>`. `WriteOp` enum variants: `Store(NewMemory, oneshot::Sender<Result<MemoryId>>)`, `Update(MemoryId, MemoryPatch, oneshot::Sender<Result<bool>>)`, `Forget(MemoryId, bool, oneshot::Sender<Result<bool>>)`, `Relate(MemoryId, MemoryId, RelationType, oneshot::Sender<Result<()>>)`, etc. `WriteWorker` task: loop of recv + try_recv (up to 31 more) + execute batch in one transaction + respond via oneshot. Channel capacity: 256. `WriteChannel::send(op) -> Result<Response>` wraps the oneshot pattern.

**TDD sequence**:
1. **Red**: Create channel + worker with in-memory DB. Send a `Store` op, await result, assert memory ID returned. Send 10 concurrent `Store` ops (via `tokio::spawn`), await all, assert all 10 succeed and produce unique IDs. Verify batching: instrument the worker to count transactions, assert fewer than 10 transactions for 10 concurrent writes. Test backpressure: fill channel to capacity (256 ops without draining), assert the 257th send blocks (use `tokio::time::timeout` to detect). Drop the channel sender, assert worker exits cleanly
2. **Green**: Implement `WriteWorker::run()` loop: `recv().await` for first op, `try_recv()` loop for batch, `BEGIN`/`COMMIT` transaction, send results. `WriteChannel::send()` creates oneshot, sends op, awaits response
3. **Refactor**: Add metrics: batch size histogram, channel depth gauge. Extract `execute_batch(ops)` method for testability

**Acceptance**:
- Concurrent writes are serialized through single writer
- Batching occurs under load (multiple ops per transaction)
- Backpressure at capacity 256 (sender blocks, does not error)
- Clean shutdown when channel is dropped

**Complexity**: L

---

#### Task 2J.2: Read Pool

**Files**: `crates/nous-core/src/db.rs`

**Implement**: `ReadPool` struct managing N read connections (default: 4). Each connection opened with `PRAGMA query_only = ON` + the same encryption key and WAL pragmas. Uses a `tokio::sync::Semaphore` (N permits) to limit concurrent reads. `ReadPool::with_conn<F, T>(f: F) -> Result<T>` acquires a permit, runs the closure on a connection via `spawn_blocking`, releases permit. Connections are reusable (not opened/closed per query).

**TDD sequence**:
1. **Red**: Create pool with 2 connections on in-memory DB. Execute 2 concurrent reads, assert both succeed. Execute a write via one pool connection, assert it fails (`query_only` pragma). Execute 5 concurrent reads on 2-connection pool, assert all 5 complete (semaphore queues excess). Assert `PRAGMA query_only` is ON for each connection
2. **Green**: Pre-open N connections in `ReadPool::new()`. Store in `Vec<Mutex<Connection>>` or use a connection recycling pattern. Semaphore for limiting concurrency
3. **Refactor**: Use a crossbeam `ArrayQueue` or similar lock-free pool instead of Vec<Mutex<>>

**Acceptance**:
- Read connections reject write operations
- Pool limits concurrent access to N connections
- Connections are reused across queries

**Complexity**: M

---

## Phase 3: nous-mcp

### 3A: CLI and Server Bootstrap

#### Task 3A.1: CLI Parsing

**Files**: `crates/nous-mcp/src/main.rs`

**Implement**: Clap derive CLI matching the spec's command surface:

```rust
#[derive(Parser)]
enum Cli {
    Serve { #[arg(long, default_value = "stdio")] transport: Transport, #[arg(long, default_value_t = 8377)] port: u16 },
    ReEmbed { #[arg(long)] model: String, #[arg(long)] variant: Option<String> },
    ReClassify { #[arg(long)] since: Option<String> },
    Category(CategoryCmd),
    Export { #[arg(long, default_value = "json")] format: String },
    Import { file: PathBuf },
    RotateKey { #[arg(long)] new_key_file: Option<PathBuf> },
    Status,
}
```

Parse `Transport` enum: `Stdio`, `Http`. Each subcommand dispatches to the appropriate handler module.

**TDD sequence**:
1. **Red**: Unit test CLI parsing: `Cli::try_parse_from(["nous-mcp", "serve"])` succeeds with default transport=stdio. `["nous-mcp", "serve", "--transport", "http", "--port", "9000"]` parses correctly. `["nous-mcp", "re-embed", "--model", "org/repo"]` parses. `["nous-mcp", "category", "add", "testing"]` parses. Invalid subcommand returns error
2. **Green**: Implement the Clap derive structs
3. **Refactor**: Move subcommand enums to separate types if `main.rs` gets large

**Acceptance**:
- All spec CLI commands parse correctly
- Default values match spec (stdio transport, port 8377)
- Help text is generated for all commands

**Complexity**: S

---

#### Task 3A.2: Configuration Loading

**Files**: `crates/nous-mcp/src/main.rs` (or new `crates/nous-mcp/src/config.rs`)

**Implement**: `Config` struct matching `~/.config/nous/config.toml` from the spec. Fields: `memory.db_path`, `embedding.model`, `embedding.variant`, `embedding.chunk_size`, `embedding.chunk_overlap`, `otlp.db_path`, `otlp.port`, `classification.confidence_threshold`, `encryption.db_key_file`. Load with `toml::from_str`. Env var overrides: `NOUS_MEMORY_DB` overrides `memory.db_path`, `NOUS_OTLP_DB` overrides `otlp.db_path`, `NOUS_DB_KEY` overrides encryption key. Create default config file on first run if absent.

**TDD sequence**:
1. **Red**: Write a config TOML string, parse it, assert all fields populated. Parse with missing optional fields, assert defaults. Set `NOUS_MEMORY_DB` env var, assert it overrides `memory.db_path`. Parse invalid TOML, assert error. Assert default config file is written when path doesn't exist
2. **Green**: Define `Config` with `serde::Deserialize`, implement `Config::load(path: Option<PathBuf>) -> Result<Config>` with file read + env var overlay
3. **Refactor**: Use `config` crate or keep manual for simplicity

**Acceptance**:
- All spec config fields are supported
- Env vars override file values
- Missing config file is auto-created with defaults
- When config is loaded, `encryption.db_key_file` is passed to `resolve_key()` as an additional override between `NOUS_DB_KEY` env var and the `~/.config/nous/db.key` default (deferred from Phase 2 Task 2I.1)

**Complexity**: S

---

#### Task 3A.3: MCP Server Bootstrap

**Files**: `crates/nous-mcp/src/server.rs`

**Implement**: `NousServer` struct holding `WriteChannel`, `ReadPool`, `EmbeddingBackend`, `CategoryClassifier`, `Config`. Implements `rmcp::ServerHandler` with tool registration. `NousServer::start(config: Config) -> Result<()>`: opens DB, runs migrations, initializes embedding backend, builds classifier, creates write channel + read pool, registers MCP tools, starts serving on configured transport (stdio or HTTP via streamable-http-server).

**TDD sequence**:
1. **Red**: Construct `NousServer` with in-memory DB and `MockEmbedding`. Assert it implements `rmcp::ServerHandler`. Call `list_tools()`, assert all expected tool names present (memory_store, memory_recall, memory_search, memory_context, memory_forget, memory_unarchive, memory_update, memory_relate, memory_unrelate, memory_category_suggest, memory_workspaces, memory_tags, memory_stats, memory_schema, memory_sql). Assert tool count matches spec (15 tools)
2. **Green**: Implement `NousServer` struct, derive or impl `ServerHandler`, register tools via `rmcp` macros. Stub tool handlers to return `NotImplemented` (filled in Task 3B)
3. **Refactor**: Group related tools into sub-modules (lifecycle, search, taxonomy, introspection)

**Acceptance**:
- Server starts without errors on both stdio and HTTP transports
- All 15 MCP tools from the spec are registered
- Server accepts `MockEmbedding` for testing

**Complexity**: M

### 3B: MCP Tool Handlers

#### Task 3B.1: Memory Lifecycle Tools

**Files**: `crates/nous-mcp/src/tools.rs`

**Implement**: Tool handlers for `memory_store`, `memory_recall`, `memory_forget`, `memory_unarchive`, `memory_update`. Each handler: (1) validates input params, (2) performs embedding if needed (store, update with content change, unarchive), (3) runs classification if category not provided (store), (4) sends write op to channel or reads from pool, (5) returns structured JSON result.

`memory_store` pipeline: validate -> embed content -> chunk -> classify -> send `WriteOp::Store` (includes chunks + vectors) -> return ID + assigned category.

**TDD sequence**:
1. **Red**: Call `memory_store` tool handler with `{title, content, type}`, assert returns `{id, category}`. Call `memory_recall` with returned ID, assert all fields match. Call `memory_update` with new title, recall again, assert title changed. Call `memory_forget` (soft), recall, assert archived. Call `memory_unarchive`, recall, assert not archived. Store without category, assert auto-classification assigned one. Store with explicit category, assert it is used (no auto-classify)
2. **Green**: Implement each handler function. Wire params from MCP tool input schema to `MemoryDb` methods via `WriteChannel` or `ReadPool`
3. **Refactor**: Extract common patterns (input validation, error mapping) into a shared handler helper

**Acceptance**:
- Full lifecycle: store -> recall -> update -> forget -> unarchive works
- Auto-classification activates when category is omitted
- Embedding + chunking happens on store and content-changing updates

**Complexity**: L

---

#### Task 3B.2: Search and Context Tools

**Files**: `crates/nous-mcp/src/tools.rs`

**Implement**: `memory_search` handler: accepts `query`, `mode` (fts/semantic/hybrid, default hybrid), and all `SearchFilters` fields. Embeds the query for semantic/hybrid modes. Calls `MemoryDb::search()`. Returns ranked results with scores. `memory_context` handler: accepts `workspace` (path or alias) and `summary` (bool, default true). Resolves workspace, calls `MemoryDb::context()`.

**TDD sequence**:
1. **Red**: Store 3 memories with distinct content. Call `memory_search` with a keyword, assert matching memory ranks first. Call with `mode: "fts"`, assert FTS-only results. Call with `mode: "semantic"`, assert vector-based results. Call `memory_context` with workspace path, assert returns high-importance memories first. Call with `summary: true`, assert content is omitted
2. **Green**: Implement handlers. For semantic search, embed the query string via the server's embedding backend. Map filter params to `SearchFilters` struct
3. **Refactor**: Add input schema validation (e.g., reject empty query for FTS mode)

**Acceptance**:
- All three search modes work end-to-end
- Context returns workspace-scoped results
- Access log is updated for returned results

**Complexity**: M

---

#### Task 3B.3: Relationship, Taxonomy, and Introspection Tools

**Files**: `crates/nous-mcp/src/tools.rs`

**Implement**: `memory_relate`, `memory_unrelate`, `memory_category_suggest`, `memory_workspaces`, `memory_tags`, `memory_stats`, `memory_schema`, `memory_sql`. `memory_workspaces` lists all workspaces with memory counts; workspaces are auto-created on first `memory_store` for a given path, no explicit creation needed. `memory_tags` lists all tags with usage counts, ordered by frequency. `memory_sql` validates input: only `SELECT`, `EXPLAIN`, `PRAGMA` (read-only), and read-only `WITH` allowed. Rejects `INSERT`, `UPDATE`, `DELETE`, `DROP`, `ALTER`, `CREATE`, `ATTACH`.

**TDD sequence**:
1. **Red**: Store two memories, call `memory_relate` with `supersedes`, assert target's `valid_until` is set. Call `memory_unrelate`, assert relationship removed. Call `memory_category_suggest` with a new category name and memory ID, assert category created and assigned. Call `memory_workspaces`, assert workspace listed with memory count. Call `memory_tags`, assert tags listed with usage counts. Call `memory_stats`, assert counts are correct. Call `memory_schema`, assert returns SQL text. Call `memory_sql` with `SELECT count(*) FROM memories`, assert returns count. Call `memory_sql` with `DELETE FROM memories`, assert rejected
2. **Green**: Implement each handler delegating to `MemoryDb` methods. For `memory_sql`, parse the SQL statement prefix and reject write operations
3. **Refactor**: Use a SQL parser (e.g., `sqlparser` crate) instead of prefix matching for more robust validation of `memory_sql`

**Acceptance**:
- All 8 tools work end-to-end
- `memory_sql` rejects all write operations
- `memory_category_suggest` computes embedding for the new category

**Complexity**: M

---

#### Task 3B.4: CLI Subcommands (Non-MCP)

**Files**: `crates/nous-mcp/src/main.rs`

**Implement**: `re-embed`, `re-classify`, `category list/add`, `export`, `import`, `rotate-key`, `status`. These run as one-shot CLI commands (not MCP tools).

`re-embed`: Download new model, register + activate in model registry, update `[embedding]` section in `~/.config/nous/config.toml` with new model and variant, drop + recreate vec table, delete all chunks, recompute category embeddings, walk all non-archived memories, chunk + embed + store. Report summary.

`export`: Serialize all memories (with tags, relationships, categories) as JSON to stdout or file.

`import`: Deserialize JSON, insert memories, re-embed on import.

`status`: Print DB path, memory count, category count, active model, channel depth.

**TDD sequence**:
1. **Red**: Store 3 memories. Run `export`, capture JSON output, assert 3 memories with tags and relationships. Wipe DB (delete file, re-open). Run `import` with the JSON, assert 3 memories restored with correct content. Run `status`, assert output contains memory count = 3. For `re-classify`: store a memory without category, run re-classify, assert category assigned. For `re-embed`: mock a model swap, assert chunks regenerated
2. **Green**: Implement each subcommand handler. Export uses `serde_json::to_writer_pretty`. Import uses `serde_json::from_reader` + store pipeline
3. **Refactor**: Share the embed+chunk+store pipeline between `memory_store` tool handler and `import`/`re-embed` commands

**Acceptance**:
- Export/import round-trip preserves all data (memories, tags, relationships, categories)
- Re-embed regenerates all chunks and vectors
- Status shows accurate counts

**Complexity**: L

### 3C: Integration Tests

#### Task 3C.1: MCP Tool Round-Trip Integration Test

**Files**: `crates/nous-mcp/tests/integration.rs`

**Implement**: End-to-end test using `NousServer` with in-memory DB and `MockEmbedding`. Exercises the full MCP tool pipeline without process boundaries: construct server, call tools programmatically via `rmcp` test harness.

**TDD sequence**:
1. **Red**: Write the test: `memory_store` -> `memory_search` (FTS) -> assert found -> `memory_search` (semantic) -> assert found -> `memory_recall` by ID -> assert full fields -> `memory_update` title -> `memory_recall` -> assert updated -> `memory_forget` (soft) -> `memory_search` -> assert not in results -> `memory_unarchive` -> `memory_search` -> assert found again
2. **Green**: Build `NousServer` in test, call tool handlers directly. Assert each step's output
3. **Refactor**: Extract test helper `TestServer` that bootstraps in-memory server with `MockEmbedding`

**Acceptance**:
- Full lifecycle exercised in a single test
- No external dependencies (no model download, no disk DB)
- All assertions pass

**Complexity**: M

---

#### Task 3C.2: Concurrent Writes Integration Test

**Files**: `crates/nous-mcp/tests/concurrent.rs`

**Implement**: Spawn 10 tokio tasks, each calling `memory_store` on the same `NousServer` instance. All 10 complete. Verify: 10 distinct memory IDs, all retrievable via `memory_recall`, no duplicate IDs, DB integrity check passes. Instrument write worker to count transactions, assert batching occurred (fewer than 10 transactions for 10 writes).

**TDD sequence**:
1. **Red**: Write the concurrent test. Spawn 10 tasks with `JoinSet`, await all, collect results, assert 10 `Ok(id)` values. Query `SELECT count(*) FROM memories`, assert 10. Run `PRAGMA integrity_check`, assert "ok"
2. **Green**: Use the `TestServer` helper from Task 3C.1. Ensure `WriteChannel` is cloneable (sender is cloneable)
3. **Refactor**: Parameterize concurrency level (test with 10, 50, 100)

**Acceptance**:
- 10 concurrent writes produce 10 memories with no data loss
- Write batching occurs (observable via transaction count or timing)
- No deadlocks or panics under concurrent load

**Complexity**: M

---

#### Task 3C.3: Semantic Search Ranking Test

**Files**: `crates/nous-mcp/tests/search_ranking.rs`

**Implement**: Store 3 memories with semantically distinct content (e.g., "Kubernetes pod scheduling", "Python dependency management", "SQL query optimization"). Search with a query close to one topic. Assert the closest memory ranks first. Use `MockEmbedding` configured to produce vectors where similar text produces similar hashes (the mock's hash-based approach gives weak semantic signal, but deterministic ordering is testable).

**TDD sequence**:
1. **Red**: Store 3 memories. Search with query matching memory #2's content. Assert memory #2 is result[0]. Assert all 3 are returned (no filtering). Assert scores are monotonically decreasing
2. **Green**: Relies on `MockEmbedding` producing consistent similarity ordering. If mock's hash doesn't give good ordering, adjust mock to use a known mapping (e.g., embed "kubernetes" and "k8s" to nearby vectors)
3. **Refactor**: Add a `MockEmbedding::with_mapping(HashMap<String, Vec<f32>>)` for controlled test scenarios

**Acceptance**:
- Closest semantic match ranks first
- All stored memories appear in results
- Scores decrease monotonically

**Complexity**: S

---

#### Task 3C.4: Export/Import Round-Trip Test

**Files**: `crates/nous-mcp/tests/export_import.rs`

**Implement**: Store 5 memories with tags, relationships, categories, and workspace assignments. Export to JSON. Create a new empty DB. Import. Verify: all 5 memories present with identical content, tags, relationships. Categories preserved. Workspaces preserved. IDs preserved (or mapped if import generates new IDs).

**TDD sequence**:
1. **Red**: Setup: store 5 memories, add tags, create relationships (including one `supersedes`), assign categories. Export to `Vec<u8>`. Open fresh DB. Import from buffer. Assert `SELECT count(*)` matches for memories, tags, relationships, categories. Spot-check: recall memory #3, assert title, content, tags all match original
2. **Green**: Implement export as JSON serialization of all tables. Import as deserialization + store pipeline
3. **Refactor**: Add checksum or version header to export format for future compatibility

**Acceptance**:
- All data survives export/import cycle
- Relationships (including supersedes side-effects) are preserved
- Import into empty DB works without errors

**Complexity**: M

---

## Phase 4: nous-otlp

### 4A: Protobuf Decoding and Schema

#### Task 4A.1: OTLP SQLite Schema

**Files**: `crates/nous-otlp/src/db.rs`

**Implement**: Schema for OTLP storage. Tables: `log_events` (timestamp, severity, body, resource_attrs, scope_attrs, log_attrs, session_id, trace_id, span_id), `spans` (trace_id, span_id, parent_span_id, name, kind, start_time, end_time, status_code, status_message, resource_attrs, span_attrs, events_json), `metrics` (name, description, unit, type, data_points_json, resource_attrs, timestamp). Use `nous_shared::sqlite` for connection setup and migrations. DB at `~/.cache/nous/otlp.db`, encrypted with same key as memory.db.

**TDD sequence**:
1. **Red**: Open in-memory OTLP DB, assert tables `log_events`, `spans`, `metrics` exist. Assert indexes on `session_id`, `trace_id`, `timestamp` columns. Insert a sample log event row, select it back, assert fields match
2. **Green**: Define `OTLP_MIGRATIONS` const array. Implement `OtlpDb::open(path, key)` using shared SQLite helpers
3. **Refactor**: Share the migration pattern with `MemoryDb` — both use `run_migrations` from `nous-shared`

**Acceptance**:
- All three OTLP tables created with correct columns
- Indexes on correlation ID columns (session_id, trace_id)
- Encryption works with same key resolution as memory DB

**Complexity**: M

---

#### Task 4A.2: Protobuf Decoding

**Files**: `crates/nous-otlp/src/decode.rs`

**Implement**: `decode_logs(body: &[u8]) -> Result<Vec<LogEvent>>`, `decode_traces(body: &[u8]) -> Result<Vec<Span>>`, `decode_metrics(body: &[u8]) -> Result<Vec<Metric>>`. Uses `opentelemetry-proto` types + `prost` for deserialization. Extracts fields from the nested protobuf structure (ResourceLogs -> ScopeLogs -> LogRecord, etc.). Maps OTLP severity numbers to strings. Serializes resource/scope attributes as JSON strings for storage.

**TDD sequence**:
1. **Red**: Construct an `ExportLogsServiceRequest` protobuf message in Rust (using `opentelemetry-proto` builder). Encode to bytes via `prost::Message::encode`. Call `decode_logs`, assert correct number of log events. Assert severity, body, timestamp fields match. Same pattern for traces (construct `ExportTraceServiceRequest`, encode, decode, verify spans) and metrics
2. **Green**: Implement decoders that walk the protobuf tree: resource -> scope -> record. Flatten into storage-friendly structs. Serialize attributes with `serde_json`
3. **Refactor**: Extract attribute flattening into a shared helper (used by all three decoders)

**Acceptance**:
- Round-trip: construct protobuf -> encode -> decode -> assert fields match
- Handles multiple resources, scopes, and records in a single request
- Attributes serialized as JSON strings

**Complexity**: M

---

#### Task 4A.3: OTLP Storage

**Files**: `crates/nous-otlp/src/db.rs`

**Implement**: `OtlpDb::store_logs(logs: &[LogEvent]) -> Result<usize>`, `OtlpDb::store_spans(spans: &[Span]) -> Result<usize>`, `OtlpDb::store_metrics(metrics: &[Metric]) -> Result<usize>`. Batch inserts in a single transaction. Returns count of inserted rows. `OtlpDb::query_logs(session_id: &str) -> Result<Vec<LogEvent>>`, `OtlpDb::query_spans(trace_id: &str) -> Result<Vec<Span>>` for cross-reference queries.

**TDD sequence**:
1. **Red**: Store 5 log events with a session_id. Query by session_id, assert 5 returned in timestamp order. Store 3 spans with a trace_id. Query by trace_id, assert 3 returned. Store metrics, assert count matches. Store empty vec, assert 0 returned (no error)
2. **Green**: Implement batch `INSERT INTO` in a transaction. Query with WHERE clause on correlation ID, ORDER BY timestamp
3. **Refactor**: Add pagination (LIMIT/OFFSET) to query methods for large result sets

**Acceptance**:
- Batch insert is atomic (all or nothing)
- Query by session_id/trace_id returns correct records
- Empty input is handled gracefully

**Complexity**: S

### 4B: HTTP Server and Endpoints

#### Task 4B.1: HTTP Server and OTLP Endpoints

**Files**: `crates/nous-otlp/src/server.rs`, `crates/nous-otlp/src/main.rs`

**Implement**: Axum HTTP server on `127.0.0.1:4318` (configurable). Three POST endpoints matching the OTLP HTTP/protobuf spec:
- `POST /v1/logs` — accepts `application/x-protobuf`, decodes via `decode_logs`, stores, returns 200
- `POST /v1/traces` — accepts `application/x-protobuf`, decodes via `decode_traces`, stores, returns 200
- `POST /v1/metrics` — accepts `application/x-protobuf`, decodes via `decode_metrics`, stores, returns 200

Content-Type validation: reject non-protobuf with 415 Unsupported Media Type. Error responses follow OTLP conventions (partial success with rejected count).

CLI: `nous-otlp serve [--port PORT] [--db PATH]`. `nous-otlp status` prints DB path, total event counts, uptime.

**TDD sequence**:
1. **Red**: Start server in test with in-memory DB on a random port. POST a protobuf-encoded `ExportLogsServiceRequest` to `/v1/logs`, assert 200. Query DB, assert log events stored. POST to `/v1/traces`, assert 200 + spans stored. POST with `Content-Type: application/json`, assert 415. POST malformed bytes, assert 400. Test `status` CLI subcommand output
2. **Green**: Build axum `Router` with 3 routes. Each handler: read body, decode protobuf, store to DB, return 200. Add content-type middleware. Implement `serve` and `status` subcommands
3. **Refactor**: Add graceful shutdown via `tokio::signal::ctrl_c`. Add request logging middleware

**Acceptance**:
- OTLP-compatible HTTP endpoints accept protobuf payloads
- Invalid content type returns 415
- Malformed protobuf returns 400
- Server binds to localhost only (no external access)

**Complexity**: M

---

#### Task 4B.2: OTLP Concurrent Ingestion Test

**Files**: `crates/nous-otlp/tests/ingestion.rs`

**Implement**: Spawn server on random port. Send 20 concurrent protobuf payloads (mix of logs, traces, metrics) via `reqwest` or `hyper` client. Assert all return 200. Query DB, assert all events stored. Assert no data loss or duplication.

**TDD sequence**:
1. **Red**: Build 20 payloads (10 log batches, 5 trace batches, 5 metric batches). Spawn 20 tokio tasks, each POSTing one payload. Await all, assert 20 x 200. Count rows in each table, assert totals match input
2. **Green**: Use `reqwest::Client` with connection pooling. Server uses write serialization (WAL mode handles concurrent writes from axum handler tasks)
3. **Refactor**: Add timing assertions — all 20 requests complete within 5 seconds (sanity check, not performance benchmark)

**Acceptance**:
- 20 concurrent requests handled without errors
- All ingested data queryable after completion
- No duplicate rows

**Complexity**: S

---

## Phase 5: Integration and Cross-Crate

### Task 5.1: Cross-Crate Correlation ID Test

**Files**: `tests/correlation.rs` (workspace-level integration test)

**Implement**: Verify that `session_id` and `trace_id` link nous-mcp memories to nous-otlp telemetry. Start both services (in-process). Store a memory via MCP with `session_id = "test-session-123"`. Send an OTLP log event with the same `session_id`. Query both DBs, assert the memory and the log event share the `session_id`. Query OTLP spans by `trace_id` from the memory, assert they exist.

**TDD sequence**:
1. **Red**: Boot `NousServer` (in-memory, MockEmbedding) and `OtlpServer` (in-memory). Call `memory_store` with `session_id = "sess-001"`, `trace_id = "trace-001"`. POST OTLP logs with `session_id = "sess-001"` and spans with `trace_id = "trace-001"`. Query memory DB: assert memory has session_id. Query OTLP DB: `query_logs("sess-001")` returns the log. `query_spans("trace-001")` returns the span. Assert the bridge works
2. **Green**: Construct both servers in the same test process. No network needed for MCP (direct handler calls). OTLP uses localhost HTTP
3. **Refactor**: Extract a `TestHarness` that boots both servers together for reuse in other integration tests

**Acceptance**:
- Same session_id appears in both databases
- Same trace_id appears in both databases
- No runtime coupling between the two servers

**Complexity**: M

---

### Task 5.2: E2E Test with Claude Code

**Files**: `tests/e2e.sh` (shell script, run manually)

**Implement**: Shell script that exercises the full production path:
1. Start `nous-mcp serve` (stdio transport, background process)
2. Start `nous-otlp serve` (HTTP on port 4318, background process)
3. Configure Claude Code MCP settings to point at `nous-mcp`
4. Run `claude -p "remember that we use uv for Python" --allowedTools memory_store`
5. Query `memory.db`: assert a memory with content containing "uv" exists
6. Query `otlp.db`: assert telemetry events with a matching session_id exist
7. Clean up: kill both servers, remove test DBs

This test requires Claude Code installed and configured. Mark as manual/CI-excluded.

**TDD sequence**:
1. **Red**: Write the shell script with assertions via `sqlite3` CLI queries. Run it, expect failures until both servers are implemented
2. **Green**: Requires all prior phases complete. Once servers work, the script should pass end-to-end
3. **Refactor**: Add error handling (trap + cleanup on failure). Add `--verbose` flag for debugging

**Acceptance**:
- Memory stored via Claude Code MCP is queryable in memory.db
- Telemetry from the same session exists in otlp.db
- Correlation IDs match across both databases

**Complexity**: L

---

### Task 5.3: sqlite-vec Integration (Deferred)

**Files**: `crates/nous-core/src/db.rs`, `crates/nous-core/build.rs`

**Implement**: When upstream publishes a fixed `sqlite-vec` crate (>= 0.1.10-alpha.4 or 0.1.10), uncomment the dependency in `nous-core/Cargo.toml` and:
1. Create `memory_vecs` vec0 virtual table in migrations
2. Replace fallback BLOB-based cosine similarity with vec0 KNN queries
3. Update `search_semantic` to use `WHERE embedding MATCH ? ORDER BY distance LIMIT ?`
4. Update `store_chunks` to insert into vec0 table
5. Update `delete_chunks` to delete from vec0 table

**Fallback build.rs approach** (if crate stays broken): Vendor `sqlite-vec.c` and `sqlite-vec.h` from the GitHub repo. Add a `build.rs` that compiles the C extension via `cc::Build`. Load at runtime via `rusqlite::Connection::load_extension`. This avoids the broken crate packaging entirely.

**TDD sequence**:
1. **Red**: With vec0 table available, store a memory with embedding. Run KNN query `SELECT * FROM memory_vecs WHERE embedding MATCH ? LIMIT 5`, assert result returned. Compare ranking with fallback brute-force, assert identical top-K ordering
2. **Green**: Switch from fallback table to vec0. Update insert/delete/query paths
3. **Refactor**: Remove fallback BLOB table code. Clean up feature flags

**Acceptance**:
- vec0 KNN query returns correct nearest neighbors
- Insert/delete work without sqlite-vec #150 edge cases (use INSERT, not INSERT OR REPLACE)
- Performance improvement over brute-force for > 1000 chunks

**Complexity**: M

---

### Task 5.4: CI Configuration

**Files**: `.github/workflows/ci.yml` or `.gitlab-ci.yml`

**Implement**: CI pipeline with: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (excludes `#[ignore]` tests that need model downloads), `cargo build --release`. Cache `~/.cargo/registry` and `target/`. Run on push to main and on MR/PR.

**TDD sequence**:
1. **Red**: Push to trigger CI. Expect format/lint/test stages. Intentionally break formatting, assert CI fails
2. **Green**: Write CI config with the four stages. Set `RUSTFLAGS="-D warnings"` for clippy
3. **Refactor**: Add a separate nightly job that runs `#[ignore]` tests with model downloads

**Acceptance**:
- CI runs on every push to main
- Format, lint, test, build stages all pass
- Ignored tests are excluded from default CI run

**Complexity**: S
