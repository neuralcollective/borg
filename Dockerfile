FROM debian:bookworm-slim AS build

# Install Zig, git, and build deps
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl xz-utils git ca-certificates libc-dev && \
    curl -fsSL https://ziglang.org/download/0.14.1/zig-linux-x86_64-0.14.1.tar.xz | \
    tar -xJ -C /opt && \
    ln -s /opt/zig-linux-x86_64-0.14.1/zig /usr/local/bin/zig && \
    rm -rf /var/lib/apt/lists/*

# Build dashboard
COPY dashboard/package.json dashboard/bun.lock* /app/dashboard/
RUN curl -fsSL https://bun.sh/install | bash && \
    cd /app/dashboard && /root/.bun/bin/bun install --frozen-lockfile 2>/dev/null || /root/.bun/bin/bun install
COPY dashboard/ /app/dashboard/
RUN cd /app/dashboard && /root/.bun/bin/bun run build

# Build borg binary
COPY . /app
RUN cd /app && zig build -Doptimize=ReleaseSafe

# --- Runtime ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    git ca-certificates docker.io && \
    rm -rf /var/lib/apt/lists/*

# Install Claude Code CLI (needed for chat subprocess agents)
RUN apt-get update && apt-get install -y --no-install-recommends nodejs npm && \
    npm install -g @anthropic-ai/claude-code@latest && \
    rm -rf /var/lib/apt/lists/*

COPY --from=build /app/zig-out/bin/borg /usr/local/bin/borg
COPY --from=build /app/dashboard/dist /opt/borg/dashboard/dist
COPY --from=build /app/container /opt/borg/container

ENV DASHBOARD_DIST_DIR=/opt/borg/dashboard/dist

WORKDIR /data
VOLUME ["/data"]
EXPOSE 3131

ENTRYPOINT ["borg"]
