#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 03 - Invoking Agents
# =============================================================================
#
# User Story:
#   As a developer, I want to send work to agents, run them synchronously or
#   asynchronously, and retrieve their results.
#
# This example covers:
#   - Spawning agent processes (shell, claude)
#   - Invoking agents with prompts (sync and async)
#   - Retrieving async invocation results
#   - Viewing invocation history
#   - Managing running processes (ps, stop, restart, logs)
#
# Prerequisites:
#   - nous daemon running (`nous start`)
#   - AWS credentials configured (for claude process type)
#
# =============================================================================

echo "=== Setup: Register an agent ==="
AGENT=$(nous agent register --name "worker-agent" --type engineer)
AGENT_ID=$(echo "$AGENT" | jq -r '.id')
echo "Agent ID: $AGENT_ID"

# Configure the agent with a spawn command
nous agent update "$AGENT_ID" \
  --process-type shell \
  --spawn-command "echo 'Hello from worker'" \
  --working-dir "/tmp"

echo ""
echo "=== Spawn an agent process ==="
# Spawn creates a process record for the agent
nous agent spawn "$AGENT_ID" --command "echo 'task running'" --type shell
# Expected output: JSON with process details (id, status, command, etc.)

echo ""
echo "=== Spawn with custom timeout ==="
nous agent spawn "$AGENT_ID" \
  --command "sleep 5 && echo done" \
  --type shell \
  --timeout 30 \
  --restart never

echo ""
echo "=== List all running processes ==="
nous agent ps
# Shows all active agent processes across all agents

echo ""
echo "=== Invoke an agent synchronously ==="
# The invoke command sends a prompt to the daemon, which dispatches work
# to the agent's configured process. This waits for the result.
nous agent invoke "$AGENT_ID" --prompt "List all files in the current directory"
# Expected output: JSON with invocation result
#   {
#     "id": "...",
#     "agent_id": "...",
#     "prompt": "List all files in the current directory",
#     "status": "completed",
#     "result": "...",
#     ...
#   }

echo ""
echo "=== Invoke an agent with a timeout ==="
nous agent invoke "$AGENT_ID" \
  --prompt "Run a long computation" \
  --timeout 60

echo ""
echo "=== Invoke an agent asynchronously ==="
# With --async, the command returns immediately with an invocation ID
INVOKE_RESULT=$(nous agent invoke "$AGENT_ID" \
  --prompt "Refactor the authentication module" \
  --async)
INVOCATION_ID=$(echo "$INVOKE_RESULT" | jq -r '.id')
echo "Invocation ID: $INVOCATION_ID"
echo "Status: $(echo "$INVOKE_RESULT" | jq -r '.status')"
# Status will be "pending" or "running"

echo ""
echo "=== Poll for async result ==="
# Check the result of an async invocation
sleep 2
nous agent invoke-result "$INVOCATION_ID"
# Returns the invocation with updated status and result

echo ""
echo "=== View invocation history ==="
nous agent invocations "$AGENT_ID"
# Lists recent invocations for this agent

# Filter by status
nous agent invocations "$AGENT_ID" --status completed --limit 5

echo ""
echo "=== View agent process logs ==="
nous agent logs "$AGENT_ID" --lines 10
# Shows recent process records for the agent

echo ""
echo "=== Stop a running agent process ==="
nous agent stop "$AGENT_ID"
# Gracefully stops the active process

# Force stop (immediate SIGKILL after grace period)
# nous agent stop "$AGENT_ID" --force --grace 5

echo ""
echo "=== Restart an agent process ==="
nous agent restart "$AGENT_ID"
# Stops existing process and starts a new one using the agent's configured command

# Restart with a different command
nous agent restart "$AGENT_ID" --command "echo 'new task'"

echo ""
echo "=== Cleanup ==="
nous agent stop "$AGENT_ID" 2>/dev/null || true
nous agent deregister "$AGENT_ID"

echo "Done! Agent invocation example complete."
