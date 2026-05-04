# Agent Memory Integration

**Status:** Draft  
**Date:** 2026-05-02  
**Workstream:** NOUS-060

---

## 1. Current State

Nous provides a persistent memory system (`crates/nous-core/src/memory/`) backed by two SQLite databases: a FTS pool (`memory-fts.db`) for structured data and full-text search, and a Vec pool (`memory-vec.db`) for vector embeddings. The memory infrastructure is production-ready but operates independently of the agent system — agents must explicitly invoke memory operations through daemon APIs; there is no automatic memory integration at the agent form or runtime level.

### 1.1 Memory Tables (FTS Pool)

| Table | Migration | Purpose | Key Columns |
|-------|-----------|---------|-------------|
| `memories` | 016 | Core memory storage | `id`, `workspace_id`, `agent_id`, `title`, `content`, `memory_type` (decision/convention/bugfix/architecture/fact/observation), `importance` (low/moderate/high), `topic_key`, `valid_from`, `valid_until`, `archived`, `session_id` (added m020) |
| `memories_fts` | 016 | FTS5 virtual table | Indexes `title \|\| content \|\| memory_type \|\| topic_key`. Auto-synced via INSERT/DELETE/UPDATE triggers. Tokenizer: porter unicode61. |
| `memory_relations` | 016 | Directed edges between memories | `source_id`, `target_id`, `relation_type` (supersedes/conflicts_with/related/compatible/scoped/not_conflict). UNIQUE on (source, target, type). |
| `memory_access_log` | 016 | Tracks memory access for decay | `memory_id`, `access_type` (recall/search/context), `session_id`, `accessed_at` |
| `agent_workspace_access` | 016 | ACL: which agents access which workspaces | `agent_id`, `workspace_id` (composite PK) |
| `memory_sessions` | 020 | Session lifecycle tracking | `id`, `agent_id`, `project`, `started_at`, `ended_at`, `summary` |
| `search_events` | 021 | Search analytics | `query_text`, `search_type` (fts/vector/hybrid/fts5_fallback), `result_count`, `latency_ms`, `workspace_id`, `agent_id` |

The `memories` table retains a legacy `embedding BLOB` column (added m018) that is dual-written but will be removed once all embeddings migrate to the Vec pool.

### 1.2 Memory Tables (Vec Pool)

| Table | Migration | Purpose | Key Columns |
|-------|-----------|---------|-------------|
| `memory_embeddings` | vec_001 | vec0 virtual table for KNN search | `memory_id TEXT PRIMARY KEY`, `embedding float[384]` |
| `memory_chunks` | vec_002 | Chunk storage for long documents | `id`, `memory_id`, `content`, `chunk_index`, `start_offset`, `end_offset` |

The Vec pool uses rusqlite with the sqlite-vec extension (v0.1.9). The FTS pool uses SQLx async with max 5 connections.

### 1.3 Memory Operations

Core operations in `crates/nous-core/src/memory/mod.rs`:

| Function | Line | Description |
|----------|------|-------------|
| `save_memory` | 277 | Insert or upsert (via `topic_key` match). Defaults `workspace_id` to 'default'. |
| `search_memories` | 420 | FTS5 MATCH query with filters (workspace, agent, type, importance), BM25-ranked. |
| `get_context` | 491 | Recent memories by `created_at DESC`. No semantic search — pure recency-based context window. |
| `search_similar` | 767 | KNN via vec0 MATCH. Returns top-K by L2 distance converted to similarity. |
| `search_hybrid` | 1127 | FTS + vector fusion via Reciprocal Rank Fusion (RRF, k=60). |
| `search_hybrid_filtered` | 1148 | Hybrid search with workspace, agent, memory_type filters applied post-KNN. |
| `relate_memories` | 536 | Create directed relation. Supersedes auto-sets `valid_until` on target. |
| `run_importance_decay` | 627 | Decay importance (high→moderate→low) for memories not accessed within threshold days. |
| `session_start` / `session_end` | 900-940 | Session lifecycle. `session_end` auto-creates an Observation memory with `topic_key` = `session/{id}`. |
| `save_prompt` | 971 | Save user prompt as low-importance Observation memory. |
| `store_embedding` | 730 | Dual-write: legacy BLOB + vec0. |
| `store_chunks` | 1077 | Store chunked content + per-chunk embeddings. |

### 1.4 Embedding Infrastructure

`crates/nous-core/src/memory/embed.rs` defines an `Embedder` trait with two implementations:

| Implementation | Purpose | Dimensions |
|----------------|---------|-----------|
| `OnnxEmbeddingModel` | Production. Runs all-MiniLM-L6-v2 via ONNX Runtime (`ort` 2.0.0-rc.12). Loads from `~/.nous/models/all-MiniLM-L6-v2.onnx`. MAX_SEQ_LEN=512. Mean pooling + L2 normalization. | 384 |
| `MockEmbedder` | Tests. Deterministic hash-based vectors. | 384 |

The dimension constant (`EMBEDDING_DIMENSION = 384`, `db/pool.rs:12`) is compile-time. Changing embedding models requires updating this constant and running a vec migration to recreate `memory_embeddings` with the new dimension.

Chunking (`memory/chunk.rs:13-26`): default 256 tokens per chunk, 64 token overlap, whitespace tokenizer.

Reranking (`memory/rerank.rs:9-47`): Reciprocal Rank Fusion (RRF) merges FTS BM25 ranks with vector similarity scores. Default k=60.

### 1.5 LLM Provider Integration

The platform uses `rig-core` 0.36 and `rig-bedrock` 0.4.5 for LLM completions. The LlmClient type alias (`nous-daemon/src/llm_client.rs:4`) is `rig_bedrock::client::Client`. Default model: `anthropic.claude-sonnet-4-20250514-v1:0` (Bedrock ARN).

Rig provides:
- `EmbeddingModel` trait (not yet wired to nous memory system — embedding is local ONNX only).
- `VectorStoreIndex` trait (auto-implements as Tool for RAG).
- `.dynamic_context(N, index)` on agents (injects top-N retrieval results before each prompt).

### 1.6 Agent Form System

Agents are defined via TOML files (agent forms) that follow the [Agent Skills Specification](https://agentskills.io) conventions for declarative agent capabilities. The Rust implementation is in `crates/nous-core/src/agents/definition.rs:1-85`. Four sections:

```toml
[agent]             # REQUIRED: name, type (engineer/manager/director/senior-manager), version
[process]           # OPTIONAL: type (claude/shell/http/sandbox), spawn_command, working_dir, auto_restart
[skills]            # OPTIONAL: refs (Vec<String> of skill names)
[metadata]          # OPTIONAL: model, timeout, tags
```

The system has no `[memory]` section. No fields exist for: memory scope, memory backend preference, context window configuration, retrieval strategy, or memory types the agent should create/access.

### 1.7 Gap Analysis

| Capability | Current State | Gap |
|-----------|---------------|-----|
| Memory-aware agent forms | Agent TOML has no memory config | Agents cannot declare memory scope (workspace, session, per-agent), retrieval strategy, or context window size |
| Automatic context injection | Agents manually call `get_context` or `search_hybrid_filtered` | No rig integration for `.dynamic_context()` with memory system |
| Session-scoped memory | `memory_sessions` table exists, `session_id` column in `memories` | No session lifecycle tied to agent spawn/cleanup. `session_start` is manually invoked. |
| Agent-scoped retrieval | `agent_id` is an optional filter in search | No isolation boundary — agents can read all memories in their workspace |
| Shared memory pools | `workspace_id` provides coarse scoping | No fine-grained sharing (team memory vs agent memory vs conversation memory) |
| Memory types aligned to agent behavior | 6 types exist (Decision/Convention/Bugfix/Architecture/Fact/Observation) | No "Skill" or "Preference" type. No procedural memory (tool usage patterns). |
| Automatic memory creation | Only `save_prompt` and `session_end` auto-create | No hooks for agents to auto-save decisions, conventions, or discoveries |

Agents today interact with memory as a separate subsystem requiring explicit daemon calls. There is no declarative layer in agent forms to express memory needs or behavior.

## 2. Memory Architecture

Agent memory follows a tiered model inspired by cognitive science and validated across production agent systems (LangChain, CrewAI, Mem0). Four memory types serve distinct roles in agent behavior.

### 2.1 Memory Type Taxonomy

| Memory Type | Definition | Lifecycle | Storage | Nous Mapping |
|-------------|-----------|-----------|---------|--------------|
| **Working Memory** | Conversation context within a session. Single-shot facts, intermediate reasoning steps. | Session-scoped. Cleared on session end unless explicitly saved. | In-memory buffer (rig conversation history). Optional save to `memories` with `MemoryType::Observation` + `session_id`. | `MemorySession` table tracks sessions. `save_prompt()` saves user input as Observation. |
| **Episodic Memory** | Specific events and interactions. Bug fixes, incidents, user feedback, search queries. | Persistent. Decays via importance. Archivable. | `memories` table with `MemoryType::Observation`, `MemoryType::Bugfix`. `memory_access_log` tracks access. | `session_end` summary auto-creates episodic memory. Access log enables future decay/consolidation. |
| **Semantic Memory** | Persistent knowledge, conventions, architecture decisions, facts. | Long-lived. High-importance entries resist decay. | `memories` table with `MemoryType::Decision`, `MemoryType::Convention`, `MemoryType::Architecture`, `MemoryType::Fact`. Embeddings in `memory_embeddings` + `memory_chunks`. | Core memory system. FTS5 + vec0 hybrid search. |
| **Procedural Memory** | Learned skills, tool usage patterns, task workflows, preferences. | Persistent. Updated via reinforcement (which tools succeeded, which failed). | Not explicitly modeled. Closest match: `MemoryType::Convention` + `skills` refs in agent forms. | **Gap** — no dedicated type. Conventions can encode tool preferences ("always use X for Y task") but tool success/failure telemetry is not captured. |

### 2.2 Memory Storage Backend

Nous memory is SQLite-native by design. Two databases split relational + vector concerns:

```
memory-fts.db (SqlitePool, SQLx async, 5 connections max)
  ├── memories                 — core table, indexed by workspace_id + agent_id
  ├── memories_fts             — FTS5 virtual, BM25 ranking
  ├── memory_relations         — directed graph edges
  ├── memory_access_log        — decay tracking
  ├── agent_workspace_access   — ACL
  ├── memory_sessions          — session lifecycle
  └── search_events            — analytics

memory-vec.db (rusqlite sync, single Arc<Mutex<Connection>>)
  ├── memory_embeddings (vec0) — KNN via sqlite-vec 0.1.9
  ├── memory_chunks            — chunked content for long documents
  └── vec_schema_version       — migration tracker
```

SQLite is the authoritative store. No external vector DB is required for prototype phase. `docs/design/vector-db-embeddings.md` outlines future Qdrant/LanceDB support via rig abstractions, but Phase 1 agent-memory integration assumes sqlite-vec.

### 2.3 Memory Scoping Model

Memories are scoped by a 3-level hierarchy:

```
workspace_id
  └── agent_id (optional)
       └── session_id (optional)
```

| Scope | Use Case | Query Pattern | Example |
|-------|----------|---------------|---------|
| **Workspace-wide** | Shared project knowledge. All agents in a codebase access the same conventions. | `workspace_id = X`, `agent_id IS NULL` | Project coding standards, architecture decisions. |
| **Agent-scoped** | Agent-specific learnings. An agent's memory of its own past actions. | `workspace_id = X AND agent_id = Y` | A code-review agent's history of bugs it has found. |
| **Session-scoped** | Conversation context. Ephemeral unless elevated. | `workspace_id = X AND agent_id = Y AND session_id = Z` | User corrections within a debugging session. |
| **Cross-agent shared** | Team memory. Multiple agents read/write to a shared pool. | `workspace_id = X AND agent_id IN (Y1, Y2, Y3)` | Incident response — multiple agents contribute findings. |

The `agent_workspace_access` table enforces ACLs: an agent can only read memories from workspaces it has been granted access to via `grant_workspace_access()` (`memory/mod.rs:667`).

Session memories are tagged via `memory_sessions.id` → `memories.session_id`. When `session_end()` is called (`memory/mod.rs:940`), a summary Observation memory is auto-created with `topic_key = session/{id}`, serving as a compressed representation of the session for future retrieval.

### 2.4 Embedding Strategy

Embeddings are f32 vectors of dimension 384 (all-MiniLM-L6-v2). Two embedding paths exist:

| Path | When | Where |
|------|------|-------|
| **Full-content embedding** | On `save_memory()` → `store_embedding()` | Dual-written to `memories.embedding BLOB` (legacy) + `memory_embeddings` vec0 table. Used by `search_similar()` for KNN. |
| **Chunk embeddings** | On `save_memory()` for content >256 tokens → `store_chunks()` → `store_chunk_embedding()` | Each chunk gets a vec0 entry in `memory_chunks` + per-chunk embedding. Enables retrieval of sub-document passages. |

Chunking (`memory/chunk.rs:17`): 256 tokens per chunk, 64 token overlap, whitespace tokenizer. Overlap ensures semantic continuity across chunk boundaries.

The embedding model is local ONNX (no network calls). `OnnxEmbeddingModel` (`memory/embed.rs:11`) loads `~/.nous/models/all-MiniLM-L6-v2.onnx` or `$NOUS_MODEL_PATH/all-MiniLM-L6-v2.onnx`. Mean pooling over token embeddings, L2 normalization to unit length.

### 2.5 Memory Types Extended

The six current memory types are sufficient for Phase 1 agent integration, but two gaps exist:

| Type | Current Mapping | Gap |
|------|----------------|-----|
| **Skill Memory** | Stored in agent forms (`[skills].refs`), not in `memories` table. | Skills are static references to Markdown files. No runtime learning ("agent learned to use tool X for task Y"). |
| **Preference Memory** | No dedicated type. Stored as `MemoryType::Convention` with importance=low. | User preferences (model choice, verbosity level, output format) have no explicit schema. Retrieval mixes them with coding conventions. |

Proposal: add two new `MemoryType` variants in a future phase:

```rust
pub enum MemoryType {
    // ... existing 6 types ...
    Skill,       // Tool usage patterns, task workflows
    Preference,  // User or agent configuration preferences
}
```

This is deferred to Phase 2 — Phase 1 uses existing types. Conventions can encode tool preferences; Observations can capture skill application outcomes.

## 3. Agent-Memory Integration Points

Agent-memory integration happens at three layers: agent form (declarative config), runtime (automatic context injection), and API (explicit memory operations).

### 3.1 Declarative Configuration (Agent Forms)

Agents declare their memory requirements in a new `[memory]` section in TOML form files (`~/.config/nous/agents/*.toml`):

```toml
[agent]
name    = "code-reviewer"
type    = "engineer"
version = "2.0.0"

[memory]
scope           = "agent"              # workspace | agent | session | shared:<agent-ids>
retrieval       = "hybrid"             # fts | vector | hybrid | recency
context_size    = 5                    # max memories injected per prompt
auto_save       = ["decision", "convention"]  # MemoryTypes to auto-save from agent output
importance_default = "moderate"        # low | moderate | high
session_tracking = true                # auto-start session on spawn, auto-end on cleanup
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `scope` | enum | `"agent"` | Memory visibility boundary. `workspace` = all agents in workspace. `agent` = only this agent. `session` = only current session. `shared:<id1>,<id2>` = explicit agent list. |
| `retrieval` | enum | `"hybrid"` | Retrieval strategy. `fts` = BM25 only. `vector` = KNN only. `hybrid` = RRF fusion. `recency` = `get_context()` by `created_at DESC`. |
| `context_size` | u32 | 5 | Max memories injected into LLM context before each prompt. 0 = no automatic injection (manual only). |
| `auto_save` | Vec\<MemoryType\> | `[]` | Memory types to auto-extract from agent completions. LLM response is parsed for structured markers; matching content is saved as memories. |
| `importance_default` | Importance | `"moderate"` | Default importance for auto-saved memories. |
| `session_tracking` | bool | `false` | If true, `session_start()` is called on `agent spawn`, `session_end()` on agent cleanup. Session summary is auto-saved. |

Backward compatibility: if `[memory]` section is absent, the agent has no automatic memory integration — it behaves identically to pre-integration agents.

### 3.2 Automatic Context Injection (Rig Integration)

Rig agents support `.dynamic_context(N, index)` where `index` is a `VectorStoreIndex` implementation. The index is queried before each prompt; top-N results are injected into the LLM context.

Nous memory becomes a dynamic context source:

```rust
// crates/nous-daemon/src/agent_memory.rs (new file)

use rig::vector_store::{VectorStoreIndex, VectorSearchRequest, VectorStoreError};
use crate::memory::{search_hybrid_filtered, DbPools, Embedder};

pub struct NousMemoryIndex {
    pools: DbPools,
    embedder: Arc<dyn Embedder>,
    workspace_id: String,
    agent_id: Option<String>,
    memory_types: Vec<MemoryType>,  // filter for specific types
}

impl VectorStoreIndex for NousMemoryIndex {
    type Filter = ();  // no rig-specific filter; we use workspace/agent scoping

    async fn top_n<T: for<'a> Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String, T)>, VectorStoreError> {
        // 1. Embed query text via self.embedder
        // 2. Call search_hybrid_filtered(pools, query_embedding, workspace_id, agent_id, memory_types, limit=req.samples)
        // 3. Map SimilarMemory results to (score, id, deserialized content)
        // 4. Return top N
    }
}
```

Agent spawn flow integrates this:

```rust
// In nous-daemon agent spawn logic:

let memory_config = agent_def.memory.unwrap_or_default();
let memory_index = NousMemoryIndex::new(
    app_state.pools.clone(),
    app_state.embedder.clone(),
    workspace_id,
    if memory_config.scope == "agent" { Some(agent_id) } else { None },
    memory_config.auto_save.clone(),  // filter by memory types if specified
);

let rag_agent = llm_client.agent(model)
    .preamble(agent_def.agent.description.unwrap_or_default())
    .dynamic_context(memory_config.context_size as usize, memory_index)
    .build();
```

Retrieval strategy (`memory_config.retrieval`):
- `"hybrid"` → `search_hybrid_filtered()` (FTS + vec0 + RRF)
- `"fts"` → `search_memories()` (BM25 only)
- `"vector"` → `search_similar()` (vec0 KNN only)
- `"recency"` → `get_context()` (no search, pure recency by `created_at DESC`)

The rig `.dynamic_context()` machinery calls `NousMemoryIndex::top_n()` before each prompt. The LLM sees injected memories as additional context without the agent manually invoking retrieval.

### 3.3 Automatic Memory Extraction

When `auto_save` is non-empty, agent completions are parsed for structured memory markers. Two extraction modes:

#### Structured Marker Extraction

Agent output is scanned for memory blocks:

```
MEMORY[type=decision, importance=high, title="Use RRF for hybrid search"]
We will use Reciprocal Rank Fusion (k=60) to merge FTS and vector results.
Rationale: BM25 and KNN rank documents differently; RRF normalizes ranks without score calibration.
END_MEMORY
```

Extraction logic (`crates/nous-daemon/src/agent_memory.rs`):

```rust
pub async fn extract_and_save_memories(
    response: &str,
    agent_id: &str,
    workspace_id: &str,
    auto_save_types: &[MemoryType],
    pools: &DbPools,
) -> Result<Vec<String>, NousError> {
    let mut saved_ids = vec![];
    for block in parse_memory_blocks(response) {
        if !auto_save_types.contains(&block.memory_type) {
            continue;  // skip types not in auto_save list
        }
        let req = SaveMemoryRequest {
            workspace_id: Some(workspace_id.to_string()),
            agent_id: Some(agent_id.to_string()),
            title: block.title,
            content: block.content,
            memory_type: block.memory_type,
            importance: Some(block.importance),
            topic_key: block.topic_key,
            valid_from: None,
            valid_until: None,
        };
        let memory = save_memory(&pools.fts, req).await?;
        saved_ids.push(memory.id);
    }
    Ok(saved_ids)
}
```

#### LLM-Based Extraction (Future)

For agents without structured markers, a secondary LLM call extracts memories:

```
System: Extract important facts, decisions, and conventions from the following agent response.
Output JSON array: [{"type": "decision", "title": "...", "content": "..."}]

Agent response: {full_response}
```

The extracted JSON is saved as memories. This adds latency (extra LLM call per response) but enables memory extraction from free-form agent output.

Phase 1 implements structured markers only. LLM-based extraction is deferred.

### 3.4 Explicit Memory API

Agents can directly invoke memory operations via tools. These tools are defined in the daemon and exposed via rig's tool system:

| Tool | Operation | Parameters | Example Use Case |
|------|-----------|------------|------------------|
| `memory_search` | Hybrid search | query (text), limit (u32), memory_types (Vec\<MemoryType\>) | "Search for prior decisions about error handling" |
| `memory_save` | Save memory | title, content, memory_type, importance, topic_key | Agent saves a convention after solving a recurring problem |
| `memory_update` | Update memory | id, title, content, importance, archived | Revise a stale architecture decision |
| `memory_relate` | Create relation | source_id, target_id, relation_type | Link a new decision that supersedes an old one |
| `memory_context` | Get recent context | limit, topic_key | Retrieve recent session history for continuation |

Tool definitions use rig's `#[derive(Tool)]` or manual `Tool` trait implementation:

```rust
// crates/nous-daemon/src/tools/memory.rs (new file)

#[derive(Deserialize)]
pub struct MemorySearchArgs {
    query: String,
    limit: Option<u32>,
    memory_types: Option<Vec<String>>,  // comma-separated
}

pub struct MemorySearchTool {
    pools: DbPools,
    embedder: Arc<dyn Embedder>,
    workspace_id: String,
    agent_id: String,
}

impl Tool for MemorySearchTool {
    const NAME: &'static str = "memory_search";

    type Args = MemorySearchArgs;
    type Output = Vec<Memory>;
    type Error = NousError;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search agent memory for relevant facts, decisions, and conventions".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural language query"},
                    "limit": {"type": "integer", "description": "Max results (default 5)"},
                    "memory_types": {"type": "array", "items": {"type": "string"}, "description": "Filter by memory type"}
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let types = args.memory_types.map(|v| v.iter().map(|s| s.parse()).collect::<Result<Vec<_>, _>>()).transpose()?;
        let embedding = self.embedder.embed(&[&args.query])?[0].clone();
        let results = search_hybrid_filtered(
            &self.pools,
            &embedding,
            &self.workspace_id,
            Some(&self.agent_id),
            types.as_deref(),
            args.limit.unwrap_or(5),
        ).await?;
        Ok(results.into_iter().map(|sm| sm.memory).collect())
    }
}
```

These tools are registered at agent spawn time if the agent's TOML declares `[tools].refs = ["memory_search", "memory_save", ...]`.

### 3.5 Session Lifecycle Integration

When `memory.session_tracking = true`, agent spawn triggers `session_start()`:

```rust
// In nous-daemon agent spawn:

let session_id = if memory_config.session_tracking {
    Some(memory::session_start(&pools.fts, agent_id, detect_current_project().name).await?.id)
} else {
    None
};

// Store session_id in agent runtime context for use in memory operations
```

Agent cleanup (on shutdown, crash, or explicit stop) triggers `session_end()`:

```rust
// In agent cleanup handler:

if let Some(session_id) = agent_runtime.session_id {
    memory::session_end(&pools.fts, &session_id, "Session ended by agent cleanup").await?;
}
```

`session_end()` (`memory/mod.rs:940`) writes a summary Observation memory with `topic_key = session/{session_id}`, `importance = moderate`, and `agent_id` set. This summary is retrievable via `memory_search("session summary")` or `get_context()` for future sessions.

### 3.6 Memory Scoping Rules

Scope enforcement at retrieval time:

| Scope | Enforced By | Query Filter |
|-------|-------------|--------------|
| `workspace` | `workspace_id` filter in SQL WHERE clause | `workspace_id = ?` |
| `agent` | `workspace_id` + `agent_id` filter | `workspace_id = ? AND agent_id = ?` |
| `session` | `workspace_id` + `agent_id` + `session_id` | `workspace_id = ? AND agent_id = ? AND session_id = ?` |
| `shared:<ids>` | `workspace_id` + `agent_id IN (...)` | `workspace_id = ? AND agent_id IN (?, ?, ...)` |

The `agent_workspace_access` ACL is checked before any memory operation. If an agent tries to read memories from a workspace it hasn't been granted access to, the query returns empty results (no error thrown — agents should not learn about workspaces they cannot access).

### 3.7 Integration Flow Diagram

```
Agent Spawn
  ├── Load AgentDefinition (TOML) → extract [memory] section
  ├── If session_tracking=true → session_start() → get session_id
  ├── Build NousMemoryIndex (scope, retrieval, types)
  ├── Construct rig agent with .dynamic_context(context_size, index)
  ├── Register memory tools (memory_search, memory_save, etc.) if in [tools].refs
  └── Return AgentRuntime (includes session_id, memory config)

Agent Prompt
  ├── Rig calls NousMemoryIndex::top_n(query) [automatic context injection]
  ├── Memories retrieved → injected into LLM prompt
  ├── LLM generates completion
  ├── If auto_save non-empty → extract_and_save_memories(response)
  └── Return completion to user

Agent Tool Call
  ├── Tool = memory_search / memory_save / memory_update / memory_relate
  ├── Execute memory operation against DbPools
  ├── Log to memory_access_log (for decay tracking)
  └── Return result to agent

Agent Cleanup
  ├── If session_id exists → session_end(summary) → save Observation memory
  ├── Deregister agent runtime
  └── Release resources
```

This three-layer integration (declarative config, automatic injection, explicit API) ensures agents can operate with memory at varying levels of sophistication: low-touch agents use automatic context injection, high-touch agents use tools for precise memory operations.

## 4. Memory in Agent Forms

Agent forms extend with a new `[memory]` TOML section, following the extensible form pattern described in the [Agent Skills Specification](https://agentskills.io), where agents declare their capabilities and resource needs declaratively. This section is optional — agents without it operate with no automatic memory integration (backward compatible).

### 4.1 Schema Extension

`crates/nous-core/src/agents/definition.rs` extends with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub agent: AgentSection,
    pub process: Option<ProcessSection>,
    pub skills: Option<SkillsSection>,
    pub memory: Option<MemorySection>,  // NEW
    pub metadata: Option<MetadataSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySection {
    #[serde(default = "default_scope")]
    pub scope: MemoryScope,

    #[serde(default = "default_retrieval")]
    pub retrieval: RetrievalStrategy,

    #[serde(default = "default_context_size")]
    pub context_size: u32,

    #[serde(default)]
    pub auto_save: Vec<String>,  // MemoryType strings (parsed at runtime)

    #[serde(default = "default_importance")]
    pub importance_default: String,  // "low" | "moderate" | "high"

    #[serde(default)]
    pub session_tracking: bool,

    #[serde(default)]
    pub workspace_override: Option<String>,  // Override default workspace for this agent
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    Workspace,
    Agent,
    Session,
    Shared(Vec<String>),  // agent IDs
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RetrievalStrategy {
    Fts,
    Vector,
    Hybrid,
    Recency,
}

fn default_scope() -> MemoryScope { MemoryScope::Agent }
fn default_retrieval() -> RetrievalStrategy { RetrievalStrategy::Hybrid }
fn default_context_size() -> u32 { 5 }
fn default_importance() -> String { "moderate".to_string() }
```

### 4.2 Example Agent Forms

#### Minimal (no memory)

```toml
[agent]
name    = "greeter"
type    = "engineer"
version = "1.0.0"

[process]
type = "claude"
spawn_command = "claude --model claude-sonnet-4-6"
```

Behavior: no automatic memory. Agent must use explicit memory tools if needed.

#### Workspace-scoped memory

```toml
[agent]
name    = "code-reviewer"
type    = "engineer"
version = "2.0.0"

[memory]
scope        = "workspace"   # All agents in workspace share memory
retrieval    = "hybrid"      # FTS + vector
context_size = 10            # Inject top-10 memories per prompt
```

Behavior:
- Agent reads all memories in `workspace_id` (no `agent_id` filter).
- Before each prompt, top-10 hybrid search results are injected.
- No auto-save, no session tracking.

#### Agent-scoped with auto-save

```toml
[agent]
name    = "architect"
type    = "senior-manager"
version = "3.1.0"

[memory]
scope              = "agent"    # Only this agent's memories
retrieval          = "hybrid"
context_size       = 5
auto_save          = ["decision", "architecture"]
importance_default = "high"
session_tracking   = true
```

Behavior:
- Agent reads only memories where `agent_id = <this agent's UUID>`.
- Top-5 hybrid search results injected per prompt.
- LLM completions are parsed for `MEMORY[type=decision]` or `MEMORY[type=architecture]` blocks → auto-saved with `importance=high`.
- `session_start()` on spawn, `session_end()` on cleanup.

#### Shared memory pool

```toml
[agent]
name    = "incident-responder"
type    = "engineer"
version = "1.5.0"

[memory]
scope        = "shared:agent-001,agent-002,agent-003"
retrieval    = "recency"      # Recent first, no search
context_size = 20             # Last 20 memories
session_tracking = true
```

Behavior:
- Agent reads memories where `agent_id IN ('agent-001', 'agent-002', 'agent-003', '<self>')`.
- No search — pure recency via `get_context()`.
- Session tracked. Summary saved on cleanup.

Use case: multiple agents collaborate on an incident, sharing findings in a shared memory pool.

#### Session-scoped ephemeral memory

```toml
[agent]
name    = "debugger"
type    = "engineer"
version = "2.0.0"

[memory]
scope            = "session"   # Only this session's memories
retrieval        = "fts"       # Keyword search only
context_size     = 5
auto_save        = ["observation"]
session_tracking = true
```

Behavior:
- Agent reads only memories where `session_id = <current session UUID>`.
- FTS search (no vector).
- Auto-save observations (e.g., stack traces, variable dumps).
- Session summary saved on cleanup; individual observations are ephemeral (importance=low → subject to decay).

### 4.3 TOML Parsing and Validation

Parsing in `crates/nous-core/src/agents/definition.rs:load_definition()`:

```rust
pub fn load_definition(path: &Path) -> Result<AgentDefinition, NousError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| NousError::Config(format!("failed to read {}: {e}", path.display())))?;
    let def: AgentDefinition = toml::from_str(&contents)
        .map_err(|e| NousError::Config(format!("failed to parse {}: {e}", path.display())))?;
    validate_definition(&def)?;
    Ok(def)
}

fn validate_definition(def: &AgentDefinition) -> Result<(), NousError> {
    if let Some(ref mem) = def.memory {
        // Validate auto_save types
        for ty in &mem.auto_save {
            ty.parse::<MemoryType>().map_err(|_| 
                NousError::Validation(format!("invalid memory type in auto_save: {ty}"))
            )?;
        }
        // Validate importance_default
        mem.importance_default.parse::<Importance>().map_err(|_|
            NousError::Validation(format!("invalid importance_default: {}", mem.importance_default))
        )?;
        // Validate shared scope
        if let MemoryScope::Shared(ref ids) = mem.scope {
            if ids.is_empty() {
                return Err(NousError::Validation("shared scope requires at least one agent ID".into()));
            }
        }
    }
    Ok(())
}
```

Validation errors at `nous agent add <file>` time prevent invalid configs from being registered.

### 4.4 Default Behavior

If `[memory]` is absent:
- `scope = Agent` (agent-scoped by default)
- `retrieval = Hybrid` (FTS + vector)
- `context_size = 0` (no automatic injection — must use tools explicitly)
- `auto_save = []` (no extraction)
- `session_tracking = false` (no session lifecycle)

This zero-config default provides memory isolation (agent-scoped) but no automatic behavior, preserving backward compatibility with agents that expect manual memory control.

### 4.5 Workspace Override

`workspace_override` allows an agent to operate on a workspace different from the default:

```toml
[memory]
scope = "workspace"
workspace_override = "shared-project-knowledge"
```

Use case: an agent serves multiple projects but reads from a shared knowledge base. The agent's `workspace_id` defaults to the project it was spawned in, but `workspace_override` forces memory operations to target a different workspace.

Overrides are validated at spawn time: the agent must have `agent_workspace_access` granted for the override workspace, or spawn fails with `NousError::Permission`.

### 4.6 Memory Configuration Precedence

Three layers of config can specify memory behavior:

1. Agent form TOML `[memory]` section (highest precedence)
2. Daemon global config `~/.config/nous/config.toml` (future: `[memory.defaults]` section)
3. Hardcoded defaults in `definition.rs` (lowest precedence)

Phase 1 implements layers 1 and 3 only. Global memory defaults are deferred to Phase 2, when operators need to enforce workspace-wide memory policies (e.g., "all agents in this workspace must use session tracking").

### 4.7 Skills + Memory Interaction

Skills are static Markdown files. Memory is dynamic runtime state. The interaction:

```toml
[skills]
refs = ["code-review", "git-workflow"]

[memory]
auto_save = ["convention"]
```

Behavior:
- Skills provide instructions (e.g., "Always check for naming conventions").
- If the agent's response includes `MEMORY[type=convention, title="Naming: use snake_case for variables"]`, the convention is saved.
- Future prompts retrieve this convention via hybrid search, reinforcing the skill guidance with learned context.

Skills teach general rules; memory captures specific instances and refinements. They are complementary.

## 5. Retrieval Strategy

Retrieval maps user or agent queries to relevant memories. Four strategies balance precision, recall, and latency.

### 5.1 Strategy Comparison

| Strategy | Mechanism | Latency | Best For | Limitations |
|----------|-----------|---------|----------|-------------|
| **FTS** | FTS5 MATCH on `memories_fts` virtual table. BM25 ranking. | O(log N + K) | Keyword/phrase recall. Exact term matching. Compliance queries ("find all decisions about GDPR"). | Fails on synonyms, paraphrasing. Query "authentication" misses memory titled "user login flow". |
| **Vector** | vec0 KNN on `memory_embeddings`. L2 distance → cosine similarity. | O(log N + K) | Semantic similarity. Paraphrase queries. Cross-lingual retrieval (if embeddings support it). | Ignores exact term importance. Query "NOT Redis" cannot negate; KNN ranks by overall similarity. |
| **Hybrid** | FTS + vec0 → Reciprocal Rank Fusion (RRF). Merges BM25 ranks + similarity ranks. | 2x FTS latency (runs both, then merges) | General-purpose. Combines keyword precision with semantic recall. | Double query cost. Requires tuning k parameter (default 60). |
| **Recency** | SQL ORDER BY `created_at DESC` on `memories` table. No search. | O(K) (index scan) | Session continuation. "What did we just discuss?" Recent context replay. | No relevance ranking. Unrelated recent memories are returned. |

### 5.2 FTS (Full-Text Search)

Implementation: `search_memories()` (`memory/mod.rs:420`).

Query path:
```sql
SELECT m.*, bm25(memories_fts) AS rank
FROM memories_fts
JOIN memories m ON memories_fts.rowid = m.rowid
WHERE memories_fts MATCH ?
  AND m.workspace_id = ?
  AND (? IS NULL OR m.agent_id = ?)
  AND (? IS NULL OR m.memory_type = ?)
  AND (m.archived = 0 OR ?)
ORDER BY rank
LIMIT ?
```

FTS5 tokenizer: porter + unicode61. Stemming reduces "running" → "run"; unicode normalization handles accents.

Query syntax supports:
- Phrase search: `"error handling"` (exact phrase)
- Boolean: `authentication AND (oauth OR saml)` NOT redis`
- Prefix: `architec*` (matches architecture, architectural)
- Column filter: `title:performance` (search only title field)

Ranking: BM25 with default parameters (k1=1.2, b=0.75). Higher scores for rare terms and shorter documents.

Filters applied post-FTS: `workspace_id`, `agent_id`, `memory_type`, `archived`. These reduce result set size but do not affect ranking.

### 5.3 Vector (Semantic Search)

Implementation: `search_similar()` (`memory/mod.rs:767`).

Query path:
1. Embed query text via `embedder.embed(&[query_text])` → f32[384]
2. Serialize embedding as float32 LE bytes
3. Query vec0:
   ```sql
   SELECT memory_id, distance
   FROM memory_embeddings
   WHERE embedding MATCH ?
   ORDER BY distance
   LIMIT ?
   ```
4. Convert L2 distance to cosine similarity: `similarity = 1.0 / (1.0 + distance)`
5. Join against `memories` table to fetch full records
6. Apply workspace/agent/type filters post-KNN

vec0 returns exact K results. If post-KNN filters remove candidates, the effective result count is <K. Callers should over-fetch (e.g., `limit = desired_count * 3`) if exact counts matter.

Similarity scoring: cosine similarity ∈ [0, 1]. Higher is better. Threshold filtering (e.g., `score >= 0.7`) can be applied to remove low-relevance results.

### 5.4 Hybrid (RRF Fusion)

Implementation: `search_hybrid_filtered()` (`memory/mod.rs:1148`).

Query path:
1. Run FTS: `search_memories()` → Vec\<Memory\> with implicit BM25 ranks (position in result list)
2. Run Vector: `search_similar()` → Vec\<SimilarMemory\> with explicit similarity scores
3. Rerank via RRF:
   ```rust
   // memory/rerank.rs:9-47
   pub fn rerank_rrf(
       fts_results: &[Memory],
       vec_results: &[(String, f32)],  // (memory_id, similarity)
       k: f32,
   ) -> Vec<(String, f32)> {
       let mut scores: HashMap<String, f32> = HashMap::new();
       for (rank, mem) in fts_results.iter().enumerate() {
           let rrf_score = 1.0 / (k + (rank as f32 + 1.0));
           *scores.entry(mem.id.clone()).or_insert(0.0) += rrf_score;
       }
       for (rank, (id, _sim)) in vec_results.iter().enumerate() {
           let rrf_score = 1.0 / (k + (rank as f32 + 1.0));
           *scores.entry(id.clone()).or_insert(0.0) += rrf_score;
       }
       let mut ranked: Vec<_> = scores.into_iter().collect();
       ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
       ranked
   }
   ```
4. Fetch full `Memory` records for top-N IDs
5. Apply workspace/agent/type filters

RRF parameter k: default 60. Higher k flattens score differences (gives lower-ranked results more weight); lower k emphasizes top ranks. k=60 is a validated default from IR literature (Cormack et al., SIGIR 2009).

Hybrid search is most expensive (2 queries + merge) but provides best precision/recall balance. Use it as the default strategy.

### 5.5 Recency (Time-Based Context)

Implementation: `get_context()` (`memory/mod.rs:491`).

Query path:
```sql
SELECT * FROM memories
WHERE workspace_id = ?
  AND (? IS NULL OR agent_id = ?)
  AND (? IS NULL OR topic_key = ?)
  AND archived = 0
ORDER BY created_at DESC
LIMIT ?
```

No FTS, no embeddings, no search — pure chronological order. Returns the K most recent memories matching filters.

Use cases:
- Session replay: agent needs recent conversation history to continue a task.
- Topic-specific context: `topic_key = "incident-123"` retrieves all memories about a specific incident, ordered by time.
- Debug mode: "show me the last 10 things this agent learned".

Recency is fast (single index scan on `created_at`) but provides no relevance signal. If recent memories are off-topic, they pollute the context window.

### 5.6 Context Window Management

LLM context windows are finite. Claude Sonnet 4.6 supports 200K tokens input; GPT-4 Turbo supports 128K. Memory injection must respect these limits.

Token budget allocation (example for 200K context):
| Component | Tokens | Source |
|-----------|--------|--------|
| System prompt | 2K | Agent definition description, skill content |
| User input | 4K | Current user query |
| Tool results | 10K | Prior tool calls in conversation |
| Injected memories | 20K | Top-N memories from retrieval (configurable via `context_size`) |
| Reserved output | 4K | LLM completion budget |
| **Total** | 40K | Leaves 160K headroom for long conversations |

Memory injection budget: `context_size` is a count, not a token budget. Each memory's `title + content` is ~200 tokens average (based on empirical sampling of Bugfix/Decision/Convention memories). `context_size = 10` → ~2K tokens. `context_size = 100` → ~20K tokens.

Truncation strategy when memories exceed budget:
1. Retrieve top-N by relevance (FTS/vector/hybrid rank).
2. Serialize each memory as `## {title}\n{content}\n---\n`.
3. Accumulate token count via `tokenizers` crate (same tokenizer as LLM).
4. Stop when cumulative tokens >= budget.
5. Include partial memory with truncation marker `[truncated...]` if budget allows.

This is implemented in `NousMemoryIndex::top_n()` — the rig integration layer.

### 5.7 Relevance Scoring

Each retrieval strategy produces a score:

| Strategy | Score Range | Interpretation |
|----------|-------------|----------------|
| FTS | BM25 score, typically [0, 20] | Higher = better term match. Absolute value depends on corpus size and query. |
| Vector | Cosine similarity [0, 1] | >0.85 = high relevance, 0.7-0.85 = moderate, <0.7 = low. |
| Hybrid (RRF) | RRF score [0, ∞) | Sum of reciprocal ranks. Not normalized; relative ordering matters more than absolute score. |
| Recency | Implicit (chronological) | No numeric score. Rank = position in time-sorted list. |

Scores are not comparable across strategies. A hybrid RRF score of 0.3 does not mean "30% relevance" — it's a ranking signal only.

For UI display or logging: normalize scores to [0, 1] by dividing by max score in the result set. This makes scores interpretable as "relative relevance within this batch".

### 5.8 Retrieval Performance

| Metric | FTS | Vector | Hybrid | Recency |
|--------|-----|--------|--------|---------|
| Query latency (10K memories) | 5-10ms | 5-10ms | 10-20ms | 1-2ms |
| Cold cache penalty | +20ms | +50ms | +70ms | +10ms |
| Memory growth (1M memories) | 50-100ms | 50-100ms | 100-200ms | 5-10ms |
| Index size (per 10K memories) | ~15 MB (FTS5) | ~15 MB (vec0) | Both | 0 (uses PK index) |

Latency benchmarks from integration tests on SQLite in WAL mode, SSD storage, no concurrency. Actual performance depends on memory content size, query complexity, and system load.

Cold cache: first query after daemon start incurs SQLite page cache misses. Subsequent queries hit warm cache → 3-5x faster.

Optimization levers:
- FTS5 `rank` LIMIT optimization: `ORDER BY bm25(...) LIMIT N` stops after N matches (O(K) instead of O(N log N) sort).
- vec0 KNN is index-only — no table scan. Performance degrades logarithmically with corpus size (HNSW graph traversal).
- Recency queries hit `(workspace_id, created_at)` composite index → sub-millisecond for <1M rows.

### 5.9 Retrieval Strategy Selection Guide

| Scenario | Recommended Strategy | Rationale |
|----------|---------------------|-----------|
| User asks "What did we decide about error handling?" | Hybrid | Keyword "error handling" + semantic understanding of "decide". |
| Agent continues debugging session | Recency | Recent observations are most relevant; no search needed. |
| Compliance audit: "Find all GDPR-related decisions" | FTS | Exact keyword "GDPR" must appear. Semantic search may miss acronym. |
| Agent needs "similar bugs to this stack trace" | Vector | Stack traces rarely share exact keywords; embedding similarity is key. |
| Agent asks "What were we just talking about?" | Recency | Pure chronological replay. |
| General retrieval for RAG context injection | Hybrid (default) | Balances precision and recall across diverse queries. |

Default to hybrid unless the use case clearly fits FTS-only, vector-only, or recency. Agents can override via `[memory].retrieval` in TOML or via `memory_search` tool calls with explicit strategy parameter.

## 6. Memory Lifecycle

Memories are created, accessed, updated, and eventually archived or pruned. The lifecycle mirrors human memory: frequently accessed memories stay fresh; unused memories fade.

### 6.1 Creation

Three paths create memories:

| Path | Trigger | Implementation |
|------|---------|---------------|
| **Explicit save** | Agent calls `memory_save` tool or user invokes `save_memory()` API | `save_memory()` (`memory/mod.rs:277`). Requires title, content, memory_type. Optional: importance, topic_key, validity window. |
| **Auto-extraction** | Agent completion contains `MEMORY[...]` block and memory type is in `auto_save` list | `extract_and_save_memories()` parses structured markers, calls `save_memory()` for each block. |
| **Session summary** | `session_end()` called on agent cleanup | Auto-creates Observation memory with `topic_key = session/{id}`, `importance = moderate`, `content = summary`. |

Creation defaults:
- `workspace_id` = agent's current workspace (or `memory.workspace_override` if set)
- `agent_id` = agent's UUID (or NULL for workspace-wide memories)
- `session_id` = current session UUID (or NULL for persistent memories)
- `importance` = `memory.importance_default` (default: moderate)
- `valid_from` = creation timestamp
- `valid_until` = NULL (indefinite)
- `archived` = false

Upsert via `topic_key`: if `topic_key` is provided and a non-archived memory with the same key exists in the same workspace, `save_memory()` updates that memory instead of creating a new one. This enables agents to revise memories (e.g., update a convention as it evolves).

### 6.2 Access Tracking

Every memory read is logged to `memory_access_log` (`memory/mod.rs:610`):

```sql
INSERT INTO memory_access_log (id, memory_id, access_type, session_id, accessed_at)
VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
```

Access types:
- `recall` — retrieved via FTS/vector/hybrid search
- `search` — explicitly queried via `memory_search` tool
- `context` — returned by `get_context()` (recency-based)

The log feeds importance decay: memories not accessed within a threshold period have their importance reduced.

### 6.3 Updates

Memories are mutable. `update_memory()` (`memory/mod.rs:354`) supports partial updates:

```rust
pub struct UpdateMemoryRequest {
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub importance: Option<Importance>,
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: Option<bool>,
}
```

Only non-None fields are updated. SQL:
```sql
UPDATE memories SET
  title = COALESCE(?, title),
  content = COALESCE(?, content),
  importance = COALESCE(?, importance),
  topic_key = COALESCE(?, topic_key),
  valid_from = COALESCE(?, valid_from),
  valid_until = COALESCE(?, valid_until),
  archived = COALESCE(?, archived),
  updated_at = CURRENT_TIMESTAMP
WHERE id = ?
```

Use cases:
- Revise a decision: change content, bump importance to high.
- Extend validity: set `valid_until` to a future date.
- Deprecate without deleting: set `archived = true`.

When content is updated, embeddings are **not** automatically regenerated. Callers must explicitly call `store_embedding()` after `update_memory()` if semantic search needs to reflect the new content. This manual step avoids expensive re-embedding on minor edits (typo fixes, formatting changes).

### 6.4 Relations

Memories form a directed graph via `memory_relations` (`memory/mod.rs:536`):

```rust
pub enum RelationType {
    Supersedes,      // New memory replaces old (auto-sets valid_until on target)
    ConflictsWith,   // Memories are incompatible (requires resolution)
    Related,         // Weak association (e.g., both about same feature)
    Compatible,      // Memories can coexist (e.g., complementary conventions)
    Scoped,          // Source is a specialization of target (e.g., team convention scopes company convention)
    NotConflict,     // Explicit non-conflict marker (resolves ambiguity)
}
```

`relate_memories()` creates edges. Special behavior for `Supersedes`:
```sql
-- Create relation
INSERT INTO memory_relations (id, source_id, target_id, relation_type, created_at)
VALUES (?, ?, ?, 'supersedes', CURRENT_TIMESTAMP)

-- Auto-set valid_until on superseded memory
UPDATE memories SET valid_until = CURRENT_TIMESTAMP WHERE id = ?
```

This pattern enables agents to evolve decisions over time:
1. Original decision: "Use Redis for caching" (id: mem-001)
2. New decision: "Use Memcached for caching, Redis for session store" (id: mem-002)
3. Relate: `relate_memories(source=mem-002, target=mem-001, type=Supersedes)`
4. Result: mem-001's `valid_until` is set to now; future searches ignore it by default (unless `include_archived=true`).

Retrieval implications:
- `search_memories()` by default filters `archived = 0 AND (valid_until IS NULL OR valid_until > CURRENT_TIMESTAMP)`.
- Agents can opt in to historical memories via explicit flag.

### 6.5 Validity Windows

Memories can be time-bounded via `valid_from` / `valid_until`:

| Field | Type | Use Case |
|-------|------|----------|
| `valid_from` | ISO 8601 timestamp | "This decision takes effect 2026-06-01" (future-dated memory) |
| `valid_until` | ISO 8601 timestamp | "This convention expires on 2027-01-01" (scheduled deprecation) |

Default: both NULL → indefinite validity.

Future-dated memories (`valid_from > CURRENT_TIMESTAMP`) are excluded from retrieval by default. Use case: schedule a convention change in advance; agents retrieve it only after the effective date.

Expired memories (`valid_until < CURRENT_TIMESTAMP`) are treated like archived: hidden by default, but retrievable via explicit filter.

### 6.6 Importance Decay

`run_importance_decay()` (`memory/mod.rs:627`) reduces importance for unused memories:

```rust
pub async fn run_importance_decay(
    pool: &SqlitePool,
    threshold_days: u32,
) -> Result<u32, NousError> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(threshold_days as i64);
    // Find memories not accessed since cutoff
    let stale = sqlx::query!(
        "SELECT m.id, m.importance FROM memories m
         LEFT JOIN memory_access_log a ON m.id = a.memory_id AND a.accessed_at > ?
         WHERE a.id IS NULL AND m.archived = 0 AND m.importance != 'low'",
        cutoff
    ).fetch_all(pool).await?;

    for mem in stale {
        let new_importance = match mem.importance.as_str() {
            "high" => Importance::Moderate,
            "moderate" => Importance::Low,
            _ => continue,  // already low
        };
        sqlx::query!("UPDATE memories SET importance = ? WHERE id = ?", new_importance.as_str(), mem.id)
            .execute(pool).await?;
    }
    Ok(stale.len() as u32)
}
```

Decay schedule:
- High → Moderate after 30 days of no access
- Moderate → Low after 60 days of no access
- Low memories remain; archival is manual

Rationale: importance affects retrieval ranking (higher importance memories can be boosted in hybrid search) and retention policies (low-importance memories are candidates for pruning).

Decay runs as a background task (cron or daemon scheduled). Frequency: daily or weekly depending on corpus size.

### 6.7 Archival

Archival is soft-delete. `archived = 1` hides memories from default queries but preserves them for audit/rollback:

```sql
UPDATE memories SET archived = 1, updated_at = CURRENT_TIMESTAMP WHERE id = ?
```

Archived memories are excluded from:
- `search_memories()` (unless `include_archived = true`)
- `get_context()`
- `search_hybrid_filtered()`
- FTS5 index (trigger deletes from `memories_fts` on archive)
- vec0 index (entry remains; filtered post-KNN)

Archived memories remain in `memories` table. Hard delete is manual (no automatic purge):

```sql
DELETE FROM memories WHERE id = ?
```

Hard delete cascades to `memory_relations` (source or target = deleted ID) and `memory_access_log`. vec0 entries are orphaned (no FK constraint) but harmless (KNN returns IDs that fail to join against `memories` → filtered out).

### 6.8 Consolidation (Future)

As memories accumulate, redundancy grows: multiple observations about the same bug, multiple conventions that overlap. Consolidation merges or summarizes related memories.

Proposed consolidation triggers:
- **Topic-based**: all memories with same `topic_key` → summarize into a single Fact or Convention.
- **Relation-based**: memories in a `Related` cluster → extract common themes, archive originals.
- **LLM-based**: pass N memories to LLM with prompt "Consolidate these into a single memory preserving key facts".

Implementation deferred to Phase 2. Requires:
1. Clustering algorithm (DBSCAN on embeddings, or relation graph traversal).
2. Summarization LLM call (consumes tokens).
3. Provenance tracking (consolidated memory links to originals via relations).

Phase 1 agents operate without consolidation — memory growth is unbounded. Operators can manually archive low-value memories or run periodic cleanup.

### 6.9 Lifecycle State Diagram

```
                    save_memory()
                         |
                         v
                   [ ACTIVE ]
                    /    |    \
    access_log() ->    update_memory()    <- relate_memories()
                    \    |    /
                         |
               run_importance_decay()
                         |
                         v
                  [ DECAYED ]
                  (importance=low)
                         |
                 archive (manual)
                         v
                   [ ARCHIVED ]
                   (archived=1)
                         |
                 hard delete (manual)
                         v
                    [ DELETED ]
```

Memories spend most of their lifecycle in ACTIVE state. Decay and archival are opt-in operations; memories do not automatically disappear.

## 7. API Surface

The memory subsystem exposes three API layers: Rust core (synchronous and async), daemon HTTP/gRPC (future), and rig integration (trait implementations for RAG).

### 7.1 Core Rust API (`crates/nous-core/src/memory/mod.rs`)

All operations are `async` and return `Result<T, NousError>`.

#### Create and Update

```rust
pub async fn save_memory(
    pool: &SqlitePool,
    req: SaveMemoryRequest,
) -> Result<Memory, NousError>
```
- Insert or upsert (via `topic_key` match).
- Returns full `Memory` record with generated UUID.
- Dual-writes to `memories` and `memories_fts` (via trigger).

```rust
pub async fn update_memory(
    pool: &SqlitePool,
    req: UpdateMemoryRequest,
) -> Result<Memory, NousError>
```
- Partial update (only non-None fields).
- Returns updated `Memory` record.
- Embeddings are **not** regenerated; caller must invoke `store_embedding()` separately.

#### Search and Retrieval

```rust
pub async fn search_memories(
    pool: &SqlitePool,
    req: SearchMemoryRequest,
) -> Result<Vec<Memory>, NousError>
```
- FTS5 MATCH query, BM25-ranked.
- Filters: `workspace_id`, `agent_id`, `memory_type`, `importance`, `include_archived`.

```rust
pub async fn search_similar(
    pools: &DbPools,
    query_embedding: &[f32],
    workspace_id: &str,
    agent_id: Option<&str>,
    limit: u32,
) -> Result<Vec<SimilarMemory>, NousError>
```
- vec0 KNN, L2 distance → cosine similarity.
- Post-KNN filter by workspace/agent.
- Returns `SimilarMemory` (Memory + score).

```rust
pub async fn search_hybrid_filtered(
    pools: &DbPools,
    query_embedding: &[f32],
    workspace_id: &str,
    agent_id: Option<&str>,
    memory_types: Option<&[MemoryType]>,
    limit: u32,
) -> Result<Vec<SimilarMemory>, NousError>
```
- FTS + vec0 → RRF fusion.
- Post-merge filter by workspace/agent/type.
- Returns unified `SimilarMemory` list, RRF-ranked.

```rust
pub async fn get_context(
    pool: &SqlitePool,
    req: ContextRequest,
) -> Result<Vec<Memory>, NousError>
```
- Recency-based: `ORDER BY created_at DESC`.
- No search, no embeddings.
- Filters: `workspace_id`, `agent_id`, `topic_key`.

#### Relations

```rust
pub async fn relate_memories(
    pool: &SqlitePool,
    req: RelateRequest,
) -> Result<MemoryRelation, NousError>
```
- Create directed edge in `memory_relations`.
- `Supersedes` relation auto-sets `valid_until` on target.

```rust
pub async fn get_memory_relations(
    pool: &SqlitePool,
    memory_id: &str,
) -> Result<Vec<MemoryRelation>, NousError>
```
- Fetch all relations where `source_id = memory_id OR target_id = memory_id`.

#### Embedding Operations

```rust
pub async fn store_embedding(
    pools: &DbPools,
    memory_id: &str,
    embedding: &[f32],
) -> Result<(), NousError>
```
- Dual-write: `memories.embedding BLOB` + `memory_embeddings` vec0.
- Validates dimension = 384 (compile-time constant).

```rust
pub async fn store_chunks(
    pools: &DbPools,
    memory_id: &str,
    content: &str,
    chunker: &Chunker,
) -> Result<Vec<Chunk>, NousError>
```
- Split content via chunker (256 tokens, 64 overlap).
- Insert into `memory_chunks`.
- Does **not** embed chunks; caller must invoke `store_chunk_embedding()` per chunk.

```rust
pub async fn store_chunk_embedding(
    pools: &DbPools,
    chunk_id: &str,
    embedding: &[f32],
) -> Result<(), NousError>
```
- Store per-chunk embedding in vec0.
- Chunk retrieval (not yet implemented) would query vec0 by chunk embeddings, then reconstruct full memory.

#### Session Lifecycle

```rust
pub async fn session_start(
    pool: &SqlitePool,
    agent_id: &str,
    project: &str,
) -> Result<MemorySession, NousError>
```
- Insert into `memory_sessions` with `started_at = CURRENT_TIMESTAMP`.
- Returns session record with UUID.

```rust
pub async fn session_end(
    pool: &SqlitePool,
    session_id: &str,
    summary: &str,
) -> Result<(), NousError>
```
- Update `memory_sessions.ended_at`.
- Auto-create Observation memory: `topic_key = session/{id}`, `content = summary`, `importance = moderate`.

#### Access Tracking and Decay

```rust
pub async fn log_access(
    pool: &SqlitePool,
    memory_id: &str,
    access_type: &str,  // "recall" | "search" | "context"
    session_id: Option<&str>,
) -> Result<(), NousError>
```
- Insert into `memory_access_log`.

```rust
pub async fn run_importance_decay(
    pool: &SqlitePool,
    threshold_days: u32,
) -> Result<u32, NousError>
```
- Decay importance for memories not accessed within threshold.
- Returns count of decayed memories.

#### Workspace ACL

```rust
pub async fn grant_workspace_access(
    pool: &SqlitePool,
    agent_id: &str,
    workspace_id: &str,
) -> Result<(), NousError>
```

```rust
pub async fn revoke_workspace_access(
    pool: &SqlitePool,
    agent_id: &str,
    workspace_id: &str,
) -> Result<(), NousError>
```

```rust
pub async fn check_workspace_access(
    pool: &SqlitePool,
    agent_id: &str,
    workspace_id: &str,
) -> Result<bool, NousError>
```

### 7.2 Rig Integration (`crates/nous-daemon/src/agent_memory.rs`, new file)

Implement rig's `VectorStoreIndex` trait to expose nous memory as a RAG context source.

```rust
use rig::vector_store::{VectorStoreIndex, VectorSearchRequest, VectorStoreError};
use nous_core::memory::{search_hybrid_filtered, DbPools, Embedder, MemoryType, SimilarMemory};

pub struct NousMemoryIndex {
    pools: DbPools,
    embedder: Arc<dyn Embedder>,
    workspace_id: String,
    agent_id: Option<String>,
    memory_types: Option<Vec<MemoryType>>,
}

impl NousMemoryIndex {
    pub fn new(
        pools: DbPools,
        embedder: Arc<dyn Embedder>,
        workspace_id: String,
        agent_id: Option<String>,
        memory_types: Option<Vec<MemoryType>>,
    ) -> Self {
        Self { pools, embedder, workspace_id, agent_id, memory_types }
    }
}

impl VectorStoreIndex for NousMemoryIndex {
    type Filter = ();  // No rig-specific filter; scoping via workspace/agent

    async fn top_n<T: for<'a> Deserialize<'a> + Send>(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String, T)>, VectorStoreError> {
        // 1. Embed query text
        let embedding = self.embedder.embed(&[&req.query])
            .map_err(|e| VectorStoreError::DatastoreError(e.into()))?
            .into_iter().next().unwrap();

        // 2. Hybrid search
        let results = search_hybrid_filtered(
            &self.pools,
            &embedding,
            &self.workspace_id,
            self.agent_id.as_deref(),
            self.memory_types.as_deref(),
            req.samples as u32,
        ).await.map_err(|e| VectorStoreError::DatastoreError(e.into()))?;

        // 3. Map to rig format: (score, id, document)
        results.into_iter().map(|sm| {
            let doc: T = serde_json::from_value(serde_json::to_value(&sm.memory)?)?;
            Ok((sm.score as f64, sm.memory.id.clone(), doc))
        }).collect()
    }

    async fn top_n_ids(
        &self,
        req: VectorSearchRequest<Self::Filter>,
    ) -> Result<Vec<(f64, String)>, VectorStoreError> {
        // Same as top_n but return only (score, id)
        let embedding = self.embedder.embed(&[&req.query])
            .map_err(|e| VectorStoreError::DatastoreError(e.into()))?
            .into_iter().next().unwrap();
        let results = search_hybrid_filtered(
            &self.pools,
            &embedding,
            &self.workspace_id,
            self.agent_id.as_deref(),
            self.memory_types.as_deref(),
            req.samples as u32,
        ).await.map_err(|e| VectorStoreError::DatastoreError(e.into()))?;
        Ok(results.into_iter().map(|sm| (sm.score as f64, sm.memory.id)).collect())
    }
}
```

Usage in agent spawn:

```rust
let memory_index = NousMemoryIndex::new(
    app_state.pools.clone(),
    app_state.embedder.clone(),
    workspace_id,
    Some(agent_id),
    None,  // all memory types
);

let rag_agent = llm_client.agent(model)
    .dynamic_context(5, memory_index)  // inject top-5 memories per prompt
    .build();
```

### 7.3 Memory Tools (rig Tool trait)

Expose memory operations as LLM-callable tools.

```rust
// crates/nous-daemon/src/tools/memory.rs (new file)

use rig::tool::{Tool, ToolDefinition};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct MemorySearchArgs {
    query: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    memory_types: Option<Vec<String>>,
}

pub struct MemorySearchTool {
    pools: DbPools,
    embedder: Arc<dyn Embedder>,
    workspace_id: String,
    agent_id: String,
}

#[async_trait]
impl Tool for MemorySearchTool {
    const NAME: &'static str = "memory_search";

    type Args = MemorySearchArgs;
    type Output = Vec<Memory>;
    type Error = NousError;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search agent memory for relevant facts, decisions, and conventions".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural language query"},
                    "limit": {"type": "integer", "description": "Max results (default 5)"},
                    "memory_types": {"type": "array", "items": {"type": "string"}, "description": "Filter by memory type (decision, convention, etc.)"}
                },
                "required": ["query"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let types = args.memory_types.map(|v| {
            v.iter().map(|s| s.parse::<MemoryType>()).collect::<Result<Vec<_>, _>>()
        }).transpose()?;
        let embedding = self.embedder.embed(&[&args.query])?[0].clone();
        let results = search_hybrid_filtered(
            &self.pools,
            &embedding,
            &self.workspace_id,
            Some(&self.agent_id),
            types.as_deref(),
            args.limit.unwrap_or(5),
        ).await?;
        Ok(results.into_iter().map(|sm| sm.memory).collect())
    }
}
```

Similarly define:
- `MemorySaveTool` (wraps `save_memory()`)
- `MemoryUpdateTool` (wraps `update_memory()`)
- `MemoryRelateTool` (wraps `relate_memories()`)
- `MemoryContextTool` (wraps `get_context()`)

Register tools at agent spawn:

```rust
let tools: Vec<Arc<dyn Tool>> = vec![
    Arc::new(MemorySearchTool::new(pools, embedder, workspace_id, agent_id)),
    Arc::new(MemorySaveTool::new(pools, workspace_id, agent_id)),
    // ... other tools
];

let agent = llm_client.agent(model)
    .tools(tools)
    .build();
```

### 7.4 Type Definitions

All core types are in `crates/nous-core/src/memory/mod.rs`:

```rust
// Memory types
pub enum MemoryType { Decision, Convention, Bugfix, Architecture, Fact, Observation }
pub enum Importance { Low, Moderate, High }
pub enum RelationType { Supersedes, ConflictsWith, Related, Compatible, Scoped, NotConflict }

// Domain objects
pub struct Memory {
    pub id: String,
    pub workspace_id: String,
    pub agent_id: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: String,      // MemoryType serialized as string
    pub importance: String,        // Importance serialized as string
    pub topic_key: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

pub struct SimilarMemory {
    #[serde(flatten)]
    pub memory: Memory,
    pub score: f32,  // cosine similarity or RRF score
}

pub struct MemoryRelation {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub created_at: String,
}

pub struct MemorySession {
    pub id: String,
    pub agent_id: String,
    pub project: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
}

pub struct Chunk {
    pub id: String,
    pub memory_id: String,
    pub content: String,
    pub index: u32,
    pub start_offset: u32,
    pub end_offset: u32,
}

// Request types
pub struct SaveMemoryRequest { /* ... */ }
pub struct UpdateMemoryRequest { /* ... */ }
pub struct SearchMemoryRequest { /* ... */ }
pub struct ContextRequest { /* ... */ }
pub struct RelateRequest { /* ... */ }
```

### 7.5 Error Handling

All operations return `Result<T, NousError>`. Error variants relevant to memory:

```rust
pub enum NousError {
    Sqlite(sqlx::Error),           // Database errors
    Validation(String),             // Invalid input (empty title, bad memory type)
    NotFound(String),               // Memory ID not found
    Permission(String),             // Agent lacks workspace access
    Internal(String),               // Unexpected error (embedding failure, vec pool lock timeout)
}
```

Error propagation:
- SQLx errors (`sqlx::Error`) map to `NousError::Sqlite`.
- Invalid `MemoryType` / `Importance` strings map to `NousError::Validation`.
- ACL failures map to `NousError::Permission`.
- ONNX embedding failures map to `NousError::Internal`.

Tools return errors to the LLM as structured JSON:

```json
{
  "error": "Validation",
  "message": "memory_type 'invalid' is not recognized. Valid types: decision, convention, bugfix, architecture, fact, observation"
}
```

The LLM can retry with corrected arguments.

### 7.6 Embedder Trait

```rust
// crates/nous-core/src/memory/embed.rs

pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NousError>;
    fn dimension(&self) -> usize;
}
```

Implementations:
- `OnnxEmbeddingModel` (production, all-MiniLM-L6-v2)
- `MockEmbedder` (tests, deterministic hash-based vectors)

Future: add `RigEmbedderAdapter` (wraps rig's `EmbeddingModel` trait) for cloud providers (Bedrock Titan, OpenAI).

### 7.7 DbPools

```rust
// crates/nous-core/src/db/pool.rs

pub type VecPool = Arc<Mutex<rusqlite::Connection>>;

pub struct DbPools {
    pub fts: SqlitePool,  // SQLx async
    pub vec: VecPool,      // rusqlite sync, single connection
}

impl DbPools {
    pub async fn connect(db_dir: &Path) -> Result<Self, NousError>;
    pub async fn run_migrations(&self) -> Result<(), NousError>;
}
```

The FTS pool handles all relational queries (`memories`, `memory_relations`, `memory_access_log`, `memory_sessions`). The Vec pool handles vec0 queries (`memory_embeddings`, `memory_chunks`). They communicate via `memory_id` (UUID string).

### 7.8 Example Usage

#### Save and retrieve a memory

```rust
use nous_core::memory::{save_memory, search_hybrid_filtered, SaveMemoryRequest, MemoryType, Importance};

let req = SaveMemoryRequest {
    workspace_id: Some("project-alpha".to_string()),
    agent_id: Some(agent_uuid.clone()),
    title: "Error Handling Convention".to_string(),
    content: "Always wrap external API calls in retry logic with exponential backoff.".to_string(),
    memory_type: MemoryType::Convention,
    importance: Some(Importance::High),
    topic_key: Some("error-handling".to_string()),
    valid_from: None,
    valid_until: None,
};

let memory = save_memory(&pools.fts, req).await?;
println!("Saved memory: {}", memory.id);

// Embed content
let embedding = embedder.embed(&[&memory.content])?[0].clone();
store_embedding(&pools, &memory.id, &embedding).await?;

// Retrieve via hybrid search
let query = "How should I handle API failures?";
let query_embedding = embedder.embed(&[query])?[0].clone();
let results = search_hybrid_filtered(
    &pools,
    &query_embedding,
    "project-alpha",
    Some(&agent_uuid),
    None,  // all types
    5,
).await?;

for result in results {
    println!("Relevance: {:.2} — {}", result.score, result.memory.title);
}
```

#### Session lifecycle

```rust
use nous_core::memory::{session_start, session_end};

let session = session_start(&pools.fts, &agent_uuid, "project-alpha").await?;
println!("Session started: {}", session.id);

// ... agent performs work ...

session_end(&pools.fts, &session.id, "Implemented retry logic for external APIs").await?;
println!("Session ended, summary saved as memory");
```

## 8. Migration Plan

Agent-memory integration is additive. No breaking changes to existing memory or agent systems. Four phases deliver incremental value.

### 8.1 Phase 1: Agent Form Extensions (2 weeks)

**Goal:** Agents can declare memory config in TOML. No runtime integration yet — validation and parsing only.

**Deliverables:**

1. Extend `AgentDefinition` with `MemorySection` (`definition.rs:7-12`)
   - Add `pub memory: Option<MemorySection>` field
   - Implement `MemorySection`, `MemoryScope`, `RetrievalStrategy` types
   - Serde defaults for all fields

2. Update `load_definition()` to parse `[memory]` section (`definition.rs:53`)
   - Validate `auto_save` contains valid MemoryType strings
   - Validate `importance_default` is "low" | "moderate" | "high"
   - Validate `shared` scope has non-empty agent ID list

3. Add example agent forms in `examples/agents/`
   - `memory-aware-reviewer.toml` (workspace scope, hybrid retrieval, auto-save)
   - `session-tracker.toml` (session scope, recency retrieval, session_tracking=true)
   - `minimal-agent.toml` (no [memory] section — backward compatible)

4. Unit tests (`definition.rs:88-226`)
   - Parse valid `[memory]` sections
   - Reject invalid memory types in `auto_save`
   - Reject empty `shared` scope
   - Defaults when `[memory]` is absent

**Acceptance Criteria:**
- `nous agent add <toml-with-memory>` parses without error.
- Invalid memory configs produce clear validation errors.
- Agents without `[memory]` continue to work unchanged.

**Files Modified:**
- `crates/nous-core/src/agents/definition.rs` (~100 lines added)
- `examples/agents/*.toml` (3 new files)

**Dependencies:** None. No runtime changes — pure schema extension.

---

### 8.2 Phase 2: Rig Integration for Context Injection (3 weeks)

**Goal:** Agents with `[memory]` config automatically retrieve memories before each prompt. No auto-save yet.

**Deliverables:**

1. Implement `NousMemoryIndex` (`nous-daemon/src/agent_memory.rs`, new file ~200 lines)
   - Implement `VectorStoreIndex` trait
   - `top_n()` calls `search_hybrid_filtered()` based on `retrieval` strategy
   - Map retrieval strategies: fts → `search_memories()`, vector → `search_similar()`, hybrid → `search_hybrid_filtered()`, recency → `get_context()`
   - Handle scoping: workspace/agent/session/shared filters

2. Integrate into agent spawn flow (`nous-daemon/src/agent/spawn.rs`)
   - Read `memory` config from `AgentDefinition`
   - Construct `NousMemoryIndex` with workspace/agent scope
   - Call `.dynamic_context(context_size, index)` on rig agent builder
   - If `session_tracking=true`, call `session_start()` and store session ID in agent runtime context

3. Integrate into agent cleanup (`nous-daemon/src/agent/cleanup.rs`)
   - If `session_id` exists in agent context, call `session_end(summary)` on cleanup
   - Summary = truncated conversation history or explicit LLM summary call

4. Add `memory_access_log` writes to `NousMemoryIndex::top_n()`
   - Log each retrieved memory with `access_type = "recall"`

5. Integration tests
   - Spawn agent with `memory.context_size=5`, issue prompt, verify rig injected 5 memories
   - Test all 4 retrieval strategies (fts, vector, hybrid, recency)
   - Test all 4 scopes (workspace, agent, session, shared)
   - Verify session lifecycle: `session_start` on spawn, `session_end` on cleanup

**Acceptance Criteria:**
- Agent spawned with `[memory]` config retrieves relevant memories before each prompt.
- Retrieved memories appear in LLM context (verifiable via rig debug logs or prompt inspection).
- Session tracking creates `MemorySession` records and summary memories.

**Files Modified:**
- `crates/nous-daemon/src/agent_memory.rs` (new, ~200 lines)
- `crates/nous-daemon/src/agent/spawn.rs` (~50 lines)
- `crates/nous-daemon/src/agent/cleanup.rs` (~30 lines)
- Integration tests (new, ~300 lines)

**Dependencies:**
- Phase 1 complete (TOML parsing).
- Existing rig 0.36 integration (already in workspace).

---

### 8.3 Phase 3: Auto-Extraction and Memory Tools (3 weeks)

**Goal:** Agents auto-save memories from completions. Agents can explicitly call memory operations as tools.

**Deliverables:**

1. Structured marker extraction (`nous-daemon/src/agent_memory.rs`, extend)
   - Parse `MEMORY[...]` blocks from agent completions
   - Extract: type, importance, title, content, topic_key
   - Filter by `auto_save` list
   - Call `save_memory()` for each extracted block
   - Embed content via `embedder.embed()`, call `store_embedding()`

2. Hook extraction into agent response pipeline
   - After LLM completion, if `auto_save` non-empty, call `extract_and_save_memories()`
   - Log extracted memory IDs (for debugging/audit)

3. Implement memory tools (`nous-daemon/src/tools/memory.rs`, new file ~400 lines)
   - `MemorySearchTool` (wraps `search_hybrid_filtered`)
   - `MemorySaveTool` (wraps `save_memory` + `store_embedding`)
   - `MemoryUpdateTool` (wraps `update_memory`)
   - `MemoryRelateTool` (wraps `relate_memories`)
   - `MemoryContextTool` (wraps `get_context`)

4. Register tools at agent spawn
   - If `[tools].refs` includes memory tools, register them with rig agent
   - Tools are scoped to agent's workspace/agent_id

5. Integration tests
   - Agent with `auto_save=["decision"]` generates completion with `MEMORY[type=decision]` → verify memory is saved
   - Agent calls `memory_search("prior decisions about X")` → verify results
   - Agent calls `memory_save(...)` explicitly → verify persistence

**Acceptance Criteria:**
- Agents with `auto_save` non-empty extract and persist memories from completions.
- Agents can search, save, update, and relate memories via tool calls.
- Tools enforce scoping (agents cannot write to other agents' memories or workspaces they lack access to).

**Files Modified:**
- `crates/nous-daemon/src/agent_memory.rs` (~150 lines added)
- `crates/nous-daemon/src/tools/memory.rs` (new, ~400 lines)
- `crates/nous-daemon/src/agent/spawn.rs` (~30 lines)
- Integration tests (~400 lines)

**Dependencies:**
- Phase 2 complete (rig integration).

---

### 8.4 Phase 4: Advanced Features (3 weeks, post-prototype)

**Goal:** Production hardening, optimization, and advanced memory lifecycle.

**Deliverables:**

1. **Workspace ACL enforcement at retrieval**
   - Verify `agent_workspace_access` before any memory query
   - Return empty results (not errors) for unauthorized access
   - Add `grant_workspace_access()` calls during agent registration

2. **Importance decay scheduler**
   - Add daemon background task: run `run_importance_decay()` daily
   - Configurable threshold via `config.toml` (default 30 days)
   - Log decay events for audit

3. **Memory consolidation (future)**
   - Cluster memories by `topic_key` or embedding similarity
   - LLM-based summarization: pass N memories → generate consolidated memory
   - Provenance tracking: consolidated memory links to originals via `Related` relations
   - Deferred beyond Phase 4 — design only

4. **Token budget enforcement**
   - Implement tokenizer-based truncation in `NousMemoryIndex::top_n()`
   - Respect LLM context window limits (configurable per model)
   - Truncate memories mid-content if needed, append `[truncated]` marker

5. **LLM-based memory extraction (future)**
   - Secondary LLM call to extract memories from free-form completions
   - JSON output schema: `[{"type": "...", "title": "...", "content": "..."}]`
   - Deferred beyond Phase 4 — design only

6. **Performance optimization**
   - Profile hybrid search latency under load
   - Add caching layer for frequent queries (in-memory LRU)
   - Benchmark vec0 scaling to 1M+ memories

7. **Monitoring and observability**
   - Expose Prometheus metrics: memory count, search latency, decay rate
   - Structured logging for memory operations (JSON logs)
   - Dashboard: memory growth over time, retrieval hit rate

**Acceptance Criteria:**
- ACL violations return empty results, not errors.
- Importance decay runs automatically; logs are auditable.
- Token budget enforcement prevents context overflow.
- Metrics are exported for monitoring.

**Files Modified:**
- `crates/nous-daemon/src/agent_memory.rs` (~100 lines)
- `crates/nous-daemon/src/background.rs` (new, ~150 lines)
- `crates/nous-core/src/memory/mod.rs` (~50 lines)
- Monitoring integration (~200 lines)

**Dependencies:**
- Phase 3 complete (auto-extraction, tools).

---

### 8.5 Backward Compatibility Strategy

All phases preserve backward compatibility:

| Component | Pre-Integration Behavior | Post-Integration Behavior |
|-----------|-------------------------|--------------------------|
| Agents without `[memory]` | Operate with no memory | Same — no change |
| Existing memory APIs | Direct calls to `save_memory()`, `search_memories()` | Same — no breaking changes |
| FTS/vec0 schema | 26 FTS migrations, 2 vec migrations | Same — no schema changes required |
| Agent definitions | `[agent]`, `[process]`, `[skills]`, `[metadata]` sections | Same — `[memory]` is optional |
| Daemon API | HTTP/gRPC endpoints (future) | Same — new endpoints additive only |

No existing agents or memory records are affected. Operators can adopt agent-memory integration on a per-agent basis by adding `[memory]` sections to TOML files.

---

### 8.6 Rollout Strategy

1. **Phase 1 ships immediately** — extends schema only, no runtime risk.
2. **Phase 2 deploys to dev/staging first** — validate rig integration under load.
3. **Phase 3 follows 2-week stabilization period** — auto-extraction has LLM parsing risk; monitor extraction accuracy.
4. **Phase 4 is opt-in per workspace** — production deployments enable ACL, decay, token budget via config flags.

Feature flags (environment variables or `config.toml`):
- `NOUS_MEMORY_AUTO_EXTRACTION_ENABLED` (default: true after Phase 3)
- `NOUS_MEMORY_DECAY_ENABLED` (default: false until Phase 4)
- `NOUS_MEMORY_TOKEN_BUDGET` (default: 20000)

---

### 8.7 Testing Strategy

| Phase | Test Coverage | Critical Paths |
|-------|---------------|----------------|
| Phase 1 | Unit tests for TOML parsing, validation | Parse all example definitions, reject invalid configs |
| Phase 2 | Integration tests for rig context injection | Spawn agent, issue prompt, verify memories injected |
| Phase 3 | Integration tests for extraction + tools | Auto-extract decision, search via tool, verify results |
| Phase 4 | Load tests for decay, ACL, token budget | 1M memories, 1000 agents, measure retrieval latency |

CI pipeline:
- Unit tests run on every commit.
- Integration tests run on PR to main.
- Load tests run nightly (optional, requires large memory corpus).

---

### 8.8 Documentation Deliverables

| Deliverable | Audience | Phase |
|------------|----------|-------|
| Agent memory integration guide (this document) | Engineers implementing the feature | Phase 1 |
| Agent definition reference (extend existing doc) | Agent authors writing TOML | Phase 2 |
| Memory API reference (Rust docs) | External integrators | Phase 3 |
| Operator runbook (memory lifecycle, decay, ACL) | SRE / ops teams | Phase 4 |

All docs live in `docs/` directory, versioned with code.

## 9. Open Decisions

The following design questions remain unresolved. Each should be addressed before or during implementation.

### 9.1 Custom Memory Types

**Question:** Should agents be able to define custom memory types beyond the 6 built-in types (Decision/Convention/Bugfix/Architecture/Fact/Observation)?

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. No custom types** | Simpler. Existing 6 types cover most use cases. Agents can use `topic_key` for finer categorization. | Limits flexibility. Domain-specific agents (medical, legal) may need specialized types (Diagnosis, CaseLaw). |
| **B. Custom types via TOML extension** | Agent TOML declares `[memory.custom_types]` section. DB stores as strings; no schema change. | Retrieval/filtering by custom type works. But cross-agent interop suffers — agents must know each other's type vocabulary. |
| **C. Custom types via registry** | Central registry in `config.toml` or DB table maps type names → descriptions. Agents reference registered types. | Consistent vocabulary across agents. But adds registry maintenance overhead. |

**Recommendation:** Start with Option A (no custom types). If demand emerges during Phase 2-3 user feedback, implement Option B (TOML extension) as it requires no schema change — `memory_type` is already a TEXT column.

**Decision owner:** Engineering lead during Phase 1 review.

---

### 9.2 Memory Scope Hierarchy

**Question:** Should memory scopes form a hierarchy where agents can access parent scopes?

**Example Hierarchy:**
```
workspace (top)
  └── team (group of agents)
       └── agent (individual)
            └── session (temporary)
```

Agent can read: own memories + team memories + workspace memories. Cannot read other agents' or sessions'.

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Flat scoping (current design)** | Simple. Each scope is independent. Explicit via `shared:<ids>`. | Requires manual enumeration of shared agents. No inheritance. |
| **B. Hierarchical scoping** | Agents naturally access broader context. Teams can define conventions all members see. | Requires new `team_id` column in `memories`, `agents` tables. Complex ACL logic. |

**Recommendation:** Start with Option A (flat). Add hierarchy only if user feedback shows clear need (e.g., "all agents on my team should share conventions"). Hierarchical scoping is a schema change + migration effort.

**Decision owner:** Product/UX input during Phase 2 alpha testing.

---

### 9.3 Context Injection Strategy

**Question:** When multiple retrieval strategies are possible, which should `.dynamic_context()` use?

**Current Design:** Agent TOML specifies `retrieval = "hybrid" | "fts" | "vector" | "recency"`. This is a per-agent config.

**Alternative:** Query-dependent strategy selection. LLM prompt includes signal: "find exact phrase" → use FTS. "semantically similar to X" → use vector.

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Static per-agent (current)** | Predictable. Agent author controls cost/latency tradeoff. | Cannot adapt to query type. Wastes resources if query is keyword-heavy but agent uses vector. |
| **B. Query-dependent routing** | Optimal per-query. Classify query intent (keyword vs semantic) → route to best strategy. | Requires query intent classifier (LLM call or heuristic). Adds latency. |
| **C. Parallel multi-strategy** | Run FTS + vector + recency in parallel → RRF merge all three. | Best recall, worst latency (3x queries). Expensive for high-QPS agents. |

**Recommendation:** Option A for Phase 2. Hybrid search (FTS + vector) already merges keyword and semantic signals — it's a good general-purpose default. Defer query-dependent routing to Phase 4 optimization if latency budgets allow.

**Decision owner:** Engineering during Phase 2 performance profiling.

---

### 9.4 Token Budget Allocation

**Question:** How should token budget be split between injected memories and other context (system prompt, user input, tool results)?

**Current Design (§5.6):** Fixed allocation (~20K tokens for memories, ~20K for other). But this wastes budget if few memories are relevant, and overflows if many are.

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Fixed allocation (current)** | Simple. Predictable. | Wastes budget or overflows depending on workload. |
| **B. Dynamic allocation** | Measure token count of other components first. Allocate remainder to memories. | Maximizes utilization. But requires tokenizer call before retrieval (latency). |
| **C. Tiered priority** | System prompt (highest priority) → user input → memories → tool results → output budget. Truncate lower-priority components first. | Ensures critical content never truncated. But complex to implement; needs priority-aware serializer. |

**Recommendation:** Option B (dynamic). Tokenize system prompt + user input + tool results → calculate remaining budget → pass to `NousMemoryIndex::top_n()` as max_tokens parameter. This requires `context_size` to become a token budget, not a count. Or add `context_token_budget` as a separate field in `[memory]` TOML.

**Decision owner:** Engineering during Phase 2 rig integration.

---

### 9.5 Embedding Model Flexibility

**Question:** Should agents be able to override the embedding model per-agent?

**Current State:** Single global embedding model (all-MiniLM-L6-v2, 384-dim). All memories use the same embeddings.

**Use Case:** A multilingual agent might want a multilingual embedding model (e.g., `multilingual-e5-large`, 1024-dim). A code agent might want code-tuned embeddings (e.g., CodeBERT).

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Global model only (current)** | Simplest. All memories comparable. Single vec0 table. | Cannot optimize per-domain. Multilingual agents stuck with English-tuned model. |
| **B. Per-agent model override** | Agent TOML specifies `[memory.embedding_model]`. Memories tagged with model ID. Separate vec0 table per model. | Flexibility. But complex: dimension mismatch, multiple vec pools, cross-agent search breaks (can't compare embeddings from different models). |
| **C. Per-workspace model** | All agents in a workspace share one model. Specified in workspace config. | Balances flexibility and simplicity. Agents in same workspace can share memories with compatible embeddings. |

**Recommendation:** Option A for Phase 1-3. Option C for Phase 4 if multi-model demand emerges. Per-agent models (Option B) are too complex for prototype phase.

**Decision owner:** Product input during Phase 3 user feedback.

---

### 9.6 Memory Consolidation Triggers

**Question:** When should memory consolidation run?

**Context (§6.8):** Consolidation merges redundant memories. But when is "redundant" detected?

**Options:**

| Option | Trigger | Pros | Cons |
|--------|---------|------|------|
| **A. Manual** | Operator runs `nous memory consolidate --workspace X` | Full control. No surprise consolidations. | Requires human intervention. Scales poorly. |
| **B. Scheduled** | Cron job: nightly or weekly consolidation | Automated. Prevents unbounded memory growth. | Fixed schedule ignores actual memory churn. May consolidate too early or too late. |
| **C. Threshold-based** | Consolidate when: (1) >N memories with same `topic_key`, or (2) >M memories in same embedding cluster | Adaptive. Consolidates only when needed. | Requires clustering algorithm (DBSCAN on embeddings). Expensive to run. |
| **D. Agent-driven** | Agent explicitly calls `memory_consolidate` tool when it detects redundancy | Context-aware. Agent knows when memories are redundant. | Requires agent sophistication. Agents must learn to recognize redundancy. |

**Recommendation:** Phase 1-3 have no consolidation (deferred). Phase 4 starts with Option A (manual) for operator control during alpha testing. Option C (threshold-based) is the long-term target for production.

**Decision owner:** Engineering during Phase 4 consolidation design.

---

### 9.7 Memory Portability Across Workspaces

**Question:** Should agents be able to export/import memories across workspaces?

**Use Case:** An agent learns conventions in workspace A. User wants to transfer those conventions to workspace B (new project with same tech stack).

**Options:**

| Option | Mechanism | Pros | Cons |
|--------|-----------|------|------|
| **A. No portability** | Memories are workspace-bound. Export = manual copy via API. | Simplest. Avoids cross-workspace pollution. | Poor UX for users with multiple projects. |
| **B. Export/import via JSON** | `nous memory export --workspace A --output conventions.json` + `nous memory import --workspace B --input conventions.json` | Standard format. Works across installations. | Loses embeddings (must re-embed on import). Loses relations (IDs change). |
| **C. Shared memory pools** | Create a "template" workspace with shared conventions. All projects read from template. | Zero-copy sharing. Single source of truth. | Requires workspace hierarchy (see 9.2). Tight coupling between projects. |

**Recommendation:** Option B (export/import) for Phase 4. Operators can export high-value memories (conventions, decisions) and seed new workspaces. Re-embedding on import is acceptable (one-time cost).

**Decision owner:** Product during Phase 4 based on user demand.

---

### 9.8 Relation Type Extensibility

**Question:** Should the set of relation types (Supersedes, ConflictsWith, etc.) be extensible?

**Current State:** 6 hardcoded types (`RelationType` enum, `memory/mod.rs:107-116`).

**Use Case:** Domain-specific relations. Medical agent: "DifferentialDiagnosis" relation between symptoms and diseases. Legal agent: "Cites" relation between case law memories.

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Fixed set (current)** | Simple. All agents share vocabulary. Relation semantics are clear. | Inflexible. Cannot model domain-specific relations. |
| **B. Custom relation types** | Agent TOML declares `[memory.relation_types]` section. DB stores as TEXT. | Flexible. But cross-agent interop suffers. Each agent invents its own relations. |
| **C. Relation registry** | Central registry maps relation names → descriptions + semantic rules (transitive? reflexive?). | Consistent vocabulary. Supports reasoning over relations (e.g., transitive closure). | Complex. Requires ontology design. |

**Recommendation:** Option A for Phase 1-4. Fixed relation types are sufficient for general-purpose agents. Domain-specific systems (medical, legal) should extend via external graph DBs (Neo4j integration via rig-neo4j) rather than overloading the nous memory system.

**Decision owner:** Architecture review during Phase 1.

---

### 9.9 LLM-Based Extraction Accuracy

**Question:** How do we validate that structured marker extraction (`MEMORY[...]`) is accurate?

**Context (§3.3):** Agents generate `MEMORY[...]` blocks. Extraction parses these. But what if the LLM generates malformed blocks or omits required fields?

**Options:**

| Option | Validation | Pros | Cons |
|--------|------------|------|------|
| **A. Best-effort parsing** | Parse what's present. Skip malformed blocks. Log errors. | Tolerant. Agents aren't blocked by syntax errors. | Silent failures. Missing memories may go unnoticed. |
| **B. Strict validation** | Reject completions with malformed `MEMORY[...]` blocks. Return error to LLM. | Forces LLM to fix syntax. Guarantees well-formed memories. | Brittle. Slows agent. Requires retry loop. |
| **C. Secondary LLM validation** | After extraction, pass extracted memories to LLM: "Are these accurate summaries?" | Catches errors. But expensive (extra LLM call per completion). | High latency. Doubles token cost. |

**Recommendation:** Option A for Phase 3. Log malformed blocks to `memory_extraction_errors` table for post-hoc analysis. If error rate >10%, revisit in Phase 4 with Option C (secondary validation).

**Decision owner:** Engineering during Phase 3 alpha testing.

---

### 9.10 Agent Memory Lifecycle After Agent Deletion

**Question:** When an agent is deregistered, what happens to its memories?

**Options:**

| Option | Behavior | Pros | Cons |
|--------|----------|------|------|
| **A. Preserve memories** | Memories remain in DB with `agent_id` set. Queryable by other agents (if workspace-scoped) or admins. | Audit trail. Historical context preserved. | DB grows indefinitely. Orphaned memories clutter search results. |
| **B. Archive memories** | Set `archived=1` on all agent's memories. Hidden from default queries. | Hides clutter. Retrievable if needed. | Archived memories still consume storage. |
| **C. Cascade delete** | Delete all memories where `agent_id = <deleted agent>`. | Clean DB. No orphans. | Loses audit trail. Cannot recover agent's learnings. |
| **D. Transfer to workspace** | Set `agent_id = NULL` on all agent's memories → becomes workspace-wide. | Preserves knowledge. Other agents benefit. | May pollute workspace with agent-specific memories that aren't generalizable. |

**Recommendation:** Option B (archive) for Phase 2-3. Operators can manually delete archived memories after retention period (e.g., 90 days). Option D (transfer) is opt-in via explicit operator action (`nous memory transfer --agent X --to-workspace`).

**Decision owner:** Product during Phase 2 based on data retention policy requirements.

---

### 9.11 Agent Skills Specification Alignment

**Question:** Should the `[memory]` section design align with or extend the [Agent Skills Specification](https://agentskills.io) for broader interoperability?

**Context:** The Agent Skills Specification provides a standard approach for declaring agent capabilities, resources, and requirements in TOML form files. Aligning our `[memory]` section with this spec would enable:
- Cross-framework compatibility (agents portable between nous and other spec-compliant systems)
- Standardized tooling for form validation and documentation
- Community-driven evolution of memory configuration patterns

**Options:**

| Option | Pros | Cons |
|--------|------|------|
| **A. Full alignment** | Our `[memory]` section follows spec conventions exactly. Agents are portable. Community tooling works. | May constrain design. Spec may not cover nous-specific needs (e.g., vec pool selection). |
| **B. Partial alignment** | Use spec-compliant field names where possible. Add nous-specific extensions with `x-nous-` prefix per spec's extension mechanism. | Balances portability and flexibility. Core fields portable, extensions documented. | Some features non-portable. Cross-framework agents must handle extensions gracefully. |
| **C. No alignment** | Design `[memory]` section independently. Simpler short-term. | No external dependencies. Full design freedom. | Agents locked into nous ecosystem. No cross-framework portability. |

**Recommendation:** Option B (partial alignment) for Phase 1. Follow spec conventions for core memory config (scope, retrieval strategy). Use `x-nous-` prefix for nous-specific extensions (e.g., `x-nous-vec-pool`, `x-nous-fts-tokenizer`). This provides portability for common use cases while preserving flexibility for advanced nous features.

Monitor agentskills.io evolution during Phase 2-3. If the spec adds memory sections that conflict with our design, propose updates to the spec based on our production experience.

**Decision owner:** Architecture review during Phase 1, with input from agentskills.io maintainers if available.

---

## Summary of Decisions

| ID | Decision | Recommended | Phase | Owner |
|----|----------|-------------|-------|-------|
| 9.1 | Custom memory types | No (use topic_key) | Phase 1 | Engineering |
| 9.2 | Memory scope hierarchy | Flat (no hierarchy) | Phase 2 | Product/UX |
| 9.3 | Context injection strategy | Static per-agent | Phase 2 | Engineering |
| 9.4 | Token budget allocation | Dynamic allocation | Phase 2 | Engineering |
| 9.5 | Embedding model flexibility | Global model only | Phase 1-3 | Product |
| 9.6 | Memory consolidation triggers | Manual (Phase 4) → threshold (future) | Phase 4 | Engineering |
| 9.7 | Memory portability | Export/import JSON | Phase 4 | Product |
| 9.8 | Relation type extensibility | Fixed set | Phase 1-4 | Architecture |
| 9.9 | LLM extraction accuracy | Best-effort + logging | Phase 3 | Engineering |
| 9.10 | Memory lifecycle after agent deletion | Archive memories | Phase 2 | Product |
| 9.11 | Agent Skills Spec alignment | Partial alignment (x-nous- extensions) | Phase 1 | Architecture |

All decisions are revisable based on user feedback and production metrics. This document will be updated as decisions solidify.
