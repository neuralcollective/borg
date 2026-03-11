use std::path::PathBuf;

/// Read the benchmark tuning context for injection into lawborg system prompts.
/// Checks $BORG_BENCH_TUNING env var first, falls back to .borg/bench-tuning.md.
pub fn read_tuning_context() -> Option<String> {
    let path = std::env::var("BORG_BENCH_TUNING")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".borg/bench-tuning.md"));
    let content = std::fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        None
    } else {
        Some(content)
    }
}
