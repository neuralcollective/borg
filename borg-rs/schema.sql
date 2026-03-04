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
  trigger_pattern TEXT NOT NULL DEFAULT '@Borg',
  added_at TEXT NOT NULL DEFAULT (datetime('now')),
  requires_trigger INTEGER NOT NULL DEFAULT 1
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
  enabled INTEGER NOT NULL DEFAULT 1
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
  started_at TEXT NOT NULL DEFAULT (datetime('now')),
  completed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_chat_runs_jid ON chat_agent_runs(jid, status);

-- ── Pipeline ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS pipeline_tasks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  repo_path TEXT NOT NULL,       -- kept for migration compat; prefer repo_id
  repo_id INTEGER REFERENCES repos(id),
  branch TEXT DEFAULT '',
  status TEXT NOT NULL DEFAULT 'backlog',
  attempt INTEGER NOT NULL DEFAULT 0,
  max_attempts INTEGER NOT NULL DEFAULT 5,
  last_error TEXT NOT NULL DEFAULT '',
  created_by TEXT NOT NULL DEFAULT '',
  notify_chat TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  mode TEXT NOT NULL DEFAULT 'sweborg',
  backend TEXT,                  -- backend that actually ran this task
  project_id INTEGER REFERENCES projects(id),
  task_type TEXT NOT NULL DEFAULT '',
  structured_data TEXT NOT NULL DEFAULT '',
  review_status TEXT,
  revision_count INTEGER NOT NULL DEFAULT 0,
  started_at TEXT,
  completed_at TEXT,
  duration_secs INTEGER,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_status ON pipeline_tasks(status);
CREATE INDEX IF NOT EXISTS idx_pipeline_repo ON pipeline_tasks(repo_path);
-- idx_pipeline_project and idx_pipeline_repo_status created in migrate()
-- after ALTER TABLE adds the columns for older databases.

-- Statuses: backlog → implement → validate → lint_fix → rebase → done → merged
--           review, pending_review (mode-specific)
--           blocked (paused, awaiting human input), failed (terminal, recyclable)

CREATE TABLE IF NOT EXISTS integration_queue (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL REFERENCES pipeline_tasks(id),
  branch TEXT NOT NULL,
  repo_path TEXT DEFAULT '',
  status TEXT DEFAULT 'queued',  -- queued | merging | merged | excluded | pending_review
  error_msg TEXT DEFAULT '',
  unknown_retries INTEGER DEFAULT 0,
  pr_number INTEGER DEFAULT 0,
  queued_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_integration_queue_status ON integration_queue(status);

CREATE TABLE IF NOT EXISTS task_outputs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL REFERENCES pipeline_tasks(id),
  phase TEXT NOT NULL,
  output TEXT NOT NULL,
  raw_stream TEXT DEFAULT '',    -- full NDJSON agent stream
  exit_code INTEGER DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
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
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_proposals_status ON proposals(status, triage_score);
CREATE INDEX IF NOT EXISTS idx_proposals_repo ON proposals(repo_path);

-- ── Projects (document workspaces) ───────────────────────────────────────

CREATE TABLE IF NOT EXISTS projects (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL,
  mode TEXT NOT NULL DEFAULT 'general',
  client_name TEXT NOT NULL DEFAULT '',
  case_number TEXT NOT NULL DEFAULT '',
  jurisdiction TEXT NOT NULL DEFAULT '',
  matter_type TEXT NOT NULL DEFAULT '',
  opposing_counsel TEXT NOT NULL DEFAULT '',
  deadline TEXT,
  privilege_level TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active',
  default_template_id INTEGER,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS project_files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  file_name TEXT NOT NULL,
  stored_path TEXT NOT NULL,
  mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
  size_bytes INTEGER NOT NULL DEFAULT 0,
  extracted_text TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_project_files_project_id ON project_files(project_id);

-- ── Parties (conflict checking) ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS parties (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  normalized_name TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'party',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_parties_project ON parties(project_id);
CREATE INDEX IF NOT EXISTS idx_parties_normalized ON parties(normalized_name);

-- ── Deadlines ────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS deadlines (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  due_date TEXT NOT NULL,
  rule_basis TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_deadlines_project ON deadlines(project_id);
CREATE INDEX IF NOT EXISTS idx_deadlines_due ON deadlines(due_date);

-- ── Unified event log ─────────────────────────────────────────────────────
-- Append-only. Never UPDATE or DELETE rows.
-- kind taxonomy and payload shapes are documented in schema_notes.md.

CREATE TABLE IF NOT EXISTS pipeline_events (
  id INTEGER PRIMARY KEY,
  task_id INTEGER REFERENCES pipeline_tasks(id),
  repo_id INTEGER REFERENCES repos(id),
  project_id INTEGER REFERENCES projects(id),
  actor TEXT NOT NULL DEFAULT '',
  kind TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_task_id ON pipeline_events(task_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_kind ON pipeline_events(kind);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_created_at ON pipeline_events(created_at);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_project ON pipeline_events(project_id);

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
CREATE UNIQUE INDEX IF NOT EXISTS idx_api_keys_owner ON api_keys(owner, provider);

-- ── Misc / legacy ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Legacy unstructured event log. New code should write to pipeline_events instead.
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

-- ── Full-text search ─────────────────────────────────────────────────────

CREATE VIRTUAL TABLE IF NOT EXISTS legal_fts USING fts5(
  project_id UNINDEXED,
  task_id UNINDEXED,
  file_path UNINDEXED,
  title,
  content,
  tokenize='porter unicode61'
);

-- Knowledge base files for agent context injection
CREATE TABLE IF NOT EXISTS knowledge_files (
  id INTEGER PRIMARY KEY,
  file_name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  size_bytes INTEGER NOT NULL DEFAULT 0,
  inline BOOLEAN NOT NULL DEFAULT 0,
  tags TEXT NOT NULL DEFAULT '',
  category TEXT NOT NULL DEFAULT 'general',
  jurisdiction TEXT NOT NULL DEFAULT '',
  project_id INTEGER,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Vector embeddings (knowledge graph) ────────────────────────────────

CREATE TABLE IF NOT EXISTS embeddings (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project_id INTEGER REFERENCES projects(id),
  task_id INTEGER REFERENCES pipeline_tasks(id),
  chunk_text TEXT NOT NULL,
  chunk_hash TEXT NOT NULL UNIQUE,
  file_path TEXT NOT NULL DEFAULT '',
  embedding BLOB NOT NULL,
  dims INTEGER NOT NULL DEFAULT 768,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_embeddings_project ON embeddings(project_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_task ON embeddings(task_id);

-- ── Citation verifications ────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS citation_verifications (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id INTEGER NOT NULL REFERENCES pipeline_tasks(id),
  citation_text TEXT NOT NULL,
  citation_type TEXT NOT NULL DEFAULT 'case',
  status TEXT NOT NULL DEFAULT 'pending',
  source TEXT NOT NULL DEFAULT '',
  treatment TEXT NOT NULL DEFAULT '',
  checked_at TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_citations_task ON citation_verifications(task_id);
