use crate::memory::hash::{card_hash, content_text, turn_content_hash};
use crate::memory::{MemoryCard, MemorySource, TurnFingerprint};
use crate::providers::types::{
    MessageContent, MessageEvent, MessageRole, SessionSource, SourceLocation,
};

const MAX_SUMMARY_CHARS: usize = 360;
const MAX_BODY_CHARS: usize = 2400;

pub fn cards_for_source(
    source: &SessionSource,
    events: &[MessageEvent],
) -> Vec<(MemoryCard, Vec<MemorySource>)> {
    if events.is_empty() {
        return Vec::new();
    }
    let Some(project) = &source.project else {
        return Vec::new();
    };

    let title = title_for_events(events).unwrap_or_else(|| {
        format!(
            "{} session {}",
            source.agent.as_str(),
            short_id(&source.session_id)
        )
    });
    let summary = summarize_events(events);
    let body = card_body(source, events);
    let canonical_hash = card_hash(&project.project_key, &title, &summary, &body);
    let card_id = format!(
        "sessio-{}-{}",
        safe_id_part(source.agent.as_str()),
        safe_id_part(&source.session_id)
    );
    let qmd_path = format!("{}/cards/{}.md", project.project_key, card_id);
    let updated_at = events
        .iter()
        .filter_map(|event| event.timestamp)
        .max()
        .unwrap_or(0);
    let source_ref = MemorySource {
        card_id: card_id.clone(),
        agent: source.agent.as_str().to_string(),
        session_id: source.session_id.clone(),
        file_path: source.file_path.clone(),
        location: events_span_location(&source.file_path, events),
    };

    vec![(
        MemoryCard {
            card_id,
            project_key: project.project_key.clone(),
            canonical_hash,
            simhash: None,
            qmd_path,
            title,
            summary: Some(summary),
            body,
            available: true,
            updated_at,
        },
        vec![source_ref],
    )]
}

fn title_for_events(events: &[MessageEvent]) -> Option<String> {
    events
        .iter()
        .find(|event| event.role == MessageRole::User)
        .map(|event| compact(&content_text(&event.content), 96))
        .filter(|s| !s.is_empty())
}

fn summarize_events(events: &[MessageEvent]) -> String {
    let first_user = events
        .iter()
        .find(|event| event.role == MessageRole::User)
        .map(|event| compact(&content_text(&event.content), MAX_SUMMARY_CHARS));
    first_user.unwrap_or_else(|| {
        format!(
            "Session contains {} normalized events across user, assistant, thinking, and tool activity.",
            events.len()
        )
    })
}

fn card_body(source: &SessionSource, events: &[MessageEvent]) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "# {}",
        title_for_events(events).unwrap_or_else(|| "Session memory".to_string())
    ));
    lines.push(String::new());
    lines.push("Summary:".to_string());
    lines.push(summarize_events(events));
    lines.push(String::new());
    lines.push("Key turns:".to_string());
    for event in events
        .iter()
        .filter(|event| {
            matches!(
                event.role,
                MessageRole::User | MessageRole::Assistant | MessageRole::Thinking
            )
        })
        .take(12)
    {
        let role = format!("{:?}", event.role).to_lowercase();
        let text = compact(&content_text(&event.content), 280);
        if text.is_empty() {
            continue;
        }
        lines.push(format!("- {}: {}", role, text));
    }
    lines.push(String::new());
    let tool_lines = tool_summaries(events);
    if !tool_lines.is_empty() {
        lines.push("Tool activity:".to_string());
        lines.extend(tool_lines);
        lines.push(String::new());
    }
    lines.push("Source:".to_string());
    lines.push(format!(
        "- {} {} {}",
        source.agent.as_str(),
        source.session_id,
        source.file_path
    ));
    compact(&lines.join("\n"), MAX_BODY_CHARS)
}

pub fn fingerprints_for_source(
    source: &SessionSource,
    events: &[MessageEvent],
) -> Vec<TurnFingerprint> {
    let Some(project) = &source.project else {
        return Vec::new();
    };
    events
        .iter()
        .map(|event| TurnFingerprint {
            project_key: project.project_key.clone(),
            agent: source.agent.as_str().to_string(),
            session_id: source.session_id.clone(),
            turn_index: event.turn_index,
            role: format!("{:?}", event.role).to_lowercase(),
            canonical_hash: turn_content_hash(event),
            location: event.location.clone(),
        })
        .collect()
}

// Aggregate the line/byte span covered by all events into a single
// SourceLocation that can sit on the card-level MemorySource. The resulting
// location lets `memory resolve` map a card back to the contiguous raw-JSONL
// range it summarizes. Falls back to a session-level pointer when none of
// the events carry offset info (e.g. Gemini until v2 follow-up).
fn events_span_location(file_path: &str, events: &[MessageEvent]) -> SourceLocation {
    let mut line_start: Option<u64> = None;
    let mut line_end: Option<u64> = None;
    let mut byte_start: Option<u64> = None;
    let mut byte_end: Option<u64> = None;
    for event in events {
        if let Some(value) = event.location.line_start {
            line_start = Some(line_start.map_or(value, |existing| existing.min(value)));
        }
        if let Some(value) = event.location.line_end {
            line_end = Some(line_end.map_or(value, |existing| existing.max(value)));
        }
        if let Some(value) = event.location.byte_start {
            byte_start = Some(byte_start.map_or(value, |existing| existing.min(value)));
        }
        if let Some(value) = event.location.byte_end {
            byte_end = Some(byte_end.map_or(value, |existing| existing.max(value)));
        }
    }
    SourceLocation {
        file_path: file_path.to_string(),
        line_start,
        line_end,
        byte_start,
        byte_end,
    }
}

fn tool_summaries(events: &[MessageEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.content {
            MessageContent::ToolUse { tool } => {
                let input = tool
                    .raw
                    .as_deref()
                    .map(|s| compact(s, 160))
                    .filter(|s| !s.is_empty());
                Some(match input {
                    Some(input) => format!("- use {}: {}", tool.name, input),
                    None => format!("- use {}", tool.name),
                })
            }
            MessageContent::ToolResult { result } => {
                let hash = result.output_hash.as_deref().unwrap_or("");
                let preview = compact(&result.text, 180);
                Some(if preview.is_empty() {
                    format!("- result hash {}", short_hash(hash))
                } else {
                    format!("- result {}: {}", short_hash(hash), preview)
                })
            }
            _ => None,
        })
        .take(12)
        .collect()
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn compact(input: &str, max_chars: usize) -> String {
    let text = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn safe_id_part(value: &str) -> String {
    let safe = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if safe.is_empty() {
        "unknown".to_string()
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::cards_for_source;
    use crate::providers::types::{
        AgentKind, MessageContent, MessageEvent, MessageRole, ProjectRef, SessionSource,
        SourceKind, SourceLocation,
    };

    #[test]
    fn creates_card_for_project_source() {
        let source = SessionSource {
            agent: AgentKind::new("codex"),
            session_id: "abc123".to_string(),
            scope: "scope".to_string(),
            file_path: "/tmp/session.jsonl".to_string(),
            project: Some(ProjectRef {
                project_key: "p_test".to_string(),
                project_path: Some("/tmp/project".to_string()),
                project_name: Some("project".to_string()),
            }),
            source_kind: SourceKind::MainSession,
            metadata: Default::default(),
        };
        let event = MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index: 0,
            role: MessageRole::User,
            content: MessageContent::Text {
                text: "Design qmd memory storage".to_string(),
            },
            timestamp: Some(1),
            location: SourceLocation::file("/tmp/session.jsonl"),
            metadata: Default::default(),
        };
        let cards = cards_for_source(&source, &[event]);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].0.project_key, "p_test");
        assert_eq!(cards[0].0.card_id, "sessio-codex-abc123");
        assert_eq!(cards[0].1[0].session_id, "abc123");
    }
}
