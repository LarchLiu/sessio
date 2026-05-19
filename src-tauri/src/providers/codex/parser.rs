use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::models::{
    is_system_noise, normalize_preview, strip_injected_context, Agent, SessionInfo, SessionMessage,
};
use crate::providers::shared::jsonl_scan;
use crate::providers::system_time_to_millis;
use crate::providers::types::SourceLocation;

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let mut out = Vec::new();
    let (live, archived) = roots()?;
    scan_dir(&live, false, &mut out);
    scan_dir(&archived, true, &mut out);
    Ok(out)
}

pub fn roots() -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok((
        home.join(".codex").join("sessions"),
        home.join(".codex").join("archived_sessions"),
    ))
}

pub fn parse_one_file(path: &Path, archived: bool) -> Result<Option<SessionInfo>> {
    parse_session(path, archived)
}

pub fn path_is_archived(path: &Path, archived_root: &Path) -> bool {
    path.starts_with(archived_root)
}

fn scan_dir(root: &Path, archived: bool, out: &mut Vec<SessionInfo>) {
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
        match parse_session(path, archived) {
            Ok(Some(info)) => out.push(info),
            Ok(None) => {}
            Err(e) => log::warn!("codex parse {} failed: {e}", path.display()),
        }
    }
}

pub fn read_messages(path: &Path) -> Result<Vec<SessionMessage>> {
    Ok(read_messages_with_locations(path)?
        .into_iter()
        .map(|(m, _)| m)
        .collect())
}

// Same as read_messages but also returns the SourceLocation (line + byte
// range) of the JSONL line each message was parsed from. One Codex line
// produces at most one SessionMessage, so line_start == line_end and the
// byte range covers the full line including its trailing newline.
pub fn read_messages_with_locations(path: &Path) -> Result<Vec<(SessionMessage, SourceLocation)>> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut out = Vec::new();
    let file_path = path.to_string_lossy().to_string();
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
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line_str) else {
            continue;
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso);
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let Some(payload) = v.get("payload") else {
            continue;
        };
        let Some(message) = interpret_payload(payload, ts) else {
            continue;
        };
        let location = SourceLocation {
            file_path: file_path.clone(),
            line_start: Some(line_number),
            line_end: Some(line_number),
            byte_start: Some(line_start_byte),
            byte_end: Some(line_end_byte),
        };
        out.push((message, location));
    }
    Ok(out)
}

fn interpret_payload(payload: &serde_json::Value, ts: Option<i64>) -> Option<SessionMessage> {
    let kind = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match kind {
        "message" => {
            let role = payload
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            if role == "developer" {
                return None;
            }
            let text = extract_message_text(payload).unwrap_or_default();
            if text.trim().is_empty() {
                return None;
            }
            if role == "user" && is_system_noise(&text) {
                return None;
            }
            Some(SessionMessage {
                role,
                text,
                timestamp: ts,
            })
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
            Some(SessionMessage {
                role: "tool_call".to_string(),
                text,
                timestamp: ts,
            })
        }
        "function_call_output" => {
            let output = payload.get("output").and_then(|x| x.as_str()).unwrap_or("");
            if output.trim().is_empty() {
                return None;
            }
            Some(SessionMessage {
                role: "tool_result".to_string(),
                text: output.to_string(),
                timestamp: ts,
            })
        }
        _ => None,
    }
}

fn parse_session(path: &Path, archived: bool) -> Result<Option<SessionInfo>> {
    let scan = jsonl_scan::scan(path)?;

    let mut id: Option<String> = None;
    let mut forked_from_id: Option<String> = None;
    let mut started_at: Option<i64> = None;
    let mut cwd: Option<String> = None;
    let mut first_user_message: Option<String> = None;
    let mut latest_ts: Option<i64> = None;

    for line in &scan.head {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso);
        if let Some(t) = ts {
            latest_ts = Some(latest_ts.map_or(t, |e| e.max(t)));
        }
        let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if t == "session_meta" {
            let payload = v.get("payload").cloned().unwrap_or_default();
            if id.is_none() {
                id = payload.get("id").and_then(|x| x.as_str()).map(String::from);
            }
            if forked_from_id.is_none() {
                forked_from_id = payload
                    .get("forked_from_id")
                    .and_then(|x| x.as_str())
                    .map(String::from);
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
        } else if t == "response_item" && first_user_message.is_none() {
            if let Some(payload) = v.get("payload") {
                if payload.get("type").and_then(|x| x.as_str()) == Some("message")
                    && payload.get("role").and_then(|x| x.as_str()) == Some("user")
                {
                    if let Some(text) = extract_message_text(payload) {
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
    }

    for line in &scan.tail {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(t) = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso)
        {
            latest_ts = Some(latest_ts.map_or(t, |e| e.max(t)));
        }
    }

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

    Ok(Some(SessionInfo {
        id: id.unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        agent: Agent::Codex,
        forked_from_id,
        project_path: cwd,
        project_name,
        started_at,
        updated_at,
        message_count: scan.message_count,
        first_user_message,
        file_path: path.to_string_lossy().into_owned(),
        file_size: scan.file_size,
        partial: scan.partial,
        available: true,
        archived,
        subagents: Vec::new(),
    }))
}

fn extract_message_text(payload: &serde_json::Value) -> Option<String> {
    let content = payload.get("content")?.as_array()?;
    let mut parts = Vec::new();
    for item in content {
        let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
        if matches!(kind, "input_text" | "text" | "output_text") {
            if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
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
}
