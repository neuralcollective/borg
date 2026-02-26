-- Add repos table and repo_id foreign keys to pipeline_tasks and proposals.
-- repo_path columns are kept for backward compatibility with existing rows.

CREATE TABLE IF NOT EXISTS repos (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  name TEXT NOT NULL,
  mode TEXT NOT NULL DEFAULT 'sweborg',
  backend TEXT,          -- NULL = use global default backend
  test_cmd TEXT NOT NULL DEFAULT '',
  prompt_file TEXT NOT NULL DEFAULT '',
  auto_merge INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE pipeline_tasks ADD COLUMN repo_id INTEGER REFERENCES repos(id);
ALTER TABLE pipeline_tasks ADD COLUMN backend TEXT;

ALTER TABLE proposals ADD COLUMN repo_id INTEGER REFERENCES repos(id);

-- Backfill repo_id from repo_path where a matching repos row exists.
-- Run after populating the repos table from .env WATCHED_REPOS / PIPELINE_REPO.
-- UPDATE pipeline_tasks SET repo_id = (SELECT id FROM repos WHERE path = repo_path);
-- UPDATE proposals SET repo_id = (SELECT id FROM repos WHERE path = repo_path);
