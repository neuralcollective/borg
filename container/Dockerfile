FROM oven/bun:1-slim

# System dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    ca-certificates \
    xz-utils \
    make \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code CLI
RUN bun install -g @anthropic-ai/claude-code@latest

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
