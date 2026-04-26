#!/usr/bin/env bash
# End-to-end test for nous-mcp and nous-otlp services.
# Manual test — excluded from CI. Run after `cargo build --release`.
# Requires: sqlite3, curl
set -euo pipefail

TMPDIR_E2E="$(mktemp -d /tmp/nous-e2e.XXXXXX)"
MCP_DB="${TMPDIR_E2E}/memory.db"
OTLP_DB="${TMPDIR_E2E}/otlp.db"
KEY_FILE="${TMPDIR_E2E}/db.key"
CONFIG_FILE="${TMPDIR_E2E}/config.toml"
IMPORT_FILE="${TMPDIR_E2E}/import.json"
MCP_PID=""
OTLP_PID=""
OTLP_PORT=0
PASS=0
FAIL=0

cleanup() {
    [ -n "$MCP_PID" ] && kill "$MCP_PID" 2>/dev/null || true
    [ -n "$OTLP_PID" ] && kill "$OTLP_PID" 2>/dev/null || true
    wait "$MCP_PID" 2>/dev/null || true
    wait "$OTLP_PID" 2>/dev/null || true
    rm -rf "$TMPDIR_E2E"
}
trap cleanup EXIT

assert_eq() {
    local label="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        echo "  PASS: $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $label (expected '$expected', got '$actual')"
        FAIL=$((FAIL + 1))
    fi
}

assert_contains() {
    local label="$1" needle="$2" haystack="$3"
    if echo "$haystack" | grep -qF "$needle"; then
        echo "  PASS: $label"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: $label (expected to contain '$needle')"
        FAIL=$((FAIL + 1))
    fi
}

# Resolve binary paths — prefer target/release, fall back to target/debug
CARGO_TARGET="${CARGO_TARGET_DIR:-target}"
if [ -x "${CARGO_TARGET}/release/nous-mcp" ]; then
    BIN_DIR="${CARGO_TARGET}/release"
elif [ -x "${CARGO_TARGET}/debug/nous-mcp" ]; then
    BIN_DIR="${CARGO_TARGET}/debug"
else
    echo "ERROR: nous-mcp binary not found. Run 'cargo build' first."
    exit 1
fi

if [ ! -x "${BIN_DIR}/nous-otlp" ]; then
    echo "ERROR: nous-otlp binary not found in ${BIN_DIR}."
    exit 1
fi

echo "=== nous e2e test ==="
echo "binaries: ${BIN_DIR}"
echo "tmpdir:   ${TMPDIR_E2E}"

# --- Setup: encryption key and config ---
echo "test-e2e-key-do-not-use" > "$KEY_FILE"

cat > "$CONFIG_FILE" <<EOF
[memory]
db_path = "${MCP_DB}"

[embedding]
model = "mock"
variant = "mock"
chunk_size = 512
chunk_overlap = 64

[otlp]
db_path = "${OTLP_DB}"
port = 4318

[classification]
confidence_threshold = 0.3

[encryption]
db_key_file = "${KEY_FILE}"
EOF

# --- Start nous-otlp serve ---
echo ""
echo "--- Starting nous-otlp ---"

# Find a free port
OTLP_PORT=$(python3 -c 'import socket; s=socket.socket(); s.bind(("",0)); print(s.getsockname()[1]); s.close()')

NOUS_DB_KEY="test-e2e-key-do-not-use" \
    "${BIN_DIR}/nous-otlp" serve --port "$OTLP_PORT" --db "$OTLP_DB" &
OTLP_PID=$!

# Wait for OTLP to be ready
for i in $(seq 1 30); do
    if curl -sf "http://127.0.0.1:${OTLP_PORT}/v1/logs" -X POST \
        -H "content-type: application/x-protobuf" -d "" 2>/dev/null; then
        break
    fi
    # 400 also means the server is up (rejects empty body)
    if curl -sf -o /dev/null -w '%{http_code}' "http://127.0.0.1:${OTLP_PORT}/v1/logs" \
        -X POST -H "content-type: application/x-protobuf" -d "" 2>/dev/null | grep -qE '(200|400)'; then
        break
    fi
    sleep 0.5
done
echo "nous-otlp listening on port ${OTLP_PORT} (pid ${OTLP_PID})"

# --- Store a memory via nous-mcp import ---
echo ""
echo "--- Storing memory via import ---"

cat > "$IMPORT_FILE" <<'IMPORTEOF'
{
  "version": 1,
  "memories": [
    {
      "id": "mem_e2etest0001",
      "title": "E2E Test Memory",
      "content": "This memory was created by the end-to-end test script.",
      "memory_type": "fact",
      "source": "e2e-test",
      "importance": "medium",
      "confidence": "high",
      "session_id": null,
      "trace_id": null,
      "agent_id": null,
      "agent_model": null,
      "valid_from": null,
      "valid_until": null,
      "category_id": null,
      "created_at": "2026-01-01T00:00:00Z",
      "updated_at": "2026-01-01T00:00:00Z",
      "tags": ["e2e", "test"],
      "relationships": []
    }
  ],
  "categories": []
}
IMPORTEOF

NOUS_DB_KEY="test-e2e-key-do-not-use" \
NOUS_MEMORY_DB="$MCP_DB" \
NOUS_DB_KEY_FILE="$KEY_FILE" \
    "${BIN_DIR}/nous-mcp" import "$IMPORT_FILE"

echo "import complete"

# --- Send a trace to nous-otlp via curl ---
echo ""
echo "--- Sending OTLP log via curl ---"

# The OTLP endpoint requires protobuf. We send a minimal (empty resource_logs)
# protobuf payload. An empty ExportLogsServiceRequest encodes to zero bytes,
# which the server accepts (stores 0 logs, returns 200).
# A truly valid protobuf requires the prost crate, which we can't use from bash.
# Instead, we verify the server responds and then query the DB directly.
HTTP_CODE=$(curl -s -o /dev/null -w '%{http_code}' \
    "http://127.0.0.1:${OTLP_PORT}/v1/logs" \
    -X POST \
    -H "content-type: application/x-protobuf" \
    -d "")

echo "OTLP /v1/logs response: HTTP ${HTTP_CODE}"

# --- Query memory DB via sqlite3 ---
echo ""
echo "--- Verifying memory DB ---"

SQLCIPHER="${BIN_DIR}/../../../sqlcipher"
if command -v sqlcipher &>/dev/null; then
    SQLCIPHER="sqlcipher"
elif [ -x "$SQLCIPHER" ]; then
    : # use the resolved path
else
    # Fall back: use the Rust binary's export command to verify
    echo "sqlcipher not found; verifying via nous-mcp export instead"

    EXPORT_OUTPUT=$(NOUS_DB_KEY="test-e2e-key-do-not-use" \
        NOUS_MEMORY_DB="$MCP_DB" \
        NOUS_DB_KEY_FILE="$KEY_FILE" \
        "${BIN_DIR}/nous-mcp" export 2>/dev/null)

    assert_contains "export contains title" "E2E Test Memory" "$EXPORT_OUTPUT"
    assert_contains "export contains content" "end-to-end test script" "$EXPORT_OUTPUT"
    assert_contains "export contains source" "e2e-test" "$EXPORT_OUTPUT"
    assert_contains "export contains tag" '"e2e"' "$EXPORT_OUTPUT"

    # Verify OTLP DB was created (file exists and is non-empty)
    if [ -s "$OTLP_DB" ]; then
        echo "  PASS: OTLP DB file exists and is non-empty"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: OTLP DB file missing or empty"
        FAIL=$((FAIL + 1))
    fi

    # Verify the OTLP server accepted our request
    if [ "$HTTP_CODE" = "200" ] || [ "$HTTP_CODE" = "400" ]; then
        echo "  PASS: OTLP server responded (HTTP ${HTTP_CODE})"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: OTLP server unexpected response (HTTP ${HTTP_CODE})"
        FAIL=$((FAIL + 1))
    fi

    echo ""
    echo "=== Results: ${PASS} passed, ${FAIL} failed ==="
    [ "$FAIL" -eq 0 ] && exit 0 || exit 1
fi

# If sqlcipher is available, query directly
MEM_COUNT=$(echo "PRAGMA key = 'test-e2e-key-do-not-use'; SELECT count(*) FROM memories;" | "$SQLCIPHER" "$MCP_DB")
assert_eq "memory count >= 1" "1" "$MEM_COUNT"

MEM_TITLE=$(echo "PRAGMA key = 'test-e2e-key-do-not-use'; SELECT title FROM memories LIMIT 1;" | "$SQLCIPHER" "$MCP_DB")
assert_eq "memory title" "E2E Test Memory" "$MEM_TITLE"

TAG_COUNT=$(echo "PRAGMA key = 'test-e2e-key-do-not-use'; SELECT count(*) FROM memory_tags;" | "$SQLCIPHER" "$MCP_DB")
assert_eq "tag count" "2" "$TAG_COUNT"

# --- Query OTLP DB ---
echo ""
echo "--- Verifying OTLP DB ---"

# OTLP DB tables should exist (even if empty — the schema is created on open)
OTLP_TABLES=$(echo "PRAGMA key = 'test-e2e-key-do-not-use'; SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;" | "$SQLCIPHER" "$OTLP_DB")
assert_contains "otlp has log_events table" "log_events" "$OTLP_TABLES"
assert_contains "otlp has spans table" "spans" "$OTLP_TABLES"
assert_contains "otlp has metrics table" "metrics" "$OTLP_TABLES"

echo ""
echo "=== Results: ${PASS} passed, ${FAIL} failed ==="
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
