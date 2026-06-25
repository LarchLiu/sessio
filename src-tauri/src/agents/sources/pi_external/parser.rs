use anyhow::Result;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::agents::runtime::types::AcpProtocolMessage;
use crate::agents::sources::pi_external::text_message_event;
use crate::agents::sources::shared::convert::project_key_for_path_or_name;
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::{
    HistoryAcpMessage, MessageContent, MessageEvent, MessageRole, Metadata, SessionSource,
    SourceLocation, ToolResultEvent, ToolUseEvent,
};
use crate::app_paths;
use crate::models::{Agent, SessionInfo, normalize_preview};
use crate::turns::{
    history_assistant_message, history_prompt_message, history_thought_message,
    history_tool_call_message_with_kind, history_tool_result_message,
};

#[derive(Debug, Deserialize)]
struct PiSessionEntry {
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default, rename = "modelId")]
    _model_id: Option<String>,
    #[serde(default, rename = "thinkingLevel")]
    _thinking_level: Option<String>,
    #[serde(default)]
    message: Option<PiMessageEnvelope>,
}

#[derive(Debug, Deserialize)]
struct PiMessageEnvelope {
    role: String,
    #[serde(default)]
    content: Vec<PiMessageContent>,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default, rename = "toolCallId")]
    tool_call_id: Option<String>,
    #[serde(default, rename = "toolName")]
    tool_name: Option<String>,
    #[serde(default, rename = "isError")]
    is_error: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct PiMessageContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thinking: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<serde_json::Value>,
}

pub fn sessions_root() -> Result<PathBuf> {
    app_paths::pi_agent_sessions_dir()
}

pub fn root_dir() -> Result<Option<PathBuf>> {
    let root = sessions_root()?;
    if root.exists() {
        Ok(Some(root))
    } else {
        Ok(None)
    }
}

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let Some(root) = root_dir()? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for project_entry in fs::read_dir(&root)? {
        let project_entry = project_entry?;
        if !project_entry.file_type()?.is_dir() {
            continue;
        }
        for entry in fs::read_dir(project_entry.path())? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            match parse_session_file(&path) {
                Ok(Some(info)) => out.push(info),
                Ok(None) => {}
                Err(error) => log::warn!("pi external parse {} failed: {error}", path.display()),
            }
        }
    }
    Ok(out)
}

pub fn parse_session_file(path: &Path) -> Result<Option<SessionInfo>> {
    let entries = read_entries(path)?;
    if entries.is_empty() {
        return Ok(None);
    }
    let session_entry = entries.iter().find(|entry| entry.entry_type == "session");
    let message_entries: Vec<&PiSessionEntry> = entries
        .iter()
        .filter(|entry| entry.entry_type == "message" && entry.message.is_some())
        .collect();
    if message_entries.is_empty() {
        return Ok(None);
    }
    let session_id = session_id_from_entries(path, session_entry);
    let project_path = session_entry
        .and_then(|entry| entry.cwd.clone())
        .or_else(|| infer_project_path_from_parent(path));
    let project_name = project_path
        .as_deref()
        .and_then(|project| {
            Path::new(project)
                .file_name()
                .and_then(|value| value.to_str())
                .map(String::from)
        })
        .or_else(|| project_dir_name_from_path(path));
    let started_at = message_entries
        .iter()
        .filter_map(|entry| entry.message.as_ref().and_then(|message| message.timestamp))
        .min()
        .or_else(|| {
            session_entry
                .and_then(|entry| entry.timestamp.as_deref())
                .and_then(parse_iso)
        });
    let updated_at = message_entries
        .iter()
        .filter_map(|entry| entry.message.as_ref().and_then(|message| message.timestamp))
        .max()
        .or_else(|| {
            fs::metadata(path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_to_millis)
        });
    let first_user_message = message_entries
        .iter()
        .filter_map(|entry| entry.message.as_ref())
        .find(|message| message.role == "user")
        .and_then(|message| text_from_content(&message.content))
        .map(|text| normalize_preview(&text));
    let file_size = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    Ok(Some(SessionInfo {
        id: session_id,
        agent: Agent::Pi,
        forked_from_agent: None,
        forked_from_id: None,
        project_path,
        project_name,
        started_at,
        updated_at,
        message_count: message_entries.len(),
        rename_title: None,
        title: first_user_message.clone(),
        first_user_message,
        file_path: path.to_string_lossy().into_owned(),
        file_size,
        partial: false,
        available: true,
        archived: false,
        origin: crate::models::SessionOrigin::Chat,
        scheduled_task_id: None,
        is_auxiliary: false,
        subagents: Vec::new(),
    }))
}

pub fn read_message_events(path: &Path, source: &SessionSource) -> Result<Vec<MessageEvent>> {
    let entries = read_entries(path)?;
    let mut out = Vec::new();
    let file_path = path.to_string_lossy().to_string();
    let mut turn_index = 0usize;

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    let mut byte_offset: u64 = 0;
    let mut line_number: u64 = 0;

    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        line_number += 1;
        let line_start_byte = byte_offset;
        let line_end_byte = byte_offset + n as u64;
        byte_offset = line_end_byte;
        let Ok(line_str) = std::str::from_utf8(&buf) else {
            continue;
        };
        if line_str.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<PiSessionEntry>(line_str) else {
            continue;
        };
        if entry.entry_type != "message" {
            continue;
        }
        let Some(message) = entry.message else {
            continue;
        };
        let location = SourceLocation {
            file_path: file_path.clone(),
            line_start: Some(line_number),
            line_end: Some(line_number),
            byte_start: Some(line_start_byte),
            byte_end: Some(line_end_byte),
        };
        for event in pi_message_events(source, turn_index, &message, location) {
            out.push(event);
            turn_index += 1;
        }
    }

    if out.is_empty() && !entries.is_empty() {
        log::debug!(
            "[pi-external-parser] no message events parsed from {}",
            path.display()
        );
    }
    Ok(out)
}

fn pi_message_events(
    source: &SessionSource,
    turn_index: usize,
    message: &PiMessageEnvelope,
    location: SourceLocation,
) -> Vec<MessageEvent> {
    let timestamp = message.timestamp;
    match message.role.as_str() {
        "user" => text_from_content(&message.content)
            .map(|text| {
                text_message_event(
                    source,
                    turn_index,
                    timestamp,
                    location,
                    text,
                    MessageRole::User,
                )
            })
            .into_iter()
            .collect(),
        "assistant" | "system" => assistant_message_events(
            source,
            turn_index,
            timestamp,
            location,
            &message.content,
            message.role == "system",
        ),
        "toolResult" => tool_result_event(source, turn_index, timestamp, location, message)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

fn assistant_message_events(
    source: &SessionSource,
    base_turn_index: usize,
    timestamp: Option<i64>,
    location: SourceLocation,
    content: &[PiMessageContent],
    system: bool,
) -> Vec<MessageEvent> {
    let mut out = Vec::new();
    for part in content {
        let role = match part.kind.as_str() {
            "thinking" => MessageRole::Thinking,
            "text" if system => MessageRole::System,
            "text" => MessageRole::Assistant,
            "toolCall" => MessageRole::ToolUse,
            _ => continue,
        };
        let event_index = base_turn_index + out.len();
        let content = match role {
            MessageRole::Thinking => {
                let Some(text) = part
                    .thinking
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                else {
                    continue;
                };
                MessageContent::Blocks {
                    blocks: crate::models::text_content_blocks(text),
                }
            }
            MessageRole::Assistant | MessageRole::System => {
                let Some(text) = part
                    .text
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                else {
                    continue;
                };
                MessageContent::Blocks {
                    blocks: crate::models::text_content_blocks(text),
                }
            }
            MessageRole::ToolUse => MessageContent::ToolUse {
                tool: ToolUseEvent {
                    name: part.name.clone().unwrap_or_else(|| "tool".to_string()),
                    input: part.arguments.clone(),
                    raw: part.arguments.as_ref().map(|value| {
                        value
                            .as_str()
                            .map(String::from)
                            .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
                    }),
                },
            },
            _ => continue,
        };
        let mut metadata = Metadata::default();
        if role == MessageRole::ToolUse {
            if let Some(id) = part.id.as_deref().filter(|value| !value.trim().is_empty()) {
                metadata.insert(
                    "tool_call_id".to_string(),
                    serde_json::Value::String(id.to_string()),
                );
            }
        }
        out.push(MessageEvent {
            source: source.clone(),
            event_id: Some(format!(
                "{}:{event_index}:{:?}:{}",
                source.file_path,
                role,
                part.id.as_deref().unwrap_or(part.kind.as_str())
            )),
            turn_index: event_index,
            role,
            content,
            timestamp,
            location: location.clone(),
            metadata,
        });
    }
    out
}

fn tool_result_event(
    source: &SessionSource,
    turn_index: usize,
    timestamp: Option<i64>,
    location: SourceLocation,
    message: &PiMessageEnvelope,
) -> Option<MessageEvent> {
    let text = text_from_content(&message.content)?;
    let mut metadata = Metadata::default();
    if let Some(id) = message
        .tool_call_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        metadata.insert(
            "tool_call_id".to_string(),
            serde_json::Value::String(id.to_string()),
        );
    }
    Some(MessageEvent {
        source: source.clone(),
        event_id: Some(format!(
            "{}:{turn_index}:tool_result:{}",
            source.file_path,
            message.tool_call_id.as_deref().unwrap_or("tool")
        )),
        turn_index,
        role: MessageRole::ToolResult,
        content: MessageContent::ToolResult {
            result: ToolResultEvent {
                tool_name: message.tool_name.clone(),
                exit_code: None,
                success: message.is_error.map(|value| !value),
                text,
                output_hash: None,
            },
        },
        timestamp,
        location,
        metadata,
    })
}

pub fn message_events_to_history_acp_messages(events: Vec<MessageEvent>) -> Vec<HistoryAcpMessage> {
    events
        .into_iter()
        .map(|event| {
            let message = history_message_from_event(&event);
            HistoryAcpMessage {
                message,
                timestamp: event.timestamp,
                location: event.location,
                synthetic: true,
            }
        })
        .collect()
}

fn history_message_from_event(event: &MessageEvent) -> AcpProtocolMessage {
    let mut message = match &event.content {
        MessageContent::ToolUse { tool } => history_tool_call_message_with_kind(
            tool_call_id_from_event(event),
            tool.name.clone(),
            pi_tool_to_acp_kind(&tool.name),
            tool.input.clone().unwrap_or(serde_json::Value::Null),
            event.timestamp,
        ),
        MessageContent::ToolResult { result } => history_tool_result_message(
            tool_call_id_from_event(event),
            serde_json::Value::String(result.text.clone()),
            event.timestamp,
        ),
        _ => {
            let text = text_from_message_content(&event.content);
            match event.role {
                MessageRole::User => history_prompt_message(
                    vec![serde_json::json!({ "type": "text", "text": text })],
                    event.timestamp,
                ),
                MessageRole::Thinking => history_thought_message(text, event.timestamp),
                MessageRole::Assistant | MessageRole::System => {
                    history_assistant_message(text, event.timestamp)
                }
                _ => history_assistant_message(text, event.timestamp),
            }
        }
    };
    message.acp_session_id = Some(event.source.session_id.clone());
    message.turn_id = event.event_id.clone();
    message
}

fn text_from_message_content(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { text } => text.clone(),
        MessageContent::Blocks { blocks } => blocks
            .iter()
            .filter_map(|block| block.text.clone())
            .collect::<String>(),
        MessageContent::ToolUse { tool } => tool.raw.clone().unwrap_or_default(),
        MessageContent::ToolResult { result } => result.text.clone(),
        MessageContent::Mixed { parts } => parts
            .iter()
            .map(text_from_message_content)
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn tool_call_id_from_event(event: &MessageEvent) -> Option<String> {
    event
        .metadata
        .get("tool_call_id")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

fn pi_tool_to_acp_kind(tool_name: &str) -> &'static str {
    match tool_name.trim().to_ascii_lowercase().as_str() {
        "bash" | "shell" | "terminal" | "run_shell_command" | "shell_command" | "exec_command" => {
            "execute"
        }
        "read" | "read_file" | "readfile" | "readfiletool" | "read_file_tool" => "read",
        "write" | "write_file" | "writefile" | "edit" | "replace" | "patch" | "apply_patch"
        | "multi_edit" | "multiedit" | "notebook_edit" => "edit",
        "delete" | "remove" | "remove_file" | "delete_file" => "delete",
        "move" | "rename" | "move_file" => "move",
        "grep" | "grep_search" | "search" | "searchtext" | "tool_search" | "toolsearch"
        | "glob" | "find_files" | "web_search" | "websearch" => "search",
        "web_fetch" | "webfetch" | "fetch" => "fetch",
        "todo_write" | "todowrite" => "task_list",
        _ => "tool_call",
    }
}

fn read_entries(path: &Path) -> Result<Vec<PiSessionEntry>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<PiSessionEntry>(&line) {
            out.push(entry);
        }
    }
    Ok(out)
}

fn text_from_content(content: &[PiMessageContent]) -> Option<String> {
    let text = content
        .iter()
        .filter(|part| part.kind == "text")
        .filter_map(|part| part.text.as_deref())
        .collect::<String>();
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn session_id_from_entries(path: &Path, session_entry: Option<&PiSessionEntry>) -> String {
    session_entry
        .and_then(|entry| entry.id.clone())
        .or_else(|| {
            path.file_stem()
                .and_then(|value| value.to_str())
                .map(String::from)
        })
        .unwrap_or_default()
}

fn infer_project_path_from_parent(path: &Path) -> Option<String> {
    let dir = path.parent()?.file_name()?.to_str()?;
    Some(dir.trim_matches('-').replace("--", "/").replace('-', "/"))
}

fn project_dir_name_from_path(path: &Path) -> Option<String> {
    let project_path = infer_project_path_from_parent(path);
    let project_name = project_path
        .as_deref()
        .and_then(|value| Path::new(value).file_name().and_then(|name| name.to_str()))
        .map(String::from);
    Some(project_key_for_path_or_name(
        project_path.as_deref(),
        project_name.as_deref(),
    ))
}

fn parse_iso(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sources::types::{AgentKind, SourceKind};

    #[test]
    fn pi_jsonl_history_preserves_thinking_tools_and_final_text() {
        let path = unique_temp_jsonl_path("pi-history");
        let content = concat!(
            "{\"type\":\"session\",\"id\":\"session-1\",\"timestamp\":\"2026-06-25T18:48:15.425Z\",\"cwd\":\"/tmp/project\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"都有什么文档\"}],\"timestamp\":1000}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"Need to inspect docs\"},{\"type\":\"toolCall\",\"id\":\"call-1\",\"name\":\"bash\",\"arguments\":{\"command\":\"find . -maxdepth 2 -type f\"}}],\"timestamp\":1001}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"toolCallId\":\"call-1\",\"toolName\":\"bash\",\"content\":[{\"type\":\"text\",\"text\":\"joke-draft.md\\n\"}],\"isError\":false,\"timestamp\":1002}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"thinking\",\"thinking\":\"\"},{\"type\":\"text\",\"text\":\"当前目录里我只看到 1 个文档：\\n\\n- `joke-draft.md`\"}],\"timestamp\":1003}}\n",
        );
        std::fs::write(&path, content).expect("write temp pi jsonl");
        let source = SessionSource {
            agent: AgentKind::new("pi"),
            session_id: "session-1".to_string(),
            scope: "test".to_string(),
            file_path: path.to_string_lossy().to_string(),
            project: None,
            source_kind: SourceKind::MainSession,
            metadata: Default::default(),
        };

        let events = read_message_events(&path, &source).expect("read pi events");
        let roles = events.iter().map(|event| event.role).collect::<Vec<_>>();
        assert_eq!(
            roles,
            vec![
                MessageRole::User,
                MessageRole::Thinking,
                MessageRole::ToolUse,
                MessageRole::ToolResult,
                MessageRole::Assistant,
            ]
        );
        assert_eq!(
            events[2]
                .metadata
                .get("tool_call_id")
                .and_then(serde_json::Value::as_str),
            Some("call-1")
        );
        assert_eq!(
            events[3]
                .metadata
                .get("tool_call_id")
                .and_then(serde_json::Value::as_str),
            Some("call-1")
        );

        let history = message_events_to_history_acp_messages(events);
        let update_types = history
            .iter()
            .filter_map(|message| message.message.update_type.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(
            update_types,
            vec![
                "agent_thought_chunk",
                "tool_call",
                "tool_call_update",
                "agent_message_chunk"
            ]
        );
        assert_eq!(
            history[2].message.data.get("toolCallId"),
            history[3].message.data.get("toolCallId")
        );
        assert_eq!(
            history[2]
                .message
                .data
                .get("update")
                .and_then(|value| value.get("kind")),
            Some(&serde_json::Value::String("execute".to_string()))
        );
        assert!(
            history[4]
                .message
                .data
                .to_string()
                .contains("joke-draft.md"),
            "final assistant text should be present in history"
        );

        let _ = std::fs::remove_file(path);
    }

    fn unique_temp_jsonl_path(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.jsonl"))
    }
}
