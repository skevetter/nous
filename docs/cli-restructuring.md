# CLI Tiered Restructuring (INI-076)

**Status:** Proposed
**Supersedes:** INI-015 (flat CLI structure, commit e9a50bc)
**Depends on:** INI-074 (crate rename from `nous-mcp` to `nous`)
**Binary:** `nous` (renamed from `nous-mcp` per INI-015)

INI-015 proposed a flat command structure where all 53 commands live at the top level or under shallow groups. That approach does not scale — `nous --help` would list 21+ top-level entries with no organizational signal. This design replaces INI-015's flat taxonomy with a tiered namespace hierarchy while incorporating INI-015's output format (`--format json|csv|human`) and error format proposals.

## 1. Audit of Current CLI Structure

The `nous` binary (crate `nous-mcp`, `src/main.rs`) defines a single `Command` enum parsed by clap. Two organizational patterns coexist: flat top-level variants and grouped subcommand structs.

### Flat Top-Level Commands (21)

| Command | Purpose | Source handler |
|---------|---------|----------------|
| `serve` | Start MCP server (stdio or HTTP) | `main.rs:785` — builds embedding, launches `NousServer` |
| `re-embed` | Re-embed all memories with a specified model | `main.rs:805` — delegates to `commands::run_re_embed` |
| `re-classify` | Re-classify memories (optional `--since`) | `main.rs:828` — delegates to `commands::run_re_classify` |
| `export` | Export memories to JSON | `main.rs:987` — delegates to `commands::run_export` |
| `import` | Import memories from JSON file | `main.rs:990` — delegates to `commands::run_import` |
| `rotate-key` | Rotate database encryption key | `main.rs:998` — delegates to `commands::run_rotate_key` |
| `status` | Show database and system status | `main.rs:1001` — delegates to `commands::run_status` |
| `trace` | Look up trace/span data by ID | `main.rs:1004` — delegates to `commands::run_trace` |
| `store` | Create a new memory | `main.rs:1016` — delegates to `commands::run_store` |
| `recall` | Recall a memory by ID | `main.rs:1056` — delegates to `commands::run_recall` |
| `update` | Update fields on an existing memory | `main.rs:1059` — delegates to `commands::run_update` |
| `forget` | Archive (soft) or delete (hard) a memory | `main.rs:1084` — delegates to `commands::run_forget` |
| `unarchive` | Restore an archived memory | `main.rs:1087` — delegates to `commands::run_unarchive` |
| `relate` | Create a typed relationship between memories | `main.rs:1090` — delegates to `commands::run_relate` |
| `unrelate` | Remove a relationship between memories | `main.rs:1097` — delegates to `commands::run_unrelate` |
| `search` | Search memories (FTS, semantic, or hybrid) | `main.rs:1104` — delegates to `commands::run_search` |
| `context` | Get workspace-scoped context summary | `main.rs:1140` — delegates to `commands::run_context` |
| `sql` | Run read-only SQL against the database | `main.rs:1143` — delegates to `commands::run_sql` |
| `schema` | Dump the database DDL schema | `main.rs:1146` — delegates to `commands::run_schema` |
| `workspaces` | List workspaces with memory counts | `main.rs:1149` — delegates to `commands::run_workspaces` |
| `tags` | List tags with usage counts | `main.rs:1152` — delegates to `commands::run_tags` |

### Grouped Subcommand Namespaces (6 groups, 30 subcommands)

| Group | Subcommands | Struct | Definition |
|-------|-------------|--------|------------|
| `model` | `list`, `info`, `register`, `activate`, `deactivate`, `switch`, `setup` | `ModelCmd` / `ModelSubcommand` | `main.rs:206–248` |
| `embedding` | `inspect`, `reset` | `EmbeddingCmd` / `EmbeddingSubcommand` | `main.rs:250–263` |
| `category` | `list`, `add`, `delete`, `rename`, `update`, `suggest` | `CategoryCmd` / `CategorySubcommand` | `main.rs:265–309` |
| `room` | `create`, `list`, `get`, `post`, `read`, `search`, `delete` | `RoomCmd` / `RoomSubcommand` | `main.rs:311–359` |
| `schedule` | `list`, `get`, `create`, `delete`, `pause`, `resume` | `ScheduleCmd` / `ScheduleSubcommand` | `main.rs:361–396` |
| `daemon` | `start`, `stop`, `restart`, `status` | `DaemonCmd` / `DaemonSubcommand` | `main.rs:398–416` |

### Separate Binary: `nous-otlp` (4 commands)

The `nous-otlp` crate (`crates/nous-otlp/src/main.rs`) produces a separate binary with its own top-level commands:

| Command | Purpose |
|---------|---------|
| `serve` | Start OTLP HTTP receiver on a port |
| `status` | Show OTLP database statistics |
| `logs` | Query OTLP log events by session ID |
| `spans` | Query OTLP spans by trace ID |

### Key Observations

1. **`--help` noise**: 21 flat commands + 6 group names = 27 entries at the top level. A user scanning `nous --help` gets no semantic grouping to orient them.
2. **Name collisions**: `status` exists as both a top-level command and a `daemon` subcommand. `serve` exists in both the `nous` and `nous-otlp` binaries.
3. **Mixed concerns at top level**: Memory CRUD (`store`, `recall`, `forget`), database admin (`re-embed`, `rotate-key`, `status`), and query tools (`sql`, `schema`, `tags`) all sit at the same level.
4. **Stale binary references**: The MCP tool handler at `server.rs:551` registers as `name = "nous-mcp"`. Six test cases in `main.rs` still use `"nous-mcp"` as the binary name in `Cli::try_parse_from` calls.
5. **`nous-otlp` isolation**: The OTLP binary duplicates output format handling and has no integration path into the main CLI.

## 2. Proposed Tiered Grouping

The Planner proposed 10 namespaces. This section evaluates that proposal, identifies conflicts, and presents the refined grouping.

### Planner Proposal (Raw)

```
nous memory   — save, search, recall, context, observe, forget, suggest, timeline, merge-projects
nous model    — (existing)
nous embedding — (existing)
nous category — (existing)
nous room     — (existing)
nous schedule — (existing)
nous daemon   — (existing)
nous admin    — config, setup, reset, status, compact
nous query    — stats, import, export, re-embed, judge, version
nous mcp      — listen
```

### Issues with the Planner Proposal

1. **`observe`, `suggest`, `timeline`, `merge-projects`, `judge`, `compact`, `config`, `setup`, `reset`, `stats`, `listen`, `version` do not exist** in the current binary. The Planner's proposal was based on the task brief's aspirational command list, not the actual `Command` enum. The real commands are documented in Section 1.

2. **`status` collision**: The Planner placed `status` in `nous admin`, but `daemon status` already exists. The current top-level `status` shows database statistics, while `daemon status` shows daemon PID/uptime. These serve different purposes and must remain distinct.

3. **`nous query` is a grab-bag**: Grouping `import`, `export`, `re-embed`, and `version` under "query" is misleading — `re-embed` is a write operation, `import` mutates the database, and `version` is informational.

4. **`nous mcp` for a single command**: Creating a namespace for one subcommand (`listen`/`serve`) adds structure without value.

### Refined Grouping

This design maps every command in the current `Command` enum to a namespace. Commands that do not exist today are excluded — they belong in future feature proposals, not a restructuring doc.

```
nous
├── memory        # Memory CRUD and search (daily driver for the CEO)
│   ├── store
│   ├── recall
│   ├── update
│   ├── forget
│   ├── unarchive
│   ├── search
│   ├── context
│   ├── relate
│   └── unrelate
├── model         # Embedding model lifecycle (unchanged)
│   ├── list
│   ├── info
│   ├── register
│   ├── activate
│   ├── deactivate
│   ├── switch
│   └── setup
├── embedding     # Low-level embedding table ops (unchanged)
│   ├── inspect
│   └── reset
├── category      # Memory categorization (unchanged)
│   ├── list
│   ├── add
│   ├── delete
│   ├── rename
│   ├── update
│   └── suggest
├── room          # Chat room operations (unchanged)
│   ├── create
│   ├── list
│   ├── get
│   ├── post
│   ├── read
│   ├── search
│   └── delete
├── schedule      # Scheduled tasks (unchanged)
│   ├── list
│   ├── get
│   ├── create
│   ├── delete
│   ├── pause
│   └── resume
├── daemon        # Daemon lifecycle (unchanged, keeps its own `status`)
│   ├── start
│   ├── stop
│   ├── restart
│   └── status
├── admin         # Database administration and maintenance
│   ├── status        # Database stats (renamed from top-level `status`)
│   ├── re-embed      # Re-embed all memories
│   ├── re-classify   # Re-classify memories
│   ├── rotate-key    # Rotate encryption key
│   ├── import        # Import memories from JSON
│   └── export        # Export memories to JSON
├── query         # Read-only introspection tools
│   ├── sql
│   ├── schema
│   ├── workspaces
│   ├── tags
│   └── trace
└── serve         # Start MCP server (top-level, no namespace)
```

### Conflict Resolutions

| Conflict | Resolution |
|----------|------------|
| `status` at top-level vs. `daemon status` | Move top-level `status` to `admin status`. `daemon status` remains unchanged. The two commands report different things: `admin status` shows DB stats; `daemon status` shows PID and uptime. |
| `serve` placement | Keep `serve` at the top level. It is the primary entrypoint for MCP integration and used in systemd units, Docker entrypoints, and daemon internals (`main.rs:533`). Nesting it under `nous mcp serve` would break all deployment configurations for no user benefit. |
| `import`/`export` in `admin` vs. `query` | Placed in `admin` because `import` mutates the database and `export` is its counterpart. Keeping them together preserves the mental model of "backup and restore." |
| `re-embed` in `admin` vs. `query` | `admin`. Re-embedding rewrites every vector in the database — it is a maintenance operation, not a query. |
| `trace` in `query` vs. top-level | `query`. Trace lookups are read-only introspection, same as `sql` and `schema`. |

### Command Count Summary

| Namespace | Subcommands | Change from current |
|-----------|-------------|---------------------|
| `memory` | 9 | New group (was 9 flat commands) |
| `model` | 7 | Unchanged |
| `embedding` | 2 | Unchanged |
| `category` | 6 | Unchanged |
| `room` | 7 | Unchanged |
| `schedule` | 6 | Unchanged |
| `daemon` | 4 | Unchanged |
| `admin` | 6 | New group (was 6 flat commands) |
| `query` | 5 | New group (was 5 flat commands) |
| `serve` | — | Stays top-level (was top-level) |
| **Total** | **52 + 1 top-level** | 0 commands lost, 0 invented |

## 3. Command Placement Specifics

Only commands that move from their current location need justification. The 6 existing grouped namespaces (`model`, `embedding`, `category`, `room`, `schedule`, `daemon`) are unchanged.

### Commands Moving to `nous memory`

| Command | Current location | Why `memory` |
|---------|-----------------|--------------|
| `store` | Top-level | Creates a memory. Core CRUD operation on the primary entity. |
| `recall` | Top-level | Reads a memory by ID. Core CRUD. |
| `update` | Top-level | Modifies memory fields. Core CRUD. |
| `forget` | Top-level | Archives or deletes a memory. Core CRUD. |
| `unarchive` | Top-level | Restores a memory. Inverse of `forget`. |
| `search` | Top-level | Searches memories by text, tags, workspace. The most-used query operation on memories. |
| `context` | Top-level | Returns workspace-scoped memory context. Operates exclusively on the memory corpus. |
| `relate` | Top-level | Creates relationships between memories. Memory-centric graph operation. |
| `unrelate` | Top-level | Removes relationships between memories. Inverse of `relate`. |

**Grouping rationale**: These 9 commands all operate on the `memories` table (or its relationships). A user working with memories — the CEO's primary workflow — types `nous memory <verb>` and sees exactly the CRUD and search operations available. No need to scan 27 top-level entries.

### Commands Moving to `nous admin`

| Command | Current location | Why `admin` |
|---------|-----------------|-------------|
| `status` | Top-level | Reports database-level statistics (memory count, embedding count, vec0 dimensions). This is operational introspection, not memory interaction. |
| `re-embed` | Top-level | Rewrites every embedding vector. Long-running, destructive maintenance — not a daily operation. |
| `re-classify` | Top-level | Re-runs classification on memories. Batch maintenance. |
| `rotate-key` | Top-level | Changes the database encryption key. Security administration. |
| `import` | Top-level | Bulk-loads memories from a JSON file. Database mutation at scale. |
| `export` | Top-level | Bulk-exports memories to JSON. Paired with `import` for backup/restore workflows. |

**Grouping rationale**: These commands share a profile — they operate on the database as a whole, run infrequently, and carry higher risk (data loss, long runtime, key rotation). Grouping them under `admin` signals "proceed with intention."

### Commands Moving to `nous query`

| Command | Current location | Why `query` |
|---------|-----------------|-------------|
| `sql` | Top-level | Runs arbitrary read-only SQL. Database introspection tool. |
| `schema` | Top-level | Dumps DDL. Database introspection tool. |
| `workspaces` | Top-level | Lists workspaces with counts. Metadata query. |
| `tags` | Top-level | Lists tags with counts. Metadata query. |
| `trace` | Top-level | Looks up trace/span data by ID. Read-only observability query. |

**Grouping rationale**: All five are read-only, produce tabular or structured output, and serve debugging/development use cases rather than daily memory interaction. `query` groups them as "tools for looking at the system."

### Commands Staying Top-Level

| Command | Why it stays |
|---------|-------------|
| `serve` | Used in systemd units (`nous serve --transport stdio`), Docker `CMD` directives, and the daemon's child process spawning (`main.rs:533`). Moving it to a namespace would break all deployment configurations. The MCP server is the system's primary runtime mode — it deserves top-level status. |

## 4. Backward Compatibility

The CEO uses this CLI daily. Breaking muscle memory with a hard cutover is not acceptable. The strategy below provides a 3-phase deprecation path.

### Phase 1: Aliases with No Warnings (v0.2.0)

Introduce the new namespaced commands alongside the old flat commands. Both work identically. No warnings, no behavior change for existing users.

**Implementation**: Add hidden clap aliases in the `Command` enum. Clap supports `#[command(hide = true)]` variants that forward to the namespaced handler.

```rust
#[derive(Debug, Subcommand)]
enum Command {
    // New namespaced command
    Memory(MemoryCmd),

    // Hidden alias — forwards to `memory store`
    #[command(hide = true)]
    Store { /* same fields */ },

    // ...
}
```

The match arm for the hidden `Store` variant calls the same handler as `Memory(MemoryCmd { command: MemorySubcommand::Store { .. } })`. Zero code duplication in handlers — only the routing layer duplicates.

### Phase 2: Deprecation Warnings (v0.3.0)

The hidden aliases remain functional but now emit a stderr warning on every invocation:

```
⚠ `nous store` is deprecated. Use `nous memory store` instead.
  This alias will be removed in v0.5.0.
```

**Implementation**: A `deprecated_alias()` helper wraps each forwarding match arm:

```rust
fn deprecated_alias(old: &str, new: &str) {
    eprintln!("⚠ `{old}` is deprecated. Use `{new}` instead.");
    eprintln!("  This alias will be removed in v0.5.0.");
}
```

Warnings go to stderr so they do not pollute `--format json` output piped to other tools.

### Phase 3: Removal (v0.5.0)

Remove all hidden alias variants from the `Command` enum. Running `nous store` produces clap's standard "unknown subcommand" error with a "did you mean?" suggestion pointing to `nous memory store`.

### Timeline

| Phase | Version | Behavior |
|-------|---------|----------|
| Aliases (silent) | v0.2.0 | Old and new commands both work, no warnings |
| Deprecation warnings | v0.3.0 | Old commands warn on stderr, still work |
| Removal | v0.5.0 | Old commands fail with clap suggestion |

The gap between v0.2.0 and v0.5.0 provides at least 3 release cycles (targeting ~2 months each) for the CEO and any scripts to migrate.

### What About Shell Scripts and Aliases?

The CEO's workflow may include shell aliases or scripts that call flat commands. To ease discovery:

1. Add a `nous migrate-check` one-shot command (introduced in v0.2.0, removed in v0.5.0) that scans a given file for deprecated command patterns and prints replacement suggestions.
2. Document the full mapping in `nous --help` during Phase 2 under a "Deprecated Commands" section.

## 5. Subcommand and Group Descriptions

All text below is copy-paste ready for clap `#[command(about = "...")]` and `#[arg(help = "...")]` attributes.

### Top-Level

```
#[command(name = "nous", about = "Nous memory system — store, search, and serve memories")]
```

### `nous serve`

```
#[command(about = "Start the MCP server")]

--transport <MODE>    Transport protocol [default: stdio] [possible values: stdio, http]
--port <PORT>         HTTP listen port [default: 8377]
--model <NAME>        Override embedding model name
--variant <VARIANT>   Override embedding model variant
--allow-shell-schedules  Enable shell-based schedule actions
--no-scheduler        Disable the scheduler loop
```

### `nous memory` Group

```
#[command(about = "Create, read, update, and search memories")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `store` | `"Store a new memory"` | `--title` (required), `--content` (required), `--type` [default: observation], `--source`, `--importance`, `--confidence`, `--tags` (comma-delimited), `--workspace`, `--session-id`, `--trace-id`, `--agent-id`, `--agent-model`, `--valid-from`, `--category-id` |
| `recall` | `"Recall a memory by ID"` | `<ID>` (positional, required) |
| `update` | `"Update fields on an existing memory"` | `<ID>` (positional, required), `--title`, `--content`, `--importance`, `--confidence`, `--tags`, `--valid-until` |
| `forget` | `"Archive or permanently delete a memory"` | `<ID>` (positional, required), `--hard` (permanently delete instead of archive) |
| `unarchive` | `"Restore an archived memory"` | `<ID>` (positional, required) |
| `search` | `"Search memories by text, tags, or workspace"` | `<QUERY>` (positional, required), `--mode` [default: hybrid, possible: fts/semantic/hybrid], `--type`, `--importance`, `--confidence`, `--workspace`, `--tags`, `--archived`, `--since`, `--until`, `--valid-only`, `--limit` [default: 20] |
| `context` | `"Get context summary for a workspace"` | `<WORKSPACE>` (positional, required), `--summary` |
| `relate` | `"Create a typed relationship between two memories"` | `<SOURCE>` `<TARGET>` `<TYPE>` (positional, required) |
| `unrelate` | `"Remove a relationship between two memories"` | `<SOURCE>` `<TARGET>` `<TYPE>` (positional, required) |

### `nous model` Group

```
#[command(about = "Manage embedding models")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `list` | `"List registered embedding models"` | *(none)* |
| `info` | `"Show detailed info for a model"` | `<ID>` (positional, required) |
| `register` | `"Register a new embedding model"` | `--name`, `--variant`, `--dimensions`, `--max-tokens` [default: 8192], `--chunk-size` [default: 512], `--chunk-overlap` [default: 64] |
| `activate` | `"Activate a model for embedding"` | `<ID>` (positional, required) |
| `deactivate` | `"Deactivate a model"` | `<ID>` (positional, required) |
| `switch` | `"Switch the active embedding model"` | `<ID>` (positional, required), `--force` (skip confirmation) |
| `setup` | `"Download and activate a preset model"` | `[PRESET]` (positional, optional — omit to list presets) |

### `nous embedding` Group

```
#[command(about = "Inspect and reset the embedding vector table")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `inspect` | `"Show vec0 dimensions and embedding count"` | *(none)* |
| `reset` | `"Drop and recreate the vec0 table"` | `--force` (skip confirmation prompt) |

### `nous category` Group

```
#[command(about = "Manage memory categories")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `list` | `"List categories"` | `--source` (filter by source: system, user, agent) |
| `add` | `"Create a new category"` | `<NAME>` (positional, required), `--parent`, `--description` |
| `delete` | `"Delete a category"` | `<NAME>` (positional, required) |
| `rename` | `"Rename a category"` | `<OLD>` `<NEW>` (positional, required) |
| `update` | `"Update category properties"` | `<NAME>` (positional, required), `--new-name`, `--description`, `--threshold` |
| `suggest` | `"Suggest a category for a memory"` | `<MEMORY_ID>` (positional, required), `--name`, `--description`, `--parent` |

### `nous room` Group

```
#[command(about = "Manage chat rooms")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `create` | `"Create a chat room"` | `<NAME>` (positional, required), `--purpose` |
| `list` | `"List chat rooms"` | `--archived` (include archived), `--limit` |
| `get` | `"Get room details by ID"` | `<ID>` (positional, required) |
| `post` | `"Post a message to a room"` | `<ROOM>` `<CONTENT>` (positional, required), `--sender`, `--reply-to` |
| `read` | `"Read messages from a room"` | `<ROOM>` (positional, required), `--limit`, `--since` |
| `search` | `"Search messages in a room"` | `<ROOM>` `<QUERY>` (positional, required), `--limit` |
| `delete` | `"Delete a chat room"` | `<ID>` (positional, required), `--hard` (permanent delete) |

### `nous schedule` Group

```
#[command(about = "Manage scheduled tasks")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `list` | `"List all schedules"` | *(none)* |
| `get` | `"Show schedule details"` | `<ID>` (positional, required) |
| `create` | `"Create a new schedule"` | `--name`, `--cron`, `--action-type`, `--payload`, `--timezone`, `--desired-outcome` |
| `delete` | `"Delete a schedule"` | `<ID>` (positional, required) |
| `pause` | `"Pause a running schedule"` | `<ID>` (positional, required) |
| `resume` | `"Resume a paused schedule"` | `<ID>` (positional, required) |

### `nous daemon` Group

```
#[command(about = "Control the nous background daemon")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `start` | `"Start the daemon"` | `--foreground` (run in foreground instead of detaching) |
| `stop` | `"Stop the running daemon"` | *(none)* |
| `restart` | `"Restart the daemon"` | `--foreground` |
| `status` | `"Show daemon PID and uptime"` | *(none)* |

### `nous admin` Group

```
#[command(about = "Database administration and maintenance")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `status` | `"Show database statistics"` | *(none)* |
| `re-embed` | `"Re-embed all memories with a model"` | `--model` (see Section 6 for proposed default), `--variant` |
| `re-classify` | `"Re-classify memories"` | `--since` (only re-classify memories after this timestamp) |
| `rotate-key` | `"Rotate the database encryption key"` | `--new-key-file` (path to new key file) |
| `import` | `"Import memories from a JSON file"` | `<FILE>` (positional, required) |
| `export` | `"Export all memories to JSON"` | `--format` [default: json] |

### `nous query` Group

```
#[command(about = "Read-only database introspection tools")]
```

| Subcommand | `about` | Key flags |
|------------|---------|-----------|
| `sql` | `"Run a read-only SQL query"` | `<QUERY>` (positional, required) |
| `schema` | `"Print the database DDL schema"` | *(none)* |
| `workspaces` | `"List workspaces with memory counts"` | *(none)* |
| `tags` | `"List tags with usage counts"` | *(none)* |
| `trace` | `"Look up trace or span data"` | `--trace-id`, `--memory-id` (mutually exclusive), `--session-id` (requires `--trace-id`) |

### Global Flags (inherited by all subcommands)

```
--config <PATH>    Config file path
--db <PATH>        Database path override
-v, --verbose      Verbose output
-q, --quiet        Quiet mode (errors only)
--format <FMT>     Output format [default: human] [possible values: human, json, csv]
```

## 6. CEO UX Fixes

Three specific UX issues have been identified. Each is scoped to a concrete code change.

### 6.1 Model List Shows Internal/Test Models

**Problem**: `nous model list` calls `db.list_models()` (`commands/model.rs:23`) and renders every row. Models registered during testing or development — names containing "placeholder", "mock", or "test" — appear alongside production models.

**Current code** (`commands/model.rs:18–105`):
```rust
pub fn run_model_list(config: &Config, format: &OutputFormat) -> Result<...> {
    let db = open_db(config)?;
    let models = db.list_models()?;  // no filtering
    // ... render all models
}
```

**Proposal**: Filter the model list in the CLI layer (not the DB layer, to avoid breaking MCP tool responses). Add a `--all` flag to show everything.

```rust
let models: Vec<_> = db.list_models()?
    .into_iter()
    .filter(|m| show_all || !is_internal_model(&m.name))
    .collect();

fn is_internal_model(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("placeholder") || lower.contains("mock") || lower.contains("test")
}
```

Default behavior shows only production models. `nous model list --all` shows everything.

### 6.2 Re-Embed Requires `--model` Flag

**Problem**: `nous re-embed --model <NAME>` (`main.rs:66–71`) requires the user to specify a model name. If the user just wants to re-embed with the currently active model, they must first run `nous model list`, find the active model's name, then pass it to `--model`. This is 3 commands for a 1-command operation.

**Current code** (`main.rs:66–71`):
```rust
ReEmbed {
    #[arg(long)]
    model: String,  // required
    #[arg(long)]
    variant: Option<String>,
}
```

**Proposal**: Make `model` an `Option<String>`. When omitted, resolve the model from:
1. The active model in the database (`db.list_models()?.iter().find(|m| m.active)`)
2. Fall back to `config.embedding.model`

```rust
ReEmbed {
    #[arg(long)]
    model: Option<String>,  // optional — defaults to active model
    #[arg(long)]
    variant: Option<String>,
}
```

In the handler, resolve before calling `run_re_embed`:
```rust
Command::ReEmbed { model, variant } => {
    let resolved_model = match model {
        Some(m) => m,
        None => {
            let db = open_db(&config)?;
            db.list_models()?
                .iter()
                .find(|m| m.active)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| config.embedding.model.clone())
        }
    };
    // ...
}
```

### 6.3 Error Hints Reference Stale Binary Name `nous-mcp`

**Problem**: The MCP tool handler registers as `name = "nous-mcp"` at `server.rs:551`, and 6 test assertions in `main.rs` use `"nous-mcp"` as the binary name in `Cli::try_parse_from` calls (lines 1328, 1378, 1438, 1622, 1708, 1757).

While the binary was already renamed to `nous` in `Cargo.toml` (`[[bin]] name = "nous"`), these stale references could surface in MCP client UIs or confuse contributors reading tests.

**Locations**:

| File | Line | Current value | Replacement |
|------|------|---------------|-------------|
| `server.rs` | 551 | `#[tool_handler(name = "nous-mcp", version = "0.1.0")]` | `#[tool_handler(name = "nous", version = "0.1.0")]` |
| `main.rs` | 1328 | `"nous-mcp"` in test | `"nous"` |
| `main.rs` | 1378 | `"nous-mcp"` in test | `"nous"` |
| `main.rs` | 1438 | `"nous-mcp"` in test | `"nous"` |
| `main.rs` | 1622 | `"nous-mcp"` in test | `"nous"` |
| `main.rs` | 1708 | `"nous-mcp"` in test | `"nous"` |
| `main.rs` | 1757 | `"nous-mcp"` in test | `"nous"` |

**Proposal**: Replace all 7 occurrences. The `tool_handler` name change is the highest priority because it affects MCP protocol responses visible to clients. The test changes are cosmetic but prevent future confusion. This fix should land as part of INI-074 (crate rename) since it directly relates to the binary name migration.

## 7. Migration Plan

### Approach: Incremental, Not Big-Bang

A big-bang restructure in one PR would touch every file in `crates/nous-mcp/src/`, rewrite all tests, and risk a broken release. The incremental approach delivers value per-PR and keeps `main` shippable at every step.

### Dependency: INI-074 (Crate Rename)

INI-074 renames the crate from `nous-mcp` to `nous-cli` (or similar) and updates all internal references. The CLI restructuring must serialize after INI-074 because:
1. Both efforts modify `main.rs` and the `Command` enum extensively.
2. The stale `nous-mcp` references (Section 6.3) are INI-074's scope, not this effort's.
3. Merging both simultaneously would create unresolvable merge conflicts.

### Implementation Sequence

| Step | Scope | PR size (est.) | Description |
|------|-------|----------------|-------------|
| 1 | `memory` namespace | Medium | Extract `store`, `recall`, `update`, `forget`, `unarchive`, `search`, `context`, `relate`, `unrelate` into `MemoryCmd`/`MemorySubcommand`. Add hidden aliases for all 9 flat commands. Update tests. |
| 2 | `admin` namespace | Medium | Extract `status`, `re-embed`, `re-classify`, `rotate-key`, `import`, `export` into `AdminCmd`/`AdminSubcommand`. Add hidden aliases. Update tests. |
| 3 | `query` namespace | Small | Extract `sql`, `schema`, `workspaces`, `tags`, `trace` into `QueryCmd`/`QuerySubcommand`. Add hidden aliases. Update tests. |
| 4 | UX fixes | Small | Implement model list filtering (6.1), re-embed default model (6.2). These changes touch `commands/model.rs` and the `ReEmbed` match arm — cleanest to land after the restructure stabilizes. |
| 5 | Deprecation warnings | Small | Add `deprecated_alias()` stderr warnings to all hidden alias match arms. Bump version to signal Phase 2. |
| 6 | Alias removal | Small | Remove all hidden `Command` variants. Bump to v0.5.0. |

Steps 1–3 can each be reviewed and merged independently. Step 4 is independent of 1–3 but easier to land after them (the `ReEmbed` variant moves to `AdminSubcommand` in step 2). Steps 5–6 are version-gated and land when the deprecation timeline is reached.

### Existing Namespaces: No Changes Required

The 6 existing groups (`model`, `embedding`, `category`, `room`, `schedule`, `daemon`) already use the `XxxCmd` / `XxxSubcommand` pattern. They require no structural changes — only the hidden alias mechanism and deprecation warnings apply to the 20 flat commands being reorganized.

### Test Strategy

Each restructuring PR must:
1. Add tests for the new namespaced path (`Cli::try_parse_from(["nous", "memory", "store", ...])`)
2. Add tests for the hidden alias path (`Cli::try_parse_from(["nous", "store", ...])`) verifying it resolves to the same handler
3. Verify `--format json` output is byte-identical between old and new paths

### Risk Mitigation

| Risk | Mitigation |
|------|------------|
| CEO's workflow breaks | Phase 1 aliases ensure old commands work silently. The CEO sees no change until Phase 2 warnings. |
| MCP tool names change | The MCP server exposes tools by function name (e.g., `store_memory`), not CLI command path. The restructure does not affect MCP tool registration. |
| Merge conflicts with concurrent work | Each step touches a distinct subset of `Command` variants. Steps 1–3 can be ordered to minimize overlap. |
| Forgotten alias after removal | The step 6 PR runs the full test suite. Any test still using a flat command path will fail, catching missed aliases. |
