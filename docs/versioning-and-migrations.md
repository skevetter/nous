# Versioning and Database Migration Strategy

> Design document for nous crate versioning and SQLite schema migration management.

## Table of Contents

1. [Overview and Motivation](#overview-and-motivation)
2. [Current State Analysis](#current-state-analysis)
3. [Crate Versioning Strategy](#crate-versioning-strategy)
4. [Database Migration Strategy](#database-migration-strategy)
5. [Migration Bootstrap Plan](#migration-bootstrap-plan)
6. [LOE Estimate](#loe-estimate)
7. [Recommended Phasing](#recommended-phasing)

---

## 1. Overview and Motivation

Nous is a Rust workspace comprising four crates that provides AI-powered memory management with SQLite/SQLCipher storage. As the project matures beyond its initial 0.1.0 release, two infrastructure gaps need to be addressed:

1. **Crate versioning** — All four crates are at 0.1.0 with no git tags, no changelog, and no release automation. There is no way to correlate a deployed binary with a specific schema version or feature set.

2. **Database migration management** — Both databases (Memory DB and OTLP DB) use an unversioned `CREATE IF NOT EXISTS` pattern for schema setup. Post-creation column additions use ad-hoc `PRAGMA table_info` checks. There is no schema version tracking, no rollback capability, and no way to detect a partially applied migration.

This document proposes a versioning strategy for the crate workspace and a migration framework for the two SQLite databases, with a concrete bootstrap plan to transition from the current unversioned state.

## 2. Current State Analysis

### 2.1 Workspace Structure

The workspace (`Cargo.toml`) defines four production crates and two test crates:

| Crate | Path | Sibling Dependencies | Role |
|-------|------|---------------------|------|
| `nous-shared` | `crates/nous-shared` | none | Shared utilities: SQLite connection, migrations runner, error types, IDs |
| `nous-core` | `crates/nous-core` | `nous-shared`, `nous-otlp` | Memory DB, embeddings, chunking, categories, rooms, schedules |
| `nous-otlp` | `crates/nous-otlp` | `nous-shared` | OTLP receiver: logs, spans, metrics storage |
| `nous-mcp` | `crates/nous-mcp` | `nous-core`, `nous-otlp`, `nous-shared` | MCP server, CLI, daemon, export/import |

**Dependency graph:**
```
nous-mcp → nous-core → nous-shared
nous-mcp → nous-otlp → nous-shared
```

All crates share `edition = "2024"` and `rust-version = "1.88"`. Key shared dependencies include `rusqlite` with `bundled-sqlcipher`, `tokio`, `serde`, and `axum`.

### 2.2 Versioning State

- **Crate versions:** All four crates are at `0.1.0`. No version has ever been bumped.
- **Git tags:** `git tag -l` returns empty — no release tags exist.
- **Changelog:** No `CHANGELOG.md` or equivalent exists in the repository.
- **Release automation:** No CI release pipelines, release scripts, or publish workflows.
- **Export format version:** `ExportData` struct (`nous-mcp/src/commands/mod.rs:49-54`) has a `version: u32` field hardcoded to `1` at `commands/admin.rs:125`. This is the only version marker in the codebase.
- **Binary version:** The daemon status endpoint (`nous-mcp/src/daemon_api.rs:60`) reports `CARGO_PKG_VERSION` (currently `0.1.0`).

**Risk:** There is no way to correlate a running binary, a database on disk, or an exported JSON file with a specific codebase revision.

### 2.3 Memory DB Schema (nous-core)

The Memory DB is opened in `nous-core/src/db.rs:246-256` via `MemoryDb::open()`. The `MIGRATIONS` array (`db.rs:12-234`) contains **40 SQL statements** executed through `run_migrations()`:

**Tables (16):** `models`, `workspaces`, `categories`, `memories`, `tags`, `memory_tags`, `relationships`, `access_log`, `memories_fts` (FTS5), `memory_chunks`, `rooms`, `room_participants`, `room_messages`, `room_messages_fts` (FTS5), `schedules`, `schedule_runs`

**Virtual tables (1):** `memory_embeddings` — a `vec0` table created dynamically by `ensure_vec0_table()` (`db.rs:1354-1367`) with a runtime dimension parameter.

**Triggers (6):** Three FTS sync triggers for `memories` (insert/update/delete), one tag cleanup trigger, two FTS sync triggers for `room_messages`.

**Indexes (16):** Covering common query patterns across memories, tags, relationships, chunks, access logs, rooms, messages, and schedules.

**Post-migration column additions** (run after `run_migrations()` in `MemoryDb::open()`):
- `migrate_models_columns()` (`db.rs:1330-1352`): Adds `variant TEXT`, `chunk_size INTEGER`, `chunk_overlap INTEGER`, `active INTEGER` to the `models` table using `PRAGMA table_info` + conditional `ALTER TABLE`.
- `migrate_categories_columns()` (`db.rs:1379-1400`): Adds `description TEXT`, `embedding BLOB`, `threshold REAL` to the `categories` table using the same pattern.

### 2.4 OTLP DB Schema (nous-otlp)

The OTLP DB is opened in `nous-otlp/src/db.rs:54-59` via `OtlpDb::open()`. The `OTLP_MIGRATIONS` array (`otlp/db.rs:6-48`) contains **10 SQL statements**:

**Tables (3):** `log_events`, `spans`, `metrics`

**Indexes (7):** Covering `session_id`, `trace_id`, `timestamp`, `start_time`, and `name`.

No virtual tables, no triggers, no post-migration column additions. This schema is simpler and more stable than the Memory DB.

### 2.5 Migration Infrastructure

Both databases use `run_migrations()` from `nous-shared/src/sqlite.rs:56-66`:

```rust
pub fn run_migrations(conn: &Connection, migrations: &[&str]) -> Result<()> {
    conn.execute_batch("BEGIN")?;
    for migration in migrations {
        if let Err(e) = conn.execute_batch(migration) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e.into());
        }
    }
    conn.execute_batch("COMMIT")?;
    Ok(())
}
```

**Key observations:**
- Every statement uses `CREATE TABLE/INDEX IF NOT EXISTS` — idempotent but unversioned.
- All statements run in a single transaction with rollback on error.
- There is no schema version tracking: no `PRAGMA user_version`, no `schema_version` table, no migration numbering.
- If a migration fails mid-array, there is no way to determine which index succeeded.
- The post-migration `ALTER TABLE` functions (`migrate_models_columns`, `migrate_categories_columns`) run outside the migration transaction as a separate ad-hoc system.

**Two independent migration systems coexist without coordination:**
1. The `MIGRATIONS`/`OTLP_MIGRATIONS` arrays (CREATE IF NOT EXISTS pattern)
2. The `migrate_*_columns()` functions (PRAGMA table_info + conditional ALTER TABLE)

### 2.6 SQLCipher and Encryption

The database uses SQLCipher for encryption at rest (`nous-shared/src/sqlite.rs`):

- **Key resolution** (`sqlite.rs:84-122`): `NOUS_DB_KEY` env var → `~/.config/nous/db.key` file → auto-generate 32 random bytes (hex-encoded, file permissions `0o600`).
- **Connection setup** (`sqlite.rs:16-47`): `PRAGMA key`, then WAL mode pragmas (`journal_mode=WAL`, `wal_autocheckpoint=1000`, `synchronous=NORMAL`, `busy_timeout=5000`, `cache_size=-64000`, `foreign_keys=ON`). Validates encryption with `SELECT count(*) FROM sqlite_master`.
- **Key rotation** (`sqlite.rs:139-196`): Backup → rekey → integrity_check → update key file. Restores backup on failure.
- **sqlite-vec** (`nous-core/src/sqlite_vec.rs`): Loaded via direct FFI call to `sqlite3_vec_init()` from vendored C source at `nous-core/vendor/sqlite-vec/sqlite-vec.c`.
- **Read pool** (`nous-core/src/channel.rs:683-700`): Opens N read-only connections (`PRAGMA query_only = ON`) sharing the same path and key. No migrations run on read connections.
- **Single-writer architecture:** `MemoryDb` holds one `Connection`. All writes go through this single connection. Migrations block all reads during execution.

**Implications for migration tooling:** Any migration framework must work with SQLCipher-encrypted databases. Most Rust migration crates (refinery, sqlx-migrate) assume plain SQLite or a supported database driver — they may not support `rusqlite` with `bundled-sqlcipher` out of the box.

## 3. Crate Versioning Strategy

### 3.1 Recommended Approach: Lockstep Versioning

**Recommendation: Lockstep semver across all four crates.**

All crates share the same version number and are bumped together on every release. Given that:

- There are only 4 crates, all tightly coupled through the dependency graph.
- The project is pre-1.0 with a single consumer (the `nous-mcp` binary).
- No crate is published to crates.io or consumed externally.
- Changes to `nous-shared` (the leaf dependency) affect all downstream crates.

Lockstep eliminates the coordination overhead of tracking per-crate compatibility matrices. The version number appears in:
- All four `Cargo.toml` files
- Git tags (e.g., `v0.2.0`)
- The daemon status endpoint (`CARGO_PKG_VERSION`)
- The `ExportData.version` field (bumped independently when the export schema changes)

**Version semantics (pre-1.0):** Following Rust/Cargo convention, `0.x.y` means:
- Bump `y` (patch) for bug fixes and non-breaking changes
- Bump `x` (minor) for breaking changes, new features, or schema migrations
- Reserve `1.0.0` for when the project considers its API and schema stable

### 3.2 Alternatives Considered

| Approach | Pros | Cons | Verdict |
|----------|------|------|---------|
| **Lockstep** (recommended) | Simple mental model, no compatibility matrix, one tag per release | Forces bumps on unchanged crates | Best fit for 4-crate pre-1.0 single-repo project |
| **Independent versioning** | Each crate bumps only when it changes | Requires compatibility tracking between crates, complex release automation, overkill for 4 crates | Appropriate when crates are published independently to crates.io |
| **Workspace-level version only** | Single version in root `Cargo.toml` | Cargo doesn't natively support workspace-level versions for all fields; requires `workspace.package.version` propagation | Viable but lockstep with explicit per-crate versions is clearer |

### 3.3 Release Management

**Version bumping:**
1. Use `cargo-workspaces` or a simple script to bump all four `Cargo.toml` versions in lockstep.
2. Update `ExportData.version` in `nous-mcp/src/commands/mod.rs` only when the export schema changes (this is independent of the crate version).

**Tagging:**
- Tag format: `v{major}.{minor}.{patch}` (e.g., `v0.2.0`)
- Tag on the merge commit to `main` after the version bump PR is merged.
- Annotated tags with a summary of changes.

**Changelog generation:**
- Use [git-cliff](https://git-cliff.org/) for automated changelog generation from conventional commits.
- Adopt conventional commit format: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`.
- Generate `CHANGELOG.md` at the workspace root.

**Release workflow (manual initially, automate later):**
1. Create a `release/v{x.y.z}` branch
2. Bump versions in all four `Cargo.toml` files
3. Run `git-cliff` to update `CHANGELOG.md`
4. Open PR, merge to `main`
5. Tag the merge commit
6. (Future) CI builds and publishes release artifacts

### 3.4 Cross-Crate Compatibility

With lockstep versioning, cross-crate compatibility is guaranteed by construction — all crates at the same version are always compatible.

**Sibling dependency declarations** should use `path` dependencies (already the case) with `version` as documentation:
```toml
nous-shared = { path = "../nous-shared", version = "=0.2.0" }
```

The `=` pin ensures that if the workspace is ever split, version mismatches are caught immediately.

**Schema version coupling:** The crate version and database schema version are correlated but not identical. A crate version bump does not always imply a schema change. The `PRAGMA user_version` (proposed in Section 4) tracks schema version independently, allowing the application to detect whether a database needs migration regardless of the crate version.

## 4. Database Migration Strategy

### 4.1 Recommended Approach: Embedded Migrations with PRAGMA user_version

**Recommendation: Custom embedded migration runner using `PRAGMA user_version` for version tracking.**

The approach:

1. **Track schema version** using SQLite's built-in `PRAGMA user_version`. This is an integer stored in the database header — no extra table needed, survives database copies, and is atomic to read/write.

2. **Numbered migration functions** replace the current `MIGRATIONS` array. Each migration is a function `fn(conn: &Connection) -> Result<()>` with a sequential version number. The runner:
   - Reads `PRAGMA user_version` to get the current schema version
   - Executes all migrations with version > current, in order
   - Sets `PRAGMA user_version = N` after each successful migration
   - Wraps each migration in its own transaction

3. **Separate version sequences** for Memory DB and OTLP DB. Each database tracks its own `user_version` independently.

```rust
struct Migration {
    version: u32,
    description: &'static str,
    up: fn(&Connection) -> Result<()>,
}

fn run_versioned_migrations(conn: &Connection, migrations: &[Migration]) -> Result<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for m in migrations.iter().filter(|m| m.version > current) {
        conn.execute_batch("BEGIN")?;
        (m.up)(conn)?;
        conn.pragma_update(None, "user_version", m.version)?;
        conn.execute_batch("COMMIT")?;
    }
    Ok(())
}
```

**Why custom over a library:** SQLCipher requires `rusqlite` with `bundled-sqlcipher`. Most Rust migration crates (refinery, sqlx-migrate, barrel) either don't support `rusqlite` directly, or require a specific database driver abstraction that conflicts with SQLCipher's `PRAGMA key` flow. A custom solution is ~100 lines of code and gives full control over the encryption setup sequence.

### 4.2 Migration File Structure

Migrations live alongside the database code in each crate:

```
crates/nous-shared/src/
  sqlite.rs          # run_versioned_migrations() runner
  migration.rs       # Migration struct, helpers

crates/nous-core/src/
  db.rs              # MemoryDb
  migrations/
    mod.rs           # pub const MIGRATIONS: &[Migration] = &[...]
    v001_initial.rs  # Bootstrap: all current tables, indexes, triggers
    v002_*.rs        # Future migrations

crates/nous-otlp/src/
  db.rs              # OtlpDb
  migrations/
    mod.rs
    v001_initial.rs  # Bootstrap: log_events, spans, metrics + indexes
```

Each migration file exports a single `pub fn up(conn: &Connection) -> Result<()>`. The `mod.rs` aggregates them into the ordered `MIGRATIONS` array.

### 4.3 Alternatives Considered

| Approach | Pros | Cons | Verdict |
|----------|------|------|---------|
| **Custom runner + `PRAGMA user_version`** (recommended) | Full SQLCipher control, minimal code (~100 LOC), no external deps, battle-tested SQLite feature | Must maintain runner code | Best fit given SQLCipher constraints |
| **refinery** | Mature Rust migration crate, supports rusqlite | Needs custom driver adapter for SQLCipher PRAGMA key sequence; `bundled-sqlcipher` feature may conflict | Would work with adapter effort, but adds complexity for limited benefit |
| **sqlx-migrate** | Part of sqlx ecosystem | Requires sqlx runtime, not compatible with raw rusqlite/SQLCipher | Not viable without major refactor |
| **Schema version table** (instead of PRAGMA) | More metadata (description, timestamp, checksum) | Extra table to create/maintain, not atomic with SQLite internals | Over-engineered for this use case; PRAGMA user_version is simpler |
| **Keep current approach** (CREATE IF NOT EXISTS) | No migration needed | Cannot add columns, rename tables, or track schema version; ad-hoc ALTER TABLE functions will proliferate | Unsustainable as schema evolves |

### 4.4 Special Considerations

**vec0 virtual tables cannot be ALTERed.** The `memory_embeddings` table (`db.rs:1354-1367`) is created with a runtime dimension parameter. sqlite-vec does not support `ALTER TABLE` on `vec0` tables — the embedding dimension is fixed at creation. Changing dimensions requires `DROP TABLE` + `CREATE VIRTUAL TABLE` with the new dimension, which destroys all stored vectors. This is already handled by `reset_embeddings()` (`db.rs:262-272`). Migrations that change embedding dimensions must:
1. Back up the database before migration
2. Drop and recreate the vec0 table
3. Re-embed all chunks (a background task, not part of the migration transaction)

**SQLite ALTER TABLE constraints:**
- `ALTER TABLE ... ADD COLUMN` is the only widely supported ALTER operation.
- `DROP COLUMN` requires SQLite 3.35.0+ (the bundled SQLCipher version should be checked).
- No `RENAME COLUMN`, `ALTER COLUMN TYPE`, or `ADD CONSTRAINT` support.
- For complex schema changes, use the **12-step ALTER TABLE process**: create new table → copy data → drop old table → rename new table. Wrap in a transaction with `PRAGMA foreign_keys = OFF` temporarily.

**FTS5 virtual tables** (`memories_fts`, `room_messages_fts`) must be rebuilt if the source table schema changes. Use `INSERT INTO fts_table(fts_table) VALUES('rebuild')` after data migration.

**Single-writer architecture implications:** Migrations run on the single write connection in `MemoryDb`. During migration, the read pool connections (via `ReadPool` in `channel.rs:683-700`) may see stale schema. Mitigation: close and reopen read pool connections after migration completes. In practice, migrations run at startup before the read pool is created, so this is not currently a concern.

**Data preservation is paramount.** The CEO uses nous in production. Every migration must preserve all existing data. Destructive operations (DROP TABLE, column removal) must copy data to the new schema first. The backup-before-migrate pattern (see Rollback Strategy) is mandatory.

### 4.5 Rollback Strategy

**Recommendation: Forward-only migrations with backup-before-migrate.**

Down migrations (rollback scripts) are risky in SQLite because:
- Some operations are irreversible (data type changes, column removals where data is lost)
- FTS5 and vec0 virtual tables have non-trivial rebuild requirements
- Testing down migrations doubles the test surface for marginal benefit

Instead:
1. **Before any migration**, copy the database file to `{db_path}.pre-v{N}.bak`
2. Run the migration forward
3. If the migration fails, restore from backup
4. Keep backups for the last 3 versions (configurable)

```rust
fn backup_before_migrate(db_path: &Path, current_version: u32) -> Result<PathBuf> {
    let backup = db_path.with_extension(format!("pre-v{}.bak", current_version + 1));
    std::fs::copy(db_path, &backup)?;
    Ok(backup)
}
```

This mirrors the existing pattern in key rotation (`sqlite.rs:139-196`) where the database is backed up before rekey and restored on failure.

### 4.6 Testing Migrations

**Unit tests for each migration:**
- Test that migration N applies cleanly to a database at version N-1.
- Test that `run_versioned_migrations()` brings a fresh database from version 0 to the latest version.
- Test that running migrations on an already-current database is a no-op.

**Snapshot testing:**
- Maintain a set of test fixture databases at known versions (can use in-memory databases created by running migrations up to version N).
- Verify that each migration preserves data: insert test rows before migration, verify they exist and are correct after migration.

**Integration with existing tests:**
- `nous-core/src/db.rs` already has a `test_migrations()` function (`db.rs:237-239`) that returns the MIGRATIONS array. This can be adapted to return the new `Migration` structs.
- The `tests/e2e` crate can include migration round-trip tests with SQLCipher-encrypted databases.

**CI considerations:**
- Run migration tests in CI on every PR that touches `migrations/` directories.
- Use `rusqlite` with `bundled-sqlcipher` in tests (already the workspace default) to ensure encryption compatibility.

## 5. Migration Bootstrap Plan

Transitioning from the current unversioned state to the versioned migration system requires careful handling of existing production databases.

**Step 1: Establish baseline version (v1)**

The first versioned migration (`v001_initial`) must handle two cases:
- **Fresh database:** Create all tables, indexes, triggers, and virtual tables from scratch. Set `user_version = 1`.
- **Existing database:** Detect that tables already exist (check `sqlite_master`), apply any missing post-migration columns (`migrate_models_columns`, `migrate_categories_columns`), ensure vec0 table exists, and set `user_version = 1`.

```rust
fn v001_initial(conn: &Connection) -> Result<()> {
    let has_memories: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memories'",
        [], |r| r.get(0),
    )?;

    if has_memories {
        // Existing database — apply missing columns and mark as v1
        migrate_models_columns(conn)?;
        migrate_categories_columns(conn)?;
    } else {
        // Fresh database — run full schema creation
        for stmt in INITIAL_SCHEMA {
            conn.execute_batch(stmt)?;
        }
    }
    Ok(())
}
```

**Step 2: Migrate the runner**

Replace the current `run_migrations()` call in `MemoryDb::open()` and `OtlpDb::open()` with `run_versioned_migrations()`. The call sequence in `MemoryDb::open()` changes from:

```rust
// Before
run_migrations(&conn, MIGRATIONS)?;
migrate_models_columns(&conn)?;
seed_placeholder_model(&conn)?;
ensure_vec0_table(&conn, dimensions)?;
migrate_categories_columns(&conn)?;
seed_categories(&conn)?;
```

To:

```rust
// After
backup_before_migrate(path, get_user_version(&conn)?)?;
run_versioned_migrations(&conn, &MEMORY_MIGRATIONS)?;
ensure_vec0_table(&conn, dimensions)?;  // Still dynamic, outside migration system
```

**Step 3: Consolidate ad-hoc migrations**

The `migrate_models_columns()` and `migrate_categories_columns()` functions are absorbed into `v001_initial`. Future column additions become `v002`, `v003`, etc. The ad-hoc pattern is retired.

**Step 4: Handle vec0 table separately**

The `memory_embeddings` vec0 table remains outside the migration system because its dimension is a runtime parameter (depends on the active embedding model). The `ensure_vec0_table()` function continues to run after migrations. If a dimension change is needed, `reset_embeddings()` handles the drop/recreate cycle.

**Rollout safety:**
- The bootstrap migration is backwards-compatible: existing databases get `user_version = 1` with no schema changes.
- New databases get the same schema as before, just created through the migration system.
- The binary can be rolled back to the pre-migration version without issue (it will ignore `user_version` and run CREATE IF NOT EXISTS as before).

## 6. LOE Estimate

| Task | Estimated Effort | Notes |
|------|-----------------|-------|
| Migration runner (`run_versioned_migrations`) | 0.5 days | ~100 LOC in `nous-shared/src/sqlite.rs`, plus `Migration` struct |
| Backup-before-migrate utility | 0.25 days | ~30 LOC, mirrors existing key rotation backup pattern |
| Memory DB v001_initial migration | 1 day | Convert 40 MIGRATIONS statements + 2 migrate_*_columns functions into versioned migration with existing-DB detection |
| OTLP DB v001_initial migration | 0.5 days | Convert 10 OTLP_MIGRATIONS statements |
| Migration tests (unit + integration) | 1 day | Fresh DB, existing DB, partial-failure, encrypted DB, data preservation |
| Refactor `MemoryDb::open()` and `OtlpDb::open()` | 0.5 days | Replace old call sequence with new runner |
| Versioning setup (git-cliff, conventional commits, bump script) | 0.5 days | Config files, CI integration |
| First version bump (0.1.0 → 0.2.0) + changelog + tag | 0.25 days | Establish the pattern |
| **Total** | **~4.5 days** | |

This estimate assumes a single engineer familiar with the codebase. The migration runner and bootstrap are the highest-risk items and should be reviewed carefully.

## 7. Recommended Phasing

### Phase 1: Migration Infrastructure (Days 1-2)
- Implement `Migration` struct and `run_versioned_migrations()` in `nous-shared`
- Implement backup-before-migrate utility
- Write unit tests for the migration runner (fresh DB, existing DB, version tracking)
- **Gate:** Migration runner passes all tests with SQLCipher-encrypted databases

### Phase 2: Bootstrap Migrations (Days 2-3)
- Write `v001_initial` for Memory DB (handles both fresh and existing databases)
- Write `v001_initial` for OTLP DB
- Refactor `MemoryDb::open()` and `OtlpDb::open()` to use the new runner
- Remove `migrate_models_columns()` and `migrate_categories_columns()` (absorbed into v001)
- Write data preservation tests (insert rows → migrate → verify rows)
- **Gate:** Existing production database opens successfully with new code, `user_version` is set to 1

### Phase 3: Versioning Setup (Day 4)
- Adopt conventional commit format
- Configure `git-cliff` for changelog generation
- Create version bump script (updates all four `Cargo.toml` files in lockstep)
- Bump to `0.2.0`, generate first `CHANGELOG.md`, create first git tag `v0.2.0`
- **Gate:** `git tag -l` shows `v0.2.0`, changelog exists, daemon reports correct version

### Phase 4: Ongoing (Post-Implementation)
- All future schema changes are versioned migrations (`v002`, `v003`, etc.)
- Each release follows the release workflow: bump → changelog → PR → merge → tag
- Migration tests run in CI on every PR touching `migrations/` directories
- Database backups are created automatically before each migration at startup
