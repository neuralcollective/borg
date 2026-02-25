# CLAUDE.md

Borg is an autonomous AI agent orchestrator written in Zig. It connects to Telegram, WhatsApp, and Discord to respond to chat messages (via Claude Code subprocess), and runs an engineering pipeline that autonomously creates, tests, and merges code changes.

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
  Dockerfile        # Pipeline agent image (bun + claude CLI)
  entrypoint.sh     # Agent entrypoint: parses JSON input, runs claude
dashboard/          # React + Vite + Tailwind web dashboard
sidecar/            # Unified messaging bridge (bun, discord.js + Baileys)
vendor/sqlite/      # Vendored SQLite amalgamation
```

## Build & Test

```bash
just t                 # Run all unit tests
just b                 # Build to zig-out/bin/borg
just r                 # Build and run
just dash              # Build dashboard
just setup             # Full setup (image + sidecar + dashboard + build)
```

Requires Zig 0.14.1+. SQLite is vendored (no external deps for the core binary).

## Configuration

All config is in `.env` (or process environment). See `src/config.zig` for the full list with defaults.

## Key Patterns

- **Transport-agnostic messaging**: `Transport` enum (`.telegram`, `.whatsapp`, `.discord`, `.web`) + `Sender` struct dispatches to the right backend.
- **Unified sidecar**: Discord and WhatsApp run in a single bun process (`sidecar/bridge.js`) communicating via multiplexed NDJSON over stdin/stdout.
- **Per-group state machine**: `IDLE → COLLECTING → RUNNING → COOLDOWN → IDLE`. Collection window batches messages. Rate-limited per group.
- **Pipeline phases**: `backlog → spec → qa → impl → done → release`. Each task gets a git worktree. Impl agents run in Docker containers, rebase agents run on host.
- **Session persistence**: Per-task session dirs (`store/sessions/task-{id}/.claude`) are bind-mounted into Docker containers so agents can resume across retries. Full NDJSON streams stored in DB for dashboard replay.
- **Self-update**: Pipeline detects merges to main on the primary repo, rebuilds, and restarts via `execve`.

## Code Style

- No slop comment prefixes (`AUDIT:`, `SECURITY:`, `NOTE:`). `TODO:` is fine.
- Keep comments concise or omit if code is self-explanatory.
- Use `bun` (not `npm`) for JS dependencies.
- Zig style: snake_case, `errdefer` for cleanup, `catch |err|` for error handling.
