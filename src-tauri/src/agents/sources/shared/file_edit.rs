use crate::agents::runtime::types::AcpProtocolMessage;
use crate::turns::history_session_update_message;

pub fn file_edit_message(
    source: &str,
    edits: Vec<serde_json::Value>,
    timestamp: Option<i64>,
) -> Option<AcpProtocolMessage> {
    if edits.is_empty() {
        return None;
    }
    let additions: i64 = edits
        .iter()
        .filter_map(|edit| edit.get("additions").and_then(|value| value.as_i64()))
        .sum();
    let deletions: i64 = edits
        .iter()
        .filter_map(|edit| edit.get("deletions").and_then(|value| value.as_i64()))
        .sum();
    let data = serde_json::json!({
        "source": source,
        "files": edits.len(),
        "additions": additions,
        "deletions": deletions,
        "edits": edits,
    });
    Some(history_session_update_message("file_edit", data, timestamp))
}
