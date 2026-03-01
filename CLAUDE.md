# CLAUDE.md

## Agent Behavior (IMPORTANT)

- MUST default to `model: "sonnet"` for all subagents. Only use Haiku for trivial lookups.
- MUST NOT enter plan mode without explicitly asking the user first. Default to task lists.

## Overview

Borg is an autonomous AI agent orchestrator written in Rust. It connects to Telegram, WhatsApp, and Discord to respond to chat messages (via Claude Code subprocess), and runs an engineering pipeline that autonomously creates, tests, and merges code changes.

## Project Structure

```
borg-rs/                # Rust implementation (active codebase)
  crates/
    borg-core/          # Pipeline, DB, config, agent traits, modes
    borg-agent/         # Claude + Ollama agent backends
    borg-server/        # Axum HTTP server, routes, logging
    borg-domains/       # Domain-specific pipeline modes (swe, legal, web, crew, sales, data)
container/
  Dockerfile            # Pipeline agent image (bun + claude CLI)
  entrypoint.sh         # Agent entrypoint: parses JSON input, runs claude
dashboard/              # React + Vite + Tailwind web dashboard
sidecar/                # Unified Discord+WhatsApp bridge (bun, discord.js + Baileys)
```

## Build & Test

```bash
just t                 # Run all unit tests
just b                 # Build release binary
just deploy            # Build and restart service
just dash              # Build dashboard
just setup             # Full setup (image + sidecar + dashboard + build)
```

Use `just install-service` + `just restart` (user systemd), not `sudo systemctl`.

## Code Style

- No slop comment prefixes (`AUDIT:`, `SECURITY:`, `NOTE:`). `TODO:` is fine.
- Use `bun` (not `npm`) for JS dependencies.

## Git Commits

- Co-author: `Co-Authored-By: Sasha Duke <sashadanielduke@gmail.com>`
- Do NOT add Claude/Anthropic co-authorship lines.
