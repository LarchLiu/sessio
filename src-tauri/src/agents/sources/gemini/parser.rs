use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::runtime::types::AcpProtocolMessage;
use crate::agents::sources::shared::attachment_text::clean_history_user_preview_text;
use crate::agents::sources::shared::cross_context::cross_context_lineage_from_payload;
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::{HistoryAcpMessage, SourceLocation};
use crate::models::{normalize_preview, Agent, SessionInfo};
use crate::turns::{
    history_assistant_message, history_permission_request_message, history_prompt_message,
    history_session_update_message, history_thought_message, history_tool_call_message,
    history_tool_result_message, history_user_message, HistoryPermissionRequest,
};
use serde_json::json;
use serde_json::Value;

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let (tmp_dir, projects_json) = paths()?;
    let base_dir = tmp_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| tmp_dir.clone());
    let mappings = load_project_mappings(&projects_json).unwrap_or_default();

    let out: Vec<SessionInfo> = collect_chat_files(&base_dir)
        .into_iter()
        .filter_map(
            |chat_path| match parse_chat_file(&chat_path, Some(&base_dir), &mappings) {
                Ok(session) => session,
                Err(e) => {
                    log::warn!("gemini parse {} failed: {e}", chat_path.display());
                    None
                }
            },
        )
        .collect();
    Ok(out)
}

pub fn paths() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok((
        home.join(".gemini").join("tmp"),
        home.join(".gemini").join("projects.json"),
    ))
}

pub fn base_dir() -> Result<std::path::PathBuf> {
    let (tmp_dir, _) = paths()?;
    Ok(tmp_dir.parent().map(Path::to_path_buf).unwrap_or(tmp_dir))
}

pub fn parse_file(path: &Path) -> Result<Vec<SessionInfo>> {
    if !is_chat_file(path) {
        return Ok(Vec::new());
    }
    Ok(parse_chat_file(
        path,
        base_dir().ok().as_deref(),
        &load_default_project_mappings()?,
    )?
    .into_iter()
    .collect())
}

pub fn remove_session_from_logs(
    path: &Path,
    session_id: &str,
    home: &Path,
    removed_root: &Path,
) -> Result<bool> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(false);
    }
    if is_chat_file(path) {
        let sid = parse_chat_file(path, None, &load_default_project_mappings()?)?
            .map(|info| info.id)
            .unwrap_or_default();
        if sid != session_id {
            return Ok(false);
        }

        let relative = path
            .strip_prefix(home)
            .map_err(|_| anyhow::anyhow!("session file is outside home: {}", path.display()))?;
        let removed_path = removed_root.join(relative);
        if let Some(parent) = removed_path.parent() {
            fs::create_dir_all(parent)?;
        }
        move_file(path, &available_removed_path(removed_path))?;
        return Ok(true);
    }
    Ok(false)
}

pub fn read_history_acp_messages_with_locations(
    path: &Path,
    session_id: &str,
) -> Result<Vec<HistoryAcpMessage>> {
    read_chat_history_acp_messages_with_locations(path, session_id)
}

#[derive(Default)]
struct ProjectMappings {
    hash_to_path: HashMap<String, String>,
    name_to_path: HashMap<String, String>,
}

fn load_project_mappings(projects_json: &Path) -> Result<ProjectMappings> {
    let text = fs::read_to_string(projects_json)?;
    let value: serde_json::Value = serde_json::from_str(&text)?;
    let projects = value
        .get("projects")
        .and_then(|x| x.as_object())
        .cloned()
        .unwrap_or_default();
    let mut hash_to_path = HashMap::new();
    let mut name_to_path = HashMap::new();
    for (path, name) in projects.iter() {
        let mut hasher = Sha256::new();
        hasher.update(path.as_bytes());
        let hash = hex::encode(hasher.finalize());
        hash_to_path.insert(hash, path.clone());
        if let Some(name_str) = name.as_str() {
            name_to_path.insert(name_str.to_string(), path.clone());
        }
    }
    Ok(ProjectMappings {
        hash_to_path,
        name_to_path,
    })
}

fn load_default_project_mappings() -> Result<ProjectMappings> {
    let (_, projects_json) = paths()?;
    Ok(load_project_mappings(&projects_json).unwrap_or_default())
}

fn resolve_project_path(dir_name: &str, mappings: &ProjectMappings) -> Option<String> {
    if dir_name.len() == 64 && dir_name.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(p) = mappings.hash_to_path.get(dir_name) {
            return Some(p.clone());
        }
        return None;
    }
    mappings.name_to_path.get(dir_name).cloned()
}

pub(crate) fn is_chat_file(path: &Path) -> bool {
    if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
        return false;
    }
    let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    file_name.starts_with("session-")
        && path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            == Some("chats")
}

fn collect_chat_files(base_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for root in [base_dir.join("tmp"), base_dir.join("history")] {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path().to_path_buf();
            if is_chat_file(&path) && seen.insert(path.clone()) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn project_alias_from_chat_path(path: &Path) -> Option<String> {
    path.parent()?
        .parent()?
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

fn read_project_root_file(path: PathBuf) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_chat_project_path(
    path: &Path,
    base_dir: Option<&Path>,
    mappings: &ProjectMappings,
) -> Option<String> {
    let alias = project_alias_from_chat_path(path)?;
    if let Some(base_dir) = base_dir {
        if let Some(root) =
            read_project_root_file(base_dir.join("tmp").join(&alias).join(".project_root"))
        {
            return Some(root);
        }
        if let Some(root) =
            read_project_root_file(base_dir.join("history").join(&alias).join(".project_root"))
        {
            return Some(root);
        }
    }
    resolve_project_path(&alias, mappings)
}

fn read_chat_file_value(path: &Path) -> Result<serde_json::Value> {
    let text = fs::read_to_string(path)?;
    let mut session = serde_json::Map::new();
    let mut messages = Vec::new();
    let mut message_index_by_id: HashMap<String, usize> = HashMap::new();
    let mut last_updated: Option<String> = None;

    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let value: serde_json::Value = serde_json::from_str(line)?;
        if let Some(update) = value.get("$set").and_then(|v| v.as_object()) {
            if let Some(updated) = update.get("lastUpdated").and_then(|v| v.as_str()) {
                last_updated = Some(updated.to_string());
            }
            continue;
        }

        let Some(object) = value.as_object() else {
            continue;
        };
        if object.get("type").is_some() {
            if let Some(id) = object.get("id").and_then(|v| v.as_str()) {
                if let Some(index) = message_index_by_id.get(id).copied() {
                    messages[index] = value;
                } else {
                    message_index_by_id.insert(id.to_string(), messages.len());
                    messages.push(value);
                }
            } else {
                messages.push(value);
            }
            continue;
        }

        for (key, value) in object {
            session.insert(key.clone(), value.clone());
        }
    }

    if let Some(last_updated) = last_updated {
        session.insert(
            "lastUpdated".to_string(),
            serde_json::Value::String(last_updated),
        );
    }
    session.insert("messages".to_string(), serde_json::Value::Array(messages));
    Ok(serde_json::Value::Object(session))
}

fn parse_chat_file(
    path: &Path,
    base_dir: Option<&Path>,
    mappings: &ProjectMappings,
) -> Result<Option<SessionInfo>> {
    let value = read_chat_file_value(path)?;
    let Some(session_id) = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
    else {
        return Ok(None);
    };
    let messages = value
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let first_message_ts = messages
        .first()
        .and_then(|m| m.get("timestamp"))
        .and_then(|v| v.as_str())
        .and_then(parse_iso);
    let last_message_ts = messages.iter().rev().find_map(|m| {
        m.get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_iso)
    });
    let started_at = value
        .get("startTime")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .or(first_message_ts);
    let updated_at = value
        .get("lastUpdated")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .or(last_message_ts)
        .or_else(|| {
            fs::metadata(path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(system_time_to_millis)
        });
    let mut forked_from_agent: Option<Agent> = None;
    let mut forked_from_id: Option<String> = None;
    let first_user = messages
        .iter()
        .filter(|message| {
            normalize_gemini_role(message.get("type").and_then(|v| v.as_str()).unwrap_or(""))
                == "user"
        })
        .find_map(|message| {
            if forked_from_id.is_none() {
                if let Some(lineage) = cross_context_lineage_from_payload(message) {
                    forked_from_agent = Some(lineage.agent);
                    forked_from_id = Some(lineage.session_id);
                }
            }
            extract_gemini_display_or_message_text(message)
        })
        .and_then(|text| clean_history_user_preview_text(&strip_image_at_references(&text)))
        .map(|text| normalize_preview(&text));
    let project_path = resolve_chat_project_path(path, base_dir, mappings);
    let project_name = project_path.as_deref().and_then(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
    });
    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(Some(SessionInfo {
        id: session_id,
        agent: Agent::Gemini,
        forked_from_agent,
        forked_from_id,
        project_path,
        project_name,
        started_at,
        updated_at,
        message_count: messages.len(),
        rename_title: None,
        title: first_user.clone(),
        first_user_message: first_user,
        file_path: path.to_string_lossy().into_owned(),
        file_size,
        partial: false,
        available: true,
        archived: path.components().any(|c| c.as_os_str() == "history"),
        origin: crate::models::SessionOrigin::Chat,
        scheduled_task_id: None,
        is_auxiliary: false,
        subagents: Vec::new(),
    }))
}

fn normalize_gemini_role(role: &str) -> String {
    match role.to_ascii_lowercase().as_str() {
        "gemini" | "model" => "assistant".to_string(),
        "" => "user".to_string(),
        other => other.to_string(),
    }
}

fn first_non_empty_text<'a>(candidates: &[Option<&'a str>]) -> Option<&'a str> {
    candidates
        .iter()
        .filter_map(|candidate| candidate.map(str::trim))
        .find(|text| !text.is_empty())
}

fn extract_text_from_value_inner(value: &serde_json::Value, depth: usize) -> Option<String> {
    if depth > 6 {
        return None;
    }
    match value {
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| extract_text_from_value_inner(item, depth + 1))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(text) = first_non_empty_text(&[
                map.get("delta").and_then(|value| value.as_str()),
                map.get("text").and_then(|value| value.as_str()),
                map.get("message").and_then(|value| value.as_str()),
                map.get("content").and_then(|value| value.as_str()),
                map.get("output").and_then(|value| value.as_str()),
                map.get("result").and_then(|value| value.as_str()),
                map.get("response").and_then(|value| value.as_str()),
            ]) {
                return Some(text.to_string());
            }
            for key in [
                "content", "message", "part", "parts", "result", "output", "response", "data",
                "payload", "item", "items",
            ] {
                if let Some(nested) = map.get(key) {
                    if let Some(text) = extract_text_from_value_inner(nested, depth + 1) {
                        return Some(text);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_text_from_value(value: &serde_json::Value) -> Option<String> {
    extract_text_from_value_inner(value, 0)
}

fn extract_gemini_message_text(message: &serde_json::Value) -> Option<String> {
    message
        .get("content")
        .and_then(extract_text_from_value)
        .or_else(|| message.get("message").and_then(extract_text_from_value))
        .or_else(|| message.get("output").and_then(extract_text_from_value))
        .or_else(|| message.get("result").and_then(extract_text_from_value))
        .or_else(|| message.get("response").and_then(extract_text_from_value))
        .or_else(|| message.get("payload").and_then(extract_text_from_value))
        .or_else(|| message.get("data").and_then(extract_text_from_value))
}

fn extract_gemini_display_text(message: &serde_json::Value) -> Option<String> {
    message
        .get("displayContent")
        .and_then(extract_text_from_value)
        .or_else(|| {
            message
                .get("display_content")
                .and_then(extract_text_from_value)
        })
}

fn extract_gemini_display_or_message_text(message: &serde_json::Value) -> Option<String> {
    extract_gemini_display_text(message).or_else(|| extract_gemini_message_text(message))
}

fn is_image_path_candidate(path: &str) -> bool {
    let normalized = path
        .split('?')
        .next()
        .unwrap_or(path)
        .split('#')
        .next()
        .unwrap_or(path)
        .trim()
        .to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg", ".avif", ".heic", ".heif",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

fn normalize_file_uri_path(file_uri: &str) -> Option<String> {
    if !file_uri
        .get(..7)
        .map(|value| value.eq_ignore_ascii_case("file://"))
        .unwrap_or(false)
    {
        return None;
    }
    let mut remainder = file_uri[7..].trim();
    if remainder.is_empty() {
        return None;
    }
    if remainder.to_ascii_lowercase().starts_with("localhost/") {
        remainder = &remainder["localhost/".len()..];
    }
    Some(remainder.replace("%20", " "))
}

fn normalize_history_image_source(value: &str) -> String {
    let trimmed = value.trim();
    normalize_file_uri_path(trimmed).unwrap_or_else(|| trimmed.to_string())
}

fn collect_content_image_sources(value: &serde_json::Value, output: &mut Vec<String>) {
    if let Some(array) = value.as_array() {
        for item in array {
            collect_content_image_sources(item, output);
        }
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(inline_data) = object
        .get("inlineData")
        .or_else(|| object.get("inline_data"))
        .and_then(|node| node.as_object())
    {
        if let Some(data) = inline_data
            .get("data")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let mime = inline_data
                .get("mimeType")
                .or_else(|| inline_data.get("mime_type"))
                .and_then(|value| value.as_str())
                .unwrap_or("image/png");
            if mime.to_ascii_lowercase().starts_with("image/") && data.len() <= 3_000_000 {
                output.push(format!("data:{mime};base64,{data}"));
            }
        }
    }
    if let Some(file_data) = object
        .get("fileData")
        .or_else(|| object.get("file_data"))
        .and_then(|node| node.as_object())
    {
        if let Some(file_uri) = file_data
            .get("fileUri")
            .or_else(|| file_data.get("file_uri"))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let mime_type = file_data
                .get("mimeType")
                .or_else(|| file_data.get("mime_type"))
                .and_then(|value| value.as_str());
            if mime_type
                .map(|value| value.to_ascii_lowercase().starts_with("image/"))
                .unwrap_or_else(|| is_image_path_candidate(file_uri))
            {
                output.push(normalize_history_image_source(file_uri));
            }
        }
    }
    for nested in object.values() {
        collect_content_image_sources(nested, output);
    }
}

fn dedupe_string_list(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn extract_message_images(message: &serde_json::Value) -> Vec<String> {
    let mut images = Vec::new();
    if let Some(content) = message.get("content") {
        collect_content_image_sources(content, &mut images);
    }
    dedupe_string_list(images)
}

fn gemini_user_prompt_content(text: &str, images: &[String]) -> Vec<Value> {
    let mut content = Vec::new();
    if !text.trim().is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    for image in images {
        content.push(json!({
            "type": "image",
            "uri": image,
            "mimeType": image_mime_type(image),
        }));
    }
    content
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

fn strip_image_at_references(text: &str) -> String {
    text.split_whitespace()
        .filter(|part| {
            !(part.starts_with('@') && is_image_path_candidate(part.trim_start_matches('@')))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn tool_call_is_error(call: &serde_json::Value, output_preview: Option<&str>) -> bool {
    call.get("status")
        .and_then(|v| v.as_str())
        .map(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "error" | "failed" | "failure" | "cancelled" | "canceled"
            )
        })
        .unwrap_or(false)
        || output_preview
            .map(|output| {
                output
                    .trim_start()
                    .to_ascii_lowercase()
                    .starts_with("error")
            })
            .unwrap_or(false)
}

fn gemini_permission_event(
    message: &serde_json::Value,
    ts: Option<i64>,
) -> Option<Vec<AcpProtocolMessage>> {
    let permission = message
        .get("permission")
        .or_else(|| message.get("permissionRequest"))
        .or_else(|| message.get("permission_request"))
        .or_else(|| message.get("toolPermission"))
        .or_else(|| message.get("tool_permission"))
        .or_else(|| message.get("confirmation"))
        .or_else(|| message.get("confirmationRequest"))
        .or_else(|| message.get("confirmation_request"))?;
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
        .or_else(|| message.get("id"))
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
        .and_then(Value::as_str)
        .map(String::from);
    let tool_name = string_value(fields, "title")
        .or_else(|| string_value(fields, "name"))
        .or_else(|| string_value(permission, "toolName"))
        .or_else(|| string_value(permission, "tool_name"))
        .unwrap_or_else(|| "tool".to_string());
    let input = fields
        .get("rawInput")
        .or_else(|| fields.get("raw_input"))
        .or_else(|| fields.get("input"))
        .or_else(|| permission.get("args"))
        .cloned()
        .unwrap_or_else(|| permission.clone());
    let options = permission
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let selected = permission
        .get("selectedOptionId")
        .or_else(|| permission.get("selected_option_id"))
        .or_else(|| permission.get("optionId"))
        .or_else(|| permission.get("option_id"))
        .and_then(Value::as_str)
        .map(String::from);
    let cancelled = permission
        .get("cancelled")
        .and_then(Value::as_bool)
        .or_else(|| {
            permission
                .get("outcome")
                .and_then(|outcome| outcome.get("outcome"))
                .and_then(Value::as_str)
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

fn string_value(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(String::from)
}

fn gemini_file_edit_message(
    call: &serde_json::Value,
    ts: Option<i64>,
    project_path: Option<&str>,
) -> Option<AcpProtocolMessage> {
    let result_display = call.get("resultDisplay")?;
    let file_diff = result_display
        .get("fileDiff")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let path = result_display
        .get("filePath")
        .or_else(|| result_display.get("file_path"))
        .and_then(|value| value.as_str())
        .or_else(|| {
            call.get("args")
                .and_then(|args| args.get("file_path").or_else(|| args.get("path")))
                .and_then(|value| value.as_str())
        })?;
    let display_path = gemini_edit_display_path(call, result_display, path, project_path);
    let additions = diff_line_count(file_diff, '+');
    let deletions = diff_line_count(file_diff, '-');
    let old_content = result_display
        .get("originalContent")
        .and_then(|value| value.as_str());
    let new_content = result_display
        .get("newContent")
        .and_then(|value| value.as_str());
    let edit = serde_json::json!({
        "path": path,
        "displayPath": display_path,
        "kind": "edit",
        "additions": additions,
        "deletions": deletions,
        "detail": file_diff,
        "patch": normalize_gemini_file_diff(path, file_diff),
        "oldContent": old_content,
        "newContent": new_content,
    });
    file_edit_message("gemini", vec![edit], ts)
}

fn gemini_edit_display_path(
    call: &serde_json::Value,
    result_display: &serde_json::Value,
    path: &str,
    project_path: Option<&str>,
) -> String {
    if let Some(arg_path) = call
        .get("args")
        .and_then(|args| args.get("file_path").or_else(|| args.get("path")))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return arg_path.to_string();
    }
    if let Some(relative_path) = project_relative_path(path, project_path) {
        return relative_path;
    }
    result_display
        .get("fileName")
        .or_else(|| result_display.get("file_name"))
        .and_then(|value| value.as_str())
        .unwrap_or(path)
        .to_string()
}

fn project_relative_path(path: &str, project_path: Option<&str>) -> Option<String> {
    let project_path = project_path?.trim_end_matches('/');
    let path = path.trim();
    if project_path.is_empty() || path.is_empty() {
        return None;
    }
    path.strip_prefix(project_path)
        .and_then(|relative| relative.strip_prefix('/'))
        .map(String::from)
        .filter(|relative| !relative.is_empty())
}

fn normalize_gemini_file_diff(path: &str, diff: &str) -> String {
    if diff.starts_with("diff --git ") {
        return diff.to_string();
    }
    let body = diff
        .lines()
        .filter(|line| {
            !line.starts_with("Index: ")
                && !line.starts_with("===")
                && !line.starts_with("--- ")
                && !line.starts_with("+++ ")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let normalized = path.trim_start_matches('/');
    format!("diff --git a/{normalized} b/{normalized}\n--- a/{normalized}\n+++ b/{normalized}\n{body}\n")
}

fn diff_line_count(diff: &str, marker: char) -> usize {
    diff.lines()
        .filter(|line| {
            line.starts_with(marker) && !line.starts_with("+++") && !line.starts_with("---")
        })
        .count()
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
        .filter_map(|edit| edit.get("additions").and_then(|value| value.as_i64()))
        .sum();
    let deletions: i64 = edits
        .iter()
        .filter_map(|edit| edit.get("deletions").and_then(|value| value.as_i64()))
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

fn output_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn read_chat_history_acp_messages_with_locations(
    path: &Path,
    session_id: &str,
) -> Result<Vec<HistoryAcpMessage>> {
    let value = read_chat_file_value(path)?;
    if value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        != Some(session_id)
    {
        return Ok(Vec::new());
    }

    let file_path = path.to_string_lossy().to_string();
    let location = SourceLocation::file(file_path);
    let project_path = resolve_chat_project_path(
        path,
        paths()
            .ok()
            .and_then(|(tmp_dir, _)| tmp_dir.parent().map(Path::to_path_buf))
            .as_deref(),
        &load_default_project_mappings().unwrap_or_default(),
    );
    let mut out = Vec::new();
    let raw_messages = value
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    for (index, raw) in raw_messages.into_iter().enumerate() {
        let counter = index + 1;
        let role = normalize_gemini_role(raw.get("type").and_then(|v| v.as_str()).unwrap_or(""));
        let ts = raw
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_iso);

        if role == "user" {
            let images = extract_message_images(&raw);
            let mut text = extract_gemini_display_or_message_text(&raw).unwrap_or_default();
            if !images.is_empty() {
                text = strip_image_at_references(&text);
            }
            let message = if images.is_empty() {
                history_user_message(text, ts)
            } else {
                history_prompt_message(gemini_user_prompt_content(&text, &images), ts)
            };
            out.push(HistoryAcpMessage {
                message,
                timestamp: ts,
                location: location.clone(),
                synthetic: true,
            });
            continue;
        }

        if role != "assistant" {
            continue;
        }

        if let Some(permission_messages) = gemini_permission_event(&raw, ts) {
            for message in permission_messages {
                out.push(HistoryAcpMessage {
                    message,
                    timestamp: ts,
                    location: location.clone(),
                    synthetic: true,
                });
            }
        }

        if let Some(thoughts) = raw.get("thoughts").and_then(|v| v.as_array()) {
            for thought in thoughts {
                let subject = thought
                    .get("subject")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let description = thought
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let text = match (subject, description) {
                    (Some(subject), Some(description)) => format!("{subject}: {description}"),
                    (Some(subject), None) => subject.to_string(),
                    (None, Some(description)) => description.to_string(),
                    (None, None) => continue,
                };
                out.push(HistoryAcpMessage {
                    message: history_thought_message(text, ts),
                    timestamp: ts,
                    location: location.clone(),
                    synthetic: true,
                });
            }
        }

        if let Some(tool_calls) = raw.get("toolCalls").and_then(|v| v.as_array()) {
            for call in tool_calls {
                let tool_id = call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| format!("gemini-tool-{counter}"));
                let tool_name = call
                    .get("displayName")
                    .or_else(|| call.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                out.push(HistoryAcpMessage {
                    message: history_tool_call_message(
                        Some(tool_id.clone()),
                        tool_name,
                        call.get("args")
                            .or_else(|| call.get("input"))
                            .cloned()
                            .unwrap_or(Value::Null),
                        ts,
                    ),
                    timestamp: ts,
                    location: location.clone(),
                    synthetic: true,
                });

                let output = call
                    .get("resultDisplay")
                    .filter(|v| !v.as_str().map(str::trim).unwrap_or("").is_empty())
                    .or_else(|| call.get("result"))
                    .cloned();
                if let Some(output) = output {
                    let error = tool_call_is_error(call, Some(&output_text(&output)));
                    let output = if error {
                        serde_json::json!({ "error": true, "output": output })
                    } else {
                        output
                    };
                    out.push(HistoryAcpMessage {
                        message: history_tool_result_message(Some(tool_id), output, ts),
                        timestamp: ts,
                        location: location.clone(),
                        synthetic: true,
                    });
                }
                if let Some(edit_message) =
                    gemini_file_edit_message(call, ts, project_path.as_deref())
                {
                    out.push(HistoryAcpMessage {
                        message: edit_message,
                        timestamp: ts,
                        location: location.clone(),
                        synthetic: true,
                    });
                }
            }
        }

        if let Some(text) = extract_gemini_display_or_message_text(&raw) {
            if !text.trim().is_empty() {
                out.push(HistoryAcpMessage {
                    message: history_assistant_message(text, ts),
                    timestamp: ts,
                    location: location.clone(),
                    synthetic: true,
                });
            }
        }
    }
    Ok(out)
}

fn parse_iso(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn available_removed_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".to_string());
    for i in 1.. {
        let candidate = parent.join(format!("{file_name}.{i}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn move_file(src: &Path, dst: &Path) -> Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            fs::copy(src, dst).map_err(|copy_err| {
                anyhow::anyhow!(
                    "move {} to {} failed: rename: {}; copy fallback: {}",
                    src.display(),
                    dst.display(),
                    rename_err,
                    copy_err
                )
            })?;
            fs::remove_file(src).map_err(|remove_err| {
                let _ = fs::remove_file(dst);
                anyhow::anyhow!(
                    "remove original after copying {} to {} failed: {}",
                    src.display(),
                    dst.display(),
                    remove_err
                )
            })?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_chat_file, read_history_acp_messages_with_locations, remove_session_from_logs,
        ProjectMappings,
    };
    use crate::models::Agent;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tmp(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parse_chat_file_and_read_messages_support_session_jsonl_with_images() {
        let dir = unique_tmp("gemini-chat-parser");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            dir.join(".gemini")
                .join("tmp")
                .join("alias")
                .join(".project_root"),
            "/tmp/project",
        )
        .unwrap();
        let image_path = dir.join("image.png");
        fs::write(&image_path, b"png").unwrap();
        let path = chats.join("session-2026-05-19T00-00-new.jsonl");
        let user_line = format!(
            r#"{{"id":"u1","type":"user","timestamp":"2026-05-19T00:00:00Z","displayContent":"look @{image}","content":[{{"text":"look"}},{{"fileData":{{"fileUri":"file://{image}","mimeType":"image/png"}}}}]}}"#,
            image = image_path.to_string_lossy()
        );
        fs::write(
            &path,
            format!(
                "{}\n{}\n{}\n",
                r#"{"sessionId":"new","startTime":"2026-05-19T00:00:00Z","lastUpdated":"2026-05-19T00:00:04Z","kind":"main"}"#,
                user_line,
                r#"{"id":"a1","type":"model","timestamp":"2026-05-19T00:00:01Z","thoughts":[{"subject":"Plan","description":"inspect"}],"toolCalls":[{"id":"tool-1","displayName":"ReadFile","args":{"path":"README.md"},"resultDisplay":"contents"}],"content":{"parts":[{"text":"done"}]}}"#,
            ),
        )
        .unwrap();

        let mappings = super::ProjectMappings::default();
        let info = super::parse_chat_file(&path, Some(&dir.join(".gemini")), &mappings)
            .unwrap()
            .unwrap();
        assert_eq!(info.id, "new");
        assert_eq!(info.project_path.as_deref(), Some("/tmp/project"));
        assert_eq!(info.first_user_message.as_deref(), Some("look"));

        let messages = read_history_acp_messages_with_locations(&path, "new").unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].message.method, "session/prompt");
        assert!(messages[0].message.data["prompt"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block["type"].as_str() == Some("image")));
        assert_eq!(
            messages[1].message.update_type.as_deref(),
            Some("agent_thought_chunk")
        );
        assert_eq!(
            messages[2].message.update_type.as_deref(),
            Some("tool_call")
        );
        assert_eq!(
            messages[2].message.data["update"]["toolCallId"].as_str(),
            Some("tool-1")
        );
        assert_eq!(
            messages[3].message.update_type.as_deref(),
            Some("tool_call_update")
        );
        assert_eq!(
            messages[4].message.update_type.as_deref(),
            Some("agent_message_chunk")
        );
        assert_eq!(
            messages[4].message.data["update"]["content"][0]["text"].as_str(),
            Some("done")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_chat_file_and_read_messages_support_session_jsonl() {
        let dir = unique_tmp("gemini-chat-jsonl-parser");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            dir.join(".gemini")
                .join("tmp")
                .join("alias")
                .join(".project_root"),
            "/tmp/jsonl-project",
        )
        .unwrap();
        let path = chats.join("session-2026-05-24T12-00-jsonl.jsonl");
        fs::write(
            &path,
            r#"{"sessionId":"jsonl","startTime":"2026-05-24T12:00:00Z","lastUpdated":"2026-05-24T12:00:00Z","kind":"main"}
{"id":"u1","timestamp":"2026-05-24T12:00:01Z","type":"user","content":[{"text":"hello"}]}
{"$set":{"lastUpdated":"2026-05-24T12:00:02Z"}}
{"id":"a1","timestamp":"2026-05-24T12:00:03Z","type":"gemini","content":"","thoughts":[{"subject":"Plan","description":"inspect"}]}
{"id":"a1","timestamp":"2026-05-24T12:00:03Z","type":"gemini","content":"done","thoughts":[{"subject":"Plan","description":"inspect"}],"toolCalls":[{"id":"tool-1","name":"read_file","args":{"path":"README.md"},"resultDisplay":"contents"}]}
"#,
        )
        .unwrap();

        let mappings = super::ProjectMappings::default();
        let info = super::parse_chat_file(&path, Some(&dir.join(".gemini")), &mappings)
            .unwrap()
            .unwrap();
        assert_eq!(info.id, "jsonl");
        assert_eq!(info.project_path.as_deref(), Some("/tmp/jsonl-project"));
        assert_eq!(info.message_count, 2);
        assert_eq!(info.first_user_message.as_deref(), Some("hello"));

        let messages = read_history_acp_messages_with_locations(&path, "jsonl").unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].message.method, "session/prompt");
        assert_eq!(
            messages[1].message.update_type.as_deref(),
            Some("agent_thought_chunk")
        );
        assert_eq!(
            messages[2].message.update_type.as_deref(),
            Some("tool_call")
        );
        assert_eq!(
            messages[2].message.data["update"]["toolCallId"].as_str(),
            Some("tool-1")
        );
        assert_eq!(
            messages[3].message.update_type.as_deref(),
            Some("tool_call_update")
        );
        assert_eq!(
            messages[4].message.update_type.as_deref(),
            Some("agent_message_chunk")
        );
        assert_eq!(
            messages[4].message.data["update"]["content"][0]["text"].as_str(),
            Some("done")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_history_acp_messages_with_locations_adds_file_edit_for_edit_result_display() {
        let dir = unique_tmp("gemini-edit-file-summary");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            dir.join(".gemini")
                .join("tmp")
                .join("alias")
                .join(".project_root"),
            "/tmp/jsonl-project",
        )
        .unwrap();
        let path = chats.join("session-2026-05-24T12-00-edit.jsonl");
        fs::write(
            &path,
            r#"{"sessionId":"edit-jsonl","startTime":"2026-05-24T12:00:00Z","lastUpdated":"2026-05-24T12:00:00Z","kind":"main"}
{"id":"u1","timestamp":"2026-05-24T12:00:01Z","type":"user","content":[{"text":"edit"}]}
{"id":"a1","timestamp":"2026-05-24T12:00:03Z","type":"gemini","content":"done","toolCalls":[{"id":"tool-1","name":"replace","displayName":"Edit","args":{"file_path":"src/main.rs","old_string":"old","new_string":"new"},"resultDisplay":{"fileDiff":"Index: main.rs\n===================================================================\n--- main.rs\tCurrent\n+++ main.rs\tProposed\n@@ -1 +1 @@\n-old\n+new\n","fileName":"main.rs","filePath":"/tmp/jsonl-project/src/main.rs","originalContent":"old\n","newContent":"new\n"}}]}
"#,
        )
        .unwrap();

        let messages = read_history_acp_messages_with_locations(&path, "edit-jsonl").unwrap();
        let file_edit = messages
            .iter()
            .map(|row| &row.message)
            .find(|message| message.update_type.as_deref() == Some("file_edit"))
            .expect("expected Gemini edit result to produce file_edit message");
        let summary = &file_edit.data["update"];
        assert_eq!(
            summary.get("source").and_then(|v| v.as_str()),
            Some("gemini")
        );
        assert_eq!(summary.get("files").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(summary.get("additions").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(summary.get("deletions").and_then(|v| v.as_u64()), Some(1));
        let edit = &summary["edits"][0];
        assert_eq!(
            edit.get("displayPath").and_then(|v| v.as_str()),
            Some("src/main.rs")
        );
        assert!(edit
            .get("patch")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .contains("diff --git"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_history_acp_messages_with_locations_parses_structured_permission() {
        let dir = unique_tmp("gemini-permission-jsonl");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        let path = chats.join("session-2026-05-29T05-38-permission.jsonl");
        fs::write(
            &path,
            r#"{"sessionId":"permission-jsonl","startTime":"2026-05-29T05:38:00Z","lastUpdated":"2026-05-29T05:38:55Z","kind":"main"}
{"id":"perm-line","timestamp":"2026-05-29T05:38:55Z","type":"gemini","content":"","permission":{"requestId":"perm-1","toolCall":{"toolCallId":"tool-1","fields":{"title":"ShellCommand","rawInput":{"command":"pnpm test"}}},"options":[{"optionId":"allow","name":"Allow","kind":"allow"},{"optionId":"reject","name":"Reject","kind":"reject"}],"selectedOptionId":"allow"}}
"#,
        )
        .unwrap();

        let messages = read_history_acp_messages_with_locations(&path, "permission-jsonl").unwrap();
        assert_eq!(messages.len(), 2);
        let request = &messages[0].message;
        assert_eq!(request.method, "session/request_permission");
        assert_eq!(request.request_id.as_deref(), Some("perm-1"));
        assert_eq!(request.data["toolCall"]["fields"]["title"], "ShellCommand");
        assert_eq!(
            request.data["toolCall"]["fields"]["rawInput"]["command"],
            "pnpm test"
        );
        assert_eq!(request.data["options"].as_array().unwrap().len(), 2);
        assert_eq!(
            messages[1].message.data["outcome"]["optionId"].as_str(),
            Some("allow")
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_session_from_chat_jsonl_moves_whole_file() {
        let home = unique_tmp("gemini-chat-remove");
        let source = home
            .join(".gemini")
            .join("tmp")
            .join("sessio")
            .join("chats")
            .join("session-2026-05-25T05-14-test.jsonl");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            r#"{"sessionId":"drop","startTime":"2026-05-25T05:14:25.987Z","lastUpdated":"2026-05-25T05:14:25.987Z","kind":"main"}
{"id":"i1","timestamp":"2026-05-25T05:15:25.960Z","type":"info","content":"Waiting for authentication...\n"}
{"$set":{"lastUpdated":"2026-05-25T05:15:25.961Z"}}
"#,
        )
        .unwrap();

        let removed_root = home.join(".sessio").join("removed-sessions");
        assert!(remove_session_from_logs(&source, "drop", &home, &removed_root).unwrap());
        assert!(!source.exists());

        let removed_path = removed_root
            .join(".gemini")
            .join("tmp")
            .join("sessio")
            .join("chats")
            .join("session-2026-05-25T05-14-test.jsonl");
        let removed = fs::read_to_string(&removed_path).unwrap();
        assert!(removed.contains(r#""sessionId":"drop""#));
        assert!(removed.contains(r#""type":"info""#));

        assert!(!remove_session_from_logs(&source, "drop", &home, &removed_root).unwrap());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn read_history_acp_messages_with_locations_sanitizes_cross_context_attachment_for_user() {
        let dir = unique_tmp("gemini-cross-context");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            dir.join(".gemini")
                .join("tmp")
                .join("alias")
                .join(".project_root"),
            "/tmp/cross-context-project",
        )
        .unwrap();
        let path = chats.join("session-2026-05-28T15-39-cross.jsonl");
        let line = serde_json::json!({
            "id": "u1",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "type": "user",
            "content": [
                { "text": "不错，写入md文档" },
                { "text": "[@sessio-cross-context-abc.md](file:///tmp/.cross-context/sessio-cross-context-abc.md)" },
                { "text": "\n<context ref=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\">\n<sessio-upload-file uri=\"file:///tmp/.cross-context/sessio-cross-context-abc.md\" name=\"sessio-cross-context-abc.md\" mimeType=\"text/markdown\">\n# Continued session from agent\n[user]\nhi\n</sessio-upload-file>\n</context>" }
            ]
        })
        .to_string();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                r#"{"sessionId":"cross-jsonl","startTime":"2026-05-28T15:39:00Z","lastUpdated":"2026-05-28T15:39:00Z","kind":"main"}"#,
                line
            ),
        )
        .unwrap();

        let messages = read_history_acp_messages_with_locations(&path, "cross-jsonl").unwrap();
        let user = messages
            .iter()
            .find(|row| row.message.method == "session/prompt")
            .expect("user message");
        let text = user.message.data["prompt"][0]["text"].as_str().unwrap();
        assert!(text.contains("不错，写入md文档"), "{}", text);
        assert!(text.contains("sessio-cross-context-abc.md"), "{}", text);

        let turns = crate::turns::session_history_turns_from_acp_messages(&messages);
        let display_text = turns[0].blocks[0].blocks[0].text.as_deref().unwrap();
        assert!(
            display_text.contains("不错，写入md文档"),
            "{}",
            display_text
        );
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
        assert!(
            !display_text.contains("Continued session from agent"),
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

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_chat_file_reads_cross_context_lineage_from_user_attachment() {
        let dir = unique_tmp("gemini-chat-lineage");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        fs::write(
            dir.join(".gemini")
                .join("tmp")
                .join("alias")
                .join(".project_root"),
            "/tmp/cross-context-project",
        )
        .unwrap();
        let context_path = dir.join("sessio-cross-context-parent.md");
        fs::write(
            &context_path,
            r#"<!-- sessio-cross:start source_agent="claude" source_session_id="claude-parent" source_file_path="/tmp/parent.jsonl" -->"#,
        )
        .unwrap();
        let path = chats.join("session-2026-05-28T15-39-lineage.jsonl");
        let line = serde_json::json!({
            "id": "u1",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "type": "user",
            "content": [
                { "text": "继续" },
                { "text": format!("[@ctx](file://{})", context_path.display()) }
            ]
        })
        .to_string();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                r#"{"sessionId":"gemini-lineage","startTime":"2026-05-28T15:39:00Z","lastUpdated":"2026-05-28T15:39:00Z","kind":"main"}"#,
                line
            ),
        )
        .unwrap();

        let mappings = ProjectMappings::default();
        let info = parse_chat_file(&path, Some(&dir.join(".gemini")), &mappings)
            .unwrap()
            .unwrap();
        assert_eq!(info.forked_from_agent, Some(Agent::Claude));
        assert_eq!(info.forked_from_id.as_deref(), Some("claude-parent"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_history_acp_messages_strips_leading_ide_context_from_user_prompt() {
        let dir = unique_tmp("gemini-ide-context");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        let path = chats.join("session-2026-05-28T15-39-ide.jsonl");
        let line = serde_json::json!({
            "id": "u1",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "type": "user",
            "content": "<ide_opened_file>secret.rs</ide_opened_file>\nreal request"
        })
        .to_string();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                r#"{"sessionId":"gemini-ide","startTime":"2026-05-28T15:39:00Z","lastUpdated":"2026-05-28T15:39:00Z","kind":"main"}"#,
                line
            ),
        )
        .unwrap();

        let messages = read_history_acp_messages_with_locations(&path, "gemini-ide").unwrap();
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

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_history_acp_messages_drops_system_noise_after_ide_context() {
        let dir = unique_tmp("gemini-ide-noise");
        let chats = dir.join(".gemini").join("tmp").join("alias").join("chats");
        fs::create_dir_all(&chats).unwrap();
        let path = chats.join("session-2026-05-28T15-39-ide-noise.jsonl");
        let line = serde_json::json!({
            "id": "u1",
            "timestamp": "2026-05-28T15:39:00.000Z",
            "type": "user",
            "content": "<ide_opened_file>secret.rs</ide_opened_file>\n<environment_context>\nnoise\n</environment_context>"
        })
        .to_string();
        fs::write(
            &path,
            format!(
                "{}\n{}\n",
                r#"{"sessionId":"gemini-ide-noise","startTime":"2026-05-28T15:39:00Z","lastUpdated":"2026-05-28T15:39:00Z","kind":"main"}"#,
                line
            ),
        )
        .unwrap();

        let messages = read_history_acp_messages_with_locations(&path, "gemini-ide-noise").unwrap();
        assert_eq!(messages.len(), 1);
        let turns = crate::turns::session_history_turns_from_acp_messages(&messages);
        assert!(turns[0].blocks.is_empty(), "{turns:#?}");

        let _ = fs::remove_dir_all(&dir);
    }
}
