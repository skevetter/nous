#!/usr/bin/env bash
set -euo pipefail

# ============================================================
# NOUS PLATFORM — End-to-End Demo
# ============================================================
# This script exercises all 9 platform features (Doctor + P0–P7)
# using the CLI with a temporary isolated data directory.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NOUS="$SCRIPT_DIR/target/release/nous-cli"

# Create isolated temp environment
DEMO_TMP=$(mktemp -d /tmp/nous-demo-XXXXXX)
export XDG_CONFIG_HOME="$DEMO_TMP/config"
export XDG_DATA_HOME="$DEMO_TMP/data"
mkdir -p "$XDG_CONFIG_HOME/nous" "$XDG_DATA_HOME/nous"

# Write a config pointing to our temp data dir
cat > "$XDG_CONFIG_HOME/nous/config.toml" <<EOF
data_dir = "$XDG_DATA_HOME/nous"
host = "127.0.0.1"
port = 18377
EOF

pause() { sleep 0.3; }

echo ""
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║          NOUS PLATFORM — End-to-End Demo                    ║"
echo "║          Data dir: $XDG_DATA_HOME/nous"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""
pause

# ============================================================
echo "=== 1. DOCTOR — Validate Setup ==="
echo ""
$NOUS doctor
pause
echo ""

# ============================================================
echo "=== 2. CHAT (P0) — Rooms & Messages ==="
echo ""

echo "--- Creating a chat room ---"
$NOUS chat create --name "demo-room" --purpose "End-to-end demo chat"
pause

echo ""
echo "--- Posting messages ---"
$NOUS chat post demo-room --sender "alice" --content "Hello from the demo!"
$NOUS chat post demo-room --sender "bob" --content "Nous platform is live!"
$NOUS chat post demo-room --sender "alice" --content "Full-text search works great"
pause

echo ""
echo "--- Reading messages ---"
$NOUS chat read demo-room --limit 5
pause

echo ""
echo "--- Searching messages (FTS5) ---"
$NOUS chat search "platform"
pause

echo ""
echo "--- Listing rooms ---"
$NOUS chat list
pause
echo ""

# ============================================================
echo "=== 3. TASKS (P1) — Issues & Tracking ==="
echo ""

echo "--- Creating tasks ---"
TASK1=$($NOUS task create "Implement auth module" --description "OAuth2 + OIDC integration" --priority high --label backend --label security --create-room | jq -r '.id')
echo "Task 1 ID: $TASK1"
TASK2=$($NOUS task create "Write auth tests" --description "Unit and integration tests" --priority medium --label testing | jq -r '.id')
echo "Task 2 ID: $TASK2"
pause

echo ""
echo "--- Linking tasks (blocks relationship) ---"
$NOUS task link "$TASK2" --blocks "$TASK1"
pause

echo ""
echo "--- Adding a note ---"
$NOUS task note "$TASK1" "Started design review with team"
pause

echo ""
echo "--- Updating task status ---"
$NOUS task update "$TASK1" --status in_progress --assignee "agent-007"
pause

echo ""
echo "--- Viewing task history ---"
$NOUS task history "$TASK1" --limit 5
pause

echo ""
echo "--- Searching tasks (FTS5) ---"
$NOUS task search "auth"
pause
echo ""

# ============================================================
echo "=== 4. WORKTREES (P2) — Git Worktree Management ==="
echo ""

# Create a temporary git repo for the worktree demo
DEMO_REPO="$DEMO_TMP/repo"
git init "$DEMO_REPO" --quiet
git -C "$DEMO_REPO" commit --allow-empty -m "init" --quiet

echo "--- Creating a worktree ---"
WT=$($NOUS worktree create --branch feat/demo-feature --slug demo-wt --repo-root "$DEMO_REPO" | jq -r '.id')
echo "Worktree ID: $WT"
pause

echo ""
echo "--- Listing worktrees ---"
$NOUS worktree list
pause

echo ""
echo "--- Showing worktree details ---"
$NOUS worktree show "$WT"
pause

echo ""
echo "--- Archiving worktree ---"
$NOUS worktree archive "$WT"
pause
echo ""

# ============================================================
echo "=== 5. ORG (P3) — Agent Hierarchy ==="
echo ""

echo "--- Registering agents ---"
DIR=$($NOUS agent register --name "ceo-agent" --type director | jq -r '.id')
echo "Director ID: $DIR"
MGR=$($NOUS agent register --name "eng-manager" --type manager --parent "$DIR" | jq -r '.id')
echo "Manager ID: $MGR"
ENG=$($NOUS agent register --name "backend-eng" --type engineer --parent "$MGR" | jq -r '.id')
echo "Engineer ID: $ENG"
pause

echo ""
echo "--- Viewing org tree ---"
$NOUS agent tree
pause

echo ""
echo "--- Sending heartbeat ---"
$NOUS agent heartbeat "$ENG" --status running
pause

echo ""
echo "--- Listing agents ---"
$NOUS agent list
pause
echo ""

# ============================================================
echo "=== 6. SCHEDULE (P4) — Cron & One-Shot ==="
echo ""

echo "--- Creating a cron schedule (every 5 minutes) ---"
SCHED=$($NOUS schedule create --name "health-check" --cron "*/5 * * * *" --action http --payload '{"url":"http://localhost/health","method":"GET"}' | jq -r '.id')
echo "Schedule ID: $SCHED"
pause

echo ""
echo "--- Creating a one-shot schedule ---"
TRIGGER_AT=$(($(date +%s) + 3600))
ONCE=$($NOUS schedule create --name "deploy-notify" --cron "@once" --action shell --payload '{"cmd":"echo deploy ready"}' --trigger-at "$TRIGGER_AT" --max-runs 1 | jq -r '.id')
echo "One-shot ID: $ONCE"
pause

echo ""
echo "--- Listing schedules ---"
$NOUS schedule list
pause

echo ""
echo "--- Showing schedule runs ---"
$NOUS schedule runs "$SCHED"
pause

echo ""
echo "--- Schedule health overview ---"
$NOUS schedule health
pause
echo ""

# ============================================================
echo "=== 7. INVENTORY (P5) — Artifact Registry ==="
echo ""

echo "--- Registering inventory items ---"
INV1=$($NOUS inventory register --name "api-server" --type docker-image --tags "production,backend,v2.1" --path "ghcr.io/nous/api:v2.1" | jq -r '.id')
echo "Item 1 ID: $INV1"
INV2=$($NOUS inventory register --name "worker-binary" --type binary --tags "production,backend" --path "/usr/local/bin/worker" | jq -r '.id')
echo "Item 2 ID: $INV2"
pause

echo ""
echo "--- Listing inventory ---"
$NOUS inventory list
pause

echo ""
echo "--- Searching by tags (AND semantics) ---"
$NOUS inventory search --tag production --tag backend
pause

echo ""
echo "--- Updating lifecycle (archive) ---"
$NOUS inventory archive "$INV2"
pause

echo ""
echo "--- Listing with status filter ---"
$NOUS inventory list --status archived
pause
echo ""

# ============================================================
echo "=== 8. MEMORY (P6) — Persistent Structured Memory ==="
echo ""

echo "--- Saving memories ---"
MEM1=$($NOUS memory save --title "API uses REST not GraphQL" --content "Team decided on REST for simplicity and caching. GraphQL considered but rejected due to complexity." --type decision --importance high --workspace demo | jq -r '.id')
echo "Memory 1 ID: $MEM1"
MEM2=$($NOUS memory save --title "API now uses GraphQL" --content "Reversed earlier decision. GraphQL chosen for flexible queries on mobile clients." --type decision --importance high --workspace demo | jq -r '.id')
echo "Memory 2 ID: $MEM2"
pause

echo ""
echo "--- Searching memories (FTS5) ---"
$NOUS memory search "GraphQL" --workspace demo
pause

echo ""
echo "--- Relating memories (supersedes) ---"
$NOUS memory relate --source "$MEM2" --target "$MEM1" --type supersedes
pause

echo ""
echo "--- Getting context ---"
$NOUS memory context --workspace demo
pause
echo ""

# ============================================================
echo "=== 9. AGENTS (P7) — Templates, Versioning & Upgrades ==="
echo ""

echo "--- Creating an agent template ---"
TPL=$($NOUS agent template create --name "code-reviewer" --type engineer --config '{"model":"opus-4","max_files":50}' --skills '[{"name":"review","path":"/skills/review.md"}]' | jq -r '.id')
echo "Template ID: $TPL"
pause

echo ""
echo "--- Instantiating agent from template ---"
AGENT=$($NOUS agent template instantiate "$TPL" --name "reviewer-alpha" --namespace demo | jq -r '.id')
echo "Agent ID: $AGENT"
pause

echo ""
echo "--- Recording a version ---"
$NOUS agent record-version --agent-id "$AGENT" --skill-hash "abc123def456" --config-hash "cfg789xyz" --skills-json '[{"name":"review","path":"/skills/review.md","hash":"abc123"}]'
pause

echo ""
echo "--- Recording a new version (simulating upgrade) ---"
$NOUS agent record-version --agent-id "$AGENT" --skill-hash "new456hash789" --config-hash "cfgNEW001" --skills-json '[{"name":"review","path":"/skills/review-v2.md","hash":"new456"}]'
pause

echo ""
echo "--- Listing versions ---"
$NOUS agent versions "$AGENT"
pause

echo ""
echo "--- Notifying upgrade available ---"
$NOUS agent notify-upgrade "$AGENT"
pause

echo ""
echo "--- Checking outdated agents ---"
$NOUS agent outdated
pause

echo ""
echo "--- Inspecting agent (full details) ---"
$NOUS agent inspect "$AGENT"
pause
echo ""

# ============================================================
echo "╔══════════════════════════════════════════════════════════════╗"
echo "║                    DEMO COMPLETE                            ║"
echo "║                                                             ║"
echo "║  All 9 features demonstrated:                               ║"
echo "║    1. Doctor    — Setup validation                          ║"
echo "║    2. Chat      — Rooms, messages, FTS5 search              ║"
echo "║    3. Tasks     — Issues, links, notes, history, FTS5       ║"
echo "║    4. Worktrees — Create, list, show, archive               ║"
echo "║    5. Org       — Agent hierarchy, tree, heartbeat          ║"
echo "║    6. Schedule  — Cron, one-shot, health                    ║"
echo "║    7. Inventory — Registry, tags, search, lifecycle         ║"
echo "║    8. Memory    — Save, search, relate, context             ║"
echo "║    9. Agents    — Templates, versions, upgrades             ║"
echo "║                                                             ║"
echo "╚══════════════════════════════════════════════════════════════╝"
echo ""

# Cleanup
echo "Cleaning up temp directory..."
rm -rf "$DEMO_TMP"
echo "Done."
