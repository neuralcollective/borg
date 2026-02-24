# borg

Autonomous AI agent orchestrator that runs Claude Code inside Docker containers, triggered via Telegram. Written in Zig.

Each registered chat gets its own isolated Docker container with Claude Code CLI. Messages are collected, formatted into prompts, and piped to the container via stdin. Responses stream back as NDJSON and are sent to the chat. Sessions persist across invocations.

## Architecture

```
Telegram Bot API
    |
    v
borg (Zig binary)
    |
    +-- SQLite (groups, messages, sessions, state)
    |
    +-- Docker CLI
          |
          v
        borg-agent container (Node 22 + Claude Code CLI)
            reads JSON from stdin → runs claude --print --output-format stream-json
            writes NDJSON to stdout
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
| `/help` | List available commands |

After registering, mention the bot by name (e.g. `@Borg`) to trigger a response.

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
- `--memory 512MB`
- `--cpus 2`
- `--network host` (required for Claude Code API access)
- `--rm` (auto-removed after exit)
- Bind mount validation blocks sensitive paths (`.ssh`, `.aws`, `.gnupg`, `.env`, etc.)

## Testing

```bash
zig build test
```

20 tests across all modules: JSON parsing, SQLite bindings, config parsing, HTTP chunked decoding, NDJSON parsing, prompt formatting, folder sanitization, trigger detection, and database operations.

## License

MIT
