-- Unified structured event log. Replaces the legacy events table's untyped
-- message/metadata model with typed kind + JSON payload.

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
