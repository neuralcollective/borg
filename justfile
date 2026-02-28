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
    for i in $(seq 1 20); do curl -sf http://127.0.0.1:3131/api/status >/dev/null && break; sleep 1; done
    curl -sf http://127.0.0.1:3131/api/status >/dev/null || (echo "borg did not come up on :3131"; exit 1)

# Stop the service
stop:
    #!/usr/bin/env bash
    if [ "$(uname)" = "Darwin" ]; then
        launchctl unload ~/Library/LaunchAgents/com.borg.agent.plist 2>/dev/null || true
    else
        systemctl --user stop borg
        systemctl --user is-active borg || true
    fi

# Build release and restart service
deploy: b restart

# Test, build, and restart service
s: t b install-service restart

ship: dash s

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
