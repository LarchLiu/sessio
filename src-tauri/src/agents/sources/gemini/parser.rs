use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::agents::sources::system_time_to_millis;
use crate::models::{is_system_noise, normalize_preview, Agent, SessionInfo, SessionMessage};

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let (tmp_dir, projects_json) = paths()?;
    let base_dir = tmp_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| tmp_dir.clone());
    let mappings = load_project_mappings(&projects_json).unwrap_or_default();

    let mut out = Vec::new();
    if tmp_dir.exists() {
        for entry in fs::read_dir(&tmp_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let dir = entry.path();
            let logs_path = dir.join("logs.json");
            if !logs_path.exists() {
                continue;
            }
            let dir_name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let project_path = resolve_project_path(dir_name, &mappings);

            match parse_logs(&logs_path, project_path.as_deref()) {
                Ok(sessions) => out.extend(sessions),
                Err(e) => log::warn!("gemini parse {} failed: {e}", logs_path.display()),
            }
        }
    }
    for chat_path in collect_chat_files(&base_dir) {
        match parse_chat_file(&chat_path, Some(&base_dir), &mappings) {
            Ok(Some(session)) => out.push(session),
            Ok(None) => {}
            Err(e) => log::warn!("gemini parse {} failed: {e}", chat_path.display()),
        }
    }
    Ok(out)
}

pub fn paths() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok((
        home.join(".gemini").join("tmp"),
        home.join(".gemini").join("projects.json"),
    ))
}

pub fn parse_logs_file(path: &Path) -> Result<Vec<SessionInfo>> {
    if is_chat_file(path) {
        return Ok(
            parse_chat_file(path, None, &load_default_project_mappings()?)?
                .into_iter()
                .collect(),
        );
    }
    let (_, projects_json) = paths()?;
    let mappings = load_project_mappings(&projects_json).unwrap_or_default();
    let dir_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let project_path = resolve_project_path(dir_name, &mappings);
    parse_logs(path, project_path.as_deref())
}

pub fn read_messages(path: &Path, session_id: &str) -> Result<Vec<SessionMessage>> {
    Ok(read_messages_with_locations(path, session_id)?
        .into_iter()
        .map(|(m, _)| m)
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
    if !path.is_file() {
        return Err(anyhow::anyhow!(
            "session path is not a file: {}",
            path.display()
        ));
    }

    let text = fs::read(path)?;
    let entries = scan_json_array_entries(&text)?;

    let mut kept = Vec::with_capacity(entries.len());
    let mut removed = Vec::new();
    for entry in entries {
        let item: serde_json::Value = serde_json::from_slice(&text[entry.start..entry.end])?;
        let sid = item.get("sessionId").and_then(|x| x.as_str()).unwrap_or("");
        if sid == session_id {
            removed.push(item);
        } else {
            kept.push(item);
        }
    }

    if removed.is_empty() {
        return Ok(false);
    }

    let relative = path
        .strip_prefix(home)
        .map_err(|_| anyhow::anyhow!("session file is outside home: {}", path.display()))?;
    let removed_path = removed_root.join(relative);
    if let Some(parent) = removed_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut removed_existing = read_json_array(&removed_path).unwrap_or_default();
    removed_existing.extend(removed);
    write_json_array(&removed_path, &removed_existing)?;
    write_json_array(path, &kept)?;
    Ok(true)
}

// Gemini stores all sessions under ~/.gemini/tmp/<dir>/logs.json as a single
// JSON array. We scan the array boundaries and then deserialize each object
// from its raw byte range so per-message offsets stay precise without a full
// custom parser.
pub fn read_messages_with_locations(
    path: &Path,
    session_id: &str,
) -> Result<
    Vec<(
        SessionMessage,
        crate::agents::sources::types::SourceLocation,
    )>,
> {
    if is_chat_file(path) {
        return read_chat_messages_with_locations(path, session_id);
    }
    let text = fs::read(path)?;
    let entries = scan_json_array_entries(&text)?;
    let mut out = Vec::new();
    let file_path = path.to_string_lossy().to_string();
    for entry in entries {
        let item: serde_json::Value = serde_json::from_slice(&text[entry.start..entry.end])?;
        let sid = item.get("sessionId").and_then(|x| x.as_str()).unwrap_or("");
        if sid != session_id {
            continue;
        }
        let role =
            normalize_gemini_role(item.get("type").and_then(|x| x.as_str()).unwrap_or("user"));
        let text = extract_gemini_message_text(&item).unwrap_or_default();
        let ts = item
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso);
        if text.is_empty() {
            continue;
        }
        out.push((
            SessionMessage {
                role,
                text,
                timestamp: ts,
                tool_call_id: None,
            },
            crate::agents::sources::types::SourceLocation {
                file_path: file_path.clone(),
                line_start: Some(entry.line_start),
                line_end: Some(entry.line_end),
                byte_start: Some(entry.start as u64),
                byte_end: Some(entry.end as u64),
            },
        ));
    }
    Ok(out)
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
    if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("json") {
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

fn parse_chat_file(
    path: &Path,
    base_dir: Option<&Path>,
    mappings: &ProjectMappings,
) -> Result<Option<SessionInfo>> {
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
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
    let first_user = messages
        .iter()
        .filter(|message| {
            normalize_gemini_role(message.get("type").and_then(|v| v.as_str()).unwrap_or(""))
                == "user"
        })
        .find_map(|message| extract_gemini_display_or_message_text(message))
        .map(|text| normalize_preview(&strip_image_at_references(&text)))
        .filter(|text| !text.trim().is_empty() && !is_system_noise(text));
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
        forked_from_id: None,
        project_path,
        project_name,
        started_at,
        updated_at,
        message_count: messages.len(),
        title: first_user.clone(),
        first_user_message: first_user,
        file_path: path.to_string_lossy().into_owned(),
        file_size,
        partial: false,
        available: true,
        archived: path.components().any(|c| c.as_os_str() == "history"),
        subagents: Vec::new(),
    }))
}

fn parse_logs(path: &Path, project_path: Option<&str>) -> Result<Vec<SessionInfo>> {
    let text = fs::read(path)?;
    let entries = match scan_json_array_entries(&text) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn!("gemini logs.json parse error {}: {e}", path.display());
            return Ok(Vec::new());
        }
    };

    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let file_path = path.to_string_lossy().into_owned();
    let file_mtime = fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(system_time_to_millis);

    let mut groups: HashMap<String, GeminiAgg> = HashMap::new();
    for entry in entries {
        let item: serde_json::Value = match serde_json::from_slice(&text[entry.start..entry.end]) {
            Ok(value) => value,
            Err(e) => {
                log::warn!("gemini logs.json item parse error {}: {e}", path.display());
                continue;
            }
        };
        let sid = match item.get("sessionId").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let role = normalize_gemini_role(item.get("type").and_then(|x| x.as_str()).unwrap_or(""));
        let text = extract_gemini_message_text(&item).unwrap_or_default();
        let ts = item
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso);

        let agg = groups.entry(sid).or_default();
        agg.count += 1;
        if let Some(t) = ts {
            agg.earliest = Some(agg.earliest.map_or(t, |e| e.min(t)));
            agg.latest = Some(agg.latest.map_or(t, |e| e.max(t)));
        }
        if agg.first_user.is_none()
            && role == "user"
            && !text.trim().is_empty()
            && !text.trim_start().starts_with('/')
            && !is_system_noise(&text)
        {
            agg.first_user = Some(normalize_preview(&strip_image_at_references(&text)));
        }
    }

    let project_name = project_path.and_then(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
    });

    let mut out = Vec::with_capacity(groups.len());
    for (sid, agg) in groups {
        let title = gemini_session_title(&sid).or_else(|| agg.first_user.clone());
        out.push(SessionInfo {
            id: sid,
            agent: Agent::Gemini,
            forked_from_id: None,
            project_path: project_path.map(String::from),
            project_name: project_name.clone(),
            started_at: agg.earliest,
            updated_at: agg.latest.or(file_mtime),
            message_count: agg.count,
            title,
            first_user_message: agg.first_user,
            file_path: file_path.clone(),
            file_size,
            partial: false,
            available: true,
            archived: false,
            subagents: Vec::new(),
        });
    }
    Ok(out)
}

fn gemini_session_title(_session_id: &str) -> Option<String> {
    None
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

fn read_chat_messages_with_locations(
    path: &Path,
    session_id: &str,
) -> Result<
    Vec<(
        SessionMessage,
        crate::agents::sources::types::SourceLocation,
    )>,
> {
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    if value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        != Some(session_id)
    {
        return Ok(Vec::new());
    }

    let file_path = path.to_string_lossy().to_string();
    let location = crate::agents::sources::types::SourceLocation::file(file_path);
    let mut out = Vec::new();
    let raw_messages = value
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    let mut counter = 0usize;

    for raw in raw_messages {
        counter += 1;
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
                for (idx, image) in images.iter().enumerate() {
                    if !text.trim().is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&format!("![Image #{}]({})", idx + 1, image));
                }
            }
            if text.trim().is_empty() || is_system_noise(&text) {
                continue;
            }
            out.push((
                SessionMessage {
                    role: "user".to_string(),
                    text,
                    timestamp: ts,
                    tool_call_id: None,
                },
                location.clone(),
            ));
            continue;
        }

        if role != "assistant" {
            continue;
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
                out.push((
                    SessionMessage {
                        role: "thinking".to_string(),
                        text,
                        timestamp: ts,
                        tool_call_id: None,
                    },
                    location.clone(),
                ));
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
                let input = call
                    .get("args")
                    .or_else(|| call.get("input"))
                    .and_then(|v| serde_json::to_string_pretty(v).ok())
                    .unwrap_or_default();
                let text = if input.trim().is_empty() {
                    format!("[{tool_name}]")
                } else {
                    format!("[{tool_name}]\n{input}")
                };
                out.push((
                    SessionMessage {
                        role: "tool_call".to_string(),
                        text,
                        timestamp: ts,
                        tool_call_id: Some(tool_id.clone()),
                    },
                    location.clone(),
                ));

                let output = call
                    .get("resultDisplay")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .or_else(|| {
                        call.get("result")
                            .and_then(|v| serde_json::to_string_pretty(v).ok())
                    });
                if let Some(output) = output {
                    let prefix = if tool_call_is_error(call, Some(&output)) {
                        "[error]"
                    } else {
                        "[result]"
                    };
                    out.push((
                        SessionMessage {
                            role: "tool_result".to_string(),
                            text: format!("{prefix}\n{output}"),
                            timestamp: ts,
                            tool_call_id: Some(tool_id),
                        },
                        location.clone(),
                    ));
                }
            }
        }

        if let Some(text) = extract_gemini_display_or_message_text(&raw) {
            if !text.trim().is_empty() {
                out.push((
                    SessionMessage {
                        role: "assistant".to_string(),
                        text,
                        timestamp: ts,
                        tool_call_id: None,
                    },
                    location.clone(),
                ));
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
struct JsonArrayEntry {
    start: usize,
    end: usize,
    line_start: u64,
    line_end: u64,
}

fn scan_json_array_entries(bytes: &[u8]) -> Result<Vec<JsonArrayEntry>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let len = bytes.len();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    let mut current_start: Option<usize> = None;
    let mut line = 1u64;
    let mut entry_line_start = 1u64;
    let mut saw_array_open = false;

    while i < len {
        let b = bytes[i];
        if b == b'\n' {
            line += 1;
            if !in_string && depth == 0 {
                i += 1;
                continue;
            }
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'"' => {
                in_string = true;
            }
            b'[' => {
                if depth == 0 {
                    saw_array_open = true;
                }
                depth += 1;
            }
            b']' => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            b'{' => {
                if depth == 1 && current_start.is_none() {
                    current_start = Some(i);
                    entry_line_start = line;
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 1 {
                    if let Some(start) = current_start.take() {
                        out.push(JsonArrayEntry {
                            start,
                            end: i + 1,
                            line_start: entry_line_start,
                            line_end: line,
                        });
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }

    // An empty `[]` is a valid (just uninteresting) Gemini logs file, so
    // only error out when no top-level array opener was ever seen.
    if !saw_array_open && bytes.iter().any(|b| !b.is_ascii_whitespace()) {
        return Err(anyhow::anyhow!("no JSON array found"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{read_messages_with_locations, remove_session_from_logs, scan_json_array_entries};
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
    fn scan_json_array_entries_tracks_object_ranges() {
        let bytes = br#"
[
  {"sessionId":"a","type":"user","message":"hello","timestamp":"2026-05-19T00:00:00Z"},
  {"sessionId":"b","type":"assistant","message":"skip","timestamp":"2026-05-19T00:00:01Z"}
]
"#;
        let entries = scan_json_array_entries(bytes).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(&bytes[entries[0].start..entries[0].end], br#"{"sessionId":"a","type":"user","message":"hello","timestamp":"2026-05-19T00:00:00Z"}"#);
        assert_eq!(&bytes[entries[1].start..entries[1].end], br#"{"sessionId":"b","type":"assistant","message":"skip","timestamp":"2026-05-19T00:00:01Z"}"#);
        assert!(entries[0].line_start >= 1);
        assert!(entries[0].line_end >= entries[0].line_start);
    }

    #[test]
    fn read_messages_with_locations_records_precise_offsets() {
        let dir = unique_tmp("gemini-parser");
        let path = dir.join("logs.json");
        fs::write(
            &path,
            r#"
[
  {"sessionId":"sess-1","type":"user","message":"hello","timestamp":"2026-05-19T00:00:00Z"},
  {"sessionId":"sess-1","type":"assistant","message":"world","timestamp":"2026-05-19T00:00:01Z"}
]
"#,
        )
        .unwrap();
        let messages = read_messages_with_locations(&path, "sess-1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0.text, "hello");
        assert!(messages[0].1.byte_start.is_some());
        assert!(messages[0].1.byte_end.is_some());
        assert!(messages[0].1.byte_end.unwrap() > messages[0].1.byte_start.unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_chat_file_and_read_messages_support_new_session_json() {
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
        let path = chats.join("session-new.json");
        fs::write(
            &path,
            format!(
                r#"{{
  "sessionId": "new",
  "startTime": "2026-05-19T00:00:00Z",
  "lastUpdated": "2026-05-19T00:00:04Z",
  "messages": [
    {{
      "id": "u1",
      "type": "user",
      "timestamp": "2026-05-19T00:00:00Z",
      "displayContent": "look @{image}",
      "content": [
        {{ "text": "look" }},
        {{ "fileData": {{ "fileUri": "file://{image}", "mimeType": "image/png" }} }}
      ]
    }},
    {{
      "id": "a1",
      "type": "model",
      "timestamp": "2026-05-19T00:00:01Z",
      "thoughts": [{{ "subject": "Plan", "description": "inspect" }}],
      "toolCalls": [
        {{
          "id": "tool-1",
          "displayName": "ReadFile",
          "args": {{ "path": "README.md" }},
          "resultDisplay": "contents"
        }}
      ],
      "content": {{ "parts": [{{ "text": "done" }}] }}
    }}
  ]
}}"#,
                image = image_path.to_string_lossy()
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

        let messages = read_messages_with_locations(&path, "new").unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0].0.role, "user");
        assert!(messages[0].0.text.contains("![Image #1]("));
        assert_eq!(messages[1].0.role, "thinking");
        assert_eq!(messages[2].0.role, "tool_call");
        assert_eq!(messages[2].0.tool_call_id.as_deref(), Some("tool-1"));
        assert_eq!(messages[3].0.role, "tool_result");
        assert_eq!(messages[4].0.role, "assistant");
        assert_eq!(messages[4].0.text, "done");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_json_array_entries_accepts_empty_array() {
        let entries = scan_json_array_entries(b"[]\n").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn scan_json_array_entries_rejects_garbage_without_array_opener() {
        assert!(scan_json_array_entries(b"not json").is_err());
    }

    #[test]
    fn remove_session_from_logs_rewrites_source_and_appends_removed_copy() {
        let home = unique_tmp("gemini-home");
        let source = home
            .join(".gemini")
            .join("tmp")
            .join("project")
            .join("logs.json");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            r#"[
  {"sessionId":"keep","type":"user","message":"stay","timestamp":"2026-05-19T00:00:00Z"},
  {"sessionId":"drop","type":"user","message":"gone-1","timestamp":"2026-05-19T00:00:01Z"},
  {"sessionId":"drop","type":"assistant","message":"gone-2","timestamp":"2026-05-19T00:00:02Z"}
]
"#,
        )
        .unwrap();

        let removed_root = home.join(".sessio").join("removed-sessions");
        assert!(remove_session_from_logs(&source, "drop", &home, &removed_root).unwrap());

        let rewritten = fs::read_to_string(&source).unwrap();
        assert!(rewritten.contains(r#""sessionId": "keep""#));
        assert!(!rewritten.contains(r#""sessionId": "drop""#));

        let removed = fs::read_to_string(
            removed_root
                .join(".gemini")
                .join("tmp")
                .join("project")
                .join("logs.json"),
        )
        .unwrap();
        assert!(removed.contains(r#""sessionId": "drop""#));
        assert!(!removed.contains(r#""sessionId": "keep""#));

        assert!(!remove_session_from_logs(&source, "drop", &home, &removed_root).unwrap());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn remove_session_from_logs_appends_to_existing_removed_file() {
        let home = unique_tmp("gemini-home-append");
        let source = home
            .join(".gemini")
            .join("tmp")
            .join("project")
            .join("logs.json");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(
            &source,
            r#"[
  {"sessionId":"one","type":"user","message":"a","timestamp":"2026-05-19T00:00:00Z"},
  {"sessionId":"two","type":"user","message":"b","timestamp":"2026-05-19T00:00:01Z"},
  {"sessionId":"one","type":"assistant","message":"c","timestamp":"2026-05-19T00:00:02Z"}
]
"#,
        )
        .unwrap();

        let removed_path = home
            .join(".sessio")
            .join("removed-sessions")
            .join(".gemini")
            .join("tmp")
            .join("project")
            .join("logs.json");
        fs::create_dir_all(removed_path.parent().unwrap()).unwrap();
        fs::write(
            &removed_path,
            r#"[{"sessionId":"existing","type":"user","message":"old"}]
"#,
        )
        .unwrap();

        let removed_root = home.join(".sessio").join("removed-sessions");
        assert!(remove_session_from_logs(&source, "one", &home, &removed_root).unwrap());

        let removed = fs::read_to_string(&removed_path).unwrap();
        assert!(removed.contains(r#""sessionId": "existing""#));
        assert!(removed.contains(r#""sessionId": "one""#));

        let _ = fs::remove_dir_all(&home);
    }
}

#[derive(Default)]
struct GeminiAgg {
    count: usize,
    earliest: Option<i64>,
    latest: Option<i64>,
    first_user: Option<String>,
}

fn parse_iso(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn read_json_array(path: &Path) -> Result<Vec<serde_json::Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read(path)?;
    let entries = scan_json_array_entries(&text)?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        out.push(serde_json::from_slice(&text[entry.start..entry.end])?);
    }
    Ok(out)
}

fn write_json_array(path: &Path, values: &[serde_json::Value]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(values)?;
    fs::write(path, format!("{text}\n"))?;
    Ok(())
}
