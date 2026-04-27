# nous-shared

Foundation crate for the Nous workspace. Provides shared error types, typed identifiers,
SQLite connection management with optional SQLCipher encryption, and XDG-compliant
directory resolution. Every other crate in the workspace depends on `nous-shared`.

## Key Types and APIs

### Error Handling

`NousError` is a unified error enum with nine variants covering SQLite errors, I/O
failures, configuration problems, encryption issues, validation failures, not-found
conditions, and general internal errors. The companion `Result<T>` type alias is
re-exported from the crate root for convenience.

### Typed Identifiers

The `define_id!` macro generates newtype `String` wrappers with `Display`, `FromStr`,
`Serialize`, and `Deserialize` implementations. Four ID types are defined: `SessionId`,
`TraceId`, `SpanId`, and `MemoryId`. `MemoryId` additionally provides a `new()` constructor
that generates UUIDv7 identifiers for time-ordered uniqueness.

### SQLite Utilities

`open_connection(path, key)` opens a SQLite database with WAL-mode pragmas
(`journal_mode`, `synchronous`, `busy_timeout`, `cache_size`, `foreign_keys`) and
optional SQLCipher encryption via the `key` parameter. `run_migrations` executes
migration SQL inside a single transaction with automatic rollback on failure.
`resolve_key` reads an encryption key from the `NOUS_DB_KEY` environment variable or
generates and persists a 32-byte hex key to `$XDG_CONFIG_HOME/nous/db.key` with
mode 0600. `rotate_key` performs a SQLCipher rekey with backup creation, integrity
verification, and automatic rollback on failure. `spawn_blocking` wraps
`tokio::task::spawn_blocking` with `NousError`-compatible error handling.

### XDG Directory Resolution

The `xdg` module resolves cache and config directories following this precedence:
`NOUS_CACHE_DIR` / `NOUS_CONFIG_DIR` environment variables, then `XDG_CACHE_HOME` /
`XDG_CONFIG_HOME` with a `nous` subdirectory, then `$HOME/.cache/nous` or
`$HOME/.config/nous`. Helper functions `db_path(name)` and `config_path(name)` build
full paths within these directories.

## Dependencies

Core dependencies include `rusqlite` for SQLite access, `thiserror` for derive-based
error definitions, `serde` and `serde_json` for serialization, `tokio` for async
runtime support, `uuid` for UUIDv7 generation, and `rand` for cryptographic key
generation.
