use std::{
    collections::{HashMap, VecDeque},
    hash::Hash,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::sync::{broadcast, Mutex};

const MAX_HISTORY_LINES: usize = 10_000;
const ENDED_STREAM_TTL: Duration = Duration::from_secs(5 * 60);

struct Stream {
    tx: broadcast::Sender<String>,
    history: VecDeque<String>,
    ended: bool,
    ended_at: Option<Instant>,
}

/// Generic per-key NDJSON stream manager.
///
/// Each active stream broadcasts lines in real-time and keeps a bounded history
/// buffer. Clients subscribe to get history replay + live tail.
pub struct StreamManager<K> {
    streams: Mutex<HashMap<K, Stream>>,
}

/// Pipeline task streams, keyed by task ID.
pub type TaskStreamManager = StreamManager<i64>;

/// Chat thread streams, keyed by thread string (e.g. "web:workspace:1:web:project-15").
pub type ChatStreamManager = StreamManager<String>;

impl<K: Eq + Hash + Clone + Send + 'static> StreamManager<K> {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            streams: Mutex::new(HashMap::new()),
        })
    }

    /// Begin streaming for a key (resets any prior state).
    pub async fn start(&self, key: K) {
        let (tx, _) = broadcast::channel(512);
        let mut map = self.streams.lock().await;
        map.retain(|_, s| {
            s.ended_at
                .map(|t| t.elapsed() < ENDED_STREAM_TTL)
                .unwrap_or(true)
        });
        map.insert(
            key,
            Stream {
                tx,
                history: VecDeque::new(),
                ended: false,
                ended_at: None,
            },
        );
    }

    /// Push an NDJSON line to the stream.
    pub async fn push_line(&self, key: &K, line: String) {
        let mut map = self.streams.lock().await;
        if let Some(s) = map.get_mut(key) {
            let _ = s.tx.send(line.clone());
            s.history.push_back(line);
            if s.history.len() > MAX_HISTORY_LINES {
                s.history.pop_front();
            }
        }
    }

    /// Mark a stream as ended (sends stream_end event, keeps history).
    pub async fn end_stream(&self, key: &K) {
        let line = r#"{"type":"stream_end"}"#.to_string();
        let mut map = self.streams.lock().await;
        if let Some(s) = map.get_mut(key) {
            s.history.push_back(line.clone());
            if s.history.len() > MAX_HISTORY_LINES {
                s.history.pop_front();
            }
            s.ended = true;
            s.ended_at = Some(Instant::now());
            let _ = s.tx.send(line);
        }
    }

    /// Remove all ended streams older than `max_age`.
    pub async fn prune_ended(&self, max_age: Duration) {
        let mut map = self.streams.lock().await;
        map.retain(|_, s| s.ended_at.map(|t| t.elapsed() < max_age).unwrap_or(true));
    }

    /// Subscribe to a stream.
    /// Returns (history_snapshot, live_receiver).
    /// If the stream has ended or doesn't exist, receiver is None.
    ///
    /// The history snapshot and the ended check are taken atomically under the
    /// lock, so the two outcomes are mutually exclusive:
    /// - ended=true: history contains the stream_end sentinel; no live rx.
    /// - ended=false: tx.subscribe() fires before end_stream() can set ended,
    ///   so the returned receiver will eventually deliver it.
    pub async fn subscribe(&self, key: &K) -> (Vec<String>, Option<broadcast::Receiver<String>>) {
        let map = self.streams.lock().await;
        match map.get(key) {
            Some(s) => {
                let history: Vec<String> = s.history.iter().cloned().collect();
                let rx = if !s.ended {
                    Some(s.tx.subscribe())
                } else {
                    None
                };
                (history, rx)
            },
            None => (Vec::new(), None),
        }
    }
}

// Convenience methods for TaskStreamManager that preserve the old API signatures.
impl TaskStreamManager {
    /// Inject a synthetic phase_result SSE line into the task's stream.
    pub async fn push_phase_result(&self, task_id: i64, phase: &str, content: &str) {
        let line = format!(
            r#"{{"type":"phase_result","phase":{},"content":{}}}"#,
            serde_json::to_string(phase).unwrap_or_default(),
            serde_json::to_string(content).unwrap_or_default(),
        );
        self.push_line(&task_id, line).await;
    }

    /// Alias for end_stream with the old name.
    pub async fn end_task(&self, task_id: i64) {
        self.end_stream(&task_id).await;
    }
}
