#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 01 - Getting Started with Nous
# =============================================================================
#
# User Story:
#   As a new user, I want to install nous, verify my setup, start the daemon,
#   and perform a basic workflow so I can confirm everything works.
#
# Prerequisites:
#   - Rust toolchain installed (for building from source)
#   - AWS credentials configured (for LLM features)
#
# =============================================================================

echo "=== Step 1: Build and install nous ==="
# Build the CLI from source (assumes you are in the repo root)
cargo build --release
# The binary is at target/release/nous
# Optionally, copy it to your PATH:
#   cp target/release/nous ~/.local/bin/

echo "=== Step 2: Run diagnostic checks ==="
# The doctor command verifies configuration, storage, database, and model files
nous doctor
# Expected output:
#   nous doctor v0.x.y
#   ==================
#   [OK] Configuration loaded
#   [OK] Storage directory exists
#   [OK] Database connection OK
#   ...

echo "=== Step 3: Start the daemon ==="
# Start the daemon in the background (default port 8377)
nous start
# Expected output:
#   nous daemon started (pid: 12345)
#   PID file: ~/.config/nous/nous.pid
#   Log file: ~/.config/nous/nous-daemon.log

# Alternatively, start in foreground (useful for development)
# nous serve

echo "=== Step 4: Register your first agent ==="
# Register an engineer agent in the default namespace
nous agent register --name "my-first-agent" --type engineer
# Expected output (JSON):
#   {
#     "id": "01234567-...",
#     "name": "my-first-agent",
#     "agent_type": "engineer",
#     "namespace": "default",
#     "status": "active",
#     ...
#   }

echo "=== Step 5: List agents ==="
nous agent list
# Shows all registered agents in the default namespace

echo "=== Step 6: Check agent details ==="
nous agent inspect my-first-agent
# Shows full inspection: version info, process status, recent invocations

echo "=== Step 7: Clean up ==="
# Deregister the test agent
nous agent deregister my-first-agent

# Stop the daemon
nous stop
# Expected output:
#   daemon stopped

echo "=== Done! ==="
echo "You have successfully set up nous. See the other examples for more workflows."
