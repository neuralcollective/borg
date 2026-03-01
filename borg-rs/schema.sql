-- Borg-rs complete SQLite schema.
-- Applied incrementally via migrations/001..005; this file is the
-- canonical single-file view of the fully-migrated state.

-- ── Repos ─────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS repos (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,            -- last path component, e.g. "borg"
  mode TEXT NOT NULL DEFAULT 'sweborg',
  backend TEXT,                  -- NULL = use global default
  test_cmd TEXT NOT NULL DEFAULT '',
  prompt_file TEXT NOT NULL DEFAULT '',
  auto_merge INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Chat infrastructure ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS registered_groups (
  jid TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  folder TEXT NOT NULL UNIQUE,
  trigger_pattern TEXT DEFAULT '@Borg',
  added_at TEXT DEFAULT (datetime('now')),
  requires_trigger INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS messages (
  id TEXT NOT NULL,
  chat_jid TEXT NOT NULL,
  sender TEXT,
  sender_name TEXT,
  content TEXT NOT NULL,
  timestamp TEXT NOT NULL,
  is_from_me INTEGER DEFAULT 0,
  is_bot_message INTEGER DEFAULT 0,
  PRIMARY KEY (chat_jid, id)
);
CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(chat_jid, timestamp);

CREATE TABLE IF NOT EXISTS sessions (
  folder TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS scheduled_tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  chat_jid TEXT NOT NULL,
  description TEXT NOT NULL,
  cron_expr TEXT NOT NULL,
  next_run TEXT,
  last_run TEXT,
  enabled INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS chat_agent_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  jid TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'running',
  transport TEXT DEFAULT '',
  original_id TEXT DEFAULT '',
  trigger_msg_id TEXT DEFAULT '',
  folder TEXT DEFAULT '',
  output TEXT DEFAULT '',
  new_session_id TEXT DEFAULT '',
  last_msg_timestamp TEXT DEFAULT '',
  started_at TEXT DEFAULT (datetime('now')),
  completed_at TEXT
);

-- ── Pipeline ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS pipeline_tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  repo_path TEXT NOT NULL,       -- kept for migration compat; prefer repo_id
  repo_id INTEGER REFERENCES repos(id),
  branch TEXT DEFAULT '',
  status TEXT NOT NULL DEFAULT 'backlog',
  attempt INTEGER DEFAULT 0,
  max_attempts INTEGER DEFAULT 5,
  last_error TEXT DEFAULT '',
  created_by TEXT DEFAULT '',
  notify_chat TEXT DEFAULT '',
  session_id TEXT DEFAULT '',
  dispatched_at TEXT DEFAULT '',
  mode TEXT DEFAULT 'sweborg',
  backend TEXT,                  -- backend that actually ran this task
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_status ON pipeline_tasks(status);

-- Statuses: backlog → implement → validate → lint_fix → rebase → done → merged
--           blocked (paused, awaiting human input)
--           failed (terminal, recyclable)

CREATE TABLE IF NOT EXISTS integration_queue (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL,
  branch TEXT NOT NULL,
  repo_path TEXT DEFAULT '',
  status TEXT DEFAULT 'queued',  -- queued | merging | merged | excluded
  error_msg TEXT DEFAULT '',
  unknown_retries INTEGER DEFAULT 0,
  pr_number INTEGER DEFAULT 0,
  queued_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS task_outputs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL,
  phase TEXT NOT NULL,
  output TEXT NOT NULL,
  raw_stream TEXT DEFAULT '',    -- full NDJSON agent stream
  exit_code INTEGER DEFAULT 0,
  created_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_outputs_task ON task_outputs(task_id);

-- ── Proposals ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS proposals (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_path TEXT NOT NULL,       -- kept for migration compat; prefer repo_id
  repo_id INTEGER REFERENCES repos(id),
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  rationale TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'proposed',  -- proposed | approved | dismissed
  triage_score INTEGER DEFAULT 0,
  triage_impact INTEGER DEFAULT 0,
  triage_feasibility INTEGER DEFAULT 0,
  triage_risk INTEGER DEFAULT 0,
  triage_effort INTEGER DEFAULT 0,
  triage_reasoning TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now'))
);

-- ── Projects (document workspaces) ───────────────────────────────────────

CREATE TABLE IF NOT EXISTS projects (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  mode TEXT NOT NULL DEFAULT 'general',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS project_files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  file_name TEXT NOT NULL,
  stored_path TEXT NOT NULL,
  mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
  size_bytes INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_project_files_project_id ON project_files(project_id);

-- ── Unified event log ─────────────────────────────────────────────────────
-- Append-only. Never UPDATE or DELETE rows.
-- kind taxonomy and payload shapes are documented in schema_notes.md.

CREATE TABLE IF NOT EXISTS pipeline_events (
  id INTEGER PRIMARY KEY,
  task_id INTEGER REFERENCES pipeline_tasks(id),
  repo_id INTEGER REFERENCES repos(id),
  kind TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_task_id ON pipeline_events(task_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_kind ON pipeline_events(kind);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_created_at ON pipeline_events(created_at);

-- ── Per-task chat ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS task_messages (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES pipeline_tasks(id),
  role TEXT NOT NULL,            -- 'user' | 'director' | 'system'
  content TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  delivered_phase TEXT           -- NULL = not yet delivered to any agent phase
);
CREATE INDEX IF NOT EXISTS idx_task_messages_task_id ON task_messages(task_id);

-- ── Runtime config ────────────────────────────────────────────────────────
-- Non-secret, runtime-tunable settings. See schema_notes.md for full key list.

CREATE TABLE IF NOT EXISTS config (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── API keys (BYOK) ──────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS api_keys (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  owner TEXT NOT NULL DEFAULT 'global',   -- chat_key, org name, or 'global'
  provider TEXT NOT NULL,                 -- e.g. 'lexisnexis', 'lexmachina', 'intelligize'
  key_name TEXT NOT NULL DEFAULT '',      -- human label for the key
  key_value TEXT NOT NULL,                -- the actual API key / token
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_api_keys_owner ON api_keys(owner, provider);

-- ── Misc / legacy ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Legacy unstructured event log. Still written by the Zig borg process.
-- New code should write to pipeline_events instead.
CREATE TABLE IF NOT EXISTS events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  level TEXT NOT NULL DEFAULT 'info',
  category TEXT NOT NULL DEFAULT 'system',
  message TEXT NOT NULL,
  metadata TEXT DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_category ON events(category, ts);
