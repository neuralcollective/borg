use std::{collections::VecDeque, sync::Arc};

use tokio::sync::broadcast;

pub(crate) struct BroadcastLayer {
    pub tx: broadcast::Sender<String>,
    pub ring: Arc<std::sync::Mutex<VecDeque<String>>>,
}

struct MessageVisitor<'a> {
    message: &'a mut String,
}

impl tracing::field::Visit for MessageVisitor<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            *self.message = value.to_string();
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message.clear();
            use std::fmt::Write;
            let _ = write!(self.message, "{value:?}");
            // Strip surrounding quotes added by Debug on &str
            if self.message.starts_with('"') && self.message.ends_with('"') {
                *self.message = self.message[1..self.message.len() - 1].to_string();
            }
        }
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for BroadcastLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => "err",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            tracing::Level::DEBUG => "debug",
            tracing::Level::TRACE => return,
        };

        let target = event.metadata().target();
        let category = if target.contains("pipeline") {
            "pipeline"
        } else if target.contains("agent") || target.contains("claude") || target.contains("codex")
        {
            "agent"
        } else {
            "system"
        };

        let mut message = String::new();
        event.record(&mut MessageVisitor {
            message: &mut message,
        });

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let json = serde_json::json!({
            "ts": ts,
            "level": level,
            "message": message,
            "category": category,
        })
        .to_string();

        let _ = self.tx.send(json.clone());
        if let Ok(mut ring) = self.ring.lock() {
            ring.push_back(json);
            if ring.len() > 500 {
                ring.pop_front();
            }
        }
    }
}
