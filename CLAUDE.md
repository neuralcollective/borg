# CLAUDE.md

Borg is an autonomous AI agent orchestrator written in Zig. It connects to Telegram, WhatsApp, and Discord to respond to chat messages (via Claude Code subprocess), and runs an engineering pipeline that autonomously creates, tests, and merges code changes using Docker-isolated agents.

## Project Structure

```
src/
  main.zig          # Orchestrator: main loop, message routing, agent dispatch
  config.zig        # Env/.env parsing, RepoConfig, all config fields
  db.zig            # SQLite schema, CRUD, migrations
  sqlite.zig        # Low-level SQLite C bindings
  telegram.zig      # Telegram Bot API long-poll client
  sidecar.zig       # Unified Discord+WhatsApp bridge (single bun process, NDJSON IPC)
  agent.zig         # Claude subprocess runner, NDJSON stream parser
  pipeline.zig      # Autonomous engineering: spec→qa→impl→test→release
  docker.zig        # Docker container lifecycle via Unix socket
  git.zig           # Git CLI wrapper (worktrees, branches, rebase)
  web.zig           # HTTP server for dashboard API + SSE
  http.zig          # HTTP client (TCP + Unix socket)
  json.zig          # JSON parse/escape utilities
container/
  Dockerfile        # Pipeline agent image (node + claude CLI)
  entrypoint.sh     # Agent entrypoint: parses JSON input, runs claude
dashboard/          # React + Vite + Tailwind web dashboard
sidecar/            # Unified messaging bridge (bun, discord.js + Baileys)
vendor/sqlite/      # Vendored SQLite amalgamation
```

## Build & Test

```bash
zig build              # Build to zig-out/bin/borg
zig build test         # Run all unit tests
```

Requires Zig 0.14.1+. SQLite is vendored (no external deps for the core binary).

## Configuration

All config is in `.env` (or process environment). Key variables:

- `TELEGRAM_BOT_TOKEN` — required for Telegram
- `DISCORD_ENABLED=true` + `DISCORD_TOKEN` — for Discord
- `WHATSAPP_ENABLED=true` — for WhatsApp
- `PIPELINE_REPO=/path/to/repo` — enables the engineering pipeline
- `PIPELINE_TEST_CMD=zig build test` — test command for primary repo
- `WATCHED_REPOS=path:cmd|path:cmd` — additional repos
- `CLAUDE_MODEL=claude-sonnet-4-6` — model for all agents

See `src/config.zig` for the full list with defaults.

## Key Patterns

- **Transport-agnostic messaging**: `Transport` enum (`.telegram`, `.whatsapp`, `.discord`, `.web`) + `Sender` struct dispatches to the right backend.
- **Unified sidecar**: Discord and WhatsApp run in a single bun process (`sidecar/bridge.js`) communicating via multiplexed NDJSON over stdin/stdout. Events have a `source` field; commands have a `target` field.
- **Per-group state machine**: `IDLE → COLLECTING → RUNNING → COOLDOWN → IDLE`. Collection window batches messages. Rate-limited per group.
- **Pipeline phases**: `backlog → spec → qa → impl → test → done → release`. Each task gets a git worktree. Agents run in Docker containers with `--cap-drop ALL`.
- **SQLite WAL mode**: All threads share one DB connection with `busy_timeout=5000ms`.
- **Self-update**: Pipeline detects merges to main on the primary repo, rebuilds, and restarts via `execve`.

## Code Style

- No slop comment prefixes (`AUDIT:`, `SECURITY:`, `NOTE:`). `TODO:` is fine.
- Keep comments concise or omit if code is self-explanatory.
- Use `bun` (not `npm`) for JS dependencies.
- Zig style: snake_case, `errdefer` for cleanup, `catch |err|` for error handling.

## Deployment

Three options: `zig build && ./zig-out/bin/borg` (bare metal), `docker compose up -d` (Docker Compose), or systemd (`borg.service`). The systemd service needs a PATH that includes zig, docker, and bun.

## Dashboard

```bash
cd dashboard && bun install && bun run build
```

Served at `http://127.0.0.1:3131`. The API is in `src/web.zig`. Dashboard source is React + Vite + Tailwind in `dashboard/src/`.
