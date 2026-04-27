# Nous — Claude Code MCP Server Setup Guide

## 1. Prerequisites
- **Rust 1.88+** — install via [rustup](https://rustup.rs/) if not already present
- **C compiler** — `gcc` or `clang` (required by SQLCipher and ONNX Runtime native builds)
- **Internet access** — needed on first semantic search to download the embedding model from HuggingFace

## 2. Building Nous
Clone the repository and build the release binary:

```bash
git clone <repo-url> nous
cd nous
cargo build --release
```

The binary is at:

```
target/release/nous-mcp
```

Run the test suite to verify the build:

```bash
cargo test
```

All tests should pass.

## 3. Claude Code Configuration
Add the following to your Claude Code MCP settings. The config file lives at `~/.claude/settings.json` (user-level) or `.claude/settings.json` (project-level).

```json
{
  "mcpServers": {
    "nous": {
      "command": "/home/skevetter/ws/nous/target/release/nous-mcp",
      "args": ["serve", "--transport", "stdio"]
    }
  }
}
```

> **The path must be absolute.** Replace `/home/skevetter/ws/nous` with your actual clone location. Tilde (`~`) and `$HOME` are not expanded by Claude Code's MCP launcher.

## 4. First Run & Verification
On first launch, Nous auto-creates several resources:

### Database

SQLite database at `~/.cache/nous/memory.db`. Created automatically with 34 migrations that set up 11 tables, FTS5 full-text search indexes, and vec0 vector search.

### Config

Default config is written to `~/.config/nous/config.toml` if it doesn't exist. Key settings:

```toml
[memory]
db_path = "~/.cache/nous/memory.db"

[embedding]
model = "onnx-community/Qwen3-Embedding-0.6B-ONNX"
variant = "onnx/model_q4f16.onnx"
chunk_size = 512
chunk_overlap = 64

[encryption]
db_key_file = "~/.config/nous/db.key"
```

### Encryption

> **WARNING: KEY LOSS = DATA LOSS.**
>
> The database is encrypted with SQLCipher. On first run, Nous auto-generates an encryption key at `~/.config/nous/db.key`. This key is **not recoverable**. If you lose or corrupt the key file, the database cannot be decrypted. **Back up `~/.config/nous/db.key` immediately.**

### Embedding Model Download

The first time you run a semantic search (`memory_search` with `mode: "semantic"` or `"hybrid"`), Nous downloads the `onnx-community/Qwen3-Embedding-0.6B-ONNX` ONNX model from HuggingFace. This is a one-time download. Subsequent runs use the cached model.

### Quick Verification

After configuring Claude Code, start a new session and try:

1. Store a test memory — call `memory_store` with a title, content, and memory_type (`"fact"`)
2. Search for it — call `memory_search` with the title as the query
3. Check stats — call `memory_stats` to see database statistics

## 5. Available MCP Tools
Nous exposes 30 MCP tools organized into these groups:

### Memory CRUD (6)

| Tool | Description |
|------|-------------|
| `memory_store` | Store a new memory with title, content, type, tags, importance, confidence |
| `memory_recall` | Recall a memory by ID (returns full record with relations, tags, category) |
| `memory_search` | Search memories using FTS, semantic, or hybrid mode with rich filters |
| `memory_update` | Update fields on a memory (re-embeds automatically on content change) |
| `memory_forget` | Archive (soft) or hard-delete a memory |
| `memory_unarchive` | Restore an archived memory |

### Relationships (2)

| Tool | Description |
|------|-------------|
| `memory_relate` | Create a typed relationship between two memories (`related`, `supersedes`, `contradicts`, `depends_on`) |
| `memory_unrelate` | Remove a relationship between two memories |

### Categories (5)

| Tool | Description |
|------|-------------|
| `memory_category_suggest` | Suggest a new category, compute its embedding, and assign it to a memory |
| `memory_category_list` | List all categories as a tree (optionally filter by source: system/user/agent) |
| `memory_category_add` | Create a new user-sourced category with optional parent, description, threshold |
| `memory_category_delete` | Delete a category by name |
| `memory_category_update` | Update a category's name, description, and/or threshold |

### Query & Context (4)

| Tool | Description |
|------|-------------|
| `memory_context` | Get context-relevant memories for a workspace path |
| `memory_workspaces` | List all workspaces with memory counts |
| `memory_tags` | List all tags with usage counts |
| `memory_stats` | Database statistics: counts by type, category, importance, workspace |

### Introspection (2)

| Tool | Description |
|------|-------------|
| `memory_schema` | Return the current database schema SQL |
| `memory_sql` | Execute a read-only SQL query against the database (SELECT/EXPLAIN/PRAGMA only) |

### OTLP Correlation (2)

| Tool | Description |
|------|-------------|
| `otlp_trace_context` | Get correlated memories, spans, and logs for a trace ID |
| `otlp_memory_context` | Look up a memory by ID and fetch correlated OTLP spans and logs |

### Chat Rooms (9)

| Tool | Description |
|------|-------------|
| `room_create` | Create a new conversation room |
| `room_list` | List rooms (optionally include archived) |
| `room_get` | Get a room by ID or name |
| `room_delete` | Archive or hard-delete a room |
| `room_post_message` | Post a message to a room |
| `room_read_messages` | Read messages with optional pagination |
| `room_search` | Full-text search within a room's messages |
| `room_info` | Get room details including participants and message count |
| `room_join` | Add a participant to a room (owner/member/observer) |

## 6. Known Limitations
- **Encryption key is auto-generated and not recoverable.** Back up `~/.config/nous/db.key` immediately after first run.
- **First semantic search requires internet** to download the embedding model from HuggingFace. Subsequent runs use the local cache.
- **Path in MCP config must be absolute.** Claude Code does not expand `~` or environment variables in MCP server commands.
- **OTLP tools require a separate OTLP database.** The OTLP collector must be running and writing to `~/.cache/nous/otlp.db` for `otlp_trace_context` and `otlp_memory_context` to return data. If OTLP is not configured, these tools return an error.

## 7. Troubleshooting
### Binary not found

If Claude Code reports that the MCP server binary cannot be found:

```
Error: spawn /path/to/nous-mcp ENOENT
```

Build the binary first with `cargo build --release`, then verify the path in your config matches `target/release/nous-mcp` in your clone directory.

### Permission denied on binary

```bash
chmod +x target/release/nous-mcp
```

### Model download failures

If the embedding model download fails (network issues, proxy, air-gapped environment):

1. Check that you have internet access from the machine running Nous
2. If behind a proxy, set `HTTPS_PROXY` before launching Claude Code
3. The model is cached in the HuggingFace cache directory (`~/.cache/huggingface/`). You can pre-download it manually:
   ```bash
   # The model is: onnx-community/Qwen3-Embedding-0.6B-ONNX
   # variant: onnx/model_q4f16.onnx
   ```

### Database locked errors

If you see `database is locked` errors, check for multiple Nous processes writing to the same database. Nous uses WAL mode with a single-writer channel, so only one instance should write at a time. Kill duplicate processes:

```bash
pgrep -f nous-mcp
```

### Config file location

Nous looks for config at `~/.config/nous/config.toml` (XDG config path). Override with a custom path or environment variables:

| Variable | Overrides |
|----------|-----------|
| `NOUS_MEMORY_DB` | `memory.db_path` |
| `NOUS_OTLP_DB` | `otlp.db_path` |
| `NOUS_DB_KEY_FILE` | `encryption.db_key_file` |
| `NOUS_DAEMON_SOCKET` | `daemon.socket_path` |
| `NOUS_ROOMS_MAX` | `rooms.max_rooms` |
| `NOUS_ROOMS_MAX_MESSAGES` | `rooms.max_messages_per_room` |
