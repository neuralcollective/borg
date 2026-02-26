FROM oven/bun:1-slim

# System dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    ca-certificates \
    xz-utils \
    make \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code CLI and make it accessible to all users
RUN bun install -g @anthropic-ai/claude-code@latest \
    && chmod o+x /root \
    && chmod -R o+rx /root/.bun

# Install Rust (stable) for pipeline agents working on Rust repos
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --quiet \
    && chmod -R o+rx /root/.cargo /root/.rustup
ENV PATH="/root/.cargo/bin:$PATH"

# Create workspace dirs
RUN mkdir -p /workspace/group /workspace/ipc /home/bun/.claude

# Copy entrypoint
COPY entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

# Set ownership
RUN chown -R bun:bun /workspace /home/bun

USER bun
WORKDIR /workspace/group

ENTRYPOINT ["/app/entrypoint.sh"]
