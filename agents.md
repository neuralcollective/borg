# Setting Up Borg

Step-by-step setup for a fresh machine.

## Prerequisites

- Linux (tested on Arch, should work on Ubuntu/Debian)
- Docker daemon running
- Zig 0.14.1+
- Bun (for Claude Code CLI and messaging sidecar)
- Claude Code CLI (`bun install -g @anthropic-ai/claude-code`)
- Claude OAuth credentials at `~/.claude/.credentials.json` (created by `claude` login)

## 1. Clone and Build

```bash
git clone <repo-url> borg && cd borg
just setup
```

This builds the binary, Docker agent image, sidecar deps, and dashboard.

## 2. Configure

Create `.env` in the project root:

```bash
# Required: at least one messaging backend
TELEGRAM_BOT_TOKEN=<token from @BotFather>

# Optional: Discord
DISCORD_ENABLED=true
DISCORD_TOKEN=<token from Discord Developer Portal>

# Optional: WhatsApp (scan QR on first start)
WHATSAPP_ENABLED=true

# Bot identity
ASSISTANT_NAME=Borg

# Pipeline (optional — enables autonomous engineering)
PIPELINE_REPO=/absolute/path/to/target/repo
PIPELINE_TEST_CMD=zig build test
# PIPELINE_AUTO_MERGE=false        # skip auto-merge for primary repo
# Additional repos (append !manual to disable auto-merge)
# WATCHED_REPOS=/path/to/api:go test ./...|/path/to/web:bun test!manual

# Remote dashboard access (default: 127.0.0.1)
# WEB_BIND=0.0.0.0

# Notify this Telegram chat about pipeline events
# PIPELINE_ADMIN_CHAT=<chat_id>
```

## 3. Run

```bash
just r
```

Or with systemd:

```bash
mkdir -p ~/.config/systemd/user
cp borg.service ~/.config/systemd/user/borg.service
systemctl --user daemon-reload
systemctl --user enable --now borg
journalctl --user -u borg -f
```

## 4. Register a Chat

In your Telegram group (or Discord channel), send `/register` to the bot. Then mention it by name (e.g. `@Borg`) to trigger a response.

## 5. Create Pipeline Tasks

Send `/task Fix the login bug` in a registered chat, or let the auto-seeder discover tasks when the pipeline is idle.

## Verify It Works

- `just status` returns JSON with version and uptime
- `/ping` in Telegram responds with `pong`
- Pipeline tasks appear at `http://127.0.0.1:3131`

## Discord Bot Setup

1. Go to https://discord.com/developers/applications
2. Create application → Bot → copy token
3. Enable **Message Content Intent** under Bot settings
4. Invite: OAuth2 → URL Generator → scopes: `bot` + `applications.commands` → permissions: Send Messages, Read Message History
5. Set `DISCORD_ENABLED=true` and `DISCORD_TOKEN=<token>` in `.env`
