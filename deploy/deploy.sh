#!/usr/bin/env bash
set -euo pipefail

HOST="${BORG_HOST:?BORG_HOST env var required (e.g. root@1.2.3.4)}"
REMOTE_DIR="/opt/borg"

echo "==> Deploying to $HOST"

# Build dashboard locally
echo "==> Building dashboard..."
(cd "$(dirname "$0")/../dashboard" && bun install --frozen-lockfile && bun run build)

# Sync repo (exclude heavy/sensitive dirs)
echo "==> Syncing files..."
rsync -az --delete \
  --exclude .git \
  --exclude node_modules \
  --exclude target \
  --exclude store \
  --exclude .env \
  --exclude '.worktrees' \
  "$(dirname "$0")/../" "$HOST:$REMOTE_DIR/"

# Build and restart on VPS
echo "==> Building on VPS..."
ssh "$HOST" "
    set -euo pipefail
    source /root/.cargo/env
    cd $REMOTE_DIR

    # Build borg-server
    cd borg-rs && cargo build --release && cd ..

    # Build agent Docker image
    docker build -t borg-agent -f container/Dockerfile container/

    # Install/restart services
    cp deploy/borg.service /etc/systemd/system/
    systemctl daemon-reload
    systemctl enable borg
    systemctl restart borg
"

echo "==> Checking health..."
sleep 3
ssh "$HOST" "curl -sf http://127.0.0.1:3131/api/health" && echo " OK" || echo " Still starting..."
