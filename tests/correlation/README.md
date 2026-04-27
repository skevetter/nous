# correlation-tests

Integration test crate that verifies the correlation model between the Nous memory
database (`nous-core`) and the OTLP telemetry database (`nous-otlp`). These tests
exercise the shared identifier contracts — `session_id` and `trace_id` — that link
memories to their originating logs and spans across the two storage systems.

## What These Tests Cover

All tests run against in-memory SQLite databases with no encryption, keeping
execution fast and dependency-free. The test suite validates seven scenarios:

- **Session ID correlation**: A memory stored with a `session_id` can be linked to
  OTLP log events sharing the same session ID. The test stores a memory via
  `MemoryDb` and a log event via `OtlpDb`, then verifies both sides carry the
  matching identifier.

- **Trace ID correlation**: A memory stored with a `trace_id` can be linked to
  OTLP spans sharing the same trace ID. Follows the same store-and-verify
  pattern using `MemoryDb` and `OtlpDb`.

- **Dual correlation**: A single memory carrying both `session_id` and `trace_id`
  correctly links to both OTLP logs (via session) and spans (via trace) simultaneously.

- **Cross-session isolation**: Three independent sessions with distinct session and
  trace IDs are created, and the tests verify that querying by one session's
  identifiers returns only that session's data with no cross-contamination.

- **Trace context lookup**: Simulates the `otlp_trace_context` MCP tool flow by
  querying memories by `trace_id` and correlating them with spans and logs from
  the OTLP database.

- **Reverse memory lookup**: Simulates the `otlp_memory_context` MCP tool flow by
  starting from a memory ID, extracting its correlation IDs, and looking up the
  associated spans and logs.

- **Edge cases**: Verifies that unknown trace IDs return empty results and that
  memories without any correlation IDs have no OTLP association.

## Key Types Used

Tests construct `NewMemory` structs (from `nous-core::types`) with various
combinations of `session_id` and `trace_id`, and `LogEvent` / `Span` structs
(from `nous-otlp::decode`). Direct SQL queries via `rusqlite::params!` are used
to verify data at the storage level, ensuring correctness beyond the API surface.

## Dependencies

Depends on `nous-core` and `nous-otlp` as dev-dependencies, plus `rusqlite` for
direct database assertions.
