use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::agents::sources::system_time_to_millis;
use crate::models::{is_system_noise, normalize_preview, Agent, SessionInfo, SessionMessage};

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let (tmp_dir, projects_json) = paths()?;
    let mappings = load_project_mappings(&projects_json).unwrap_or_default();

    let mut out = Vec::new();
    if !tmp_dir.exists() {
        return Ok(out);
    }
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
        let role = item
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or("user")
            .to_string();
        let text = item
            .get("message")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
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

fn resolve_project_path(dir_name: &str, mappings: &ProjectMappings) -> Option<String> {
    if dir_name.len() == 64 && dir_name.chars().all(|c| c.is_ascii_hexdigit()) {
        if let Some(p) = mappings.hash_to_path.get(dir_name) {
            return Some(p.clone());
        }
        return None;
    }
    mappings.name_to_path.get(dir_name).cloned()
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
        let role = item
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let text = item
            .get("message")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
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
            agg.first_user = Some(normalize_preview(&text));
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
