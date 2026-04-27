# Functions Module — Design Document

## 1. Summary
The Functions Module adds reusable, named, versioned actions to Nous. A function encapsulates a
unit of work — a bash script, Python program, TypeScript module, compiled Rust WASM binary, or MCP
tool invocation — that schedules reference by ID instead of embedding action details inline.

**Relationship to the scheduler.** The scheduler workstream (designed in parallel) introduces a
`schedules` table with `action_type` + `action_payload` columns for inline action definitions. The
Functions Module adds a `functions` table and a `function_ref_id` foreign key on `schedules`,
enabling one function to back many schedules without copy-paste duplication.

**Scope estimate.** ~1,000–1,500 new lines across `nous-core` (schema, CRUD, execution engine,
WriteOp extensions) and `nous-mcp` (7 new MCP tools, config additions). The `nous-shared` crate
gains a `FunctionId` type via the existing `define_id!` macro.

## 2. Current Architecture
Nous is a Rust MCP memory server organized into four workspace crates:

| Crate | Role | Key types |
|-------|------|-----------|
| `nous-shared` | Shared types, SQLite helpers (`open_connection`, `run_migrations`), XDG path resolution, ID types via `define_id!` macro | `MemoryId`, `SessionId`, `TraceId`, `SpanId`, `NousError` |
| `nous-core` | Memory database, write channel, read pool, embedding backend, chunker, category classifier | `MemoryDb`, `WriteChannel`, `WriteOp`, `ReadPool`, `EmbeddingBackend`, `Chunker` |
| `nous-mcp` | 15 MCP tools via `rmcp`, stdio + HTTP transport, CLI entry point, configuration | `NousServer`, `Config`, tool param structs with `JsonSchema` derive |
| `nous-otlp` | OTLP receiver on a separate SQLite database, ingests logs/traces/metrics | `OtlpDb`, log/trace/metric tables |

**Write path.** All mutations flow through a `WriteChannel` — an `mpsc::Sender<WriteOp>` that
serializes writes through a single connection. The receiver batches up to `BATCH_LIMIT` (32)
operations per transaction. Each `WriteOp` variant carries a `oneshot::Sender` for the response.
Current variants: `Store`, `Update`, `Forget`, `Relate`, `Unrelate`, `Unarchive`,
`CategorySuggest`, `StoreChunks`, `DeleteChunks`, `LogAccess`.

**Read path.** `ReadPool` holds a pool of read-only SQLite connections behind a semaphore, enabling
concurrent queries without contending with the write channel.

**Schema evolution.** `MIGRATIONS` is a `&[&str]` array in `db.rs`. Each element is a SQL statement
(CREATE TABLE, CREATE INDEX, CREATE TRIGGER). `run_migrations` from `nous-shared` executes them
idempotently using `IF NOT EXISTS` guards. The existing schema defines: `models`, `workspaces`,
`categories`, `memories`, `tags`, `memory_tags`, `relationships`, `access_log`, `memory_chunks`,
`memories_fts` (FTS5), `memory_embeddings` (vec0).

**ID generation.** IDs use UUIDv7 via `uuid::Uuid::now_v7().to_string()`, producing
time-ordered, text primary keys. The `define_id!` macro generates a newtype with `Display`,
`FromStr`, `Serialize`, and `Deserialize` impls.

**MCP tool pattern.** Tool parameters are structs deriving `Debug, Deserialize, JsonSchema`. Tools
are registered on `NousServer` using `#[tool_router]` and `#[tool(name = "...", description =
"...")]` attributes from the `rmcp` macros crate. Each tool method receives
`Parameters<ParamStruct>` and returns `CallToolResult`.

**Configuration.** `config.toml` at `~/.config/nous/config.toml` (XDG) with sections: `[memory]`,
`[embedding]`, `[otlp]`, `[classification]`, `[encryption]`. Parsed into a `Config` struct with
per-section sub-structs, each implementing `Default`. Environment variables (`NOUS_MEMORY_DB`,
`NOUS_OTLP_DB`, `NOUS_DB_KEY_FILE`) override config file values.

**What exists today vs. what the scheduler adds.** The `schedules` table does not exist yet — it is
being designed in a parallel workstream. That workstream introduces `schedules` with `action_type`
(e.g., `"bash"`, `"mcp_tool"`) and `action_payload` (inline JSON/script content). The Functions
Module extends that design with a `function_ref_id` column that references the `functions` table,
giving schedules a choice between inline actions and named function references.

## 3. Proposed Changes

### 3a. SQLite Schema
The `functions` table stores both inline content and references to external files. Exactly one of
`content` or `path` is non-NULL per row, enforced by a CHECK constraint.

```sql
CREATE TABLE IF NOT EXISTS functions (
    id TEXT NOT NULL PRIMARY KEY,
    name TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    description TEXT,
    language TEXT NOT NULL CHECK (
        language IN ('bash', 'rust', 'typescript', 'python', 'mcp_tool')
    ),
    content TEXT,
    path TEXT,
    metadata TEXT,  -- JSON: arbitrary key-value pairs (timeout, env vars, etc.)
    checksum TEXT,  -- SHA-256 of content or file at path, for integrity verification
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((content IS NOT NULL AND path IS NULL) OR (content IS NULL AND path IS NOT NULL)),
    UNIQUE(name, version)
);

-- Indexes
CREATE INDEX IF NOT EXISTS idx_functions_name ON functions(name);
CREATE INDEX IF NOT EXISTS idx_functions_language ON functions(language);
CREATE INDEX IF NOT EXISTS idx_functions_created ON functions(created_at);
```

**Design decisions:**

- `id` is a UUIDv7 text primary key (via `FunctionId::new()`), matching the existing `MemoryId`
  pattern. Time-ordered, globally unique, no auto-increment coordination needed.
- `description` is optional free-text, displayed by `function_list` and `function_get`.
- `metadata` stores JSON — e.g., `{"timeout_ms": 30000, "env": {"API_KEY": "$SECRET"}}`. Parsed
  at execution time, not at insert time. Schema validation is the caller's responsibility.
- `checksum` is a hex-encoded SHA-256. Computed on insert/update. On execution, the runtime
  recomputes and compares — a mismatch aborts execution and returns an error.
- Timestamps use ISO 8601 format (`%Y-%m-%dT%H:%M:%fZ`) to match the existing `memories` table
  convention (not Unix epoch as in the research doc draft).
- The `UNIQUE(name, version)` constraint prevents duplicate versions of the same function name.

**Schedule integration column** (added to the planned `schedules` table by the scheduler
workstream):

```sql
ALTER TABLE schedules ADD COLUMN function_ref_id TEXT REFERENCES functions(id);
```

### 3b. Language Support Model
| Language | Storage | Execution model | Sandboxing | Dependencies |
|----------|---------|-----------------|------------|--------------|
| `bash` | `content` (inline script) | Subprocess: `bash -c <content>` | OS-level: run as unprivileged user, optional `timeout(1)` wrapper | System shell, coreutils |
| `python` | `path` (external `.py` file) | Subprocess: `python3 <path>` | OS-level: same as bash | Python 3 runtime on `$PATH` or configured path |
| `typescript` | `path` (external `.ts` file) | Subprocess: `npx tsx <path>` or `node <path>` (compiled JS) | OS-level: same as bash | Node.js runtime on `$PATH` or configured path |
| `rust` | `path` (external `.wasm` file) | WASM: compiled to `.wasm` offline, executed via `wasmtime` | WASM sandbox: no filesystem/network unless explicitly granted via WASI capabilities | `wasmtime` crate linked into `nous-core` |
| `mcp_tool` | `content` (JSON payload) | Direct dispatch: parsed as `{"tool": "...", "params": {...}}`, routed to existing MCP tool handlers | None needed — executes within the Nous process with existing tool permissions | None — uses the MCP tool registry already in `NousServer` |

**Storage rule:** `bash` and `mcp_tool` use `content` (inline) because their payloads are small
and self-contained. `python`, `typescript`, and `rust` use `path` because they reference external
files that may have imports, dependencies, or compilation artifacts.

### 3c. Compilation and Execution Model
**Subprocess (bash, python, typescript).** The executor spawns a child process via
`tokio::process::Command`, captures stdout/stderr, and enforces a configurable timeout (default
30s). Exit code 0 = success; non-zero = failure with stderr as error message.

```rust
// Simplified execution flow for subprocess languages
async fn exec_subprocess(
    binary: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<ExecResult> {
    let child = Command::new(binary)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let output = tokio::time::timeout(timeout, child.wait_with_output()).await??;

    Ok(ExecResult {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into(),
        stderr: String::from_utf8_lossy(&output.stderr).into(),
    })
}
```

**Why subprocess, not embedded interpreters.** Embedding Python or Node.js runtimes adds ~50–100MB
of binary size, FFI complexity, and GIL/event-loop conflicts with Tokio. Subprocess execution
reuses the user's installed runtimes, keeps the Nous binary lean, and provides natural process
isolation. The tradeoff: subprocess startup adds ~10–50ms latency per invocation, acceptable for
scheduled tasks that run on minute+ intervals.

**WASM (rust).** Rust functions compile to `.wasm` targets offline (`cargo build --target
wasm32-wasip2`). At runtime, `nous-core` loads the `.wasm` binary via the `wasmtime` crate,
instantiates a WASI module with controlled capabilities (no network by default, read-only
filesystem access to a sandboxed directory), and calls the exported `run` function.

```rust
// Simplified WASM execution flow
async fn exec_wasm(wasm_path: &Path, timeout: Duration) -> Result<ExecResult> {
    let engine = Engine::default();
    let module = Module::from_file(&engine, wasm_path)?;
    let mut store = Store::new(&engine, WasiCtx::builder().build());
    let instance = Linker::new(&engine)
        .instantiate(&mut store, &module)?;

    let run = instance.get_typed_func::<(), i32>(&mut store, "run")?;
    let exit_code = tokio::time::timeout(timeout, async {
        run.call(&mut store, ())
    }).await??;

    Ok(ExecResult { exit_code, stdout: store.stdout(), stderr: store.stderr() })
}
```

**Why WASM, not native subprocess.** A compiled Rust binary could run as a subprocess, but WASM
provides: (1) sandboxed execution — the function cannot access arbitrary files or network unless
explicitly granted, (2) portable artifacts — `.wasm` files run on any host without recompilation,
(3) deterministic resource limits via the `wasmtime` fuel mechanism.

**Direct dispatch (mcp_tool).** The `content` field contains JSON:
`{"tool": "memory_search", "params": {"query": "auth", "limit": 5}}`. The executor parses this,
looks up the tool in `NousServer`'s tool registry, and calls it directly — no subprocess, no
serialization overhead. This enables functions that compose existing MCP tools into higher-level
operations.

**Why direct dispatch, not self-MCP-call.** Calling the MCP server over stdio/HTTP from within
itself introduces serialization overhead, potential deadlocks (if the write channel is saturated),
and transport complexity. Direct dispatch reuses the existing `WriteChannel` and `ReadPool`
references already held by `NousServer`.

### 3d. Schedule Integration
The planned `schedules` table gains a nullable `function_ref_id TEXT REFERENCES functions(id)`
column. When the scheduler fires a schedule:

1. If `function_ref_id IS NOT NULL`: resolve the function by ID, verify checksum, execute it.
2. If `function_ref_id IS NULL`: fall back to the existing `action_type` + `action_payload`
   inline execution path.

This is fully backward compatible — existing schedules (once the scheduler workstream lands)
continue to work without modification. The `function_ref_id` column defaults to NULL.

**Resolution logic in the scheduler executor:**

```rust
async fn resolve_action(schedule: &Schedule, db: &ReadPool) -> Result<Action> {
    match &schedule.function_ref_id {
        Some(fn_id) => {
            let func = db.get_function(fn_id)?;
            verify_checksum(&func)?;
            Ok(Action::Function(func))
        }
        None => Ok(Action::Inline {
            action_type: schedule.action_type.clone(),
            payload: schedule.action_payload.clone(),
        }),
    }
}
```

**One function, many schedules.** Multiple schedules can reference the same `function_ref_id`. When
the function is updated (creating a new version), existing schedules remain pinned to the old
version ID. Upgrading requires an explicit `schedule_update` call with the new function ID — see
Section 3e.

### 3e. Versioning Model
Functions are immutable after creation. "Updating" a function creates a new row with the same
`name` and an incremented `version`. The old version remains in the database, queryable and
executable.

**Version lifecycle:**

1. **Create v1:** `function_create(name="backup_db", language="bash", content="pg_dump ...")`
   → inserts row with `version=1`, returns `id="019..."`.
2. **Create v2:** `function_update(name="backup_db", content="pg_dump --format=custom ...")`
   → inserts a new row with `version=2`, new `id="019..."`. Version 1 remains unchanged.
3. **Pin a schedule:** `schedule.function_ref_id = "019..."` (the v1 ID). The schedule runs v1
   until explicitly updated.
4. **Upgrade:** `schedule_update(id="sched_id", function_ref_id="019..."` (the v2 ID). Now the
   schedule runs v2.

**Querying versions:**

- `function_get(id)` — returns the exact version identified by that ID.
- `function_versions(name)` — returns all versions of a named function, ordered by version
  descending.
- `function_list` — returns the latest version of each function by default. Pass
  `all_versions=true` to see every version.

**Latest resolution:** To get the latest version of a function by name:

```sql
SELECT * FROM functions
WHERE name = ?1
ORDER BY version DESC
LIMIT 1;
```

This query is used by `function_update` to determine the next version number
(`current_max_version + 1`).

### 3f. Cascade Behavior on Deletion
**Decision: RESTRICT (block deletion when referenced), with a `force` escape hatch.**

Deleting a function that active schedules depend on would silently break those schedules at their
next trigger time — a failure mode that's hard to debug because it's time-delayed.

**Default behavior (`function_delete(id)`):**

1. Query `schedules` for rows where `function_ref_id = ?1`.
2. If any exist, return an error listing the referencing schedule IDs:
   `"Cannot delete function {id}: referenced by schedules [{s1}, {s2}]"`.
3. If none exist, delete the function row.

**Force behavior (`function_delete(id, force=true)`):**

1. Set `function_ref_id = NULL` and `enabled = 0` on all referencing schedules.
2. Delete the function row.
3. Return a result listing the disabled schedule IDs so the caller can re-enable them with new
   function references or inline actions.

**Why RESTRICT over CASCADE.** CASCADE (deleting referencing schedules) is too destructive — a
schedule may have cron configuration, history, and metadata worth preserving. Setting
`function_ref_id = NULL` + disabling is the safer path: the schedule still exists, it just won't
fire until reconfigured.

**Why not ON DELETE SET NULL at the SQL level.** SQL-level `ON DELETE SET NULL` would silently
orphan schedules without disabling them, causing runtime errors when the scheduler tries to resolve
a NULL `function_ref_id` that was previously non-NULL. Application-level enforcement gives us
control to disable the schedule in the same transaction.

## 4. MCP Tool Surface
Eight new MCP tools, registered on `NousServer` following the existing `#[tool]` attribute pattern:

| Tool | Parameters | Return | Description |
|------|-----------|--------|-------------|
| `function_create` | `name`, `language`, `content` or `path`, `description?`, `metadata?` | Created function with `id`, `version`, `checksum` | Create a new function (v1) |
| `function_list` | `language?`, `name?`, `all_versions?`, `limit?`, `offset?` | Array of functions | List functions; returns latest version per name by default |
| `function_get` | `id` | Single function with all fields | Retrieve a function by its exact ID |
| `function_update` | `name`, `content?` or `path?`, `description?`, `metadata?` | New version of the function | Create a new version with incremented version number; original version unchanged |
| `function_delete` | `id`, `force?` | Deletion result with list of affected schedule IDs | Delete a function; blocks if referenced unless `force=true` |
| `function_versions` | `name` | Array of all versions, ordered by version descending | List all versions of a named function |
| `function_test` | `id`, `args?` | `ExecResult` with exit code, stdout, stderr, duration | Execute a function in a dry-run context and return output |
| `function_import` | `source_dir`, `overwrite?` | Import summary with imported/skipped counts and errors | Bulk-import functions from a directory tree following the filesystem convention (Section 5) |

**Parameter structs** (in `crates/nous-mcp/src/tools.rs`):

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionCreateParams {
    pub name: String,
    pub language: String,
    pub content: Option<String>,
    pub path: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<String>,  // JSON string
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionListParams {
    pub language: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub all_versions: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionGetParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionUpdateParams {
    pub name: String,
    pub content: Option<String>,
    pub path: Option<String>,
    pub description: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionDeleteParams {
    pub id: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionVersionsParams {
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionTestParams {
    pub id: String,
    pub args: Option<String>,  // JSON string passed as stdin or CLI args
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FunctionImportParams {
    pub source_dir: String,
    #[serde(default)]
    pub overwrite: bool,
}
```

**WriteOp extensions** (in `crates/nous-core/src/channel.rs`):

```rust
pub enum WriteOp {
    // ... existing variants ...
    FunctionCreate(NewFunction, oneshot::Sender<Result<Function>>),
    FunctionDelete(FunctionId, bool, oneshot::Sender<Result<DeleteResult>>),
}
```

`FunctionCreate` routes through the write channel because it mutates the database.
`FunctionDelete` also routes through the write channel because it may update `schedules` rows
(when `force=true`). `function_update` reuses the `FunctionCreate` WriteOp since it inserts a new
row with an incremented version number. Read operations (`function_list`, `function_get`,
`function_versions`) go through `ReadPool` directly.

## 5. Management Interface
**Decision: MCP tools as the primary runtime API, with a filesystem import convention for bulk
loading.**

MCP tools (`function_create`, `function_list`, etc.) are the authoritative interface for creating,
querying, and managing functions. This matches how Nous already works — all memory operations go
through MCP tools, not direct filesystem access.

**Filesystem import convention.** A `function_import` tool reads functions from a directory
structure:

```
functions/
├── backup_db/
│   ├── 1/
│   │   ├── function.toml    # name, language, description, metadata
│   │   └── content.sh       # the script
│   └── 2/
│       ├── function.toml
│       └── content.sh
└── classify_email/
    └── 1/
        ├── function.toml
        └── main.py
```

`function.toml` structure:

```toml
name = "backup_db"
language = "bash"
description = "Dump the production database to S3"

[metadata]
timeout_ms = 60000
```

The `function_import` tool:
1. Walks the directory tree.
2. For each `<name>/<version>/` directory, reads `function.toml` and the content/path file.
3. Calls the same `FunctionCreate` WriteOp for each, skipping any `(name, version)` pairs that
   already exist in the database.
4. Returns a summary: `{imported: 5, skipped: 2, errors: []}`.

**Why both.** MCP tools serve interactive use — an agent creating a one-off function during a
session. Filesystem import serves version-controlled bulk loading — a repository of functions
checked into git, imported on server startup or via a CI pipeline. The filesystem convention also
enables `function_export` (Phase 3) for backup and migration.

## 6. Configuration
New `[functions]` section in `config.toml`, parsed into a `FunctionsConfig` struct with defaults:

```toml
[functions]
# Paths to language runtimes (auto-detected from $PATH if omitted)
bash_path = "/bin/bash"
python_path = "/usr/bin/python3"
node_path = "/usr/bin/node"

# WASM execution
wasm_cache_dir = "~/.cache/nous/wasm"

# Limits
max_content_size = 65536     # 64 KB max inline content
default_timeout_ms = 30000   # 30 seconds
max_timeout_ms = 300000      # 5 minutes hard cap

# Restrict which languages are enabled
allowed_languages = ["bash", "python", "typescript", "rust", "mcp_tool"]
```

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct FunctionsConfig {
    pub bash_path: String,
    pub python_path: String,
    pub node_path: String,
    pub wasm_cache_dir: String,
    pub max_content_size: usize,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub allowed_languages: Vec<String>,
}

impl Default for FunctionsConfig {
    fn default() -> Self {
        Self {
            bash_path: "bash".into(),
            python_path: "python3".into(),
            node_path: "node".into(),
            wasm_cache_dir: "~/.cache/nous/wasm".into(),
            max_content_size: 65_536,
            default_timeout_ms: 30_000,
            max_timeout_ms: 300_000,
            allowed_languages: vec![
                "bash".into(), "python".into(), "typescript".into(),
                "rust".into(), "mcp_tool".into(),
            ],
        }
    }
}
```

The `Config` struct in `crates/nous-mcp/src/config.rs` gains a `pub functions: FunctionsConfig`
field. Environment variable override: `NOUS_FUNCTIONS_WASM_DIR` overrides `wasm_cache_dir`.

## 7. Testing Strategy
| Layer | Scope | Location | What it validates |
|-------|-------|----------|-------------------|
| Unit | Schema | `crates/nous-core/src/db.rs` (tests module) | `functions` table creation, CHECK constraints reject invalid `language` values, `UNIQUE(name, version)` prevents duplicates, content/path XOR constraint |
| Unit | CRUD | `crates/nous-core/src/functions.rs` (new file) | Insert, query by ID, query by name, list with filters, version increment logic, deletion with RESTRICT behavior |
| Unit | Checksum | `crates/nous-core/src/functions.rs` | SHA-256 computation on insert, mismatch detection on verification |
| Unit | WriteOp | `crates/nous-core/src/channel.rs` (tests module) | `FunctionCreate` and `FunctionDelete` round-trip through write channel, response arrives on oneshot |
| Integration | Bash execution | `tests/e2e/` | Create a bash function via MCP, execute via `function_test`, verify stdout contains expected output |
| Integration | mcp_tool execution | `tests/e2e/` | Create an mcp_tool function that calls `memory_store`, execute it, verify the memory exists |
| Integration | Python execution | `tests/e2e/` | Create a python function referencing a `.py` file, execute, verify exit code and stdout |
| Integration | Schedule integration | `tests/e2e/` | Create a function, attach to a schedule via `function_ref_id`, trigger the schedule, verify function executed |
| Integration | Deletion RESTRICT | `tests/e2e/` | Create a function, attach to a schedule, attempt delete → expect error. Then force-delete → expect schedule disabled |
| Integration | Versioning | `tests/e2e/` | Create v1, update to v2, verify both versions queryable, verify schedule still pinned to v1 |
| E2E | Full lifecycle | `tests/e2e/` | Create function via MCP → list → get → test → attach to schedule → trigger → verify output → delete with force → verify schedule disabled |
| E2E | Import | `tests/e2e/` | Create a directory tree with `function.toml` files → call `function_import` → verify all functions loaded → re-import → verify idempotent (0 new imports) |

**Test infrastructure.** E2E tests start a `NousServer` in-process with an in-memory SQLite
database (`:memory:`), issue MCP tool calls via the `rmcp` client, and assert on `CallToolResult`
content. This matches the existing test pattern in `tests/e2e/`.

## 8. Risks and Open Questions
| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| **Runtime not installed.** Python, Node.js, or bash not available on the host. | Medium | High — functions of that language silently fail. | Validate runtime availability at server startup; log a warning for each missing runtime. `allowed_languages` config lets operators disable unsupported languages. `function_create` rejects languages not in `allowed_languages`. |
| **WASM compilation latency.** First execution of a Rust WASM function cold-starts the `wasmtime` engine (~100–200ms). | Medium | Low — only affects first invocation per engine instance. | Cache the `wasmtime::Engine` instance across invocations. Pre-compile `.wasm` to `.cwasm` (native cache) in `wasm_cache_dir` on first load; subsequent loads take <5ms. |
| **Content size limits.** Large inline scripts exceed `max_content_size`, or large `.wasm` files consume excessive memory. | Low | Medium — OOM or slow DB queries on bloated `content` columns. | Enforce `max_content_size` (default 64KB) on `content` column at insert time. For `path`-based functions, validate file exists and size is under a configurable limit (default 10MB for WASM). |
| **Circular MCP tool invocations.** An `mcp_tool` function calls another function that calls the original, creating infinite recursion. | Low | High — stack overflow or deadlock on write channel. | Track invocation depth in the execution context. Cap at 3 levels (configurable). Return an error: `"Maximum function invocation depth (3) exceeded"`. |
| **Migration on encrypted databases.** `ALTER TABLE schedules ADD COLUMN` may behave differently with SQLCipher. | Low | Medium — migration fails, server won't start. | Test the migration against both plaintext and SQLCipher databases in CI. The `ALTER TABLE ADD COLUMN` statement is supported by SQLCipher without issues, but the test confirms it. |
| **Checksum drift.** File at `path` changes on disk without updating the function's `checksum` in the database. | Medium | Medium — function executes unexpected code. | Recompute checksum before each execution. On mismatch, abort and return an error with both checksums. The `function_update` tool is the correct path to register a new version with the updated content. |
| **Subprocess escape.** Bash or Python function executes arbitrary system commands. | High (by design) | Depends on deployment — low in dev, high in shared environments. | Document that subprocess functions run with the Nous process's privileges. In production deployments, recommend running Nous in a container or as a restricted user. WASM is the safe-by-default alternative for untrusted code. |

**Open questions for future iterations:**

1. Should `function_test` have a separate timeout from production execution? (Recommend: yes, default 10s for test, 30s for production.)
2. Should function execution results be persisted in an `execution_log` table? (Recommend: defer to Phase 3; log to OTLP traces for now.)
3. Should functions support input parameters beyond `args`? (Recommend: defer. Current design passes `args` as a JSON string via stdin; a formal parameter schema is a Phase 3+ concern.)

## 9. Phased Implementation Plan
### Phase 1: Schema + CRUD + bash/mcp_tool execution

**Scope:** Foundation — the `functions` table, core CRUD operations, and execution of the two
simplest languages (bash via subprocess, mcp_tool via direct dispatch).

| Item | Details |
|------|---------|
| New files | `crates/nous-core/src/functions.rs` (CRUD + execution), `crates/nous-shared/src/ids.rs` (add `FunctionId`) |
| Modified files | `crates/nous-core/src/db.rs` (MIGRATIONS), `crates/nous-core/src/channel.rs` (WriteOp variants), `crates/nous-mcp/src/tools.rs` (param structs), `crates/nous-mcp/src/server.rs` (#[tool] registrations), `crates/nous-mcp/src/config.rs` (FunctionsConfig) |
| New MCP tools | `function_create`, `function_list`, `function_get`, `function_delete`, `function_versions` |
| Tests | Unit tests for schema + CRUD + checksum. Integration tests for bash and mcp_tool execution. |
| Effort | 3–5 days, ~800 lines |
| Depends on | Nothing — can proceed independently of the scheduler workstream |

### Phase 2: Python/TypeScript execution + function_update + function_import

**Scope:** Subprocess execution for Python and TypeScript, the `function_update` tool (versioning),
and the filesystem import convention.

| Item | Details |
|------|---------|
| New files | None — extends `functions.rs` with Python/TS execution paths |
| Modified files | `crates/nous-core/src/functions.rs` (exec_subprocess for python/ts), `crates/nous-mcp/src/tools.rs` (FunctionUpdateParams, FunctionImportParams), `crates/nous-mcp/src/server.rs` (new tool registrations) |
| New MCP tools | `function_update`, `function_import` |
| Tests | Integration tests for Python and TypeScript execution. Import idempotency tests. Versioning lifecycle test. |
| Effort | 3–5 days, ~400 lines |
| Depends on | Phase 1 |

### Phase 3: WASM/Rust execution + function_test + schedule integration

**Scope:** WASM execution via `wasmtime`, the `function_test` dry-run tool, and wiring
`function_ref_id` into the scheduler's execution path.

| Item | Details |
|------|---------|
| New dependencies | `wasmtime` crate in `crates/nous-core/Cargo.toml` |
| New files | None — extends `functions.rs` with WASM execution path |
| Modified files | `crates/nous-core/src/functions.rs` (exec_wasm, invocation depth tracking), `crates/nous-core/Cargo.toml` (wasmtime dep), `crates/nous-mcp/src/tools.rs` (FunctionTestParams), `crates/nous-mcp/src/server.rs` |
| New MCP tools | `function_test` |
| Schedule integration | `function_ref_id` resolution logic in the scheduler executor (coordinate with scheduler workstream) |
| Tests | WASM execution with a pre-compiled test `.wasm` binary. function_test dry-run. Full E2E lifecycle. Schedule→function round-trip (requires scheduler). |
| Effort | 5–7 days, ~500 lines |
| Depends on | Phase 2; scheduler workstream (for schedule integration tests) |

**Total estimated scope:** 11–17 days, ~1,700 lines across all phases.
