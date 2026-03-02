#!/usr/bin/env bash
set -euo pipefail

HOST="${BORG_HOST:?BORG_HOST env var required}"
REMOTE_DIR="/root/borg"

echo "Deploying to $HOST..."

ssh "$HOST" "
    set -euo pipefail
    cd $REMOTE_DIR

    git pull --ff-only

    cd deploy
    docker compose build borg
    docker compose up -d
"

echo "Deploy complete. Checking health..."
sleep 3
curl -sf "https://api.borg.legal/api/status" | jq .status || echo "Health check failed (may still be starting)"
