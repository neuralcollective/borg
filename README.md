# Borg

Autonomous Workforce — domain-specific AI pipelines that research, draft, build, review, and ship work end-to-end. Real-time dashboard and chat integration (Telegram, Discord, WhatsApp).

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
PIPELINE_TEST_CMD="cargo test"
```

```bash
just ship   # build dashboard, test, build binary, deploy
```

Dashboard at `http://127.0.0.1:3131`.

## Pipelines

Borg ships with pipeline presets for software engineering, legal, medical writing, healthcare admin, construction, sales, data analysis, frontend, and more. Each preset defines its own phases, system prompts, tool access, and autonomous scanning rules.

Create custom pipelines via the mode creator in the dashboard or the API. Presets are starting points — everything is configurable.

### How It Works

Tasks move through configurable phases. Each task gets its own git branch. Agents run in Docker containers (preferred), bubblewrap sandboxes, or direct mode — controlled by `SANDBOX_BACKEND`. Sessions persist across retries.

When `CONTINUOUS_MODE=true`, preset scanners periodically analyse watched repos and create tasks autonomously.

### MCP Integrations

Domain-specific MCP servers extend agent capabilities — legal research (CourtListener, EDGAR, Federal Register, EUR-Lex, and more), building permits (Shovels), banking (Plaid), plus BYOK support for premium providers (LexisNexis, Westlaw, Clio, iManage).

### Chat

Mention the bot in a registered Telegram, Discord, or WhatsApp group. The agent runs with full conversation context. Each group has its own persistent session.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `TELEGRAM_BOT_TOKEN` | — | Telegram bot token |
| `DISCORD_TOKEN` | — | Discord bot token |
| `WA_AUTH_DIR` | — | WhatsApp auth directory (set to enable) |
| `PIPELINE_REPO` | — | Primary repo path |
| `PIPELINE_TEST_CMD` | — | Test command for primary repo |
| `WATCHED_REPOS` | — | Additional repos (`path:cmd\|path:cmd`) |
| `CONTINUOUS_MODE` | `false` | Auto-seed tasks when idle |
| `MODEL` | `claude-sonnet-4-6` | Model for all agents |
| `PIPELINE_MAX_AGENTS` | `2` | Max concurrent pipeline agents |
| `MAX_CHAT_AGENTS` | `4` | Max concurrent chat agents |
| `WEB_PORT` | `3131` | Dashboard port |
| `SANDBOX_BACKEND` | `auto` | `auto`, `docker`, `bwrap`, or `none` |

## Commands

| Just | Description |
|---|---|
| `just ship` | Dashboard + test + build + install + restart |
| `just setup` | Full setup (image, sidecar, dashboard, build) |
| `just deploy` | Build + restart |
| `just t` | Run tests |
| `just b` | Build release binary |
| `just dash` | Build dashboard |

Requires Rust, Docker, Bun.

## License

Business Source License 1.1 — see [LICENSE](LICENSE). Changes to AGPL v3.0 on 2030-01-01.
