#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 02 - Agent Lifecycle
# =============================================================================
#
# User Story:
#   As a platform operator, I want to create agents in a hierarchy, inspect
#   their state, update their configuration, and remove them when done.
#
# This example covers:
#   - Registering agents of different types
#   - Building parent-child hierarchies
#   - Inspecting agents
#   - Updating agent status and configuration
#   - Deregistering agents (with and without cascade)
#
# Prerequisites:
#   - nous daemon running (`nous start`)
#
# =============================================================================

echo "=== Register a director (top-level) ==="
DIRECTOR=$(nous agent register --name "project-director" --type director)
DIRECTOR_ID=$(echo "$DIRECTOR" | jq -r '.id')
echo "Director ID: $DIRECTOR_ID"

echo ""
echo "=== Register a manager under the director ==="
MANAGER=$(nous agent register \
  --name "backend-manager" \
  --type manager \
  --parent "$DIRECTOR_ID")
MANAGER_ID=$(echo "$MANAGER" | jq -r '.id')
echo "Manager ID: $MANAGER_ID"

echo ""
echo "=== Register engineers under the manager ==="
ENG1=$(nous agent register \
  --name "api-engineer" \
  --type engineer \
  --parent "$MANAGER_ID" \
  --room "backend-room" \
  --metadata '{"speciality": "REST APIs"}')
ENG1_ID=$(echo "$ENG1" | jq -r '.id')

ENG2=$(nous agent register \
  --name "db-engineer" \
  --type engineer \
  --parent "$MANAGER_ID" \
  --room "backend-room" \
  --metadata '{"speciality": "database migrations"}')
ENG2_ID=$(echo "$ENG2" | jq -r '.id')

echo "Engineer IDs: $ENG1_ID, $ENG2_ID"

echo ""
echo "=== View the agent tree ==="
nous agent tree
# Expected output: hierarchical JSON showing director -> manager -> engineers

echo ""
echo "=== List children of the manager ==="
nous agent children "$MANAGER_ID"

echo ""
echo "=== List ancestors of an engineer ==="
nous agent ancestors "$ENG1_ID"
# Shows: [director, manager]

echo ""
echo "=== Inspect an agent in detail ==="
nous agent inspect "$ENG1_ID"
# Shows: agent details, current version, template, process status, invocations

echo ""
echo "=== Update agent configuration ==="
nous agent update "$ENG1_ID" \
  --process-type claude \
  --spawn-command "claude --dangerously-skip-permissions" \
  --working-dir "/workspace/api" \
  --auto-restart true

echo ""
echo "=== Update agent status ==="
nous agent status "$ENG1_ID" idle
# Valid statuses: active, inactive, archived, running, idle, blocked, done

echo ""
echo "=== Send a heartbeat ==="
nous agent heartbeat "$ENG1_ID" --status running
# Updates last_seen_at and sets status to running

echo ""
echo "=== Search agents by name ==="
nous agent search "engineer"
# Full-text search across agent names and metadata

echo ""
echo "=== List agents filtered by type ==="
nous agent list --type engineer
nous agent list --status active --limit 10

echo ""
echo "=== Check for stale agents (no heartbeat in 15 minutes) ==="
nous agent stale --threshold 900

echo ""
echo "=== Deregister a single agent (no children) ==="
nous agent deregister "$ENG2_ID"
# Expected: {"result": "deleted"}

echo ""
echo "=== Deregister with cascade (removes children too) ==="
nous agent deregister "$DIRECTOR_ID" --cascade
# Expected: {"result": "cascaded"}
# This removes the director, manager, and remaining engineer

echo ""
echo "=== Verify cleanup ==="
nous agent list
# Should show no agents from this example

echo "Done! Agent lifecycle complete."
