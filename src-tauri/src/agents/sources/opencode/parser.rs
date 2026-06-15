use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::agents::sources::types::{HistoryAcpMessage, SourceLocation};
use crate::models::{normalize_preview, Agent, SessionInfo};
use crate::turns::{
    history_assistant_message, history_thought_message, history_tool_call_message_with_kind,
    history_tool_result_message, history_user_message,
};

const SQLITE_PREFIX: &str = "sqlite:";

pub fn base_dir() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os("XDG_DATA_HOME") {
        let custom = PathBuf::from(custom);
        if !custom.as_os_str().is_empty() {
            return Ok(custom.join("opencode"));
        }
    }
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".local").join("share").join("opencode"))
}

pub fn base_dir_if_exists() -> Result<Option<PathBuf>> {
    let base = base_dir()?;
    Ok(if base.exists() { Some(base) } else { None })
}

pub fn db_path() -> Result<PathBuf> {
    Ok(base_dir()?.join("opencode.db"))
}

/// Stable scope id for the merged OpenCode source. Sessio treats the OpenCode
/// store as one logical scope (the base directory) because dedupe runs across
/// all SQLite rows.
pub fn scope_id(base: &Path) -> String {
    base.to_string_lossy().to_string()
}

/// Virtual `file_path` for a SQLite-backed session. We anchor on `:ses_` when
/// splitting so paths containing `:` (e.g. `C:\Users\...`) still parse.
pub fn sqlite_source(db: &Path, session_id: &str) -> String {
    format!("{SQLITE_PREFIX}{}:{}", db.display(), session_id)
}

pub fn parse_sqlite_source(source: &str) -> Option<(PathBuf, String)> {
    let body = source.strip_prefix(SQLITE_PREFIX)?;
    let split_at = body.rfind(":ses_")?;
    let (db, sid) = body.split_at(split_at);
    Some((PathBuf::from(db), sid[1..].to_string()))
}

pub fn list_sessions() -> Result<Vec<SessionInfo>> {
    let path = match db_path() {
        Ok(path) if path.exists() => path,
        _ => return Ok(Vec::new()),
    };
    let conn = open_read_only(&path)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, title, directory, time_created, time_updated \
             FROM session ORDER BY time_updated DESC",
        )
        .context("prepare opencode session listing")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, title, directory, time_created, time_updated) in rows {
        let summary = first_user_summary(&conn, &id).ok().flatten();
        out.push(build_session_info(
            &id,
            title,
            directory,
            time_created,
            time_updated,
            summary,
            sqlite_source(&path, &id),
        ));
    }
    Ok(out)
}

pub fn parse_one(file_path: &str, fallback_session_id: &str) -> Result<Option<SessionInfo>> {
    let (db, session_id) = match parse_sqlite_source(file_path) {
        Some(parts) => parts,
        None => {
            // Older indexed rows or third-party callers may still pass a bare
            // session id with the canonical db path resolved here. We don't
            // honor JSON-storage paths anymore.
            if fallback_session_id.is_empty() {
                return Ok(None);
            }
            (db_path()?, fallback_session_id.to_string())
        }
    };
    parse_sqlite_session(&db, &session_id)
}

pub fn read_history_acp_messages_with_locations(
    file_path: &str,
    fallback_session_id: &str,
) -> Result<Vec<HistoryAcpMessage>> {
    let (db, session_id) = match parse_sqlite_source(file_path) {
        Some(parts) => parts,
        None => {
            if fallback_session_id.is_empty() {
                return Ok(Vec::new());
            }
            (db_path()?, fallback_session_id.to_string())
        }
    };
    let raw = read_sqlite_messages(&db, &session_id)?;
    Ok(raw_messages_to_history(&raw, file_path))
}

#[derive(Debug, Clone)]
struct OpencodeMessage {
    role: String,
    time_created: i64,
    parts: Vec<OpencodePart>,
}

#[derive(Debug, Clone)]
struct OpencodePart {
    /// Discriminator from the OpenCode `data.type` field. We currently render
    /// `text`, `reasoning`, `tool`, `file`, and the `todo*` family;
    /// everything else is retained as `kind` so future renderers can opt in
    /// without re-running the parser.
    kind: String,
    /// Trimmed payload for `kind == "text"` and `kind == "reasoning"`.
    text: Option<String>,
    /// OpenCode's `callID` for `kind == "tool"`. Used so the `tool_call` and
    /// the matching `tool_call_update` end up with the same ACP toolCallId
    /// and the frontend can merge input + output into one rendered call.
    tool_call_id: Option<String>,
    /// Tool name for `kind == "tool"`.
    tool_name: Option<String>,
    /// `state.input` payload for `kind == "tool"`.
    tool_input: Option<Value>,
    /// `state.output` payload for `kind == "tool"` when the inner status is
    /// `completed` or `error`. None for `pending` / `running`.
    tool_output: Option<Value>,
    /// `kind == "file"` references files by URL; we surface them as resource
    /// blocks so the existing assistant-message renderer picks them up.
    file_url: Option<String>,
    file_name: Option<String>,
    file_media_type: Option<String>,
}

fn open_read_only(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open opencode sqlite at {}", path.display()))
}

fn parse_sqlite_session(db: &Path, session_id: &str) -> Result<Option<SessionInfo>> {
    if !db.exists() {
        return Ok(None);
    }
    let conn = open_read_only(db)?;
    let mut stmt = conn.prepare(
        "SELECT id, title, directory, time_created, time_updated FROM session WHERE id = ?",
    )?;
    let row = stmt
        .query_row([session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        })
        .ok();
    let Some((id, title, directory, time_created, time_updated)) = row else {
        return Ok(None);
    };
    let summary = first_user_summary(&conn, &id).ok().flatten();
    Ok(Some(build_session_info(
        &id,
        title,
        directory,
        time_created,
        time_updated,
        summary,
        sqlite_source(db, &id),
    )))
}

fn first_user_summary(conn: &Connection, session_id: &str) -> Result<Option<String>> {
    let mut messages_stmt = conn
        .prepare("SELECT id, data FROM message WHERE session_id = ? ORDER BY time_created ASC")?;
    let rows = messages_stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (msg_id, data) in rows {
        let role = serde_json::from_str::<Value>(&data)
            .ok()
            .and_then(|value| value.get("role").and_then(Value::as_str).map(String::from))
            .unwrap_or_default();
        if role != "user" {
            continue;
        }
        let mut parts_stmt = conn
            .prepare("SELECT data FROM part WHERE message_id = ? ORDER BY time_created ASC")?;
        let texts: Vec<String> = parts_stmt
            .query_map([&msg_id], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .filter_map(|raw| {
                let value: Value = serde_json::from_str(&raw).ok()?;
                extract_part_text(&value)
            })
            .collect();
        let combined = texts.join("\n");
        if !combined.trim().is_empty() {
            return Ok(Some(normalize_preview(&combined)));
        }
    }
    Ok(None)
}

fn read_sqlite_messages(db: &Path, session_id: &str) -> Result<Vec<OpencodeMessage>> {
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_read_only(db)?;
    let mut messages_stmt = conn.prepare(
        "SELECT id, time_created, data FROM message WHERE session_id = ? ORDER BY time_created ASC",
    )?;
    let message_rows = messages_stmt
        .query_map([session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut parts_stmt = conn.prepare(
        "SELECT message_id, data FROM part WHERE session_id = ? ORDER BY time_created ASC",
    )?;
    let mut parts_by_message: HashMap<String, Vec<Value>> = HashMap::new();
    for entry in parts_stmt.query_map([session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })? {
        let (message_id, raw) = entry?;
        if let Ok(value) = serde_json::from_str::<Value>(&raw) {
            parts_by_message.entry(message_id).or_default().push(value);
        }
    }

    let mut out = Vec::with_capacity(message_rows.len());
    for (message_id, time_created, raw_data) in message_rows {
        let data: Value = serde_json::from_str(&raw_data).unwrap_or(Value::Null);
        let role = data
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let parts = parts_by_message
            .remove(&message_id)
            .unwrap_or_default()
            .into_iter()
            .map(value_to_part)
            .collect();
        out.push(OpencodeMessage {
            role,
            time_created: time_created.unwrap_or(0),
            parts,
        });
    }
    Ok(out)
}

fn value_to_part(value: Value) -> OpencodePart {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let text = match kind.as_str() {
        "text" | "reasoning" => value
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(String::from),
        _ => None,
    };
    // OpenCode's SQLite layer flattens TS-side `ToolInvocationPart` to a
    // bare `type: "tool"` row with `tool` (name), `callID`, and a nested
    // `state` carrying `status`, `input`, and `output`. The TS schema in
    // the source repo (`tool-invocation` + `toolInvocation.{toolName,...}`)
    // is the runtime memory shape, not the persisted shape; we read what's
    // actually on disk. See memory [[opencode-sqlite-schema]].
    let is_tool = kind == "tool";
    let tool_call_id = is_tool
        .then(|| {
            value
                .get("callID")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(String::from)
        })
        .flatten();
    let tool_name = is_tool
        .then(|| {
            value
                .get("tool")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(String::from)
        })
        .flatten();
    let tool_input = is_tool
        .then(|| value.pointer("/state/input").cloned())
        .flatten();
    // `state.output` only exists once the tool finishes (status `completed`
    // or `error`). Streaming `pending`/`running` states have no output yet.
    let tool_output = is_tool
        .then(|| value.pointer("/state/output").cloned())
        .flatten();
    let file_url = (kind == "file")
        .then(|| value.get("url").and_then(Value::as_str).map(String::from))
        .flatten();
    let file_name = (kind == "file")
        .then(|| {
            value
                .get("filename")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .flatten();
    let file_media_type = (kind == "file")
        .then(|| {
            value
                .get("mediaType")
                .and_then(Value::as_str)
                .map(String::from)
        })
        .flatten();
    OpencodePart {
        kind,
        text,
        tool_call_id,
        tool_name,
        tool_input,
        tool_output,
        file_url,
        file_name,
        file_media_type,
    }
}

fn extract_part_text(value: &Value) -> Option<String> {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" | "reasoning" => value
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(String::from),
        "tool" => {
            let tool_name = value
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[Tool: {tool_name}]"))
        }
        "file" => {
            let label = value
                .get("filename")
                .and_then(Value::as_str)
                .or_else(|| value.get("url").and_then(Value::as_str))
                .unwrap_or("file");
            Some(format!("[File: {label}]"))
        }
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_session_info(
    session_id: &str,
    title: Option<String>,
    directory: Option<String>,
    time_created: Option<i64>,
    time_updated: Option<i64>,
    summary: Option<String>,
    file_path: String,
) -> SessionInfo {
    let project_path = directory.clone();
    let project_name = directory.as_deref().and_then(|dir| {
        Path::new(dir)
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
    });
    let display_title = title
        .clone()
        .or_else(|| summary.clone())
        .or_else(|| project_name.clone());
    SessionInfo {
        id: session_id.to_string(),
        agent: Agent::Opencode,
        forked_from_agent: None,
        forked_from_id: None,
        project_path,
        project_name,
        started_at: time_created,
        updated_at: time_updated.or(time_created),
        message_count: 0,
        rename_title: None,
        title: display_title,
        first_user_message: summary,
        file_path,
        file_size: 0,
        partial: false,
        available: true,
        archived: false,
        subagents: Vec::new(),
    }
}

/// Map an OpenCode built-in tool name to the ACP `kind` value the frontend
/// uses to pick a renderer. Sticking to the small ACP vocabulary
/// (`execute` / `read` / `edit` / `delete` / `move` / `search` / `fetch` /
/// `think` / `task_list` / `tool_call`) lets OpenCode `bash` calls show up
/// as shell-execution cards, `read` as file-read cards, etc.
///
/// Unknown tool names — custom tools, MCP-added tools, future built-ins —
/// fall back to the generic `tool_call` bucket. The frontend then routes by
/// `tool.title` (the OpenCode `tool` name) through `canonicalKnownToolName`,
/// so e.g. `task` still ends up labeled "Task" even though it doesn't have
/// its own ACP kind.
fn opencode_tool_to_acp_kind(tool_name: &str) -> &'static str {
    match tool_name {
        "bash" => "execute",
        "read" => "read",
        "write" | "edit" | "patch" | "multiedit" | "multi_edit" => "edit",
        "delete" | "remove" => "delete",
        "move" | "rename" => "move",
        // `list` / `ls` aren't mapped to an ACP kind on purpose. The
        // frontend's `canonicalToolKindDisplay` doesn't have a list bucket,
        // and forcing them into `read` would make them render as "Read
        // <path>" (the file-read card), which is misleading for directory
        // listings. Falling through to `tool_call` lets
        // `canonicalKnownToolName` match the raw tool name and render
        // "List" via its dedicated case.
        "grep" | "glob" | "search" | "websearch" | "web_search" => "search",
        "webfetch" | "fetch" => "fetch",
        // `task` is the OpenCode subagent dispatcher — semantically closer
        // to "delegate a job" than to "think". Leave the kind generic so
        // the frontend falls through to its `canonicalKnownToolName` map
        // which renders it as "Task".
        _ => "tool_call",
    }
}

fn raw_messages_to_history(
    messages: &[OpencodeMessage],
    file_path: &str,
) -> Vec<HistoryAcpMessage> {
    let mut out = Vec::new();
    for message in messages {
        let timestamp = (message.time_created != 0).then_some(message.time_created);
        let location = SourceLocation::file(file_path);

        // User turns merge all `text` parts into one prompt; OpenCode never
        // mixes user reasoning or tool calls into the user role.
        if message.role == "user" {
            let combined_text = message
                .parts
                .iter()
                .filter(|part| part.kind == "text")
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string();
            if !combined_text.is_empty() {
                out.push(HistoryAcpMessage {
                    message: history_user_message(combined_text, timestamp),
                    timestamp,
                    location,
                    synthetic: true,
                });
            }
            continue;
        }

        // Assistant / other roles: walk parts in order so reasoning, text,
        // and tool events keep their original sequence in the rendered
        // transcript. We coalesce consecutive `text`/`reasoning` parts into
        // one chunk each so the UI doesn't render every partial fragment as
        // a separate bubble.
        let mut pending_text: Vec<String> = Vec::new();
        let mut pending_reasoning: Vec<String> = Vec::new();
        let flush_text = |buf: &mut Vec<String>, out: &mut Vec<HistoryAcpMessage>| {
            if buf.is_empty() {
                return;
            }
            let combined = std::mem::take(buf).join("\n").trim().to_string();
            if combined.is_empty() {
                return;
            }
            out.push(HistoryAcpMessage {
                message: history_assistant_message(combined, timestamp),
                timestamp,
                location: location.clone(),
                synthetic: true,
            });
        };
        let flush_reasoning = |buf: &mut Vec<String>, out: &mut Vec<HistoryAcpMessage>| {
            if buf.is_empty() {
                return;
            }
            let combined = std::mem::take(buf).join("\n").trim().to_string();
            if combined.is_empty() {
                return;
            }
            out.push(HistoryAcpMessage {
                message: history_thought_message(combined, timestamp),
                timestamp,
                location: location.clone(),
                synthetic: true,
            });
        };

        for part in &message.parts {
            match part.kind.as_str() {
                "text" => {
                    flush_reasoning(&mut pending_reasoning, &mut out);
                    if let Some(text) = &part.text {
                        pending_text.push(text.clone());
                    }
                }
                "reasoning" => {
                    flush_text(&mut pending_text, &mut out);
                    if let Some(text) = &part.text {
                        pending_reasoning.push(text.clone());
                    }
                }
                "tool" => {
                    flush_reasoning(&mut pending_reasoning, &mut out);
                    flush_text(&mut pending_text, &mut out);
                    let tool_name = part
                        .tool_name
                        .clone()
                        .unwrap_or_else(|| "tool".to_string());
                    let acp_kind = opencode_tool_to_acp_kind(&tool_name);
                    let raw_input = part.tool_input.clone().unwrap_or(Value::Null);
                    let call_id = part.tool_call_id.clone();
                    out.push(HistoryAcpMessage {
                        message: history_tool_call_message_with_kind(
                            call_id.clone(),
                            tool_name,
                            acp_kind,
                            raw_input,
                            timestamp,
                        ),
                        timestamp,
                        location: location.clone(),
                        synthetic: true,
                    });
                    // Only emit a tool_call_update once we actually have an
                    // output. OpenCode runs tools synchronously inside its
                    // own process, so completed calls have both input and
                    // output materialized in the SQLite row. Pending /
                    // running calls stay as a single tool_call event so the
                    // renderer doesn't show a phantom empty result.
                    if let Some(output) = part.tool_output.clone() {
                        out.push(HistoryAcpMessage {
                            message: history_tool_result_message(call_id, output, timestamp),
                            timestamp,
                            location: location.clone(),
                            synthetic: true,
                        });
                    }
                }
                "file" => {
                    flush_reasoning(&mut pending_reasoning, &mut out);
                    flush_text(&mut pending_text, &mut out);
                    let label = part
                        .file_name
                        .clone()
                        .or_else(|| part.file_url.clone())
                        .unwrap_or_else(|| "file".to_string());
                    let media = part.file_media_type.as_deref().unwrap_or("file");
                    out.push(HistoryAcpMessage {
                        message: history_assistant_message(
                            format!("[{media}: {label}]"),
                            timestamp,
                        ),
                        timestamp,
                        location: location.clone(),
                        synthetic: true,
                    });
                }
                // step-start, source-url, unknown future kinds: dropped on
                // purpose. step-start is just a marker; source-url is rare
                // metadata that doesn't carry visible content.
                _ => {}
            }
        }
        flush_reasoning(&mut pending_reasoning, &mut out);
        flush_text(&mut pending_text, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_db() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sessio-opencode-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join("opencode.db")
    }

    fn create_test_db(path: &Path) -> Connection {
        let conn = Connection::open(path).expect("open sqlite");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                directory TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
             );
             CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES session(id) ON DELETE CASCADE
             );
             CREATE TABLE part (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES session(id) ON DELETE CASCADE,
                FOREIGN KEY(message_id) REFERENCES message(id) ON DELETE CASCADE
             );",
        )
        .expect("create schema");
        conn
    }

    #[test]
    fn parse_sqlite_source_handles_windows_paths() {
        let parsed =
            parse_sqlite_source("sqlite:C:\\opencode\\opencode.db:ses_abc").expect("parsed");
        assert_eq!(parsed.0, PathBuf::from("C:\\opencode\\opencode.db"));
        assert_eq!(parsed.1, "ses_abc");
    }

    #[test]
    fn sqlite_session_listing_reads_summary_and_history() {
        let db = unique_db();
        let conn = create_test_db(&db);
        conn.execute(
            "INSERT INTO session VALUES ('ses_1', 'Title 1', '/tmp/proj', 1000, 2000)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message VALUES ('msg_1', 'ses_1', 1000, ?)",
            [r#"{"role":"user"}"#],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('part_1', 'ses_1', 'msg_1', 1000, ?)",
            [r#"{"type":"text","text":"hello opencode"}"#],
        )
        .unwrap();
        drop(conn);

        let session = parse_sqlite_session(&db, "ses_1").unwrap().expect("session");
        assert_eq!(session.id, "ses_1");
        assert_eq!(session.agent, Agent::Opencode);
        assert_eq!(session.title.as_deref(), Some("Title 1"));
        assert_eq!(session.project_path.as_deref(), Some("/tmp/proj"));
        assert_eq!(session.first_user_message.as_deref(), Some("hello opencode"));
        assert_eq!(session.started_at, Some(1000));
        assert_eq!(session.updated_at, Some(2000));
        assert!(session.file_path.starts_with("sqlite:"));

        let messages = read_sqlite_messages(&db, "ses_1").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].parts[0].text.as_deref(), Some("hello opencode"));

        let _ = fs::remove_file(&db);
        let _ = fs::remove_dir_all(db.parent().unwrap());
    }

    #[test]
    fn raw_messages_emit_user_assistant_and_tool_history() {
        fn text_part(text: &str) -> OpencodePart {
            OpencodePart {
                kind: "text".to_string(),
                text: Some(text.to_string()),
                tool_call_id: None,
                tool_name: None,
                tool_input: None,
                tool_output: None,
                file_url: None,
                file_name: None,
                file_media_type: None,
            }
        }
        fn reasoning_part(text: &str) -> OpencodePart {
            OpencodePart {
                kind: "reasoning".to_string(),
                text: Some(text.to_string()),
                tool_call_id: None,
                tool_name: None,
                tool_input: None,
                tool_output: None,
                file_url: None,
                file_name: None,
                file_media_type: None,
            }
        }
        fn tool_part(
            call_id: &str,
            tool: &str,
            input: Value,
            output: Option<Value>,
        ) -> OpencodePart {
            OpencodePart {
                kind: "tool".to_string(),
                text: None,
                tool_call_id: Some(call_id.to_string()),
                tool_name: Some(tool.to_string()),
                tool_input: Some(input),
                tool_output: output,
                file_url: None,
                file_name: None,
                file_media_type: None,
            }
        }

        let messages = vec![
            OpencodeMessage {
                role: "user".to_string(),
                time_created: 1000,
                parts: vec![text_part("hello")],
            },
            OpencodeMessage {
                role: "assistant".to_string(),
                time_created: 1100,
                parts: vec![
                    reasoning_part("planning the response"),
                    text_part("hi"),
                    tool_part(
                        "call-1",
                        "bash",
                        json!({ "cmd": "ls" }),
                        Some(Value::String("readme.md\n".to_string())),
                    ),
                ],
            },
        ];
        let history = raw_messages_to_history(&messages, "sqlite:/tmp:ses_x");
        // user, assistant reasoning, assistant text, tool call, tool result.
        assert_eq!(history.len(), 5);
        assert_eq!(history[0].message.method, "session/prompt");
        assert_eq!(history[1].message.method, "session/update");
        assert_eq!(
            history[1].message.update_type.as_deref(),
            Some("agent_thought_chunk")
        );
        assert_eq!(history[2].message.method, "session/update");
        assert_eq!(
            history[2].message.update_type.as_deref(),
            Some("agent_message_chunk")
        );
        assert_eq!(history[3].message.method, "session/update");
        assert_eq!(history[3].message.update_type.as_deref(), Some("tool_call"));
        // tool_call should carry the OpenCode callID so the renderer can
        // merge the tool_call_update that follows.
        let call_data = &history[3].message.data;
        assert_eq!(
            call_data.pointer("/update/toolCallId").and_then(Value::as_str),
            Some("call-1")
        );
        // bash maps to ACP `execute`, not the generic `tool_call`.
        assert_eq!(
            call_data.pointer("/update/kind").and_then(Value::as_str),
            Some("execute")
        );
        assert_eq!(
            history[4].message.update_type.as_deref(),
            Some("tool_call_update")
        );
        let update_data = &history[4].message.data;
        assert_eq!(
            update_data
                .pointer("/update/toolCallId")
                .and_then(Value::as_str),
            Some("call-1")
        );
    }

    #[test]
    fn value_to_part_handles_tool_completed_state() {
        // Mirror the actual on-disk shape: `type: "tool"` with nested
        // `state.{status,input,output}` and a top-level `tool` name.
        let part = value_to_part(json!({
            "type": "tool",
            "tool": "bash",
            "callID": "call-1",
            "state": {
                "status": "completed",
                "input": { "cmd": "ls" },
                "output": "readme.md\n",
            }
        }));
        assert_eq!(part.kind, "tool");
        assert_eq!(part.tool_name.as_deref(), Some("bash"));
        assert_eq!(
            part.tool_input.as_ref().and_then(|v| v.get("cmd")).and_then(Value::as_str),
            Some("ls")
        );
        assert_eq!(part.tool_output.as_ref().and_then(Value::as_str), Some("readme.md\n"));
    }

    #[test]
    fn value_to_part_skips_output_for_pending_tool() {
        let part = value_to_part(json!({
            "type": "tool",
            "tool": "bash",
            "callID": "call-1",
            "state": {
                "status": "pending",
                "input": { "cmd": "ls" },
            }
        }));
        assert!(part.tool_output.is_none());
    }

    #[test]
    fn value_to_part_extracts_reasoning_text() {
        let part = value_to_part(json!({
            "type": "reasoning",
            "text": "thinking about the problem",
        }));
        assert_eq!(part.kind, "reasoning");
        assert_eq!(part.text.as_deref(), Some("thinking about the problem"));
    }
}
