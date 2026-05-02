#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 06 - Sandboxed Agents
# =============================================================================
#
# User Story:
#   As a security-conscious operator, I want to run agents in isolated
#   environments with restricted working directories and controlled
#   process lifecycles.
#
# This example covers:
#   - Spawning agents with restricted working directories
#   - Using different process types (shell, claude, http)
#   - Configuring restart policies
#   - Monitoring sandboxed agent health
#   - Timeout-based lifecycle management
#
# Prerequisites:
#   - nous daemon running (`nous start`)
#
# =============================================================================

echo "=== Setup: Create a sandbox workspace ==="
SANDBOX_DIR=$(mktemp -d /tmp/nous-sandbox-XXXXXX)
echo "Sandbox directory: $SANDBOX_DIR"

echo ""
echo "=== Register a sandboxed shell agent ==="
SHELL_AGENT=$(nous agent register \
  --name "sandbox-shell" \
  --type engineer \
  --metadata '{"sandbox": true, "restricted": true}')
SHELL_ID=$(echo "$SHELL_AGENT" | jq -r '.id')

# Configure with restricted working directory
nous agent update "$SHELL_ID" \
  --process-type shell \
  --spawn-command "bash -c 'echo ready'" \
  --working-dir "$SANDBOX_DIR" \
  --auto-restart false

echo "Shell agent ID: $SHELL_ID"

echo ""
echo "=== Register a sandboxed Claude agent ==="
CLAUDE_AGENT=$(nous agent register \
  --name "sandbox-claude" \
  --type engineer \
  --metadata '{"sandbox": true, "model": "claude"}')
CLAUDE_ID=$(echo "$CLAUDE_AGENT" | jq -r '.id')

# Configure Claude process type with isolated workspace
nous agent update "$CLAUDE_ID" \
  --process-type claude \
  --spawn-command "claude --dangerously-skip-permissions" \
  --working-dir "$SANDBOX_DIR" \
  --auto-restart true

echo "Claude agent ID: $CLAUDE_ID"

echo ""
echo "=== Spawn with timeout (auto-terminate) ==="
# Process will be killed after 300 seconds
nous agent spawn "$SHELL_ID" \
  --command "bash -c 'while true; do echo heartbeat; sleep 10; done'" \
  --type shell \
  --working-dir "$SANDBOX_DIR" \
  --timeout 300 \
  --restart never
# Timeout ensures the process cannot run indefinitely

echo ""
echo "=== Spawn with restart policy ==="
# on-failure: restart only if exit code != 0
nous agent spawn "$SHELL_ID" \
  --command "bash -c 'echo working && exit 0'" \
  --type shell \
  --working-dir "$SANDBOX_DIR" \
  --restart on-failure

# always: restart regardless of exit code (use with caution)
# nous agent spawn "$SHELL_ID" --command "..." --restart always

echo ""
echo "=== Monitor sandboxed agents ==="
# List all running processes
nous agent ps

# Check specific agent status
nous agent inspect "$SHELL_ID"

echo ""
echo "=== Heartbeat monitoring ==="
# Agents should send heartbeats; stale detection finds unresponsive ones
nous agent heartbeat "$SHELL_ID" --status running
nous agent heartbeat "$CLAUDE_ID" --status running

# After some time, check for stale agents (threshold in seconds)
nous agent stale --threshold 60

echo ""
echo "=== Invoke sandboxed agent with work ==="
# Send a prompt to the sandboxed agent
nous agent invoke "$SHELL_ID" \
  --prompt "Create a file called output.txt with 'hello world'" \
  --timeout 30

echo ""
echo "=== View process logs ==="
nous agent logs "$SHELL_ID" --lines 5
nous agent logs "$CLAUDE_ID" --lines 5

echo ""
echo "=== Stop sandboxed agents ==="
# Graceful stop
nous agent stop "$SHELL_ID"
nous agent stop "$CLAUDE_ID"

# Verify stopped
nous agent ps

echo ""
echo "=== Cleanup ==="
nous agent deregister "$SHELL_ID"
nous agent deregister "$CLAUDE_ID"
rm -rf "$SANDBOX_DIR"

echo "Done! Sandbox agents example complete."
