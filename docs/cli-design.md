# Nous CLI Design

## Overview

Nous currently exposes two binaries: `nous-mcp` (MCP server + limited management CLI) and `nous-otlp` (OTLP HTTP receiver). The existing CLI covers server operations (serve), batch operations (re-embed, re-classify), category management (list/add), and import/export, but 11 major library operations have no CLI surface: search, memory CRUD, model management, SQL queries, OTLP queries, workspace/tag listing, access analytics, and database inspection.

This design extends the CLI to expose all library operations while maintaining MCP server compatibility. The unified CLI supports two user profiles: **operators** performing batch operations and database administration, and **developers** querying memory state during development and debugging.

The design follows Rust CLI conventions (clap derive API), introduces structured output formats (JSON/CSV), implements consistent error handling with exit codes, and proposes testing infrastructure for E2E validation.

**Binary naming note:** This document uses `nous` as the command name throughout. The current binary is named `nous-mcp`. This design proposes renaming the binary from `nous-mcp` to `nous` for simplicity, as it now serves as a general-purpose CLI beyond just the MCP server functionality.

## Command Hierarchy and Taxonomy

Commands divide into **query operations** (read-only, fast, developer-facing) and **admin operations** (write operations, batch jobs, operator-facing).

### Query Commands (Read-Only)

```
nous search <query>                  # Search memories (FTS/semantic/hybrid)
nous recall <memory-id>              # Recall memory by ID with relations
nous context <workspace>             # Workspace-scoped context
nous sql <query>                     # Read-only SQL queries
nous status                          # Database statistics (existing)
nous schema                          # DDL schema dump
nous workspaces                      # List workspaces with counts
nous tags                            # List tags with counts
nous model list                      # List embedding models
nous model info <model-id>           # Detailed model metadata
nous embedding inspect               # Current vec0 dimensions and counts
nous category list                   # Category tree (existing)
nous otlp logs <session-id>          # Query OTLP log events
nous otlp spans <trace-id>           # Query OTLP spans
```

### Admin Commands (Write Operations)

```
nous store                           # Store new memory (interactive/flags)
nous update <memory-id>              # Update memory fields
nous forget <memory-id>              # Archive (soft) or delete (hard)
nous unarchive <memory-id>           # Restore archived memory
nous relate <source> <target>        # Create typed relationship
nous unrelate <source> <target>      # Remove relationship
nous model register                  # Register new embedding model
nous model activate <model-id>       # Activate model
nous model deactivate <model-id>     # Deactivate model
nous model switch <model-id>         # Switch active model (warns on vec0 reset)
nous embedding reset                 # Drop and recreate vec0 table
nous category add <name>             # Add category (existing)
nous category suggest <memory-id>    # Suggest + assign category
nous re-embed                        # Re-embed all memories (existing)
nous re-classify                     # Re-classify memories (existing)
nous import <file>                   # Import from JSON (existing)
nous export                          # Export to JSON (existing)
nous rotate-key                      # Rotate encryption key (existing)
nous serve                           # Start MCP server (existing)
```

### Full Command Tree

```
nous
├── search <query> [--mode fts|semantic|hybrid] [--filters ...]
├── recall <memory-id>
├── context <workspace>
├── sql <query>
├── status
├── schema
├── workspaces
├── tags
├── store [--title ...] [--content ...] [--type ...]
├── update <memory-id> [--title ...] [--content ...]
├── forget <memory-id> [--hard]
├── unarchive <memory-id>
├── relate <source> <target> <type>
├── unrelate <source> <target> <type>
├── model
│   ├── list
│   ├── info <model-id>
│   ├── register --name <name> --variant <variant> --dimensions <n>
│   ├── activate <model-id>
│   ├── deactivate <model-id>
│   └── switch <model-id> [--force]
├── embedding
│   ├── inspect
│   └── reset [--force]
├── category
│   ├── list [--source system|user|agent]
│   ├── add <name> [--parent <id>] [--description <text>]
│   └── suggest <memory-id>
├── re-embed --model <name> [--variant <variant>]
├── re-classify [--since <timestamp>]
├── import <file>
├── export [--format json]
├── rotate-key [--new-key-file <path>]
├── serve [--transport stdio|http] [--port <n>]
└── otlp
    ├── logs <session-id> [--limit <n>]
    └── spans <trace-id> [--limit <n>]
```

## Subcommand Structure with Examples

### Memory Operations

#### `nous store` — Create Memory

**Flags:**
- `--title <text>` (required)
- `--content <text>` (required, or read from stdin if `-`)
- `--type <type>` (default: observation; values: decision, convention, bugfix, architecture, fact, observation)
- `--source <text>` (default: "cli")
- `--importance <level>` (default: moderate; values: low, moderate, high)
- `--confidence <level>` (default: moderate; values: low, moderate, high)
- `--tags <tag1,tag2,...>`
- `--workspace <path>` (default: current directory)
- `--session-id <uuid>`
- `--trace-id <uuid>`
- `--agent-id <text>`
- `--agent-model <text>`
- `--valid-from <timestamp>`
- `--category-id <id>`

**Example:**
```bash
nous store \
  --title "Database connection pool size increased" \
  --content "Increased connection pool from 10 to 50 to handle increased load from batch jobs." \
  --type decision \
  --importance high \
  --tags "database,performance"
```

**With stdin:**
```bash
cat decision.txt | nous store --title "Migration strategy" --content - --type decision
```

**Output (default):**
```
Memory stored: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
```

**Output (JSON):**
```json
{"memory_id": "mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T", "created_at": "2026-04-26T23:45:12Z"}
```

#### `nous recall <memory-id>` — Recall Memory

**Flags:**
- `<memory-id>` (positional, required)
- `--format <fmt>` (default: human; values: human, json)

**Example:**
```bash
nous recall mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
```

**Output (default):**
```
ID: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
Title: Database connection pool size increased
Type: decision
Importance: high
Confidence: moderate
Workspace: /home/user/project
Tags: database, performance
Created: 2026-04-26T23:45:12Z
Updated: 2026-04-26T23:45:12Z

Content:
Increased connection pool from 10 to 50 to handle increased load from batch jobs.

Relations:
  Supersedes: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5S
```

#### `nous update <memory-id>` — Update Memory

**Flags:**
- `<memory-id>` (positional, required)
- `--title <text>`
- `--content <text>` (or `-` for stdin)
- `--tags <tag1,tag2,...>` (replaces existing tags)
- `--importance <level>`
- `--confidence <level>`
- `--valid-until <timestamp>`

At least one update field required.

**Example:**
```bash
nous update mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T \
  --importance moderate \
  --tags "database,performance,resolved"
```

#### `nous forget <memory-id>` — Archive or Delete

**Flags:**
- `<memory-id>` (positional, required)
- `--hard` (hard delete; default is soft archive)

**Example:**
```bash
nous forget mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T       # Archive (soft)
nous forget mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T --hard # Delete (hard)
```

**Output:**
```
Memory archived: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
```

#### `nous unarchive <memory-id>` — Restore Archived

**Flags:**
- `<memory-id>` (positional, required)

**Example:**
```bash
nous unarchive mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
```

#### `nous relate <source> <target> <type>` — Create Relationship

**Flags:**
- `<source>` (source memory ID, positional)
- `<target>` (target memory ID, positional)
- `<type>` (relation type, positional; values: related, supersedes, contradicts, depends_on)

**Example:**
```bash
nous relate mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T mem_01HQZX3Y7J8K9M0N1P2Q3R4S5S supersedes
```

#### `nous unrelate <source> <target> <type>` — Remove Relationship

**Flags:**
- `<source>` (source memory ID, positional)
- `<target>` (target memory ID, positional)
- `<type>` (relation type, positional)

**Example:**
```bash
nous unrelate mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T mem_01HQZX3Y7J8K9M0N1P2Q3R4S5S supersedes
```

### Search Operations

#### `nous search <query>` — Search Memories

**Flags:**
- `<query>` (positional, required)
- `--mode <mode>` (default: hybrid; values: fts, semantic, hybrid)
- `--type <type>` (filter by memory type)
- `--category <id>` (filter by category ID)
- `--workspace <path>` (filter by workspace path)
- `--importance <level>` (filter by importance)
- `--confidence <level>` (filter by confidence)
- `--tags <tag1,tag2,...>` (filter by tags, AND semantics)
- `--archived` (include archived memories; default: false)
- `--since <timestamp>` (created after timestamp)
- `--until <timestamp>` (created before timestamp)
- `--valid-only` (only memories valid at current time)
- `--limit <n>` (default: 20, max: 100)
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous search "database connection pool" --mode hybrid --type decision --limit 10
```

**Output (default):**
```
1. [mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T] Database connection pool size increased (rank: 0.92)
   Type: decision | Importance: high | Created: 2026-04-26T23:45:12Z
   Tags: database, performance

2. [mem_01HQZX3Y7J8K9M0N1P2Q3R4S5U] Connection pool monitoring added (rank: 0.85)
   Type: fact | Importance: moderate | Created: 2026-04-25T14:30:00Z
   Tags: database, monitoring

Found 2 results (limit: 10)
```

**Output (JSON):**
```json
{
  "query": "database connection pool",
  "mode": "hybrid",
  "results": [
    {
      "memory_id": "mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T",
      "title": "Database connection pool size increased",
      "memory_type": "decision",
      "importance": "high",
      "rank": 0.92,
      "tags": ["database", "performance"],
      "created_at": "2026-04-26T23:45:12Z"
    }
  ],
  "count": 2,
  "limit": 10
}
```

**Output (CSV):**
```csv
memory_id,title,memory_type,importance,rank,tags,created_at
mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T,Database connection pool size increased,decision,high,0.92,"database,performance",2026-04-26T23:45:12Z
```

#### `nous context <workspace>` — Workspace Context

**Flags:**
- `<workspace>` (workspace path, positional, required)
- `--summary <text>` (optional context summary for semantic filtering)
- `--format <fmt>` (default: human; values: human, json)

**Example:**
```bash
nous context /home/user/project --summary "authentication flow"
```

**Output (default):**
```
Workspace: /home/user/project
Memories: 5

1. [mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T] OAuth2 flow implemented
   Type: fact | Importance: high | Created: 2026-04-20T10:00:00Z

2. [mem_01HQZX3Y7J8K9M0N1P2Q3R4S5U] JWT token validation added
   Type: decision | Importance: high | Created: 2026-04-21T15:30:00Z

...
```

#### `nous sql <query>` — Execute Read-Only SQL

**Flags:**
- `<query>` (SQL query, positional, required)
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous sql "SELECT memory_type, COUNT(*) as count FROM memories GROUP BY memory_type"
```

**Output (default):**
```
memory_type   | count
--------------+------
decision      | 45
convention    | 23
bugfix        | 67
architecture  | 12
fact          | 234
observation   | 156
```

**Output (JSON):**
```json
{
  "columns": ["memory_type", "count"],
  "rows": [
    {"memory_type": "decision", "count": 45},
    {"memory_type": "convention", "count": 23}
  ]
}
```

**Note:** Only SELECT queries allowed. All write operations (INSERT, UPDATE, DELETE, DROP, CREATE) rejected with exit code 2 (usage error).

#### `nous status` — Database Statistics

**Existing command.** Enhanced with `--format json`.

**Flags:**
- `--format <fmt>` (default: human; values: human, json)

**Example:**
```bash
nous status --format json
```

**Output (CSV):**
```bash
nous status --format csv
```

```csv
metric,value
memories,537
chunks,1234
embeddings,1234
categories,23
workspaces,3
```

#### `nous schema` — DDL Schema Dump

**New command.**

**Flags:**
- None

**Example:**
```bash
nous schema > schema.sql
```

#### `nous workspaces` — List Workspaces

**New command.**

**Flags:**
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous workspaces
```

**Output (default):**
```
ID | Path                      | Memories
---+---------------------------+---------
1  | /home/user/project-a      | 234
2  | /home/user/project-b      | 56
3  | /home/user/experiments    | 12
```

**Output (CSV):**
```bash
nous workspaces --format csv
```

```csv
id,path,memories
1,/home/user/project-a,234
2,/home/user/project-b,56
3,/home/user/experiments,12
```

#### `nous tags` — List Tags

**New command.**

**Flags:**
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous tags --format csv
```

**Output (CSV):**
```csv
tag,count
database,89
performance,67
authentication,45
```

### Model Management

#### `nous model list` — List Models

**Flags:**
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous model list
```

**Output (default):**
```
ID | Name                      | Variant            | Dims | Max Tokens | Active | Created
---+---------------------------+--------------------+------+------------+--------+----------
1  | BAAI/bge-small-en-v1.5    | onnx/model.onnx    | 384  | 512        | *      | 2026-04-01
2  | BAAI/bge-base-en-v1.5     | onnx/model.onnx    | 768  | 512        |        | 2026-04-15
```

**Output (CSV):**
```bash
nous model list --format csv
```

```csv
id,name,variant,dimensions,max_tokens,active,created
1,BAAI/bge-small-en-v1.5,onnx/model.onnx,384,512,true,2026-04-01
2,BAAI/bge-base-en-v1.5,onnx/model.onnx,768,512,false,2026-04-15
```

**Note:** The `*` in the Active column indicates the currently active model. When vec0 dimensions are detected, display them alongside the model dimensions to warn of mismatches.

#### `nous model info <model-id>` — Model Details

**Flags:**
- `<model-id>` (positional, required)
- `--format <fmt>` (default: human; values: human, json)

**Example:**
```bash
nous model info 1
```

**Output (default):**
```
Model ID: 1
Name: BAAI/bge-small-en-v1.5
Variant: onnx/model.onnx
Dimensions: 384
Max Tokens: 512
Chunk Size: 512
Chunk Overlap: 64
Active: yes
Created: 2026-04-01T10:00:00Z

Embeddings: 1,234 chunks in vec0 table (dimensions: 384)
```

#### `nous model register` — Register Model

**Flags:**
- `--name <name>` (required; e.g., "BAAI/bge-small-en-v1.5")
- `--variant <variant>` (required; e.g., "onnx/model.onnx")
- `--dimensions <n>` (required)
- `--chunk-size <n>` (default: 512)
- `--chunk-overlap <n>` (default: 64)

**Example:**
```bash
nous model register \
  --name "BAAI/bge-base-en-v1.5" \
  --variant "onnx/model.onnx" \
  --dimensions 768 \
  --chunk-size 512 \
  --chunk-overlap 64
```

**Output:**
```
Model registered: ID 2
```

#### `nous model activate <model-id>` — Activate Model

**Flags:**
- `<model-id>` (positional, required)

**Example:**
```bash
nous model activate 2
```

**Output:**
```
Model activated: 2 (BAAI/bge-base-en-v1.5)
```

#### `nous model deactivate <model-id>` — Deactivate Model

**Flags:**
- `<model-id>` (positional, required)

**Example:**
```bash
nous model deactivate 1
```

**Output:**
```
Model deactivated: 1 (BAAI/bge-small-en-v1.5)
```

#### `nous model switch <model-id>` — Switch Active Model

**Flags:**
- `<model-id>` (positional, required)
- `--force` (skip confirmation prompt if vec0 reset needed)

**Behavior:**
1. Check current active model dimensions vs. target model dimensions
2. If dimensions differ, warn that vec0 table will be reset (all embeddings deleted)
3. Prompt for confirmation unless `--force` specified
4. Deactivate current model
5. Activate target model
6. If dimensions differ, call `reset_embeddings()` to drop and recreate vec0 table

**Example:**
```bash
nous model switch 2
```

**Output:**
```
Warning: Switching from model 1 (384 dims) to model 2 (768 dims) will reset all embeddings.
This will delete 1,234 chunks from the vec0 table.

Proceed? (y/N): y

Model switched: 2 (BAAI/bge-base-en-v1.5)
Embeddings reset: vec0 table recreated with 768 dimensions
```

**With `--force`:**
```bash
nous model switch 2 --force
```

**Output:**
```
Model switched: 2 (BAAI/bge-base-en-v1.5)
Embeddings reset: vec0 table recreated with 768 dimensions
```

### Database Administration

#### `nous embedding inspect` — Inspect Embedding State

**Flags:**
- `--format <fmt>` (default: human; values: human, json)

**Behavior:**
Queries the vec0 table to detect current dimensions, counts embeddings, shows active model info, and warns if dimensions mismatch between active model and vec0 table.

**Example:**
```bash
nous embedding inspect
```

**Output (default):**
```
Active Model: 1 (BAAI/bge-small-en-v1.5)
Model Dimensions: 384
Chunk Size: 512
Chunk Overlap: 64

vec0 Table:
  Dimensions: 384
  Embeddings: 1,234 chunks

Status: OK (dimensions match)
```

**Dimension mismatch output:**
```
Active Model: 2 (BAAI/bge-base-en-v1.5)
Model Dimensions: 768
Chunk Size: 512
Chunk Overlap: 64

vec0 Table:
  Dimensions: 384
  Embeddings: 1,234 chunks

Status: ERROR — dimension mismatch!
The active model produces 768-dimensional embeddings, but vec0 table expects 384 dimensions.
Run 'nous embedding reset' to recreate the vec0 table, or switch back to a 384-dim model.
```

#### `nous embedding reset` — Reset Embedding Table

**Flags:**
- `--force` (skip confirmation prompt)

**Behavior:**
Calls `reset_embeddings()` to drop the vec0 table and recreate it with dimensions matching the active model. All embeddings are deleted. Chunks in `memory_chunks` remain but have no embeddings until re-embed runs.

**Example:**
```bash
nous embedding reset
```

**Output:**
```
Warning: This will delete all 1,234 embeddings from the vec0 table.
Chunks will remain in memory_chunks but have no embeddings until you run 'nous re-embed'.

Proceed? (y/N): y

vec0 table reset: 1,234 embeddings deleted
New dimensions: 768 (matching active model 2: BAAI/bge-base-en-v1.5)
```

**With `--force`:**
```bash
nous embedding reset --force
```

**Output:**
```
vec0 table reset: 1,234 embeddings deleted
New dimensions: 768 (matching active model 2: BAAI/bge-base-en-v1.5)
```

#### `nous re-embed` — Re-Embed All Memories

**Existing command.** Enhanced to call `ensure_vec0_table()` before embedding.

**Flags:**
- `--model <name>` (required; e.g., "BAAI/bge-small-en-v1.5")
- `--variant <variant>` (optional; e.g., "onnx/model.onnx"; default from config)

**Behavior:**
1. Load embedding backend for specified model/variant
2. Detect model dimensions via backend
3. Call `ensure_vec0_table(dimensions)` to create/verify vec0 table
4. Register model if not already registered
5. Activate model
6. Re-embed all memories (delete chunks, re-chunk, embed, store chunks+embeddings)

**Example:**
```bash
nous re-embed --model BAAI/bge-base-en-v1.5 --variant onnx/model.onnx
```

**Output:**
```
Loading model: BAAI/bge-base-en-v1.5 (variant: onnx/model.onnx)
Model dimensions: 768
Ensuring vec0 table: 768 dimensions
Model registered: ID 2
Model activated: 2

Re-embedding 537 memories...
[=========================] 537/537 (100%)

Re-embedding complete:
  Memories processed: 537
  Chunks created: 1,234
  Embeddings stored: 1,234
```

#### `nous re-classify` — Re-Classify Memories

**Existing command.** No changes.

**Flags:**
- `--since <timestamp>` (optional; only re-classify memories created after this timestamp)

**Example:**
```bash
nous re-classify --since 2026-04-01T00:00:00Z
```

#### `nous category list` — List Categories

**Existing command.** Enhanced with `--format json|csv`.

**Flags:**
- `--source <source>` (optional; values: system, user, agent)
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous category list --source user
```

#### `nous category add <name>` — Add Category

**Existing command.** No changes.

**Flags:**
- `<name>` (category name, positional, required)
- `--parent <id>` (parent category ID)
- `--description <text>` (category description)

**Example:**
```bash
nous category add "Performance" --description "Performance-related decisions and measurements"
```

#### `nous category suggest <memory-id>` — Suggest Category

**New command.**

**Flags:**
- `<memory-id>` (positional, required)
- `--name <name>` (required; category name to create)
- `--description <text>` (optional; category description)
- `--parent <id>` (optional; parent category ID)

**Behavior:**
Creates a new agent-sourced category with the provided name and description, then assigns it to the specified memory. This is a convenience command that combines `category add` and memory update in one operation.

**Example:**
```bash
nous category suggest mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T --name "Performance" --description "Performance-related decisions"
```

**Output:**
```
Category created: 12 (Performance)
Memory updated: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
```

#### `nous import <file>` — Import Memories

**Existing command.** No changes.

**Flags:**
- `<file>` (JSON file path, positional, required)

**Example:**
```bash
nous import backup.json
```

#### `nous export` — Export Memories

**Existing command.** Enhanced with `--format csv`.

**Flags:**
- `--format <fmt>` (default: json; values: json, csv)

**Example:**
```bash
nous export --format json > backup.json
nous export --format csv > backup.csv
```

#### `nous rotate-key` — Rotate Encryption Key

**Existing command.** No changes.

**Flags:**
- `--new-key-file <path>` (optional; if not provided, generates a new key)

**Example:**
```bash
nous rotate-key --new-key-file ~/.config/nous/db-new.key
```

### Configuration Management

No new CLI commands for config management in this design. Configuration is managed exclusively through:
1. Direct editing of TOML file at `~/.config/nous/config.toml`
2. Environment variables
3. CLI flags (where applicable)

Precedence remains: CLI flags > env vars > config file > compiled defaults.

See Future Extensibility section for potential `nous config` commands.

### Server Operations

#### `nous serve` — Start MCP Server

**Existing command.** No changes.

**Flags:**
- `--transport <transport>` (default: stdio; values: stdio, http)
- `--port <n>` (default: 8377; only used with http transport)
- `--model <name>` (override config embedding model)
- `--variant <variant>` (override config embedding variant)

**Example:**
```bash
nous serve --transport stdio
nous serve --transport http --port 8377
```

### Import/Export

See Database Administration section above for `nous import` and `nous export` commands.

### OTLP Operations

**Note:** OTLP commands remain in the separate `nous-otlp` binary. This design does NOT integrate OTLP operations into the main `nous` CLI — the `nous-otlp` binary continues to operate independently as a specialized OTLP receiver and query tool.

The design adds query commands to expose stored telemetry data via the existing `nous-otlp` binary.

#### `nous-otlp serve` — Start OTLP Receiver

**Existing command.** No changes.

**Flags:**
- `--port <n>` (default: 4318)
- `--db <path>` (override config otlp.db_path)

**Example:**
```bash
nous-otlp serve --port 4318
```

#### `nous-otlp status` — OTLP Database Status

**Existing command.** Enhanced with `--format json`.

**Flags:**
- `--db <path>` (override config otlp.db_path)
- `--format <fmt>` (default: human; values: human, json)

**Example:**
```bash
nous-otlp status
```

**Output (default):**
```
db_path: /home/user/.cache/nous/otlp.db
log_events: 1,234
spans: 567
metrics: 89
```

#### `nous-otlp logs <session-id>` — Query Log Events

**New command.**

**Flags:**
- `<session-id>` (session ID, positional, required)
- `--limit <n>` (default: 100)
- `--offset <n>` (default: 0)
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous-otlp logs ses_01HQZX3Y7J8K9M0N1P2Q3R4S5T --limit 50
```

**Output (default):**
```
Timestamp             | Severity | Body
----------------------+----------+--------------------------------------
2026-04-26T23:45:12Z | INFO     | Memory stored: mem_01HQZX3Y7J8K9M...
2026-04-26T23:45:13Z | DEBUG    | Embedding computed: 384 dimensions
2026-04-26T23:45:14Z | INFO     | Search completed: 5 results
```

#### `nous-otlp spans <trace-id>` — Query Spans

**New command.**

**Flags:**
- `<trace-id>` (trace ID, positional, required)
- `--limit <n>` (default: 100)
- `--offset <n>` (default: 0)
- `--format <fmt>` (default: human; values: human, json, csv)

**Example:**
```bash
nous-otlp spans trc_01HQZX3Y7J8K9M0N1P2Q3R4S5T
```

**Output (default):**
```
Span ID | Name             | Kind     | Start Time           | Duration | Status
--------+------------------+----------+----------------------+----------+-------
span_01 | memory_store     | INTERNAL | 2026-04-26T23:45:12Z | 45ms     | OK
span_02 | embed_text       | INTERNAL | 2026-04-26T23:45:12Z | 30ms     | OK
span_03 | store_chunks     | INTERNAL | 2026-04-26T23:45:12Z | 10ms     | OK
```

## Flag and Argument Conventions

### Global Flags (Proposed)

Add global flags to all commands (except `serve`) to override config and control output:

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--config <path>` | string | `~/.config/nous/config.toml` | Override config file path |
| `--db <path>` | string | (from config) | Override memory database path |
| `--verbose`, `-v` | flag | false | Enable verbose diagnostic output to stderr |
| `--quiet`, `-q` | flag | false | Suppress non-essential output |
| `--format <fmt>` | string | human | Output format: human, json, csv |

**Implementation via clap derive:**

```rust
#[derive(Debug, Parser)]
#[command(name = "nous-mcp", about = "Nous memory system CLI")]
struct Cli {
    #[arg(long, global = true, help = "Config file path")]
    config: Option<PathBuf>,
    
    #[arg(long, global = true, help = "Database path")]
    db: Option<PathBuf>,
    
    #[arg(short, long, global = true, help = "Verbose output")]
    verbose: bool,
    
    #[arg(short, long, global = true, help = "Quiet mode")]
    quiet: bool,
    
    #[command(subcommand)]
    command: Command,
}
```

### Output Format Flag

Commands returning data should accept `--format <fmt>`:
- `human` — default, human-readable tables/lists
- `json` — structured JSON for machine parsing
- `csv` — CSV for data export and spreadsheet import

**Implementation:**

```rust
#[derive(Debug, Clone, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Csv,
}
```

### Positional Arguments

Use positional arguments for required identifiers:
- `nous recall <memory-id>`
- `nous update <memory-id>`
- `nous model info <model-id>`
- `nous relate <source> <target> <type>`

**Implementation:**

```rust
Update {
    #[arg(help = "Memory ID")]
    memory_id: String,
    
    #[arg(long, help = "New title")]
    title: Option<String>,
    
    // ...
}
```

### Named Flags

Use named flags (long form with optional short aliases) for optional parameters:
- `--title <text>`
- `--type <type>`
- `--importance <level>`
- `--limit <n>`, `-n <n>`

### Boolean Flags

Use presence-based boolean flags (no value required):
- `--hard` (hard delete)
- `--force` (skip confirmation)
- `--archived` (include archived)
- `--valid-only` (only valid memories)

**Implementation:**

```rust
Forget {
    memory_id: String,
    
    #[arg(long, help = "Hard delete (not archive)")]
    hard: bool,
}
```

### Enum Arguments

Use `ValueEnum` for constrained string arguments with auto-completion and validation:
- `--mode <mode>` (values: fts, semantic, hybrid)
- `--type <type>` (values: decision, convention, bugfix, architecture, fact, observation)
- `--importance <level>` (values: low, moderate, high)
- `--confidence <level>` (values: low, moderate, high)

**Implementation:**

```rust
#[derive(Debug, Clone, ValueEnum)]
enum SearchMode {
    Fts,
    Semantic,
    Hybrid,
}

Search {
    query: String,
    
    #[arg(long, value_enum, default_value = "hybrid")]
    mode: SearchMode,
}
```

### Help Text

Add help text to all arguments using `#[arg(help = "...")]`:

```rust
Search {
    #[arg(help = "Search query string")]
    query: String,
    
    #[arg(long, value_enum, default_value = "hybrid", help = "Search mode: fts, semantic, or hybrid")]
    mode: SearchMode,
    
    #[arg(long, help = "Filter by memory type")]
    memory_type: Option<MemoryType>,
    
    #[arg(long, short = 'n', default_value = "20", help = "Maximum results to return (1-100)")]
    limit: u32,
}
```

### Stdin Support

For commands accepting large text input (e.g., `--content`), support stdin via `-`:

```bash
cat content.txt | nous store --title "Migration strategy" --content - --type decision
```

**Implementation:**

```rust
fn read_content(content_arg: &str) -> Result<String, Box<dyn Error>> {
    if content_arg == "-" {
        let mut buffer = String::new();
        io::stdin().read_to_string(&mut buffer)?;
        Ok(buffer)
    } else {
        Ok(content_arg.to_string())
    }
}
```

### Confirmation Prompts

For destructive operations (`forget --hard`, `embedding reset`, `model switch` with dimension change), prompt for confirmation unless `--force` specified:

```rust
fn confirm(message: &str, force: bool) -> Result<bool, Box<dyn Error>> {
    if force {
        return Ok(true);
    }
    
    eprintln!("{}", message);
    eprint!("\nProceed? (y/N): ");
    io::stderr().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    
    Ok(input.trim().eq_ignore_ascii_case("y"))
}

// In the command handler:
if !confirm("Warning: ...", force)? {
    eprintln!("Aborted.");
    std::process::exit(130);  // Standard exit code for user cancellation (SIGINT)
}
```

## Output Formats

### Human Format (Default)

Readable tables, lists, and key-value output for terminal use:

**Tables** (for list commands like `search`, `model list`, `workspaces`, `tags`):
```
ID | Name                      | Variant            | Dims | Active
---+---------------------------+--------------------+------+-------
1  | BAAI/bge-small-en-v1.5    | onnx/model.onnx    | 384  | *
2  | BAAI/bge-base-en-v1.5     | onnx/model.onnx    | 768  |
```

**Lists** (for numbered results like `search`):
```
1. [mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T] Database connection pool size increased
   Type: decision | Importance: high | Created: 2026-04-26T23:45:12Z
```

**Key-value** (for single-record commands like `recall`, `model info`, `stats`):
```
ID: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
Title: Database connection pool size increased
Type: decision
Importance: high
```

**Confirmation prompts and warnings** go to stderr:
```
Warning: This will delete all 1,234 embeddings from the vec0 table.

Proceed? (y/N):
```

### JSON Format

Structured JSON for machine parsing and API integration:

**Single record:**
```json
{
  "memory_id": "mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T",
  "title": "Database connection pool size increased",
  "memory_type": "decision",
  "importance": "high",
  "confidence": "moderate",
  "tags": ["database", "performance"],
  "created_at": "2026-04-26T23:45:12Z"
}
```

**Multiple records:**
```json
{
  "query": "database connection pool",
  "mode": "hybrid",
  "results": [
    {
      "memory_id": "mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T",
      "title": "Database connection pool size increased",
      "rank": 0.92
    }
  ],
  "count": 2,
  "limit": 10
}
```

**Implementation:**

```rust
if format == OutputFormat::Json {
    let output = serde_json::to_string_pretty(&results)?;
    println!("{}", output);
} else {
    // human format
}
```

### CSV Format

CSV for data export and spreadsheet import. Headers on first line, quoted fields with commas/newlines:

```csv
memory_id,title,memory_type,importance,rank,tags,created_at
mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T,Database connection pool size increased,decision,high,0.92,"database,performance",2026-04-26T23:45:12Z
```

**Implementation:**

Use the `csv` crate for proper escaping:

```rust
if format == OutputFormat::Csv {
    let mut wtr = csv::Writer::from_writer(io::stdout());
    wtr.write_record(&["memory_id", "title", "memory_type", "importance", "rank", "tags", "created_at"])?;
    
    for result in results {
        wtr.write_record(&[
            &result.memory.id,
            &result.memory.title,
            &result.memory.memory_type.to_string(),
            &result.memory.importance.to_string(),
            &result.rank.to_string(),
            &result.tags.join(","),
            &result.memory.created_at.to_rfc3339(),
        ])?;
    }
    
    wtr.flush()?;
}
```

### Error Output

**All error messages and diagnostics go to stderr**, not stdout. This allows clean piping of stdout to files or other commands:

```rust
if let Err(e) = run_command(&cli) {
    eprintln!("Error: {}", e);
    std::process::exit(1);
}
```

**Progress indicators** (for long-running operations like `re-embed`) also go to stderr:

```rust
eprintln!("Re-embedding 537 memories...");
eprintln!("[=========================] 537/537 (100%)");
```

### Verbose Output

When `--verbose` is specified, write diagnostic information to stderr:

```rust
if cli.verbose {
    eprintln!("Loading config from: {}", config_path.display());
    eprintln!("Opening database: {}", db_path.display());
    eprintln!("Active model: {} (dimensions: {})", model.name, model.dimensions);
}
```

## Error Handling Strategy

### Exit Codes

| Code | Meaning | Example |
|------|---------|---------|
| 0 | Success | Command completed successfully |
| 1 | General error | Database error, IO error, internal error |
| 2 | Usage error | Invalid arguments, missing required flags, invalid enum value |
| 3 | Not found | Memory ID not found, model ID not found |
| 4 | Conflict | Duplicate model registration, dimension mismatch |
| 130 | User cancelled | User declined confirmation prompt for destructive operation |

**Implementation:**

```rust
fn main() {
    let exit_code = match run() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {}", e);
            // PROPOSED: Add NotFound, Validation, and Conflict variants to NousError
            match e.downcast_ref::<NousError>() {
                Some(NousError::Validation(_)) => 2,
                Some(NousError::NotFound(_)) => 3,
                Some(NousError::Conflict(_)) => 4,
                _ => 1,
            }
        }
    };
    
    std::process::exit(exit_code);
}
```

### Structured Error Messages

Error messages follow the pattern: `Error: [error-code] <message>`

**Examples:**

```
Error: [E001] Database not found: /home/user/.cache/nous/memory.db
Error: [E002] Memory not found: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
Error: [E003] Invalid memory type: "unknown" (expected: decision, convention, bugfix, architecture, fact, observation)
Error: [E004] Dimension mismatch: active model produces 768-dim embeddings but vec0 table expects 384 dims
Error: [E005] SQL query rejected: write operations not allowed (INSERT, UPDATE, DELETE, DROP, CREATE)
```

**Error code registry:**

| Code | Category | Description |
|------|----------|-------------|
| E001 | Database | Database file not found or inaccessible |
| E002 | NotFound | Resource not found (memory, model, category) |
| E003 | Validation | Invalid argument value or constraint violation |
| E004 | Conflict | Operation conflicts with current state |
| E005 | Security | Operation rejected for security reasons |
| E006 | Encryption | Encryption key error |
| E007 | Embedding | Embedding model error |
| E008 | Config | Configuration error |

**Implementation:**

PROPOSED extensions to `NousError` in `crates/nous-shared/src/error.rs`:

```rust
// PROPOSED: Add these variants to the existing NousError enum
#[derive(Debug, thiserror::Error)]
pub enum NousError {
    // ... existing variants ...
    
    #[error("[E001] Database error: {0}")]
    Database(String),
    
    #[error("[E002] Not found: {0}")]
    NotFound(String),  // PROPOSED
    
    #[error("[E003] Validation error: {0}")]
    Validation(String),  // PROPOSED
    
    #[error("[E004] Conflict: {0}")]
    Conflict(String),  // PROPOSED
    
    #[error("[E005] Security error: {0}")]
    Security(String),  // PROPOSED
    
    #[error("[E006] Encryption error: {0}")]
    Encryption(String),
    
    #[error("[E007] Embedding error: {0}")]
    Embedding(String),
    
    #[error("[E008] Configuration error: {0}")]
    Config(String),
}
```

### User-Friendly Error Context

Provide actionable guidance in error messages:

**Before:**
```
Error: Database error: unable to open database file
```

**After:**
```
Error: [E001] Database not found: /home/user/.cache/nous/memory.db

The database file does not exist. This is normal on first run.
The database will be created automatically when you run a command that needs it.

Try: nous status
```

**Implementation:**

```rust
fn enhance_error(e: NousError) -> String {
    match e {
        NousError::Database(msg) if msg.contains("unable to open") => {
            format!("{}\n\nThe database file does not exist. This is normal on first run.\nThe database will be created automatically when you run a command that needs it.\n\nTry: nous status", e)
        }
        NousError::NotFound(msg) if msg.contains("Memory not found") => {
            format!("{}\n\nCheck the memory ID and try again.\n\nTry: nous search <query>", e)
        }
        _ => e.to_string(),
    }
}
```

### Validation Errors

Catch validation errors early with detailed messages:

```rust
impl Command {
    fn validate(&self) -> Result<(), NousError> {
        match self {
            Command::Search { limit, .. } => {
                if *limit == 0 || *limit > 100 {
                    return Err(NousError::Validation(
                        "limit must be between 1 and 100".to_string()
                    ));
                }
            }
            Command::Store { title, content, .. } => {
                if title.is_empty() {
                    return Err(NousError::Validation(
                        "title cannot be empty".to_string()
                    ));
                }
                if content.is_empty() {
                    return Err(NousError::Validation(
                        "content cannot be empty".to_string()
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }
}
```

### Handling Permission Errors

If the database or config files have incorrect permissions, provide clear guidance:

```
Error: [E001] Database permission denied: /home/user/.cache/nous/memory.db

The database file exists but is not readable.

Fix: chmod 600 /home/user/.cache/nous/memory.db
```

## Configuration Management

### Configuration Sources

Configuration is loaded from three sources with the following precedence:

1. **CLI flags** (highest priority) — `--db`, `--config`
2. **Environment variables** — `NOUS_MEMORY_DB`, `NOUS_CONFIG_DIR`, etc.
3. **Config file** — `~/.config/nous/config.toml`
4. **Compiled defaults** (lowest priority)

### TOML Configuration File

Location: `~/.config/nous/config.toml` (XDG convention)

**Full schema:**

```toml
[memory]
db_path = "/home/user/.cache/nous/memory.db"

[embedding]
model = "BAAI/bge-small-en-v1.5"
variant = "onnx/model.onnx"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "/home/user/.cache/nous/otlp.db"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "/home/user/.config/nous/db.key"
```

**Default values** (from `crates/nous-mcp/src/config.rs:64-126`):

| Key | Default |
|-----|---------|
| `memory.db_path` | `~/.cache/nous/memory.db` |
| `embedding.model` | `BAAI/bge-small-en-v1.5` |
| `embedding.variant` | `onnx/model.onnx` |
| `embedding.chunk_size` | 512 |
| `embedding.chunk_overlap` | 64 |
| `otlp.db_path` | `~/.cache/nous/otlp.db` |
| `otlp.port` | 4318 |
| `classification.confidence_threshold` | 0.3 |
| `encryption.db_key_file` | `~/.config/nous/db.key` |

### Environment Variable Overrides

| Variable | Overrides | Example |
|----------|-----------|---------|
| `NOUS_MEMORY_DB` | `memory.db_path` | `/data/nous.db` |
| `NOUS_OTLP_DB` | `otlp.db_path` | `/data/otlp.db` |
| `NOUS_DB_KEY_FILE` | `encryption.db_key_file` | `/secrets/db.key` |
| `NOUS_DB_KEY` | Direct encryption key (bypasses key file) | `0123456789abcdef...` |
| `NOUS_CACHE_DIR` | XDG cache directory base | `/var/cache/nous` |
| `NOUS_CONFIG_DIR` | XDG config directory base | `/etc/nous` |

**Env var application** (from `crates/nous-mcp/src/config.rs:160-176`):

```rust
fn apply_env_overrides(config: &mut Config) {
    if let Ok(val) = env::var("NOUS_MEMORY_DB") {
        config.memory.db_path = val;
    }
    if let Ok(val) = env::var("NOUS_OTLP_DB") {
        config.otlp.db_path = val;
    }
    if let Ok(val) = env::var("NOUS_DB_KEY_FILE") {
        config.encryption.db_key_file = val;
    }
}
```

### CLI Flag Overrides

Global flags override both config file and env vars:

```bash
nous status --db /tmp/test.db --config /tmp/test-config.toml
```

**Implementation:**

```rust
fn load_config(cli: &Cli) -> Result<Config, ConfigError> {
    let config_path = cli.config.clone()
        .unwrap_or_else(|| config_path("config.toml"));
    
    let mut config = Config::load_or_create(&config_path)?;
    config.apply_env_overrides();
    
    // CLI flag overrides
    if let Some(db_path) = &cli.db {
        config.memory.db_path = db_path.to_string_lossy().to_string();
    }
    
    Ok(config)
}
```

### Encryption Key Resolution

The encryption key is resolved via `resolve_key()` in `crates/nous-shared/src/sqlite.rs:84-122`:

1. Check `NOUS_DB_KEY` env var (direct hex key)
2. Check key file path (default `~/.config/nous/db.key`)
3. If neither exists, auto-generate 32-byte hex key and write to key file (mode 0600)

**Auto-generated key file format:**

```
0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
```

**Security note:** The key file has mode 0600 (read/write for owner only). If permissions are incorrect, the CLI should error with E006.

### Config File Creation

If no config file exists, create a default one on first run:

```rust
// PROPOSED: Add this method to Config in crates/nous-mcp/src/config.rs
impl Config {
    pub fn load_or_create(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            let default_config = Config::default();
            let toml_str = toml::to_string_pretty(&default_config)?;
            
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            fs::write(path, toml_str)?;
            eprintln!("Created default config: {}", path.display());
        }
        
        let content = fs::read_to_string(path)?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }
}
```

### Precedence Example

Given:
- Config file: `memory.db_path = "/home/user/.cache/nous/memory.db"`
- Env var: `NOUS_MEMORY_DB=/data/nous.db`
- CLI flag: `--db /tmp/test.db`

**Result:** `/tmp/test.db` (CLI flag wins)

Given:
- Config file: `memory.db_path = "/home/user/.cache/nous/memory.db"`
- Env var: `NOUS_MEMORY_DB=/data/nous.db`
- CLI flag: (none)

**Result:** `/data/nous.db` (env var wins)

Given:
- Config file: `memory.db_path = "/home/user/.cache/nous/memory.db"`
- Env var: (none)
- CLI flag: (none)

**Result:** `/home/user/.cache/nous/memory.db` (config file wins)

## KV-Cache Management Commands

### Context: INI-012 KV-Cache Design

The KV-cache design (from the research phase) addresses decoder-based embedding models that require Key-Value cache tensors as ONNX session inputs. The design introduced:

- **Phase 1** (merged, PR #59): `ModelArch` enum (Encoder/Decoder), architecture-aware pooling (mean vs. last-token), max_tokens detection
- **Phase 2** (PR #60 in review): KV-cache tensor detection/construction, position IDs, named input binding, decoder output selection, `ensure_vec0_table()`, `reset_embeddings()`, register_model fix
- **Phase 3** (future): `rebuild_embeddings()`, E2E tests

The CLI design must expose cache operations and handle dimension mismatches when switching models.

### Core Operations

#### `ensure_vec0_table(dimensions: u32)` — Ensure Table Exists

**Behavior:**
- If vec0 table does not exist, create it with specified dimensions
- If vec0 table exists, query its dimensions via `vec0_info` pragma
- If dimensions match, do nothing
- If dimensions differ, return an error (caller must reset)

**CLI integration:** Called automatically by `nous re-embed` before embedding. Not exposed as a standalone CLI command.

**Implementation location:** `crates/nous-core/src/db.rs` (PROPOSED: to be added in INI-012 Phase 2)

#### `reset_embeddings()` — Drop and Recreate vec0 Table

**Behavior:**
- Drop the vec0 table
- Recreate it with dimensions matching the active model
- All embeddings are deleted
- Chunks in `memory_chunks` remain but have no embeddings until re-embed runs

**CLI integration:** Exposed as `nous embedding reset [--force]`

**Implementation location:** `crates/nous-core/src/db.rs` (PROPOSED: to be added in INI-012 Phase 2)

### CLI Commands

#### `nous embedding inspect` — Inspect Cache State

**Purpose:** Show current vec0 dimensions, embedding count, active model dimensions, and warn if they mismatch.

**Implementation:**

```rust
pub fn run_embedding_inspect(config: &Config, format: OutputFormat) -> Result<(), Box<dyn Error>> {
    let key = resolve_key()?;
    let db = MemoryDb::open(&config.memory.db_path, Some(&key))?;
    let active_model = db.active_model()?
        .ok_or("No active model")?;
    
    // Query vec0 dimensions via vec0_info pragma
    let vec0_dims = db.query_vec0_dimensions()?;
    let vec0_count = db.query_vec0_count()?;
    
    let status = if vec0_dims == active_model.dimensions {
        "OK (dimensions match)"
    } else {
        "ERROR — dimension mismatch!"
    };
    
    if format == OutputFormat::Json {
        let output = json!({
            "active_model": {
                "id": active_model.id,
                "name": active_model.name,
                "dimensions": active_model.dimensions,
                "chunk_size": active_model.chunk_size,
            },
            "vec0": {
                "dimensions": vec0_dims,
                "embeddings": vec0_count,
            },
            "status": status,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Active Model: {} ({})", active_model.id, active_model.name);
        println!("Model Dimensions: {}", active_model.dimensions);
        println!("Chunk Size: {}", active_model.chunk_size);
        println!("Chunk Overlap: {}", active_model.chunk_overlap);
        println!();
        println!("vec0 Table:");
        println!("  Dimensions: {}", vec0_dims);
        println!("  Embeddings: {} chunks", vec0_count);
        println!();
        println!("Status: {}", status);
        
        if vec0_dims != active_model.dimensions {
            eprintln!();
            eprintln!("The active model produces {}-dimensional embeddings, but vec0 table expects {} dimensions.", active_model.dimensions, vec0_dims);
            eprintln!("Run 'nous embedding reset' to recreate the vec0 table, or switch back to a {}-dim model.", vec0_dims);
        }
    }
    
    Ok(())
}
```

#### `nous embedding reset [--force]` — Reset Embedding Table

**Purpose:** Drop the vec0 table and recreate it with dimensions matching the active model. Prompts for confirmation unless `--force`.

**Implementation:**

```rust
pub fn run_embedding_reset(config: &Config, force: bool) -> Result<(), Box<dyn Error>> {
    let key = resolve_key()?;
    let db = MemoryDb::open(&config.memory.db_path, Some(&key))?;
    let active_model = db.active_model()?
        .ok_or("No active model")?;
    
    let vec0_count = db.query_vec0_count()?;
    
    if !confirm(&format!(
        "Warning: This will delete all {} embeddings from the vec0 table.\n\
         Chunks will remain in memory_chunks but have no embeddings until you run 'nous re-embed'.",
        vec0_count
    ), force)? {
        eprintln!("Aborted.");
        std::process::exit(130);
    }
    
    db.reset_embeddings()?;
    
    println!("vec0 table reset: {} embeddings deleted", vec0_count);
    println!("New dimensions: {} (matching active model {}: {})", 
        active_model.dimensions, active_model.id, active_model.name);
    
    Ok(())
}
```

#### `nous model switch <model-id> [--force]` — Switch Active Model

**Purpose:** Switch the active model and reset embeddings if dimensions differ.

**Behavior:**
1. Query current active model dimensions
2. Query target model dimensions
3. If dimensions match, just switch (deactivate current, activate target)
4. If dimensions differ:
   - Warn that vec0 will be reset
   - Prompt for confirmation unless `--force`
   - Deactivate current model
   - Activate target model
   - Call `reset_embeddings()` to recreate vec0 table with new dimensions

**Implementation:**

```rust
pub fn run_model_switch(
    config: &Config,
    model_id: i64,
    force: bool,
) -> Result<(), Box<dyn Error>> {
    let key = resolve_key()?;
    let db = MemoryDb::open(&config.memory.db_path, Some(&key))?;
    
    let target_model = db.get_model(model_id)?
        .ok_or_else(|| NousError::NotFound(format!("Model not found: {}", model_id)))?;
    
    let current_model = db.active_model()?;
    
    if let Some(current) = &current_model {
        if current.id == model_id {
            println!("Model {} is already active", model_id);
            return Ok(());
        }
        
        if current.dimensions != target_model.dimensions {
            let vec0_count = db.query_vec0_count()?;
            
            if !confirm(&format!(
                "Warning: Switching from model {} ({} dims) to model {} ({} dims) will reset all embeddings.\n\
                 This will delete {} chunks from the vec0 table.",
                current.id, current.dimensions,
                target_model.id, target_model.dimensions,
                vec0_count
            ), force)? {
                eprintln!("Aborted.");
                std::process::exit(130);
            }
            
            db.deactivate_model(current.id)?;
            db.activate_model(model_id)?;
            db.reset_embeddings()?;
            
            println!("Model switched: {} ({})", model_id, target_model.name);
            println!("Embeddings reset: vec0 table recreated with {} dimensions", target_model.dimensions);
        } else {
            db.deactivate_model(current.id)?;
            db.activate_model(model_id)?;
            
            println!("Model switched: {} ({})", model_id, target_model.name);
        }
    } else {
        db.activate_model(model_id)?;
        println!("Model activated: {} ({})", model_id, target_model.name);
    }
    
    Ok(())
}
```

#### `nous model list` — List Models with vec0 Info

**Enhancement:** Show vec0 dimensions alongside model dimensions to highlight mismatches.

**Output (with mismatch):**

```
ID | Name                      | Variant            | Dims | Max Tokens | Active | vec0 Dims
---+---------------------------+--------------------+------+------------+--------+----------
1  | BAAI/bge-small-en-v1.5    | onnx/model.onnx    | 384  | 512        |        | —
2  | BAAI/bge-base-en-v1.5     | onnx/model.onnx    | 768  | 512        | *      | 384 ⚠️

⚠️  Warning: Active model produces 768-dim embeddings but vec0 table has 384 dims.
    Run 'nous embedding inspect' for details.
```

### KV-Cache Implementation Notes

**From Phase 2 design (PR #60):**

- `ensure_vec0_table(dimensions)` checks if the vec0 table exists and has the correct dimensions. If not, creates/recreates it.
- `reset_embeddings()` drops the vec0 table and recreates it with dimensions from the active model.
- `register_model()` enhanced to detect model dimensions from the ONNX session instead of requiring them as an argument.

**CLI usage patterns:**

1. **First run:** `nous re-embed --model X --variant Y` creates vec0 table with detected dimensions
2. **Model switch (same dims):** `nous model switch <id>` just switches active model
3. **Model switch (different dims):** `nous model switch <id>` warns, prompts, resets vec0 on confirmation
4. **Manual reset:** `nous embedding reset` drops vec0 and recreates with active model dims
5. **Inspect state:** `nous embedding inspect` shows current dims and warns if mismatch exists

## Testing Strategy

### Testing Directive

E2E tests are a standing CEO directive. All CLI commands must have E2E tests covering the golden path and major error cases.

### Test Infrastructure

#### Test Database Setup

Each test uses an isolated database to prevent interference:

```rust
fn test_db_path() -> PathBuf {
    let pid = std::process::id();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    
    std::env::temp_dir()
        .join(format!("nous-test-{}-{}-{}.db", pid, timestamp, counter))
}

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);
```

**Test cleanup:**

```rust
struct TestDb {
    path: PathBuf,
    key: String,
}

impl TestDb {
    fn new() -> Self {
        let path = test_db_path();
        let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
        Self { path, key }
    }
    
    fn db(&self) -> MemoryDb {
        MemoryDb::open(&self.path, Some(&self.key)).unwrap()
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
```

#### Test Config

```rust
impl Config {
    pub fn for_test(db_path: PathBuf) -> Self {
        Config {
            memory: MemoryConfig { db_path },
            embedding: EmbeddingConfig {
                model: "BAAI/bge-small-en-v1.5".to_string(),
                variant: "onnx/model.onnx".to_string(),
                chunk_size: 512,
                chunk_overlap: 64,
            },
            otlp: OtlpConfig {
                db_path: PathBuf::from("/tmp/nous-test-otlp.db"),
                port: 4318,
            },
            classification: ClassificationConfig {
                confidence_threshold: 0.3,
            },
            encryption: EncryptionConfig {
                db_key_file: PathBuf::from("/tmp/nous-test.key"),
            },
        }
    }
}
```

#### Mock Embedding for Tests

Use `MockEmbedding` or `FixtureEmbedding` from `crates/nous-core/src/embed.rs`:

```rust
let embedding = MockEmbedding::new(384);
```

### Test Categories

#### Unit Tests (Existing)

Existing unit tests in `crates/nous-mcp/src/main.rs:219-476` cover:
- Clap parsing (`Cli::try_parse_from`)
- NousServer initialization
- MCP tool enumeration

Continue this pattern for new CLI commands.

#### Integration Tests

Test command execution end-to-end with a real database but mock embeddings:

**File:** `crates/nous-mcp/tests/cli_integration.rs`

```rust
#[test]
fn test_store_and_recall() {
    let test_db = TestDb::new();
    let config = Config::for_test(test_db.path.clone());
    let embedding = Arc::new(MockEmbedding::new(384));
    
    // Store a memory
    let result = run_store(
        &config,
        "Test memory".to_string(),
        "This is test content".to_string(),
        MemoryType::Fact,
        // ... other args
        &embedding,
    );
    assert!(result.is_ok());
    let memory_id = result.unwrap();
    
    // Recall it
    let result = run_recall(&config, &memory_id, OutputFormat::Json);
    assert!(result.is_ok());
}

#[test]
fn test_search_fts() {
    let test_db = TestDb::new();
    let config = Config::for_test(test_db.path.clone());
    let embedding = Arc::new(MockEmbedding::new(384));
    
    // Store several memories
    run_store(&config, "Database config", "...", MemoryType::Decision, &embedding).unwrap();
    run_store(&config, "API design", "...", MemoryType::Architecture, &embedding).unwrap();
    
    // Search
    let results = run_search(
        &config,
        "database",
        SearchMode::Fts,
        SearchFilters::default(),
        OutputFormat::Human,
    );
    assert!(results.is_ok());
    let results = results.unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].memory.title.contains("Database"));
}

#[test]
fn test_model_switch_dimension_mismatch() {
    let test_db = TestDb::new();
    let config = Config::for_test(test_db.path.clone());
    let db = test_db.db();
    
    // Register two models with different dimensions
    let model1 = db.register_model("model-384", "variant", 384, 512, 64).unwrap();
    let model2 = db.register_model("model-768", "variant", 768, 512, 64).unwrap();
    
    // Activate model1, create vec0 table
    db.activate_model(model1).unwrap();
    db.ensure_vec0_table(384).unwrap();
    
    // Switch to model2 (should require reset)
    let result = run_model_switch(&config, model2, true); // force=true
    assert!(result.is_ok());
    
    // Verify vec0 dimensions changed
    let dims = db.query_vec0_dimensions().unwrap();
    assert_eq!(dims, 768);
}

#[test]
fn test_embedding_inspect() {
    let test_db = TestDb::new();
    let config = Config::for_test(test_db.path.clone());
    let db = test_db.db();
    
    let model_id = db.register_model("test-model", "variant", 384, 512, 64).unwrap();
    db.activate_model(model_id).unwrap();
    db.ensure_vec0_table(384).unwrap();
    
    let result = run_embedding_inspect(&config, OutputFormat::Json);
    assert!(result.is_ok());
}
```

#### E2E Tests (Shell Scripts)

The justfile already has an `e2e` recipe. Expand it to cover all CLI commands:

**File:** `tests/e2e/test_cli.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail

# Setup
export NOUS_MEMORY_DB="/tmp/nous-e2e-test.db"
export NOUS_DB_KEY="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
rm -f "$NOUS_MEMORY_DB"

# Test: store + recall
echo "Testing store + recall..."
MEMORY_ID=$(cargo run --bin nous-mcp --quiet -- store \
  --title "E2E test memory" \
  --content "This is a test" \
  --type fact \
  --format json | jq -r '.memory_id')

cargo run --bin nous-mcp --quiet -- recall "$MEMORY_ID" --format json > /tmp/recall.json
TITLE=$(jq -r '.title' /tmp/recall.json)
[[ "$TITLE" == "E2E test memory" ]] || { echo "FAIL: title mismatch"; exit 1; }

echo "✓ store + recall"

# Test: search
echo "Testing search..."
cargo run --bin nous-mcp --quiet -- search "test" --mode fts --format json > /tmp/search.json
COUNT=$(jq -r '.count' /tmp/search.json)
[[ "$COUNT" -ge 1 ]] || { echo "FAIL: no results"; exit 1; }

echo "✓ search"

# Test: model management
echo "Testing model management..."
cargo run --bin nous-mcp --quiet -- model list --format json > /tmp/models.json
MODEL_COUNT=$(jq '. | length' /tmp/models.json)
[[ "$MODEL_COUNT" -ge 1 ]] || { echo "FAIL: no models"; exit 1; }

echo "✓ model management"

# Test: status
echo "Testing status..."
cargo run --bin nous-mcp --quiet -- status --format json > /tmp/status.json
MEMORY_COUNT=$(jq -r '.memories' /tmp/status.json)
[[ "$MEMORY_COUNT" -ge 1 ]] || { echo "FAIL: status broken"; exit 1; }

echo "✓ status"

# Test: export + import
echo "Testing export + import..."
cargo run --bin nous-mcp --quiet -- export --format json > /tmp/export.json
EXPORT_COUNT=$(jq '.memories | length' /tmp/export.json)
[[ "$EXPORT_COUNT" -ge 1 ]] || { echo "FAIL: export failed"; exit 1; }

rm -f "$NOUS_MEMORY_DB"
cargo run --bin nous-mcp --quiet -- import /tmp/export.json
cargo run --bin nous-mcp --quiet -- status --format json > /tmp/status2.json
IMPORT_COUNT=$(jq -r '.memories' /tmp/status2.json)
[[ "$IMPORT_COUNT" == "$EXPORT_COUNT" ]] || { echo "FAIL: import count mismatch"; exit 1; }

echo "✓ export + import"

# Cleanup
rm -f "$NOUS_MEMORY_DB" /tmp/*.json

echo "All E2E tests passed!"
```

**Run via justfile:**

```makefile
e2e:
    bash tests/e2e/test_cli.sh
```

### Test Scenarios

#### Golden Path Tests

| Command | Scenario |
|---------|----------|
| `store` | Store memory with all fields, verify via recall |
| `recall` | Recall existing memory, verify fields |
| `search` | Search FTS/semantic/hybrid, verify results |
| `update` | Update memory fields, verify changes |
| `forget` | Archive memory (soft), verify archived flag |
| `forget --hard` | Delete memory (hard), verify not found |
| `unarchive` | Restore archived memory, verify active |
| `relate` | Create relationship, verify via recall |
| `unrelate` | Remove relationship, verify removed |
| `model list` | List models, verify count |
| `model register` | Register new model, verify in list |
| `model activate` | Activate model, verify active flag |
| `model switch` | Switch models (same dims), verify active |
| `model switch` | Switch models (diff dims), verify reset |
| `embedding inspect` | Inspect vec0 state, verify dimensions |
| `embedding reset` | Reset vec0, verify embeddings deleted |
| `model switch` | Switch between 384-dim and 768-dim models, verify dimension change |
| `embedding inspect` | Query after model switch, verify mismatch warning |
| `category list` | List categories, verify tree structure |
| `category add` | Add category, verify in list |
| `export` | Export to JSON, verify structure |
| `import` | Import from JSON, verify count |
| `status` | Query stats, verify counts |
| `sql` | Execute SELECT query, verify results |

#### Error Case Tests

| Command | Scenario | Expected Exit Code |
|---------|----------|-------------------|
| `recall` | Memory not found | 3 |
| `update` | Memory not found | 3 |
| `forget` | Memory not found | 3 |
| `model activate` | Model not found | 3 |
| `search` | Invalid limit (0 or >100) | 2 |
| `store` | Empty title | 2 |
| `store` | Empty content | 2 |
| `sql` | Write query (INSERT) | 2 |
| `model switch` | Dimension mismatch, user declines | 130 (user cancelled) |
| `embedding reset` | User declines | 130 (user cancelled) |
| `embedding inspect` | vec0 table missing | 1 |
| `model switch` | Model not found | 3 |
| `embedding reset` | No active model | 1 |

### CI Integration

Add E2E tests to GitHub Actions CI:

```yaml
# .github/workflows/ci.yml
- name: Run E2E tests
  run: just e2e
```

## Future Extensibility

### Interactive Config Management

Future work could add CLI commands for config management:

**`nous config show [<key>]` — Display Configuration**

Show current configuration values, resolved from all sources (file, env vars, CLI flags).

**Example:**
```bash
nous config show
```

**Output:**
```
memory.db_path: /home/user/.cache/nous/memory.db (from config file)
embedding.model: BAAI/bge-small-en-v1.5 (from config file)
embedding.variant: onnx/model.onnx (from config file)
otlp.port: 4318 (default)
```

**`nous config set <key> <value>` — Update Configuration**

Write a configuration value to the TOML file.

**Example:**
```bash
nous config set embedding.chunk_size 1024
```

**`nous config reset <key>` — Reset to Default**

Remove a key from the config file, reverting to default value.

**Example:**
```bash
nous config reset embedding.chunk_size
```

### Plugin System

Future work could introduce a plugin system for custom CLI commands:

**Design sketch:**

- Plugins register via TOML manifest in `~/.config/nous/plugins/<name>/manifest.toml`
- Manifest declares command name, arguments, and executable path
- CLI dispatches to plugin executables, passing config and parsed args as JSON
- Plugins read stdin for input, write stdout for output, use exit codes for errors

**Example manifest:**

```toml
[plugin]
name = "export-markdown"
version = "0.1.0"
description = "Export memories as markdown files"

[[commands]]
name = "export-md"
executable = "./export-md"
args = [
  { name = "output-dir", type = "string", required = true },
  { name = "workspace", type = "string", required = false },
]
```

**CLI integration:**

```bash
nous export-md /tmp/markdown-export --workspace /home/user/project
```

**Plugin interface:**

```json
{
  "config": { ... },
  "args": {
    "output_dir": "/tmp/markdown-export",
    "workspace": "/home/user/project"
  }
}
```

### Custom Commands via Config

Allow users to define custom commands as shell scripts in config:

```toml
[commands.backup]
script = """
#!/bin/bash
nous export --format json | gzip > ~/backups/nous-$(date +%Y%m%d).json.gz
"""

[commands.sync]
script = """
#!/bin/bash
nous export --format json | ssh backup-server 'cat > /backups/nous.json'
"""
```

**CLI integration:**

```bash
nous backup
nous sync
```

### MCP Tool to CLI Parity

Current state: 15 MCP tools, subset exposed as CLI commands. Future work should achieve 1:1 parity:

| MCP Tool | CLI Command | Status |
|----------|-------------|--------|
| `memory_store` | `nous store` | ✓ Designed |
| `memory_recall` | `nous recall` | ✓ Designed |
| `memory_search` | `nous search` | ✓ Designed |
| `memory_context` | `nous context` | ✓ Designed |
| `memory_forget` | `nous forget` | ✓ Designed |
| `memory_unarchive` | `nous unarchive` | ✓ Designed |
| `memory_update` | `nous update` | ✓ Designed |
| `memory_relate` | `nous relate` | ✓ Designed |
| `memory_unrelate` | `nous unrelate` | ✓ Designed |
| `memory_category_suggest` | `nous category suggest` | ✓ Designed |
| `memory_workspaces` | `nous workspaces` | ✓ Designed |
| `memory_tags` | `nous tags` | ✓ Designed |
| `memory_stats` | `nous status` | ✓ Exists |
| `memory_schema` | `nous schema` | ✓ Designed |
| `memory_sql` | `nous sql` | ✓ Designed |

### OTLP CLI Expansion

The OTLP component currently has minimal CLI surface (serve, status, logs, spans). Future expansion:

- `nous-otlp metrics <name>` — Query metrics by name
- `nous-otlp traces <session-id>` — Query all spans for a session
- `nous-otlp export` — Export telemetry to JSON
- `nous-otlp import` — Import telemetry from JSON
- `nous-otlp retention <duration>` — Set retention policy (auto-delete old data)

### Advanced Search Features

- **Semantic similarity threshold:** `nous search "query" --mode semantic --similarity-threshold 0.8`
- **Faceted search:** `nous search "query" --facet-by type,importance --format json`
- **Saved searches:** Store frequent searches in config, recall via name: `nous search --saved recent-decisions`

### Interactive Mode

For exploratory workflows, add an interactive REPL:

```bash
nous repl
```

**REPL features:**
- Command history
- Auto-completion
- Multi-line input
- Search result navigation (next/prev)
- Recall from search results by number

**Example session:**

```
nous> search database --mode hybrid
1. [mem_01HQZX3Y...] Database connection pool size increased
2. [mem_01HQZX3Z...] Connection pool monitoring added

nous> recall 1
ID: mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T
Title: Database connection pool size increased
...

nous> update mem_01HQZX3Y7J8K9M0N1P2Q3R4S5T --importance high
Memory updated.

nous> exit
```

### Database Vacuum

Add `nous vacuum` command to reclaim space after deleting memories:

```bash
nous vacuum
```

**Implementation:**

```rust
pub fn run_vacuum(config: &Config) -> Result<(), Box<dyn Error>> {
    let key = resolve_key()?;
    let db = MemoryDb::open(&config.memory.db_path, Some(&key))?;
    db.vacuum()?;
    println!("Database vacuumed successfully.");
    Ok(())
}
```

### Performance Profiling

Add `--profile` flag to measure command execution time and resource usage:

```bash
nous search "database" --profile
```

**Output:**

```
Results: 5 memories

Profiling:
  Database open: 12ms
  FTS search: 45ms
  Semantic search: 120ms
  Result fusion: 8ms
  Total: 185ms
```
