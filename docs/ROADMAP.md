# Borg Roadmap

Audit findings and feature opportunities across the Rust backend, web dashboard, and sidecar.

## Critical Fixes

- **Env var leakage in sidecar**: `bridge.js` passes all `process.env` to Claude subprocess, exposing Discord tokens, API keys, and DB credentials. Whitelist only essential vars.
- **API keys in query strings**: `lawborg-mcp` passes keys as `?api_key=` for regulations.gov, Congress.gov, CanLII. Move to `Authorization` headers.
- **Container entrypoint masking errors**: `|| true` defeats `set -e`; input JSON written world-readable to `/tmp`; `WORKDIR` not validated.

## High Priority Refactors

### Rust

- **Split `pipeline.rs`** (2,673 lines): Extract dispatcher, phases, seeding, integration queue, and health/self-update into separate modules.
- **Split `routes.rs`** (1,685 lines): Extract into `routes/tasks.rs`, `routes/proposals.rs`, `routes/stream.rs`, `routes/chat.rs`, `routes/settings.rs`.
- **Consolidate `db.rs` row mappers**: 20+ near-identical `row_to_*` functions. Use a macro or generic mapper.
- **Unify chat logic across transports**: Telegram, Discord, and WhatsApp handlers duplicate collection windows, agent dispatch, and session management. Single `ChatAgent` abstraction.
- **Fix unwrap/panic in production paths**: `main.rs` startup panic, `sidecar.rs` test panics leaking into parsing, `observer.rs` unwraps on tempfile ops.
- **Fix `ChatCollector` race condition**: Running counter incremented before `mark_dispatched()` creates a message-drop window.
- **Add `.context()` to bare `?` operators**: Many error propagations lose context.

### Dashboard

- **Extract shared `useEventSource()` hook**: 5 separate SSE implementations across components. Centralize with reconnection, cleanup, backoff.
- **Deduplicate utilities**: `formatToolInput()` (2 copies), `parseStream()`/`parseEvents()` (2 copies), `MessageBubble` (3 copies).
- **Fix chat-panel race condition**: Thread change during in-flight fetch displays wrong messages. Add `AbortController`.
- **Fix infinite SSE reconnect**: No max retries or backoff on connection error.
- **Centralize constants**: Magic numbers (500 max logs, 2000 max events, polling intervals) scattered across files.

### Sidecar

- **Refactor `lawborg-mcp` handleTool** (570 lines): Massive switch with duplicated param-building. Extract helpers to cut ~400 lines.
- **Add exponential backoff to WhatsApp reconnection**: Currently hardcoded 3s retry forever.
- **Fix silent JSON parse error in agent handler**: Bare `catch {}` loses session IDs.
- **Add rate limiting**: No throttling on legal API calls; can exhaust quotas.
- **Fix Dockerfile permissions**: `chmod o+x /root` and `chmod -R o+rx /root/.bun` overly permissive.

## New Feature Opportunities

### Pipeline & Backend

- **Task priorities & scheduling**: Urgent/normal/low levels, scheduled tasks, dependency chains.
- **Cost tracking**: Per-task token usage, cost estimates, budget thresholds ("abort if >$50").
- **Structured logging**: `tracing::info!(task_id, phase, duration_ms, ...)` for ELK/Datadog integration.
- **Prometheus `/metrics` endpoint**: Task counts, phase durations, active agents. Quick win (~2 hours).
- **Audit trail**: Immutable log of every status change, user action, agent output per task.
- **Task templates**: Reusable workflows ("create task from template 'add-docs'").
- **Agent plugin system**: Register custom backends via trait (local LLMs, proprietary APIs).
- **Integration tests**: No end-to-end coverage currently. Task lifecycle, chat dispatch, concurrent scheduling.

### Dashboard

- **Command palette** (`Cmd+K`): Keyboard-driven navigation and search.
- **Analytics view**: Success/failure rates over time, backend performance, per-repo trends.
- **Multi-dimensional filtering**: Status, mode, date range, error patterns (only repo filter exists).
- **Task history**: Browsable archive with pagination (completed tasks disappear quickly).
- **Bulk actions**: Multi-select retry, reassign, priority change.
- **Phase timing**: Elapsed time per phase, click phase badge to jump to relevant logs.
- **Mobile improvements**: Expand nav tabs, increase font sizes, test on real devices.

### Sidecar

- **Health check endpoint**: HTTP server returning Discord/WhatsApp/agent status for systemd probes.
- **Graceful shutdown**: SIGTERM handler that kills agent sessions, disconnects cleanly.
- **Input validation**: Document IDs passed directly into URLs without sanitization.
- **Shared MCP utilities**: Extract `authedFetch()`, `buildQueryString()`, `requireApiKey()` for reuse across MCP servers.
