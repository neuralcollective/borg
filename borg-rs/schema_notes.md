# Schema Notes

## .env vs config table

### .env — secrets and bootstrap-only values

These must be present before the process starts (before the DB is opened) or are sensitive credentials that should never be stored in SQLite.

| Key | Reason |
|-----|--------|
| `TELEGRAM_BOT_TOKEN` | Secret |
| `DISCORD_TOKEN` | Secret |
| `ANTHROPIC_API_KEY` | Secret |
| `CLAUDE_CODE_OAUTH_TOKEN` | Secret, rotates from credentials file |
| `PIPELINE_REPO` | Needed to locate the DB file itself |
| `DATA_DIR` | Needed to locate the DB file itself |
| `WHATSAPP_AUTH_DIR` | Path to Baileys auth state; needed at sidecar start |
| `WHATSAPP_ENABLED` | Determines whether sidecar starts at all |
| `DISCORD_ENABLED` | Same |
| `PIPELINE_ADMIN_CHAT` | Used before DB is ready for startup notifications |
| `WATCHED_REPOS` | Static topology parsed at boot; repos table is authoritative at runtime |
| `DASHBOARD_DIST_DIR` | Static filesystem path |

### config table — runtime-tunable, non-secret

These can be read/written via the dashboard or API without restarting the process.

| Key | Default | Notes |
|-----|---------|-------|
| `assistant_name` | `Borg` | |
| `trigger_pattern` | `@Borg` | |
| `model` | `claude-sonnet-4-6` | Claude model slug |
| `continuous_mode` | `false` | `'true'` or `'false'` |
| `release_interval_mins` | `180` | |
| `chat_collection_window_ms` | `3000` | |
| `chat_cooldown_ms` | `5000` | |
| `agent_timeout_s` | `1000` | |
| `max_chat_agents` | `4` | |
| `chat_rate_limit` | `5` | Messages per window |
| `pipeline_max_agents` | `4` | |
| `pipeline_max_backlog` | `5` | Max concurrent tasks |
| `pipeline_seed_cooldown_s` | `3600` | Min seconds between seed scans |
| `pipeline_tick_s` | `30` | Main loop interval |
| `pipeline_proposal_threshold` | `8` | Min triage score to auto-promote |
| `remote_check_interval_s` | `300` | Git fetch interval for self-update |
| `container_image` | `borg-agent:latest` | |
| `container_memory_mb` | `1024` | |
| `container_setup` | `` | Path to setup script sourced in container |
| `web_bind` | `127.0.0.1` | |
| `web_port` | `3131` | |
| `git_author_name` | `` | |
| `git_author_email` | `` | |
| `git_committer_name` | `` | |
| `git_committer_email` | `` | |
| `git_via_borg` | `false` | `'true'` or `'false'` |
| `git_claude_coauthor` | `false` | `'true'` or `'false'` |

At startup, borg-rs seeds missing config keys from .env / defaults so the table is always complete. .env values take precedence on the first run; afterwards the config table wins unless the key is in the secrets list above.

---

## pipeline_events kind taxonomy

All `payload` values are JSON objects. Unknown keys are ignored for forward compatibility.

### Task lifecycle

| kind | payload fields | description |
|------|---------------|-------------|
| `task_created` | `title`, `description`, `mode`, `created_by` | New task inserted into backlog |
| `status_changed` | `from`, `to` | Task status transition |
| `phase_started` | `phase`, `attempt` | Agent phase begins |
| `phase_completed` | `phase`, `attempt`, `exit_code`, `duration_ms` | Agent phase finished |

### Agent output

| kind | payload fields | description |
|------|---------------|-------------|
| `agent_message` | `role`, `content` | Text message from an agent run |
| `tool_use` | `tool`, `input` | Claude tool call (truncated input) |
| `test_output` | `exit_code`, `stdout`, `stderr` | Test runner result |

### User interaction

| kind | payload fields | description |
|------|---------------|-------------|
| `user_message` | `from`, `transport`, `content` | Human message routed to a task |
| `director_message` | `content` | Director/orchestrator message to a task |

### Git / release

| kind | payload fields | description |
|------|---------------|-------------|
| `git_push` | `branch`, `repo` | Branch pushed to remote |
| `pr_created` | `pr_number`, `url`, `branch` | Pull request opened |
| `pr_merged` | `pr_number`, `branch` | PR merged to main |

### Errors

| kind | payload fields | description |
|------|---------------|-------------|
| `error` | `message`, `phase`, `attempt` | Non-fatal error during a phase |

### Example payloads

```json
// task_created
{"title": "Add rate limiting", "description": "...", "mode": "sweborg", "created_by": "alice"}

// status_changed
{"from": "spec", "to": "qa"}

// phase_completed
{"phase": "impl", "attempt": 2, "exit_code": 0, "duration_ms": 45230}

// test_output
{"exit_code": 1, "stdout": "running 14 tests\n...", "stderr": "FAILED: test_foo"}

// pr_created
{"pr_number": 42, "url": "https://github.com/org/repo/pull/42", "branch": "borg/task-17-add-rate-limiting"}

// error
{"message": "docker run failed: OOM", "phase": "impl", "attempt": 1}
```

---

## Migration strategy

All migrations are additive — no columns are dropped, no tables are renamed. The existing Zig borg process and the Rust rewrite can share the same DB file during a rolling transition.

**Order of operations:**

1. Run `001_initial.sql` on a fresh DB, or verify all tables exist on an existing DB (the Zig process already created them).
2. Run `002_repos_table.sql` to add the `repos` table and `repo_id` columns. The Zig process ignores the new columns. After running, populate `repos` from the WATCHED_REPOS / PIPELINE_REPO config, then backfill `repo_id` on existing rows:
   ```sql
   UPDATE pipeline_tasks SET repo_id = (SELECT id FROM repos WHERE path = repo_path);
   UPDATE proposals SET repo_id = (SELECT id FROM repos WHERE path = repo_path);
   ```
3. Run `003_events_table.sql` to add `pipeline_events`. The Zig process continues writing to the legacy `events` table; borg-rs writes to `pipeline_events`. Both can coexist.
4. Run `004_task_messages.sql`. The Zig process has no task_messages concept; rows only appear from borg-rs.
5. Run `005_config_table.sql`. Seed from .env defaults on first start.

**Never remove `repo_path` from `pipeline_tasks` or `proposals`** until the Zig process is fully retired and all reads have been migrated to JOIN through `repos`.

The `state` table's `schema_version` key is maintained by the Zig process. Borg-rs uses its own migration tracking (e.g., a `_migrations` table or embedded version in `state`) and does not rely on the Zig schema_version value.
