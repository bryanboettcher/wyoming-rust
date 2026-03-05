#!/usr/bin/env bash
set -euo pipefail

PI=root@10.13.1.51
LINES=${1:-30}

ssh "$PI" "docker logs wyoming-satellite --tail $LINES"
