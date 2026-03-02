# Borg

Autonomous AI agent orchestrator — runs domain-specific pipelines that research, draft, implement, review, and ship work end-to-end. Built-in modes for software engineering, legal, healthcare admin, construction, sales, data analysis, and more. Ships with a real-time web dashboard and chat integration across Telegram, Discord, and WhatsApp.

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
just ship   # test, build, deploy
```

Dashboard at `http://127.0.0.1:3131`.

## Pipeline Modes

Each mode defines its own phases, system prompts, tools, and autonomous scanning presets.

| Mode | Label | Category | Description |
|---|---|---|---|
| `sweborg` | Software Engineering | Engineering | Implement → validate → lint → rebase. Docker-isolated agents. |
| `webborg` | Frontend | Engineering | Web performance, accessibility, visual polish, UX improvements. |
| `databorg` | Data Analysis | Engineering | Data quality audits, pipeline review, insight discovery. |
| `lawborg` | Legal | Professional Services | Contract analysis, case research, regulatory compliance. Integrates with CourtListener, EDGAR, Federal Register, and premium research tools (LexisNexis, Westlaw) via MCP. |
| `healthborg` | Healthcare Admin | Professional Services | Insurance appeals, prior authorization, medical bill review. Regulatory research via shared legal MCP. |
| `buildborg` | Construction | Professional Services | Permit research, contractor search, cost estimation, code compliance. Powered by Shovels permit database via MCP. |
| `salesborg` | Sales Outreach | Professional Services | Prospect research, personalised outreach drafting, follow-up sequences. |
| `crewborg` | Talent Search | People & Ops | Candidate sourcing, evaluation, and ranked shortlists. |
| `chefborg` | Recipe Dev | Creative | Recipe development and testing with nutritional analysis. |

Modes are extensible — create custom modes with your own phases, prompts, and tools via the dashboard or API.

## How It Works

### Pipeline

Tasks move through configurable phases. A typical engineering task:

1. **Backlog** — git worktree created on a new branch
2. **Implement** — agent writes code with full tool access (Read, Write, Edit, Bash, etc.)
3. **Validate** — test command runs; failures retry back to implement
4. **Lint** — auto-fix linting issues
5. **Rebase** — rebase onto main, resolve conflicts
6. **Integration** — PR created, merged (or held for manual review)

Professional services modes (legal, health, construction, sales) use an implement → review flow where an independent reviewer checks the work before completion.

Each task gets its own git worktree. Agents can run in Docker containers with `--cap-drop ALL` or in bubblewrap sandboxes. Sessions persist across retries.

### Autonomous Scanning

When `CONTINUOUS_MODE=true`, each mode's preset scanners periodically analyse repos and create tasks. Examples: security audits, dependency updates, performance issues, accessibility gaps, contract review opportunities, data quality checks.

### MCP Integrations

Domain-specific MCP (Model Context Protocol) servers extend agent capabilities:

- **Legal** — CourtListener case law, SEC EDGAR filings, Federal Register, state legislation, plus BYOK premium tools
- **Construction** — Shovels V2 permit database (170M+ permits, contractor profiles, geographic search)
- **Banking** — Plaid API (accounts, transactions, balances, identity)
- **OCR** — kreuzberg document extraction (when installed)

### Chat

Mention the bot by name in a registered Telegram/Discord/WhatsApp group. Messages are batched, then the agent runs with full conversation context. Each group has its own state machine with rate limiting.

### Self-Update

When a merge lands on the primary repo, Borg rebuilds itself and restarts automatically.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `TELEGRAM_BOT_TOKEN` | — | Telegram bot token (required) |
| `DISCORD_ENABLED` | `false` | Enable Discord bridge |
| `DISCORD_TOKEN` | — | Discord bot token |
| `WHATSAPP_ENABLED` | `false` | Enable WhatsApp bridge |
| `PIPELINE_REPO` | — | Primary repo path (enables pipeline) |
| `PIPELINE_TEST_CMD` | — | Test command for primary repo |
| `PIPELINE_AUTO_MERGE` | `true` | Auto-merge PRs for primary repo |
| `WATCHED_REPOS` | — | Additional repos: `path:cmd\|path:cmd` |
| `WEB_BIND` | `127.0.0.1` | Dashboard bind address |
| `WEB_PORT` | `3131` | Dashboard port |
| `CONTINUOUS_MODE` | `false` | Auto-seed tasks when idle |
| `CLAUDE_MODEL` | `claude-sonnet-4-6` | Model for all agents |
| `RELEASE_INTERVAL_MINS` | `180` | Min interval between integration runs |
| `MAX_CONCURRENT_AGENTS` | `4` | Global concurrent agent limit |

### Merge Modes

PRs are auto-merged by default. For manual review, set `PIPELINE_AUTO_MERGE=false` or append `!manual` to watched repo entries.

## Commands

| Just | Description |
|---|---|
| `just ship` | Test, build dashboard, deploy |
| `just t` | Run tests |
| `just b` | Build release binary |
| `just dash` | Build dashboard |
| `just setup` | Full setup (image + sidecar + dashboard + build) |

Requires Rust, Docker, Bun.

## License

Business Source License 1.1 — see [LICENSE](LICENSE). Changes to AGPL v3.0 on 2030-01-01.
