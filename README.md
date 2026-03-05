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

## Privacy & Security

Borg is engineered for **Am Law 100** and **Magic Circle** compliance standards (SOC 2, ISO 27001, GDPR).

**The "Dual-Proxy Air-Gap" Architecture:**
1.  **Air-Gapped Execution:** Agents run in containers with **zero network interfaces** (`--unshare-net`). Exfiltration to the open internet is physically impossible.
2.  **Zero-Retention Inference:** All LLM traffic is proxied to **AWS Bedrock** (contractual Zero Data Retention). Anthropic never sees or logs client data.
3.  **Zero-Retention Research:** Web search is proxied to **Brave Search Enterprise ZDR**. Queries are processed in-memory and never logged.
4.  **Envelope Encryption:** All credentials are encrypted at rest with AES-256-GCM.
5.  **Burn-After-Reading:** Mandatory 7-day purge cycle for legal tasks ensures sensitive data does not accumulate.

See [ARCHITECTURE.md](ARCHITECTURE.md#data-privacy--security-the-big-law-standard) for the full security specification.

## Pipelines

Borg is focused on two first-class workforce modes:
- `sweborg` / `swe` (software engineering)
- `lawborg` / `legal` (legal work)

Additional presets exist, but are experimental templates and not active product investment targets. They can be used as references for custom mode design.

Create custom pipelines via the mode creator in the dashboard or the API. Presets are starting points — everything is configurable.

### Success Gates (Legal + SWE)

Work should prioritize these measurable outcomes before expanding non-core domains:

- Legal quality: citation verification pass rate, first-pass human review acceptance, turnaround time, and revision rate.
- SWE reliability: task-to-merge success rate, retry/failure rate, CI pass rate, and cycle time.
- Operational stability: active task throughput, blocked-task aging, and stream/log observability coverage.

### Regulated Workflows

For jurisdiction-sensitive legal pipelines, add a `compliance_check` phase in Mode Creator.

- Profiles:
- `uk_sra` (UK SRA / Law Society oriented checks)
- `us_prof_resp` (US professional-responsibility checks)
- Enforcement:
- `warn`: record findings and continue
- `block`: move task to `pending_review` until reviewer requests revision
- Recommended placement:
- after drafting/review agent phases, before terminal `done`

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
| `EXPERIMENTAL_DOMAINS` | `false` | Enable non-core domain presets (health/web/crew/sales/data/chef/build/medwrite) |
| `MAX_CHAT_AGENTS` | `4` | Max concurrent chat agents |
| `WEB_PORT` | `3131` | Dashboard port |
| `SANDBOX_BACKEND` | `auto` | `auto`, `docker`, `bwrap`, or `none` |
| `BORG_MASTER_KEY` | — | Hex-encoded 256-bit key for DB encryption |

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
