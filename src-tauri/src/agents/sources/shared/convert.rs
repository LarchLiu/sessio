use sha2::{Digest, Sha256};
use std::path::Path;

use crate::agents::sources::types::{
    AgentKind, HistoryAcpMessage, MessageContent, MessageEvent, MessageRole, Metadata, ProjectRef,
    SessionRecord, SessionSource, SourceKind, ToolResultEvent, ToolUseEvent,
};
use crate::models::{Agent, SessionContentBlock, SessionInfo};

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

pub fn message_events_from_history_acp_messages(
    source: &SessionSource,
    events: Vec<HistoryAcpMessage>,
) -> Vec<MessageEvent> {
    events
        .into_iter()
        .enumerate()
        .map(|(turn_index, event)| {
            let role = acp_message_role(&event);
            let content = acp_message_content(&event, role);
            let text = event_text_for_id(&content);
            MessageEvent {
                source: source.clone(),
                event_id: Some(event_id(source, turn_index, role_id(role), &text)),
                turn_index,
                role,
                content,
                timestamp: event.timestamp,
                location: event.location,
                metadata: Default::default(),
            }
        })
        .collect()
}

fn acp_message_role(event: &HistoryAcpMessage) -> MessageRole {
    let message = &event.message;
    if message.method == "session/prompt" && message.direction == "client_to_agent" {
        return MessageRole::User;
    }
    if message.method == "session/request_permission" && message.message_kind == "request" {
        return MessageRole::ToolUse;
    }
    if message.method != "session/update" {
        return MessageRole::Unknown;
    }
    match acp_update_type(&message.data, message.update_type.as_deref()).as_deref() {
        Some("agent_message_chunk") => MessageRole::Assistant,
        Some("agent_thought_chunk") => MessageRole::Thinking,
        Some("tool_call") => MessageRole::ToolUse,
        Some("tool_call_update") => MessageRole::ToolResult,
        _ => MessageRole::Unknown,
    }
}

fn acp_message_content(event: &HistoryAcpMessage, role: MessageRole) -> MessageContent {
    let message = &event.message;
    match role {
        MessageRole::User => MessageContent::Blocks {
            blocks: content_blocks_from_value(message.data.get("prompt")),
        },
        MessageRole::Assistant | MessageRole::Thinking => MessageContent::Blocks {
            blocks: content_blocks_from_value(update_field(&message.data, "content")),
        },
        MessageRole::ToolUse => tool_use_content(&message.data),
        MessageRole::ToolResult => tool_result_content(&message.data),
        _ => MessageContent::Text {
            text: serde_json::to_string(&message.data).unwrap_or_default(),
        },
    }
}

fn tool_use_content(data: &serde_json::Value) -> MessageContent {
    let update = update_value(data).unwrap_or(data);
    let tool_call = update
        .get("toolCall")
        .or_else(|| update.get("tool_call"))
        .unwrap_or(update);
    let fields = tool_call.get("fields").unwrap_or(tool_call);
    let input = fields
        .get("rawInput")
        .or_else(|| fields.get("raw_input"))
        .or_else(|| update.get("rawInput"))
        .or_else(|| update.get("input"))
        .cloned();
    let raw = input.as_ref().map(|value| {
        value
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
    });
    MessageContent::ToolUse {
        tool: ToolUseEvent {
            name: fields
                .get("title")
                .or_else(|| fields.get("name"))
                .or_else(|| update.get("title"))
                .and_then(|value| value.as_str())
                .unwrap_or("Tool Use")
                .to_string(),
            input,
            raw,
        },
    }
}

fn tool_result_content(data: &serde_json::Value) -> MessageContent {
    let update = update_value(data).unwrap_or(data);
    let output = update
        .get("rawOutput")
        .or_else(|| update.get("output"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let text = output
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| serde_json::to_string(&output).unwrap_or_default());
    MessageContent::ToolResult {
        result: ToolResultEvent {
            tool_name: None,
            exit_code: None,
            success: None,
            text: compact_tool_result(&text),
            output_hash: Some(hash_text(&text)),
        },
    }
}

fn content_blocks_from_value(value: Option<&serde_json::Value>) -> Vec<SessionContentBlock> {
    let Some(value) = value else {
        return Vec::new();
    };
    let values = value
        .as_array()
        .cloned()
        .unwrap_or_else(|| vec![value.clone()]);
    values
        .into_iter()
        .filter_map(|item| {
            let kind = item
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("text");
            match kind {
                "text" => Some(SessionContentBlock::text(
                    item.get("text")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                )),
                "image" => Some(SessionContentBlock::image(
                    item.get("uri")
                        .or_else(|| item.get("data"))
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    item.get("mimeType")
                        .and_then(|value| value.as_str())
                        .map(String::from),
                )),
                "resource" | "resource_link" => Some(SessionContentBlock::resource(
                    item.get("uri")
                        .and_then(|value| value.as_str())
                        .map(String::from),
                    item.get("name")
                        .and_then(|value| value.as_str())
                        .map(String::from),
                    item.get("mimeType")
                        .and_then(|value| value.as_str())
                        .map(String::from),
                )),
                _ => None,
            }
        })
        .collect()
}

fn update_value(data: &serde_json::Value) -> Option<&serde_json::Value> {
    data.get("update")
}

fn update_field<'a>(data: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    update_value(data).and_then(|update| update.get(key))
}

fn acp_update_type(data: &serde_json::Value, fallback: Option<&str>) -> Option<String> {
    let update = update_value(data)?;
    let value = update
        .get("sessionUpdate")
        .or_else(|| update.get("session_update"))
        .and_then(|value| value.as_str())
        .or(fallback)?;
    Some(
        match value {
            "plan_update" => "plan",
            other => other,
        }
        .to_string(),
    )
}

fn role_id(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Thinking => "thinking",
        MessageRole::System => "system",
        MessageRole::ToolUse => "tool_use",
        MessageRole::ToolResult => "tool_result",
        MessageRole::Unknown => "unknown",
    }
}

fn event_text_for_id(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { text } => text.clone(),
        MessageContent::Blocks { blocks } => blocks
            .iter()
            .filter_map(|block| block.text.clone().or_else(|| block.uri.clone()))
            .collect::<Vec<_>>()
            .join("\n"),
        MessageContent::ToolUse { tool } => {
            format!("{}:{}", tool.name, tool.raw.clone().unwrap_or_default())
        }
        MessageContent::ToolResult { result } => result.text.clone(),
        MessageContent::Mixed { parts } => parts
            .iter()
            .map(event_text_for_id)
            .collect::<Vec<_>>()
            .join("\n"),
    }
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
    Path::new(&info.file_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| info.file_path.clone())
}

#[cfg(test)]
mod tests {
    use super::{message_events_from_history_acp_messages, project_key_for_path_or_name};
    use crate::agents::sources::types::{
        AgentKind, HistoryAcpMessage, MessageContent, ProjectRef, SessionSource, SourceKind,
        SourceLocation,
    };
    use crate::turns::{
        history_prompt_message, history_tool_call_message, history_tool_result_message,
    };
    use serde_json::Value;

    fn row(message: crate::agents::runtime::types::AcpProtocolMessage) -> HistoryAcpMessage {
        HistoryAcpMessage {
            message,
            timestamp: None,
            location: SourceLocation::file("/tmp/session.jsonl"),
            synthetic: true,
        }
    }

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
        let events = message_events_from_history_acp_messages(
            &source,
            vec![
                row(history_tool_call_message(
                    None,
                    "shell",
                    serde_json::json!({ "cmd": "cargo check" }),
                    None,
                )),
                row(history_tool_result_message(
                    None,
                    Value::String("ok ".repeat(1000)),
                    None,
                )),
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
    fn preserves_structured_message_blocks_for_indexing() {
        let source = SessionSource {
            agent: AgentKind::new("claude"),
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
        let events = message_events_from_history_acp_messages(
            &source,
            vec![row(history_prompt_message(
                vec![
                    serde_json::json!({ "type": "text", "text": "see" }),
                    serde_json::json!({
                        "type": "resource_link",
                        "uri": "file:///tmp/spec.md",
                        "name": "spec.md",
                        "mimeType": "text/markdown"
                    }),
                ],
                None,
            ))],
        );
        match &events[0].content {
            MessageContent::Blocks { blocks } => {
                assert_eq!(blocks.len(), 2);
                assert_eq!(blocks[1].kind, "resource");
                assert_eq!(blocks[1].uri.as_deref(), Some("file:///tmp/spec.md"));
            }
            other => panic!("expected structured blocks, got {other:?}"),
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
