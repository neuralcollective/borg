-- Borg-rs complete Postgres schema.
-- Clean-break control-plane schema; SQLite is no longer supported.

-- ── Repos ─────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS repos (
  id BIGSERIAL PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,            -- last path component, e.g. "borg"
  mode TEXT NOT NULL DEFAULT 'sweborg',
  backend TEXT,                  -- NULL = use global default
  test_cmd TEXT NOT NULL DEFAULT '',
  prompt_file TEXT NOT NULL DEFAULT '',
  auto_merge BIGINT NOT NULL DEFAULT 1,
  repo_slug TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);

-- ── Projects (document workspaces) ───────────────────────────────────────
-- Defined before pipeline_tasks which references projects(id).

CREATE TABLE IF NOT EXISTS projects (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  mode TEXT NOT NULL DEFAULT 'general',
  repo_path TEXT NOT NULL DEFAULT '',
  client_name TEXT NOT NULL DEFAULT '',
  case_number TEXT NOT NULL DEFAULT '',
  jurisdiction TEXT NOT NULL DEFAULT '',
  matter_type TEXT NOT NULL DEFAULT '',
  opposing_counsel TEXT NOT NULL DEFAULT '',
  deadline TEXT,
  privilege_level TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active',
  default_template_id BIGINT,
  session_privileged BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);

-- ── Chat infrastructure ───────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS registered_groups (
  jid TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  folder TEXT NOT NULL UNIQUE,
  trigger_pattern TEXT NOT NULL DEFAULT '@Borg',
  added_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  requires_trigger BIGINT NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS messages (
  id TEXT NOT NULL,
  chat_jid TEXT NOT NULL,
  sender TEXT,
  sender_name TEXT,
  content TEXT NOT NULL,
  timestamp TEXT NOT NULL,
  is_from_me BIGINT DEFAULT 0,
  is_bot_message BIGINT DEFAULT 0,
  raw_stream TEXT,
  input_tokens BIGINT,
  output_tokens BIGINT,
  cost_usd DOUBLE PRECISION,
  model TEXT,
  PRIMARY KEY (chat_jid, id)
);
CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(chat_jid, timestamp);

CREATE TABLE IF NOT EXISTS sessions (
  folder TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  created_at TEXT DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);

CREATE TABLE IF NOT EXISTS scheduled_tasks (
  id BIGSERIAL PRIMARY KEY,
  chat_jid TEXT NOT NULL,
  description TEXT NOT NULL,
  cron_expr TEXT NOT NULL,
  next_run TEXT,
  last_run TEXT,
  enabled BIGINT NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS chat_agent_runs (
  id BIGSERIAL PRIMARY KEY,
  jid TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'running',
  transport TEXT DEFAULT '',
  original_id TEXT DEFAULT '',
  trigger_msg_id TEXT DEFAULT '',
  folder TEXT DEFAULT '',
  output TEXT DEFAULT '',
  new_session_id TEXT DEFAULT '',
  last_msg_timestamp TEXT DEFAULT '',
  started_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  completed_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_chat_runs_jid ON chat_agent_runs(jid, status);

-- ── Pipeline ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS pipeline_tasks (
  id BIGSERIAL PRIMARY KEY,
  title TEXT NOT NULL,
  description TEXT NOT NULL,
  repo_path TEXT NOT NULL,
  repo_id BIGINT REFERENCES repos(id),
  branch TEXT DEFAULT '',
  status TEXT NOT NULL DEFAULT 'backlog',
  attempt BIGINT NOT NULL DEFAULT 0,
  max_attempts BIGINT NOT NULL DEFAULT 5,
  last_error TEXT NOT NULL DEFAULT '',
  created_by TEXT NOT NULL DEFAULT '',
  notify_chat TEXT NOT NULL DEFAULT '',
  session_id TEXT NOT NULL DEFAULT '',
  mode TEXT NOT NULL DEFAULT 'sweborg',
  backend TEXT,
  project_id BIGINT REFERENCES projects(id),
  task_type TEXT NOT NULL DEFAULT '',
  requires_exhaustive_corpus_review BIGINT NOT NULL DEFAULT 0,
  structured_data TEXT NOT NULL DEFAULT '',
  review_status TEXT,
  revision_count BIGINT NOT NULL DEFAULT 0,
  chat_thread TEXT NOT NULL DEFAULT '',
  started_at TEXT,
  completed_at TEXT,
  duration_secs BIGINT,
  total_input_tokens BIGINT DEFAULT 0,
  total_output_tokens BIGINT DEFAULT 0,
  total_cost_usd DOUBLE PRECISION DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  updated_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_status ON pipeline_tasks(status);
CREATE INDEX IF NOT EXISTS idx_pipeline_repo ON pipeline_tasks(repo_path);
CREATE INDEX IF NOT EXISTS idx_pipeline_project ON pipeline_tasks(project_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_repo_status ON pipeline_tasks(repo_id, status);

-- Statuses: backlog → implement → validate → lint_fix → rebase → done → merged
--           review, pending_review (mode-specific)
--           blocked (paused, awaiting human input), failed (terminal, recyclable)

CREATE TABLE IF NOT EXISTS integration_queue (
  id BIGSERIAL PRIMARY KEY,
  task_id BIGINT NOT NULL REFERENCES pipeline_tasks(id),
  branch TEXT NOT NULL,
  repo_path TEXT DEFAULT '',
  status TEXT DEFAULT 'queued',  -- queued | merging | merged | excluded | pending_review
  error_msg TEXT DEFAULT '',
  unknown_retries BIGINT DEFAULT 0,
  pr_number BIGINT DEFAULT 0,
  queued_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_integration_queue_status ON integration_queue(status);

CREATE TABLE IF NOT EXISTS task_outputs (
  id BIGSERIAL PRIMARY KEY,
  task_id BIGINT NOT NULL REFERENCES pipeline_tasks(id),
  phase TEXT NOT NULL,
  output TEXT NOT NULL,
  raw_stream TEXT DEFAULT '',    -- full NDJSON agent stream
  exit_code BIGINT DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_task_outputs_task ON task_outputs(task_id);

-- ── Proposals ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS proposals (
  id BIGSERIAL PRIMARY KEY,
  repo_path TEXT NOT NULL,
  repo_id BIGINT REFERENCES repos(id),
  title TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  rationale TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'proposed',  -- proposed | approved | dismissed
  triage_score BIGINT DEFAULT 0,
  triage_impact BIGINT DEFAULT 0,
  triage_feasibility BIGINT DEFAULT 0,
  triage_risk BIGINT DEFAULT 0,
  triage_effort BIGINT DEFAULT 0,
  triage_reasoning TEXT DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_proposals_status ON proposals(status, triage_score);
CREATE INDEX IF NOT EXISTS idx_proposals_repo ON proposals(repo_path);

-- ── Project files ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS project_files (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  file_name TEXT NOT NULL,
  source_path TEXT NOT NULL DEFAULT '',
  stored_path TEXT NOT NULL,
  mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
  size_bytes BIGINT NOT NULL DEFAULT 0,
  extracted_text TEXT NOT NULL DEFAULT '',
  content_hash TEXT NOT NULL DEFAULT '',
  privileged BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_project_files_project_id ON project_files(project_id);
CREATE INDEX IF NOT EXISTS idx_project_files_project_name ON project_files(project_id, file_name);
CREATE INDEX IF NOT EXISTS idx_project_files_project_source_path ON project_files(project_id, source_path);
CREATE INDEX IF NOT EXISTS idx_project_files_project_name_id ON project_files(project_id, file_name, id DESC);
CREATE INDEX IF NOT EXISTS idx_project_files_project_source_path_id ON project_files(project_id, source_path, id DESC);
CREATE INDEX IF NOT EXISTS idx_project_files_project_hash ON project_files(project_id, content_hash);
CREATE INDEX IF NOT EXISTS idx_project_files_project_created ON project_files(project_id, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_project_files_created_global ON project_files(created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_project_files_project_priv_created ON project_files(project_id, privileged, created_at DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_project_files_project_text_created ON project_files(project_id, created_at DESC, id DESC) WHERE extracted_text != '';

CREATE TABLE IF NOT EXISTS project_corpus_stats (
  project_id BIGINT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
  total_files BIGINT NOT NULL DEFAULT 0,
  total_bytes BIGINT NOT NULL DEFAULT 0,
  privileged_files BIGINT NOT NULL DEFAULT 0,
  text_files BIGINT NOT NULL DEFAULT 0,
  text_chars BIGINT NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);

-- Durable resumable uploads for large file and zip ingestion.
CREATE TABLE IF NOT EXISTS upload_sessions (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  file_name TEXT NOT NULL,
  mime_type TEXT NOT NULL DEFAULT 'application/octet-stream',
  file_size BIGINT NOT NULL DEFAULT 0,
  chunk_size BIGINT NOT NULL DEFAULT 0,
  total_chunks BIGINT NOT NULL DEFAULT 0,
  uploaded_bytes BIGINT NOT NULL DEFAULT 0,
  is_zip BIGINT NOT NULL DEFAULT 0,
  privileged BIGINT NOT NULL DEFAULT 0,
  status TEXT NOT NULL DEFAULT 'uploading', -- uploading | processing | done | failed
  stored_path TEXT NOT NULL DEFAULT '',
  error TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  updated_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_upload_sessions_project ON upload_sessions(project_id, status, id);

CREATE TABLE IF NOT EXISTS upload_session_chunks (
  session_id BIGINT NOT NULL REFERENCES upload_sessions(id) ON DELETE CASCADE,
  chunk_index BIGINT NOT NULL,
  size_bytes BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  PRIMARY KEY(session_id, chunk_index)
);
-- ── Parties (conflict checking) ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS parties (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  normalized_name TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'party',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_parties_project ON parties(project_id);
CREATE INDEX IF NOT EXISTS idx_parties_normalized ON parties(normalized_name);

-- ── Deadlines ────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS deadlines (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  due_date TEXT NOT NULL,
  rule_basis TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_deadlines_project ON deadlines(project_id);
CREATE INDEX IF NOT EXISTS idx_deadlines_due ON deadlines(due_date);

-- ── Unified event log ─────────────────────────────────────────────────────
-- Append-only. Never UPDATE or DELETE rows.

CREATE TABLE IF NOT EXISTS pipeline_events (
  id BIGSERIAL PRIMARY KEY,
  task_id BIGINT REFERENCES pipeline_tasks(id),
  repo_id BIGINT REFERENCES repos(id),
  project_id BIGINT REFERENCES projects(id),
  actor TEXT NOT NULL DEFAULT '',
  kind TEXT NOT NULL,
  payload TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_task_id ON pipeline_events(task_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_kind ON pipeline_events(kind);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_created_at ON pipeline_events(created_at);
CREATE INDEX IF NOT EXISTS idx_pipeline_events_project ON pipeline_events(project_id);

-- ── Per-task chat ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS task_messages (
  id BIGSERIAL PRIMARY KEY,
  task_id BIGINT NOT NULL REFERENCES pipeline_tasks(id),
  role TEXT NOT NULL,            -- 'user' | 'director' | 'system'
  content TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  delivered_phase TEXT           -- NULL = not yet delivered to any agent phase
);
CREATE INDEX IF NOT EXISTS idx_task_messages_task_id ON task_messages(task_id);

-- ── Users ────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS users (
  id BIGSERIAL PRIMARY KEY,
  username TEXT UNIQUE NOT NULL,
  display_name TEXT NOT NULL DEFAULT '',
  password_hash TEXT NOT NULL,
  is_admin BOOLEAN NOT NULL DEFAULT FALSE,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);

CREATE TABLE IF NOT EXISTS user_settings (
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY (user_id, key)
);

-- ── Workspaces / tenancy ────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS workspaces (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  slug TEXT NOT NULL DEFAULT '',
  kind TEXT NOT NULL DEFAULT 'personal', -- personal | org | shared | system
  owner_user_id BIGINT REFERENCES users(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_workspaces_owner ON workspaces(owner_user_id, kind);

CREATE TABLE IF NOT EXISTS workspace_memberships (
  workspace_id BIGINT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL DEFAULT 'member', -- owner | admin | member | viewer
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_workspace_memberships_user ON workspace_memberships(user_id, workspace_id);

-- ── Runtime config ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS config (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);

-- ── API keys (BYOK) ──────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS api_keys (
  id BIGSERIAL PRIMARY KEY,
  owner TEXT NOT NULL DEFAULT 'global',
  provider TEXT NOT NULL,
  key_name TEXT NOT NULL DEFAULT '',
  key_value TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_api_keys_owner ON api_keys(owner, provider);

-- ── Linked consumer credentials ──────────────────────────────────────────

CREATE TABLE IF NOT EXISTS linked_credentials (
  id BIGSERIAL PRIMARY KEY,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  provider TEXT NOT NULL,
  auth_kind TEXT NOT NULL DEFAULT '',
  account_email TEXT NOT NULL DEFAULT '',
  account_label TEXT NOT NULL DEFAULT '',
  credential_bundle TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'disconnected',
  expires_at TEXT NOT NULL DEFAULT '',
  last_validated_at TEXT NOT NULL DEFAULT '',
  last_used_at TEXT NOT NULL DEFAULT '',
  last_error TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  updated_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_linked_credentials_user_provider
  ON linked_credentials(user_id, provider);
CREATE INDEX IF NOT EXISTS idx_linked_credentials_status
  ON linked_credentials(status, provider);

-- ── Cloud storage connections ───────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cloud_connections (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  provider TEXT NOT NULL, -- dropbox | google_drive | onedrive
  access_token TEXT NOT NULL DEFAULT '',
  refresh_token TEXT NOT NULL DEFAULT '',
  token_expiry TEXT NOT NULL DEFAULT '',
  account_email TEXT NOT NULL DEFAULT '',
  account_id TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_cloud_connections_project ON cloud_connections(project_id);

-- ── Misc / legacy ─────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS state (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- Legacy unstructured event log.
CREATE TABLE IF NOT EXISTS events (
  id BIGSERIAL PRIMARY KEY,
  ts BIGINT NOT NULL,
  level TEXT NOT NULL DEFAULT 'info',
  category TEXT NOT NULL DEFAULT 'system',
  message TEXT NOT NULL,
  metadata TEXT DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON events(ts);
CREATE INDEX IF NOT EXISTS idx_events_category ON events(category, ts);

-- ── Full-text search ─────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS legal_fts (
  project_id BIGINT NOT NULL,
  task_id BIGINT NOT NULL,
  file_path TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  content TEXT NOT NULL DEFAULT '',
  search_vector tsvector GENERATED ALWAYS AS (
    to_tsvector('english', coalesce(title, '') || ' ' || coalesce(content, ''))
  ) STORED
);
CREATE INDEX IF NOT EXISTS idx_legal_fts_task_path ON legal_fts(task_id, file_path);
CREATE INDEX IF NOT EXISTS idx_legal_fts_project ON legal_fts(project_id, task_id);
CREATE INDEX IF NOT EXISTS idx_legal_fts_search ON legal_fts USING GIN(search_vector);

-- Knowledge base files for agent context injection
CREATE TABLE IF NOT EXISTS knowledge_files (
  id BIGSERIAL PRIMARY KEY,
  file_name TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  size_bytes BIGINT NOT NULL DEFAULT 0,
  "inline" BIGINT NOT NULL DEFAULT 0,
  tags TEXT NOT NULL DEFAULT '',
  category TEXT NOT NULL DEFAULT 'general',
  jurisdiction TEXT NOT NULL DEFAULT '',
  project_id BIGINT,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_knowledge_files_category_created ON knowledge_files(category, jurisdiction, created_at);

-- Git repos always cloned and available to agents
CREATE TABLE IF NOT EXISTS knowledge_repos (
  id BIGSERIAL PRIMARY KEY,
  workspace_id BIGINT REFERENCES workspaces(id),
  user_id BIGINT REFERENCES users(id),
  url TEXT NOT NULL,
  name TEXT NOT NULL DEFAULT '',
  local_path TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'pending',
  error_msg TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_knowledge_repos_workspace ON knowledge_repos(workspace_id);
CREATE INDEX IF NOT EXISTS idx_knowledge_repos_user ON knowledge_repos(user_id, workspace_id);

-- ── Vector embeddings (knowledge graph) ────────────────────────────────

CREATE TABLE IF NOT EXISTS embeddings (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT REFERENCES projects(id),
  task_id BIGINT REFERENCES pipeline_tasks(id),
  chunk_text TEXT NOT NULL,
  chunk_hash TEXT NOT NULL UNIQUE,
  file_path TEXT NOT NULL DEFAULT '',
  embedding BYTEA NOT NULL,
  dims BIGINT NOT NULL DEFAULT 768,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_embeddings_project ON embeddings(project_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_task ON embeddings(task_id);

-- ── Citation verifications ────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS citation_verifications (
  id BIGSERIAL PRIMARY KEY,
  task_id BIGINT NOT NULL REFERENCES pipeline_tasks(id),
  citation_text TEXT NOT NULL,
  citation_type TEXT NOT NULL DEFAULT 'case',
  status TEXT NOT NULL DEFAULT 'pending',
  source TEXT NOT NULL DEFAULT '',
  treatment TEXT NOT NULL DEFAULT '',
  checked_at TEXT,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_citations_task ON citation_verifications(task_id);

-- ── Migrations ────────────────────────────────────────────────────────
DO $$ BEGIN
  ALTER TABLE messages ADD COLUMN raw_stream TEXT;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE pipeline_tasks ADD COLUMN chat_thread TEXT NOT NULL DEFAULT '';
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE pipeline_tasks ADD COLUMN requires_exhaustive_corpus_review BIGINT NOT NULL DEFAULT 0;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE users ADD COLUMN default_workspace_id BIGINT REFERENCES workspaces(id);
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE projects ADD COLUMN workspace_id BIGINT REFERENCES workspaces(id);
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE pipeline_tasks ADD COLUMN workspace_id BIGINT REFERENCES workspaces(id);
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE knowledge_files ADD COLUMN workspace_id BIGINT REFERENCES workspaces(id);
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE api_keys ADD COLUMN workspace_id BIGINT REFERENCES workspaces(id);
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

CREATE INDEX IF NOT EXISTS idx_projects_workspace ON projects(workspace_id, id DESC);
CREATE INDEX IF NOT EXISTS idx_pipeline_tasks_workspace ON pipeline_tasks(workspace_id, id DESC);
CREATE INDEX IF NOT EXISTS idx_knowledge_files_workspace ON knowledge_files(workspace_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_api_keys_workspace ON api_keys(workspace_id, provider);

DO $$ BEGIN
  ALTER TABLE knowledge_files ADD COLUMN user_id BIGINT REFERENCES users(id);
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

CREATE INDEX IF NOT EXISTS idx_knowledge_files_user ON knowledge_files(user_id, workspace_id, created_at DESC);

-- ── Project sharing ─────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS project_shares (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  role TEXT NOT NULL DEFAULT 'viewer',  -- owner | editor | viewer
  granted_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_project_shares_project_user ON project_shares(project_id, user_id);
CREATE INDEX IF NOT EXISTS idx_project_shares_user ON project_shares(user_id, project_id);

CREATE TABLE IF NOT EXISTS project_share_links (
  id BIGSERIAL PRIMARY KEY,
  project_id BIGINT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  token TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL DEFAULT '',
  expires_at TEXT NOT NULL,
  created_by BIGINT REFERENCES users(id) ON DELETE SET NULL,
  revoked BIGINT NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_project_share_links_token ON project_share_links(token);
CREATE INDEX IF NOT EXISTS idx_project_share_links_project ON project_share_links(project_id);

-- ── Cron scheduling ─────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cron_jobs (
  id BIGSERIAL PRIMARY KEY,
  name TEXT NOT NULL,
  schedule TEXT NOT NULL,
  job_type TEXT NOT NULL DEFAULT 'agent_task',
  config TEXT NOT NULL DEFAULT '{}',
  project_id BIGINT REFERENCES projects(id) ON DELETE SET NULL,
  enabled BIGINT NOT NULL DEFAULT 1,
  last_run TEXT,
  next_run TEXT,
  created_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS'))
);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_next_run ON cron_jobs(next_run) WHERE enabled = 1;

CREATE TABLE IF NOT EXISTS cron_runs (
  id BIGSERIAL PRIMARY KEY,
  job_id BIGINT NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
  started_at TEXT NOT NULL DEFAULT (to_char(timezone('UTC', now()), 'YYYY-MM-DD HH24:MI:SS')),
  finished_at TEXT,
  status TEXT NOT NULL DEFAULT 'running',
  result TEXT,
  error TEXT,
  task_id BIGINT
);
CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs(job_id, started_at DESC);

-- ── Cost tracking migrations ────────────────────────────────────────────
DO $$ BEGIN
  ALTER TABLE messages ADD COLUMN input_tokens BIGINT;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE messages ADD COLUMN output_tokens BIGINT;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE messages ADD COLUMN cost_usd DOUBLE PRECISION;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE messages ADD COLUMN model TEXT;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE pipeline_tasks ADD COLUMN total_input_tokens BIGINT DEFAULT 0;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE pipeline_tasks ADD COLUMN total_output_tokens BIGINT DEFAULT 0;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;

DO $$ BEGIN
  ALTER TABLE pipeline_tasks ADD COLUMN total_cost_usd DOUBLE PRECISION DEFAULT 0;
EXCEPTION WHEN duplicate_column THEN NULL;
END $$;
