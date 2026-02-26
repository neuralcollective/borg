-- Runtime-editable config. Secrets and bootstrap paths stay in .env.
-- Keys that belong here are listed in schema_notes.md.
--
-- Config keys stored here (non-secret, runtime-tunable):
--   assistant_name
--   trigger_pattern
--   model                        (claude model slug)
--   continuous_mode              ('true'|'false')
--   release_interval_mins
--   chat_collection_window_ms
--   chat_cooldown_ms
--   agent_timeout_s
--   max_chat_agents
--   chat_rate_limit
--   pipeline_max_agents
--   pipeline_max_backlog
--   pipeline_seed_cooldown_s
--   pipeline_tick_s
--   pipeline_proposal_threshold
--   remote_check_interval_s
--   container_image
--   container_memory_mb
--   container_setup              (path to setup script)
--   web_bind
--   web_port
--   git_author_name
--   git_author_email
--   git_committer_name
--   git_committer_email
--   git_via_borg                 ('true'|'false')
--   git_claude_coauthor          ('true'|'false')
--
-- Keys that stay in .env (secrets / paths needed before DB is open):
--   TELEGRAM_BOT_TOKEN
--   DISCORD_TOKEN
--   ANTHROPIC_API_KEY
--   CLAUDE_CODE_OAUTH_TOKEN
--   PIPELINE_REPO                (needed to locate DB file itself)
--   DATA_DIR
--   WHATSAPP_AUTH_DIR
--   WHATSAPP_ENABLED
--   DISCORD_ENABLED
--   PIPELINE_ADMIN_CHAT
--   WATCHED_REPOS                (static topology; repos table is authoritative at runtime)
--   DASHBOARD_DIST_DIR

CREATE TABLE IF NOT EXISTS config (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
