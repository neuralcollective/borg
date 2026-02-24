# borg

Autonomous AI agent orchestrator that runs Claude Code inside Docker containers, triggered via Telegram. Includes an autonomous engineering pipeline with three agent personas (Manager, QA, Worker) for automated feature development. Written in Zig.

Each registered chat gets its own isolated Docker container with Claude Code CLI. Messages are collected, formatted into prompts, and piped to the container via stdin. Responses stream back as NDJSON and are sent to the chat. Sessions persist across invocations.

## Architecture

```
Telegram Bot API
    |
    v
borg (Zig binary)
    |
    +-- SQLite (groups, messages, sessions, pipeline tasks, integration queue)
    |
    +-- Docker CLI
    |     |
    |     v
    |   borg-agent container (Node 22 + Claude Code CLI)
    |       reads JSON from stdin -> runs claude --print --output-format stream-json
    |       writes NDJSON to stdout
    |
    +-- Pipeline Thread (autonomous engineering)
          |
          +-- Micro-loop: backlog -> spec -> qa -> impl -> test -> done
          |     Manager agent -> spec.md
          |     QA agent -> test files
          |     Worker agent -> implementation
          |     Zig runs tests deterministically
          |     Retry with error context (max 3 attempts)
          |
          +-- Macro-loop: release train (every N hours)
                Merge queued branches one-by-one
                Self-healing: exclude failing branches
                Push to main, notify admin
```

## Requirements

- Zig 0.14.1+
- Docker
- A Telegram bot token (from [@BotFather](https://t.me/BotFather))
- Claude Code OAuth credentials (`~/.claude/.credentials.json`) or `CLAUDE_CODE_OAUTH_TOKEN` env var

## Setup

### 1. Build the container image

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

## Commands

| Command | Description |
|---------|-------------|
| `/register` | Register the current chat for agent responses |
| `/unregister` | Unregister the current chat |
| `/status` | Show uptime, group count, active agents, model |
| `/groups` | List all registered groups |
| `/chatid` | Show the chat's internal ID |
| `/ping` | Check if the bot is online |
| `/task <title>` | Create an engineering pipeline task |
| `/tasks` | List pipeline tasks and their status |
| `/pipeline` | Show pipeline configuration |
| `/help` | List available commands |

After registering, mention the bot by name (e.g. `@Borg`) to trigger a response.

## Autonomous Engineering Pipeline

The pipeline runs as a separate thread and processes tasks through a multi-stage loop:

### Micro-loop (per task)

1. **Backlog**: Task created via `/task` command
2. **Spec**: Manager agent reads the codebase and writes `spec.md` with requirements, file paths, and acceptance criteria
3. **QA**: QA agent reads `spec.md` and writes test files that should initially fail
4. **Impl**: Worker agent reads spec + tests and writes implementation code
5. **Test**: Zig runs the configured test command deterministically
6. **Done**: Tests pass, branch queued for integration
7. **Retry**: Tests fail, Worker gets stderr context and retries (max 3 attempts)

Each task gets its own git branch (`feature/task-{id}`). Git is the state machine.

### Macro-loop (release train)

Runs every N hours (configurable, default 6h):

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

### Pipeline config

| Variable | Default | Description |
|----------|---------|-------------|
| `PIPELINE_REPO` | (empty) | Path to target repository (enables pipeline) |
| `PIPELINE_TEST_CMD` | `zig build test` | Command to run tests |
| `PIPELINE_LINT_CMD` | (empty) | Optional lint command |
| `PIPELINE_ADMIN_CHAT` | (empty) | Telegram chat ID for pipeline notifications |
| `RELEASE_INTERVAL_HOURS` | `6` | Hours between release trains |

## Config

| Variable | Default | Description |
|----------|---------|-------------|
| `TELEGRAM_BOT_TOKEN` | (required) | Telegram Bot API token |
| `CLAUDE_CODE_OAUTH_TOKEN` | (from credentials file) | OAuth token for Claude Code |
| `ASSISTANT_NAME` | `Borg` | Bot's display name and trigger word |
| `TRIGGER_PATTERN` | `@Borg` | Mention pattern to trigger the bot |
| `DATA_DIR` | `data` | Directory for session and IPC data |
| `CONTAINER_IMAGE` | `borg-agent:latest` | Docker image for agent containers |
| `CLAUDE_MODEL` | `claude-opus-4-6` | Model passed to Claude Code CLI |

## Container security

Each agent container runs with:

- `--cap-drop ALL` (no Linux capabilities)
- `--security-opt no-new-privileges:true`
- `--pids-limit 256`
- `--memory 512MB` (1GB for pipeline agents)
- `--cpus 2`
- `--network host` (required for Claude Code API access)
- `--rm` (auto-removed after exit)
- Bind mount validation blocks sensitive paths (`.ssh`, `.aws`, `.gnupg`, `.env`, etc.)

## Testing

```bash
zig build test
```

Tests across all modules: JSON parsing, SQLite bindings, config parsing, HTTP chunked decoding, NDJSON parsing, prompt formatting, folder sanitization, trigger detection, database operations, git operations, pipeline logic.

## License

MIT
