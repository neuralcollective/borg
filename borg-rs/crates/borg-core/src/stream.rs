use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

const MAX_HISTORY_LINES: usize = 10_000;

struct TaskStream {
    tx: broadcast::Sender<String>,
    history: VecDeque<String>,
    ended: bool,
}

/// Per-task NDJSON stream manager.
///
/// Each running agent phase broadcasts its raw stdout lines here in real-time.
/// Clients can subscribe to get history replay + live tail for any task.
pub struct TaskStreamManager {
    streams: Mutex<HashMap<i64, TaskStream>>,
}

impl TaskStreamManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            streams: Mutex::new(HashMap::new()),
        })
    }

    /// Begin streaming for a task (resets any prior state).
    pub async fn start(&self, task_id: i64) {
        let (tx, _) = broadcast::channel(512);
        let mut map = self.streams.lock().await;
        map.insert(task_id, TaskStream {
            tx,
            history: VecDeque::new(),
            ended: false,
        });
    }

    /// Push an NDJSON line to the task's stream.
    pub async fn push_line(&self, task_id: i64, line: String) {
        let mut map = self.streams.lock().await;
        if let Some(s) = map.get_mut(&task_id) {
            let _ = s.tx.send(line.clone());
            s.history.push_back(line);
            if s.history.len() > MAX_HISTORY_LINES {
                s.history.pop_front();
            }
        }
    }

    /// Inject a synthetic phase_result SSE line into the task's stream.
    pub async fn push_phase_result(&self, task_id: i64, phase: &str, content: &str) {
        let line = format!(
            r#"{{"type":"phase_result","phase":{},"content":{}}}"#,
            serde_json::to_string(phase).unwrap_or_default(),
            serde_json::to_string(content).unwrap_or_default(),
        );
        self.push_line(task_id, line).await;
    }

    /// Mark a task stream as ended (sends stream_end event, keeps history).
    pub async fn end_task(&self, task_id: i64) {
        let line = r#"{"type":"stream_end"}"#.to_string();
        let mut map = self.streams.lock().await;
        if let Some(s) = map.get_mut(&task_id) {
            let _ = s.tx.send(line.clone());
            s.history.push_back(line);
            if s.history.len() > MAX_HISTORY_LINES {
                s.history.pop_front();
            }
            s.ended = true;
        }
    }

    /// Subscribe to a task's stream.
    /// Returns (history_snapshot, live_receiver).
    /// If the stream has ended or doesn't exist, receiver is None.
    pub async fn subscribe(
        &self,
        task_id: i64,
    ) -> (Vec<String>, Option<broadcast::Receiver<String>>) {
        let map = self.streams.lock().await;
        match map.get(&task_id) {
            Some(s) => {
                let history: Vec<String> = s.history.iter().cloned().collect();
                let rx = if !s.ended { Some(s.tx.subscribe()) } else { None };
                (history, rx)
            }
            None => (Vec::new(), None),
        }
    }
}
