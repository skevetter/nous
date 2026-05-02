# Nous Examples

This directory contains runnable examples demonstrating common workflows with the `nous` CLI.

## Examples

| File | Description |
|------|-------------|
| [01-getting-started.sh](01-getting-started.sh) | Build, verify setup, start daemon, and run a basic workflow |
| [02-agent-lifecycle.sh](02-agent-lifecycle.sh) | Register agents in a hierarchy, inspect, update, and deregister |
| [03-invoke-agent.sh](03-invoke-agent.sh) | Spawn processes, invoke agents (sync/async), view results and logs |
| [04-configuration.sh](04-configuration.sh) | Config file format, env vars, CLI flags, LLM provider setup |
| [05-agent-definitions.sh](05-agent-definitions.sh) | Templates, instantiation, version tracking, rollback, upgrades |
| [06-sandbox-agents.sh](06-sandbox-agents.sh) | Isolated agents with restricted dirs, timeouts, restart policies |
| [07-daemon-management.sh](07-daemon-management.sh) | Start/stop daemon, port config, PID/log files, MCP server mode |
| [sample-agent.toml](sample-agent.toml) | Annotated agent definition showing the full data model |

## Prerequisites

- Rust toolchain (to build from source)
- AWS credentials (for LLM-powered agent invocation via Bedrock)
- `jq` installed (used in examples to parse JSON output)

## Quick Start

```bash
# Build
cargo build --release

# Verify setup
nous doctor

# Start the daemon
nous start

# Run an example
bash examples/01-getting-started.sh
```

## Architecture

```
nous start          -->  HTTP daemon (port 8377)
nous agent ...      -->  Direct DB or daemon API calls
nous mcp-server     -->  Stdio MCP transport (for AI tool integration)
```

## Key Concepts

- **Agents** are registered in a hierarchy (director > senior-manager > manager > engineer)
- **Namespaces** provide multi-tenant isolation
- **Templates** define reusable agent configurations
- **Processes** are runtime instances managed by the daemon
- **Invocations** send work prompts to agents and track results
- **Versions** snapshot an agent's skill and config state for rollback
