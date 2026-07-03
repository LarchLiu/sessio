use super::IndexedSessionRecord;

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
