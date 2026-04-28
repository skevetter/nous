#!/usr/bin/env bash
set -euo pipefail

UNIT_DIR="${HOME}/.config/systemd/user"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

mkdir -p "${UNIT_DIR}"
cp "${SCRIPT_DIR}/nous-cli.service" "${UNIT_DIR}/nous-cli.service"

systemctl --user daemon-reload
systemctl --user enable nous-cli
systemctl --user start nous-cli

systemctl --user status nous-cli
