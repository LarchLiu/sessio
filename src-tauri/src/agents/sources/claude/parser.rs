use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::agents::runtime::types::AcpProtocolMessage;
use crate::agents::sources::shared::attachment_text::{
    clean_history_preview_candidate_text, clean_history_user_preview_text,
};
use crate::agents::sources::shared::cross_context::cross_context_lineage_from_payload;
use crate::agents::sources::shared::time::parse_iso;
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::{HistoryAcpMessage, SourceLocation};
use crate::models::{normalize_preview, Agent, SessionInfo, SubagentInfo};
use crate::turns::{
    history_assistant_message, history_content_update, history_permission_request_message,
    history_prompt_message, history_session_update_message, history_thought_message,
    history_todo_message, history_tool_call_message, history_tool_result_message,
    history_user_message, HistoryPermissionRequest,
};
use serde_json::Value;

const REVERSE_METADATA_CHUNK_SIZE: u64 = 16 * 1024;

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let root = match root_dir()? {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let project_dir = entry.path();
        match scan_project_dir(&project_dir) {
            Ok(mut group) => out.append(&mut group),
            Err(e) => log::warn!("claude scan {} failed: {e}", project_dir.display()),
        }
    }
    Ok(out)
}

pub fn root_dir() -> Result<Option<PathBuf>> {
    let home = dirs::home_dir().context("no home dir")?;
    let root = home.join(".claude").join("projects");
    if root.exists() {
        Ok(Some(root))
    } else {
        Ok(None)
    }
}

pub fn scan_project_dir(project_dir: &Path) -> Result<Vec<SessionInfo>> {
    if !project_dir.is_dir() {
        return Ok(Vec::new());
    }
    let index = read_index(&project_dir.join("sessions-index.json")).ok();
    let mut group: Vec<SessionInfo> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    for f in fs::read_dir(project_dir)? {
        let f = f?;
        let p = f.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        match parse_session(&p) {
            Ok(Some(info)) => {
                seen_ids.insert(info.id.clone());
                group.push(info);
            }
            Ok(None) => {}
            Err(e) => log::warn!("claude parse {} failed: {e}", p.display()),
        }
    }

    if let Some(idx) = &index {
        for entry in &idx.entries {
            if seen_ids.contains(&entry.session_id) {
                continue;
            }
            if let Some(info) = info_from_index(entry, idx, project_dir) {
                seen_ids.insert(info.id.clone());
                group.push(info);
            }
        }
    }

    for sub_entry in fs::read_dir(project_dir)?.flatten() {
        if !sub_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let sub_id = match sub_entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if seen_ids.contains(&sub_id) {
            continue;
        }
        let subagents_dir = sub_entry.path().join("subagents");
        if !subagents_dir.is_dir() {
            continue;
        }
        let subagents = read_subagents(&subagents_dir);
        if subagents.is_empty() {
            continue;
        }
        let earliest = subagents.iter().filter_map(|s| s.started_at).min();
        let latest = subagents.iter().filter_map(|s| s.updated_at).max();
        seen_ids.insert(sub_id.clone());
        group.push(SessionInfo {
            id: sub_id,
            agent: Agent::Claude,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: None,
            project_name: None,
            started_at: earliest,
            updated_at: latest,
            message_count: 0,
            rename_title: None,
            title: None,
            first_user_message: None,
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: false,
            archived: true,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents,
        });
    }

    let index_cwd = index.as_ref().and_then(|i| {
        i.original_path
            .clone()
            .or_else(|| i.entries.iter().find_map(|e| e.project_path.clone()))
    });
    let shared_cwd = index_cwd.or_else(|| group.iter().find_map(|s| s.project_path.clone()));
    let dir_name = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(String::from);
    let shared_project_name = shared_cwd.as_deref().and_then(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
    });
    for s in group.iter_mut() {
        if s.project_path.is_none() {
            s.project_path = shared_cwd.clone();
        }
        if s.project_name.is_none() {
            s.project_name = shared_project_name.clone().or_else(|| dir_name.clone());
        }
        let subagents_dir = project_dir.join(&s.id).join("subagents");
        if subagents_dir.is_dir() {
            s.subagents = read_subagents(&subagents_dir);
        }
    }
    Ok(group)
}

// Single-file reindex path: parse only the given jsonl and fill in the
// project-shared metadata (cwd/name, sibling subagents). Lets the watcher
// react to a single jsonl write without rescanning every session in the
// project directory.
pub fn parse_single_file(path: &Path) -> Result<Option<SessionInfo>> {
    let Some(parent) = path.parent() else {
        return Ok(None);
    };
    let path_buf = path.to_path_buf();
    let mut info = match parse_session(&path_buf)? {
        Some(i) => i,
        None => return Ok(None),
    };

    let index = read_index(&parent.join("sessions-index.json")).ok();
    if info.project_path.is_none() {
        info.project_path = index.as_ref().and_then(|i| {
            i.original_path
                .clone()
                .or_else(|| i.entries.iter().find_map(|e| e.project_path.clone()))
        });
    }
    if info.project_name.is_none() {
        let from_cwd = info.project_path.as_deref().and_then(|p| {
            Path::new(p)
                .file_name()
                .and_then(|s| s.to_str())
                .map(String::from)
        });
        let dir_name = parent
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from);
        info.project_name = from_cwd.or(dir_name);
    }

    let subagents_dir = parent.join(&info.id).join("subagents");
    if subagents_dir.is_dir() {
        info.subagents = read_subagents(&subagents_dir);
    }

    Ok(Some(info))
}

pub fn read_history_acp_messages_with_locations(path: &Path) -> Result<Vec<HistoryAcpMessage>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut out = Vec::new();
    let file_path = path.to_string_lossy().to_string();
    let mut buf = Vec::new();
    let mut byte_offset: u64 = 0;
    let mut line_number: u64 = 0;
    let mut edit_tools: HashMap<String, ClaudeEditTool> = HashMap::new();

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
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line_str) else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso);
        let location = SourceLocation {
            file_path: file_path.clone(),
            line_start: Some(line_number),
            line_end: Some(line_number),
            byte_start: Some(line_start_byte),
            byte_end: Some(line_end_byte),
        };
        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if t == "permission-mode" {
            if let Some(mode) = v.get("permissionMode").and_then(|x| x.as_str()) {
                out.push(HistoryAcpMessage {
                    message: history_session_update_message(
                        "current_mode",
                        serde_json::json!({
                            "sessionUpdate": "current_mode_update",
                            "currentModeId": mode,
                            "raw": v,
                        }),
                        ts,
                    ),
                    timestamp: ts,
                    location,
                    synthetic: true,
                });
            }
            continue;
        }
        if let Some(permission_messages) = claude_permission_event(&v, ts) {
            for message in permission_messages {
                out.push(HistoryAcpMessage {
                    message,
                    timestamp: ts,
                    location: location.clone(),
                    synthetic: true,
                });
            }
        }
        if t != "user" && t != "assistant" {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let role_raw = msg
            .get("role")
            .and_then(|x| x.as_str())
            .unwrap_or(t)
            .to_string();
        let tool_result_meta = v.get("toolUseResult");
        for message in expand_message(&role_raw, msg, ts, &mut edit_tools, tool_result_meta) {
            out.push(HistoryAcpMessage {
                message,
                timestamp: ts,
                location: location.clone(),
                synthetic: true,
            });
        }
    }
    Ok(out)
}

// Convert a single Anthropic message into one or more synthetic ACP messages:
// - assistant text -> {role:"assistant"}
// - assistant tool_use -> {role:"tool_call"}
// - user tool_result -> {role:"tool_result"}
// - user text -> {role:"user"}
#[derive(Debug, Clone)]
struct ClaudeEditTool {
    name: String,
    input: serde_json::Value,
}

fn expand_message(
    role_raw: &str,
    msg: &serde_json::Value,
    ts: Option<i64>,
    edit_tools: &mut HashMap<String, ClaudeEditTool>,
    tool_result_meta: Option<&serde_json::Value>,
) -> Vec<AcpProtocolMessage> {
    let content = match msg.get("content") {
        Some(c) => c,
        None => return Vec::new(),
    };

    if let Some(s) = content.as_str() {
        let text = s.to_string();
        if text.trim().is_empty() {
            return Vec::new();
        }
        return vec![history_role_message(role_raw, text, ts)];
    }

    let arr = match content.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    let flush_text = |out: &mut Vec<AcpProtocolMessage>, text_parts: &mut Vec<String>| {
        if text_parts.is_empty() {
            return;
        }
        let cleaned = text_parts.join("\n");
        text_parts.clear();
        if cleaned.trim().is_empty() {
            return;
        }
        out.push(history_role_message(role_raw, cleaned, ts));
    };

    for item in arr {
        let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                    text_parts.push(t.to_string());
                }
            }
            "image" => {
                flush_text(&mut out, &mut text_parts);
                if let Some(image) = claude_image_item_to_content(item) {
                    out.push(history_role_content_message(role_raw, vec![image], ts));
                }
            }
            "thinking" => {
                flush_text(&mut out, &mut text_parts);
                if let Some(t) = item.get("thinking").and_then(|x| x.as_str()) {
                    if !t.trim().is_empty() {
                        out.push(history_thought_message(t.to_string(), ts));
                    }
                }
            }
            "reasoning" => {
                flush_text(&mut out, &mut text_parts);
                let reasoning_text = item
                    .get("reasoning")
                    .or_else(|| item.get("thinking"))
                    .or_else(|| item.get("text"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();
                if !reasoning_text.is_empty() {
                    out.push(history_thought_message(reasoning_text.to_string(), ts));
                }
            }
            "tool_use" => {
                flush_text(&mut out, &mut text_parts);
                let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                if matches!(name, "Write" | "Edit" | "MultiEdit") {
                    if let Some(id) = item.get("id").and_then(|x| x.as_str()) {
                        edit_tools.insert(
                            id.to_string(),
                            ClaudeEditTool {
                                name: name.to_string(),
                                input: item.get("input").cloned().unwrap_or_default(),
                            },
                        );
                    }
                }
                if name == "TodoWrite" {
                    if let Some(todos) = item.get("input").and_then(|i| i.get("todos")) {
                        out.push(history_todo_message(
                            todos.clone(),
                            ts,
                            item.get("id")
                                .or_else(|| item.get("tool_use_id"))
                                .or_else(|| item.get("toolUseId"))
                                .or_else(|| item.get("tool_useId"))
                                .or_else(|| item.get("toolId"))
                                .or_else(|| item.get("tool_id"))
                                .and_then(|x| x.as_str())
                                .map(String::from),
                        ));
                        continue;
                    }
                }
                out.push(history_tool_call_message(
                    item.get("id")
                        .or_else(|| item.get("tool_use_id"))
                        .or_else(|| item.get("toolUseId"))
                        .or_else(|| item.get("tool_useId"))
                        .or_else(|| item.get("toolId"))
                        .or_else(|| item.get("tool_id"))
                        .and_then(|x| x.as_str())
                        .map(String::from),
                    name,
                    item.get("input").cloned().unwrap_or(Value::Null),
                    ts,
                ));
            }
            "tool_result" => {
                flush_text(&mut out, &mut text_parts);
                let output = extract_tool_result_value(item);
                if !is_empty_tool_result(&output) {
                    let tool_use_id = item
                        .get("tool_use_id")
                        .or_else(|| item.get("toolUseId"))
                        .or_else(|| item.get("tool_useId"))
                        .or_else(|| item.get("toolUseID"))
                        .or_else(|| item.get("toolId"))
                        .or_else(|| item.get("tool_id"))
                        .or_else(|| item.get("id"))
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    out.push(history_tool_result_message(tool_use_id.clone(), output, ts));
                    if let Some(id) = tool_use_id {
                        if let Some(edit) = edit_tools.remove(&id) {
                            if let Some(summary) =
                                claude_file_edit_message(&edit, ts, tool_result_meta)
                            {
                                out.push(summary);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    flush_text(&mut out, &mut text_parts);

    out
}

fn history_role_message(role: &str, text: String, timestamp: Option<i64>) -> AcpProtocolMessage {
    match role {
        "user" => history_user_message(text, timestamp),
        "assistant" => history_assistant_message(text, timestamp),
        "thinking" => history_thought_message(text, timestamp),
        _ => history_assistant_message(text, timestamp),
    }
}

fn history_role_content_message(
    role: &str,
    content: Vec<Value>,
    timestamp: Option<i64>,
) -> AcpProtocolMessage {
    match role {
        "user" => history_prompt_message(content, timestamp),
        "assistant" => history_content_update("agent_message_chunk", content, timestamp),
        "thinking" => history_content_update("agent_thought_chunk", content, timestamp),
        _ => history_content_update("agent_message_chunk", content, timestamp),
    }
}

fn extract_tool_result_text(item: &serde_json::Value) -> String {
    let content = match item.get("content") {
        Some(c) => c,
        None => return String::new(),
    };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for sub in arr {
            if sub.get("type").and_then(|x| x.as_str()) == Some("text") {
                if let Some(t) = sub.get("text").and_then(|x| x.as_str()) {
                    parts.push(t.to_string());
                }
            } else if let Some(t) = sub.as_str() {
                parts.push(t.to_string());
            }
        }
        return parts.join("\n");
    }
    String::new()
}

fn extract_tool_result_value(item: &serde_json::Value) -> Value {
    let Some(content) = item.get("content") else {
        return Value::Null;
    };
    if let Some(arr) = content.as_array() {
        let blocks = arr
            .iter()
            .filter_map(|sub| {
                if sub.get("type").and_then(|x| x.as_str()) == Some("image") {
                    return claude_image_item_to_content(sub);
                }
                if sub.get("type").and_then(|x| x.as_str()) == Some("text") {
                    return sub
                        .get("text")
                        .and_then(|x| x.as_str())
                        .filter(|text| !text.trim().is_empty())
                        .map(|text| serde_json::json!({ "type": "text", "text": text }));
                }
                sub.as_str()
                    .filter(|text| !text.trim().is_empty())
                    .map(|text| serde_json::json!({ "type": "text", "text": text }))
            })
            .collect::<Vec<_>>();
        if !blocks.is_empty() {
            return Value::Array(blocks);
        }
    }
    let text = extract_tool_result_text(item);
    if text.trim().is_empty() {
        Value::Null
    } else {
        Value::String(text)
    }
}

fn is_empty_tool_result(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn claude_permission_event(
    v: &serde_json::Value,
    ts: Option<i64>,
) -> Option<Vec<AcpProtocolMessage>> {
    let permission = v
        .get("permission")
        .or_else(|| v.get("permissionRequest"))
        .or_else(|| v.get("permission_request"))
        .or_else(|| v.get("toolPermission"))
        .or_else(|| v.get("tool_permission"))?;
    let tool_call = permission
        .get("toolCall")
        .or_else(|| permission.get("tool_call"))
        .cloned();
    let fields = tool_call
        .as_ref()
        .and_then(|tool_call| tool_call.get("fields"))
        .unwrap_or_else(|| tool_call.as_ref().unwrap_or(permission));
    let request_id = permission
        .get("requestId")
        .or_else(|| permission.get("request_id"))
        .or_else(|| v.get("uuid"))
        .or_else(|| {
            tool_call
                .as_ref()
                .and_then(|tool_call| tool_call.get("toolCallId"))
        })
        .or_else(|| {
            tool_call
                .as_ref()
                .and_then(|tool_call| tool_call.get("tool_call_id"))
        })
        .and_then(|x| x.as_str())
        .map(String::from);
    let tool_name = string_field(fields, "title")
        .or_else(|| string_field(fields, "name"))
        .or_else(|| string_field(permission, "toolName"))
        .or_else(|| string_field(permission, "tool_name"))
        .unwrap_or_else(|| "tool".to_string());
    let input = fields
        .get("rawInput")
        .or_else(|| fields.get("raw_input"))
        .or_else(|| fields.get("input"))
        .cloned()
        .unwrap_or_else(|| permission.clone());
    let options = permission
        .get("options")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    let selected = permission
        .get("selectedOptionId")
        .or_else(|| permission.get("selected_option_id"))
        .or_else(|| permission.get("optionId"))
        .or_else(|| permission.get("option_id"))
        .and_then(|x| x.as_str())
        .map(String::from);
    let cancelled = permission
        .get("cancelled")
        .and_then(|x| x.as_bool())
        .or_else(|| {
            permission
                .get("outcome")
                .and_then(|outcome| outcome.get("outcome"))
                .and_then(|x| x.as_str())
                .map(|value| value == "cancelled")
        });
    Some(history_permission_request_message(
        HistoryPermissionRequest {
            request_id,
            tool_name,
            input,
            options,
            selected_option_id: selected,
            cancelled,
            tool_call,
            raw: permission.clone(),
            timestamp: ts,
        },
    ))
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|x| x.as_str()).map(String::from)
}

fn claude_image_item_to_content(item: &serde_json::Value) -> Option<Value> {
    let source = item.get("source")?;
    let source_type = source.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match source_type {
        "base64" => {
            let media_type = source
                .get("media_type")
                .and_then(|x| x.as_str())
                .unwrap_or("image/png");
            let data = source.get("data").and_then(|x| x.as_str())?;
            Some(serde_json::json!({
                "type": "image",
                "uri": format!("data:{media_type};base64,{data}"),
                "mimeType": media_type,
            }))
        }
        "url" => {
            let url = source.get("url").and_then(|x| x.as_str())?;
            Some(serde_json::json!({
                "type": "image",
                "uri": url,
                "mimeType": image_mime_type(url),
            }))
        }
        _ => None,
    }
}

fn image_mime_type(uri: &str) -> Option<String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        return rest
            .split_once(';')
            .map(|(mime_type, _)| mime_type)
            .filter(|mime_type| mime_type.to_ascii_lowercase().starts_with("image/"))
            .map(String::from);
    }
    let normalized = uri
        .split('?')
        .next()
        .unwrap_or(uri)
        .split('#')
        .next()
        .unwrap_or(uri)
        .to_ascii_lowercase();
    let mime = if normalized.ends_with(".png") {
        "image/png"
    } else if normalized.ends_with(".jpg") || normalized.ends_with(".jpeg") {
        "image/jpeg"
    } else if normalized.ends_with(".gif") {
        "image/gif"
    } else if normalized.ends_with(".webp") {
        "image/webp"
    } else if normalized.ends_with(".bmp") {
        "image/bmp"
    } else if normalized.ends_with(".svg") {
        "image/svg+xml"
    } else if normalized.ends_with(".avif") {
        "image/avif"
    } else if normalized.ends_with(".heic") {
        "image/heic"
    } else if normalized.ends_with(".heif") {
        "image/heif"
    } else {
        return None;
    };
    Some(mime.to_string())
}

fn claude_file_edit_message(
    edit: &ClaudeEditTool,
    ts: Option<i64>,
    tool_result_meta: Option<&serde_json::Value>,
) -> Option<AcpProtocolMessage> {
    let edits = if let Some(meta_edit) = claude_meta_edit(tool_result_meta) {
        vec![meta_edit]
    } else {
        match edit.name.as_str() {
            "Write" => claude_write_edits(&edit.input),
            "Edit" => claude_edit_edits(&edit.input),
            "MultiEdit" => claude_multi_edit_edits(&edit.input),
            _ => Vec::new(),
        }
    };
    file_edit_message("claude", edits, ts)
}

fn claude_meta_edit(tool_result_meta: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let meta = tool_result_meta?;
    let path = meta
        .get("filePath")
        .or_else(|| meta.get("file_path"))
        .and_then(|x| x.as_str())?;
    let hunks = claude_structured_patch_hunks(meta);
    if hunks.is_empty() {
        return None;
    }
    let additions: usize = hunks
        .iter()
        .filter_map(|h| h.get("additions").and_then(|x| x.as_u64()))
        .map(|x| x as usize)
        .sum();
    let deletions: usize = hunks
        .iter()
        .filter_map(|h| h.get("deletions").and_then(|x| x.as_u64()))
        .map(|x| x as usize)
        .sum();
    let detail = hunks
        .iter()
        .filter_map(|h| h.get("detail").and_then(|x| x.as_str()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let patch = claude_meta_patch(path, &hunks);
    let old_content = meta
        .get("oldString")
        .and_then(|x| x.as_str())
        .map(String::from);
    let new_content = meta
        .get("newString")
        .and_then(|x| x.as_str())
        .map(String::from);
    Some(serde_json::json!({
        "path": path,
        "displayPath": path,
        "kind": "edit",
        "additions": additions,
        "deletions": deletions,
        "detail": detail,
        "patch": patch,
        "oldContent": old_content,
        "newContent": new_content,
        "hunks": hunks,
    }))
}

fn claude_structured_patch_hunks(meta: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(patches) = meta.get("structuredPatch").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    patches
        .iter()
        .filter_map(|patch| {
            let lines = patch.get("lines").and_then(|x| x.as_array())?;
            let detail_lines: Vec<String> = lines
                .iter()
                .filter_map(|line| line.as_str().map(String::from))
                .collect();
            let additions = detail_lines
                .iter()
                .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
                .count();
            let deletions = detail_lines
                .iter()
                .filter(|line| line.starts_with('-') && !line.starts_with("---"))
                .count();
            let old_start = patch.get("oldStart").and_then(|x| x.as_u64());
            let old_lines = patch.get("oldLines").and_then(|x| x.as_u64());
            let new_start = patch.get("newStart").and_then(|x| x.as_u64());
            let new_lines = patch.get("newLines").and_then(|x| x.as_u64());
            let header = format!(
                "@@ -{},{} +{},{} @@",
                old_start.unwrap_or(0),
                old_lines.unwrap_or(0),
                new_start.unwrap_or(0),
                new_lines.unwrap_or(0)
            );
            let detail = if detail_lines.is_empty() {
                header
            } else {
                format!("{header}\n{}", detail_lines.join("\n"))
            };
            Some(serde_json::json!({
                "oldStart": old_start,
                "oldLines": old_lines,
                "newStart": new_start,
                "newLines": new_lines,
                "additions": additions,
                "deletions": deletions,
                "detail": detail,
            }))
        })
        .collect()
}

fn claude_meta_patch(path: &str, hunks: &[serde_json::Value]) -> String {
    let normalized = path.trim_start_matches('/');
    let body = hunks
        .iter()
        .filter_map(|h| h.get("detail").and_then(|x| x.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "diff --git a/{normalized} b/{normalized}\n--- a/{normalized}\n+++ b/{normalized}\n{body}\n"
    )
}

fn claude_write_edits(input: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(path) = input.get("file_path").and_then(|x| x.as_str()) else {
        return Vec::new();
    };
    let content = input.get("content").and_then(|x| x.as_str()).unwrap_or("");
    vec![serde_json::json!({
        "path": path,
        "displayPath": path,
        "kind": "write",
        "additions": line_count(content),
        "deletions": 0,
        "detail": content,
        "newContent": content,
    })]
}

fn claude_edit_edits(input: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(path) = input.get("file_path").and_then(|x| x.as_str()) else {
        return Vec::new();
    };
    let old_string = input
        .get("old_string")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let new_string = input
        .get("new_string")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    vec![serde_json::json!({
        "path": path,
        "displayPath": path,
        "kind": "edit",
        "additions": line_count(new_string),
        "deletions": line_count(old_string),
        "detail": format!("--- old\n{old_string}\n+++ new\n{new_string}"),
        "oldContent": old_string,
        "newContent": new_string,
    })]
}

fn claude_multi_edit_edits(input: &serde_json::Value) -> Vec<serde_json::Value> {
    let Some(path) = input.get("file_path").and_then(|x| x.as_str()) else {
        return Vec::new();
    };
    let Some(edits) = input.get("edits").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let additions: usize = edits
        .iter()
        .map(|edit| {
            edit.get("new_string")
                .and_then(|x| x.as_str())
                .map(line_count)
                .unwrap_or(0)
        })
        .sum();
    let deletions: usize = edits
        .iter()
        .map(|edit| {
            edit.get("old_string")
                .and_then(|x| x.as_str())
                .map(line_count)
                .unwrap_or(0)
        })
        .sum();
    let detail = edits
        .iter()
        .enumerate()
        .map(|(idx, edit)| {
            let old_string = edit
                .get("old_string")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let new_string = edit
                .get("new_string")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            format!(
                "Edit {}\n--- old\n{}\n+++ new\n{}",
                idx + 1,
                old_string,
                new_string
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    vec![serde_json::json!({
        "path": path,
        "displayPath": path,
        "kind": "multi_edit",
        "additions": additions,
        "deletions": deletions,
        "detail": detail,
    })]
}

fn file_edit_message(
    source: &str,
    edits: Vec<serde_json::Value>,
    ts: Option<i64>,
) -> Option<AcpProtocolMessage> {
    if edits.is_empty() {
        return None;
    }
    let additions: i64 = edits
        .iter()
        .filter_map(|e| e.get("additions").and_then(|x| x.as_i64()))
        .sum();
    let deletions: i64 = edits
        .iter()
        .filter_map(|e| e.get("deletions").and_then(|x| x.as_i64()))
        .sum();
    let data = serde_json::json!({
        "source": source,
        "files": edits.len(),
        "additions": additions,
        "deletions": deletions,
        "edits": edits,
    });
    Some(history_session_update_message("file_edit", data, ts))
}

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count().max(1)
    }
}

fn parse_session(path: &PathBuf) -> Result<Option<SessionInfo>> {
    let mut cwd: Option<String> = None;
    let reverse = latest_reverse_metadata_from_file(path).unwrap_or_default();
    let mut forked_from_agent: Option<Agent> = None;
    let mut forked_from_id: Option<String> = None;
    let mut first_user_message: Option<String> = None;
    let mut earliest_ts: Option<i64> = None;

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if cwd.is_none() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                if !c.is_empty() {
                    cwd = Some(c.to_string());
                }
            }
        }
        if let Some(t) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso)
        {
            earliest_ts = Some(earliest_ts.map_or(t, |e| e.min(t)));
        }
        let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if kind == "user" && first_user_message.is_none() {
            if let Some(msg) = v.get("message") {
                if forked_from_id.is_none() {
                    if let Some(lineage) = cross_context_lineage_from_payload(msg) {
                        forked_from_agent = Some(lineage.agent);
                        forked_from_id = Some(lineage.session_id);
                    }
                }
                if let Some(cleaned) = clean_history_user_preview_text(&extract_message_text(msg)) {
                    first_user_message = Some(normalize_preview(&cleaned));
                }
            }
        }
        if cwd.is_some()
            && earliest_ts.is_some()
            && (reverse.title.is_some() || first_user_message.is_some())
        {
            break;
        }
    }

    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let updated_at = reverse.updated_at.or_else(|| {
        fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(system_time_to_millis)
    });

    let project_name = cwd.as_ref().and_then(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
    });
    let title = reverse.title.or_else(|| first_user_message.clone());

    Ok(Some(SessionInfo {
        id,
        agent: Agent::Claude,
        forked_from_agent,
        forked_from_id,
        project_path: cwd,
        project_name,
        started_at: earliest_ts,
        updated_at,
        message_count: 0,
        rename_title: None,
        title,
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

fn extract_message_text(message: &serde_json::Value) -> String {
    let content = match message.get("content") {
        Some(c) => c,
        None => return String::new(),
    };
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
            match kind {
                "text" => {
                    if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                        parts.push(t.to_string());
                    }
                }
                "thinking" | "reasoning" => {
                    if let Some(t) = item
                        .get("thinking")
                        .or_else(|| item.get("reasoning"))
                        .or_else(|| item.get("text"))
                        .and_then(|x| x.as_str())
                    {
                        parts.push(t.to_string());
                    }
                }
                "tool_use" => {
                    let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                    parts.push(format!("[tool_use: {name}]"));
                }
                "tool_result" => {
                    if let Some(t) = item.get("content").and_then(|x| x.as_str()) {
                        parts.push(format!("[tool_result] {t}"));
                    }
                }
                _ => {}
            }
        }
        return parts.join("\n");
    }
    String::new()
}

fn ai_title_from_value(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|x| x.as_str()) != Some("ai-title") {
        return None;
    }
    v.get("aiTitle")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_preview)
}

fn last_prompt_from_value(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|x| x.as_str()) != Some("last-prompt") {
        return None;
    }
    v.get("lastPrompt")
        .and_then(|x| x.as_str())
        .and_then(clean_history_preview_candidate_text)
        .map(|cleaned| normalize_preview(&cleaned))
}

#[derive(Default)]
struct ReverseMetadata {
    title: Option<String>,
    updated_at: Option<i64>,
}

fn latest_reverse_metadata_from_file(path: &Path) -> Result<ReverseMetadata> {
    let mut file = File::open(path)?;
    let mut offset = file.metadata()?.len();
    let mut latest_ai_title = None;
    let mut latest_last_prompt = None;
    let mut updated_at = None;

    let mut carry = Vec::new();
    while offset > 0 {
        let read_size = REVERSE_METADATA_CHUNK_SIZE.min(offset);
        offset -= read_size;
        file.seek(SeekFrom::Start(offset))?;

        let mut chunk = vec![0; read_size as usize];
        file.read_exact(&mut chunk)?;
        chunk.extend_from_slice(&carry);

        let complete = if offset > 0 {
            match chunk.iter().position(|b| *b == b'\n') {
                Some(pos) => {
                    carry = chunk[..pos].to_vec();
                    &chunk[pos + 1..]
                }
                None => {
                    carry = chunk;
                    continue;
                }
            }
        } else {
            carry.clear();
            &chunk[..]
        };

        for line in complete.split(|b| *b == b'\n').rev() {
            if line.is_empty() {
                continue;
            }
            let Ok(text) = std::str::from_utf8(line) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            if updated_at.is_none() {
                updated_at = v
                    .get("timestamp")
                    .and_then(|x| x.as_str())
                    .and_then(parse_iso);
            }
            if latest_ai_title.is_none() {
                latest_ai_title = ai_title_from_value(&v);
            }
            if latest_last_prompt.is_none() {
                latest_last_prompt = last_prompt_from_value(&v);
            }
            if latest_ai_title.is_some() && updated_at.is_some() {
                return Ok(ReverseMetadata {
                    title: latest_ai_title,
                    updated_at,
                });
            }
        }
    }
    Ok(ReverseMetadata {
        title: latest_ai_title.or(latest_last_prompt),
        updated_at,
    })
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexFile {
    #[serde(default)]
    entries: Vec<IndexEntry>,
    #[serde(default)]
    original_path: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexEntry {
    session_id: String,
    #[serde(default)]
    full_path: Option<String>,
    #[serde(default)]
    file_mtime: Option<i64>,
    #[serde(default)]
    first_prompt: Option<String>,
    #[serde(default)]
    message_count: Option<usize>,
    #[serde(default)]
    created: Option<String>,
    #[serde(default)]
    modified: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
}

fn read_index(path: &Path) -> Result<IndexFile> {
    let text = fs::read_to_string(path)?;
    let idx: IndexFile = serde_json::from_str(&text)?;
    Ok(idx)
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubagentMeta {
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn read_subagents(dir: &Path) -> Vec<SubagentInfo> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        match parse_subagent(&p) {
            Ok(Some(info)) => out.push(info),
            Ok(None) => {}
            Err(e) => log::warn!("subagent parse {} failed: {e}", p.display()),
        }
    }
    out.sort_by_key(|a| a.started_at);
    out
}

fn parse_subagent(path: &Path) -> Result<Option<SubagentInfo>> {
    let mut earliest_ts: Option<i64> = None;
    let updated_from_tail = latest_reverse_metadata_from_file(path)
        .ok()
        .and_then(|m| m.updated_at);

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(t) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso)
        {
            earliest_ts = Some(earliest_ts.map_or(t, |e| e.min(t)));
            break;
        }
    }

    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let meta_path = path.with_extension("meta.json");
    let meta: SubagentMeta = fs::read_to_string(&meta_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let updated_at = updated_from_tail.or_else(|| {
        fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(system_time_to_millis)
    });

    Ok(Some(SubagentInfo {
        id,
        agent_type: meta.agent_type,
        description: meta.description,
        started_at: earliest_ts,
        updated_at,
        message_count: 0,
        first_user_message: None,
        file_path: path.to_string_lossy().into_owned(),
        file_size,
        partial: false,
        available: true,
    }))
}

// Parse a single subagent jsonl in isolation, recovering its parent session
// id from the path layout `<project>/<parent_session_id>/subagents/<id>.jsonl`.
// Used by per-file reindex paths so a subagent change doesn't touch the
// parent session's main row.
pub fn parse_single_subagent_file(path: &Path) -> Result<Option<(String, SubagentInfo)>> {
    let parent_session_id = match path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
    {
        Some(s) => s.to_string(),
        None => return Ok(None),
    };
    match parse_subagent(path)? {
        Some(info) => Ok(Some((parent_session_id, info))),
        None => Ok(None),
    }
}

fn info_from_index(entry: &IndexEntry, idx: &IndexFile, project_dir: &Path) -> Option<SessionInfo> {
    let file_path = entry.full_path.clone().unwrap_or_default();
    let cwd = entry
        .project_path
        .clone()
        .or_else(|| idx.original_path.clone());
    let project_name = cwd.as_ref().and_then(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
    });
    let started_at = entry.created.as_deref().and_then(parse_iso);
    let modified_ts = entry.modified.as_deref().and_then(parse_iso);
    let updated_at = entry.file_mtime.or(modified_ts).or(started_at);
    let preview = entry
        .first_prompt
        .as_deref()
        .and_then(clean_history_preview_candidate_text)
        .map(|cleaned| normalize_preview(&cleaned));
    let (file_size, available) = if file_path.is_empty() {
        (0, false)
    } else {
        match fs::metadata(&file_path) {
            Ok(m) => (m.len(), true),
            Err(_) => (0, false),
        }
    };
    let subagents_dir = project_dir.join(&entry.session_id).join("subagents");
    let subagents = if subagents_dir.is_dir() {
        read_subagents(&subagents_dir)
    } else {
        Vec::new()
    };
    Some(SessionInfo {
        id: entry.session_id.clone(),
        agent: Agent::Claude,
        forked_from_agent: None,
        forked_from_id: None,
        project_path: cwd,
        project_name,
        started_at,
        updated_at,
        message_count: entry.message_count.unwrap_or(0),
        rename_title: None,
        title: preview.clone(),
        first_user_message: preview,
        file_path,
        file_size,
        partial: true,
        available,
        archived: !available,
        origin: crate::models::SessionOrigin::Chat,
        scheduled_task_id: None,
        is_auxiliary: false,
        subagents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn prompt_text(row: &HistoryAcpMessage) -> String {
        row.message.data["prompt"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(block_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn message_role(message: &AcpProtocolMessage) -> &'static str {
        if message.method == "session/prompt" {
            return "user";
        }
        match message.update_type.as_deref() {
            Some("agent_thought_chunk") => "thinking",
            Some("tool_call") => "tool_call",
            Some("tool_call_update") => "tool_result",
            Some("file_edit") => "file_edit",
            _ => "unknown",
        }
    }

    fn message_text(message: &AcpProtocolMessage) -> String {
        update_from_message(message)["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(block_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn block_text(block: &serde_json::Value) -> Option<&str> {
        block
            .get("text")
            .or_else(|| block.get("uri"))
            .and_then(|value| value.as_str())
    }

    fn update_from_message(message: &AcpProtocolMessage) -> &serde_json::Value {
        message.data.get("update").unwrap_or(&message.data)
    }

    fn tool_call_id(message: &AcpProtocolMessage) -> Option<&str> {
        update_from_message(message)
            .get("toolCallId")
            .and_then(|value| value.as_str())
    }

    fn tool_title(message: &AcpProtocolMessage) -> Option<&str> {
        update_from_message(message)
            .get("title")
            .and_then(|value| value.as_str())
    }

    fn raw_input(message: &AcpProtocolMessage) -> &serde_json::Value {
        update_from_message(message)
            .get("rawInput")
            .unwrap_or(&serde_json::Value::Null)
    }

    fn raw_output(message: &AcpProtocolMessage) -> &serde_json::Value {
        update_from_message(message)
            .get("rawOutput")
            .unwrap_or(&serde_json::Value::Null)
    }

    #[test]
    fn expand_message_converts_todo_write_to_todo_role() {
        let msg = serde_json::json!({
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_todos",
                    "name": "TodoWrite",
                    "input": {
                        "todos": [
                            {
                                "content": "Verify parser todo rendering",
                                "activeForm": "Verifying parser",
                                "status": "completed"
                            },
                            {
                                "content": "Style todos in session detail",
                                "activeForm": "Styling todos",
                                "status": "in_progress"
                            }
                        ]
                    }
                }
            ]
        });

        let mut edits = HashMap::new();
        let out = expand_message("assistant", &msg, Some(1_700_000_000_000), &mut edits, None);

        assert_eq!(out.len(), 1);
        assert_eq!(message_role(&out[0]), "tool_call");
        assert_eq!(tool_call_id(&out[0]), Some("toolu_todos"));
        assert_eq!(tool_title(&out[0]), Some("TodoWrite"));
        assert_eq!(
            raw_input(&out[0])
                .get("entries")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(
            raw_input(&out[0])["entries"][0]["content"],
            "Verify parser todo rendering"
        );
        assert_eq!(raw_input(&out[0])["entries"][0]["status"], "completed");
    }

    #[test]
    fn expand_message_parses_reasoning_and_tool_id_aliases() {
        let msg = serde_json::json!({
            "role": "assistant",
            "content": [
                { "type": "reasoning", "reasoning": "checking aliases" },
                {
                    "type": "tool_use",
                    "toolUseId": "tool_alias",
                    "name": "Read",
                    "input": { "file_path": "README.md" }
                }
            ]
        });
        let result = serde_json::json!({
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "toolUseId": "tool_alias",
                    "content": [
                        { "type": "text", "text": "ok" },
                        "tail"
                    ]
                }
            ]
        });
        let mut edits = HashMap::new();
        let out = expand_message("assistant", &msg, Some(1), &mut edits, None);
        assert_eq!(out.len(), 2);
        assert_eq!(message_role(&out[0]), "thinking");
        assert_eq!(message_text(&out[0]), "checking aliases");
        assert_eq!(message_role(&out[1]), "tool_call");
        assert_eq!(tool_call_id(&out[1]), Some("tool_alias"));
        assert_eq!(tool_title(&out[1]), Some("Read"));
        assert_eq!(raw_input(&out[1])["file_path"], "README.md");

        let result_out = expand_message("user", &result, Some(2), &mut edits, None);
        assert_eq!(result_out.len(), 1);
        assert_eq!(message_role(&result_out[0]), "tool_result");
        assert_eq!(tool_call_id(&result_out[0]), Some("tool_alias"));
        assert_eq!(raw_output(&result_out[0])[0]["text"], "ok");
        assert_eq!(raw_output(&result_out[0])[1]["text"], "tail");
    }

    #[test]
    fn parse_session_title_prefers_ai_title_then_last_prompt_then_first_user_message() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-parser-title-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        let last_prompt_path = dir.join("last-prompt-session.jsonl");
        fs::write(
            &last_prompt_path,
            r#"{"type":"user","timestamp":"2026-05-18T05:09:15.000Z","cwd":"/tmp/project","message":{"role":"user","content":"first user"}}
{"type":"last-prompt","lastPrompt":"last prompt title","leafUuid":"leaf","sessionId":"last-prompt-session"}
"#,
        )
        .unwrap();
        let last_prompt_info = parse_session(&last_prompt_path).unwrap().unwrap();
        assert_eq!(last_prompt_info.title.as_deref(), Some("last prompt title"));
        assert_eq!(
            last_prompt_info.first_user_message.as_deref(),
            Some("first user")
        );

        let ai_title_path = dir.join("ai-title-session.jsonl");
        fs::write(
            &ai_title_path,
            r#"{"type":"user","timestamp":"2026-05-18T05:09:15.000Z","cwd":"/tmp/project","message":{"role":"user","content":"first user"}}
{"type":"last-prompt","lastPrompt":"last prompt title","leafUuid":"leaf","sessionId":"ai-title-session"}
{"type":"ai-title","aiTitle":"ai title wins"}
"#,
        )
        .unwrap();
        let ai_title_info = parse_session(&ai_title_path).unwrap().unwrap();
        assert_eq!(ai_title_info.title.as_deref(), Some("ai title wins"));

        let latest_ai_title_path = dir.join("latest-ai-title-session.jsonl");
        fs::write(
            &latest_ai_title_path,
            r#"{"type":"user","timestamp":"2026-05-18T05:09:15.000Z","cwd":"/tmp/project","message":{"role":"user","content":"first user"}}
{"type":"ai-title","aiTitle":"old ai title"}
{"type":"last-prompt","lastPrompt":"newer last prompt","leafUuid":"leaf","sessionId":"latest-ai-title-session"}
{"type":"ai-title","aiTitle":"latest ai title"}
"#,
        )
        .unwrap();
        let latest_ai_title_info = parse_session(&latest_ai_title_path).unwrap().unwrap();
        assert_eq!(
            latest_ai_title_info.title.as_deref(),
            Some("latest ai title")
        );

        let latest_last_prompt_path = dir.join("latest-last-prompt-session.jsonl");
        fs::write(
            &latest_last_prompt_path,
            r#"{"type":"user","timestamp":"2026-05-18T05:09:15.000Z","cwd":"/tmp/project","message":{"role":"user","content":"first user"}}
{"type":"last-prompt","lastPrompt":"old last prompt","leafUuid":"leaf-1","sessionId":"latest-last-prompt-session"}
{"type":"last-prompt","lastPrompt":"latest last prompt","leafUuid":"leaf-2","sessionId":"latest-last-prompt-session"}
"#,
        )
        .unwrap();
        let latest_last_prompt_info = parse_session(&latest_last_prompt_path).unwrap().unwrap();
        assert_eq!(
            latest_last_prompt_info.title.as_deref(),
            Some("latest last prompt")
        );

        fs::remove_file(last_prompt_path).ok();
        fs::remove_file(ai_title_path).ok();
        fs::remove_file(latest_ai_title_path).ok();
        fs::remove_file(latest_last_prompt_path).ok();
        fs::remove_dir(dir).ok();
    }

    #[test]
    fn expand_message_adds_file_edit_summary_after_successful_edit_result() {
        let msg = serde_json::json!({
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_edit",
                    "name": "Edit",
                    "input": {
                        "file_path": "/tmp/project/src/app.rs",
                        "old_string": "old\nline",
                        "new_string": "new\nline\nmore",
                        "replace_all": false
                    }
                }
            ]
        });
        let result = serde_json::json!({
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "toolu_edit",
                    "content": "The file /tmp/project/src/app.rs has been updated successfully."
                }
            ]
        });
        let mut edits = HashMap::new();
        let tool_events = expand_message("assistant", &msg, Some(1), &mut edits, None);
        assert_eq!(tool_events.len(), 1);
        assert_eq!(message_role(&tool_events[0]), "tool_call");

        let meta = serde_json::json!({
            "filePath": "/tmp/project/src/app.rs",
            "oldString": "old\nline",
            "newString": "new\nline\nmore",
            "structuredPatch": [
                {
                    "oldStart": 10,
                    "oldLines": 2,
                    "newStart": 10,
                    "newLines": 3,
                    "lines": [
                        "-old",
                        "-line",
                        "+new",
                        "+line",
                        "+more"
                    ]
                }
            ]
        });
        let result_events = expand_message("user", &result, Some(2), &mut edits, Some(&meta));
        assert_eq!(result_events.len(), 2);
        assert_eq!(message_role(&result_events[0]), "tool_result");
        assert_eq!(message_role(&result_events[1]), "file_edit");
        let value = update_from_message(&result_events[1]);
        assert_eq!(value.get("additions").and_then(|x| x.as_u64()), Some(3));
        assert_eq!(value.get("deletions").and_then(|x| x.as_u64()), Some(2));
        let edit = value
            .get("edits")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .unwrap();
        assert_eq!(
            edit.get("hunks")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|h| h.get("oldStart"))
                .and_then(|x| x.as_u64()),
            Some(10)
        );
        assert!(edit
            .get("detail")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .starts_with("@@ -10,2 +10,3 @@"));
        assert_eq!(
            edit.get("oldContent").and_then(|x| x.as_str()),
            Some("old\nline")
        );
        assert_eq!(
            edit.get("newContent").and_then(|x| x.as_str()),
            Some("new\nline\nmore")
        );
    }

    #[test]
    fn expand_message_leaves_cross_context_cleanup_to_turn_builder() {
        let msg = serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "不错，写入md文档" },
                {
                    "type": "text",
                    "text": "[@sessio-cross-context-abc.md](file:///tmp/.cross-context/sessio-cross-context-abc.md)"
                },
                {
                    "type": "text",
                    "text": "\n<context ref=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\">\n<sessio-upload-file uri=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\" name=\"sessio-cross-context-abc.md\" mimeType=\"text/markdown\">\n# Continued session from agent\n[user]\nhi\n</sessio-upload-file>\n</context>"
                }
            ]
        });
        let mut edits = HashMap::new();
        let out = expand_message("user", &msg, Some(1), &mut edits, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].method, "session/prompt");
        let text = out[0].data["prompt"][0]["text"].as_str().unwrap();
        assert!(text.contains("不错，写入md文档"));
        assert!(text.contains("sessio-cross-context-abc.md"));
        assert!(text.contains("<sessio-upload-file"));

        let rows = vec![HistoryAcpMessage {
            message: out[0].clone(),
            timestamp: Some(1),
            location: SourceLocation::file("/tmp/session.jsonl"),
            synthetic: true,
        }];
        let turns = crate::turns::session_history_turns_from_acp_messages(&rows);
        let display_text = turns[0].blocks[0].blocks[0].text.as_deref().unwrap();
        assert!(display_text.contains("不错，写入md文档"));
        assert_eq!(turns[0].blocks[0].blocks.len(), 2);
        assert!(!display_text.contains("sessio-cross-context-abc.md"));
        assert!(!display_text.contains("<sessio-upload-file"));
        assert!(!display_text.contains("Continued session from agent"));
        let attachment = &turns[0].blocks[0].blocks[1];
        assert_eq!(attachment.kind, "resource");
        assert_eq!(
            attachment.name.as_deref(),
            Some("sessio-cross-context-abc.md")
        );
        assert_eq!(
            attachment.uri.as_deref(),
            Some("file:///tmp/.cross-context/sessio-cross-context-abc.md")
        );
    }

    #[test]
    fn read_subagent_messages_sanitize_cross_context_attachment_for_user() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-subagent-cross-context-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let subagents_dir = dir.join("project").join("parent-session").join("subagents");
        fs::create_dir_all(&subagents_dir).unwrap();

        let path = subagents_dir.join("subagent-session.jsonl");
        let line = serde_json::json!({
            "type": "user",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "message": {
                "role": "user",
                "content": [
                    { "type": "text", "text": "继续整理" },
                    {
                        "type": "text",
                        "text": "[@sessio-cross-context-abc.md](file:///tmp/.cross-context/sessio-cross-context-abc.md)"
                    },
                    {
                        "type": "text",
                        "text": "\n<context ref=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\">\n<sessio-upload-file uri=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\" name=\"sessio-cross-context-abc.md\" mimeType=\"text/markdown\">\n# Continued session from agent\n[user]\nhi\n</sessio-upload-file>\n</context>"
                    }
                ]
            }
        })
        .to_string();
        fs::write(&path, format!("{line}\n")).unwrap();

        let messages = read_history_acp_messages_with_locations(&path).unwrap();
        let user = messages
            .iter()
            .find(|row| row.message.method == "session/prompt")
            .expect("user message");
        let text = prompt_text(user);
        assert!(text.contains("继续整理"), "{}", text);
        assert!(text.contains("sessio-cross-context-abc.md"), "{}", text);

        let turns = crate::turns::session_history_turns_from_acp_messages(&messages);
        let display_text = turns[0].blocks[0].blocks[0].text.as_deref().unwrap();
        assert!(display_text.contains("继续整理"), "{}", display_text);
        assert_eq!(turns[0].blocks[0].blocks.len(), 2);
        assert!(
            !display_text.contains("sessio-cross-context-abc.md"),
            "{}",
            display_text
        );
        assert!(
            !display_text.contains("<sessio-upload-file"),
            "{}",
            display_text
        );
        let attachment = &turns[0].blocks[0].blocks[1];
        assert_eq!(attachment.kind, "resource");
        assert_eq!(
            attachment.name.as_deref(),
            Some("sessio-cross-context-abc.md")
        );
        assert_eq!(
            attachment.uri.as_deref(),
            Some("file:///tmp/.cross-context/sessio-cross-context-abc.md")
        );
        assert!(
            !display_text.contains("Continued session from agent"),
            "{}",
            display_text
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_history_acp_messages_with_locations_parses_structured_permission() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-permission-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("permission-session.jsonl");
        let line = serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-05-29T05:38:55.000Z",
            "uuid": "perm-line",
            "permission": {
                "requestId": "perm-1",
                "toolCall": {
                    "toolCallId": "tool-1",
                    "fields": {
                        "title": "Bash",
                        "rawInput": { "command": "pnpm test" }
                    }
                },
                "options": [
                    { "optionId": "allow", "name": "Allow", "kind": "allow" },
                    { "optionId": "reject", "name": "Reject", "kind": "reject" }
                ],
                "selectedOptionId": "allow"
            }
        });
        fs::write(&path, format!("{line}\n")).unwrap();

        let events = read_history_acp_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 2);
        let event = &events[0].message;
        assert_eq!(event.method, "session/request_permission");
        assert_eq!(event.request_id.as_deref(), Some("perm-1"));
        assert_eq!(event.data["toolCall"]["fields"]["title"], "Bash");
        assert_eq!(
            event.data["toolCall"]["fields"]["rawInput"]["command"],
            "pnpm test"
        );
        assert_eq!(
            events[1].message.data["outcome"]["optionId"].as_str(),
            Some("allow")
        );
        assert_eq!(event.data["options"].as_array().unwrap().len(), 2);
        assert_eq!(event.data["toolCall"]["toolCallId"], "tool-1");

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn parse_session_first_user_message_strips_cross_context_attachment_text() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-cross-context-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("cross-context-preview-session.jsonl");
        let line = serde_json::json!({
            "type": "user",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "cwd": "/tmp/project",
            "message": {
                "role": "user",
                "content": [
                    { "type": "text", "text": "不错，写入md文档" },
                    {
                        "type": "text",
                        "text": "[@sessio-cross-context-abc.md](file:///tmp/.cross-context/sessio-cross-context-abc.md)"
                    },
                    {
                        "type": "text",
                        "text": "\n<context ref=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\">\n<sessio-upload-file uri=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\" name=\"sessio-cross-context-abc.md\" mimeType=\"text/markdown\">\n# Continued session from agent\n[user]\nhi\n</sessio-upload-file>\n</context>"
                    }
                ]
            }
        })
        .to_string();
        fs::write(&path, format!("{line}\n")).unwrap();

        let info = parse_session(&path).unwrap().unwrap();
        let preview = info.first_user_message.as_deref().unwrap_or("");
        assert!(
            preview.contains("不错，写入md文档"),
            "preview should keep user text: {preview}"
        );
        assert!(
            !preview.contains("sessio-upload-file"),
            "preview should drop attachment tag: {preview}"
        );
        assert!(
            !preview.contains("Continued session from agent"),
            "preview should drop replayed body: {preview}"
        );
        assert!(
            !preview.contains("sessio-cross-context-abc.md"),
            "preview should drop attachment link: {preview}"
        );
    }

    #[test]
    fn parse_session_reads_cross_context_lineage_from_user_attachment() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-cross-context-lineage-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let context_path = dir.join("sessio-cross-context-parent.md");
        fs::write(
            &context_path,
            r#"<!-- sessio-cross:start source_agent="codex" source_session_id="codex-parent" source_file_path="/tmp/parent.jsonl" -->

# Continued session from agent

<!-- sessio-cross:end -->"#,
        )
        .unwrap();
        let path = dir.join("cross-context-lineage-session.jsonl");
        let line = serde_json::json!({
            "type": "user",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "cwd": "/tmp/project",
            "message": {
                "role": "user",
                "content": [
                    { "type": "text", "text": "继续" },
                    { "type": "text", "text": format!("[@ctx](file://{})", context_path.display()) }
                ]
            }
        })
        .to_string();
        fs::write(&path, format!("{line}\n")).unwrap();

        let info = parse_session(&path).unwrap().unwrap();
        assert_eq!(info.forked_from_agent, Some(Agent::Codex));
        assert_eq!(info.forked_from_id.as_deref(), Some("codex-parent"));

        fs::remove_file(&path).ok();
        fs::remove_file(&context_path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_history_acp_messages_strips_leading_ide_context_from_user_prompt() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-ide-context-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("ide-context-session.jsonl");
        let line = serde_json::json!({
            "type": "user",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "message": {
                "role": "user",
                "content": "<ide_opened_file>secret.rs</ide_opened_file>\nreal request"
            }
        })
        .to_string();
        fs::write(&path, format!("{line}\n")).unwrap();

        let messages = read_history_acp_messages_with_locations(&path).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message.method, "session/prompt");
        assert_eq!(
            messages[0].message.data["prompt"][0]["text"].as_str(),
            Some("<ide_opened_file>secret.rs</ide_opened_file>\nreal request")
        );
        let turns = crate::turns::session_history_turns_from_acp_messages(&messages);
        assert_eq!(
            turns[0].blocks[0].blocks[0].text.as_deref(),
            Some("real request")
        );

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_history_acp_messages_drops_system_noise_after_ide_context() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-claude-ide-noise-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("ide-noise-session.jsonl");
        let line = serde_json::json!({
            "type": "user",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "message": {
                "role": "user",
                "content": "<ide_opened_file>secret.rs</ide_opened_file>\n<environment_context>\nnoise\n</environment_context>"
            }
        })
        .to_string();
        fs::write(&path, format!("{line}\n")).unwrap();

        let messages = read_history_acp_messages_with_locations(&path).unwrap();
        assert_eq!(messages.len(), 1);
        let turns = crate::turns::session_history_turns_from_acp_messages(&messages);
        assert!(turns[0].blocks.is_empty(), "{turns:#?}");

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }
}
