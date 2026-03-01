# Borg Roadmap

Feature opportunities and remaining refactors. Fixed items from prior audits have been removed.

## Refactors

### Rust

- **Split `pipeline.rs`** (~2,700 lines): Extract dispatcher, phases, seeding, integration queue, and health/self-update into separate modules.
- **Split `routes.rs`** (~1,700 lines): Extract into `routes/tasks.rs`, `routes/proposals.rs`, `routes/stream.rs`, `routes/chat.rs`, `routes/settings.rs`.
- **Consolidate `db.rs` row mappers**: 20+ near-identical `row_to_*` functions. Use a macro or generic mapper.
- **Unify chat logic across transports**: Telegram, Discord, and WhatsApp handlers duplicate collection windows, agent dispatch, and session management. Single `ChatAgent` abstraction.
- **Add `.context()` to bare `?` operators**: Many error propagations lose context.
- **Move blocking git operations to `spawn_blocking`**: All `run_git` calls use `std::process::Command` in async context, starving Tokio threads under load.

### Dashboard

- **Extract shared `useEventSource()` hook**: Multiple SSE implementations across components. Centralize with reconnection, cleanup, backoff.
- **Deduplicate `MessageBubble`**: 3 copies across components.

### Sidecar

- **Refactor `lawborg-mcp` handleTool** (~570 lines): Massive switch with duplicated param-building. Extract helpers.
- **Shared MCP utilities**: Extract `authedFetch()`, `buildQueryString()`, `requireApiKey()` for reuse across MCP servers.

## New Feature Opportunities

### Pipeline & Backend

- **Task priorities & scheduling**: Urgent/normal/low levels, scheduled tasks, dependency chains.
- **Cost tracking**: Per-task token usage, cost estimates, budget thresholds ("abort if >$50").
- **Structured logging**: `tracing::info!(task_id, phase, duration_ms, ...)` for ELK/Datadog integration.
- **Prometheus `/metrics` endpoint**: Task counts, phase durations, active agents.
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
