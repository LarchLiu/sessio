pub mod codex;
pub mod claude;
pub mod gemini;
pub mod jsonl_scan;

use anyhow::Result;
use std::time::SystemTime;

use crate::models::SessionInfo;

pub fn list_all() -> Vec<SessionInfo> {
    let mut out = Vec::new();
    for f in [
        codex::list_sessions as fn() -> Result<Vec<SessionInfo>>,
        claude::list_sessions,
        gemini::list_sessions,
    ] {
        match f() {
            Ok(mut v) => out.append(&mut v),
            Err(e) => log::warn!("reader failed: {e}"),
        }
    }
    out
}

pub fn system_time_to_millis(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}
