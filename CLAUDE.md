# Nous

Agent memory and telemetry platform. Rust workspace with 4 crates.

## Conventions

- Rust edition 2024, minimum rust-version 1.85
- Use `thiserror` for all error types
- `rusqlite` with `bundled-sqlcipher` feature for SQLite (encryption at rest)
- `rmcp` for MCP protocol implementation
- `ort` for ONNX embedding inference
- `tokenizers` + `hf-hub` for model loading
- `axum` for HTTP servers
- `clap` with derive for CLI parsing
- Follow existing patterns in the codebase

## Dependencies

All shared dependencies go in `[workspace.dependencies]` in the root Cargo.toml. Crate-level Cargo.toml files reference them with `workspace = true`.

## Python

Always use `uv` with virtual environments for Python packages. Never install to system Python. Never use `--break-system-packages`.

## Commits

No adverbs or filler in commit messages. Use imperative mood.

## Crate Structure

- `nous-shared` — common types, SQLite helpers, XDG paths, typed IDs
- `nous-core` — memory schema, CRUD, search, embedding trait, chunking, classification
- `nous-mcp` — MCP server binary + CLI commands
- `nous-otlp` — OTLP HTTP receiver binary
