#!/usr/bin/env bash
set -euo pipefail

# =============================================================================
# 04 - Configuration
# =============================================================================
#
# User Story:
#   As a platform operator, I want to configure nous using config files,
#   environment variables, and CLI flags to match my environment.
#
# This example covers:
#   - Config file location and format (TOML)
#   - Available configuration options
#   - Environment variable overrides
#   - CLI flag overrides
#   - LLM provider configuration (AWS Bedrock)
#
# =============================================================================

echo "=== Configuration File Location ==="
# nous looks for its config file at:
#   Linux:   ~/.config/nous/config.toml
#   macOS:   ~/Library/Application Support/nous/config.toml
#
# If the file does not exist, sensible defaults are used.

echo ""
echo "=== Default Configuration Values ==="
# When no config file is present, nous uses:
#   data_dir = ~/.local/share/nous     (Linux)
#   host     = "127.0.0.1"
#   port     = 8377
#
#   [search]
#   tokenizer = "porter unicode61"

echo ""
echo "=== Example config.toml ==="
cat <<'TOML'
# ~/.config/nous/config.toml

# Where nous stores its SQLite databases and model files
data_dir = "/home/user/.local/share/nous"

# Daemon bind address and port
host = "127.0.0.1"
port = 8377

# Full-text search configuration
[search]
tokenizer = "porter unicode61"
TOML

echo ""
echo "=== Creating a config file ==="
# Create the config directory
mkdir -p ~/.config/nous

# Write a custom configuration
cat > ~/.config/nous/config.toml <<'EOF'
# Custom nous configuration
data_dir = "/home/user/.local/share/nous"
host = "127.0.0.1"
port = 9000

[search]
tokenizer = "porter unicode61"
EOF

echo "Config written to ~/.config/nous/config.toml"

echo ""
echo "=== CLI Flag Overrides ==="
# The --port flag overrides the config file for any command:
nous --port 9000 doctor
nous --port 9000 agent list

# Start daemon on a custom port:
# nous start --port 9000
# nous --port 9000 serve

echo ""
echo "=== LLM Provider Configuration ==="
# nous uses AWS Bedrock for LLM capabilities.
# Configuration can be set via CLI flags, environment variables, or config.

# Method 1: CLI flags (highest priority)
# nous start --model "anthropic.claude-sonnet-4-20250514-v1:0" --region "us-west-2" --profile "my-aws-profile"

# Method 2: Environment variables
export NOUS_MODEL="anthropic.claude-sonnet-4-20250514-v1:0"
export NOUS_REGION="us-west-2"
export NOUS_PROFILE="my-aws-profile"

# These are read by the daemon at startup:
# nous start

echo ""
echo "=== AWS Credential Methods ==="
# nous checks for AWS credentials in this order:
#   1. AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY (explicit keys)
#   2. AWS_PROFILE (named profile from ~/.aws/credentials)
#   3. AWS_CONTAINER_CREDENTIALS_RELATIVE_URI (ECS/container credentials)
#
# If no credentials are found, LLM features are disabled but the daemon still runs.

echo ""
echo "=== Verifying Configuration ==="
# Use `nous doctor` to verify your setup
nous doctor
# Expected output shows status of:
#   - Configuration loading
#   - Storage directory
#   - Database connectivity
#   - Port availability
#   - Embedding model files

echo ""
echo "=== Data Directory Structure ==="
# After running nous, the data directory contains:
#   ~/.local/share/nous/
#     nous.db          - Main SQLite database (agents, tasks, chat, etc.)
#     nous-vec.db      - Vector/embedding database
#     models/          - Downloaded embedding model files

echo ""
echo "=== Environment Variables Reference ==="
cat <<'REF'
Variable          Purpose                              Default
-----------       -----------------------------------  -----------------------
NOUS_MODEL        LLM model ID for Bedrock             anthropic.claude-sonnet-4-20250514-v1:0
NOUS_REGION       AWS region for Bedrock               us-west-2
NOUS_PROFILE      AWS profile name                     (none)
RUST_LOG          Log level for tracing                (none, use info/debug)
REF

echo ""
echo "Done! Configuration overview complete."
