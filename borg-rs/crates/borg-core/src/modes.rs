use std::sync::OnceLock;

use crate::types::PipelineMode;

static MODES: OnceLock<Vec<PipelineMode>> = OnceLock::new();

/// Register all built-in modes. Must be called once at startup before any
/// `get_mode` / `all_modes` calls. Typically called from the server binary
/// with the modes provided by `borg_domains::all_modes()`.
pub fn register_modes(modes: Vec<PipelineMode>) {
    MODES.set(modes).ok();
}

pub fn all_modes() -> Vec<PipelineMode> {
    MODES.get().cloned().unwrap_or_default()
}

pub fn get_mode(name: &str) -> Option<PipelineMode> {
    let alias = match name {
        "swe" => "sweborg",
        "legal" => "lawborg",
        _ => name,
    };
    all_modes().into_iter().find(|m| m.name == alias)
}
