-- Per-task chat thread. Stores director / user / system messages tied to a
-- specific pipeline task, separate from global chat_agent_runs.

CREATE TABLE IF NOT EXISTS task_messages (
  id INTEGER PRIMARY KEY,
  task_id INTEGER NOT NULL REFERENCES pipeline_tasks(id),
  role TEXT NOT NULL,          -- 'user' | 'director' | 'system'
  content TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  delivered_phase TEXT         -- NULL = not yet delivered to an agent phase
);
CREATE INDEX IF NOT EXISTS idx_task_messages_task_id ON task_messages(task_id);
