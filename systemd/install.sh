#!/usr/bin/env bash
set -euo pipefail

UNIT_DIR="${HOME}/.config/systemd/user"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "${UNIT_DIR}"
cp "${SCRIPT_DIR}/nous-mcp.service" "${UNIT_DIR}/nous-mcp.service"

systemctl --user daemon-reload
systemctl --user enable nous-mcp
systemctl --user start nous-mcp

systemctl --user status nous-mcp
