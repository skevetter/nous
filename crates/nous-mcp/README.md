# nous-mcp

MCP (Model Context Protocol) server and CLI for the Nous memory system. This crate
is the primary user-facing binary. It exposes 21 MCP tools over stdio or HTTP
transport for AI agents to store, search, classify, and correlate memories. It also
provides CLI subcommands for data management, key rotation, and OTLP trace
inspection.

## MCP Tools

The server registers 21 tools via the rmcp `#[tool_router]` macro:

- **Memory CRUD**: `memory_store`, `memory_recall`, `memory_search`, `memory_context`,
  `memory_forget`, `memory_unarchive`, `memory_update`
- **Relationships**: `memory_relate`, `memory_unrelate`
- **Categories**: `memory_category_suggest`, `memory_category_list`,
  `memory_category_add`, `memory_category_delete`, `memory_category_update`
- **Introspection**: `memory_workspaces`, `memory_tags`, `memory_stats`,
  `memory_schema`, `memory_sql` (read-only queries enforced by safety checks)
- **OTLP Correlation**: `otlp_trace_context`, `otlp_memory_context`

## CLI Subcommands

- `serve` — Start the MCP server over stdio (default) or HTTP transport with
  configurable port, model, and variant
- `export` / `import` — Versioned JSON export and import of memories, relationships,
  tags, and categories
- `re-embed` — Re-generate embeddings for all stored memories using the active model
- `re-classify` — Re-run category classification against all memories
- `category` — Category CRUD: `list`, `add`, `delete`, `rename`, `update` with
  source filtering and parent assignment
- `rotate-key` — SQLCipher encryption key rotation with backup and integrity
  verification
- `status` — Print database path, memory count, model, and dimension info
- `trace` — Query OTLP correlation data by trace ID, session ID, or memory ID

## Server Architecture

`NousServer` holds a `WriteChannel` for batched mutations, a `ReadPool` for
concurrent queries, an `Arc<dyn EmbeddingBackend>` for vector generation, a
`CategoryClassifier`, a `Chunker`, and a `Config`. All MCP tool handlers route
through these shared components. The embedding backend tries `OnnxBackend` first
and falls back to `MockEmbedding(384)` if ONNX initialization fails.

## Configuration

Configuration is loaded from TOML with five sections: `MemoryConfig`,
`EmbeddingConfig`, `OtlpConfig`, `ClassificationConfig`, and `EncryptionConfig`.
Defaults include model `onnx-community/Qwen3-Embedding-0.6B-ONNX`, variant
`onnx/model_q4.onnx`, chunk size 512 with overlap 64, and confidence threshold 0.3.
Environment variable overrides are supported: `NOUS_MEMORY_DB`, `NOUS_OTLP_DB`,
and `NOUS_DB_KEY_FILE`.

## Dependencies

Depends on `nous-core` for the memory engine, `nous-otlp` for OTLP database access,
`nous-shared` for error types and SQLite helpers, `rmcp` for MCP protocol
implementation, `clap` for CLI parsing, `axum` for HTTP transport, and `tokio` for
the async runtime.
