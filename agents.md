# Setting Up Borg

Step-by-step setup for a fresh machine.

## Prerequisites

- Linux (tested on Arch, should work on Ubuntu/Debian)
- Docker daemon running
- Zig 0.14.1+ (`curl -fsSL https://ziglang.org/download/0.14.1/zig-linux-x86_64-0.14.1.tar.xz | sudo tar -xJ -C /opt && sudo ln -s /opt/zig-linux-x86_64-0.14.1/zig /usr/local/bin/zig`)
- Node.js 18+ (for Claude Code CLI and chat bridges)
- Claude Code CLI (`bun install -g @anthropic-ai/claude-code`)
- Claude OAuth credentials at `~/.claude/.credentials.json` (created by `claude` login)

## 1. Clone and Build

```bash
git clone <repo-url> borg && cd borg
zig build
```

## 2. Build Pipeline Container Image

```bash
docker build -t borg-agent:latest -f container/Dockerfile container/
```

This image has Node.js + Claude Code CLI. Pipeline agents run inside it.

## 3. Install Chat Bridge Dependencies

```bash
# WhatsApp (optional)
cd whatsapp && bun install && cd ..

# Discord (optional)
cd discord && bun install && cd ..
```

## 4. Build Dashboard

```bash
cd dashboard && bun install && bun run build && cd ..
```

## 5. Configure

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
# Additional repos: pipe-delimited path:test_cmd pairs
# WATCHED_REPOS=/path/to/api:go test ./...|/path/to/web:bun test

# Notify this Telegram chat about pipeline events
# PIPELINE_ADMIN_CHAT=<chat_id>
```

## 6. Run

```bash
./zig-out/bin/borg
```

Or with systemd:

```bash
# Edit borg.service: set WorkingDirectory and PATH
sudo cp borg.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now borg
journalctl -u borg -f
```

The systemd service must have a PATH including zig, node, docker, and bun. See `borg.service` for an example.

## 7. Register a Chat

In your Telegram group (or Discord channel), send `/register` to the bot. Then mention it by name (e.g. `@Borg`) to trigger a response.

## 8. Create Pipeline Tasks

Send `/task Fix the login bug` in a registered chat, or let the auto-seeder discover tasks when the pipeline is idle.

## Verify It Works

- `curl http://127.0.0.1:3131/api/status` returns JSON with version and uptime
- `journalctl -u borg -f` shows `Borg X.Y.Z-<hash> online`
- `/ping` in Telegram responds with `pong`
- Pipeline tasks appear at `http://127.0.0.1:3131`

## Discord Bot Setup

1. Go to https://discord.com/developers/applications
2. Create application → Bot → copy token
3. Enable **Message Content Intent** under Bot settings
4. Invite: OAuth2 → URL Generator → scopes: `bot` + `applications.commands` → permissions: Send Messages, Read Message History → copy URL and open in browser
5. Set `DISCORD_ENABLED=true` and `DISCORD_TOKEN=<token>` in `.env`

## Docker Compose (Alternative)

```bash
# Edit .env, then:
docker compose up -d
docker compose logs -f
```

Mount target repos as volumes in `docker-compose.yml`:

```yaml
volumes:
  - /path/to/repo:/repos/my-project
```

Then set `PIPELINE_REPO=/repos/my-project` in `.env`.
