use sha2::{Digest, Sha256};

use crate::providers::types::{MessageContent, MessageEvent};

pub fn canonical_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn sha256_hex(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    hex::encode(hasher.finalize())
}

/// Cross-session content fingerprint for a single turn.
///
/// Only hashes `role + canonical_text(content)`. Intentionally **does not**
/// include agent / session_id / turn_index, so two turns with the same
/// normalized text in different sessions (or in continuation replays from
/// a different agent) collide. The source location is preserved separately
/// via the `turn_fingerprints` PK (`project_key, agent, session_id, turn_index`).
pub fn turn_content_hash(event: &MessageEvent) -> String {
    let role = format!("{:?}", event.role);
    let text = canonical_text(&content_text(&event.content));
    sha256_hex(&[&role, &text])
}

pub fn turn_content_len(event: &MessageEvent) -> usize {
    canonical_text(&content_text(&event.content))
        .chars()
        .count()
}

pub fn record_hash(project_key: &str, title: &str, summary: &str, body: &str) -> String {
    let title = canonical_text(title);
    let summary = canonical_text(summary);
    let body = canonical_text(body);
    sha256_hex(&[project_key, &title, &summary, &body])
}

pub fn content_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { text } => text.clone(),
        MessageContent::ToolUse { tool } => {
            let input = tool
                .input
                .as_ref()
                .map(|v| v.to_string())
                .or_else(|| tool.raw.clone())
                .unwrap_or_default();
            format!("{} {}", tool.name, input)
        }
        MessageContent::ToolResult { result } => result.text.clone(),
        MessageContent::Mixed { parts } => parts
            .iter()
            .map(content_text)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_text, sha256_hex, turn_content_hash};
    use crate::providers::types::{
        AgentKind, MessageContent, MessageEvent, MessageRole, ProjectRef, SessionSource,
        SourceKind, SourceLocation,
    };

    #[test]
    fn canonical_text_normalizes_whitespace() {
        assert_eq!(canonical_text(" hello\n\n world\t"), "hello world");
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        assert_eq!(sha256_hex(&["a", "b"]), sha256_hex(&["a", "b"]));
        assert_ne!(sha256_hex(&["ab"]), sha256_hex(&["a", "b"]));
    }

    #[test]
    fn turn_content_hash_is_stable_across_sessions_and_agents() {
        let mk_event = |agent: &str, session_id: &str, turn_index, text: &str| {
            let source = SessionSource {
                agent: AgentKind::new(agent),
                session_id: session_id.to_string(),
                scope: "scope".to_string(),
                file_path: format!("/tmp/{session_id}.jsonl"),
                project: Some(ProjectRef {
                    project_key: "p".to_string(),
                    project_path: None,
                    project_name: None,
                }),
                source_kind: SourceKind::MainSession,
                metadata: Default::default(),
            };
            MessageEvent {
                source: source.clone(),
                event_id: None,
                turn_index,
                role: MessageRole::User,
                content: MessageContent::Text {
                    text: text.to_string(),
                },
                timestamp: None,
                location: SourceLocation::file(source.file_path.clone()),
                metadata: Default::default(),
            }
        };

        let a = mk_event("codex", "sess-a", 0, "design qmd memory");
        let b = mk_event("claude", "sess-b", 7, "design qmd memory");
        assert_eq!(turn_content_hash(&a), turn_content_hash(&b));

        let c = mk_event("codex", "sess-a", 0, "different request");
        assert_ne!(turn_content_hash(&a), turn_content_hash(&c));
    }
}
