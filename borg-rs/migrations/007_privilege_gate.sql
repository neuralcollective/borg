-- migration 007: Add privilege gate columns
ALTER TABLE projects ADD COLUMN session_privileged INTEGER NOT NULL DEFAULT 0;
ALTER TABLE project_files ADD COLUMN privileged INTEGER NOT NULL DEFAULT 0;
