use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::models::{is_system_noise, normalize_preview, Agent, SessionInfo, SessionMessage};
use crate::providers::system_time_to_millis;

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
    let text = fs::read_to_string(path)?;
    let arr: Vec<serde_json::Value> = serde_json::from_str(&text)?;
    let mut out = Vec::new();
    for item in arr {
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
        out.push(SessionMessage {
            role,
            text,
            timestamp: ts,
        });
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
    let text = fs::read_to_string(path)?;
    let arr: Vec<serde_json::Value> = match serde_json::from_str(&text) {
        Ok(v) => v,
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
    for item in arr {
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
        out.push(SessionInfo {
            id: sid,
            agent: Agent::Gemini,
            project_path: project_path.map(String::from),
            project_name: project_name.clone(),
            started_at: agg.earliest,
            updated_at: agg.latest.or(file_mtime),
            message_count: agg.count,
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
