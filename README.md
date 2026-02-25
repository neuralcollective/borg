# Borg

Autonomous Software Engineering System — runs an engineering pipeline that writes specs, generates tests, implements code, and merges PRs with zero human intervention. Ships with a real-time web dashboard for monitoring and approval. Also serves as a chat agent across Telegram, Discord, and WhatsApp.

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

## How It Works

### Chat

Mention the bot by name in a registered group and it responds using Claude Code as a subprocess. Messages are batched in a short collection window, then the agent runs with the full conversation context. Each group has its own state machine (`IDLE → COLLECTING → RUNNING → COOLDOWN`) with rate limiting.

### Pipeline

When `PIPELINE_REPO` is set, Borg runs an autonomous engineering pipeline. Tasks move through phases:

1. **Backlog** — git worktree created on a new branch
2. **Spec** — manager agent writes `spec.md` (requirements, files, acceptance criteria)
3. **QA** — QA agent writes failing tests based on the spec
4. **Impl** — worker agent implements code to pass the tests (Docker-isolated)
5. **Test** — repo test command runs; failures retry up to 5 times
6. **Done** — branch queued for integration
7. **Release** — PR created, rebased on main, merged (or held for manual review)

Each task gets its own git worktree. Impl agents run in Docker containers with `--cap-drop ALL`. Rebase agents run on the host. Sessions persist across retries via per-task session dirs.

When `CONTINUOUS_MODE=true`, the pipeline auto-seeds tasks by scanning repos for refactoring opportunities, bugs, and missing test coverage.

### Self-Update

When a merge lands on the primary repo (`is_self=true`), Borg rebuilds itself and restarts via `execve`.

### Bot Commands

| Command | Description |
|---|---|
| `/register` | Register chat for bot responses |
| `/task <title>` | Create a pipeline task |
| `/tasks` | List pipeline tasks |
| `/status` | Version, uptime, config |
| `/ping` | Health check |

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
