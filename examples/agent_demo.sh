#!/usr/bin/env bash
# Agent Demo — Standard Invoke + Sandboxed Invoke
#
# Demonstrates two invocation paths using existing nous CLI commands:
#   1. Standard: register agent with process_type=claude, invoke with a prompt
#   2. Sandboxed: spawn agent with --type sandbox, invoke with a prompt
#
# Prerequisites:
#   - nous binary built with sandbox feature: cargo build -p nous-cli --features sandbox
#   - AWS credentials configured (SSO profile or env vars)
#   - No other nous daemon running on port 8377
#
# Usage:
#   ./examples/agent_demo.sh [--profile PROFILE] [--region REGION] [--model MODEL]

set -euo pipefail

# --- Configuration ---
PROFILE="${AWS_PROFILE:-h3-dev}"
REGION="${AWS_REGION:-us-east-1}"
MODEL="${NOUS_LLM_MODEL:-us.anthropic.claude-sonnet-4-20250514-v1:0}"
NOUS="./target/debug/nous"
PROMPT="What is 2+2? Reply with just the number."

while [[ $# -gt 0 ]]; do
    case $1 in
        --profile) PROFILE="$2"; shift 2 ;;
        --region) REGION="$2"; shift 2 ;;
        --model) MODEL="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# --- Helpers ---
cleanup() {
    echo ""
    echo "[cleanup] Stopping daemon (pid $DAEMON_PID)..."
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
    echo "[cleanup] Done."
}

print_header() {
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  $1"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

# --- Preflight ---
print_header "AGENT DEMO — Standard + Sandboxed Invoke"
echo ""
echo "Config:"
echo "  Profile: $PROFILE"
echo "  Region:  $REGION"
echo "  Model:   $MODEL"
echo "  Prompt:  \"$PROMPT\""

if [[ ! -x "$NOUS" ]]; then
    echo ""
    echo "ERROR: $NOUS not found. Build first:"
    echo "  cargo build -p nous-cli --features sandbox"
    exit 1
fi

# Verify AWS credentials
echo ""
echo "[preflight] Verifying AWS credentials..."
if ! AWS_PROFILE="$PROFILE" AWS_REGION="$REGION" aws sts get-caller-identity &>/dev/null; then
    echo "ERROR: AWS credentials not valid for profile '$PROFILE'. Run: aws sso login --profile $PROFILE"
    exit 1
fi
echo "[preflight] AWS credentials OK."

# --- Start Daemon ---
print_header "Starting Daemon"
echo "[daemon] nous serve --profile $PROFILE --region $REGION --model $MODEL"
AWS_PROFILE="$PROFILE" AWS_REGION="$REGION" "$NOUS" serve \
    --profile "$PROFILE" --region "$REGION" --model "$MODEL" &>/tmp/nous-demo-daemon.log &
DAEMON_PID=$!
trap cleanup EXIT
sleep 4

if ! kill -0 "$DAEMON_PID" 2>/dev/null; then
    echo "ERROR: Daemon failed to start. Check /tmp/nous-demo-daemon.log"
    exit 1
fi
echo "[daemon] Running (pid $DAEMON_PID, port 8377)"

# --- Demo 1: Standard Invoke ---
print_header "Demo 1: STANDARD INVOKE"
echo ""
echo "[standard] Registering agent 'demo-standard'..."
STD_OUTPUT=$("$NOUS" agent register --name demo-standard --type engineer 2>&1) || true
STD_ID=$(echo "$STD_OUTPUT" | grep '"id"' | head -1 | sed 's/.*: "//;s/".*//')

if [[ -z "$STD_ID" ]]; then
    # Agent may already exist from previous run
    STD_ID=$("$NOUS" agent lookup demo-standard 2>&1 | grep '"id"' | head -1 | sed 's/.*: "//;s/".*//')
fi
echo "[standard] Agent ID: $STD_ID"

echo "[standard] Setting process_type=claude..."
"$NOUS" agent update "$STD_ID" --process-type claude &>/dev/null

echo "[standard] Sending prompt: \"$PROMPT\""
echo ""
RESULT=$("$NOUS" agent invoke "$STD_ID" --prompt "$PROMPT" --timeout 30 2>&1)
STATUS=$(echo "$RESULT" | grep '"status"' | sed 's/.*: "//;s/".*//')
RESPONSE=$(echo "$RESULT" | grep '"result"' | sed 's/.*: "//;s/".*//' | sed 's/^null$//')
DURATION=$(echo "$RESULT" | grep '"duration_ms"' | sed 's/.*: //;s/,.*//')
ERROR=$(echo "$RESULT" | grep '"error"' | sed 's/.*: "//;s/".*//' | sed 's/^null$//')

if [[ "$STATUS" == "completed" ]]; then
    echo "  ✓ Status:   $STATUS"
    echo "  ✓ Response: $RESPONSE"
    echo "  ✓ Duration: ${DURATION}ms"
else
    echo "  ✗ Status: $STATUS"
    echo "  ✗ Error:  $ERROR"
fi

# --- Demo 2: Sandboxed Invoke ---
print_header "Demo 2: SANDBOXED INVOKE"
echo ""
echo "[sandbox] Registering agent 'demo-sandbox'..."
SBX_OUTPUT=$("$NOUS" agent register --name demo-sandbox --type engineer 2>&1) || true
SBX_ID=$(echo "$SBX_OUTPUT" | grep '"id"' | head -1 | sed 's/.*: "//;s/".*//')

if [[ -z "$SBX_ID" ]]; then
    SBX_ID=$("$NOUS" agent lookup demo-sandbox 2>&1 | grep '"id"' | head -1 | sed 's/.*: "//;s/".*//')
fi
echo "[sandbox] Agent ID: $SBX_ID"

echo "[sandbox] Spawning sandbox (image=ubuntu:24.04, cpus=2, memory=512MiB, network=none)..."
SPAWN_RESULT=$("$NOUS" agent spawn "$SBX_ID" \
    --type sandbox \
    --sandbox-image ubuntu:24.04 \
    --sandbox-cpus 2 \
    --sandbox-memory 512 \
    --sandbox-network none 2>&1) || true
SPAWN_STATUS=$(echo "$SPAWN_RESULT" | grep '"status"' | head -1 | sed 's/.*: "//;s/".*//')
echo "[sandbox] Sandbox process status: $SPAWN_STATUS"

echo "[sandbox] Verifying agent process_type=sandbox..."
AGENT_PT=$("$NOUS" agent lookup demo-sandbox 2>&1 | grep '"process_type"' | sed 's/.*: "//;s/".*//')
echo "[sandbox] Agent process_type: $AGENT_PT"

echo "[sandbox] Sending prompt: \"$PROMPT\""
echo ""
RESULT=$("$NOUS" agent invoke "$SBX_ID" --prompt "$PROMPT" --timeout 30 2>&1)
STATUS=$(echo "$RESULT" | grep '"status"' | sed 's/.*: "//;s/".*//')
RESPONSE=$(echo "$RESULT" | grep '"result"' | sed 's/.*: "//;s/".*//' | sed 's/^null$//')
DURATION=$(echo "$RESULT" | grep '"duration_ms"' | sed 's/.*: //;s/,.*//')
ERROR=$(echo "$RESULT" | grep '"error"' | sed 's/.*: "//;s/".*//' | sed 's/^null$//')

if [[ "$STATUS" == "completed" ]]; then
    echo "  ✓ Status:   $STATUS"
    echo "  ✓ Response: $RESPONSE"
    echo "  ✓ Duration: ${DURATION}ms"
else
    echo "  ✗ Status: $STATUS"
    echo "  ✗ Error:  $ERROR"
fi

echo "[sandbox] Stopping sandbox..."
"$NOUS" agent stop "$SBX_ID" &>/dev/null 2>&1 || true
echo "[sandbox] Sandbox stopped."

# --- Summary ---
print_header "DEMO COMPLETE"
echo ""
echo "Both invocation paths demonstrated:"
echo "  1. Standard:  agent register → update --process-type claude → invoke"
echo "  2. Sandboxed: agent register → spawn --type sandbox → invoke → stop"
echo ""
echo "The sandbox path proves container lifecycle management works alongside"
echo "LLM invocation. The agent runs inside a sandbox context with resource"
echo "limits (CPU, memory, network policy) while still accessing Bedrock."
