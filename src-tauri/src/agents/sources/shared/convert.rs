use sha2::{Digest, Sha256};
use std::path::Path;

use crate::agents::sources::types::{
    AgentKind, MessageContent, MessageEvent, MessageRole, Metadata, ProjectRef, SessionRecord,
    SessionSource, SourceKind, SourceLocation, ToolResultEvent, ToolUseEvent,
};
use crate::models::{Agent, SessionInfo, SessionMessage};

const TOOL_RESULT_PREVIEW_CHARS: usize = 1200;

pub fn agent_kind(agent: Agent) -> AgentKind {
    AgentKind::new(agent.as_str())
}

pub fn session_record_from_info(info: &SessionInfo) -> SessionRecord {
    SessionRecord {
        source: session_source_from_info(info),
        started_at: info.started_at,
        updated_at: info.updated_at,
        message_count: info.message_count,
        title: info.title.clone(),
        first_user_message: info.first_user_message.clone(),
        file_size: info.file_size,
        file_mtime: None,
        partial: info.partial,
        available: info.available,
        archived: info.archived,
        children: info
            .subagents
            .iter()
            .map(|sub| {
                let mut source = session_source_from_info(info);
                source.session_id = sub.id.clone();
                source.file_path = sub.file_path.clone();
                source.source_kind = SourceKind::Subagent;
                SessionRecord {
                    source,
                    started_at: sub.started_at,
                    updated_at: sub.updated_at,
                    message_count: sub.message_count,
                    title: sub.first_user_message.clone(),
                    first_user_message: sub.first_user_message.clone(),
                    file_size: sub.file_size,
                    file_mtime: None,
                    partial: sub.partial,
                    available: sub.available,
                    archived: info.archived,
                    children: Vec::new(),
                    metadata: Default::default(),
                }
            })
            .collect(),
        metadata: Default::default(),
    }
}

pub fn session_source_from_info(info: &SessionInfo) -> SessionSource {
    let mut metadata = Metadata::default();
    if let Some(started_at) = info.started_at {
        metadata.insert(
            "started_at".to_string(),
            serde_json::Value::Number(started_at.into()),
        );
    }
    if let Some(updated_at) = info.updated_at {
        metadata.insert(
            "updated_at".to_string(),
            serde_json::Value::Number(updated_at.into()),
        );
    }
    if let Some(forked_from_agent) = info.forked_from_agent {
        metadata.insert(
            "forked_from_agent".to_string(),
            serde_json::Value::String(forked_from_agent.as_str().to_string()),
        );
    }
    if let Some(forked_from_id) = &info.forked_from_id {
        metadata.insert(
            "forked_from_id".to_string(),
            serde_json::Value::String(forked_from_id.clone()),
        );
    }
    SessionSource {
        agent: agent_kind(info.agent),
        session_id: info.id.clone(),
        scope: scope_for_info(info),
        file_path: info.file_path.clone(),
        project: project_ref_for_info(info),
        source_kind: if info.archived {
            SourceKind::Archive
        } else {
            SourceKind::MainSession
        },
        metadata,
    }
}

pub fn message_events_from_messages(
    source: &SessionSource,
    messages: Vec<(SessionMessage, SourceLocation)>,
) -> Vec<MessageEvent> {
    messages
        .into_iter()
        .enumerate()
        .map(|(turn_index, (message, location))| {
            let role = message_role(&message.role);
            let content = message_content(&message.role, &message.text);
            MessageEvent {
                source: source.clone(),
                event_id: Some(event_id(source, turn_index, &message.role, &message.text)),
                turn_index,
                role,
                content,
                timestamp: message.timestamp,
                location,
                metadata: Default::default(),
            }
        })
        .collect()
}

fn message_role(role: &str) -> MessageRole {
    match role {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        "thinking" => MessageRole::Thinking,
        "tool" | "tool_use" | "function_call" | "tool_call" | "todo" => MessageRole::ToolUse,
        "tool_result" | "function_call_output" => MessageRole::ToolResult,
        "system" => MessageRole::System,
        _ => MessageRole::Unknown,
    }
}

fn message_content(role: &str, text: &str) -> MessageContent {
    match message_role(role) {
        MessageRole::ToolUse => {
            let (name, raw) = if role == "todo" {
                ("TodoWrite".to_string(), Some(text.to_string()))
            } else {
                parse_tool_call_text(text)
            };
            MessageContent::ToolUse {
                tool: ToolUseEvent {
                    name,
                    input: raw
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                    raw,
                },
            }
        }
        MessageRole::ToolResult => MessageContent::ToolResult {
            result: ToolResultEvent {
                tool_name: None,
                exit_code: None,
                success: None,
                text: compact_tool_result(text),
                output_hash: Some(hash_text(text)),
            },
        },
        _ => MessageContent::Text {
            text: text.to_string(),
        },
    }
}

fn parse_tool_call_text(text: &str) -> (String, Option<String>) {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            let name = rest[..end].trim().to_string();
            let raw = rest[end + 1..].trim();
            return (
                if name.is_empty() {
                    "tool".to_string()
                } else {
                    name
                },
                if raw.is_empty() {
                    None
                } else {
                    Some(raw.to_string())
                },
            );
        }
    }
    ("tool".to_string(), Some(trimmed.to_string()))
}

fn compact_tool_result(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= TOOL_RESULT_PREVIEW_CHARS {
        return normalized;
    }
    let mut out = normalized
        .chars()
        .take(TOOL_RESULT_PREVIEW_CHARS.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn event_id(source: &SessionSource, turn_index: usize, role: &str, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.agent.as_str().as_bytes());
    hasher.update(source.session_id.as_bytes());
    hasher.update(turn_index.to_string().as_bytes());
    hasher.update(role.as_bytes());
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

fn project_ref_for_info(info: &SessionInfo) -> Option<ProjectRef> {
    let project_path = info.project_path.clone();
    let project_name = info.project_name.clone();
    if project_path.is_none() && project_name.is_none() {
        return None;
    }
    let project_key =
        project_key_for_path_or_name(project_path.as_deref(), project_name.as_deref());
    Some(ProjectRef {
        project_key,
        project_path,
        project_name,
    })
}

pub fn project_key_for_path_or_name(
    project_path: Option<&str>,
    project_name: Option<&str>,
) -> String {
    project_path
        .map(project_key_slug)
        .or_else(|| project_name.map(project_key_slug))
        .unwrap_or_else(|| "unknown".to_string())
}

pub fn project_key_slug(input: &str) -> String {
    let slug = input
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if slug.is_empty() {
        "unknown".to_string()
    } else {
        slug
    }
}

fn scope_for_info(info: &SessionInfo) -> String {
    if info.agent == Agent::Gemini {
        return info.file_path.clone();
    }
    Path::new(&info.file_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| info.file_path.clone())
}

#[cfg(test)]
mod tests {
    use super::{message_events_from_messages, project_key_for_path_or_name};
    use crate::agents::sources::types::{
        AgentKind, MessageContent, ProjectRef, SessionSource, SourceKind, SourceLocation,
    };
    use crate::models::SessionMessage;

    #[test]
    fn converts_tool_messages_to_structured_content() {
        let source = SessionSource {
            agent: AgentKind::new("codex"),
            session_id: "s1".to_string(),
            scope: "scope".to_string(),
            file_path: "/tmp/session.jsonl".to_string(),
            project: Some(ProjectRef {
                project_key: "p".to_string(),
                project_path: None,
                project_name: None,
            }),
            source_kind: SourceKind::MainSession,
            metadata: Default::default(),
        };
        let events = message_events_from_messages(
            &source,
            vec![
                (
                    SessionMessage {
                        role: "tool_call".to_string(),
                        text: "[shell]\n{\"cmd\":\"cargo check\"}".to_string(),
                        timestamp: None,
                        tool_call_id: None,
                    },
                    SourceLocation::file("/tmp/session.jsonl"),
                ),
                (
                    SessionMessage {
                        role: "tool_result".to_string(),
                        text: "ok ".repeat(1000),
                        timestamp: None,
                        tool_call_id: None,
                    },
                    SourceLocation::file("/tmp/session.jsonl"),
                ),
            ],
        );
        match &events[0].content {
            MessageContent::ToolUse { tool } => assert_eq!(tool.name, "shell"),
            other => panic!("expected tool use, got {other:?}"),
        }
        match &events[1].content {
            MessageContent::ToolResult { result } => {
                assert!(result.output_hash.is_some());
                assert!(result.text.len() < "ok ".repeat(1000).len());
            }
            other => panic!("expected tool result, got {other:?}"),
        }
    }

    #[test]
    fn project_key_uses_readable_path_slug() {
        assert_eq!(
            project_key_for_path_or_name(Some("/Users/alex/Work/cloudgeek/sessio"), None),
            "-Users-alex-Work-cloudgeek-sessio"
        );
        assert_eq!(
            project_key_for_path_or_name(Some("C:\\Work\\cloudgeek\\sessio.v2"), None),
            "C--Work-cloudgeek-sessio-v2"
        );
        assert_eq!(
            project_key_for_path_or_name(Some("/Users/alex/项目/sessio"), None),
            "-Users-alex-项目-sessio"
        );
    }
}
