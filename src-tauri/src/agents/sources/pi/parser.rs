use anyhow::Result;
use serde::Deserialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::agents::runtime::types::AcpProtocolMessage;
use crate::agents::sources::pi::text_message_event;
use crate::agents::sources::shared::attachment_text::clean_history_user_preview_text;
use crate::agents::sources::shared::convert::project_key_for_path_or_name;
use crate::agents::sources::shared::time::parse_iso;
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::{
    HistoryAcpMessage, MessageContent, MessageEvent, MessageRole, Metadata, SessionSource,
    SourceLocation, ToolResultEvent, ToolUseEvent,
};
use crate::app_paths;
use crate::models::{normalize_preview, Agent, SessionInfo};
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
    #[serde(default)]
    details: Option<serde_json::Value>,
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
                Err(error) => log::warn!("pi parse {} failed: {error}", path.display()),
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
        .filter(|message| message.role == "user")
        .filter_map(|message| text_from_content(&message.content))
        .filter_map(|text| clean_history_user_preview_text(&text))
        .next()
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
            "[pi-parser] no message events parsed from {}",
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
    if let Some(tool_name) = message
        .tool_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        metadata.insert(
            "tool_name".to_string(),
            serde_json::Value::String(tool_name.to_string()),
        );
    }
    if let Some(details) = message.details.clone() {
        metadata.insert("tool_details".to_string(), details);
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
        MessageContent::ToolResult { result } => {
            history_tool_result_message_from_event(event, result)
        }
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

fn history_tool_result_message_from_event(
    event: &MessageEvent,
    result: &ToolResultEvent,
) -> AcpProtocolMessage {
    let mut message = history_tool_result_message(
        tool_call_id_from_event(event),
        serde_json::Value::String(result.text.clone()),
        event.timestamp,
    );
    let Some(update) = message
        .data
        .get_mut("update")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return message;
    };
    if matches!(result.success, Some(false)) {
        update.insert(
            "status".to_string(),
            serde_json::Value::String("failed".to_string()),
        );
    }
    if let Some(tool_name) = event
        .metadata
        .get("tool_name")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| result.tool_name.clone())
    {
        update.insert(
            "title".to_string(),
            serde_json::Value::String(tool_name.clone()),
        );
        update.insert(
            "kind".to_string(),
            serde_json::Value::String(pi_tool_to_acp_kind(&tool_name).to_string()),
        );
    }
    let content = pi_tool_result_content_from_event(event);
    if !content.is_empty() {
        update.insert("content".to_string(), serde_json::Value::Array(content));
    }
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

fn pi_tool_result_content_from_event(event: &MessageEvent) -> Vec<serde_json::Value> {
    let tool_name = event
        .metadata
        .get("tool_name")
        .and_then(serde_json::Value::as_str);
    let Some(details) = event.metadata.get("tool_details") else {
        return Vec::new();
    };
    pi_tool_result_content_from_details(tool_name, details)
        .into_iter()
        .collect()
}

fn pi_tool_result_content_from_details(
    tool_name: Option<&str>,
    details: &serde_json::Value,
) -> Option<serde_json::Value> {
    let has_diff_payload = details
        .get("patch")
        .and_then(serde_json::Value::as_str)
        .is_some()
        || details
            .get("diff")
            .and_then(serde_json::Value::as_str)
            .is_some();
    if !has_diff_payload {
        return None;
    }
    if let Some(tool_name) = tool_name {
        let normalized = tool_name.trim().to_ascii_lowercase();
        if normalized != "edit" && normalized != "patch" && normalized != "apply_patch" {
            return None;
        }
    }
    let mut out = serde_json::Map::new();
    out.insert(
        "type".to_string(),
        serde_json::Value::String("diff".to_string()),
    );
    let path = details
        .get("path")
        .or_else(|| details.get("filePath"))
        .or_else(|| details.get("file_path"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string);
    if let Some(path) = path.as_deref() {
        out.insert(
            "path".to_string(),
            serde_json::Value::String(path.to_string()),
        );
    }
    if let Some(patch) = details
        .get("patch")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        let patch =
            normalize_pi_line_diff(path.as_deref(), patch).unwrap_or_else(|| patch.to_string());
        out.insert("patch".to_string(), serde_json::Value::String(patch));
    }
    if let Some(diff) = details
        .get("diff")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        let diff =
            normalize_pi_line_diff(path.as_deref(), diff).unwrap_or_else(|| diff.to_string());
        out.insert("diff".to_string(), serde_json::Value::String(diff));
        if let Some(diff) = out.get("diff").cloned() {
            out.insert("detail".to_string(), diff);
        }
    }
    if let Some(first_changed_line) = details
        .get("firstChangedLine")
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value > 0)
    {
        out.insert(
            "meta".to_string(),
            serde_json::json!({
                "firstChangedLine": first_changed_line,
                "startLine": first_changed_line,
                "line": first_changed_line,
            }),
        );
    }
    Some(serde_json::Value::Object(out))
}

/// Pi/OMP edit tools often return a compact line-numbered diff instead of a
/// unified diff (`+12|new line`, ` 12|context`). Convert it before the shared
/// ACP renderer sees the tool result so all agents get the same diff UI.
fn normalize_pi_line_diff(path: Option<&str>, value: &str) -> Option<String> {
    #[derive(Clone)]
    struct Line {
        marker: char,
        number: usize,
        text: String,
    }

    let mut lines = Vec::new();
    for raw in value.lines().filter(|line| !line.trim().is_empty()) {
        let mut chars = raw.chars();
        let marker = chars.next()?;
        if !matches!(marker, ' ' | '+' | '-') {
            return None;
        }
        let numbered = chars.as_str();
        let (number, text) = numbered.split_once('|')?;
        let number = number.trim().parse::<usize>().ok()?;
        lines.push(Line {
            marker,
            number,
            text: text.to_string(),
        });
    }
    if lines.is_empty() {
        return None;
    }

    let mut groups: Vec<Vec<Line>> = Vec::new();
    for line in lines {
        let starts_new_group = groups.is_empty()
            || groups.last().is_some_and(|group| {
                let previous = group.last().expect("group is non-empty");
                line.number > previous.number.saturating_add(8)
            });
        if starts_new_group {
            groups.push(Vec::new());
        }
        groups.last_mut().expect("group created").push(line);
    }

    let display_path = path
        .unwrap_or("file")
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    let mut output = format!("--- a/{display_path}\n+++ b/{display_path}\n");
    for (index, group) in groups.iter().enumerate() {
        let old_start = group.first().map(|line| line.number).unwrap_or(1);
        let new_start = old_start;
        let old_count = group.iter().filter(|line| line.marker != '+').count();
        let new_count = group.iter().filter(|line| line.marker != '-').count();
        if index > 0 {
            output.push('\n');
        }
        output.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        for (line_index, line) in group.iter().enumerate() {
            if line_index > 0 {
                output.push('\n');
            }
            if line.marker == ' ' {
                output.push(' ');
            } else {
                output.push(line.marker);
            }
            output.push_str(&line.text);
        }
    }
    Some(output)
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

    #[test]
    fn pi_jsonl_history_extracts_edit_diff_details_into_tool_result_update() {
        let path = unique_temp_jsonl_path("pi-edit-history");
        let content = concat!(
            "{\"type\":\"session\",\"id\":\"session-1\",\"timestamp\":\"2026-06-25T18:48:15.425Z\",\"cwd\":\"/tmp/project\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"改一下文件\"}],\"timestamp\":1000}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"toolCall\",\"id\":\"call-edit-1\",\"name\":\"edit\",\"arguments\":{\"path\":\"world-cup.md\",\"edits\":[{\"oldText\":\"old\",\"newText\":\"new\"}]}}],\"timestamp\":1001}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"toolCallId\":\"call-edit-1\",\"toolName\":\"edit\",\"content\":[{\"type\":\"text\",\"text\":\"Successfully replaced 1 block(s) in world-cup.md.\"}],\"details\":{\"diff\":\"-1 old\\n+1 new\",\"patch\":\"--- world-cup.md\\n+++ world-cup.md\\n@@ -1 +1 @@\\n-old\\n+new\\n\",\"firstChangedLine\":1},\"isError\":false,\"timestamp\":1002}}\n"
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
        let history = message_events_to_history_acp_messages(events);
        let update = history[2]
            .message
            .data
            .get("update")
            .and_then(serde_json::Value::as_object)
            .expect("tool result update");
        assert_eq!(
            update.get("title").and_then(serde_json::Value::as_str),
            Some("edit")
        );
        assert_eq!(
            update.get("kind").and_then(serde_json::Value::as_str),
            Some("edit")
        );
        let content = update
            .get("content")
            .and_then(serde_json::Value::as_array)
            .expect("diff content");
        assert_eq!(
            content[0]["type"],
            serde_json::Value::String("diff".to_string())
        );
        assert_eq!(
            content[0]["patch"].as_str(),
            Some("--- world-cup.md\n+++ world-cup.md\n@@ -1 +1 @@\n-old\n+new\n")
        );
        assert_eq!(content[0]["detail"].as_str(), Some("-1 old\n+1 new"));

        let turns = crate::turns::session_history_turns_from_acp_messages(&history);
        let file_edit = turns[0]
            .blocks
            .iter()
            .find(|block| block.update_type.as_deref() == Some("file_edit"))
            .expect("file_edit block");
        let data = file_edit.data.as_ref().expect("file_edit data");
        assert_eq!(
            data["edits"][0]["path"],
            serde_json::Value::String("world-cup.md".to_string())
        );
        assert_eq!(
            data["edits"][0]["patch"].as_str(),
            Some("--- world-cup.md\n+++ world-cup.md\n@@ -1 +1 @@\n-old\n+new\n")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pi_line_numbered_diff_is_normalized_to_unified_patch() {
        let patch = normalize_pi_line_diff(
            Some("apps/report.html"),
            " 10|before\n-11|old\n+11|new\n 12|after\n\n 30|later\n+31|added",
        )
        .expect("line-numbered patch");
        assert!(patch
            .starts_with("--- a/apps/report.html\n+++ b/apps/report.html\n@@ -10,3 +10,3 @@\n"));
        assert!(patch.contains("\n before\n-old\n+new\n after\n"));
        assert!(patch.contains("@@ -30,1 +30,2 @@\n later\n+added"));
    }

    fn unique_temp_jsonl_path(prefix: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.jsonl"))
    }
}
