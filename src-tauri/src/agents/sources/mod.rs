pub mod claude;
pub mod codex;
pub mod gemini;
pub mod registry;
pub mod shared;
pub mod types;

use anyhow::Result;
use std::time::SystemTime;

use crate::models::SessionInfo;

pub fn list_all() -> Vec<SessionInfo> {
    let mut out = Vec::new();
    for f in [
        codex::parser::list_sessions as fn() -> Result<Vec<SessionInfo>>,
        claude::parser::list_sessions,
        gemini::parser::list_sessions,
    ] {
        match f() {
            Ok(mut v) => out.append(&mut v),
            Err(e) => log::warn!("source parser failed: {e}"),
        }
    }
    out
}

pub fn system_time_to_millis(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

pub fn builtin_agent_sources() -> registry::AgentSourceRegistry {
    let mut registry = registry::AgentSourceRegistry::new();
    registry.register(codex::CodexSource);
    registry.register(claude::ClaudeSource);
    registry.register(gemini::GeminiSource);
    registry
}
