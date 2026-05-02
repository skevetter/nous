# Agent Tools Design

**Status:** Draft
**Date:** 2026-05-02
**Workstream:** NOUS-061

---

## Table of Contents

1. [Current State](#1-current-state)
2. [Tool Architecture](#2-tool-architecture)
3. [Default Tool Set](#3-default-tool-set)
4. [Tool Declaration in Agent Forms](#4-tool-declaration-in-agent-forms)
5. [Tool Execution Model](#5-tool-execution-model)
6. [Tool Discovery & Registration](#6-tool-discovery--registration)
7. [Custom Tools](#7-custom-tools)
8. [Integration with rig](#8-integration-with-rig)
9. [Migration Plan](#9-migration-plan)
10. [Open Decisions](#10-open-decisions)

---

## 1. Current State

### 1.1 MCP Tool Surface (nous-daemon)

The daemon exposes 105 tools via an MCP-compatible HTTP interface at `POST /mcp/tools`
and `POST /mcp/call` (`crates/nous-daemon/src/lib.rs:126-128`). Tools are defined as
static `ToolSchema` structs in `crates/nous-daemon/src/routes/mcp.rs:24-29`:

```rust
#[derive(Serialize, Clone)]
pub struct ToolSchema {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}
```

Tool dispatch is a single `match` on `request.name` in `call_tool()` that routes to
`nous_core` functions — rooms, messages, tasks, agents, memory, schedules, worktrees,
and inventory. There is no registry, no middleware pipeline, no permission checks, and
no tool-level timeout or sandboxing.

Tool categories currently served:

| Category | Count | Examples |
|----------|-------|---------|
| Rooms & messaging | 13 | `room_create`, `room_post_message`, `room_search`, `room_wait`, `room_subscribe`, `room_inspect` |
| Tasks | 19 | `task_create`, `task_list`, `task_update`, `task_close`, `task_depends_add`, `task_batch_close` |
| Agents | 31 | `agent_register`, `agent_list`, `agent_inspect`, `agent_heartbeat`, `agent_spawn`, `agent_invoke` |
| Memory | 19 | `memory_save`, `memory_search`, `memory_search_hybrid`, `memory_relate`, `memory_embed`, `memory_session_summary` |
| Schedules | 7 | `schedule_create`, `schedule_list`, `schedule_delete`, `schedule_runs_list`, `schedule_health` |
| Worktrees | 5 | `worktree_create`, `worktree_list`, `worktree_delete` |
| Inventory & artifacts | 11 | `inventory_register`, `inventory_list`, `inventory_search`, `artifact_register`, `artifact_update` |

### 1.2 Scheduled Actions

The scheduler (`crates/nous-daemon/src/scheduler.rs:261`) dispatches four action types:

```rust
match schedule.action_type.as_str() {
    "mcp_tool"       => dispatch_mcp_tool(state, &schedule.action_payload).await,
    "shell"          => dispatch_shell(&schedule.action_payload, config).await,
    "http"           => dispatch_http(&schedule.action_payload).await,
    "agent_invoke"   => dispatch_agent_invoke(state, &schedule.action_payload).await,
    other            => Err(NousError::Validation(format!("unknown action_type: {other}"))),
}
```

The `action_type` enum is enforced at the database level
(`crates/nous-core/src/db/pool.rs:221`):

```sql
action_type TEXT NOT NULL CHECK(action_type IN ('mcp_tool','shell','http','agent_invoke'))
```

### 1.3 Agent Form System (No Tool Declarations)

Agent definitions (`crates/nous-core/src/agents/definition.rs:7-44`) support four TOML
sections:

```rust
pub struct AgentDefinition {
    pub agent: AgentSection,       // name, type, version, namespace, description
    pub process: Option<ProcessSection>,   // type, spawn_command, working_dir, auto_restart
    pub skills: Option<SkillsSection>,     // refs: Vec<String>
    pub metadata: Option<MetadataSection>, // model, timeout, tags
}
```

There is **no `[tools]` section**. Agents cannot declare which tools they need, which
tools they are forbidden from using, or any tool-level permissions or constraints. The
skill refs (`[skills].refs`) load markdown files from a skills directory
(`definition.rs:60-85`) but skills are opaque text — no structured tool bindings.

### 1.4 Process Types and LLM Integration

Process dispatch (`process_manager.rs`) routes to `invoke_claude` (rig Agent) or
`invoke_shell` (subprocess) based on `agent.process_type`. The rig `AgentBuilder`
supports `.tool(impl Tool)` registration but **no tools are currently registered** —
agents operate in prompt-only mode.

### 1.5 Sandbox Infrastructure

`crates/nous-core/src/agents/sandbox.rs` defines `SandboxConfig` with container
isolation primitives (image, CPUs, memory, network policy, volumes, secrets). The
`processes` module supports `create_sandbox_process()` for container-based execution.
This provides the substrate for sandboxed tool execution.

### 1.6 Gap Analysis

| Capability | Current State | Gap |
|-----------|---------------|-----|
| Tool trait abstraction | No Rust `Tool` trait in nous-core | Need trait bridging to rig's `Tool` trait |
| Tool registry | Static `Vec<ToolSchema>` in mcp.rs | Need dynamic registry with add/remove/query |
| Tool permissions | None — all 105 tools are unconditionally available | Need per-agent tool allowlists/denylists |
| Tool declaration in forms | No `[tools]` section in TOML | Need declarative tool configuration |
| Tool sandboxing | Sandbox infra exists but not tool-aware | Need per-tool execution constraints |
| Semantic tool selection | No embeddings for tool descriptions | Need rig `ToolEmbedding` integration |
| Custom tools | Not possible | Need plugin/registration system |
| Tool timeout/retry | No tool-level timeout | Need per-tool execution policy |

## 2. Tool Architecture

### 2.1 Core Trait: `AgentTool`

The central abstraction is a Rust trait that bridges nous-native tools with rig's `Tool`
trait. This trait lives in a new `crates/nous-core/src/tools/` module.

```rust
// crates/nous-core/src/tools/mod.rs

use std::future::Future;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::NousError;

pub mod builtin;
pub mod registry;
pub mod permissions;
pub mod execution;

/// Metadata describing a tool's capabilities and constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub category: ToolCategory,
    pub version: String,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub permissions: ToolPermissions,
    pub execution_policy: ExecutionPolicy,
    pub tags: Vec<String>,
}

/// Tool categories for organization and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    FileSystem,
    Shell,
    Http,
    Memory,
    AgentComms,
    Database,
    CodeAnalysis,
    Custom,
}

/// The core tool trait. All nous tools implement this.
pub trait AgentTool: Send + Sync + 'static {
    fn metadata(&self) -> &ToolMetadata;

    fn call(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> impl Future<Output = Result<ToolOutput, ToolError>> + Send;
}
```

### 2.2 Tool Context

Every tool invocation receives a `ToolContext` carrying the calling agent's identity,
permissions, and runtime state. This enables per-agent authorization without global
mutable state.

```rust
/// Runtime context passed to every tool invocation.
#[derive(Debug, Clone)]
pub struct ToolContext {
    pub agent_id: String,
    pub agent_name: String,
    pub namespace: String,
    pub workspace_dir: Option<PathBuf>,
    pub session_id: Option<String>,
    pub timeout: Duration,
    pub permissions: ResolvedPermissions,
}

```

**`ToolContext` fields:**

| Field | Type | Populated When |
|-------|------|----------------|
| `agent_id` | `String` | Always — set from `agent.id` at spawn |
| `agent_name` | `String` | Always — set from `agent.name` at spawn |
| `namespace` | `String` | Always — set from `agent.namespace` at spawn |
| `workspace_dir` | `Option<PathBuf>` | When the agent form specifies `[process].working_dir` |
| `session_id` | `Option<String>` | When the invocation is tracked (i.e., `invocation.id` exists) |
| `timeout` | `Duration` | Always — defaults to 30s, overridden by `[tools.execution].default_timeout_secs` |
| `permissions` | `ResolvedPermissions` | Always — resolved from agent form `[tools.permissions]` merged with type defaults |

```rust
/// Resolved permission set for the calling agent.
#[derive(Debug, Clone)]
pub struct ResolvedPermissions {
    pub allowed_tools: Option<Vec<String>>,
    pub denied_tools: Option<Vec<String>>,
    pub allowed_paths: Option<Vec<PathBuf>>,
    pub network_access: NetworkPolicy,
    pub max_output_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    None,
    Isolated,
    AllowList,
    Unrestricted,
}
```

### 2.3 Tool Output and Error Types

```rust
/// Structured output from a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: Vec<ToolContent>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    Text { text: String },
    Json { data: Value },
    Binary { mime_type: String, data: Vec<u8> },
    Error { message: String },
}

/// Tool-specific error type.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("timeout after {0:?}")]
    Timeout(Duration),

    #[error("tool not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Internal(#[from] NousError),
}
```

### 2.4 Tool Permissions

```rust
/// Declarative permissions for a tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPermissions {
    pub filesystem: Option<FileSystemPermission>,
    pub network: Option<NetworkPermission>,
    pub shell: Option<ShellPermission>,
    pub requires_confirmation: bool,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSystemPermission {
    pub read_paths: Vec<String>,
    pub write_paths: Vec<String>,
    pub deny_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPermission {
    pub allowed_hosts: Vec<String>,
    pub denied_hosts: Vec<String>,
    pub max_request_size_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellPermission {
    pub allowed_commands: Vec<String>,
    pub denied_commands: Vec<String>,
    pub allow_arbitrary: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
    Critical,
}
```

### 2.5 Execution Policy

```rust
/// Per-tool execution constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub max_output_bytes: usize,
    pub sandbox_required: bool,
    pub idempotent: bool,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            max_retries: 0,
            retry_delay_ms: 1000,
            max_output_bytes: 1_048_576, // 1 MiB
            sandbox_required: false,
            idempotent: false,
        }
    }
}
```

### 2.6 Architecture Diagram

```
┌──────────────────────────────────────────────────────┐
│                    Agent Runtime                      │
│  ┌─────────────┐   ┌───────────────┐                │
│  │ rig Agent    │──▶│ ToolSet       │                │
│  │ (LLM)       │   │ (rig adapter) │                │
│  └─────────────┘   └───────┬───────┘                │
│                             │                        │
│                    ┌────────▼────────┐               │
│                    │  ToolRegistry   │               │
│                    │  (dynamic)      │               │
│                    └────────┬────────┘               │
│            ┌───────────┬────┴────┬──────────┐       │
│            ▼           ▼        ▼          ▼       │
│      ┌──────────┐ ┌────────┐ ┌───────┐ ┌───────┐  │
│      │BuiltinFS │ │ShellTool│ │HttpTool│ │Custom │  │
│      │MemoryTool│ │         │ │       │ │Tools  │  │
│      └────┬─────┘ └───┬────┘ └──┬────┘ └──┬────┘  │
│           │            │         │          │       │
│    ┌──────▼────────────▼─────────▼──────────▼───┐  │
│    │           Permission Gate                   │  │
│    │   (ToolContext + ResolvedPermissions)        │  │
│    └────────────────────┬────────────────────────┘  │
│                         ▼                            │
│    ┌──────────────────────────────────────────────┐  │
│    │          Execution Engine                     │  │
│    │  (timeout, retry, output capture, sandbox)    │  │
│    └──────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────┘
```

## 3. Default Tool Set

Every nous agent ships with a curated set of built-in tools, organized by category.
Tools are registered in `crates/nous-core/src/tools/builtin/mod.rs`. The built-in set
provides file system operations, shell execution, HTTP requests, memory persistence,
agent communication, and code analysis — covering the core capabilities agents need for
autonomous software engineering tasks.

### 3.1 File System Tools

| Tool Name | Description | Risk | Sandbox |
|-----------|-------------|------|---------|
| `fs_read` | Read file contents (text or binary) | Low | No |
| `fs_write` | Write/overwrite file | Medium | No |
| `fs_list` | List directory contents with glob patterns | Low | No |
| `fs_search` | Search file contents via regex (ripgrep-style) | Low | No |
| `fs_edit` | Apply targeted edits (old_string → new_string) | Medium | No |
| `fs_stat` | Get file metadata (size, modified, permissions) | Low | No |
| `fs_mkdir` | Create directories | Low | No |
| `fs_delete` | Delete files or directories | High | No |

```rust
// crates/nous-core/src/tools/builtin/filesystem.rs

pub struct FsReadTool;

impl AgentTool for FsReadTool {
    fn metadata(&self) -> &ToolMetadata {
        &ToolMetadata {
            name: "fs_read".into(),
            description: "Read file contents from the local filesystem".into(),
            category: ToolCategory::FileSystem,
            version: "0.1.0".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute file path" },
                    "offset": { "type": "integer", "description": "Line offset (0-based)" },
                    "limit": { "type": "integer", "description": "Max lines to read" }
                },
                "required": ["path"]
            }),
            output_schema: None,
            permissions: ToolPermissions {
                filesystem: Some(FileSystemPermission {
                    read_paths: vec!["**".into()],
                    write_paths: vec![],
                    deny_paths: vec![],
                }),
                ..Default::default()
            },
            execution_policy: ExecutionPolicy {
                timeout_secs: 10,
                ..Default::default()
            },
            tags: vec!["filesystem".into(), "read".into()],
        }
    }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let path = args.get("path").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'path' required".into()))?;
        let path = PathBuf::from(path);

        // Validate path against ctx.permissions.allowed_paths
        self.check_read_permission(&path, ctx)?;

        let content = tokio::fs::read_to_string(&path).await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Apply offset/limit for large files
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
        let lines: String = content.lines()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(ToolOutput {
            content: vec![ToolContent::Text { text: lines }],
            metadata: None,
        })
    }
}
```

### 3.2 Shell Execution Tools

| Tool Name | Description | Risk | Sandbox |
|-----------|-------------|------|---------|
| `shell_exec` | Execute a shell command | High | Optional |
| `shell_exec_background` | Run command in background, return handle | High | Optional |
| `shell_read_output` | Read output from a background command | Low | No |
| `shell_kill` | Kill a background process | Medium | No |

Shell tools are the highest-risk category. Default policy: `requires_confirmation: true`
for non-sandboxed agents. Sandboxed agents (container-based) run shell commands without
confirmation since the blast radius is contained.

### 3.3 HTTP Tools

| Tool Name | Description | Risk | Sandbox |
|-----------|-------------|------|---------|
| `http_request` | Make an HTTP request (GET/POST/PUT/DELETE) | Medium | No |
| `http_fetch` | Fetch URL content and extract text | Low | No |

Network tools respect `NetworkPermission.allowed_hosts`. Default policy for agents
without explicit network config: `NetworkPolicy::Isolated` (no outbound requests).

### 3.4 Memory Tools

| Tool Name | Description | Risk | Sandbox |
|-----------|-------------|------|---------|
| `memory_save` | Save a memory (decision, convention, fact, etc.) | Low | No |
| `memory_search` | Search memories via FTS5 | Low | No |
| `memory_search_hybrid` | Hybrid FTS + vector search | Low | No |
| `memory_get_context` | Get recent context memories | Low | No |
| `memory_relate` | Create a relationship between memories | Low | No |

These wrap the existing `crates/nous-core/src/memory/mod.rs` functions. The `ToolContext`
supplies `agent_id` and `workspace_id` to scope memory operations. No agent can access
another agent's private workspace memories unless explicitly granted via
`agent_workspace_access`.

### 3.5 Agent Communication Tools

| Tool Name | Description | Risk | Sandbox |
|-----------|-------------|------|---------|
| `room_post` | Post a message to a chat room | Low | No |
| `room_read` | Read messages from a room | Low | No |
| `room_create` | Create a new chat room | Low | No |
| `room_wait` | Wait for a message (blocking, with timeout) | Low | No |
| `task_create` | Create a task in the task management system | Low | No |
| `task_update` | Update task status or add a note | Low | No |

These correspond to the existing MCP tools but filtered through the permission system.

### 3.6 Code Analysis Tools

| Tool Name | Description | Risk | Sandbox |
|-----------|-------------|------|---------|
| `code_grep` | Search codebase with regex patterns | Low | No |
| `code_glob` | Find files by glob pattern | Low | No |
| `code_symbols` | List functions/types/imports in a file | Low | No |

Code analysis tools are read-only and always low-risk. They operate within
`ctx.workspace_dir` boundaries.

### 3.7 Default Tool Sets by Agent Type

Not every agent needs every tool. Default tool sets are configured per `AgentType`:

| Agent Type | Default Tools | Rationale |
|-----------|--------------|-----------|
| Engineer | All filesystem, shell, code analysis, memory, comms | Full implementation capability |
| Manager | Memory, comms, task management | Coordination, not implementation |
| Director | Memory, comms, task management | Strategic coordination |
| Senior Manager | Memory, comms, task management | Portfolio oversight |

These defaults are overridable via the `[tools]` section in agent forms (Section 4).

## 4. Tool Declaration in Agent Forms

The existing `AgentDefinition` struct (`crates/nous-core/src/agents/definition.rs:7-12`)
gains a new optional `[tools]` section. This follows the Agent Skills Specification
pattern of declarative capability configuration while extending it with nous-specific
permission and constraint semantics.

### 4.1 TOML Schema

```toml
[agent]
name       = "code-reviewer"
type       = "engineer"
version    = "1.0.0"
description = "Reviews pull requests for code quality"

[process]
type          = "claude"
spawn_command = "claude --model claude-sonnet-4-6"

[skills]
refs = ["code-review", "git-workflow"]

[tools]
# Explicit allowlist — only these tools are available.
# If omitted, the agent gets the default set for its type (see Section 3.7).
allow = [
    "fs_read",
    "fs_search",
    "code_grep",
    "code_glob",
    "code_symbols",
    "shell_exec",
    "memory_save",
    "memory_search",
    "room_post",
    "room_read",
]

# Explicit denylist — these tools are never available, even if in `allow`.
deny = ["fs_delete", "shell_kill"]

# Custom tools loaded from external crates or scripts.
custom = [
    { name = "lint_check", script = "scripts/lint.sh", description = "Run project linter" },
]

[tools.permissions]
filesystem_read  = ["**/*.rs", "**/*.toml", "**/*.md"]
filesystem_write = []
network_hosts    = []
shell_commands   = ["git", "cargo", "rustfmt"]
require_confirmation = false

[tools.execution]
default_timeout_secs = 30
max_retries          = 1
max_output_bytes     = 2097152   # 2 MiB
sandbox_required     = false

[metadata]
model   = "global.anthropic.claude-sonnet-4-6-v1"
timeout = 3600
tags    = ["review", "quality"]
```

### 4.2 Full-Featured Agent Form Example

```toml
[agent]
name        = "build-engineer"
type        = "engineer"
version     = "1.0.0"
namespace   = "eng"
description = "Implements features, fixes bugs, and runs builds"

[process]
type          = "claude"
spawn_command = "claude --model claude-opus-4-6"
working_dir   = "/workspace/project"
auto_restart  = false

[skills]
refs = ["code-review", "git-workflow", "rust-best-practices"]

[tools]
allow = [
    "fs_read", "fs_write", "fs_edit", "fs_list", "fs_search", "fs_mkdir",
    "shell_exec", "shell_exec_background", "shell_read_output",
    "code_grep", "code_glob", "code_symbols",
    "memory_save", "memory_search", "memory_search_hybrid", "memory_get_context",
    "room_post", "room_read", "room_wait",
    "task_update",
    "http_fetch",
]
deny = ["fs_delete"]

[tools.permissions]
filesystem_read  = ["**"]
filesystem_write = ["src/**", "tests/**", "Cargo.toml"]
network_hosts    = ["crates.io", "docs.rs", "github.com"]
shell_commands   = ["cargo", "rustfmt", "git", "grep", "find"]
require_confirmation = false

[tools.execution]
default_timeout_secs = 60
max_retries          = 2
max_output_bytes     = 4194304   # 4 MiB
sandbox_required     = false

[metadata]
model   = "global.anthropic.claude-opus-4-6-v1"
timeout = 7200
tags    = ["build", "implementation"]
```

### 4.3 Rust Type Changes

The `AgentDefinition` struct in `definition.rs` gains a new field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub agent: AgentSection,
    pub process: Option<ProcessSection>,
    pub skills: Option<SkillsSection>,
    pub tools: Option<ToolsSection>,       // NEW
    pub metadata: Option<MetadataSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsSection {
    pub allow: Option<Vec<String>>,
    pub deny: Option<Vec<String>>,
    pub custom: Option<Vec<CustomToolDef>>,
    pub permissions: Option<ToolPermissionsConfig>,
    pub execution: Option<ExecutionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomToolDef {
    pub name: String,
    pub script: Option<String>,
    pub description: String,
    pub input_schema: Option<Value>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPermissionsConfig {
    pub filesystem_read: Option<Vec<String>>,
    pub filesystem_write: Option<Vec<String>>,
    pub network_hosts: Option<Vec<String>>,
    pub shell_commands: Option<Vec<String>>,
    pub require_confirmation: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionConfig {
    pub default_timeout_secs: Option<u64>,
    pub max_retries: Option<u32>,
    pub max_output_bytes: Option<usize>,
    pub sandbox_required: Option<bool>,
}
```

### 4.4 Resolution Logic

Tool configuration follows a layered resolution, similar to the LLM config pattern
established in `multi-provider-llm.md`:

```
Agent form [tools] section  >  Agent type defaults  >  Platform defaults
```

1. If `[tools].allow` is specified, use exactly that set.
2. If `[tools].allow` is absent, use the default set for the agent's type (Section 3.7).
3. Apply `[tools].deny` as a filter on top (always removes tools, never adds).
4. Merge `[tools].permissions` with tool-level defaults (agent form overrides).
5. Merge `[tools].execution` with per-tool `ExecutionPolicy` defaults.

This matches the Agent Skills Specification's `allowed-tools` field semantics while
extending it with deny-lists and fine-grained permission scoping.

## 5. Tool Execution Model

### 5.1 Execution Pipeline

Every tool invocation passes through a five-stage pipeline:

```
1. Validate  →  2. Authorize  →  3. Execute  →  4. Capture  →  5. Record
```

```rust
// crates/nous-core/src/tools/execution.rs

pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
}

impl ToolExecutor {
    pub async fn invoke(
        &self,
        tool_name: &str,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        // 1. Validate: look up tool, validate args against input_schema
        let tool = self.registry.get(tool_name)
            .ok_or_else(|| ToolError::NotFound(tool_name.into()))?;
        self.validate_args(&tool.metadata().input_schema, &args)?;

        // 2. Authorize: check agent permissions against tool requirements
        self.authorize(tool.metadata(), ctx)?;

        // 3. Execute: run with timeout and optional sandbox
        let policy = &tool.metadata().execution_policy;
        let result = self.execute_with_policy(tool.as_ref(), args, ctx, policy).await;

        // 4. Capture: truncate output if over max_output_bytes
        let output = self.capture_output(result, policy.max_output_bytes)?;

        // 5. Record: log invocation for analytics and procedural memory
        self.record_invocation(tool_name, ctx, &output).await;

        Ok(output)
    }

    async fn execute_with_policy(
        &self,
        tool: &dyn AgentToolDyn,
        args: Value,
        ctx: &ToolContext,
        policy: &ExecutionPolicy,
    ) -> Result<ToolOutput, ToolError> {
        let timeout = Duration::from_secs(policy.timeout_secs);
        let mut last_err = None;

        for attempt in 0..=policy.max_retries {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(
                    policy.retry_delay_ms * attempt as u64
                )).await;
            }

            match tokio::time::timeout(timeout, tool.call_dyn(args.clone(), ctx)).await {
                Ok(Ok(output)) => return Ok(output),
                Ok(Err(e)) if policy.idempotent && attempt < policy.max_retries => {
                    last_err = Some(e);
                    continue;
                }
                Ok(Err(e)) => return Err(e),
                Err(_) => {
                    last_err = Some(ToolError::Timeout(timeout));
                    if !policy.idempotent || attempt >= policy.max_retries {
                        return Err(ToolError::Timeout(timeout));
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| ToolError::ExecutionFailed("unknown".into())))
    }
}
```

### 5.2 Authorization

The authorization check runs before every tool invocation:

```rust
fn authorize(
    &self,
    tool_meta: &ToolMetadata,
    ctx: &ToolContext,
) -> Result<(), ToolError> {
    let perms = &ctx.permissions;

    // Check allowlist
    if let Some(ref allowed) = perms.allowed_tools {
        if !allowed.iter().any(|t| t == &tool_meta.name) {
            return Err(ToolError::PermissionDenied(
                format!("tool '{}' not in agent's allowlist", tool_meta.name)
            ));
        }
    }

    // Check denylist
    if let Some(ref denied) = perms.denied_tools {
        if denied.iter().any(|t| t == &tool_meta.name) {
            return Err(ToolError::PermissionDenied(
                format!("tool '{}' is explicitly denied", tool_meta.name)
            ));
        }
    }

    // Check network policy
    if tool_meta.category == ToolCategory::Http
        && perms.network_access == NetworkPolicy::None
    {
        return Err(ToolError::PermissionDenied(
            "network access not permitted for this agent".into()
        ));
    }

    Ok(())
}
```

### 5.3 Sandboxed Execution

For tools with `sandbox_required: true` or agents with a `SandboxConfig` in their
process section, tool execution is delegated to a container:

```rust
async fn execute_sandboxed(
    &self,
    tool: &dyn AgentToolDyn,
    args: Value,
    ctx: &ToolContext,
    sandbox_config: &SandboxConfig,
) -> Result<ToolOutput, ToolError> {
    // Implementation in Phase 2 — see Section 9
    // Integrates with SandboxConfig from sandbox.rs and create_sandbox_process()
    let sandbox = SandboxBuilder::new(sandbox_config)
        .mount(ctx.workspace_dir.as_deref().unwrap_or(Path::new(".")), "/workspace")
        .network_policy(sandbox_config.network_policy)
        .build()
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

    let output = sandbox.exec(|| tool.call_dyn(args, ctx)).await
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

    output
}
```

This integrates with the existing `SandboxConfig` struct from
`crates/nous-core/src/agents/sandbox.rs:4-13` and the `create_sandbox_process` function
from `crates/nous-core/src/agents/processes.rs:147-185`.

### 5.4 Error Handling Strategy

| Error Type | Recovery | Agent Visible |
|-----------|----------|---------------|
| `PermissionDenied` | No retry. Return error to LLM. | Yes — "You don't have permission to use this tool." |
| `InvalidArgs` | No retry. Return schema hint to LLM. | Yes — "Invalid arguments. Expected: {schema}" |
| `Timeout` | Retry if idempotent. | Yes — "Tool timed out after {N}s." |
| `ExecutionFailed` | Retry if idempotent. | Yes — "Tool failed: {message}" |
| `NotFound` | No retry. | Yes — "Tool '{name}' not found." |
| `Internal` | Log and return generic error. | Yes — "Internal error." |

All errors are surfaced to the LLM as `ToolContent::Error` so the agent can decide
whether to try an alternative approach. This follows the pattern established by Claude
Code and OpenAI Assistants where tool errors are informational, not fatal.

### 5.5 Output Truncation

Large outputs waste LLM context window tokens. The executor enforces
`max_output_bytes` by truncating with a trailer:

```rust
fn capture_output(
    &self,
    result: Result<ToolOutput, ToolError>,
    max_bytes: usize,
) -> Result<ToolOutput, ToolError> {
    let output = result?;
    let serialized = serde_json::to_string(&output.content)
        .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

    if serialized.len() > max_bytes {
        let truncated = &serialized[..max_bytes];
        Ok(ToolOutput {
            content: vec![ToolContent::Text {
                text: format!(
                    "{}\n\n[Output truncated at {} bytes. Total: {} bytes]",
                    truncated, max_bytes, serialized.len()
                ),
            }],
            metadata: output.metadata,
        })
    } else {
        Ok(output)
    }
}
```

## 6. Tool Discovery & Registration

### 6.1 Tool Registry

The `ToolRegistry` is the central store for all available tools. It replaces the static
`get_tool_schemas()` function in `crates/nous-daemon/src/routes/mcp.rs:51`.

```rust
// crates/nous-core/src/tools/registry.rs

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{AgentTool, ToolMetadata, ToolCategory};

pub type DynTool = Arc<dyn AgentToolDyn>;

/// Central registry of all available tools.
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, DynTool>>,
    categories: RwLock<HashMap<ToolCategory, Vec<String>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            categories: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register(&self, tool: impl AgentTool) {
        let meta = tool.metadata().clone();
        let name = meta.name.clone();
        let category = meta.category;
        let dyn_tool: DynTool = Arc::new(tool);

        let mut tools = self.tools.write().await;
        tools.insert(name.clone(), dyn_tool);

        let mut cats = self.categories.write().await;
        cats.entry(category).or_default().push(name);
    }

    pub async fn get(&self, name: &str) -> Option<DynTool> {
        self.tools.read().await.get(name).cloned()
    }

    pub async fn list(&self) -> Vec<ToolMetadata> {
        self.tools.read().await.values()
            .map(|t| t.metadata_dyn().clone())
            .collect()
    }

    pub async fn list_by_category(&self, category: ToolCategory) -> Vec<ToolMetadata> {
        let cats = self.categories.read().await;
        let tools = self.tools.read().await;
        cats.get(&category)
            .map(|names| {
                names.iter()
                    .filter_map(|n| tools.get(n))
                    .map(|t| t.metadata_dyn().clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn deregister(&self, name: &str) -> bool {
        let mut tools = self.tools.write().await;
        if let Some(tool) = tools.remove(name) {
            let category = tool.metadata_dyn().category;
            let mut cats = self.categories.write().await;
            if let Some(names) = cats.get_mut(&category) {
                names.retain(|n| n != name);
            }
            true
        } else {
            false
        }
    }

    pub async fn count(&self) -> usize {
        self.tools.read().await.len()
    }
}
```

### 6.2 Dynamic Dispatch Trait

To store heterogeneous tools in a single `HashMap`, we need object-safe wrappers:

```rust
/// Object-safe wrapper for AgentTool, enabling dynamic dispatch.
pub trait AgentToolDyn: Send + Sync + 'static {
    fn metadata_dyn(&self) -> &ToolMetadata;
    fn call_dyn(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>>;
}

impl<T: AgentTool> AgentToolDyn for T {
    fn metadata_dyn(&self) -> &ToolMetadata {
        self.metadata()
    }

    fn call_dyn(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(self.call(args, ctx))
    }
}
```

### 6.3 Built-in Tool Registration

At daemon startup, the default tools are registered before any agent is spawned:

```rust
// crates/nous-daemon/src/main.rs (or a dedicated init module)

pub async fn register_builtin_tools(registry: &ToolRegistry) {
    // Filesystem
    registry.register(FsReadTool).await;
    registry.register(FsWriteTool).await;
    registry.register(FsEditTool).await;
    registry.register(FsListTool).await;
    registry.register(FsSearchTool).await;
    registry.register(FsStatTool).await;
    registry.register(FsMkdirTool).await;
    registry.register(FsDeleteTool).await;

    // Shell
    registry.register(ShellExecTool).await;
    registry.register(ShellExecBackgroundTool).await;
    registry.register(ShellReadOutputTool).await;
    registry.register(ShellKillTool).await;

    // HTTP
    registry.register(HttpRequestTool).await;
    registry.register(HttpFetchTool).await;

    // Memory
    registry.register(MemorySaveTool).await;
    registry.register(MemorySearchTool).await;
    registry.register(MemorySearchHybridTool).await;
    registry.register(MemoryGetContextTool).await;
    registry.register(MemoryRelateTool).await;

    // Agent comms
    registry.register(RoomPostTool).await;
    registry.register(RoomReadTool).await;
    registry.register(RoomCreateTool).await;
    registry.register(RoomWaitTool).await;
    registry.register(TaskCreateTool).await;
    registry.register(TaskUpdateTool).await;

    // Code analysis
    registry.register(CodeGrepTool).await;
    registry.register(CodeGlobTool).await;
    registry.register(CodeSymbolsTool).await;
}
```

### 6.4 Agent-Scoped Tool Resolution

When an agent is spawned, its available tools are resolved from the registry:

```rust
/// Resolve the set of tools available to a specific agent.
pub async fn resolve_agent_tools(
    registry: &ToolRegistry,
    agent_def: &AgentDefinition,
    agent_type: &AgentType,
) -> Vec<DynTool> {
    let all_tools = registry.list().await;

    // Determine allowed tool names
    let allowed_names: HashSet<String> = if let Some(ref tools_section) = agent_def.tools {
        if let Some(ref allow) = tools_section.allow {
            allow.iter().cloned().collect()
        } else {
            default_tools_for_type(agent_type)
        }
    } else {
        default_tools_for_type(agent_type)
    };

    // Apply deny list
    let denied_names: HashSet<String> = agent_def.tools
        .as_ref()
        .and_then(|t| t.deny.as_ref())
        .map(|d| d.iter().cloned().collect())
        .unwrap_or_default();

    let final_names: HashSet<String> = allowed_names
        .difference(&denied_names)
        .cloned()
        .collect();

    // Collect matching tools from registry
    let mut resolved = Vec::new();
    for name in &final_names {
        if let Some(tool) = registry.get(name).await {
            resolved.push(tool);
        }
    }
    resolved
}

fn default_tools_for_type(agent_type: &AgentType) -> HashSet<String> {
    match agent_type {
        AgentType::Engineer => [
            "fs_read", "fs_write", "fs_edit", "fs_list", "fs_search",
            "fs_stat", "fs_mkdir", "shell_exec", "shell_exec_background",
            "shell_read_output", "code_grep", "code_glob", "code_symbols",
            "memory_save", "memory_search", "memory_search_hybrid",
            "memory_get_context", "room_post", "room_read", "room_wait",
            "task_update", "http_fetch",
        ].iter().map(|s| s.to_string()).collect(),
        AgentType::Manager | AgentType::Director | AgentType::SeniorManager => [
            "memory_save", "memory_search", "memory_search_hybrid",
            "memory_get_context", "memory_relate",
            "room_post", "room_read", "room_create", "room_wait",
            "task_create", "task_update",
        ].iter().map(|s| s.to_string()).collect(),
    }
}
```

### 6.5 Progressive Disclosure (Agent Skills Spec Alignment)

Following the Agent Skills Specification's three-stage progressive disclosure:

1. **Discovery** (~100 tokens per tool): At agent startup, only `name` and `description`
   from `ToolMetadata` are included in the system prompt — enough for the LLM to know
   when a tool is relevant.

2. **Activation**: When the LLM selects a tool, the full `input_schema` is provided
   for argument construction.

3. **Execution**: The tool runs through the execution pipeline (Section 5).

This minimizes context window consumption. Per-tool estimates: ~100 tokens for discovery
(name + description), ~500 tokens with full input schema. An agent with 25 tools uses
~2,500 tokens for discovery versus ~12,500 tokens if full schemas were always loaded.

## 7. Custom Tools

Custom tools allow users and developers to extend the tool system beyond the built-in
set. Three extension mechanisms are supported, ordered by complexity.

### 7.1 Script-Based Tools (Simplest)

Script tools wrap shell scripts or executables. They are declared in the `[tools].custom`
array of an agent form and require no Rust code.

```toml
[tools]
custom = [
    { name = "lint_check", script = "scripts/lint.sh", description = "Run project linter" },
    { name = "test_runner", script = "scripts/test.py", description = "Run test suite", timeout_secs = 120 },
]
```

The script tool adapter:

```rust
// crates/nous-core/src/tools/custom/script_tool.rs

pub struct ScriptTool {
    meta: ToolMetadata,
    script_path: PathBuf,
}

impl ScriptTool {
    pub fn from_def(def: &CustomToolDef, base_dir: &Path) -> Result<Self, NousError> {
        let script_path = base_dir.join(def.script.as_deref()
            .ok_or_else(|| NousError::Validation("script path required".into()))?);

        if !script_path.exists() {
            return Err(NousError::NotFound(
                format!("script not found: {}", script_path.display())
            ));
        }

        Ok(Self {
            meta: ToolMetadata {
                name: def.name.clone(),
                description: def.description.clone(),
                category: ToolCategory::Custom,
                version: "0.1.0".into(),
                input_schema: def.input_schema.clone().unwrap_or_else(|| {
                    serde_json::json!({
                        "type": "object",
                        "properties": {
                            "args": {
                                "type": "string",
                                "description": "Arguments to pass to the script"
                            }
                        }
                    })
                }),
                output_schema: None,
                permissions: ToolPermissions {
                    shell: Some(ShellPermission {
                        allowed_commands: vec![script_path.display().to_string()],
                        denied_commands: vec![],
                        allow_arbitrary: false,
                    }),
                    risk_level: RiskLevel::Medium,
                    ..Default::default()
                },
                execution_policy: ExecutionPolicy {
                    timeout_secs: def.timeout_secs.unwrap_or(30),
                    ..Default::default()
                },
                tags: vec!["custom".into(), "script".into()],
            },
            script_path,
        })
    }
}

impl AgentTool for ScriptTool {
    fn metadata(&self) -> &ToolMetadata { &self.meta }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let script_args = args.get("args")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let output = tokio::process::Command::new(&self.script_path)
            .arg(script_args)
            .current_dir(ctx.workspace_dir.as_deref().unwrap_or(Path::new(".")))
            .output()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if output.status.success() {
            Ok(ToolOutput {
                content: vec![ToolContent::Text { text: stdout.into_owned() }],
                metadata: None,
            })
        } else {
            Err(ToolError::ExecutionFailed(
                format!("exit code {}: {}", output.status, stderr)
            ))
        }
    }
}
```

### 7.2 Rust Plugin Tools (Advanced)

For performance-critical or complex tools, developers implement the `AgentTool` trait
directly in Rust and register them with the `ToolRegistry`:

```rust
// Example: A custom database query tool

pub struct DbQueryTool {
    meta: ToolMetadata,
    pool: SqlitePool,
}

impl AgentTool for DbQueryTool {
    fn metadata(&self) -> &ToolMetadata { &self.meta }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let query = args.get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs("'query' field required".into()))?;

        // Read-only queries only
        if !query.trim_start().to_uppercase().starts_with("SELECT") {
            return Err(ToolError::PermissionDenied(
                "only SELECT queries are permitted".into()
            ));
        }

        let rows: Vec<Value> = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?
            .iter()
            .map(|row| row_to_json(row)) // illustrative — actual impl uses sqlx::FromRow
            .collect();

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: serde_json::json!(rows) }],
            metadata: None,
        })
    }
}
```

### 7.3 MCP Server Tools (External)

For tools provided by external MCP servers, a proxy tool bridges the nous tool system
with remote MCP endpoints:

```rust
pub struct McpProxyTool {
    meta: ToolMetadata,
    server_url: String,
    tool_name: String,
}

impl AgentTool for McpProxyTool {
    fn metadata(&self) -> &ToolMetadata { &self.meta }

    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput, ToolError> {
        let request = serde_json::json!({
            "name": self.tool_name,
            "arguments": args,
        });

        let response = reqwest::Client::new()
            .post(&format!("{}/mcp/call", self.server_url))
            .json(&request)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let body: Value = response.json().await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput {
            content: vec![ToolContent::Json { data: body }],
            metadata: None,
        })
    }
}
```

### 7.4 Custom Tool Declaration in Agent Forms

All three mechanisms are configured through the `[tools].custom` array:

```toml
[tools]
custom = [
    # Script tool
    { name = "lint_check", script = "scripts/lint.sh", description = "Run linter" },

    # MCP proxy tool (future)
    # { name = "github_pr", mcp_server = "http://localhost:8080", description = "Create GitHub PR" },
]
```

Rust plugin tools are registered programmatically at daemon startup, not through TOML.
This is intentional — Rust tools require compilation and cannot be hot-loaded from a
config file.

## 8. Integration with rig

### 8.1 rig's Tool Trait

rig-core 0.36 defines the `Tool` trait (`rig::tool::Tool`):

```rust
pub trait Tool: Sized + Send + Sync {
    type Error: Error + Send + Sync + 'static;
    type Args: for<'a> Deserialize<'a> + Send + Sync;
    type Output: Serialize;

    const NAME: &'static str;

    fn definition(&self, prompt: String)
        -> impl Future<Output = ToolDefinition> + Send + Sync;

    fn call(&self, args: Self::Args)
        -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;

    fn name(&self) -> String { Self::NAME.to_string() }
}
```

Key differences from nous's `AgentTool`:

| Aspect | rig `Tool` | nous `AgentTool` |
|--------|-----------|-----------------|
| Args type | Generic associated type (typed) | `serde_json::Value` (untyped) |
| Output type | Generic associated type (typed) | `ToolOutput` (structured enum) |
| Context | None — no execution context | `ToolContext` with permissions |
| Definition | Returns `ToolDefinition` (name + description + schema) | Returns `ToolMetadata` (richer) |
| Registration | `AgentBuilder::tool(impl Tool)` | `ToolRegistry::register()` |

### 8.2 Bridge Adapter: `NousToolAdapter`

To register nous tools with rig agents, we need an adapter that implements rig's `Tool`
trait by delegating to `AgentToolDyn`:

```rust
// crates/nous-core/src/tools/rig_bridge.rs

use rig::tool::{Tool, ToolDefinition};
use serde::{Deserialize, Serialize};

use super::{AgentToolDyn, ToolContext, ToolOutput, ToolError};

/// Adapts a nous AgentTool to rig's Tool trait.
pub struct NousToolAdapter {
    inner: Arc<dyn AgentToolDyn>,
    ctx: ToolContext,
}

impl NousToolAdapter {
    pub fn new(tool: Arc<dyn AgentToolDyn>, ctx: ToolContext) -> Self {
        Self { inner: tool, ctx }
    }
}

#[derive(Debug, Deserialize)]
pub struct GenericArgs {
    #[serde(flatten)]
    pub args: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct GenericOutput {
    pub content: String,
    pub is_error: bool,
}

impl std::fmt::Display for GenericOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct AdapterError {
    pub message: String,
}

impl Tool for NousToolAdapter {
    type Error = AdapterError;
    type Args = GenericArgs;
    type Output = GenericOutput;

    const NAME: &'static str = "nous_tool";

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let meta = self.inner.metadata_dyn();
        ToolDefinition {
            name: meta.name.clone(),
            description: meta.description.clone(),
            parameters: meta.input_schema.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        match self.inner.call_dyn(args.args, &self.ctx).await {
            Ok(output) => {
                let text = output.content.iter()
                    .map(|c| match c {
                        ToolContent::Text { text } => text.clone(),
                        ToolContent::Json { data } => serde_json::to_string_pretty(data)
                            .unwrap_or_default(),
                        ToolContent::Error { message } => format!("Error: {message}"),
                        ToolContent::Binary { mime_type, .. } =>
                            format!("[binary: {mime_type}]"),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(GenericOutput { content: text, is_error: false })
            }
            Err(e) => Ok(GenericOutput {
                content: e.to_string(),
                is_error: true,
            }),
        }
    }
}
```

### 8.3 Building a rig Agent with nous Tools

The agent spawn path in `process_manager.rs` builds a rig `Agent` with tools:

```rust
use rig::client::CompletionClient;

async fn build_agent_with_tools(
    client: &impl CompletionClient,
    model: &str,
    preamble: &str,
    tools: Vec<Arc<dyn AgentToolDyn>>,
    ctx: ToolContext,
) -> Agent<impl CompletionModel> {
    let mut builder = client.agent(model);

    if !preamble.is_empty() {
        builder = builder.preamble(preamble);
    }

    for tool in tools {
        let adapter = NousToolAdapter::new(tool, ctx.clone());
        builder = builder.tool(adapter);
    }

    builder.build()
}
```

### 8.4 ToolEmbedding for Semantic Tool Selection

rig's `ToolEmbedding` trait enables RAG-based tool discovery — the LLM doesn't need to
see all tool definitions; instead, semantically relevant tools are retrieved based on
the prompt:

```rust
pub trait ToolEmbedding: Tool {
    type InitError: Error + Send + Sync + 'static;
    type Context: for<'a> Deserialize<'a> + Serialize;
    type State: Send;

    fn embedding_docs(&self) -> Vec<String>;
    fn context(&self) -> Self::Context;
    fn init(state: Self::State, context: Self::Context) -> Result<Self, Self::InitError>;
}
```

For nous tools, the `embedding_docs()` returns the tool description plus example usage
patterns, enabling the vector store to match tools semantically:

```rust
impl ToolEmbedding for NousToolAdapter {
    type InitError = AdapterError;
    type Context = ToolMetadata;
    type State = (Arc<dyn AgentToolDyn>, ToolContext);

    fn embedding_docs(&self) -> Vec<String> {
        let meta = self.inner.metadata_dyn();
        vec![
            format!("{}: {}", meta.name, meta.description),
            format!("Category: {:?}. Tags: {}", meta.category, meta.tags.join(", ")),
        ]
    }

    fn context(&self) -> Self::Context {
        self.inner.metadata_dyn().clone()
    }

    fn init(
        state: Self::State,
        _context: Self::Context,
    ) -> Result<Self, Self::InitError> {
        Ok(Self::new(state.0, state.1))
    }
}
```

This integrates with the existing sqlite-vec embedding infrastructure
(`crates/nous-core/src/memory/embed.rs`) — tool embedding documents can be stored
alongside memory embeddings for unified semantic retrieval.

### 8.5 Agent Builder Integration Pattern

The full flow for spawning a tool-equipped agent:

```rust
// In ProcessRegistry::invoke_claude (process_manager.rs)

// 1. Resolve agent definition
let agent_def = load_definition(&agent_form_path)?;

// 2. Resolve available tools
let tools = resolve_agent_tools(&state.tool_registry, &agent_def, &agent_type).await;

// 3. Build tool context from agent metadata
let ctx = ToolContext {
    agent_id: agent.id.clone(),
    agent_name: agent.name.clone(),
    namespace: agent.namespace.clone(),
    workspace_dir: agent_def.process.as_ref()
        .and_then(|p| p.working_dir.as_deref())
        .map(PathBuf::from),
    session_id: Some(invocation.id.clone()),
    timeout: Duration::from_secs(
        agent_def.tools.as_ref()
            .and_then(|t| t.execution.as_ref())
            .and_then(|e| e.default_timeout_secs)
            .unwrap_or(30)
    ),
    permissions: resolve_permissions(&agent_def),
};

// 4. Build rig agent with tools
let rig_agent = build_agent_with_tools(
    client.as_ref(), &model, &preamble, tools, ctx
).await;

// 5. Execute prompt
let output = rig_agent.prompt(prompt).await
    .map_err(|e| NousError::Internal(e.to_string()))?;
```

## 9. Migration Plan

Implementation is split into four phases. Each phase produces a compiling, testable
milestone. No phase depends on backwards compatibility — prototype phase allows breaking
changes.

### Phase 1: Core Abstractions (1-2 weeks)

**Goal:** Establish the tool trait, registry, and execution engine in `nous-core`.

**Files created:**

| File | Contents |
|------|----------|
| `crates/nous-core/src/tools/mod.rs` | `AgentTool` trait, `AgentToolDyn`, `ToolMetadata`, `ToolCategory`, `ToolOutput`, `ToolContent`, `ToolError`, `ToolPermissions`, `ExecutionPolicy`, `ToolContext` |
| `crates/nous-core/src/tools/registry.rs` | `ToolRegistry` with register/get/list/deregister |
| `crates/nous-core/src/tools/execution.rs` | `ToolExecutor` with validate → authorize → execute → capture → record pipeline |
| `crates/nous-core/src/tools/permissions.rs` | `ResolvedPermissions`, authorization logic |

**Files modified:**

| File | Change |
|------|--------|
| `crates/nous-core/src/lib.rs` | Add `pub mod tools;` |
| `crates/nous-core/Cargo.toml` | No new deps needed (serde, serde_json, tokio, thiserror already in workspace) |

**Tests:** Unit tests for registry CRUD, permission checks, execution timeout, output
truncation.

**Commit:** `feat: add tool trait, registry, and execution engine (NOUS-061 phase 1)`

### Phase 2: Built-in Tools (1-2 weeks)

**Goal:** Implement the default tool set (Section 3) as `AgentTool` implementations.

**Files created:**

| File | Tools | Count |
|------|-------|-------|
| `crates/nous-core/src/tools/builtin/mod.rs` | Re-exports, `register_builtin_tools()` | — |
| `crates/nous-core/src/tools/builtin/filesystem.rs` | `FsReadTool`, `FsWriteTool`, `FsEditTool`, `FsListTool`, `FsSearchTool`, `FsStatTool`, `FsMkdirTool`, `FsDeleteTool` | 8 |
| `crates/nous-core/src/tools/builtin/shell.rs` | `ShellExecTool`, `ShellExecBackgroundTool`, `ShellReadOutputTool`, `ShellKillTool` | 4 |
| `crates/nous-core/src/tools/builtin/http.rs` | `HttpRequestTool`, `HttpFetchTool` | 2 |
| `crates/nous-core/src/tools/builtin/memory.rs` | `MemorySaveTool`, `MemorySearchTool`, `MemorySearchHybridTool`, `MemoryGetContextTool`, `MemoryRelateTool` | 5 |
| `crates/nous-core/src/tools/builtin/comms.rs` | `RoomPostTool`, `RoomReadTool`, `RoomCreateTool`, `RoomWaitTool`, `TaskCreateTool`, `TaskUpdateTool` | 6 |
| `crates/nous-core/src/tools/builtin/code.rs` | `CodeGrepTool`, `CodeGlobTool`, `CodeSymbolsTool` | 3 |

**Files modified:**

| File | Change |
|------|--------|
| `crates/nous-core/Cargo.toml` | Add `reqwest` (workspace) for HTTP tools |

**Tests:** Integration tests for each tool with mock filesystem and database fixtures.

**Commit:** `feat: implement built-in tool set (NOUS-061 phase 2)`

### Phase 3: Agent Form Integration (1 week)

**Goal:** Add `[tools]` section to agent form TOML and wire it to tool resolution.

**Files modified:**

| File | Change |
|------|--------|
| `crates/nous-core/src/agents/definition.rs` | Add `ToolsSection`, `CustomToolDef`, `ToolPermissionsConfig`, `ExecutionConfig` structs. Add `tools: Option<ToolsSection>` to `AgentDefinition`. |
| `crates/nous-core/src/tools/mod.rs` | Add `resolve_agent_tools()`, `default_tools_for_type()` |
| `crates/nous-core/src/tools/custom/mod.rs` | `ScriptTool` for script-based custom tools |
| `crates/nous-core/src/tools/custom/script_tool.rs` | Script tool implementation |

**Tests:** TOML parsing tests for full and minimal tool sections. Resolution tests for
allow/deny logic. Script tool execution tests.

**Commit:** `feat: add [tools] section to agent forms (NOUS-061 phase 3)`

### Phase 4: rig Bridge + Daemon Wiring (1-2 weeks)

**Goal:** Bridge nous tools to rig agents. Wire `ToolRegistry` into `AppState` and
`ProcessRegistry`.

**Files created:**

| File | Contents |
|------|----------|
| `crates/nous-core/src/tools/rig_bridge.rs` | `NousToolAdapter` implementing rig `Tool` + `ToolEmbedding` |

**Files modified:**

| File | Change |
|------|--------|
| `crates/nous-daemon/src/state.rs` | Add `tool_registry: Arc<ToolRegistry>` to `AppState` |
| `crates/nous-daemon/src/main.rs` | Initialize `ToolRegistry`, call `register_builtin_tools()` |
| `crates/nous-daemon/src/process_manager.rs` | Update `invoke_claude()` to build agents with tools via `NousToolAdapter` |
| `crates/nous-daemon/src/routes/mcp.rs` | Replace static `get_tool_schemas()` with `tool_registry.list()` |
| `crates/nous-daemon/Cargo.toml` | Already has `rig-core` — no new dep needed |

**Tests:** End-to-end test: define agent form with `[tools]`, spawn via ProcessRegistry,
verify tools are available to rig agent.

**Commit:** `feat: bridge nous tools to rig agents (NOUS-061 phase 4)`

### Phase Summary

| Phase | Deliverable | Duration | Dependencies |
|-------|------------|----------|-------------|
| 1 | Core trait + registry + executor | 1-2 weeks | None |
| 2 | Built-in tool implementations | 1-2 weeks | Phase 1 |
| 3 | Agent form `[tools]` section | 1 week | Phase 1 |
| 4 | rig bridge + daemon wiring | 1-2 weeks | Phases 1-3, rig adoption (completed) |

Total estimated effort: 4-7 weeks.

## 10. Open Decisions

### 10.1 Typed vs Untyped Tool Arguments

**Question:** Should `AgentTool::call()` accept `serde_json::Value` (untyped) or use
a generic associated type like rig's `Tool::Args` (typed)?

**Trade-offs:**

| Approach | Pros | Cons |
|----------|------|------|
| `Value` (untyped) | Simple registry, easy dynamic dispatch, one `HashMap<String, DynTool>` | No compile-time validation, runtime deserialization errors |
| Associated type (typed) | Compile-time safety, better IDE support | Cannot store heterogeneous tools in one collection without type erasure |

**Recommendation:** Start with `Value` for the registry and dynamic dispatch layer.
Individual tool implementations validate args internally. This matches the MCP protocol
(JSON in, JSON out) and avoids the complexity of rig's associated-type pattern for the
registry. The rig bridge adapter (`NousToolAdapter`) handles the type conversion at
the boundary.

### 10.2 Tool Invocation Telemetry

**Question:** Should tool invocations be recorded in the database for analytics and
procedural memory?

**Options:**

1. **Extend `agent_invocations`** table with tool-specific fields (tool_name, duration,
   success/failure).
2. **New `tool_invocations`** table for fine-grained tool telemetry.
3. **Memory-based:** Auto-create `MemoryType::Observation` entries for tool usage
   patterns (ties into `agent-memory.md` procedural memory gap).

**Recommendation:** Option 2 (new table) for analytics, plus Option 3 for agent-visible
procedural memory. The invocations table supports dashboards and debugging; the memory
entries enable agents to learn which tools work best for specific tasks.

### 10.3 Hot-Loading Custom Tools

**Question:** Can custom tools (script or plugin) be added/removed without restarting
the daemon?

**Current answer:** The `ToolRegistry` supports `register()` and `deregister()` at
runtime, so hot-loading is architecturally possible. However, the TOML-based agent
form system loads tool definitions at agent spawn time, not at daemon startup. A
runtime API for tool management (e.g., `POST /tools/register`) is deferred.

### 10.4 Tool Composition

**Question:** Should tools be composable — can one tool invoke another?

**Concern:** Tool composition creates re-entrancy risks (tool A calls tool B which
calls tool A). It also complicates permission checking (does tool A inherit the caller's
permissions or use its own?).

**Recommendation:** Defer tool composition. Tools execute atomically — they do not
invoke other tools. If an agent needs a multi-step workflow, the LLM orchestrates it
by calling tools sequentially. This matches how Claude Code, OpenAI Assistants, and
LangChain operate — the LLM is the orchestrator, not the tools.

### 10.5 MCP Protocol Compatibility

**Question:** Should the nous tool system maintain MCP protocol compatibility for the
existing `/mcp/tools` and `/mcp/call` endpoints?

**Recommendation:** Yes. The existing MCP HTTP interface is the primary way external
agents (Claude Code, etc.) interact with nous. The `ToolRegistry` should power the MCP
endpoints, replacing the static `get_tool_schemas()` in `mcp.rs`. The `ToolSchema`
struct from mcp.rs (`name`, `description`, `input_schema`) maps directly to
`ToolMetadata` with field selection.

### 10.6 Confirmation Flow for High-Risk Tools

**Question:** How are `requires_confirmation: true` tools handled when the agent runs
unattended?

**Options:**

1. **Fail closed:** High-risk tools return `PermissionDenied` in unattended mode.
2. **Auto-approve in sandbox:** If the agent runs in a sandbox, high-risk tools
   auto-approve since the blast radius is contained.
3. **Escalate:** Post a confirmation request to the coordination room and block until
   a manager approves.

**Recommendation:** Option 2 for sandboxed agents (consistent with how Claude Code
handles sandboxed shell commands). Option 1 for non-sandboxed agents in prototype phase.
Option 3 (human-in-the-loop approval) is a future enhancement.

### 10.7 Tool Versioning

**Question:** Should tools have semantic versions? How are version conflicts handled
when an agent form requests a specific tool version?

**Recommendation:** Include `version` in `ToolMetadata` but do not enforce version
matching in prototype phase. Tools are singleton instances in the registry. Version
conflicts become relevant when custom tools from external sources are supported — defer
version resolution until then.

### 10.8 Agent Skills Specification Alignment

**Question:** How closely should nous tools align with the Agent Skills Specification?

The Agent Skills Spec defines skills as folders with `SKILL.md` files containing
metadata frontmatter and markdown instructions. Nous already uses a similar pattern
for its skill system (`[skills].refs` loading markdown files). Tools are distinct from
skills — a skill may reference multiple tools, but tools are the atomic execution units.

**Recommendation:** Align with Agent Skills Spec for the skill layer (already done).
For tools, adopt the Spec's progressive disclosure pattern (Section 6.5) and
`allowed-tools` frontmatter field. Do not conflate skills and tools — a skill is
knowledge (instructions + references), a tool is capability (executable action).
