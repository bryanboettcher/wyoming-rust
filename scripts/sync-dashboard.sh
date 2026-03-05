#!/usr/bin/env bash
set -euo pipefail

PI=root@10.13.1.51
PI_DIR=/root/voice-assistant

echo "==> Syncing dashboard files..."
rsync -av --delete deploy/dashboard/ "$PI:$PI_DIR/dashboard/"

echo "==> Restarting dashboard container..."
ssh "$PI" "cd $PI_DIR && docker compose restart dashboard"
