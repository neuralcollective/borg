# ── Build stage ──
FROM alpine:3.21 AS build

RUN apk add --no-cache curl xz git

# Zig
RUN curl -fsSL https://ziglang.org/download/0.14.1/zig-linux-x86_64-0.14.1.tar.xz | \
    tar -xJ -C /opt && \
    ln -s /opt/zig-linux-x86_64-0.14.1/zig /usr/local/bin/zig

# Dashboard
RUN curl -fsSL https://bun.sh/install | sh
COPY dashboard/package.json dashboard/bun.lock* /app/dashboard/
RUN cd /app/dashboard && ~/.bun/bin/bun install
COPY dashboard/ /app/dashboard/
RUN cd /app/dashboard && ~/.bun/bin/bun run build

# Borg binary (static musl target)
COPY . /app
RUN cd /app && zig build -Doptimize=ReleaseSafe -Dtarget=x86_64-linux-musl

# ── Runtime ──
FROM alpine:3.21

RUN apk add --no-cache git ca-certificates bash curl unzip && \
    curl -fsSL https://bun.sh/install | bash && \
    ln -s /root/.bun/bin/bun /usr/local/bin/bun && \
    bun install -g @anthropic-ai/claude-code@latest && \
    rm -rf /tmp/*

# Docker CLI only (not the daemon)
COPY --from=docker:27-cli /usr/local/bin/docker /usr/local/bin/docker

COPY --from=build /app/zig-out/bin/borg /usr/local/bin/borg
COPY --from=build /app/dashboard/dist /opt/borg/dashboard/dist
COPY --from=build /app/container /opt/borg/container

ENV DASHBOARD_DIST_DIR=/opt/borg/dashboard/dist

WORKDIR /data
VOLUME ["/data"]
EXPOSE 3131

ENTRYPOINT ["borg"]
