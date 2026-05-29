use sha2::{Digest, Sha256};

use crate::agents::sources::types::{MessageContent, MessageEvent};
use crate::models::SessionContentBlock;

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
        MessageContent::Blocks { blocks } => content_blocks_text(blocks),
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

pub fn content_blocks_text(blocks: &[SessionContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(content_block_text)
        .collect::<Vec<_>>()
        .join("\n")
}

fn content_block_text(block: &SessionContentBlock) -> Option<String> {
    match block.kind.as_str() {
        "text" => block.text.clone(),
        "image" | "audio" => Some(attachment_marker(
            if block.kind == "audio" {
                "audio"
            } else {
                "image"
            },
            block.uri.as_deref().or(block.data.as_deref()),
            block.mime_type.as_deref(),
        )),
        "resource" | "resource_link" => Some(resource_marker(block)),
        _ => block
            .text
            .clone()
            .or_else(|| block.uri.clone())
            .or_else(|| block.name.clone())
            .or_else(|| serde_json::to_string(block).ok()),
    }
}

fn resource_marker(block: &SessionContentBlock) -> String {
    let name = block
        .name
        .as_deref()
        .or(block.title.as_deref())
        .or(block.description.as_deref())
        .or(block.uri.as_deref())
        .unwrap_or("attachment");
    match block.uri.as_deref().filter(|uri| !uri.trim().is_empty()) {
        Some(uri) => format!("[file: {name}|{uri}]"),
        None => format!("[file: {name}]"),
    }
}

fn attachment_marker(kind: &str, identity: Option<&str>, mime_type: Option<&str>) -> String {
    let detail = identity.or(mime_type).unwrap_or(kind).trim();
    if detail.is_empty() {
        format!("[{kind}]")
    } else {
        format!("[{kind}: {detail}]")
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_text, content_text, sha256_hex, turn_content_hash};
    use crate::agents::sources::types::{
        AgentKind, MessageContent, MessageEvent, MessageRole, ProjectRef, SessionSource,
        SourceKind, SourceLocation,
    };
    use crate::models::SessionContentBlock;

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

    #[test]
    fn content_text_preserves_structured_attachment_identity() {
        let content = MessageContent::Blocks {
            blocks: vec![
                SessionContentBlock::text("review this"),
                SessionContentBlock::resource(
                    Some("file:///tmp/spec.md".to_string()),
                    Some("spec.md".to_string()),
                    Some("text/markdown".to_string()),
                ),
                SessionContentBlock::image(
                    "file:///tmp/screen.png".to_string(),
                    Some("image/png".to_string()),
                ),
            ],
        };

        let text = content_text(&content);
        assert!(text.contains("review this"));
        assert!(text.contains("[file: spec.md|file:///tmp/spec.md]"));
        assert!(text.contains("[image: file:///tmp/screen.png]"));
    }
}
