# Architecture Overhaul — Task Reference

> Generated 2026-03-14. Canonical task list is in Claude Code task system.
> Conversation: `docs/conversation-2026-03-14-architecture-planning.txt`

## Decisions

- **Agent SDK** primary Claude backend (replaces CLI subprocess for chat + pipeline)
- **Bedrock** primary ZDR provider; Vertex stubbed for later
- **Claude subscription auth** still supported (OAuth tokens passthrough)
- **Existing backends kept** as modular options behind `AgentBackend` trait
- **Traits everywhere** — plugin-ready extensibility across all subsystems
- **OpenAI** — keep Codex CLI backend, evaluate Rust harness later
- **Cron/scheduling** — time-triggered tasks on top of existing auto-enqueue
- **No embedded SQLite search** — Vespa handles everything
- **No in-process MCP client** — instrument servers directly, Agent SDK hooks for observability

## Backend Architecture

```
AgentBackend (trait)
├── AgentSdkBackend    — Claude via Agent SDK (primary, default)
├── ClaudeBackend      — Claude via CLI subprocess (legacy)
├── CodexBackend       — OpenAI via Codex CLI
├── ContainerBackend   — any CLI agent in Docker/bwrap (generalized)
├── GeminiBackend      — Google Gemini
└── OllamaBackend      — local models

BORG_BACKEND=agent-sdk|claude|codex|container|gemini|ollama
Selection: task.backend → repo.backend → config.backend
```

## Trait Map

| Trait | Implementations | Files |
|---|---|---|
| AgentBackend | 6 backends | borg-agent/src/*.rs |
| SearchProvider | VespaSearch | borg-server/src/search.rs, vespa.rs |
| StorageProvider | S3, Local | borg-server/src/storage.rs |
| EmbeddingProvider | Voyage API | borg-core/src/knowledge.rs |
| MessageChannel | Telegram, Discord, WhatsApp, Slack, Web | sidecar/, telegram.rs, routes/chat.rs |
| SecretStore | Plaintext, Encrypted (ChaCha20) | borg-core/src/secrets.rs |
| IngestionBackend | SQS, Local | borg-server/src/ingestion.rs |
| BackupBackend | S3, Local | borg-server/src/backup.rs |
| DocumentParser | PDF, DOCX, MD, Plain, HTML | borg-core/src/parser.rs |

## Task Dependency Graph

```
#1 Core traits ──┬── #2 StorageProvider
                 ├── #3 SearchProvider
                 ├── #4 EmbeddingProvider
                 ├── #5 MessageChannel
                 ├── #6 IngestionBackend
                 ├── #7 BackupBackend
                 ├── #8 Expand AgentBackend ──┬── #12 Migrate chat
                 ├── #19 Secret encryption    ├── #14 ContainerBackend
                 └── #26 DocumentParser       └── #18 ReliableProvider

#9 TS bridge ── #10 Rust bridge client ──┬── #11 ProviderConfig
                                         ├── #12 Migrate chat ── #23 Token streaming
                                         ├── #13 Migrate pipeline
                                         ├── #15 BORG_BACKEND config
                                         ├── #16 Hook handlers ──┬── #17 Timeline UI
                                         ├── #24 Cost tracking   │
                                         └── #25 Deploy scripts  │

#22 PluginRegistry (blocked by #1-#7)
#21 Cron/scheduling (independent)
#20 MCP instrumentation (independent)
#27 Integration tests (blocked by #10,12,13,15,18)
```
