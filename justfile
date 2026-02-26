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

# Restart the systemd user service
restart:
    systemctl --user restart borg

# Build release and restart service
deploy: b restart

# Test, build, and restart service
s: t b install-service restart

ship: s

# Install/update the systemd user service file
install-service:
    mkdir -p ~/.config/systemd/user
    cp borg.service ~/.config/systemd/user/borg.service
    systemctl --user daemon-reload

# Check status via API
status:
    curl -s http://127.0.0.1:3131/api/status | jq .
