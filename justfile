# Borg — autonomous AI agent orchestrator

# Run all unit tests
t:
    zig build test

# Build the binary
b:
    zig build

# Build and run
r:
    zig build && ./zig-out/bin/borg

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

# Check status via API
status:
    curl -s http://127.0.0.1:3131/api/status | jq .
