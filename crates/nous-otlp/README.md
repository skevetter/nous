# nous-otlp

OpenTelemetry Protocol (OTLP) ingestion server and storage layer for the Nous system.
Receives logs, traces, and metrics over HTTP, decodes both Protobuf and JSON payloads,
and persists them in a SQLite database. The stored telemetry data can be correlated with
Nous memories via shared session and trace identifiers.

## Key Types and APIs

### Database (`db`)

`OtlpDb` manages a SQLite database with three tables: `log_events`, `spans`, and
`metrics`, each with appropriate indexes on timestamp and correlation IDs. It provides
batched insert methods (`store_logs`, `store_spans`, `store_metrics`) that use
transactions for efficiency, and query methods (`query_logs` by session ID,
`query_spans` by trace ID) with offset/limit pagination support.

### Decoding (`decode`)

The decode module handles both Protobuf and JSON OTLP payloads. `decode_logs`,
`decode_traces`, and `decode_metrics` parse binary Protobuf using `prost` and
`opentelemetry-proto` definitions. Parallel JSON decoders (`decode_logs_json`,
`decode_traces_json`, `decode_metrics_json`) handle the JSON encoding. Decoded
data is normalized into three structs: `LogEvent` (with timestamp, severity, body,
resource/scope/log attributes, and optional session/trace/span IDs), `Span` (with
trace ID, span ID, parent span ID, name, kind, timing, status, and attributes),
and `Metric` (with name, description, unit, type, and data points as JSON).
Helper functions handle hex encoding of trace and span IDs and attribute-to-JSON
conversion.

### HTTP Server (`server`)

An Axum router exposes three POST endpoints: `/v1/logs`, `/v1/traces`, and
`/v1/metrics`. Each endpoint inspects the `Content-Type` header to choose between
Protobuf (`application/x-protobuf`) and JSON decoding, then stores the decoded
records via `OtlpDb`. `run_server(db, addr)` starts the server with graceful
shutdown support via tokio signal handling.

### CLI

The binary provides two subcommands: `serve` (with configurable port and database
path) and `status` (prints the database location and basic stats). The default
database path follows XDG conventions via `nous-shared`, placing the file at
`$XDG_CACHE_HOME/nous/otlp.db`.

## How It Fits in Nous

The OTLP server runs alongside the MCP server. AI agent sessions emit telemetry
(logs, traces) that land in the OTLP database. The `nous-mcp` crate's
`otlp_trace_context` and `otlp_memory_context` tools query both databases to
correlate memories with their originating telemetry using shared `trace_id` and
`session_id` fields.

## Dependencies

Depends on `nous-shared` for error types and XDG path resolution, `axum` for the
HTTP server, `prost` and `opentelemetry-proto` for Protobuf decoding, `rusqlite`
for storage, `serde_json` for JSON handling, and `tokio` for the async runtime.
