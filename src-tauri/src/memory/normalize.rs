use crate::agents::sources::types::{MessageContent, MessageEvent};
use crate::models::{strip_injected_context, SessionContentBlock};

const CROSS_START: &str = "<!-- sessio-cross:start";
const CROSS_END: &str = "<!-- sessio-cross:end -->";

pub fn normalize_events(events: Vec<MessageEvent>) -> Vec<MessageEvent> {
    events
        .into_iter()
        .filter_map(strip_cross_replay_from_event)
        .collect()
}

fn strip_cross_replay_from_event(mut event: MessageEvent) -> Option<MessageEvent> {
    event.content = match event.content {
        MessageContent::Text { text } => {
            let text = clean_text(&text);
            if text.trim().is_empty() {
                return None;
            }
            MessageContent::Text { text }
        }
        MessageContent::Blocks { blocks } => {
            let blocks = clean_content_blocks(blocks);
            if blocks.is_empty() {
                return None;
            }
            MessageContent::Blocks { blocks }
        }
        MessageContent::Mixed { parts } => {
            let parts = parts
                .into_iter()
                .filter_map(|part| match part {
                    MessageContent::Text { text } => {
                        let text = clean_text(&text);
                        if text.trim().is_empty() {
                            None
                        } else {
                            Some(MessageContent::Text { text })
                        }
                    }
                    MessageContent::Blocks { blocks } => {
                        let blocks = clean_content_blocks(blocks);
                        if blocks.is_empty() {
                            None
                        } else {
                            Some(MessageContent::Blocks { blocks })
                        }
                    }
                    other => Some(other),
                })
                .collect::<Vec<_>>();
            if parts.is_empty() {
                return None;
            }
            MessageContent::Mixed { parts }
        }
        other => other,
    };
    Some(event)
}

fn clean_content_blocks(blocks: Vec<SessionContentBlock>) -> Vec<SessionContentBlock> {
    blocks
        .into_iter()
        .filter_map(|mut block| {
            if is_cross_context_block(&block) {
                return None;
            }
            if block.kind == "text" {
                let text = block.text.as_deref().map(clean_text).unwrap_or_default();
                if text.trim().is_empty() {
                    return None;
                }
                block.text = Some(text);
            }
            Some(block)
        })
        .collect()
}

fn is_cross_context_block(block: &SessionContentBlock) -> bool {
    block.uri.as_deref().is_some_and(is_cross_context_value)
        || block.name.as_deref().is_some_and(is_cross_context_value)
        || block.title.as_deref().is_some_and(is_cross_context_value)
        || block
            .description
            .as_deref()
            .is_some_and(is_cross_context_value)
        || block
            .text
            .as_deref()
            .is_some_and(|text| text.contains(CROSS_START) || is_cross_context_value(text))
}

fn is_cross_context_value(value: &str) -> bool {
    value.contains("sessio-cross-context") || value.contains("/.cross-context/")
}

pub fn strip_sessio_cross_replay(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let Some(start) = rest.find(CROSS_START) else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after_start = &rest[start..];
        let Some(end_rel) = after_start.find(CROSS_END) else {
            break;
        };
        rest = &after_start[end_rel + CROSS_END.len()..];
    }
    out.trim().to_string()
}

fn clean_text(input: &str) -> String {
    strip_injected_context(&strip_sessio_cross_replay(input))
}

#[cfg(test)]
mod tests {
    use crate::agents::sources::types::{
        AgentKind, MessageContent, MessageEvent, MessageRole, SessionSource, SourceKind,
        SourceLocation,
    };
    use crate::models::SessionContentBlock;

    use super::strip_sessio_cross_replay;

    #[test]
    fn strips_cross_replay_block() {
        let input = r#"<!-- sessio-cross:start source_agent="codex" source_session_id="abc" -->

# Continued session from agent
old context

<!-- sessio-cross:end -->

new request"#;
        assert_eq!(strip_sessio_cross_replay(input), "new request");
    }

    #[test]
    fn keeps_text_without_cross_replay() {
        assert_eq!(strip_sessio_cross_replay("hello"), "hello");
    }

    #[test]
    fn strips_ide_injected_context_after_cross_replay() {
        let input = "<ide_opened_file>noise</ide_opened_file> real request";
        assert_eq!(super::clean_text(input), "real request");
    }

    #[test]
    fn drops_unclosed_cross_replay_to_avoid_indexing_context() {
        assert_eq!(
            strip_sessio_cross_replay("before <!-- sessio-cross:start bad"),
            "before"
        );
    }

    #[test]
    fn normalizes_structured_blocks_without_indexing_cross_context_attachment() {
        let source = SessionSource {
            agent: AgentKind::new("claude"),
            session_id: "s1".to_string(),
            scope: "scope".to_string(),
            file_path: "/tmp/s1.jsonl".to_string(),
            project: None,
            source_kind: SourceKind::MainSession,
            metadata: Default::default(),
        };
        let events = super::normalize_events(vec![MessageEvent {
            source,
            event_id: None,
            turn_index: 0,
            role: MessageRole::User,
            content: MessageContent::Blocks {
                blocks: vec![
                    SessionContentBlock::text(
                        "<ide_opened_file>noise</ide_opened_file> real request",
                    ),
                    SessionContentBlock::resource(
                        Some(
                            "file:///tmp/.cross-context/sessio-cross-context-parent.md".to_string(),
                        ),
                        Some("sessio-cross-context-parent.md".to_string()),
                        Some("text/markdown".to_string()),
                    ),
                    SessionContentBlock::resource(
                        Some("file:///tmp/spec.md".to_string()),
                        Some("spec.md".to_string()),
                        Some("text/markdown".to_string()),
                    ),
                ],
            },
            timestamp: None,
            location: SourceLocation::file("/tmp/s1.jsonl"),
            metadata: Default::default(),
        }]);

        assert_eq!(events.len(), 1);
        let MessageContent::Blocks { blocks } = &events[0].content else {
            panic!("expected structured blocks");
        };
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text.as_deref(), Some("real request"));
        assert_eq!(blocks[1].uri.as_deref(), Some("file:///tmp/spec.md"));
    }
}
