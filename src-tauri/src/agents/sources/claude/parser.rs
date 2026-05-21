use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::agents::sources::shared::jsonl_scan;
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::SourceLocation;
use crate::models::{
    is_system_noise, normalize_preview, strip_injected_context, Agent, SessionInfo, SessionMessage,
    SubagentInfo,
};

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
            forked_from_id: None,
            project_path: None,
            project_name: None,
            started_at: earliest,
            updated_at: latest,
            message_count: 0,
            title: None,
            first_user_message: None,
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: false,
            archived: true,
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

pub fn read_messages(path: &Path) -> Result<Vec<SessionMessage>> {
    Ok(read_messages_with_locations(path)?
        .into_iter()
        .map(|(m, _)| m)
        .collect())
}

// Same as read_messages but also returns the SourceLocation for each
// SessionMessage. A single Claude JSONL line can expand into multiple
// SessionMessage entries (e.g. an assistant turn with text + tool_use);
// all expansions share the originating line's location so resolve can
// point back to the exact JSONL line.
pub fn read_messages_with_locations(path: &Path) -> Result<Vec<(SessionMessage, SourceLocation)>> {
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
        let t = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if t != "user" && t != "assistant" {
            continue;
        }
        let ts = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso);
        let Some(msg) = v.get("message") else {
            continue;
        };
        let role_raw = msg
            .get("role")
            .and_then(|x| x.as_str())
            .unwrap_or(t)
            .to_string();
        let location = SourceLocation {
            file_path: file_path.clone(),
            line_start: Some(line_number),
            line_end: Some(line_number),
            byte_start: Some(line_start_byte),
            byte_end: Some(line_end_byte),
        };
        let tool_result_meta = v.get("toolUseResult");
        for sm in expand_message(&role_raw, msg, ts, &mut edit_tools, tool_result_meta) {
            out.push((sm, location.clone()));
        }
    }
    Ok(out)
}

// Convert a single Anthropic message into one or more SessionMessage entries:
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
) -> Vec<SessionMessage> {
    let content = match msg.get("content") {
        Some(c) => c,
        None => return Vec::new(),
    };

    if let Some(s) = content.as_str() {
        if s.trim().is_empty() {
            return Vec::new();
        }
        if role_raw == "user" && is_system_noise(s) {
            return Vec::new();
        }
        return vec![SessionMessage {
            role: role_raw.to_string(),
            text: s.to_string(),
            timestamp: ts,
            tool_call_id: None,
        }];
    }

    let arr = match content.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();

    for item in arr {
        let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(t) = item.get("text").and_then(|x| x.as_str()) {
                    text_parts.push(t.to_string());
                }
            }
            "image" => {
                if let Some(markdown) = claude_image_item_to_markdown(item, text_parts.len() + 1) {
                    text_parts.push(markdown);
                }
            }
            "thinking" => {
                if !text_parts.is_empty() {
                    let joined = text_parts.join("\n");
                    if !(joined.trim().is_empty() || role_raw == "user" && is_system_noise(&joined))
                    {
                        out.push(SessionMessage {
                            role: role_raw.to_string(),
                            text: joined,
                            timestamp: ts,
                            tool_call_id: None,
                        });
                    }
                    text_parts.clear();
                }
                if let Some(t) = item.get("thinking").and_then(|x| x.as_str()) {
                    if !t.trim().is_empty() {
                        out.push(SessionMessage {
                            role: "thinking".to_string(),
                            text: t.to_string(),
                            timestamp: ts,
                            tool_call_id: None,
                        });
                    }
                }
            }
            "tool_use" => {
                if !text_parts.is_empty() {
                    let joined = text_parts.join("\n");
                    if !(joined.trim().is_empty() || role_raw == "user" && is_system_noise(&joined))
                    {
                        out.push(SessionMessage {
                            role: role_raw.to_string(),
                            text: joined,
                            timestamp: ts,
                            tool_call_id: None,
                        });
                    }
                    text_parts.clear();
                }
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
                        if let Ok(text) = serde_json::to_string(todos) {
                            out.push(SessionMessage {
                                role: "todo".to_string(),
                                text,
                                timestamp: ts,
                                tool_call_id: item
                                    .get("id")
                                    .and_then(|x| x.as_str())
                                    .map(String::from),
                            });
                            continue;
                        }
                    }
                }
                let input_pretty = item
                    .get("input")
                    .and_then(|i| serde_json::to_string_pretty(i).ok())
                    .unwrap_or_default();
                let text = if input_pretty.trim().is_empty() || input_pretty.trim() == "{}" {
                    format!("[{name}]")
                } else {
                    format!("[{name}]\n{input_pretty}")
                };
                out.push(SessionMessage {
                    role: "tool_call".to_string(),
                    text,
                    timestamp: ts,
                    tool_call_id: item.get("id").and_then(|x| x.as_str()).map(String::from),
                });
            }
            "tool_result" => {
                if !text_parts.is_empty() {
                    let joined = text_parts.join("\n");
                    if !(joined.trim().is_empty() || role_raw == "user" && is_system_noise(&joined))
                    {
                        out.push(SessionMessage {
                            role: role_raw.to_string(),
                            text: joined,
                            timestamp: ts,
                            tool_call_id: None,
                        });
                    }
                    text_parts.clear();
                }
                let body = extract_tool_result_text(item);
                if !body.trim().is_empty() {
                    let tool_use_id = item
                        .get("tool_use_id")
                        .and_then(|x| x.as_str())
                        .map(String::from);
                    out.push(SessionMessage {
                        role: "tool_result".to_string(),
                        text: body,
                        timestamp: ts,
                        tool_call_id: tool_use_id.clone(),
                    });
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

    if !text_parts.is_empty() {
        let joined = text_parts.join("\n");
        if !(joined.trim().is_empty() || role_raw == "user" && is_system_noise(&joined)) {
            out.push(SessionMessage {
                role: role_raw.to_string(),
                text: joined,
                timestamp: ts,
                tool_call_id: None,
            });
        }
    }

    out
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
        for (idx, sub) in arr.iter().enumerate() {
            if sub.get("type").and_then(|x| x.as_str()) == Some("text") {
                if let Some(t) = sub.get("text").and_then(|x| x.as_str()) {
                    parts.push(t.to_string());
                }
            } else if sub.get("type").and_then(|x| x.as_str()) == Some("image") {
                if let Some(markdown) = claude_image_item_to_markdown(sub, idx + 1) {
                    parts.push(markdown);
                }
            }
        }
        return parts.join("\n");
    }
    String::new()
}

fn claude_image_item_to_markdown(item: &serde_json::Value, idx: usize) -> Option<String> {
    let source = item.get("source")?;
    let source_type = source.get("type").and_then(|x| x.as_str()).unwrap_or("");
    match source_type {
        "base64" => {
            let media_type = source
                .get("media_type")
                .and_then(|x| x.as_str())
                .unwrap_or("image/png");
            let data = source.get("data").and_then(|x| x.as_str())?;
            Some(format!("![Image #{idx}](data:{media_type};base64,{data})"))
        }
        "url" => {
            let url = source.get("url").and_then(|x| x.as_str())?;
            Some(format!("![Image #{idx}]({url})"))
        }
        _ => None,
    }
}

fn claude_file_edit_message(
    edit: &ClaudeEditTool,
    ts: Option<i64>,
    tool_result_meta: Option<&serde_json::Value>,
) -> Option<SessionMessage> {
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
    Some(serde_json::json!({
        "path": path,
        "displayPath": path,
        "kind": "edit",
        "additions": additions,
        "deletions": deletions,
        "detail": detail,
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

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count().max(1)
    }
}

fn parse_session(path: &PathBuf) -> Result<Option<SessionInfo>> {
    let scan = jsonl_scan::scan(path)?;

    let mut cwd: Option<String> = None;
    let mut ai_title: Option<String> =
        ai_title_from_lines(scan.head.iter().chain(scan.tail.iter()));
    let mut last_prompt: Option<String> =
        last_prompt_from_lines(scan.head.iter().chain(scan.tail.iter()));
    let mut first_user_message: Option<String> = None;
    let mut earliest_ts: Option<i64> = None;
    let mut latest_ts: Option<i64> = None;

    for line in &scan.head {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if ai_title.is_none() {
            ai_title = ai_title_from_value(&v);
        }
        if last_prompt.is_none() {
            last_prompt = last_prompt_from_value(&v);
        }
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
            latest_ts = Some(latest_ts.map_or(t, |e| e.max(t)));
        }
        let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if kind == "user" && first_user_message.is_none() {
            if let Some(msg) = v.get("message") {
                let text = extract_message_text(msg);
                if !text.trim().is_empty() && !is_system_noise(&text) {
                    let cleaned = strip_injected_context(&text);
                    if !cleaned.is_empty() {
                        first_user_message = Some(normalize_preview(&cleaned));
                    }
                }
            }
        }
    }

    for line in &scan.tail {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if ai_title.is_none() {
            ai_title = ai_title_from_value(&v);
        }
        if last_prompt.is_none() {
            last_prompt = last_prompt_from_value(&v);
        }
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
            latest_ts = Some(latest_ts.map_or(t, |e| e.max(t)));
        }
    }

    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let updated_at = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(system_time_to_millis)
        .or(latest_ts);

    let project_name = cwd.as_ref().and_then(|p| {
        Path::new(p)
            .file_name()
            .and_then(|s| s.to_str())
            .map(String::from)
    });
    let title = ai_title
        .or_else(|| ai_title_from_file(path).ok().flatten())
        .or_else(|| last_prompt_from_file(path).ok().flatten())
        .or(last_prompt)
        .or_else(|| first_user_message.clone());

    Ok(Some(SessionInfo {
        id,
        agent: Agent::Claude,
        forked_from_id: None,
        project_path: cwd,
        project_name,
        started_at: earliest_ts,
        updated_at,
        message_count: scan.message_count,
        title,
        first_user_message,
        file_path: path.to_string_lossy().into_owned(),
        file_size: scan.file_size,
        partial: scan.partial,
        available: true,
        archived: false,
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

fn ai_title_from_lines<'a>(lines: impl Iterator<Item = &'a String>) -> Option<String> {
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(title) = ai_title_from_value(&v) {
            return Some(title);
        }
    }
    None
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

fn ai_title_from_file(path: &Path) -> Result<Option<String>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = line?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(title) = ai_title_from_value(&v) {
            return Ok(Some(title));
        }
    }
    Ok(None)
}

fn last_prompt_from_lines<'a>(lines: impl Iterator<Item = &'a String>) -> Option<String> {
    let mut last_prompt = None;
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(prompt) = last_prompt_from_value(&v) {
            last_prompt = Some(prompt);
        }
    }
    last_prompt
}

fn last_prompt_from_value(v: &serde_json::Value) -> Option<String> {
    if v.get("type").and_then(|x| x.as_str()) != Some("last-prompt") {
        return None;
    }
    v.get("lastPrompt")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_preview)
}

fn last_prompt_from_file(path: &Path) -> Result<Option<String>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut last_prompt = None;
    for line in reader.lines() {
        let line = line?;
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(prompt) = last_prompt_from_value(&v) {
            last_prompt = Some(prompt);
        }
    }
    Ok(last_prompt)
}

fn parse_iso(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
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
    let scan = jsonl_scan::scan(path)?;
    let mut first_user_message: Option<String> = None;
    let mut earliest_ts: Option<i64> = None;
    let mut latest_ts: Option<i64> = None;

    for line in &scan.head {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(t) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso)
        {
            earliest_ts = Some(earliest_ts.map_or(t, |e| e.min(t)));
            latest_ts = Some(latest_ts.map_or(t, |e| e.max(t)));
        }
        let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if kind == "user" && first_user_message.is_none() {
            if let Some(msg) = v.get("message") {
                let text = extract_message_text(msg);
                if !text.trim().is_empty() && !is_system_noise(&text) {
                    let cleaned = strip_injected_context(&text);
                    if !cleaned.is_empty() {
                        first_user_message = Some(normalize_preview(&cleaned));
                    }
                }
            }
        }
    }
    for line in &scan.tail {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(t) = v
            .get("timestamp")
            .and_then(|x| x.as_str())
            .and_then(parse_iso)
        {
            latest_ts = Some(latest_ts.map_or(t, |e| e.max(t)));
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

    let updated_at = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(system_time_to_millis)
        .or(latest_ts);

    Ok(Some(SubagentInfo {
        id,
        agent_type: meta.agent_type,
        description: meta.description,
        started_at: earliest_ts,
        updated_at,
        message_count: scan.message_count,
        first_user_message,
        file_path: path.to_string_lossy().into_owned(),
        file_size: scan.file_size,
        partial: scan.partial,
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
        .filter(|s| !s.trim().is_empty() && !is_system_noise(s))
        .map(strip_injected_context)
        .filter(|s| !s.is_empty())
        .map(|s| normalize_preview(&s));
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
        forked_from_id: None,
        project_path: cwd,
        project_name,
        started_at,
        updated_at,
        message_count: entry.message_count.unwrap_or(0),
        title: preview.clone(),
        first_user_message: preview,
        file_path,
        file_size,
        partial: true,
        available,
        archived: !available,
        subagents,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        let out = expand_message(
            "assistant",
            &msg,
            Some(1_700_000_000_000),
            &mut edits,
            None,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, "todo");
        assert_eq!(out[0].timestamp, Some(1_700_000_000_000));
        assert_eq!(out[0].tool_call_id.as_deref(), Some("toolu_todos"));
        assert!(out[0].text.contains("Verify parser todo rendering"));
        assert!(out[0].text.contains("\"status\":\"completed\""));
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

        fs::remove_file(last_prompt_path).ok();
        fs::remove_file(ai_title_path).ok();
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
        assert_eq!(tool_events[0].role, "tool_call");

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
        assert_eq!(result_events[0].role, "tool_result");
        assert_eq!(result_events[1].role, "file_edit");
        let value: serde_json::Value = serde_json::from_str(&result_events[1].text).unwrap();
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
        assert!(
            edit.get("detail")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .starts_with("@@ -10,2 +10,3 @@")
        );
    }
}
