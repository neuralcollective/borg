FROM oven/bun:1-slim

# System dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    ca-certificates \
    xz-utils \
    make \
    openssh-client \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code CLI and Agent SDK, make accessible to the bun user via group permissions
RUN bun install -g @anthropic-ai/claude-code@latest @anthropic-ai/claude-agent-sdk \
    && chgrp bun /root && chmod g+x /root \
    && chgrp -R bun /root/.bun && chmod -R g+rx /root/.bun

# Install Rust (stable) for pipeline agents working on Rust repos
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --quiet \
    && chgrp -R bun /root/.cargo /root/.rustup && chmod -R g+rx /root/.cargo /root/.rustup
ENV PATH="/root/.cargo/bin:$PATH"

# Create workspace dirs and cache mount points owned by bun
RUN mkdir -p \
    /workspace/group /workspace/ipc /workspace/repo \
    /mirrors \
    /home/bun/.claude \
    /home/bun/.bun/install/cache \
    /home/bun/.cargo/registry

# Copy entrypoint
COPY entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

# Set ownership
RUN chown -R bun:bun /workspace /mirrors /home/bun

USER bun
WORKDIR /workspace/repo

ENTRYPOINT ["/app/entrypoint.sh"]
