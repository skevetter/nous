# Nous

Rust MCP memory server with OTLP trace ingestion. Stores, searches, and retrieves structured memories through the [Model Context Protocol](https://modelcontextprotocol.io/), with an independent OTLP HTTP endpoint for trace-to-memory correlation.

## Features

- **Hybrid search** — FTS5 full-text (BM25), semantic (ONNX embeddings via `ort`), or both fused with Reciprocal Rank Fusion
- **Encryption at rest** — SQLite via SQLCipher (`rusqlite` with `bundled-sqlcipher`)
- **15 MCP tools** — `memory_store`, `memory_recall`, `memory_search`, `memory_context`, `memory_forget`, `memory_update`, `memory_relate`, and more
- **OTLP HTTP receiver** — ingests OpenTelemetry traces, logs, and metrics into a local SQLite database
- **Trace correlation** — memories link to trace/span IDs for observability context
- **Categories, tags, workspaces** — hierarchical organization with importance and confidence metadata

## Architecture

| Crate | Role |
|-------|------|
| `nous-shared` | Typed IDs, SQLite helpers, XDG paths, error types |
| `nous-core` | Memory schema, CRUD, FTS5/semantic/hybrid search, embedding trait, chunking, classification |
| `nous-mcp` | MCP server binary (stdio + HTTP transports) and management CLI |
| `nous-otlp` | OTLP HTTP receiver binary, decodes protobuf into SQLite |

## Quick Start

Requires Rust 1.88+.

```bash
cargo build

# MCP server (stdio, default)
cargo run -p nous-mcp -- serve

# MCP server (HTTP on port 8377)
cargo run -p nous-mcp -- serve --transport http --port 8377

# OTLP receiver (HTTP on port 4318)
cargo run -p nous-otlp -- serve
```

Or use the justfile:

```bash
just serve-mcp
just serve-otlp
just test
just check        # fmt + clippy + test
```

## CLI Commands

```
nous-mcp serve              Start MCP server
nous-mcp status             Show database stats
nous-mcp export             Export memories as JSON
nous-mcp import <file>      Import from JSON
nous-mcp re-embed           Re-embed all memories with a new model
nous-mcp re-classify        Re-classify memories
nous-mcp category list      List categories
nous-mcp category add       Add a category
nous-mcp rotate-key         Rotate SQLCipher encryption key
```

## Usage with Claude Code

Add to `.mcp.json` at your project root:

```json
{
  "mcpServers": {
    "nous": {
      "command": "path/to/nous-mcp",
      "args": ["serve"]
    }
  }
}
```

This starts `nous-mcp` in stdio transport mode (the default). Claude Code manages the process lifecycle.

For HTTP transport instead:

```bash
nous-mcp serve --transport http --port 8377
```

## Configuration

`nous-mcp` reads `~/.config/nous/config.toml` on startup. If the file does not exist, it creates one with default values.

```toml
[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "model_q4f16.onnx"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "~/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "~/.config/nous/db.key"
```

| Section | Key | Default | Purpose |
|---------|-----|---------|---------|
| `memory` | `db_path` | `~/.cache/nous/memory.db` | SQLCipher database for memories |
| `embedding` | `model` | `onnx-community/Qwen3-Embedding-0.6B-ONNX` | ONNX embedding model from Hugging Face |
| `embedding` | `variant` | `model_q4f16.onnx` | Quantized ONNX variant |
| `embedding` | `chunk_size` | `512` | Token window per text chunk |
| `embedding` | `chunk_overlap` | `64` | Overlapping tokens between adjacent chunks |
| `otlp` | `db_path` | `~/.cache/nous/otlp.db` | SQLite database for ingested telemetry |
| `otlp` | `port` | `4318` | OTLP HTTP receiver port |
| `classification` | `confidence_threshold` | `0.3` | Minimum score to assign a category |
| `encryption` | `db_key_file` | `~/.config/nous/db.key` | SQLCipher key file (auto-generated if missing) |

### Environment variable overrides

Environment variables take precedence over `config.toml`:

| Variable | Overrides |
|----------|-----------|
| `NOUS_MEMORY_DB` | `memory.db_path` |
| `NOUS_OTLP_DB` | `otlp.db_path` |
| `NOUS_DB_KEY_FILE` | `encryption.db_key_file` |

```bash
NOUS_MEMORY_DB=/tmp/test.db nous-mcp serve
```

## Telemetry Setup (OTLP)

`nous-otlp` runs a standalone HTTP server that accepts OpenTelemetry data in protobuf format and stores it in a local SQLite database.

### Endpoints

| Method | Path | Accepts |
|--------|------|---------|
| POST | `/v1/logs` | `ExportLogsServiceRequest` (protobuf) |
| POST | `/v1/traces` | `ExportTraceServiceRequest` (protobuf) |
| POST | `/v1/metrics` | `ExportMetricsServiceRequest` (protobuf) |

All endpoints require `Content-Type: application/x-protobuf`. Requests with other content types return HTTP 415.

### Storage

Ingested data lands in `~/.cache/nous/otlp.db` (override with `--db` flag or `[otlp]` config section). Three tables — `log_events`, `spans`, `metrics` — with indexes on `trace_id`, `session_id`, `timestamp`, and `name`.

### CLI

```bash
nous-otlp serve                 # Start on default port 4318
nous-otlp serve --port 9318     # Custom port
nous-otlp serve --db /tmp/t.db  # Custom database path
nous-otlp status                # Show record counts per table
nous-otlp status --db /tmp/t.db # Status for a specific database
```

### Claude Code integration

Claude Code emits OTLP telemetry when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. Point it at the `nous-otlp` receiver:

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
```

`nous-otlp` only accepts protobuf encoding. If Claude Code defaults to JSON (`OTEL_EXPORTER_OTLP_PROTOCOL=http/json`), override it:

```bash
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
```
