# Borg — autonomous AI agent orchestrator

# Run all unit tests
t:
    cd borg-rs && cargo test

# Build release binary
b:
    cd borg-rs && cargo build --release

# Build and run (debug)
r:
    cd borg-rs && cargo build && ./target/debug/borg-server

# Build dashboard
dash:
    cd dashboard && bun install && bun run build

# Build Docker agent image
image:
    docker build -t borg-agent:latest -f container/Dockerfile container/

# Boot the local dependency stack: Postgres, SeaweedFS, Vespa.
stack-up:
    docker compose -f deploy/docker-compose.stack.yml up -d

# Stop the local dependency stack.
stack-down:
    docker compose -f deploy/docker-compose.stack.yml down -v

# Tail dependency stack logs.
stack-logs:
    docker compose -f deploy/docker-compose.stack.yml logs -f --tail=200

# Run the local ingest/retrieval load harness against a running Borg server.
local-loadtest *ARGS='':
    python3 deploy/local_loadtest.py {{ARGS}}

# Install sidecar dependencies
sidecar:
    cd sidecar && bun install

# Full setup: build everything
setup: image sidecar dash b

# Restart/start the service and verify API comes up.
restart:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        launchctl unload ~/Library/LaunchAgents/com.borg.agent.plist 2>/dev/null || true
        launchctl load -w ~/Library/LaunchAgents/com.borg.agent.plist
    else
        systemctl --user daemon-reload
        systemctl --user enable borg >/dev/null 2>&1 || true
        systemctl --user restart borg 2>/dev/null || systemctl --user start borg
    fi
    for i in $(seq 1 20); do
      curl -sf http://127.0.0.1:3131/api/health >/dev/null 2>&1 && break
      sleep 1
    done
    if ! curl -sf http://127.0.0.1:3131/api/health >/dev/null 2>&1; then
      echo "borg did not come up on :3131"
      if [ "$(uname)" = "Darwin" ]; then
        launchctl list com.borg.agent 2>/dev/null || true
        tail -n 80 store/borg-server.log 2>/dev/null || true
      else
        systemctl --user --no-pager --full status borg 2>/dev/null || true
        journalctl --user -u borg -n 80 --no-pager 2>/dev/null || true
      fi
      exit 1
    fi

# Stop the service
stop:
    #!/usr/bin/env bash
    if [ "$(uname)" = "Darwin" ]; then
        launchctl unload ~/Library/LaunchAgents/com.borg.agent.plist 2>/dev/null || true
    else
        systemctl --user stop borg
        systemctl --user is-active borg || true
    fi

# Ensure Postgres (and other deps) are running
ensure-stack:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! docker ps --format '{{"{{"}}{{".Names"}}{{"}}"}}' 2>/dev/null | grep -q borg-postgres; then
        echo "starting dependency stack (postgres, seaweedfs, vespa)..."
        docker compose -f deploy/docker-compose.stack.yml up -d
        echo "waiting for postgres..."
        for i in $(seq 1 30); do
            docker exec borg-postgres pg_isready -U borg -d borg >/dev/null 2>&1 && break
            sleep 1
        done
    fi

# Build release and restart service
deploy: ensure-stack b restart

# Test, build, and restart service
s: t b install-service restart

ship: ensure-stack dash s

# Connect to borg postgres
db:
    docker exec -it borg-postgres psql -U borg -d borg

# Full remote bootstrap + deploy (requires BORG_HOST env var).
agent-deploy:
    deploy/agent-deploy.sh

# Terraform hybrid infra (requires deploy/terraform/hybrid/terraform.tfvars)
infra-plan:
    deploy/provision-hybrid.sh plan

infra-apply:
    deploy/provision-hybrid.sh apply

infra-destroy:
    deploy/provision-hybrid.sh destroy

# Provision hybrid infra and deploy Borg in one run.
infra-ship:
    deploy/provision-and-deploy.sh

# Install/update the service file (systemd on Linux, launchd on macOS)
install-service:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname)" = "Darwin" ]; then
        mkdir -p ~/Library/LaunchAgents
        BORG_DIR="$(cd "$(dirname "$0")" && pwd)"
        sed "s|__BORG_DIR__|${BORG_DIR}|g; s|__HOME__|${HOME}|g" borg.plist.template > ~/Library/LaunchAgents/com.borg.agent.plist
    else
        mkdir -p ~/.config/systemd/user
        cp borg.service ~/.config/systemd/user/borg.service
        systemctl --user daemon-reload
        systemctl --user enable borg >/dev/null 2>&1 || true
    fi

# Serve landing page locally at http://localhost:3000
landing:
    bunx http-server landing -p 3000

# Check service status
status:
    #!/usr/bin/env bash
    if [ "$(uname)" = "Darwin" ]; then
        launchctl list com.borg.agent 2>/dev/null || \
        curl -sf http://127.0.0.1:3131/api/status 2>/dev/null | jq . || \
        echo "borg is not running"
    else
        systemctl --user status borg 2>/dev/null || \
        systemctl status borg 2>/dev/null || \
        curl -sf http://127.0.0.1:3131/api/status 2>/dev/null | jq . || \
        echo "borg is not running"
    fi
