# e2e-tests

End-to-end test crate that validates the Nous system by running the actual compiled
binaries (`nous-mcp` and `nous-otlp`) as subprocesses. Unlike unit and integration
tests that call Rust APIs directly, these tests exercise the full stack including
CLI argument parsing, environment variable configuration, database creation,
process lifecycle, and HTTP communication.

## Test Infrastructure

The `TestEnv` struct manages a temporary directory containing isolated database
files (`memory.db`, `otlp.db`), a key file, and import data. All paths are passed
to binaries via environment variables (`NOUS_DB_KEY`, `NOUS_MEMORY_DB`,
`NOUS_DB_KEY_FILE`, `NOUS_OTLP_DB`). Helper functions locate the compiled binaries
in the cargo target directory and provide wrappers for running `nous-mcp`
subcommands. `OtlpServer` starts `nous-otlp serve` as a child process on an
ephemeral port, with automatic cleanup on drop. `wait_for_otlp` polls the
`/v1/logs` endpoint until the server is ready, with a 30-second timeout.

## What These Tests Cover

### Import and Export

Tests verify that JSON import files containing memories, tags, and categories are
correctly ingested via `nous-mcp import` and that `nous-mcp export` produces
output containing the expected data. A roundtrip test imports a dataset with
hierarchical categories and confirms both parent and child categories survive
the export cycle.

### OTLP Server

Tests start the `nous-otlp` server binary, verify it creates and populates the
database file, and confirm the `/v1/logs` endpoint responds to HTTP POST requests
with Protobuf content type.

### Full E2E Flow

A comprehensive test combines OTLP server startup, memory import, HTTP POST to the
OTLP endpoint, export verification, and database file validation in a single
scenario.

### Category CRUD

A full suite of category management tests exercises the CLI: add with description,
list with source filtering, rename, update (description and threshold), delete,
delete refusal when children exist, rename via the update subcommand, and a
complete lifecycle test (add, rename, update, verify, delete, verify deletion).

### Re-classify

Verifies that `nous-mcp re-classify` runs successfully and reports progress on
stderr after importing memories with seeded system categories.

### OTLP Correlation CLI

The `TraceTestEnv` struct seeds memories and OTLP data directly via library APIs,
then runs `nous-mcp trace` as a subprocess. Tests cover lookup by trace ID, dual
lookup by trace and session ID, reverse lookup by memory ID, empty results for
unknown trace IDs, and error reporting when a memory lacks correlation identifiers.

## Dependencies

Depends on `nous-core` and `nous-otlp` for direct database seeding in trace tests,
`reqwest` for HTTP client calls, `serde_json` for parsing JSON output, `tempfile`
for temporary directory management, and `tokio` for the async test runtime.
