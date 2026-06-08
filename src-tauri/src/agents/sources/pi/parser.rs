use anyhow::{Context, Result};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::agents::runtime::types::AcpProtocolMessage;
use crate::agents::sources::shared::convert::project_key_for_path_or_name;
use crate::agents::sources::system_time_to_millis;
use crate::agents::sources::types::{HistoryAcpMessage, SourceLocation};
use crate::models::{normalize_preview, Agent, SessionInfo};
use crate::turns::session_history_turns_from_acp_messages;

pub fn sessions_root() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".sessio").join("astra-pi-agent").join("sessions"))
}

pub fn root_dir() -> Result<Option<PathBuf>> {
    let root = sessions_root()?;
    if root.exists() {
        Ok(Some(root))
    } else {
        Ok(None)
    }
}

pub fn project_dir_for_workspace_path(workspace_path: Option<&str>) -> Result<PathBuf> {
    Ok(sessions_root()?.join(project_dir_name_for_workspace_path(workspace_path)))
}

pub fn project_dir_name_for_workspace_path(workspace_path: Option<&str>) -> String {
    let workspace_path = workspace_path
        .map(str::trim)
        .filter(|value| !value.is_empty());
    project_key_for_path_or_name(workspace_path, None)
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
    let session_id = session_id_from_path(path).unwrap_or_else(|| {
        path.file_stem()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let messages = read_history_acp_messages_with_locations(path, &session_id)?;
    if messages.is_empty() {
        return Ok(None);
    }
    let turns = session_history_turns_from_acp_messages(&messages);
    let first_user_message = turns.iter().find_map(turn_first_user_preview);
    let started_at = messages
        .iter()
        .filter_map(|message| message.timestamp)
        .min();
    let updated_at = messages
        .iter()
        .filter_map(|message| message.timestamp)
        .max()
        .or_else(|| {
            fs::metadata(path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_to_millis)
        });
    let file_size = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let project_path = messages
        .iter()
        .find_map(|message| project_path_from_message(&message.message));
    let project_name = project_path
        .as_deref()
        .and_then(|project| {
            Path::new(project)
                .file_name()
                .and_then(|value| value.to_str())
                .map(String::from)
        })
        .or_else(|| project_dir_name_from_path(path));

    Ok(Some(SessionInfo {
        id: session_id,
        agent: Agent::AstraPi,
        forked_from_agent: None,
        forked_from_id: None,
        project_path,
        project_name,
        started_at,
        updated_at,
        message_count: messages.len(),
        rename_title: None,
        title: first_user_message.clone(),
        first_user_message,
        file_path: path.to_string_lossy().into_owned(),
        file_size,
        partial: false,
        available: true,
        archived: false,
        subagents: Vec::new(),
    }))
}

pub fn read_history_acp_messages_with_locations(
    path: &Path,
    session_id: &str,
) -> Result<Vec<HistoryAcpMessage>> {
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
        let Ok(message) = serde_json::from_str::<AcpProtocolMessage>(line_str) else {
            continue;
        };
        let timestamp = history_timestamp(&message);
        out.push(HistoryAcpMessage {
            message,
            timestamp,
            location: SourceLocation {
                file_path: file_path.clone(),
                line_start: Some(line_number),
                line_end: Some(line_number),
                byte_start: Some(line_start_byte),
                byte_end: Some(line_end_byte),
            },
            synthetic: false,
        });
    }

    if out.is_empty() {
        log::debug!(
            "[pi-parser] no protocol messages parsed from {} for session {}",
            path.display(),
            session_id
        );
    }

    Ok(out)
}

fn history_timestamp(message: &AcpProtocolMessage) -> Option<i64> {
    sessio_timestamp_value(&message.data)
        .or_else(|| iso_timestamp_value(&message.data))
        .or_else(|| nested_iso_timestamp_value(&message.data, &["meta", "timestamp"]))
        .or_else(|| nested_iso_timestamp_value(&message.data, &["_meta", "timestamp"]))
        .or_else(|| nested_iso_timestamp_value(&message.data, &["update", "meta", "timestamp"]))
        .or_else(|| nested_iso_timestamp_value(&message.data, &["update", "_meta", "timestamp"]))
}

fn sessio_timestamp_value(value: &serde_json::Value) -> Option<i64> {
    value
        .get("_meta")
        .and_then(|value| value.get("sessio"))
        .and_then(|value| value.get("timestamp"))
        .and_then(|value| {
            value.as_i64().or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.parse::<i64>().ok().or_else(|| parse_iso(text)))
            })
        })
}

fn iso_timestamp_value(value: &serde_json::Value) -> Option<i64> {
    value.as_str().and_then(parse_iso)
}

fn nested_iso_timestamp_value(value: &serde_json::Value, path: &[&str]) -> Option<i64> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    iso_timestamp_value(current)
}

fn parse_iso(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp_millis())
}

fn turn_first_user_preview(turn: &crate::models::SessionHistoryTurn) -> Option<String> {
    turn.blocks.iter().find_map(|block| {
        if block.kind != "user" {
            return None;
        }
        let text = block
            .blocks
            .iter()
            .filter_map(|part| {
                (part.kind == "text")
                    .then(|| part.text.as_deref())
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if text.is_empty() {
            None
        } else {
            Some(normalize_preview(&text))
        }
    })
}

fn project_path_from_message(message: &AcpProtocolMessage) -> Option<String> {
    let sessio_workspace = value_string(&message.data, &["_meta", "sessio", "workspacePath"])
        .or_else(|| value_string(&message.data, &["_meta", "sessio", "workspace_path"]));
    if sessio_workspace.is_some() {
        return sessio_workspace;
    }
    match message.method.as_str() {
        "session/new" | "session/load" | "session/resume" | "session/fork" => {
            value_string(&message.data, &["workspacePath"])
                .or_else(|| value_string(&message.data, &["workspace_path"]))
        }
        _ => None,
    }
}

fn project_dir_name_from_path(path: &Path) -> Option<String> {
    let root = sessions_root().ok()?;
    let parent = path.parent()?;
    if parent == root {
        return None;
    }
    parent
        .strip_prefix(root)
        .ok()?
        .components()
        .next()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
}

fn value_string(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    current.as_str().map(ToString::to_string)
}

fn session_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();
    decode_session_file_name(&stem)
}

pub fn session_file_name(session_id: &str) -> String {
    let mut out = String::from("pi-");
    for byte in session_id.as_bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push_str(".jsonl");
    out
}

fn decode_session_file_name(stem: &str) -> Option<String> {
    let hex = stem.strip_prefix("pi-")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.as_bytes().chunks_exact(2);
    for pair in &mut chars {
        let text = std::str::from_utf8(pair).ok()?;
        bytes.push(u8::from_str_radix(text, 16).ok()?);
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::{
        decode_session_file_name, parse_session_file, read_history_acp_messages_with_locations,
        session_file_name,
    };
    use std::fs;

    #[test]
    fn session_file_name_roundtrip() {
        let session_id = "pi-session:abc/123";
        let file_name = session_file_name(session_id);
        let stem = file_name.strip_suffix(".jsonl").unwrap();
        assert_eq!(decode_session_file_name(stem).as_deref(), Some(session_id));
    }

    #[test]
    fn project_dir_name_uses_memory_project_key() {
        assert_eq!(
            super::project_dir_name_for_workspace_path(Some("/Users/alex/Work/cloudgeek/sessio")),
            "-Users-alex-Work-cloudgeek-sessio"
        );
        assert_eq!(super::project_dir_name_for_workspace_path(None), "unknown");
    }

    #[test]
    fn parse_session_file_reads_persisted_acp_transcript() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-pi-parser-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let session_id = "pi-session-1";
        let path = dir.join(session_file_name(session_id));
        let contents = concat!(
            "{\"direction\":\"agent_to_client\",\"messageKind\":\"response\",\"method\":\"session/new\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-1\",\"turnId\":null,\"requestId\":null,\"updateType\":null,\"data\":{\"sessionId\":\"pi-session-1\",\"workspacePath\":\"/tmp/demo\",\"meta\":{\"timestamp\":\"2026-06-08T10:00:00Z\"}}}\n",
            "{\"direction\":\"client_to_agent\",\"messageKind\":\"request\",\"method\":\"session/prompt\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-1\",\"turnId\":\"turn-1\",\"requestId\":null,\"updateType\":null,\"data\":{\"prompt\":[{\"type\":\"text\",\"text\":\"hello from pi\"}],\"meta\":{\"timestamp\":\"2026-06-08T10:00:01Z\"}}}\n",
            "{\"direction\":\"agent_to_client\",\"messageKind\":\"notification\",\"method\":\"session/update\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-1\",\"turnId\":\"turn-1\",\"requestId\":null,\"updateType\":\"agent_message_chunk\",\"data\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}],\"meta\":{\"timestamp\":\"2026-06-08T10:00:02Z\"}}}}\n",
            "{\"direction\":\"agent_to_client\",\"messageKind\":\"response\",\"method\":\"session/prompt\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-1\",\"turnId\":\"turn-1\",\"requestId\":null,\"updateType\":null,\"data\":{\"stopReason\":\"end_turn\",\"meta\":{\"timestamp\":\"2026-06-08T10:00:03Z\"}}}\n"
        );
        fs::write(&path, contents).unwrap();

        let messages = read_history_acp_messages_with_locations(&path, session_id).unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].timestamp, Some(1_780_912_800_000));
        assert_eq!(messages[1].location.line_start, Some(2));

        let session = parse_session_file(&path).unwrap().unwrap();
        assert_eq!(session.id, session_id);
        assert_eq!(session.agent, crate::models::Agent::AstraPi);
        assert_eq!(session.project_path.as_deref(), Some("/tmp/demo"));
        assert_eq!(session.first_user_message.as_deref(), Some("hello from pi"));
        assert_eq!(session.title.as_deref(), Some("hello from pi"));
        assert_eq!(session.message_count, 4);
        assert_eq!(session.started_at, Some(1_780_912_800_000));
        assert_eq!(session.updated_at, Some(1_780_912_803_000));
        assert!(!session.partial);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_session_file_reads_sessio_persistence_metadata() {
        let dir = std::env::temp_dir().join(format!(
            "sessio-pi-parser-meta-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let session_id = "pi-session-meta";
        let path = dir.join(session_file_name(session_id));
        let contents = concat!(
            "{\"direction\":\"agent_to_client\",\"messageKind\":\"response\",\"method\":\"session/new\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-meta\",\"turnId\":null,\"requestId\":null,\"updateType\":null,\"data\":{\"sessionId\":\"pi-session-meta\",\"_meta\":{\"sessio\":{\"timestamp\":1780912800000,\"workspacePath\":\"/tmp/meta-demo\"}}}}\n",
            "{\"direction\":\"client_to_agent\",\"messageKind\":\"request\",\"method\":\"session/prompt\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-meta\",\"turnId\":\"turn-1\",\"requestId\":null,\"updateType\":null,\"data\":{\"prompt\":[{\"type\":\"text\",\"text\":\"hello via metadata\"}],\"_meta\":{\"sessio\":{\"timestamp\":1780912801000,\"workspacePath\":\"/tmp/meta-demo\"}}}}\n",
            "{\"direction\":\"agent_to_client\",\"messageKind\":\"response\",\"method\":\"session/prompt\",\"protocolVersion\":\"1\",\"acpSessionId\":\"pi-session-meta\",\"turnId\":\"turn-1\",\"requestId\":null,\"updateType\":null,\"data\":{\"stopReason\":\"end_turn\",\"_meta\":{\"sessio\":{\"timestamp\":1780912803000,\"workspacePath\":\"/tmp/meta-demo\"}}}}\n"
        );
        fs::write(&path, contents).unwrap();

        let messages = read_history_acp_messages_with_locations(&path, session_id).unwrap();
        assert_eq!(messages[0].timestamp, Some(1_780_912_800_000));
        assert_eq!(messages[1].timestamp, Some(1_780_912_801_000));

        let session = parse_session_file(&path).unwrap().unwrap();
        assert_eq!(session.project_path.as_deref(), Some("/tmp/meta-demo"));
        assert_eq!(session.project_name.as_deref(), Some("meta-demo"));
        assert_eq!(
            session.first_user_message.as_deref(),
            Some("hello via metadata")
        );
        assert_eq!(session.started_at, Some(1_780_912_800_000));
        assert_eq!(session.updated_at, Some(1_780_912_803_000));

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
    }
}
