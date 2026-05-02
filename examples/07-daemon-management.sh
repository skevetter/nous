#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 07 - Daemon Management
# =============================================================================
#
# User Story:
#   As a platform operator, I want to manage the nous daemon lifecycle
#   including starting, stopping, monitoring, and configuring the HTTP server.
#
# This example covers:
#   - Starting the daemon (foreground and background)
#   - Stopping the daemon gracefully
#   - Port configuration
#   - PID file management
#   - Log file location
#   - Health checking
#
# =============================================================================

echo "=== Daemon Overview ==="
# The nous daemon is an HTTP server that:
#   - Hosts the MCP (Model Context Protocol) API
#   - Manages agent processes
#   - Runs the scheduler for cron-like tasks
#   - Provides the agent invocation runtime
#
# Default address: 127.0.0.1:8377
# PID file:        ~/.config/nous/nous.pid
# Log file:        ~/.config/nous/nous-daemon.log

echo ""
echo "=== Method 1: Start as background daemon ==="
# The `start` command is an alias for `serve --daemon`
nous start
# Expected output:
#   nous daemon started (pid: 12345)
#   PID file: /home/user/.config/nous/nous.pid
#   Log file: /home/user/.config/nous/nous-daemon.log

# Wait for daemon to be ready
sleep 2

echo ""
echo "=== Health check ==="
# The daemon exposes an HTTP endpoint; verify it responds
curl -s http://127.0.0.1:8377/health || echo "(health endpoint may not exist yet)"

# Alternative: use nous doctor to check daemon connectivity
nous doctor

echo ""
echo "=== Stop the daemon ==="
nous stop
# Expected output:
#   daemon stopped
#
# If daemon is not running:
#   daemon not running (no PID file)
#
# If PID file is stale:
#   daemon not running (stale PID file removed)

echo ""
echo "=== Method 2: Start on a custom port ==="
nous start --port 9000
sleep 2

# All commands must use the same port override
nous --port 9000 agent list
nous --port 9000 doctor

nous stop
sleep 1

echo ""
echo "=== Method 3: Start in foreground (for development) ==="
# Foreground mode is useful for debugging - logs go to stdout
# Press Ctrl+C to stop
#
# nous serve
#
# With verbose logging:
# RUST_LOG=debug nous serve

echo ""
echo "=== Method 4: Start with LLM configuration ==="
# Configure the LLM provider at daemon startup
nous start \
  --model "anthropic.claude-sonnet-4-20250514-v1:0" \
  --region "us-west-2" \
  --profile "my-aws-profile"
sleep 2
nous stop
sleep 1

echo ""
echo "=== PID File Management ==="
# The PID file is automatically managed:
#   - Created when daemon starts
#   - Removed when daemon stops cleanly
#   - Detected as stale if process no longer exists
PID_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/nous/nous.pid"
echo "PID file location: $PID_FILE"

# Check if daemon is running by reading PID file
if [ -f "$PID_FILE" ]; then
  PID=$(cat "$PID_FILE")
  if kill -0 "$PID" 2>/dev/null; then
    echo "Daemon is running (PID: $PID)"
  else
    echo "Stale PID file detected (PID: $PID)"
    rm -f "$PID_FILE"
  fi
else
  echo "Daemon is not running (no PID file)"
fi

echo ""
echo "=== Log File ==="
LOG_FILE="${XDG_CONFIG_HOME:-$HOME/.config}/nous/nous-daemon.log"
echo "Log file location: $LOG_FILE"

# View recent log entries
if [ -f "$LOG_FILE" ]; then
  echo "Last 10 lines:"
  tail -n 10 "$LOG_FILE"
fi

echo ""
echo "=== MCP Server Mode (stdio transport) ==="
# For integration with AI tools (e.g., Claude Code), nous can run as an
# MCP server using stdio transport instead of HTTP:
#
# nous mcp-server
#
# With specific tool prefixes:
# nous mcp-server --tools "chat,task,agent"
#
# With LLM config:
# nous mcp-server --model "anthropic.claude-sonnet-4-20250514-v1:0" --region "us-west-2"

echo ""
echo "=== Graceful Shutdown Behavior ==="
# When stopped (via `nous stop` or SIGTERM):
#   1. Daemon receives SIGTERM
#   2. Stops accepting new connections
#   3. Waits for in-flight requests to complete
#   4. Shuts down the process registry (stops managed processes)
#   5. Closes database connections
#   6. Removes PID file
#   7. Exits with code 0
#
# If daemon does not stop within 5 seconds, `nous stop` reports a timeout.

echo ""
echo "Done! Daemon management overview complete."
