use crate::models::strip_injected_context;
use crate::providers::types::{MessageContent, MessageEvent};

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
}
