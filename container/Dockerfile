FROM node:22-slim

# System dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code CLI
RUN npm install -g @anthropic-ai/claude-code@latest

# Create workspace dirs
RUN mkdir -p /workspace/group /workspace/ipc /home/node/.claude

# Copy entrypoint
COPY entrypoint.sh /app/entrypoint.sh
RUN chmod +x /app/entrypoint.sh

# Set ownership
RUN chown -R node:node /workspace /home/node

USER node
WORKDIR /workspace/group

ENTRYPOINT ["/app/entrypoint.sh"]
