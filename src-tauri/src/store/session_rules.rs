use std::collections::HashMap;

use super::IndexedSessionRecord;
use crate::models::{Agent, SessionInfo};

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn file_mtime_for(file_path: &str) -> Option<i64> {
    if file_path.is_empty() {
        return None;
    }
    std::fs::metadata(file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as i64)
        })
}

pub(crate) fn is_virtual_session_ref(value: &str) -> bool {
    value.trim_start().starts_with("astra://")
}

pub(crate) fn is_real_session_file_path(file_path: &str) -> bool {
    !file_path.trim().is_empty() && !is_virtual_session_ref(file_path)
}

pub(crate) fn is_placeholder_indexed_session(record: &IndexedSessionRecord) -> bool {
    record.file_size == 0 && record.available
}

pub(crate) fn insert_best_session(
    sessions: &mut HashMap<(Agent, String), SessionInfo>,
    session: SessionInfo,
) {
    let key = (session.agent, session.id.clone());
    let replace = sessions
        .get(&key)
        .map(|current| better_session_candidate(&session, current))
        .unwrap_or(true);
    if replace {
        sessions.insert(key, session);
    }
}

pub(crate) fn better_session_candidate(candidate: &SessionInfo, current: &SessionInfo) -> bool {
    if candidate.available != current.available {
        return candidate.available;
    }
    if candidate.partial != current.partial {
        return !candidate.partial;
    }
    let candidate_real_path = is_real_session_file_path(&candidate.file_path);
    let current_real_path = is_real_session_file_path(&current.file_path);
    if candidate_real_path != current_real_path {
        return candidate_real_path;
    }
    if candidate.file_path.is_empty() != current.file_path.is_empty() {
        return !candidate.file_path.is_empty();
    }
    session_time(candidate) > session_time(current)
}

pub(crate) fn session_time(session: &SessionInfo) -> i64 {
    session.updated_at.or(session.started_at).unwrap_or(0)
}
