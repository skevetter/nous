# Nous

Agent memory and telemetry platform. Rust workspace with 4 crates:

| Crate | Purpose |
|-------|---------|
| `nous-shared` | Common types, SQLite helpers, XDG paths, typed IDs |
| `nous-core` | Memory schema, CRUD, search, embedding, chunking, classification |
| `nous-mcp` | MCP server binary + CLI commands |
| `nous-otlp` | OTLP HTTP receiver binary |

## Quick Start

```bash
cargo build --release
# Binary at target/release/nous-mcp
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
