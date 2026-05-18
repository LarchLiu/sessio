use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::models::{
    is_system_noise, normalize_preview, strip_injected_context, Agent, SessionInfo, SessionMessage,
};
use crate::providers::shared::jsonl_scan;
use crate::providers::system_time_to_millis;

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
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = v
            .get("timestamp")
            .and_then(|t| t.as_str())
            .and_then(parse_iso);
        if v.get("type").and_then(|t| t.as_str()) != Some("response_item") {
            continue;
        }
        let payload = match v.get("payload") {
            Some(p) => p,
            None => continue,
        };
        let kind = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "message" => {
                let role = payload
                    .get("role")
                    .and_then(|r| r.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = extract_message_text(payload).unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                if role == "user" && is_system_noise(&text) {
                    continue;
                }
                out.push(SessionMessage {
                    role,
                    text,
                    timestamp: ts,
                });
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
                out.push(SessionMessage {
                    role: "tool_call".to_string(),
                    text,
                    timestamp: ts,
                });
            }
            "function_call_output" => {
                let output = payload.get("output").and_then(|x| x.as_str()).unwrap_or("");
                if output.trim().is_empty() {
                    continue;
                }
                out.push(SessionMessage {
                    role: "tool_result".to_string(),
                    text: output.to_string(),
                    timestamp: ts,
                });
            }
            _ => continue,
        }
    }
    Ok(out)
}

fn parse_session(path: &Path, archived: bool) -> Result<Option<SessionInfo>> {
    let scan = jsonl_scan::scan(path)?;

    let mut id: Option<String> = None;
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
    use super::parse_one_file;
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
}
