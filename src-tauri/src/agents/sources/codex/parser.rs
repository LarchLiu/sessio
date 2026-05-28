use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::agents::sources::shared::attachment_text::{
    parse_xmlish_attrs, percent_decode, sanitize_user_attachment_text, sanitize_user_preview_text,
};
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::SourceLocation;
use crate::models::{
    is_system_noise, normalize_preview, strip_injected_context, Agent, SessionInfo, SessionMessage,
    SubagentInfo,
};

const REVERSE_TIMESTAMP_CHUNK_SIZE: u64 = 16 * 1024;

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let mut parsed = Vec::new();
    let (live, archived) = roots()?;
    let titles = load_session_index_titles();
    scan_dir(&live, false, &titles, &mut parsed);
    scan_dir(&archived, true, &titles, &mut parsed);
    Ok(group_codex_sessions(parsed))
}

pub fn roots() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok((
        home.join(".codex").join("sessions"),
        home.join(".codex").join("archived_sessions"),
    ))
}

pub fn parse_one_file(path: &Path, archived: bool) -> Result<Option<SessionInfo>> {
    let titles = load_session_index_titles();
    Ok(parse_session(path, archived, &titles)?.map(|p| p.info))
}

pub fn parse_one_file_with_relation(
    path: &Path,
    archived: bool,
) -> Result<Option<CodexParsedSession>> {
    let titles = load_session_index_titles();
    parse_session(path, archived, &titles)
}

pub fn parse_one_subagent_file(path: &Path, archived: bool) -> Result<Option<CodexParsedSubagent>> {
    let Some(parsed) = parse_one_file_with_relation(path, archived)? else {
        return Ok(None);
    };
    let Some(parent_thread_id) = parsed.parent_thread_id.clone() else {
        return Ok(None);
    };
    Ok(Some(CodexParsedSubagent {
        parent_thread_id,
        info: parsed.into_subagent(),
    }))
}

pub fn find_session_file_by_id(session_id: &str) -> Result<Option<(std::path::PathBuf, bool)>> {
    let (live, archived) = roots()?;
    let titles = load_session_index_titles();
    for (root, is_archived) in [(live.as_path(), false), (archived.as_path(), true)] {
        if !root.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            match parse_session(path, is_archived, &titles) {
                Ok(Some(parsed))
                    if parsed.parent_thread_id.is_none() && parsed.info.id == session_id =>
                {
                    return Ok(Some((path.to_path_buf(), is_archived)));
                }
                Ok(_) => {}
                Err(e) => log::warn!("codex parse {} failed: {e}", path.display()),
            }
        }
    }
    Ok(None)
}

pub fn path_is_archived(path: &Path, archived_root: &Path) -> bool {
    path.starts_with(archived_root)
}

fn scan_dir(
    root: &Path,
    archived: bool,
    titles: &HashMap<String, String>,
    out: &mut Vec<CodexParsedSession>,
) {
    if !root.exists() {
        return;
    }
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        match parse_session(path, archived, titles) {
            Ok(Some(info)) => out.push(info),
            Ok(None) => {}
            Err(e) => log::warn!("codex parse {} failed: {e}", path.display()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CodexParsedSession {
    pub info: SessionInfo,
    pub parent_thread_id: Option<String>,
    pub agent_nickname: Option<String>,
    pub agent_role: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodexParsedSubagent {
    pub parent_thread_id: String,
    pub info: SubagentInfo,
}

impl CodexParsedSession {
    pub fn into_subagent(self) -> SubagentInfo {
        SubagentInfo {
            id: self.info.id,
            agent_type: self.agent_role,
            description: self.agent_nickname,
            started_at: self.info.started_at,
            updated_at: self.info.updated_at,
            message_count: self.info.message_count,
            first_user_message: self.info.first_user_message,
            file_path: self.info.file_path,
            file_size: self.info.file_size,
            partial: self.info.partial,
            available: self.info.available,
        }
    }
}

fn group_codex_sessions(parsed: Vec<CodexParsedSession>) -> Vec<SessionInfo> {
    let mut top_level = Vec::new();
    let mut subagents_by_parent: HashMap<String, Vec<SubagentInfo>> = HashMap::new();
    for item in parsed {
        if let Some(parent_thread_id) = item.parent_thread_id.clone() {
            subagents_by_parent
                .entry(parent_thread_id)
                .or_default()
                .push(item.into_subagent());
        } else {
            top_level.push(item.info);
        }
    }
    for session in top_level.iter_mut() {
        if let Some(mut subagents) = subagents_by_parent.remove(&session.id) {
            subagents.sort_by_key(|s| s.started_at);
            session.subagents = subagents;
        }
    }
    top_level
}

pub fn read_messages(path: &Path) -> Result<Vec<SessionMessage>> {
    Ok(read_messages_with_locations(path)?
        .into_iter()
        .map(|(m, _)| m)
        .collect())
}

// Same as read_messages but also returns the SourceLocation (line + byte
// range) of the JSONL line each message was parsed from. Some Codex lines can
// expand into multiple SessionMessage entries (e.g. image_generation_call as
// tool call + result), all sharing the originating line location.
pub fn read_messages_with_locations(path: &Path) -> Result<Vec<(SessionMessage, SourceLocation)>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut out = Vec::new();
    let file_path = path.to_string_lossy().to_string();
    let mut buf = Vec::new();
    let mut byte_offset: u64 = 0;
    let mut line_number: u64 = 0;
    let mut cwd: Option<String> = None;

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
        update_codex_cwd(&v, &mut cwd);
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso);
        let messages = interpret_record(&v, ts, cwd.as_deref());
        if messages.is_empty() {
            continue;
        }
        let location = SourceLocation {
            file_path: file_path.clone(),
            line_start: Some(line_number),
            line_end: Some(line_number),
            byte_start: Some(line_start_byte),
            byte_end: Some(line_end_byte),
        };
        for message in messages {
            out.push((message, location.clone()));
        }
    }
    Ok(out)
}

fn interpret_record(
    v: &serde_json::Value,
    ts: Option<i64>,
    cwd: Option<&str>,
) -> Vec<SessionMessage> {
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "response_item" => match v.get("payload") {
            Some(payload) => interpret_payload(payload, ts),
            None => Vec::new(),
        },
        "event_msg" => match v.get("payload") {
            Some(payload) => interpret_event_payload(payload, ts, cwd),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn update_codex_cwd(v: &serde_json::Value, cwd: &mut Option<String>) {
    let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if t != "session_meta" && t != "turn_context" {
        return;
    }
    if let Some(c) = v
        .get("payload")
        .and_then(|p| p.get("cwd"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        *cwd = Some(c.to_string());
    }
}

fn interpret_event_payload(
    payload: &serde_json::Value,
    ts: Option<i64>,
    cwd: Option<&str>,
) -> Vec<SessionMessage> {
    match payload.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "patch_apply_end" => {
            if payload.get("success").and_then(|x| x.as_bool()) != Some(true) {
                return Vec::new();
            }
            let Some(changes) = payload.get("changes").and_then(|x| x.as_object()) else {
                return Vec::new();
            };
            let edits: Vec<serde_json::Value> = changes
                .iter()
                .map(|(path, change)| {
                    let kind = change
                        .get("type")
                        .and_then(|x| x.as_str())
                        .unwrap_or("update");
                    let summary = codex_change_summary(kind, path, change);
                    serde_json::json!({
                        "path": path,
                        "displayPath": display_path(path, cwd),
                        "kind": kind,
                        "additions": summary.additions,
                        "deletions": summary.deletions,
                        "detail": summary.detail,
                        "patch": summary.patch,
                        "oldContent": summary.old_content,
                        "newContent": summary.new_content,
                    })
                })
                .collect();
            file_edit_message("codex", edits, ts).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn interpret_payload(payload: &serde_json::Value, ts: Option<i64>) -> Vec<SessionMessage> {
    let kind = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "message" => {
            let role = payload
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            if role == "developer" {
                return Vec::new();
            }
            let raw = extract_message_text(payload).unwrap_or_default();
            let text = if role == "user" {
                sanitize_user_attachment_text(&raw)
            } else {
                raw
            };
            if text.trim().is_empty() {
                return Vec::new();
            }
            if role == "user" && is_system_noise(&text) {
                return Vec::new();
            }
            vec![SessionMessage {
                role,
                text,
                timestamp: ts,
                tool_call_id: None,
            }]
        }
        "function_call" => {
            let name = payload
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("tool");
            let args_raw = payload
                .get("arguments")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let args_pretty = serde_json::from_str::<serde_json::Value>(args_raw)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_else(|| args_raw.to_string());
            let text = if args_pretty.trim().is_empty() {
                format!("[{name}]")
            } else {
                format!("[{name}]\n{args_pretty}")
            };
            vec![SessionMessage {
                role: "tool_call".to_string(),
                text,
                timestamp: ts,
                tool_call_id: payload
                    .get("call_id")
                    .and_then(|x| x.as_str())
                    .map(String::from),
            }]
        }
        "function_call_output" => {
            let output = extract_function_call_output_text(payload);
            if output.trim().is_empty() {
                return Vec::new();
            }
            vec![SessionMessage {
                role: "tool_result".to_string(),
                text: output,
                timestamp: ts,
                tool_call_id: payload
                    .get("call_id")
                    .and_then(|x| x.as_str())
                    .map(String::from),
            }]
        }
        "custom_tool_call" => {
            let name = payload
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("tool");
            let input = payload
                .get("input")
                .and_then(|x| x.as_str())
                .map(String::from)
                .or_else(|| {
                    payload
                        .get("arguments")
                        .and_then(|x| x.as_str())
                        .map(String::from)
                })
                .or_else(|| {
                    payload
                        .get("input")
                        .and_then(|x| serde_json::to_string_pretty(x).ok())
                })
                .unwrap_or_default();
            let text = if input.trim().is_empty() {
                format!("[{name}]")
            } else {
                format!("[{name}]\n{input}")
            };
            vec![SessionMessage {
                role: "tool_call".to_string(),
                text,
                timestamp: ts,
                tool_call_id: payload
                    .get("call_id")
                    .or_else(|| payload.get("id"))
                    .and_then(|x| x.as_str())
                    .map(String::from),
            }]
        }
        "custom_tool_call_output" => {
            let output = payload
                .get("output")
                .map(extract_tool_output_text)
                .unwrap_or_default();
            if output.trim().is_empty() {
                return Vec::new();
            }
            vec![SessionMessage {
                role: "tool_result".to_string(),
                text: output,
                timestamp: ts,
                tool_call_id: payload
                    .get("call_id")
                    .or_else(|| payload.get("id"))
                    .and_then(|x| x.as_str())
                    .map(String::from),
            }]
        }
        "reasoning" => {
            let text = extract_reasoning_text(payload);
            if text.trim().is_empty() {
                return Vec::new();
            }
            vec![SessionMessage {
                role: "thinking".to_string(),
                text,
                timestamp: ts,
                tool_call_id: None,
            }]
        }
        "web_search_call" => interpret_web_search_call(payload, ts),
        "image_generation_call" => interpret_image_generation_call(payload, ts),
        _ => Vec::new(),
    }
}

fn parse_session(
    path: &Path,
    archived: bool,
    session_index_titles: &HashMap<String, String>,
) -> Result<Option<CodexParsedSession>> {
    let mut id: Option<String> = None;
    let mut forked_from_agent: Option<Agent> = None;
    let mut forked_from_id: Option<String> = None;
    let mut started_at: Option<i64> = None;
    let mut cwd: Option<String> = None;
    let mut parent_thread_id: Option<String> = None;
    let mut agent_nickname: Option<String> = None;
    let mut agent_role: Option<String> = None;
    let mut first_user_message: Option<String> = None;
    let mut internal_guardian_session = false;
    let mut skip_replayed_user_message = false;
    let reverse_ts = latest_timestamp_from_file(path).ok().flatten();

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
        let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if t == "session_meta" {
            let payload = v.get("payload").cloned().unwrap_or_default();
            if is_codex_guardian_session(&payload) {
                internal_guardian_session = true;
            }
            if id.is_none() {
                id = payload.get("id").and_then(|x| x.as_str()).map(String::from);
            }
            if forked_from_id.is_none() {
                forked_from_id = payload
                    .get("forked_from_id")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                if forked_from_id.is_some() {
                    forked_from_agent = Some(Agent::Codex);
                }
            }
            if started_at.is_none() {
                started_at = payload
                    .get("timestamp")
                    .and_then(|x| x.as_str())
                    .and_then(parse_iso);
            }
            if cwd.is_none() {
                cwd = payload
                    .get("cwd")
                    .and_then(|x| x.as_str())
                    .map(String::from);
            }
            if parent_thread_id.is_none() {
                parent_thread_id = codex_parent_thread_id(&payload);
            }
            if agent_nickname.is_none() {
                agent_nickname = payload
                    .get("agent_nickname")
                    .and_then(|x| x.as_str())
                    .or_else(|| {
                        payload
                            .get("source")
                            .and_then(|x| x.get("subagent"))
                            .and_then(|x| x.get("thread_spawn"))
                            .and_then(|x| x.get("agent_nickname"))
                            .and_then(|x| x.as_str())
                    })
                    .map(String::from);
            }
            if agent_role.is_none() {
                agent_role = payload
                    .get("agent_role")
                    .and_then(|x| x.as_str())
                    .or_else(|| {
                        payload
                            .get("source")
                            .and_then(|x| x.get("subagent"))
                            .and_then(|x| x.get("thread_spawn"))
                            .and_then(|x| x.get("agent_role"))
                            .and_then(|x| x.as_str())
                    })
                    .map(String::from);
            }
        } else if t == "compacted" {
            skip_replayed_user_message = true;
        } else if t == "event_msg" && first_user_message.is_none() {
            if let Some(payload) = v.get("payload") {
                if payload.get("type").and_then(|x| x.as_str()) == Some("user_message") {
                    if let Some(text) = payload.get("message").and_then(|x| x.as_str()) {
                        if forked_from_id.is_none() {
                            if let Some(lineage) = cross_context_lineage_from_text(text) {
                                forked_from_agent = Some(lineage.agent);
                                forked_from_id = Some(lineage.session_id);
                            }
                        }
                        if !is_system_noise(text) {
                            let cleaned = strip_injected_context(&sanitize_user_preview_text(text));
                            if !cleaned.is_empty() {
                                first_user_message = Some(normalize_preview(&cleaned));
                            }
                        }
                    }
                }
            }
        } else if t == "response_item" && first_user_message.is_none() {
            if let Some(payload) = v.get("payload") {
                if payload.get("type").and_then(|x| x.as_str()) == Some("message")
                    && payload.get("role").and_then(|x| x.as_str()) == Some("user")
                {
                    if skip_replayed_user_message {
                        skip_replayed_user_message = false;
                        continue;
                    }
                    if forked_from_id.is_none() {
                        if let Some(lineage) = cross_context_lineage_from_payload(payload) {
                            forked_from_agent = Some(lineage.agent);
                            forked_from_id = Some(lineage.session_id);
                        }
                    }
                    if let Some(text) = extract_message_preview_text(payload) {
                        if !is_system_noise(&text) {
                            let cleaned = strip_injected_context(&text);
                            if !cleaned.is_empty() {
                                first_user_message = Some(normalize_preview(&cleaned));
                            }
                        }
                    }
                }
            }
        }
        let current_id = id.as_deref();
        let has_index_title = current_id
            .and_then(|session_id| session_index_titles.get(session_id))
            .is_some();
        if id.is_some()
            && started_at.is_some()
            && cwd.is_some()
            && (has_index_title || first_user_message.is_some())
        {
            break;
        }
    }

    if internal_guardian_session {
        return Ok(None);
    }

    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let updated_at = reverse_ts.or_else(|| {
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

    let id = id.unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    let title = session_index_titles
        .get(&id)
        .cloned()
        .or_else(|| first_user_message.clone());

    let info = SessionInfo {
        id,
        agent: Agent::Codex,
        forked_from_agent,
        forked_from_id,
        project_path: cwd,
        project_name,
        started_at,
        updated_at,
        message_count: 0,
        title,
        first_user_message,
        file_path: path.to_string_lossy().into_owned(),
        file_size,
        partial: false,
        available: true,
        archived,
        subagents: Vec::new(),
    };
    Ok(Some(CodexParsedSession {
        info,
        parent_thread_id,
        agent_nickname,
        agent_role,
    }))
}

fn codex_parent_thread_id(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("source")
        .and_then(|x| x.get("subagent"))
        .and_then(|x| x.get("thread_spawn"))
        .and_then(|x| x.get("parent_thread_id"))
        .and_then(|x| x.as_str())
        .map(String::from)
}

struct CrossContextLineage {
    agent: Agent,
    session_id: String,
}

fn cross_context_lineage_from_text(text: &str) -> Option<CrossContextLineage> {
    for path in cross_context_paths_from_text(text) {
        if let Some(lineage) = cross_context_lineage_from_file(&path) {
            return Some(lineage);
        }
    }
    parse_cross_context_lineage(text)
}

fn cross_context_lineage_from_payload(payload: &serde_json::Value) -> Option<CrossContextLineage> {
    for text in raw_text_values(payload) {
        if let Some(lineage) = cross_context_lineage_from_text(&text) {
            return Some(lineage);
        }
    }
    None
}

fn raw_text_values(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_raw_text_values(value, &mut out);
    out
}

fn collect_raw_text_values(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_raw_text_values(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["text", "message", "content", "value"] {
                if let Some(child) = map.get(key) {
                    collect_raw_text_values(child, out);
                }
            }
        }
        _ => {}
    }
}

fn cross_context_paths_from_text(text: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for marker in ["file://", "uri=\"file://", "ref=\"file://"] {
        let mut rest = text;
        while let Some(start) = rest.find(marker) {
            let after = &rest[start + marker.len()..];
            let end = after
                .find(|ch: char| ch == '"' || ch == ')' || ch == '<' || ch.is_whitespace())
                .unwrap_or(after.len());
            let raw = &after[..end];
            if raw.contains("sessio-cross-context") {
                paths.push(PathBuf::from(percent_decode(raw)));
            }
            rest = &after[end..];
        }
    }
    paths
}

fn cross_context_lineage_from_file(path: &Path) -> Option<CrossContextLineage> {
    let text = fs::read_to_string(path).ok()?;
    parse_cross_context_lineage(&text)
}

fn parse_cross_context_lineage(text: &str) -> Option<CrossContextLineage> {
    let start = text.find("<!-- sessio-cross:start")?;
    let after_start = &text[start..];
    let end = after_start.find("-->")?;
    let attrs_start = "<!-- sessio-cross:start".len();
    let attrs = parse_xmlish_attrs(&after_start[attrs_start..end]);
    let agent = attrs
        .get("source_agent")
        .and_then(|value| Agent::from_db_str(value))?;
    let session_id = attrs.get("source_session_id")?.trim();
    if session_id.is_empty() {
        return None;
    }
    Some(CrossContextLineage {
        agent,
        session_id: session_id.to_string(),
    })
}

fn is_codex_guardian_session(payload: &serde_json::Value) -> bool {
    payload
        .get("source")
        .and_then(|x| x.get("subagent"))
        .and_then(|x| x.get("other"))
        .and_then(|x| x.as_str())
        == Some("guardian")
}

fn load_session_index_titles() -> HashMap<String, String> {
    let Some(home) = dirs::home_dir() else {
        return HashMap::new();
    };
    let path = home.join(".codex").join("session_index.jsonl");
    let Ok(file) = File::open(&path) else {
        return HashMap::new();
    };
    let reader = BufReader::new(file);
    let mut titles = HashMap::new();
    for line in reader.lines().map_while(|line| line.ok()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else {
            continue;
        };
        let Some(thread_name) = v.get("thread_name").and_then(|x| x.as_str()) else {
            continue;
        };
        if id.is_empty() || thread_name.trim().is_empty() {
            continue;
        }
        let cleaned = strip_injected_context(&sanitize_user_preview_text(thread_name));
        if cleaned.is_empty() {
            continue;
        }
        titles.insert(id.to_string(), normalize_preview(&cleaned));
    }
    titles
}

fn latest_timestamp_from_file(path: &Path) -> Result<Option<i64>> {
    let mut file = File::open(path)?;
    let mut offset = file.metadata()?.len();
    let mut carry = Vec::new();
    while offset > 0 {
        let read_size = REVERSE_TIMESTAMP_CHUNK_SIZE.min(offset);
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
            if let Some(t) = v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .and_then(parse_iso)
            {
                return Ok(Some(t));
            }
        }
    }
    Ok(None)
}

fn extract_message_text(payload: &serde_json::Value) -> Option<String> {
    extract_message_text_inner(payload)
}

fn extract_message_preview_text(payload: &serde_json::Value) -> Option<String> {
    if let Some(text) = payload.get("content").and_then(|x| x.as_str()) {
        let cleaned = sanitize_user_preview_text(text);
        if !cleaned.trim().is_empty() {
            return Some(cleaned);
        }
    }
    let mut parts = Vec::new();
    if let Some(content) = payload.get("content").and_then(|x| x.as_array()) {
        for item in content {
            let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if matches!(kind, "input_text" | "text" | "output_text") {
                if let Some(text) = item
                    .get("text")
                    .or_else(|| item.get("value"))
                    .or_else(|| item.get("content"))
                    .and_then(|x| x.as_str())
                {
                    let cleaned = sanitize_user_preview_text(text);
                    if !cleaned.trim().is_empty() {
                        parts.push(cleaned);
                    }
                }
            }
        }
    }
    if !parts.is_empty() {
        return Some(parts.join("\n"));
    }
    for key in ["text", "message"] {
        if let Some(text) = payload.get(key).and_then(|x| x.as_str()) {
            let cleaned = sanitize_user_preview_text(text);
            if !cleaned.trim().is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn extract_message_text_inner(payload: &serde_json::Value) -> Option<String> {
    if let Some(text) = payload.get("content").and_then(|x| x.as_str()) {
        if !text.trim().is_empty() {
            return Some(text.to_string());
        }
    }
    let mut parts = Vec::new();
    if let Some(content) = payload.get("content").and_then(|x| x.as_array()) {
        for item in content {
            let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if matches!(kind, "input_text" | "text" | "output_text") {
                if let Some(text) = item
                    .get("text")
                    .or_else(|| item.get("value"))
                    .or_else(|| item.get("content"))
                    .and_then(|x| x.as_str())
                {
                    if !text.trim().is_empty() {
                        parts.push(text.to_string());
                    }
                }
            } else if matches!(kind, "input_image" | "image_url") {
                if let Some(markdown) = image_item_to_markdown(item, parts.len() + 1) {
                    parts.push(markdown);
                }
            }
        }
    }
    if !parts.is_empty() {
        return Some(parts.join("\n"));
    }
    for key in ["text", "message"] {
        if let Some(text) = payload.get(key).and_then(|x| x.as_str()) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

fn extract_function_call_output_text(payload: &serde_json::Value) -> String {
    let Some(output) = payload.get("output") else {
        return String::new();
    };
    extract_tool_output_text(output)
}

fn extract_tool_output_text(output: &serde_json::Value) -> String {
    if let Some(s) = output.as_str() {
        return s.to_string();
    }
    if let Some(arr) = output.as_array() {
        let mut parts = Vec::new();
        for (idx, item) in arr.iter().enumerate() {
            let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if matches!(kind, "text" | "input_text" | "output_text") {
                if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                    parts.push(t.to_string());
                }
            } else if matches!(kind, "input_image" | "image_url") {
                if let Some(markdown) = image_item_to_markdown(item, idx + 1) {
                    parts.push(markdown);
                }
            } else if let Some(t) = item
                .get("text")
                .or_else(|| item.get("value"))
                .or_else(|| item.get("content"))
                .and_then(|x| x.as_str())
            {
                parts.push(t.to_string());
            } else {
                let nested = extract_tool_output_text(item);
                if !nested.trim().is_empty() {
                    parts.push(nested);
                }
            }
        }
        return parts.join("\n");
    }
    if let Some(obj) = output.as_object() {
        let mut parts = Vec::new();
        for key in ["content", "message", "text", "output", "result", "data"] {
            if let Some(next) = obj.get(key) {
                let nested = extract_tool_output_text(next);
                if !nested.trim().is_empty() {
                    parts.push(nested);
                }
            }
        }
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    serde_json::to_string_pretty(output).unwrap_or_default()
}

fn extract_reasoning_text(payload: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(arr) = payload.get("summary").and_then(|x| x.as_array()) {
        for item in arr {
            if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                parts.push(text.to_string());
            } else if let Some(text) = item.as_str() {
                parts.push(text.to_string());
            }
        }
    }
    if let Some(arr) = payload.get("content").and_then(|x| x.as_array()) {
        for item in arr {
            if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                parts.push(text.to_string());
            }
        }
    }
    parts.join("\n")
}

fn interpret_web_search_call(payload: &serde_json::Value, ts: Option<i64>) -> Vec<SessionMessage> {
    let Some(action) = payload.get("action") else {
        return Vec::new();
    };
    let args_pretty = match serde_json::to_string_pretty(action) {
        Ok(s) if !s.trim().is_empty() && s != "null" => s,
        _ => return Vec::new(),
    };
    vec![SessionMessage {
        role: "tool_call".to_string(),
        text: format!("[web_search]\n{args_pretty}"),
        timestamp: ts,
        tool_call_id: payload
            .get("id")
            .or_else(|| payload.get("call_id"))
            .and_then(|x| x.as_str())
            .map(String::from),
    }]
}

fn interpret_image_generation_call(
    payload: &serde_json::Value,
    ts: Option<i64>,
) -> Vec<SessionMessage> {
    let call_id = payload.get("id").and_then(|x| x.as_str()).map(String::from);
    let mut args = serde_json::Map::new();
    if let Some(status) = payload.get("status").and_then(|x| x.as_str()) {
        args.insert(
            "status".to_string(),
            serde_json::Value::String(status.to_string()),
        );
    }
    if let Some(phase) = payload.get("phase").and_then(|x| x.as_str()) {
        args.insert(
            "phase".to_string(),
            serde_json::Value::String(phase.to_string()),
        );
    }
    if let Some(prompt) = payload
        .get("revised_prompt")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.insert(
            "revised_prompt".to_string(),
            serde_json::Value::String(prompt.to_string()),
        );
    }
    let args_pretty = serde_json::to_string_pretty(&serde_json::Value::Object(args))
        .unwrap_or_else(|_| "{}".to_string());
    let mut out = vec![SessionMessage {
        role: "tool_call".to_string(),
        text: format!("[image_generation]\n{args_pretty}"),
        timestamp: ts,
        tool_call_id: call_id.clone(),
    }];
    if let Some(result) = payload
        .get("result")
        .and_then(image_generation_result_to_markdown)
    {
        out.push(SessionMessage {
            role: "tool_result".to_string(),
            text: result,
            timestamp: ts,
            tool_call_id: call_id,
        });
    }
    out
}

fn image_generation_result_to_markdown(result: &serde_json::Value) -> Option<String> {
    if let Some(s) = result.as_str().map(str::trim).filter(|s| !s.is_empty()) {
        let src = if looks_like_image_src(s) {
            s.to_string()
        } else {
            format!("data:image/png;base64,{s}")
        };
        return Some(format!("![Generated Image]({src})"));
    }
    if let Some(arr) = result.as_array() {
        let images: Vec<String> = arr
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                item.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| {
                        let src = if looks_like_image_src(s) {
                            s.to_string()
                        } else {
                            format!("data:image/png;base64,{s}")
                        };
                        format!("![Generated Image #{}]({src})", idx + 1)
                    })
            })
            .collect();
        if !images.is_empty() {
            return Some(images.join("\n"));
        }
    }
    None
}

fn looks_like_image_src(s: &str) -> bool {
    s.starts_with("data:")
        || s.starts_with("http://")
        || s.starts_with("https://")
        || s.starts_with("asset:")
        || s.starts_with("blob:")
        || s.starts_with('/')
}

fn file_edit_message(
    source: &str,
    edits: Vec<serde_json::Value>,
    ts: Option<i64>,
) -> Option<SessionMessage> {
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
    let text = serde_json::json!({
        "source": source,
        "files": edits.len(),
        "additions": additions,
        "deletions": deletions,
        "edits": edits,
    })
    .to_string();
    Some(SessionMessage {
        role: "file_edit".to_string(),
        text,
        timestamp: ts,
        tool_call_id: None,
    })
}

fn display_path(path: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd else {
        return path.to_string();
    };
    let prefix = if cwd.ends_with('/') {
        cwd.to_string()
    } else {
        format!("{cwd}/")
    };
    path.strip_prefix(&prefix).unwrap_or(path).to_string()
}

struct CodexChangeSummary {
    additions: usize,
    deletions: usize,
    detail: String,
    patch: Option<String>,
    old_content: Option<String>,
    new_content: Option<String>,
}

fn codex_change_summary(kind: &str, path: &str, change: &serde_json::Value) -> CodexChangeSummary {
    if let Some(diff) = change.get("unified_diff").and_then(|x| x.as_str()) {
        let (additions, deletions) = unified_diff_counts(diff);
        return CodexChangeSummary {
            additions,
            deletions,
            detail: diff.to_string(),
            patch: Some(unified_diff_to_file_patch(path, diff)),
            old_content: None,
            new_content: None,
        };
    }

    let content = change.get("content").and_then(|x| x.as_str()).unwrap_or("");
    let line_count = line_count(content);
    match kind {
        "add" => CodexChangeSummary {
            additions: line_count,
            deletions: 0,
            detail: format!("+++ added content\n{content}"),
            patch: None,
            old_content: None,
            new_content: Some(content.to_string()),
        },
        "delete" => CodexChangeSummary {
            additions: 0,
            deletions: line_count,
            detail: format!("--- deleted content\n{content}"),
            patch: None,
            old_content: Some(content.to_string()),
            new_content: None,
        },
        _ => CodexChangeSummary {
            additions: 0,
            deletions: 0,
            detail: content.to_string(),
            patch: None,
            old_content: None,
            new_content: None,
        },
    }
}

fn unified_diff_to_file_patch(path: &str, diff: &str) -> String {
    let normalized = path.trim_start_matches('/');
    format!(
        "diff --git a/{normalized} b/{normalized}\n--- a/{normalized}\n+++ b/{normalized}\n{diff}"
    )
}

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count().max(1)
    }
}

fn unified_diff_counts(diff: &str) -> (usize, usize) {
    let mut additions = 0;
    let mut deletions = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    (additions, deletions)
}

fn image_item_to_markdown(item: &serde_json::Value, idx: usize) -> Option<String> {
    if let Some(data) = item
        .get("image")
        .or_else(|| item.get("data"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let media_type = item
            .get("media_type")
            .or_else(|| item.get("mime_type"))
            .and_then(|x| x.as_str())
            .unwrap_or("image/png");
        let src = if looks_like_image_src(data) {
            data.to_string()
        } else {
            format!("data:{media_type};base64,{data}")
        };
        return Some(format!("![Image #{idx}]({src})"));
    }
    let url = item
        .get("image_url")
        .and_then(|x| x.as_str())
        .or_else(|| item.get("url").and_then(|x| x.as_str()))
        .or_else(|| item.get("path").and_then(|x| x.as_str()))?;
    Some(format!("![Image #{idx}]({url})"))
}

fn parse_iso(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::{parse_one_file, read_messages_with_locations};
    use std::fs;

    #[test]
    fn first_session_meta_id_wins_over_replayed_session_meta() {
        let dir =
            std::env::temp_dir().join(format!("sessio-codex-parser-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("renamed-session-file.jsonl");
        fs::write(
            &path,
            r#"{"timestamp":"2026-05-18T05:09:14.748Z","type":"session_meta","payload":{"id":"019e397d-032a-72f3-ab4d-5e69683a02ae","forked_from_id":"019e364b-640b-7de0-aaba-9b1a1f5e6b87","timestamp":"2026-05-18T05:09:14.666Z","cwd":"/tmp/new"}}
{"timestamp":"2026-05-17T14:16:11.000Z","type":"session_meta","payload":{"id":"019e364b-640b-7de0-aaba-9b1a1f5e6b87","timestamp":"2026-05-17T14:16:11.000Z","cwd":"/tmp/old"}}
{"timestamp":"2026-05-18T05:09:15.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}
"#,
        )
        .unwrap();

        let info = parse_one_file(&path, false).unwrap().unwrap();
        assert_eq!(info.id, "019e397d-032a-72f3-ab4d-5e69683a02ae");
        assert_eq!(info.project_path.as_deref(), Some("/tmp/new"));

        fs::remove_file(path).ok();
        fs::remove_dir(dir).ok();
    }

    #[test]
    fn read_messages_with_locations_records_line_and_byte_range() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-location-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        // Line 1: skipped (session_meta is not a response_item)
        // Line 2: produces a user message
        // Line 3: produces an assistant message
        let line1 = r#"{"timestamp":"2026-05-18T05:09:14.748Z","type":"session_meta","payload":{"id":"abc","timestamp":"2026-05-18T05:09:14.666Z"}}"#;
        let line2 = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#;
        let line3 = r#"{"timestamp":"2026-05-18T05:09:16.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi back"}]}}"#;
        let body = format!("{line1}\n{line2}\n{line3}\n");
        fs::write(&path, &body).unwrap();

        let line1_bytes = line1.len() as u64 + 1; // include the \n
        let line2_bytes = line2.len() as u64 + 1;

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 2, "session_meta line must not yield messages");

        let (user_msg, user_loc) = &events[0];
        assert_eq!(user_msg.role, "user");
        assert_eq!(user_msg.text, "hello");
        assert_eq!(user_loc.line_start, Some(2));
        assert_eq!(user_loc.line_end, Some(2));
        assert_eq!(user_loc.byte_start, Some(line1_bytes));
        assert_eq!(
            user_loc.byte_end,
            Some(line1_bytes + line2_bytes),
            "byte_end should include trailing \\n"
        );

        let (asst_msg, asst_loc) = &events[1];
        assert_eq!(asst_msg.role, "assistant");
        assert_eq!(asst_loc.line_start, Some(3));
        assert_eq!(asst_loc.line_end, Some(3));

        // Verify the byte range actually slices back to the original JSONL line.
        let slice = &body.as_bytes()
            [user_loc.byte_start.unwrap() as usize..user_loc.byte_end.unwrap() as usize];
        let slice_str = std::str::from_utf8(slice).unwrap();
        assert!(slice_str.starts_with(line2));
        assert!(slice_str.ends_with('\n'));

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_with_locations_filters_developer_messages() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-developer-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let developer = r#"{"timestamp":"2026-05-18T05:09:14.900Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"internal instruction"}]}}"#;
        let user = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"real request"}]}}"#;
        fs::write(&path, format!("{developer}\n{user}\n")).unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.role, "user");
        assert_eq!(events[0].0.text, "real request");

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_keeps_reasoning_and_images_as_displayable_markdown() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-rich-message-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let user = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"look\n<image name=[Image #1]>\n</image>"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]}}"#;
        let reasoning = r#"{"timestamp":"2026-05-18T05:09:16.000Z","type":"response_item","payload":{"type":"reasoning","summary":[{"text":"checking the screenshot"}],"content":null}}"#;
        let tool_output = r#"{"timestamp":"2026-05-18T05:09:17.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call_1","output":[{"type":"input_image","image_url":"data:image/png;base64,def","detail":"original"}]}}"#;
        fs::write(&path, format!("{user}\n{reasoning}\n{tool_output}\n")).unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].0.role, "user");
        assert!(!events[0].0.text.contains("<image"));
        assert!(!events[0].0.text.contains("</image>"));
        assert!(events[0]
            .0
            .text
            .contains("![Image #2](data:image/png;base64,abc)"));
        assert_eq!(events[1].0.role, "thinking");
        assert_eq!(events[1].0.text, "checking the screenshot");
        assert_eq!(events[2].0.role, "tool_result");
        assert_eq!(events[2].0.tool_call_id.as_deref(), Some("call_1"));
        assert!(events[2]
            .0
            .text
            .contains("![Image #1](data:image/png;base64,def)"));

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_keeps_image_generation_results() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-image-generation-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let image_generation = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"response_item","payload":{"type":"image_generation_call","id":"ig_1","status":"completed","revised_prompt":"draw a small icon","result":"abc123"}}"#;
        fs::write(&path, format!("{image_generation}\n")).unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0.role, "tool_call");
        assert_eq!(events[0].0.tool_call_id.as_deref(), Some("ig_1"));
        assert!(events[0].0.text.contains("[image_generation]"));
        assert!(events[0].0.text.contains("draw a small icon"));
        assert_eq!(events[1].0.role, "tool_result");
        assert_eq!(events[1].0.tool_call_id.as_deref(), Some("ig_1"));
        assert!(events[1]
            .0
            .text
            .contains("![Generated Image](data:image/png;base64,abc123)"));
        assert_eq!(events[0].1.line_start, events[1].1.line_start);

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_keeps_web_search_actions() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-web-search-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let search_call = r#"{"timestamp":"2026-05-23T01:56:19.628Z","type":"response_item","payload":{"type":"web_search_call","status":"completed","action":{"type":"search","queries":["Bun runtime Rust rewrite news May 2026","Bun rewrite in Rust announcement"]}}}"#;
        let search_end = r#"{"timestamp":"2026-05-23T01:56:19.628Z","type":"event_msg","payload":{"type":"web_search_end","call_id":"ws_1","query":"Bun runtime Rust rewrite news May 2026 ...","action":{"type":"search","queries":["Bun runtime Rust rewrite news May 2026","Bun rewrite in Rust announcement"]}}}"#;
        let open_page_call = r#"{"timestamp":"2026-05-23T01:56:33.727Z","type":"response_item","payload":{"type":"web_search_call","status":"completed","action":{"type":"open_page","url":"https://github.com/oven-sh/bun/pull/30412"}}}"#;
        let open_page_end = r#"{"timestamp":"2026-05-23T01:56:33.727Z","type":"event_msg","payload":{"type":"web_search_end","call_id":"ws_2","query":"https://github.com/oven-sh/bun/pull/30412","action":{"type":"open_page","url":"https://github.com/oven-sh/bun/pull/30412"}}}"#;
        fs::write(
            &path,
            format!("{search_call}\n{search_end}\n{open_page_call}\n{open_page_end}\n"),
        )
        .unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0.role, "tool_call");
        assert!(events[0].0.tool_call_id.is_none());
        assert!(events[0].0.text.contains("[web_search]"));
        assert!(events[0]
            .0
            .text
            .contains("Bun runtime Rust rewrite news May 2026"));
        assert!(!events[0].0.text.contains("completed"));
        assert_eq!(events[1].0.role, "tool_call");
        assert!(events[1]
            .0
            .text
            .contains("https://github.com/oven-sh/bun/pull/30412"));

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_keeps_custom_tool_calls() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-custom-tool-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let call = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"response_item","payload":{"type":"custom_tool_call","call_id":"call_custom","name":"apply_patch","input":"*** Begin Patch\n*** End Patch"}}"#;
        let output = r#"{"timestamp":"2026-05-18T05:09:16.000Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call_custom","output":{"content":[{"text":"Done"}]}}}"#;
        fs::write(&path, format!("{call}\n{output}\n")).unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0.role, "tool_call");
        assert_eq!(events[0].0.tool_call_id.as_deref(), Some("call_custom"));
        assert!(events[0].0.text.contains("[apply_patch]"));
        assert_eq!(events[1].0.role, "tool_result");
        assert_eq!(events[1].0.tool_call_id.as_deref(), Some("call_custom"));
        assert_eq!(events[1].0.text, "Done");

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_keeps_patch_apply_edit_summary() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-file-edit-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let meta = r#"{"timestamp":"2026-05-18T05:09:14.000Z","type":"session_meta","payload":{"id":"abc","timestamp":"2026-05-18T05:09:14.000Z","cwd":"/tmp/project"}}"#;
        let patch = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"event_msg","payload":{"type":"patch_apply_end","success":true,"call_id":"call_1","changes":{"/tmp/project/src/app.rs":{"type":"update","unified_diff":"@@ -1,2 +1,3 @@\n use a;\n-old\n+new\n+more\n","move_path":null}}}}"#;
        fs::write(&path, format!("{meta}\n{patch}\n")).unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0.role, "file_edit");
        let value: serde_json::Value = serde_json::from_str(&events[0].0.text).unwrap();
        assert_eq!(value.get("files").and_then(|x| x.as_u64()), Some(1));
        assert_eq!(value.get("additions").and_then(|x| x.as_u64()), Some(2));
        assert_eq!(value.get("deletions").and_then(|x| x.as_u64()), Some(1));
        assert_eq!(
            value
                .get("edits")
                .and_then(|x| x.as_array())
                .and_then(|a| a.first())
                .and_then(|e| e.get("displayPath"))
                .and_then(|x| x.as_str()),
            Some("src/app.rs")
        );

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn read_messages_counts_content_only_add_and_delete_changes() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-file-rewrite-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let meta = r#"{"timestamp":"2026-05-18T05:09:14.000Z","type":"session_meta","payload":{"id":"abc","timestamp":"2026-05-18T05:09:14.000Z","cwd":"/tmp/project"}}"#;
        let delete = r#"{"timestamp":"2026-05-18T05:09:15.000Z","type":"event_msg","payload":{"type":"patch_apply_end","success":true,"call_id":"call_1","changes":{"/tmp/project/src/app.rs":{"type":"delete","content":"old\nfile\n"}}}}"#;
        let add = r#"{"timestamp":"2026-05-18T05:09:16.000Z","type":"event_msg","payload":{"type":"patch_apply_end","success":true,"call_id":"call_2","changes":{"/tmp/project/src/app.rs":{"type":"add","content":"new\nfile\nmore\n"}}}}"#;
        fs::write(&path, format!("{meta}\n{delete}\n{add}\n")).unwrap();

        let events = read_messages_with_locations(&path).unwrap();
        assert_eq!(events.len(), 2);

        let deleted: serde_json::Value = serde_json::from_str(&events[0].0.text).unwrap();
        assert_eq!(deleted.get("additions").and_then(|x| x.as_u64()), Some(0));
        assert_eq!(deleted.get("deletions").and_then(|x| x.as_u64()), Some(2));
        assert!(deleted
            .get("edits")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|e| e.get("detail"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .starts_with("--- deleted content\nold"));

        let added: serde_json::Value = serde_json::from_str(&events[1].0.text).unwrap();
        assert_eq!(added.get("additions").and_then(|x| x.as_u64()), Some(3));
        assert_eq!(added.get("deletions").and_then(|x| x.as_u64()), Some(0));
        assert!(added
            .get("edits")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|e| e.get("detail"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .starts_with("+++ added content\nnew"));

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn parse_session_title_omits_uploaded_image_and_file_context() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-upload-title-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let meta = r#"{"timestamp":"2026-05-25T07:05:45.000Z","type":"session_meta","payload":{"id":"upload-thread","timestamp":"2026-05-25T07:05:45.000Z","cwd":"/tmp/project"}}"#;
        let user = r##"{"timestamp":"2026-05-25T07:05:46.089Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"图片和游戏策划可以结合吗"},{"type":"input_text","text":"<image>"},{"type":"input_image","image_url":"data:image/png;base64,abc","detail":"high"},{"type":"input_text","text":"</image>"},{"type":"input_text","text":"[@dotAGE 游戏复刻思路研究.md](file:///Users/alex/Documents/dotAGE%20游戏复刻思路研究.md)\n<context ref=\"file:///Users/alex/Documents/dotAGE%20游戏复刻思路研究.md\">\n# **dotAGE 深度技术解构与复刻可行性研究报告**\nvery long file body\n</context>"}]}}"##;
        fs::write(&path, format!("{meta}\n{user}\n")).unwrap();

        let info = parse_one_file(&path, false).unwrap().unwrap();
        assert_eq!(
            info.first_user_message.as_deref(),
            Some("图片和游戏策划可以结合吗")
        );
        let preview = info.first_user_message.as_deref().unwrap();
        assert!(!preview.contains("base64"));
        assert!(!preview.contains("very long file body"));
        assert!(!preview.contains("file://"));
        assert!(!preview.contains("dotAGE"));
        assert!(!preview.contains("[image]"));
        assert!(!preview.contains("[file:"));

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn parse_session_ignores_replayed_user_message_after_compaction() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-compaction-title-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        let meta = r#"{"timestamp":"2026-05-24T07:08:10.000Z","type":"session_meta","payload":{"id":"compacted-thread","timestamp":"2026-05-24T07:08:10.000Z","cwd":"/tmp/project"}}"#;
        let compacted = r#"{"timestamp":"2026-05-24T07:30:00.000Z","type":"compacted","payload":{"message":"summary","replacement_history":[{"type":"message","role":"user"}]}}"#;
        let replayed_user = r#"{"timestamp":"2026-05-24T07:30:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"replayed user message from compacted history"}]}}"#;
        let real_user = r#"{"timestamp":"2026-05-24T07:31:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"继续检查 session 列表","images":[],"local_images":[],"text_elements":[]}}"#;
        fs::write(
            &path,
            format!("{meta}\n{compacted}\n{replayed_user}\n{real_user}\n"),
        )
        .unwrap();

        let info = parse_one_file(&path, false).unwrap().unwrap();
        assert_eq!(
            info.first_user_message.as_deref(),
            Some("继续检查 session 列表")
        );
        assert_eq!(info.title.as_deref(), Some("继续检查 session 列表"));

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn parse_session_reads_cross_context_lineage_when_session_meta_has_no_fork() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-cross-lineage-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let context_path = dir.join("sessio-cross-context-parent-123.md");
        fs::write(
            &context_path,
            r#"<!-- sessio-cross:start source_agent="claude" source_session_id="parent-session" source_file_path="/tmp/parent.jsonl" -->

# Continued session from agent
hello

<!-- sessio-cross:end -->"#,
        )
        .unwrap();
        let path = dir.join("session.jsonl");
        let meta = r#"{"timestamp":"2026-05-24T07:08:10.000Z","type":"session_meta","payload":{"id":"child-session","timestamp":"2026-05-24T07:08:10.000Z","cwd":"/tmp/project"}}"#;
        let user = format!(
            r#"{{"timestamp":"2026-05-24T07:31:00.000Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"现在怎么样"}},{{"type":"input_text","text":"[@sessio-cross-context-parent-123.md](file://{})\n<context ref=\"file://{}\"></context>"}}]}}}}"#,
            context_path.display(),
            context_path.display()
        );
        fs::write(&path, format!("{meta}\n{user}\n")).unwrap();

        let info = parse_one_file(&path, false).unwrap().unwrap();
        assert_eq!(info.forked_from_agent, Some(crate::models::Agent::Claude));
        assert_eq!(info.forked_from_id.as_deref(), Some("parent-session"));

        fs::remove_file(&path).ok();
        fs::remove_file(&context_path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn parse_session_hides_internal_guardian_sessions() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-guardian-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("guardian.jsonl");
        let meta = r#"{"timestamp":"2026-05-24T11:43:37.000Z","type":"session_meta","payload":{"id":"guardian-thread","timestamp":"2026-05-24T11:43:37.000Z","cwd":"/tmp/project","thread_source":"subagent","source":{"subagent":{"other":"guardian"}}}}"#;
        let user = r#"{"timestamp":"2026-05-24T11:43:38.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"internal approval transcript"}]}}"#;
        fs::write(&path, format!("{meta}\n{user}\n")).unwrap();

        let parsed = parse_one_file(&path, false).unwrap();
        assert!(parsed.is_none());

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn codex_subagent_metadata_parses_from_thread_spawn() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-codex-parser-subagent-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("subagent.jsonl");
        let meta = r#"{"timestamp":"2026-05-20T16:31:20.060Z","type":"session_meta","payload":{"id":"child-thread","timestamp":"2026-05-20T16:31:19.702Z","cwd":"/tmp/project","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-thread","depth":1,"agent_nickname":"Beauvoir","agent_role":"worker"}}},"thread_source":"subagent","agent_nickname":"Beauvoir","agent_role":"worker"}}"#;
        let user = r#"{"timestamp":"2026-05-20T16:31:21.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"do the side quest"}]}}"#;
        fs::write(&path, format!("{meta}\n{user}\n")).unwrap();

        let parsed = super::parse_one_file_with_relation(&path, false)
            .unwrap()
            .unwrap();
        assert_eq!(parsed.info.id, "child-thread");
        assert_eq!(parsed.parent_thread_id.as_deref(), Some("parent-thread"));
        assert_eq!(parsed.agent_nickname.as_deref(), Some("Beauvoir"));
        assert_eq!(parsed.agent_role.as_deref(), Some("worker"));

        let sub = super::parse_one_subagent_file(&path, false)
            .unwrap()
            .unwrap();
        assert_eq!(sub.parent_thread_id, "parent-thread");
        assert_eq!(sub.info.id, "child-thread");
        assert_eq!(sub.info.agent_type.as_deref(), Some("worker"));
        assert_eq!(sub.info.description.as_deref(), Some("Beauvoir"));
        assert_eq!(
            sub.info.first_user_message.as_deref(),
            Some("do the side quest")
        );

        fs::remove_file(&path).ok();
        fs::remove_dir(&dir).ok();
    }

    #[test]
    fn codex_grouping_attaches_subagents_and_hides_child_sessions() {
        let parent = super::CodexParsedSession {
            info: crate::models::SessionInfo {
                id: "parent".to_string(),
                agent: crate::models::Agent::Codex,
                forked_from_agent: None,
                forked_from_id: None,
                project_path: Some("/tmp/project".to_string()),
                project_name: Some("project".to_string()),
                started_at: Some(1),
                updated_at: Some(3),
                message_count: 1,
                title: None,
                first_user_message: Some("main".to_string()),
                file_path: "/tmp/parent.jsonl".to_string(),
                file_size: 1,
                partial: false,
                available: true,
                archived: false,
                subagents: Vec::new(),
            },
            parent_thread_id: None,
            agent_nickname: None,
            agent_role: None,
        };
        let child = super::CodexParsedSession {
            info: crate::models::SessionInfo {
                id: "child".to_string(),
                agent: crate::models::Agent::Codex,
                forked_from_agent: None,
                forked_from_id: None,
                project_path: Some("/tmp/project".to_string()),
                project_name: Some("project".to_string()),
                started_at: Some(2),
                updated_at: Some(4),
                message_count: 1,
                title: None,
                first_user_message: Some("side".to_string()),
                file_path: "/tmp/child.jsonl".to_string(),
                file_size: 1,
                partial: false,
                available: true,
                archived: false,
                subagents: Vec::new(),
            },
            parent_thread_id: Some("parent".to_string()),
            agent_nickname: Some("Ada".to_string()),
            agent_role: Some("worker".to_string()),
        };

        let grouped = super::group_codex_sessions(vec![child, parent]);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].id, "parent");
        assert_eq!(grouped[0].subagents.len(), 1);
        assert_eq!(grouped[0].subagents[0].id, "child");
        assert_eq!(
            grouped[0].subagents[0].agent_type.as_deref(),
            Some("worker")
        );
    }
}
