# Borg

Autonomous AI agent orchestrator — connects to chat (Telegram, Discord, WhatsApp), responds via Claude Code, and runs an engineering pipeline that creates, tests, and merges code changes in Docker-isolated containers.

## Quick Start

```bash
git clone <repo-url> borg && cd borg
just setup   # builds binary, Docker image, sidecar, dashboard
```

Create `.env`:

```bash
TELEGRAM_BOT_TOKEN=<from @BotFather>
ASSISTANT_NAME=Borg
PIPELINE_REPO=/path/to/your/repo
PIPELINE_TEST_CMD=zig build test
```

```bash
just r   # build and run
```

Dashboard at `http://127.0.0.1:3131`. Send `/register` in a Telegram group, then mention `@Borg`.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `TELEGRAM_BOT_TOKEN` | — | Telegram bot token (required) |
| `DISCORD_ENABLED` | `false` | Enable Discord bridge |
| `DISCORD_TOKEN` | — | Discord bot token |
| `WHATSAPP_ENABLED` | `false` | Enable WhatsApp bridge |
| `PIPELINE_REPO` | — | Primary repo path (enables pipeline) |
| `PIPELINE_TEST_CMD` | `zig build test` | Test command for primary repo |
| `PIPELINE_AUTO_MERGE` | `true` | Auto-merge PRs for primary repo |
| `WATCHED_REPOS` | — | Additional repos: `path:cmd\|path:cmd` |
| `WEB_BIND` | `127.0.0.1` | Dashboard bind address (`0.0.0.0` for remote) |
| `WEB_PORT` | `3131` | Dashboard port |
| `CONTINUOUS_MODE` | `false` | Auto-seed tasks when idle |
| `CLAUDE_MODEL` | `claude-sonnet-4-6` | Model for all agents |
| `RELEASE_INTERVAL_MINS` | `180` | Min interval between integration runs |
| `AGENT_TIMEOUT_S` | `1000` | Max agent runtime in seconds |
| `MAX_CONCURRENT_AGENTS` | `4` | Global concurrent agent limit |
| `MAX_PIPELINE_AGENTS` | `4` | Max concurrent pipeline agents |
| `PIPELINE_ADMIN_CHAT` | — | Telegram chat ID for pipeline notifications |

### Merge Modes

By default, PRs are auto-merged after passing tests. To require manual merge review:

- **Primary repo**: set `PIPELINE_AUTO_MERGE=false`
- **Watched repos**: append `!manual` to the entry: `/path/to/repo:make test!manual`

Manual-merge repos still get PRs created, pushed, and rebased automatically — they just skip the merge step.

### Multi-Repo

```bash
PIPELINE_REPO=/home/me/myproject
PIPELINE_TEST_CMD=zig build test
WATCHED_REPOS=/home/me/work-repo:make test!manual|/home/me/other:npm test
```

The dashboard shows a repo filter dropdown when multiple repos are configured.

## Commands

| Just | Description |
|---|---|
| `just r` | Build and run |
| `just t` | Run tests |
| `just b` | Build only |
| `just dash` | Build dashboard |
| `just image` | Build Docker agent image |
| `just setup` | Full setup |

Requires Zig 0.14.1+, Docker, Bun.
