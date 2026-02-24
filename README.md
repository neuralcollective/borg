# borg

Autonomous AI agent orchestrator triggered via Telegram and WhatsApp. Chat messages run Claude Code directly as a subprocess. The autonomous engineering pipeline runs agents in Docker containers with git worktree isolation. Written in Zig.

Each registered chat gets a per-group state machine with message batching, rate limiting, and non-blocking threaded agent execution. Messages are collected during a configurable window, formatted into prompts, and piped to `claude --print`. Responses stream back as NDJSON and are sent to the chat. Sessions persist across invocations via `--resume`.

## Architecture

```
Telegram Bot API ──┐
                   ├──> borg (Zig binary)
WhatsApp Web ──────┘        |
                            +── SQLite (groups, messages, sessions, pipeline tasks)
                            |
                            +── Per-Group State Machine
                            |     IDLE → COLLECTING (3s window) → RUNNING → COOLDOWN → IDLE
                            |     Agent runs as direct claude subprocess (threaded)
                            |     Messages batched during collection window
                            |     Rate limited (N triggers/min/group)
                            |
                            +── Pipeline Thread (autonomous engineering)
                                  |
                                  +── Micro-loop: backlog → spec → qa → impl → test → done
                                  |     Each task in isolated git worktree
                                  |     Agents run in Docker containers (security boundary)
                                  |
                                  +── Macro-loop: release train (every N hours)
                                        Merge queued branches, self-healing, push to main
```

## Requirements

- Zig 0.14.1+
- Docker (only for pipeline; chat works without Docker)
- A Telegram bot token (from [@BotFather](https://t.me/BotFather))
- Claude Code CLI installed (`npm install -g @anthropic-ai/claude-code`)
- Claude Code OAuth credentials (`~/.claude/.credentials.json`) or `CLAUDE_CODE_OAUTH_TOKEN` env var

## Setup

### 1. Build the container image (for pipeline only)

```bash
docker build -t borg-agent:latest -f container/Dockerfile container/
```

### 2. Configure environment

Create a `.env` file in the project root:

```
TELEGRAM_BOT_TOKEN=your-telegram-bot-token
ASSISTANT_NAME=Borg
CLAUDE_MODEL=claude-opus-4-6
```

The OAuth token is read automatically from `~/.claude/.credentials.json` (rotated by Claude Code). You can also set `CLAUDE_CODE_OAUTH_TOKEN` in `.env` as a fallback.

### 3. Build and run

```bash
zig build
./zig-out/bin/borg
```

Stop with `SIGTERM` or `Ctrl+C` for graceful shutdown (waits for running agents).

## Commands

| Command | Description |
|---------|-------------|
| `/register` | Register the current chat for agent responses |
| `/unregister` | Unregister the current chat |
| `/status` | Show uptime, group count, active agents, model |
| `/groups` | List all registered groups with phases |
| `/chatid` | Show the chat's internal ID |
| `/ping` | Check if the bot is online |
| `/task <title>` | Create an engineering pipeline task |
| `/tasks` | List pipeline tasks and their status |
| `/pipeline` | Show pipeline configuration |
| `/help` | List available commands |

After registering, mention the bot by name (e.g. `@Borg`) to trigger a response. Commands work from both Telegram and WhatsApp.

## Chat Agent Lifecycle

When a trigger is detected, the per-group state machine manages the agent lifecycle:

1. **IDLE**: Waiting for trigger
2. **COLLECTING** (3s window): Accumulating messages. Additional messages extend the window slightly.
3. **RUNNING**: Agent thread spawned. `claude` runs as a direct subprocess. Messages during this phase are stored and included in the next invocation.
4. **COOLDOWN** (5s): Brief pause before accepting new triggers.

This prevents rapid-fire messages from spawning multiple agents and ensures message batching.

## Autonomous Engineering Pipeline

The pipeline runs as a separate thread and processes tasks through a multi-stage loop:

### Continuous operation

When the backlog is empty, the pipeline automatically scans the target repository and seeds small refactoring/quality tasks. This means borg never truly idles - it continuously improves the codebase. Seeded tasks focus on refactoring, not new features: extracting duplication, improving error handling, adding missing test coverage, simplifying complex code.

Seeding respects a cooldown (1h between scans) and a backlog cap (max 5 active tasks) to avoid runaway task creation.

Tasks can also be created manually via `/task <title>` in chat.

### Micro-loop (per task)

1. **Backlog**: Task created via seeder or `/task` command
2. **Spec**: Manager agent reads the codebase and writes `spec.md` with requirements, file paths, and acceptance criteria
3. **QA**: QA agent reads `spec.md` and writes test files that should initially fail
4. **Impl**: Worker agent reads spec + tests and writes implementation code
5. **Test**: Zig runs the configured test command deterministically
6. **Done**: Tests pass, branch queued for integration
7. **Retry**: Tests fail, Worker gets stderr context and retries (max 3 attempts)

Each task gets its own git worktree at `{repo}/.worktrees/task-{id}`, keeping the main working tree clean.

### Macro-loop (release train)

Runs every N hours (configurable, default 3h):

1. Freeze the integration queue
2. Create `release-candidate` branch from `main`
3. Merge feature branches one-by-one, run tests after each
4. Self-healing: exclude branches that cause merge conflicts or test failures
5. Fast-forward `main` to release-candidate
6. Push to remote, notify admin via Telegram with digest

### Agent personas

| Persona | Tools | Role |
|---------|-------|------|
| Manager | Read, Glob, Grep, Write | Writes spec.md only |
| QA | Read, Glob, Grep, Write | Writes test files only |
| Worker | Read, Glob, Grep, Write, Edit, Bash | Implements features |

## WhatsApp Support

Borg can connect to WhatsApp Web as an additional messaging backend using a Node.js bridge built on [Baileys](https://github.com/WhiskeySockets/Baileys).

### Setup

```bash
cd whatsapp && bun install && cd ..
```

Add to `.env`:

```
WHATSAPP_ENABLED=true
WHATSAPP_AUTH_DIR=whatsapp/auth
```

On first start, a QR code will be printed to the terminal. Scan it with WhatsApp on your phone (Settings > Linked Devices > Link a Device). Auth state is persisted in the `WHATSAPP_AUTH_DIR` directory.

WhatsApp groups use `wa:` prefix for JIDs (e.g., `wa:123456789-987654321@g.us`). Register and trigger them the same way as Telegram groups.

## Config

| Variable | Default | Description |
|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | (required) | Telegram Bot API token |
| `CLAUDE_CODE_OAUTH_TOKEN` | (from credentials file) | OAuth token for Claude Code |
| `ASSISTANT_NAME` | `Borg` | Bot's display name and trigger word |
| `TRIGGER_PATTERN` | `@Borg` | Mention pattern to trigger the bot |
| `DATA_DIR` | `data` | Directory for session and IPC data |
| `CONTAINER_IMAGE` | `borg-agent:latest` | Docker image for pipeline containers |
| `CLAUDE_MODEL` | `claude-opus-4-6` | Model passed to Claude Code CLI |
| `COLLECTION_WINDOW_MS` | `3000` | Message collection window before spawning agent |
| `COOLDOWN_MS` | `5000` | Cooldown period after agent completes |
| `AGENT_TIMEOUT_S` | `600` | Max seconds an agent can run |
| `MAX_CONCURRENT_AGENTS` | `4` | Global limit on concurrent agent runs |
| `RATE_LIMIT_PER_MINUTE` | `5` | Max triggers per minute per group |
| `WHATSAPP_ENABLED` | `false` | Enable WhatsApp Web bridge |
| `WHATSAPP_AUTH_DIR` | `whatsapp/auth` | Directory for WhatsApp auth state |

### Pipeline config

| Variable | Default | Description |
|----------|---------|-------------|
| `PIPELINE_REPO` | (empty) | Path to target repository (enables pipeline) |
| `PIPELINE_TEST_CMD` | `zig build test` | Command to run tests |
| `PIPELINE_LINT_CMD` | (empty) | Optional lint command |
| `PIPELINE_ADMIN_CHAT` | (empty) | Telegram chat ID for pipeline notifications |
| `RELEASE_INTERVAL_MINS` | `180` | Minutes between release trains |

## Container security (pipeline only)

Pipeline agent containers run with:

- `--cap-drop ALL` (no Linux capabilities)
- `--security-opt no-new-privileges:true`
- `--pids-limit 256`
- `--memory 1GB`
- `--cpus 2`
- `--network host` (required for Claude Code API access)
- `--rm` (auto-removed after exit)
- Bind mount validation blocks sensitive paths (`.ssh`, `.aws`, `.gnupg`, `.env`, etc.)

Chat agents run as direct subprocesses (no container overhead).

## Testing

```bash
zig build test
```

Tests across all modules: JSON parsing, SQLite bindings, config parsing, HTTP chunked decoding, NDJSON parsing, prompt formatting, folder sanitization, trigger detection, database operations, git operations, pipeline logic, state machine transitions, rate limiting.

## License

MIT
