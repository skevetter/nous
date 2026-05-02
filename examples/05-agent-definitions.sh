#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 05 - Agent Definitions and Templates
# =============================================================================
#
# User Story:
#   As a team lead, I want to define reusable agent templates so I can
#   quickly instantiate pre-configured agents for common roles.
#
# This example covers:
#   - Creating agent templates
#   - Listing and inspecting templates
#   - Instantiating agents from templates
#   - Version tracking and rollback
#   - Upgrade notifications
#
# Prerequisites:
#   - nous daemon running (`nous start`)
#
# =============================================================================

echo "=== Create an agent template ==="
# Templates define reusable configurations for agent roles.
# They are immutable once created.

TEMPLATE=$(nous agent template create \
  --name "code-reviewer" \
  --type engineer \
  --config '{"process_type": "claude", "spawn_command": "claude --dangerously-skip-permissions", "working_dir": "/workspace", "auto_restart": true, "review_style": "thorough"}' \
  --skills '[{"name": "code-review", "path": "/skills/code-review.md"}]')
TEMPLATE_ID=$(echo "$TEMPLATE" | jq -r '.id')
echo "Template ID: $TEMPLATE_ID"
echo "$TEMPLATE" | jq .

echo ""
echo "=== Create another template (monitoring agent) ==="
MONITOR_TPL=$(nous agent template create \
  --name "health-monitor" \
  --type engineer \
  --config '{"process_type": "shell", "spawn_command": "bash /scripts/health-check.sh", "auto_restart": true}' \
  --skills '[{"name": "monitoring", "path": "/skills/monitoring.md"}]')
MONITOR_TPL_ID=$(echo "$MONITOR_TPL" | jq -r '.id')
echo "Monitor Template ID: $MONITOR_TPL_ID"

echo ""
echo "=== List all templates ==="
nous agent template list
# Shows all registered templates

# Filter by type
nous agent template list --type engineer

echo ""
echo "=== Get a template by ID ==="
nous agent template get "$TEMPLATE_ID"

echo ""
echo "=== Instantiate an agent from a template ==="
# This creates a new agent with the template's configuration pre-applied
REVIEWER=$(nous agent template instantiate "$TEMPLATE_ID" \
  --name "pr-reviewer-1" \
  --namespace "default")
REVIEWER_ID=$(echo "$REVIEWER" | jq -r '.id')
echo "Reviewer Agent ID: $REVIEWER_ID"
echo "$REVIEWER" | jq .

echo ""
echo "=== Instantiate with config overrides ==="
# Override specific template config values
REVIEWER2=$(nous agent template instantiate "$TEMPLATE_ID" \
  --name "pr-reviewer-2" \
  --config-overrides '{"review_style": "quick", "working_dir": "/workspace/frontend"}')
REVIEWER2_ID=$(echo "$REVIEWER2" | jq -r '.id')
echo "Reviewer 2 Agent ID: $REVIEWER2_ID"

echo ""
echo "=== Instantiate under a parent ==="
# Create a manager first
MGR=$(nous agent register --name "review-lead" --type manager)
MGR_ID=$(echo "$MGR" | jq -r '.id')

# Instantiate reviewers under the manager
nous agent template instantiate "$TEMPLATE_ID" \
  --name "pr-reviewer-3" \
  --parent "$MGR_ID"

echo ""
echo "=== Version Tracking ==="
# Record a version (captures current skill and config state)
VERSION=$(nous agent record-version \
  --agent-id "$REVIEWER_ID" \
  --skill-hash "abc123def456" \
  --config-hash "789xyz000" \
  --skills-json '[{"name": "code-review", "path": "/skills/code-review.md", "hash": "abc123"}]')
VERSION_ID=$(echo "$VERSION" | jq -r '.id')
echo "Version ID: $VERSION_ID"

echo ""
echo "=== List version history ==="
nous agent versions "$REVIEWER_ID" --limit 10

echo ""
echo "=== Record a new version (simulating an upgrade) ==="
VERSION2=$(nous agent record-version \
  --agent-id "$REVIEWER_ID" \
  --skill-hash "newskill789" \
  --config-hash "newconfig456" \
  --skills-json '[{"name": "code-review", "path": "/skills/code-review-v2.md", "hash": "newskill789"}]')
VERSION2_ID=$(echo "$VERSION2" | jq -r '.id')

echo ""
echo "=== Rollback to a previous version ==="
nous agent rollback "$REVIEWER_ID" --version "$VERSION_ID"
echo "Rolled back to version: $VERSION_ID"

echo ""
echo "=== Upgrade notifications ==="
# Mark an agent as having an upgrade available
nous agent notify-upgrade "$REVIEWER_ID"

# List agents with pending upgrades
nous agent outdated
nous agent outdated --namespace default --limit 10

echo ""
echo "=== Inspect agent (shows version + template info) ==="
nous agent inspect "$REVIEWER_ID"
# Output includes:
#   - current_version (hash info)
#   - template (reference to code-reviewer template)
#   - version_count
#   - active_process
#   - recent_invocations

echo ""
echo "=== Cleanup ==="
nous agent deregister "$MGR_ID" --cascade
nous agent deregister "$REVIEWER_ID" 2>/dev/null || true
nous agent deregister "$REVIEWER2_ID" 2>/dev/null || true

echo "Done! Agent definitions and templates example complete."
