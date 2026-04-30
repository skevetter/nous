# 06: P5-P7 Features — Inventory, Memory, Agent Lifecycle

**Initiative:** INI-076  
**Status:** Draft  
**Author:** Technical Writer (paseo agent)  
**Date:** 2026-04-29

---

## Table of Contents

1. [Goals](#1-goals)
2. [Non-Goals](#2-non-goals)
3. [Inventory (P5)](#3-inventory-p5)
4. [Memory (P6) — Existing Core Feature](#4-memory-p6--existing-core-feature)
5. [Agent Lifecycle & Versioning (P7)](#5-agent-lifecycle--versioning-p7)
6. [Dependencies](#6-dependencies)
7. [Open Questions](#7-open-questions)

---

## 1. Goals

**P5 — Inventory:**

- Track all artifacts produced or consumed by agents — worktrees, rooms, schedules, branches, files, Docker images, and binaries — in a single queryable registry
- Tag-based discovery: find all artifacts tagged `production` or owned by a given agent without joining multiple tables
- Defined artifact lifecycle: `registered → active → archived → deleted`
- Ownership transfer when an agent deregisters — artifacts either transfer to a named successor or become orphaned (queryable but unowned)
- Namespace scoping so cross-team artifact enumeration is impossible without explicit access
- MCP tools and CLI commands consistent with the patterns in `docs/design/03-api-interfaces.md`

**P6 — Memory (existing core feature):**

- Nous's core value proposition: persistent, searchable, structured memory that survives across sessions and agent restarts
- Three complementary search modes already operational: FTS5 keyword (BM25), semantic KNN via sqlite-vec, and Hybrid (Reciprocal Rank Fusion)
- Planned extensions: cross-memory synthesis, temporal validity enforcement, multi-agent sharing, importance decay, and richer relationship types

**P7 — Agent Lifecycle & Versioning:**

- Version tracking per agent: each deployment records hashes of the skills and config it loaded, so skill drift is detectable at any later point
- Upgrade detection: compare the current skill hash against the latest known hash and flag agents running outdated code
- Template instantiation: create a new agent from a named template with optional per-instance config overrides
- Explicit forced re-read mechanism to address the stale-skill problem — old skill versions cause incorrect behavior in production

---

## 2. Non-Goals

- Artifact content storage: the registry tracks metadata and paths, not file bytes
- Cross-namespace artifact queries: namespace boundaries are hard; agents in namespace `A` cannot enumerate artifacts in namespace `B`
- Binary diff or versioning of artifact content (e.g., tracking file change history)
- Memory replication across Nous instances (multi-node sync is a future concern)
- Agent process management: the registry tracks logical agents; process lifecycle belongs to the Paseo runtime
- Automatic skill download or hot-reload: the upgrade path notifies agents, but loading new skill content requires the agent to re-read its skill files
- Garbage collection of deleted artifacts from disk: `artifact_deregister` removes the DB record; filesystem cleanup is the caller's responsibility

---

## 3. Inventory (P5)

### 3.1 Overview

The Inventory feature adds an `artifacts` table to the Nous SQLite database — the same database that stores memories, rooms, and tasks (see `docs/design/02-data-layer.md`). All writes go through `WriteChannel`; reads use `ReadPool`. The MCP server (see `docs/design/03-api-interfaces.md`) exposes five tools; the `nous inventory` CLI subcommand mirrors them.

```
┌──────────────────────────────────────────────────────────────────┐
│ Nous Process                                                     │
│                                                                  │
│  ┌──────────────────────────┐   ┌──────────────────────────────┐ │
│  │  MCP Server              │   │  CLI (nous inventory …)      │ │
│  │  artifact_register       │   │  nous inventory register     │ │
│  │  artifact_list           │   │  nous inventory list         │ │
│  │  artifact_update         │   │  nous inventory search       │ │
│  │  artifact_search         │   │  nous inventory archive      │ │
│  │  artifact_deregister     │   │  nous inventory deregister   │ │
│  └────────────┬─────────────┘   └──────────────┬───────────────┘ │
│               │                                  │                │
│               └─────────────┬────────────────────┘               │
│                             ▼                                     │
│                     ArtifactService                               │
│                     register()                                    │
│                     list()                                        │
│                     search()     ◄── tag + type + namespace       │
│                     archive()                                     │
│                     transfer()   ◄── on agent deregister          │
│                             │                                     │
│              ┌──────────────┴──────────────────┐                 │
│              ▼                                  ▼                 │
│        WriteChannel                         ReadPool              │
│        (INSERT/UPDATE)                      (SELECT)              │
└──────────────────────────────────────────────────────────────────┘
                             │
                             ▼
                   SQLite (WAL mode)
                   ~/.cache/nous/memory.db
                   ┌──────────────────────┐
                   │      artifacts       │
                   └──────────────────────┘
```

### 3.2 Data Model

IDs follow the UUIDv7 convention (see `docs/design/01-system-architecture.md`). The `metadata` column stores arbitrary JSON for artifact-type-specific fields (e.g., Docker image digest, branch HEAD SHA). The `tags` column stores a JSON array of lowercase string labels.

```sql
CREATE TABLE IF NOT EXISTS artifacts (
    id             TEXT NOT NULL PRIMARY KEY,      -- UUIDv7
    name           TEXT NOT NULL,
    type           TEXT NOT NULL CHECK (type IN (
                       'worktree', 'room', 'schedule', 'branch',
                       'file', 'docker-image', 'binary'
                   )),
    owner_agent_id TEXT REFERENCES agents(id)
                       ON DELETE SET NULL,
    namespace      TEXT NOT NULL DEFAULT 'default',
    path           TEXT,                           -- filesystem or logical path
    status         TEXT NOT NULL DEFAULT 'active'
                       CHECK (status IN ('active', 'archived', 'deleted')),
    metadata       TEXT,                           -- JSON, artifact-type-specific
    tags           TEXT NOT NULL DEFAULT '[]',     -- JSON array of strings
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived_at    TEXT                            -- set on → archived
);

CREATE INDEX IF NOT EXISTS idx_artifacts_owner
    ON artifacts(owner_agent_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_namespace_type
    ON artifacts(namespace, type);
CREATE INDEX IF NOT EXISTS idx_artifacts_status
    ON artifacts(status);
CREATE INDEX IF NOT EXISTS idx_artifacts_name
    ON artifacts(name);
```

| Column | Type | Notes |
|--------|------|-------|
| `id` | `TEXT` (UUIDv7) | Globally unique; lexicographically sortable by creation time |
| `name` | `TEXT` | Human-readable identifier, unique within `(namespace, type)` by convention |
| `type` | `TEXT` enum | `worktree`, `room`, `schedule`, `branch`, `file`, `docker-image`, `binary` |
| `owner_agent_id` | `TEXT` FK → `agents.id` | Agent that registered this artifact; `NULL` if owner deregistered without transfer |
| `namespace` | `TEXT` | Org hierarchy scope (e.g., team ID); default `'default'` |
| `path` | `TEXT` | Filesystem path or logical path (e.g., `~/.paseo/worktrees/abc123/`) |
| `status` | `TEXT` enum | `active` → `archived` → `deleted` |
| `metadata` | `TEXT` (JSON) | Type-specific fields: branch HEAD SHA, Docker digest, file MIME type, etc. |
| `tags` | `TEXT` (JSON array) | Free-form lowercase labels, e.g. `["production","api","team-a"]` |
| `archived_at` | `TEXT` | ISO-8601 timestamp set when status transitions to `archived` |

Tag filtering uses SQLite's `json_each()` function to shred the JSON array at query time:

```sql
-- Find all production artifacts owned by agent abc-123
SELECT * FROM artifacts
WHERE owner_agent_id = 'abc-123'
  AND EXISTS (
      SELECT 1 FROM json_each(tags) WHERE value = 'production'
  )
  AND status = 'active';
```

### 3.3 Lifecycle

```
artifact_register ──► active ──► archived ──► deleted
                                    │
                         archived_at set; filesystem
                         unchanged (caller's responsibility)
```

| Transition | Trigger | Side-effects |
|------------|---------|--------------|
| `→ active` | `artifact_register` call | DB insert; `status = 'active'` |
| `active → archived` | `artifact_archive` call or owner deregisters | DB UPDATE; `archived_at` set to current UTC timestamp |
| `archived → deleted` | `artifact_deregister` call | DB DELETE; filesystem cleanup is caller's responsibility |
| `active → archived` (auto) | Owner agent deregisters with `transfer_to = null` | All active artifacts owned by the deregistering agent move to `archived`; `owner_agent_id` set to `NULL` |

Once an artifact is `deleted` (DB row removed), it cannot be restored. Callers should use `archived` as the resting state for artifacts that are no longer in use but may need audit history.

### 3.4 Tag-Based Discovery

Tags are free-form lowercase strings stored as a JSON array in `artifacts.tags`. `artifact_search` filters by one or more tag values; an artifact matches if it carries **all** requested tags (AND semantics).

```json
// Example artifact tags
{"tags": ["production", "api", "team-a"]}
{"tags": ["staging", "worker", "team-b"]}
```

Common tag conventions:

| Tag | Meaning |
|-----|---------|
| `production` | Artifact deployed in or representing a production workload |
| `staging` / `dev` | Environment designators |
| `team-<id>` | Owning team, for cross-agent discovery within a namespace |
| `task-<uuid>` | Links artifact to a specific task (same pattern as room topics) |
| `ephemeral` | Short-lived artifact; safe to archive after 24 hours of inactivity |

Tag mutation: `artifact_update` replaces the entire `tags` array atomically via `WriteChannel`. There is no append-only patch; callers read the current tags, merge locally, and write the merged array.

### 3.5 Ownership and Transfer

When an agent deregisters via `agent_deregister`, the `ArtifactService` runs a transfer pass before removing the agent row:

```
agent_deregister(agent_id, transfer_to?)
    │
    ├── if transfer_to IS NOT NULL:
    │       UPDATE artifacts SET owner_agent_id = transfer_to
    │       WHERE owner_agent_id = agent_id AND status = 'active'
    │
    └── if transfer_to IS NULL:
            UPDATE artifacts SET owner_agent_id = NULL, status = 'archived',
                   archived_at = now()
            WHERE owner_agent_id = agent_id AND status = 'active'
```

Orphaned artifacts (`owner_agent_id IS NULL`, `status = 'active'`) are a valid state — they are fully queryable and can be claimed by a new owner via `artifact_update`. The `artifact_list` tool accepts `--orphaned` to enumerate them.

The `agents` table uses `ON DELETE SET NULL` on the `owner_agent_id` FK so that a hard-delete of an agent row (bypassing the service layer) does not cascade-delete artifacts.

### 3.6 Namespace Scoping

The `namespace` column on `artifacts` mirrors the namespace concept from `docs/design/05-features-p2-p4.md §4.3` (org hierarchy). Every MCP tool and CLI command scopes queries to the calling agent's namespace unless the caller is a root-level agent.

Namespace values are short strings, typically the team ID (e.g., `59e2`) or a compound path (`bf91/59e2`). The default namespace is `'default'` for backward compatibility.

Rules:
- `artifact_register` sets `namespace` from the calling agent's registration record; callers cannot override to a different namespace.
- `artifact_list` and `artifact_search` filter by `namespace = caller_namespace` unless `--global` is passed (root agents only).
- `artifact_update` and `artifact_archive` reject requests where `caller_namespace != artifact.namespace`.

### 3.7 API Surface

#### MCP Tools

| Tool | Parameters | Returns |
|------|-----------|---------|
| `artifact_register` | `name`, `type`, `path?`, `tags?`, `metadata?` | `{ id, name, type, status, created_at }` |
| `artifact_list` | `type?`, `status?`, `owner_agent_id?`, `orphaned?`, `limit?` | Array of artifact objects |
| `artifact_update` | `id`, `name?`, `path?`, `tags?`, `metadata?` | `{ updated: bool }` |
| `artifact_search` | `tags[]`, `type?`, `status?`, `namespace?`, `limit?` | Array of artifact objects |
| `artifact_deregister` | `id`, `hard?` | `{ deleted: bool }` — soft-delete (archived) unless `hard=true` |

`artifact_register` and `artifact_update` flow through `WriteChannel`. `artifact_list` and `artifact_search` use `ReadPool`. `artifact_deregister` with `hard=false` goes through `WriteChannel` (UPDATE to `deleted`); with `hard=true` it issues a DELETE.

#### CLI Commands

```
nous inventory register --name <name> --type <type> [--path <path>] [--tags tag1,tag2] [--metadata '{"key":"val"}']
nous inventory list [--type <type>] [--status active|archived|deleted] [--owner <agent_id>] [--orphaned]
nous inventory search --tag <tag> [--tag <tag>...] [--type <type>] [--namespace <ns>]
nous inventory archive <id>
nous inventory deregister <id> [--hard]
nous inventory show <id>
```

---

## 4. Memory (P6) — Existing Core Feature

### 4.1 Architecture Overview

Memory is the core of Nous. Every agent call to `mem_save`, `mem_search`, or `mem_context` goes through the same SQLite database at `~/.cache/nous/memory.db`. The memory subsystem spans three source files in `crates/nous-core/src/`:

| File | Role |
|------|------|
| `db.rs` | Schema migrations, `MemoryDb` struct, CRUD operations |
| `channel.rs` | `WriteChannel` (serial write worker) and `ReadPool` (concurrent reads) |
| `search.rs` | FTS5 (`search_fts`), vector KNN (`search_semantic`), hybrid RRF (`fuse_rrf`), and context pagination (`context`) |
| `chunk.rs` | `Chunker` struct — splits content into overlapping token windows |
| `classify.rs` | `CategoryClassifier` — embedding-based category assignment |
| `embed.rs` | `EmbeddingBackend` trait, `OnnxBackend` implementation (ONNX runtime via `ort`) |

```
  Agent (MCP call or CLI)
        │
        ▼
  Handler (tools.rs / commands.rs)
        │
        ├── write op ──► WriteChannel ──► write_worker (serial)
        │                                       │
        │                                  SQLite transaction
        │                                  BATCH_LIMIT=32 ops/tx
        │
        └── read op ──► ReadPool ──► spawn_blocking(query)
                        (up to N parallel connections)
```

All operations on memories, tags, relationships, chunks, and embeddings share a single WAL-mode SQLite file. The `WriteChannel` serialises all mutations; the `ReadPool` allows concurrent reads via read-only connections opened with `PRAGMA query_only = ON`.

### 4.2 Data Model

All DDL lives in the `MIGRATIONS` slice in `crates/nous-core/src/db.rs`. The schema is append-only (new columns via `ALTER TABLE ADD COLUMN` in migration functions) and applied at startup.

**Core tables:**

```sql
-- memories: primary store (db.rs:39-58)
CREATE TABLE IF NOT EXISTS memories (
    id          TEXT PRIMARY KEY,           -- UUIDv7
    title       TEXT NOT NULL,
    content     TEXT NOT NULL,
    memory_type TEXT NOT NULL CHECK(memory_type IN
                    ('decision','convention','bugfix','architecture','fact','observation')),
    source      TEXT,
    importance  TEXT NOT NULL DEFAULT 'moderate'
                    CHECK(importance IN ('low','moderate','high')),
    confidence  TEXT NOT NULL DEFAULT 'moderate'
                    CHECK(confidence IN ('low','moderate','high')),
    workspace_id INTEGER REFERENCES workspaces(id),
    session_id  TEXT,
    trace_id    TEXT,
    agent_id    TEXT,
    agent_model TEXT,
    valid_from  TEXT,
    valid_until TEXT,
    archived    INTEGER NOT NULL DEFAULT 0,
    category_id INTEGER REFERENCES categories(id),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- tags + memory_tags M2M (db.rs:60-69)
CREATE TABLE IF NOT EXISTS tags (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS memory_tags (
    memory_id TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    tag_id    INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (memory_id, tag_id)
);

-- relationships (db.rs:71-77)
CREATE TABLE IF NOT EXISTS relationships (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id     TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    target_id     TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    relation_type TEXT NOT NULL CHECK(relation_type IN
                      ('related','supersedes','contradicts','depends_on')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- access_log (db.rs:79-85)
CREATE TABLE IF NOT EXISTS access_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id   TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    accessed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    access_type TEXT NOT NULL,   -- 'recall', 'search', 'context'
    session_id  TEXT
);

-- memory_chunks (db.rs:95-104)
CREATE TABLE IF NOT EXISTS memory_chunks (
    id          TEXT PRIMARY KEY,           -- "{memory_id}:{chunk_index}"
    memory_id   TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content     TEXT NOT NULL,
    token_count INTEGER NOT NULL,
    model_id    INTEGER NOT NULL REFERENCES models(id),
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- categories (db.rs:29-37) — hierarchical via parent_id
CREATE TABLE IF NOT EXISTS categories (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL,
    parent_id   INTEGER REFERENCES categories(id),
    source      TEXT NOT NULL DEFAULT 'system',  -- 'system' | 'user' | 'agent'
    description TEXT,
    embedding   BLOB,
    threshold   REAL,
    created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE(name, parent_id)
);

-- vec0 virtual table — created dynamically at startup (db.rs:1354-1368)
-- CREATE VIRTUAL TABLE memory_embeddings USING vec0(
--     chunk_id TEXT PRIMARY KEY,
--     embedding float[{dimensions}]   -- dimensions from active ONNX model
-- );
```

**Seeded category tree** (15 top-level categories, each with 3-4 children):

`infrastructure`, `data-platform`, `ci-cd`, `security`, `tooling`, `observability`, `architecture`, `workflow`, `cost`, `languages`, `libraries`, `team`, `project`, `debugging`, `configuration`

The seeder uses `INSERT OR IGNORE` so re-running migrations is safe.

**`Supersedes` side-effect:** when `db.rs:relate_on` records a `supersedes` relationship from `source → target`, it immediately sets `target.valid_until = now()`, effectively marking the superseded memory as expired.

### 4.3 Write Path — WriteChannel

`WriteChannel` (`channel.rs:99-460`) serialises all mutations through a single `tokio::sync::mpsc` channel of capacity `CHANNEL_CAPACITY = 256`. A background Tokio task (`write_worker`) drains the channel in batches of up to `BATCH_LIMIT = 32` ops per SQLite transaction.

```
caller:
  WriteChannel.store(NewMemory)
      │
      ├── oneshot::channel() → (resp_tx, resp_rx)
      ├── mpsc::Sender::send(WriteOp::Store(memory, resp_tx))  ← async, backpressures at 256
      └── resp_rx.await                                        ← blocks until write_worker commits

write_worker (single Tokio task):
  loop:
    first = rx.recv().await          ← blocks until at least one op
    batch = [first]
    while batch.len() < 32:
        match rx.try_recv():
            Ok(op)  → batch.push(op)
            Err(_)  → break           ← channel empty, flush now
    spawn_blocking:
        tx = conn.unchecked_transaction()
        for op in batch: dispatch(op, &tx)
        tx.commit()
```

`WriteOp` variants (channel.rs:22-96):

| Variant | Operation |
|---------|-----------|
| `Store` | INSERT memory row + tags |
| `Update` | UPDATE memory fields (partial patch) |
| `Forget` | Soft-delete (`archived=1`) or hard-delete |
| `Unarchive` | Restore soft-deleted memory |
| `Relate` / `Unrelate` | INSERT / DELETE relationships row |
| `StoreChunks` | INSERT into `memory_chunks` + `memory_embeddings` |
| `DeleteChunks` | DELETE chunks and embeddings for a memory |
| `LogAccess` | INSERT into `access_log` |
| `CategorySuggest` | INSERT category + UPDATE memory.category_id |
| `CategoryDelete` / `CategoryRename` / `CategoryUpdate` | Mutate categories table |
| `CreateRoom` / `PostMessage` / … | Room operations (P0) |
| `CreateSchedule` / … | Schedule operations (P4) |

On process restart, `write_worker` transitions any `schedule_runs` with `status = 'running'` to `status = 'failed'` to prevent ghost run records.

### 4.4 Read Path — ReadPool

`ReadPool` (`channel.rs:682-962`) holds a fixed pool of read-only SQLite connections opened with `PRAGMA query_only = ON`. Access is guarded by a `Semaphore` matching the pool size; each `with_conn` call leases one connection, runs the query in `spawn_blocking`, then returns the connection to the pool.

```rust
// channel.rs:702-744
pub async fn with_conn<F, T>(&self, f: F) -> Result<T>
where F: FnOnce(&Connection) -> Result<T> + Send + 'static
{
    let permit = self.semaphore.acquire().await?;   // blocks if pool exhausted
    let conn = self.connections.lock().await.pop()?;
    let result = tokio::task::spawn_blocking(|| f(&conn)).await;
    self.connections.lock().await.push(conn);       // return to pool
    drop(permit);
    result
}
```

Pool size is configured at startup (default: 4 connections). All `ReadPool` methods (`list_rooms`, `get_room`, `list_messages`, `search_messages`) follow the same pattern: acquire, query, return.

### 4.5 FTS5 Full-Text Search

The `memories_fts` virtual table (db.rs:87-93) is a content-tracked FTS5 index over `title`, `content`, and `memory_type`:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
    title, content, memory_type,
    content='memories',
    content_rowid='rowid'
);
```

Three triggers keep it in sync with the `memories` table (db.rs:105-118):

| Trigger | Event | Action |
|---------|-------|--------|
| `memories_ai` | AFTER INSERT | INSERT into `memories_fts` |
| `memories_ad` | AFTER DELETE | `DELETE` shadow row from `memories_fts` |
| `memories_au` | AFTER UPDATE | `DELETE` old shadow row, INSERT new shadow row |

**Scoring:** `search_fts` (search.rs:164-217) uses `bm25(memories_fts, 10.0, 1.0, 0.5)` — title matches are weighted 10×, content 1×, memory_type 0.5×. The final rank multiplies BM25 by an importance weight: `high=3.0`, `moderate=2.0`, `low=1.0` (search.rs:79-85). Results are ordered by rank ascending (BM25 returns negative values; lower = better match).

**FTS query sanitisation:** `sanitize_fts_query` (search.rs:15-29) wraps tokens containing `-`, `:`, `.`, `/`, `\`, `@`, `#`, `!`, or `+` in double quotes. Without this, queries like `INI-076` fail with `"no such column: 076"` because FTS5 interprets the hyphen as a column-prefix operator.

### 4.6 Vector Search — sqlite-vec (vec0)

The `memory_embeddings` virtual table uses the sqlite-vec (`vec0`) extension, loaded via `crates/nous-core/src/sqlite_vec.rs`. It stores one float vector per chunk:

```sql
-- Created at startup; dimension matches the active ONNX model (db.rs:1362-1367)
CREATE VIRTUAL TABLE memory_embeddings USING vec0(
    chunk_id  TEXT PRIMARY KEY,
    embedding float[384]          -- 384 for BAAI/bge-small-en-v1.5
);
```

`search_semantic` (search.rs:219-315) queries via KNN:

```sql
SELECT chunk_id, distance
FROM memory_embeddings
WHERE embedding MATCH ?1   -- query blob (float32 LE bytes)
  AND k = ?2               -- knn_k = limit * 5 (over-fetch for filter)
ORDER BY distance;
```

The search then:
1. Maps `chunk_id` back to `memory_id` via a batch `SELECT FROM memory_chunks WHERE id IN (…)`.
2. Keeps only the best (smallest) distance per memory across all its chunks.
3. Applies `SearchFilters` (type, workspace, tags, etc.) by loading matching memories in a single `SELECT IN (…)`.
4. Scores each result as `(1.0 / (1.0 + distance)) * importance_weight`.

**Embedding backend:** `OnnxBackend` (`embed.rs:273-481`) loads an ONNX model via `ort` at startup. The default model is `BAAI/bge-small-en-v1.5` (384 dimensions), downloaded from Hugging Face Hub on first run or loaded from a local `--model-dir`. Mean-pooling over non-padding tokens followed by L2 normalisation produces the final embedding. The `batch_size` for inference is 32 texts per ONNX forward pass.

If the active model changes (different dimensions), `MemoryDb::reset_embeddings` drops and recreates the `memory_embeddings` table; existing chunk rows in `memory_chunks` persist but must be re-embedded.

### 4.7 Hybrid Search — Reciprocal Rank Fusion

`SearchMode::Hybrid` runs both `search_fts` and `search_semantic` independently, then merges their ranked lists using Reciprocal Rank Fusion (`fuse_rrf`, search.rs:641-683).

RRF formula for each memory: `score += 1.0 / (k + rank_position + 1)` for each list it appears in, where `k = 60.0`. Results appearing in both lists receive contributions from both. Final score is multiplied by `importance_weight`.

```
FTS rank list:        [A(1), B(2), C(3), D(4)]
Semantic rank list:   [C(1), A(2), E(3), B(4)]

RRF scores (k=60):
  A: 1/(61) + 1/(62) = 0.0164 + 0.0161 = 0.0325  ← high: in both lists
  C: 1/(63) + 1/(61) = 0.0159 + 0.0164 = 0.0323
  B: 1/(62) + 1/(64) = 0.0161 + 0.0156 = 0.0317
  D: 1/(64) = 0.0156
  E: 1/(63) = 0.0159

Merged order: A, C, B, E, D
```

Hybrid mode is the recommended default for agent workloads because it retrieves memories that are lexically relevant (exact term matches) and semantically similar (concept matches) without needing to tune per-query weights.

### 4.8 Chunking

`Chunker` (`chunk.rs:12-110`) splits memory content into overlapping windows for embedding. The default parameters come from the active model's record in the `models` table (`chunk_size=512`, `chunk_overlap=64`).

Algorithm:
1. Tokenise the text into whitespace-delimited word spans (byte-offset pairs).
2. If total word count ≤ `chunk_size`, return one chunk covering the full text.
3. Otherwise, advance by `step = chunk_size - chunk_overlap` words per window.
4. If the final chunk is shorter than `min_chunk = 32` words, merge it into the penultimate chunk.

Chunk IDs in the database are `"{memory_id}:{chunk_index}"` (db.rs:1081). This makes chunk deletion straightforward: the `DELETE FROM memory_embeddings WHERE chunk_id = ?1` call identifies the exact embedding rows without a join.

`store_chunks` (db.rs:1055-1102) writes chunk rows and embedding blobs atomically in the same `WriteChannel` transaction as the parent memory store.

### 4.9 Classification

`CategoryClassifier` (`classify.rs:10-138`) assigns a category to a memory at store time using cosine similarity against pre-computed category embeddings.

At startup `CategoryClassifier::new` loads all categories from the `categories` table and calls `embedder.embed()` for any that lack a stored embedding blob. The embeddings are cached in-process in a `HashMap<i64, (Category, Vec<f32>)>`.

Classification at call time (`classify.rs:30-53`):
1. Filter to top-level categories (`parent_id IS NULL`); find the one with cosine similarity > `threshold` (default 0.5).
2. If the top-level match has children, filter to those children and find the best child match.
3. Return the child ID if found, otherwise the top-level ID. Return `None` if no category exceeds the threshold.

Per-category thresholds override the default: set via `category_update(..., threshold=0.7)` for high-precision categories.

The classifier is refreshed via `classifier.refresh(db, embedder)` after any `CategorySuggest` write, so newly added categories are available immediately.

### 4.10 Planned Extensions

The following extensions are designed but not yet implemented:

**Cross-memory synthesis** — given a query or tag set, retrieve the N most relevant memories, pass their content to an LLM, and store the synthesised summary as a new memory with `memory_type='observation'` and relationships pointing back to the source memories. This enables progressive summarisation without discarding originals.

**Temporal validity enforcement** — a background Tokio task runs on a configurable interval and archives memories where `valid_until < now()` and `archived = 0`. Currently, `valid_until` is checked at query time via `SearchFilters.valid_only` (search.rs:497-500) but expired memories remain in the active set unless explicitly archived.

**Multi-agent memory sharing** — namespace-scoped sharing: memories with `workspace_id = shared_workspace` are visible to any agent whose registration record includes that workspace in its allowed list. Currently, `workspace_id` is set per-memory but the access-control layer is not enforced.

**Memory importance decay** — reduce importance from `high → moderate → low` for memories that have not been accessed (no `access_log` entry) for a configurable number of days. The `access_log` table already records every recall, search, and context operation; decay logic needs only a scheduled sweep over `most_accessed` counts.

**Richer relationship types** — add `implements`, `references`, `extends` to the `relation_type` CHECK constraint. Currently the four types are `related`, `supersedes`, `contradicts`, `depends_on`. New types require a migration adding the values to the CHECK and handling them in `relate_on` / `unrelate_on`.

---

## 5. Agent Lifecycle & Versioning (P7)

### 5.1 Overview

The `org-management` plugin (`bf91-59e2` codebase) already handles basic agent CRUD — register, deregister, heartbeat, list, lookup. P7 adds two new concerns on top of that foundation:

1. **Versioning** — track which version of each skill an agent loaded when it started. A version is a content hash, not a semver string. Hash changes when the skill file changes.
2. **Upgrade paths** — detect agents running stale skill versions and provide a structured mechanism to force them to re-read their skills.

The stale-skill problem is the primary driver: an agent spawned two hours ago may be running a skill that has since been updated. Without explicit version tracking, there is no way to detect this condition or trigger a re-read.

```
  agent spawns
       │
       ▼
  reads skill files
  computes SHA-256 of each skill content
       │
       ▼
  agent_register(agent_id, skills=[{name, hash}], config_hash=SHA-256(config))
       │
       ▼
  agent_versions row inserted
       │
       ▼
  agent runs — heartbeat includes current_version_id
       │
  skill file updated on disk (new deployment)
       │
       ▼
  upgrade detector: compare stored hash vs sha256(current file)
       │
       ├── match → agent is current
       └── mismatch → set agents.upgrade_available = 1
                       post notification to agent's room
                       agent calls: skill_reload / restart
```

### 5.2 Data Model

Two new tables extend the existing `agents` table from `org-management`:

```sql
-- agent_versions: records what each agent loaded at startup
CREATE TABLE IF NOT EXISTS agent_versions (
    id           TEXT NOT NULL PRIMARY KEY,  -- UUIDv7
    agent_id     TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    skill_hash   TEXT NOT NULL,              -- SHA-256 of concatenated skill file contents
    config_hash  TEXT NOT NULL,              -- SHA-256 of effective config.toml at startup
    skills_json  TEXT NOT NULL DEFAULT '[]', -- JSON array: [{name, path, hash}]
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
CREATE INDEX IF NOT EXISTS idx_agent_versions_agent
    ON agent_versions(agent_id);

-- agent_templates: reusable blueprints for spawning agents
CREATE TABLE IF NOT EXISTS agent_templates (
    id             TEXT NOT NULL PRIMARY KEY,  -- UUIDv7
    name           TEXT NOT NULL UNIQUE,
    type           TEXT NOT NULL,              -- e.g. 'worker', 'reviewer', 'monitor'
    default_config TEXT NOT NULL DEFAULT '{}', -- JSON config merged with instance overrides
    skill_refs     TEXT NOT NULL DEFAULT '[]', -- JSON array of skill file paths or names
    created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
```

**Additions to the existing `agents` table:**

```sql
ALTER TABLE agents ADD COLUMN current_version_id TEXT REFERENCES agent_versions(id);
ALTER TABLE agents ADD COLUMN upgrade_available   INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agents ADD COLUMN template_id         TEXT REFERENCES agent_templates(id);
```

The `agents` table already carries `status`, `last_seen_at`, `namespace`, and registration metadata from `org-management`. These three new columns integrate versioning without replacing the existing schema.

| Column | Notes |
|--------|-------|
| `current_version_id` | FK to the `agent_versions` row recorded at last startup; NULL until first `agent_register` with skills |
| `upgrade_available` | Set to `1` by the upgrade detector; cleared to `0` when the agent re-registers with a new version |
| `template_id` | If the agent was instantiated from a template, FK to `agent_templates`; NULL for manually registered agents |

### 5.3 Version Tracking

When an agent starts, it reads its skill files from disk and computes:

```
skill_hash   = SHA-256(sorted(skill_file_contents))
config_hash  = SHA-256(effective config.toml after env-var overrides)
skills_json  = [{name: "...", path: "...", hash: SHA-256(file_content)}, ...]
```

It then calls `agent_register` (or `agent_heartbeat` if already registered) with these values. The server:

1. Inserts a new `agent_versions` row.
2. Updates `agents.current_version_id` to point at the new row.
3. Clears `agents.upgrade_available = 0` (the agent just re-read its skills).

Version history is preserved — old `agent_versions` rows are not deleted on re-registration. This allows a rollback query: `SELECT * FROM agent_versions WHERE agent_id = ? ORDER BY created_at DESC` returns the version timeline.

The `skills_json` column stores per-skill hashes so the upgrade detector can report which specific skill changed, not just that some skill changed.

### 5.4 Upgrade Detection and the Stale-Skill Problem

**The stale-skill problem:** When skill files change on disk (e.g., after a deployment), agents that loaded those files earlier continue running with the old content until they restart or explicitly re-read the files. Old skill versions cause incorrect behavior: outdated tool schemas, deprecated protocols, removed commands.

**Upgrade detection** runs as a background task on a configurable interval (default: every 5 minutes). For each active agent with a `current_version_id`:

1. Load `agent_versions.skills_json` for the current version.
2. For each skill, read the file at the recorded path and compute `SHA-256`.
3. If any hash differs from the stored hash: set `agents.upgrade_available = 1` and post a notification to the agent's registered room (if any).

The notification message format:
```json
{
  "event": "skill_upgrade_available",
  "agent_id": "<id>",
  "changed_skills": ["skill-name-1", "skill-name-2"],
  "detected_at": "2026-04-29T14:00:00.000Z"
}
```

**Forced re-read mechanism:** `agent_notify_upgrade` MCP tool sends the notification immediately (bypassing the background interval). Agents are expected to handle the `skill_upgrade_available` event by calling the Paseo skill tool to re-load the updated skill content and then calling `agent_register` again to record the new version hashes.

This is advisory, not enforcement — the Nous server cannot force an agent to reload its skills. The contract requires agent authors to subscribe to their coordination room and handle upgrade notifications. Agents that ignore the notification will keep `upgrade_available = 1` and appear in `agent_list --outdated` output.

**`agent_list --outdated`** returns all agents where `upgrade_available = 1`, along with the list of changed skills from the most recent upgrade detection run.

### 5.5 Template Instantiation

`agent_template_create` registers a named template. `agent_instantiate` creates a new agent from a template with optional per-instance config overrides:

```
agent_instantiate(template_id, name?, config_overrides?)
    │
    ├── SELECT * FROM agent_templates WHERE id = template_id
    ├── effective_config = merge(template.default_config, config_overrides)
    ├── skill_refs = template.skill_refs
    └── INSERT INTO agents (..., template_id, status='active')
        INSERT INTO agent_versions (agent_id, skill_hash=NULL, ...)
        RETURN new agent_id
```

The template stores `skill_refs` as a JSON array of skill file paths or registered skill names. The instantiated agent is expected to read those skills at startup and call `agent_register` with the computed hashes.

Config merging follows JSON merge-patch (RFC 7396): `config_overrides` keys overwrite template defaults; absent keys inherit the template value.

Example template:

```json
{
  "name": "code-reviewer",
  "type": "reviewer",
  "default_config": {
    "provider": "claude/sonnet",
    "mode": "bypassPermissions",
    "review_depth": "standard"
  },
  "skill_refs": [
    "org:code-reviewer",
    "superpowers:systematic-debugging"
  ]
}
```

### 5.6 Status Management

Agent status values extend the existing `org-management` status model:

```
registered
    │
    ▼
 active ──────────────────────────► done
    │            ▲                   │
    │            │ unblock           ▼
    ▼            │                closed
 blocked ────────┘
    │
    ▼
 error ──────────────────────────► closed
    │
    ▼
  idle  (agent alive but between tasks)
```

| Status | Meaning | Typical trigger |
|--------|---------|-----------------|
| `active` | Agent running and processing a task | `agent_register` or task assignment |
| `idle` | Agent alive, no current task | Task completion, waiting for next assignment |
| `blocked` | Agent waiting on another agent or resource | Dependency not satisfied |
| `done` | Current task complete; agent may be archived | Task `→ done` transition |
| `error` | Agent encountered an unrecoverable error | Exception or timeout in task execution |
| `closed` | Agent deregistered or archived | `agent_deregister` |

Transitions are set via `agent_update_status(agent_id, status, reason?)`. The `reason` field is stored as `metadata` JSON and surfaced in `agent_inspect` output. Heartbeats (`agent_heartbeat`) update `last_seen_at` without changing status.

### 5.7 Rollback

Rollback reverts an agent to the config and skill set it ran at a previous version:

```
agent_rollback(agent_id, version_id)
    │
    ├── SELECT skills_json, config_hash FROM agent_versions WHERE id = version_id
    ├── verify version_id.agent_id == agent_id
    ├── UPDATE agents SET current_version_id = version_id, upgrade_available = 0
    └── post notification to agent room:
        {"event": "rollback_requested", "target_version_id": version_id,
         "skills_json": [...], "config_hash": "..."}
```

As with upgrades, the rollback is advisory. The server records the intent and notifies the agent; the agent must restart using the specified skill versions. The `skills_json` array in the notification includes the exact file paths and expected hashes, so the agent knows precisely which versions to load.

If the skill files at the recorded paths no longer exist (e.g., deleted after a deployment), the rollback notification includes `"missing_skills": ["path/to/skill.md"]` and the agent must handle this error — Nous cannot reconstruct deleted files.

### 5.8 API Surface

#### MCP Tools

| Tool | Parameters | Returns |
|------|-----------|---------|
| `agent_register` | `agent_id`, `name`, `type`, `namespace?`, `skills?` (array `{name,path,hash}`), `config_hash?`, `template_id?` | `{ agent_id, version_id }` |
| `agent_update_status` | `agent_id`, `status`, `reason?` | `{ updated: bool }` |
| `agent_heartbeat` | `agent_id`, `metadata?` | `{ last_seen_at }` |
| `agent_deregister` | `agent_id`, `transfer_artifacts_to?` | `{ deregistered: bool }` |
| `agent_list` | `status?`, `namespace?`, `outdated?`, `limit?` | Array of agent objects |
| `agent_inspect` | `agent_id` | Full agent object with version history |
| `agent_versions` | `agent_id` | Array of `agent_versions` rows, newest first |
| `agent_rollback` | `agent_id`, `version_id` | `{ rollback_requested: bool }` |
| `agent_notify_upgrade` | `agent_id` | `{ notified: bool }` — immediate upgrade notification |
| `agent_template_create` | `name`, `type`, `default_config?`, `skill_refs?` | `{ template_id }` |
| `agent_template_list` | `type?` | Array of template objects |
| `agent_instantiate` | `template_id`, `name?`, `config_overrides?` | `{ agent_id, version_id }` |

#### CLI Commands

```
nous agent register --name <name> --type <type> [--namespace <ns>] [--skill <path>...]
nous agent list [--status <status>] [--outdated] [--namespace <ns>]
nous agent inspect <id>
nous agent versions <id>
nous agent status <id> <status> [--reason <text>]
nous agent deregister <id> [--transfer-to <agent_id>]
nous agent rollback <id> --version <version_id>
nous agent heartbeat <id>

nous template create --name <name> --type <type> [--config <json>] [--skill <ref>...]
nous template list [--type <type>]
nous template instantiate <template_id> [--name <name>] [--override <key=value>...]
```

---

## 6. Dependencies

| Feature | Depends on | Notes |
|---------|-----------|-------|
| P5 Inventory | `agents` table (P3 Org Hierarchy, `docs/design/05-features-p2-p4.md §4`) | `artifacts.owner_agent_id` is a FK to `agents.id`; agents must exist before artifacts can be owned |
| P5 Inventory | `WriteChannel` / `ReadPool` (`docs/design/02-data-layer.md`) | All artifact mutations go through `WriteChannel`; reads through `ReadPool` |
| P5 Inventory | UUIDv7 ID generation (`docs/design/01-system-architecture.md`) | `artifacts.id` must be UUIDv7 |
| P5 Inventory | MCP server (`docs/design/03-api-interfaces.md`) | Five new tools registered in `crates/nous-cli/src/server.rs` |
| P6 Memory (extensions) | Schedule Engine (P4, `docs/design/05-features-p2-p4.md §5`) | Temporal validity enforcement and importance decay require a background scheduled sweep |
| P6 Memory (extensions) | ONNX embedding backend (`crates/nous-core/src/embed.rs`) | Cross-memory synthesis requires embeddings to find related memories |
| P7 Agent Versioning | `agents` table (P3) | Three new columns added to the existing `agents` table |
| P7 Agent Versioning | Rooms (P0, `docs/design/04-features-p0-p1.md §4`) | Upgrade notifications are posted to the agent's coordination room via `post_message` |
| P7 Agent Versioning | `WriteChannel` / `ReadPool` | Agent version writes go through `WriteChannel` |
| P7 Agent Templates | `agents` table (P3) | `agents.template_id` FK references `agent_templates.id` |

---

## 7. Open Questions

**P5 — Inventory**

1. **Tag query semantics** — AND vs OR: the current design requires all requested tags to be present (AND). Should `artifact_search` support OR semantics with an explicit `--any-tag` flag? AND is stricter and more useful for filtering `production` + `team-a`; OR is better for broad discovery.

2. **Artifact deduplication** — nothing prevents two agents from registering artifacts with the same `(namespace, name, type)` tuple. Should there be a uniqueness constraint, or is the intent to allow multiple versions of an artifact (e.g., two branches with the same name)?

3. **Artifact FTS** — should artifact names and metadata be indexed in an FTS5 table for full-text search, similar to `memories_fts`? The current design uses exact tag matching and SQL `LIKE` on `name`. A separate `artifacts_fts` virtual table would support freetext queries but adds maintenance complexity.

**P6 — Memory**

4. **Synthesis LLM target** — which model should cross-memory synthesis call? It should use the same `EmbeddingBackend` + a chat model; the design does not yet specify how the synthesis model is configured or which API it calls.

5. **Importance decay schedule** — what is the decay period before `high → moderate` and `moderate → low`? Candidate: 30 days without an `access_log` entry for `high`, 14 days for `moderate`. These values affect recall quality and need empirical validation.

6. **Multi-agent namespace boundary enforcement** — the current `workspace_id` approach is permissive: any agent that knows a `workspace_id` can query its memories. How should access be enforced? Options: FK to an `agent_workspace_access` table, or a per-memory ACL column.

**P7 — Agent Lifecycle & Versioning**

7. **Upgrade enforcement vs. advisory** — the current design is advisory only. Should there be a mechanism to refuse `agent_heartbeat` for agents running skills older than N days, forcing a restart? This prevents indefinitely stale agents but may cause unexpected outages.

8. **Template versioning** — `agent_templates` has no version history. If a template changes after agents are already instantiated from it, those agents silently diverge from the new template. Should templates be immutable (new template required for each change), or should `agent_instantiate` record the template state at spawn time?

9. **Skill path portability** — `skills_json` stores absolute filesystem paths. If Nous is moved to a different machine or the skills directory is renamed, the stored paths become invalid and rollback/upgrade detection breaks. Should paths be stored relative to a configurable `skills_root`?
