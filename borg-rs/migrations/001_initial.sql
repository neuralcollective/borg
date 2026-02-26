-- Initial schema mirrored from the Zig borg db.zig migrate() function.
-- All tables use CREATE TABLE IF NOT EXISTS so this is safe to re-run.

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

CREATE TABLE IF NOT EXISTS state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS pipeline_tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  repo_path TEXT NOT NULL,
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
  created_at TEXT DEFAULT (datetime('now')),
  updated_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_status ON pipeline_tasks(status);

-- Named integration_queue in code; pipeline_queue is an alias used in docs.
CREATE TABLE IF NOT EXISTS integration_queue (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL,
  branch TEXT NOT NULL,
  repo_path TEXT DEFAULT '',
  status TEXT DEFAULT 'queued',
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
  raw_stream TEXT DEFAULT '',
  exit_code INTEGER DEFAULT 0,
  created_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_outputs_task ON task_outputs(task_id);

CREATE TABLE IF NOT EXISTS proposals (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  repo_path TEXT NOT NULL,
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  rationale TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'proposed',
  triage_score INTEGER DEFAULT 0,
  triage_impact INTEGER DEFAULT 0,
  triage_feasibility INTEGER DEFAULT 0,
  triage_risk INTEGER DEFAULT 0,
  triage_effort INTEGER DEFAULT 0,
  triage_reasoning TEXT DEFAULT '',
  created_at TEXT DEFAULT (datetime('now'))
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

-- Legacy system event log (level/category model).
-- Superseded by the new structured events table in migration 003.
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
