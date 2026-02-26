# CLAUDE.md

Borg is an autonomous AI agent orchestrator written in Rust. It connects to Telegram, WhatsApp, and Discord to respond to chat messages (via Claude Code subprocess), and runs an engineering pipeline that autonomously creates, tests, and merges code changes.

## Project Structure

```
borg-rs/                # Rust implementation (active codebase)
  crates/
    borg-core/          # Pipeline, DB, config, agent traits, modes
    borg-agent/         # Claude + Ollama agent backends
    borg-server/        # Axum HTTP server, routes, logging
container/
  Dockerfile            # Pipeline agent image (bun + claude CLI)
  entrypoint.sh         # Agent entrypoint: parses JSON input, runs claude
dashboard/              # React + Vite + Tailwind web dashboard
sidecar/                # Unified Discord+WhatsApp bridge (bun, discord.js + Baileys)
```

## Build & Test

```bash
just t                 # Run all unit tests
just b                 # Build release binary
just deploy            # Build and restart service
just dash              # Build dashboard
just setup             # Full setup (image + sidecar + dashboard + build)
```

Requires Rust stable. Use the user service flow (`just install-service` + `just restart`) instead of `sudo systemctl`.

## Configuration

All config is in `.env` (or process environment). Key variables:

- `PIPELINE_REPO`, `PIPELINE_TEST_CMD` — primary repo path and test command
- `PIPELINE_AUTO_MERGE=true|false` — auto-merge PRs for primary repo (default: true)
- `WATCHED_REPOS=path:cmd|path:cmd` — additional repos, append `!manual` to disable auto-merge. Optional third field for prompt file: `path:cmd:prompt_file`
- `WEB_BIND=0.0.0.0` — bind address for dashboard (default: 127.0.0.1)
- `CONTINUOUS_MODE=true` — auto-seed tasks when pipeline is idle
- `CONTAINER_SETUP=path/to/setup.sh` — script sourced at container start
- `CONTAINER_MEMORY_MB=1024` — container memory limit
- `PIPELINE_MAX_BACKLOG=5` — max concurrent pipeline tasks
- `PIPELINE_SEED_COOLDOWN_S=3600` — min seconds between seed scans
- `PIPELINE_TICK_S=30` — pipeline main loop interval
- `REMOTE_CHECK_INTERVAL_S=300` — git fetch interval for self-update
- `DISCORD_TOKEN`, `WA_AUTH_DIR`, `WA_DISABLED`, `OBSERVER_CONFIG`

## Key Patterns

- **Transport-agnostic messaging**: `Transport` enum (telegram/whatsapp/discord/web) + `Sender` dispatches to the right backend.
- **Unified sidecar**: Discord and WhatsApp run in a single bun process (`sidecar/bridge.js`) via multiplexed NDJSON over stdin/stdout.
- **Per-group state machine**: `IDLE → COLLECTING → RUNNING → COOLDOWN → IDLE`. Collection window batches messages.
- **Pipeline phases**: `backlog → spec → qa → impl → done → release`. Each task gets a git worktree. Impl agents run in Docker containers, rebase agents run on host.
- **Session persistence**: Per-task session dirs (`store/sessions/task-{id}/`) bind-mounted into Docker containers so agents resume across retries.
- **Per-repo prompts**: Pipeline agents receive repo-specific context via `.borg/prompt.md` or explicit `prompt_file` in WATCHED_REPOS.
- **Self-update**: Pipeline detects merges to main on the primary repo, rebuilds, and restarts via `execve`.

## Code Style

- No slop comment prefixes (`AUDIT:`, `SECURITY:`, `NOTE:`). `TODO:` is fine.
- Keep comments concise or omit if code is self-explanatory.
- Use `bun` (not `npm`) for JS dependencies.
- Rust style: snake_case, `?` for error propagation, `anyhow` for errors.

## Git Commits

- Co-author: `Co-Authored-By: Sasha Duke <sashadanielduke@gmail.com>`
- Do NOT add Claude/Anthropic co-authorship lines.
