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
