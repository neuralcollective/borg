# borg

Autonomous AI agent orchestrator for Telegram, WhatsApp, and Discord. Chat messages run Claude Code as a subprocess; the engineering pipeline runs agents in Docker containers with git worktree isolation. Supports multiple repositories with independent pipelines. Includes a web dashboard for monitoring. Written in Zig.

## Architecture

```
Telegram Bot API ──┐
                   │
WhatsApp Web ──────┼──> borg (Zig binary) ──> Web Dashboard (:3131)
                   │
Discord Gateway ───┘        |
                            +── SQLite (groups, messages, sessions, pipeline tasks)
                            |
                            +── Per-Group State Machine (chat agents)
                            |     IDLE → COLLECTING → RUNNING → COOLDOWN → IDLE
                            |     Direct claude subprocess, threaded, rate-limited
                            |
                            +── Pipeline Thread (autonomous engineering)
                                  Multi-repo: independent worktrees, tests, release trains
                                  Concurrent phase processing (up to 4 agents)
                                  Per-repo phase dispatch: same phase runs in parallel across repos
                                  backlog → spec → qa → impl → test → done
                                  Release train merges to main, self-heals, self-updates
```

## Requirements

- Zig 0.14.1+
- Docker (pipeline only; chat works without Docker)
- Telegram bot token ([@BotFather](https://t.me/BotFather))
- Claude Code CLI (`npm install -g @anthropic-ai/claude-code`)
- OAuth credentials (`~/.claude/.credentials.json` or `CLAUDE_CODE_OAUTH_TOKEN`)

## Quick start

```bash
# Build container image (pipeline only)
docker build -t borg-agent:latest -f container/Dockerfile container/

# Configure
cat > .env <<EOF
TELEGRAM_BOT_TOKEN=your-token
ASSISTANT_NAME=Borg
CLAUDE_MODEL=claude-opus-4-6
PIPELINE_REPO=/path/to/your/repo
EOF

# Build and run
zig build && ./zig-out/bin/borg
```

Stop with `Ctrl+C` for graceful shutdown (waits for running agents).

## Commands

| Command | Description |
|---------|-------------|
| `/register` | Register this chat for agent responses |
| `/unregister` | Unregister this chat |
| `/status` | Show version, uptime, groups, model |
| `/version` | Show build version |
| `/groups` | List registered groups with phases |
| `/task <title>` | Create an engineering pipeline task |
| `/tasks` | List pipeline tasks |
| `/pipeline` | Show pipeline config |
| `/chatid` | Show chat ID |
| `/ping` | Check if online |
| `/help` | List commands |

Mention the bot by name (e.g. `@Borg`) to trigger a response.

## Chat agent lifecycle

Per-group state machine prevents spam and batches messages:

1. **IDLE** → trigger detected (`@Borg`)
2. **COLLECTING** (3s) → accumulating messages
3. **RUNNING** → `claude` subprocess in dedicated thread
4. **COOLDOWN** (5s) → brief pause before next trigger

Rate limited to N triggers/min/group. Messages during RUNNING are stored for the next invocation. Sessions persist via `--resume`.

## Engineering pipeline

The pipeline runs as a separate thread with concurrent phase processing — multiple tasks progress simultaneously across different phases. Each watched repo gets independent worktrees, test commands, and release trains.

### Multi-repo support

Configure multiple repositories for the pipeline to work on in parallel:

```bash
# Primary repo (always first, triggers self-update on merge)
PIPELINE_REPO=/home/user/my-project
PIPELINE_TEST_CMD=zig build test

# Additional repos (pipe-delimited, each entry is path:test_cmd)
WATCHED_REPOS=/home/user/api-server:go test ./...|/home/user/frontend:bun test
```

Each repo operates independently: separate worktrees, separate test commands, separate release trains. Tasks from different repos in the same phase run concurrently; same-repo tasks in the same phase are serialized.

### Task lifecycle

1. **Backlog** → branch + worktree created in the task's repo
2. **Spec** → Manager agent writes `spec.md` (requirements, file paths, acceptance criteria)
3. **QA** → QA agent writes tests that initially fail
4. **Impl** → Worker agent implements code to pass tests
5. **Test** → repo-specific test command runs
6. **Done** → queued for release train
7. **Retry** → on test failure, Worker gets error context (max 3 attempts)

Each task gets its own git worktree at `{repo}/.worktrees/task-{id}`. Session continuity: agents resume from the previous phase's session.

### Release train

Runs at configurable intervals (default 3h), independently per repo:

1. Create `release-candidate` from `main`
2. Merge branches one-by-one, test after each (using repo-specific test command)
3. Exclude branches causing conflicts or test failures (sent back for rebase)
4. Fast-forward `main`, push, notify admin
5. Self-update: rebuild and restart if primary repo's source changed

### Auto-seeding

When idle, the pipeline scans each watched repo, rotating between three analysis modes:
- **Refactoring**: code quality, duplication, naming
- **Bug audit**: security vulnerabilities, race conditions, resource leaks
- **Test coverage**: untested code paths, missing edge cases

Seeded tasks are small and focused. Cooldown: 1h (60s in continuous mode). Backlog cap: 5 tasks.

### Agent personas

| Persona | Tools | Role |
|---------|-------|------|
| Manager | Read, Glob, Grep, Write | Writes spec.md only |
| QA | Read, Glob, Grep, Write | Writes test files only |
| Worker | Read, Glob, Grep, Write, Edit, Bash | Implements features |

## Web dashboard

Accessible at `http://127.0.0.1:3131` (configurable via `WEB_PORT`).

Shows pipeline task list with repo badges (when multiple repos configured), task detail with agent output, release queue, live log stream (SSE), and system status including version, uptime, and repo count.

To rebuild after changes: `cd dashboard && bun run build`

## WhatsApp support

```bash
cd whatsapp && bun install && cd ..
```

Add to `.env`:
```
WHATSAPP_ENABLED=true
WHATSAPP_AUTH_DIR=whatsapp/auth
```

Scan the QR code on first start. Auth state persists in `WHATSAPP_AUTH_DIR`.

## Discord support

```bash
cd discord && bun install && cd ..
```

Add to `.env`:
```
DISCORD_ENABLED=true
DISCORD_TOKEN=your-bot-token
```

Create a bot at [Discord Developer Portal](https://discord.com/developers/applications), enable **Message Content Intent** under Bot settings, and invite to your server with `bot` + `applications.commands` scopes.

## Config

| Variable | Default | Description |
|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | (required) | Telegram Bot API token |
| `CLAUDE_CODE_OAUTH_TOKEN` | (auto) | OAuth token (auto-read from credentials file) |
| `ASSISTANT_NAME` | `Borg` | Bot name and trigger word |
| `CLAUDE_MODEL` | `claude-sonnet-4-6` | Model for Claude Code CLI |
| `COLLECTION_WINDOW_MS` | `3000` | Message batching window |
| `COOLDOWN_MS` | `5000` | Cooldown after agent completes |
| `AGENT_TIMEOUT_S` | `600` | Max agent runtime |
| `MAX_CONCURRENT_AGENTS` | `4` | Global concurrent agent limit |
| `RATE_LIMIT_PER_MINUTE` | `5` | Triggers per minute per group |
| `WEB_PORT` | `3131` | Dashboard port |
| `WHATSAPP_ENABLED` | `false` | Enable WhatsApp bridge |

### Pipeline

| Variable | Default | Description |
|----------|---------|-------------|
| `PIPELINE_REPO` | (empty) | Primary repo path (enables pipeline) |
| `PIPELINE_TEST_CMD` | `zig build test` | Test command for primary repo |
| `WATCHED_REPOS` | (empty) | Additional repos (`path:cmd\|path:cmd`) |
| `PIPELINE_ADMIN_CHAT` | (empty) | Chat ID for notifications |
| `RELEASE_INTERVAL_MINS` | `180` | Minutes between release trains |
| `CONTINUOUS_MODE` | `false` | Run release train after every completed task |

## Deployment

### Docker Compose (recommended for portability)

```bash
# Edit .env with your config, then:
docker compose up -d

# View logs
docker compose logs -f

# Rebuild after code changes
docker compose up -d --build
```

Mount your repos into the container via `docker-compose.yml` volumes:

```yaml
volumes:
  - /path/to/your/repo:/repos/my-project
```

Then set `PIPELINE_REPO=/repos/my-project` in `.env`.

### systemd (Linux, no Docker overhead)

```bash
# Edit borg.service paths if needed, then:
sudo ln -sf $(pwd)/borg.service /etc/systemd/system/borg.service
sudo systemctl daemon-reload
sudo systemctl enable --now borg

# Check status / follow logs
systemctl status borg
journalctl -u borg -f
```

### Bare metal

```bash
zig build && ./zig-out/bin/borg
```

## Database migrations

Schema upgrades are handled automatically via versioned migrations. Fresh installs get the full schema; existing databases run only new ALTER TABLE migrations. The migration version is tracked in the `state` table.

## Container security

Pipeline containers run with: `--cap-drop ALL`, `--security-opt no-new-privileges`, `--pids-limit 256`, `--memory 1GB`, `--cpus 2`, `--network host`, `--rm`. Bind mount validation blocks sensitive paths. Chat agents run as direct subprocesses.

## Testing

```bash
zig build test
```

## License

MIT
