use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};

use super::types::AgentRuntimeEvent;
use crate::agents::runtime::types::AcpProtocolMessage;
use crate::agents::sources::pi::parser::{project_dir_for_workspace_path, session_file_name};
use crate::models::{normalize_preview, Agent, SessionInfo};
use crate::store::SessionStore;
use tauri::AppHandle;

#[derive(Clone)]
pub struct PiAcpSessionStore {
    inner: Arc<PiAcpSessionStoreInner>,
}

struct PiAcpSessionStoreInner {
    store: Arc<dyn SessionStore>,
    sessions: Mutex<HashMap<String, PersistedPiSession>>,
}

#[derive(Debug, Clone)]
struct PersistedPiSession {
    agent_session_id: String,
    file_path: String,
    workspace_path: String,
    first_user_message: Option<String>,
    started_at: i64,
    last_updated_at: i64,
    message_count: usize,
    transcript_lines: Vec<String>,
    /// Tracks the runtime's request to keep this session out of the sidebar
    /// (fake agent sessions, helper sessions). Persisted via SessionInfo's
    /// `is_auxiliary` flag — the field name predates that schema and stays
    /// here as the runtime-side concept.
    is_auxiliary: bool,
}

const ENABLE_PI_ACP_TRANSCRIPT_PERSISTENCE: bool = true;

impl PiAcpSessionStore {
    pub fn new(_app: AppHandle, store: Arc<dyn SessionStore>) -> Self {
        Self {
            inner: Arc::new(PiAcpSessionStoreInner {
                store,
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn watch_runtime_events(
        &self,
        runtime: crate::agents::runtime::RuntimeManager,
    ) -> Result<()> {
        if !ENABLE_PI_ACP_TRANSCRIPT_PERSISTENCE {
            let _ = runtime;
            log::info!("[pi-acp-session-store] transcript persistence disabled");
            return Ok(());
        }
        let runtime_filter = runtime.clone();
        let receiver = runtime.subscribe_events_filtered(move |payload| {
            use crate::agents::runtime::types::AgentRuntimeEventPayload;

            match payload {
                AgentRuntimeEventPayload::SessionStarted { agent, .. } => *agent == Agent::AstraPi,
                AgentRuntimeEventPayload::AcpProtocolMessage { .. } => {
                    runtime_filter.event_session_agent(payload) == Some(Agent::AstraPi)
                }
                AgentRuntimeEventPayload::SessionEnded { .. } => {
                    runtime_filter.event_session_agent(payload) == Some(Agent::AstraPi)
                }
                _ => false,
            }
        })?;
        let service = self.clone();
        thread::spawn(move || {
            for event in receiver {
                if let Err(error) = service.handle_runtime_event(event) {
                    log::warn!("[pi-acp-session-store] {error}");
                }
            }
        });
        Ok(())
    }

    fn handle_runtime_event(&self, event: AgentRuntimeEvent) -> Result<()> {
        use crate::agents::runtime::types::AgentRuntimeEventPayload;

        match &event.payload {
            AgentRuntimeEventPayload::SessionStarted {
                agent,
                sessio_runtime_session_id,
                agent_runtime_session_id,
                workspace_path,
                metadata,
                ..
            } => {
                if *agent != Agent::AstraPi {
                    return Ok(());
                }
                self.remember_session_start(
                    sessio_runtime_session_id,
                    agent_runtime_session_id,
                    workspace_path,
                    event.timestamp,
                    metadata.contains_key("astraRunId"),
                )?;
            }
            AgentRuntimeEventPayload::AcpProtocolMessage {
                sessio_runtime_session_id,
                message,
                ..
            } => {
                if !self.is_known_pi_session(sessio_runtime_session_id)? {
                    return Ok(());
                }
                let agent_session_id = match message.acp_session_id.as_deref() {
                    Some(value) if !value.trim().is_empty() => value,
                    _ => return Ok(()),
                };
                let workspace_path = session_workspace_path(message);
                self.remember_session_start(
                    sessio_runtime_session_id,
                    agent_session_id,
                    workspace_path.as_deref().unwrap_or(""),
                    event.timestamp,
                    false,
                )?;
                self.append_protocol_message(
                    sessio_runtime_session_id,
                    agent_session_id,
                    message,
                    event.timestamp,
                )?;
            }
            AgentRuntimeEventPayload::SessionEnded {
                sessio_runtime_session_id,
            } => {
                self.finish_session(sessio_runtime_session_id)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn is_known_pi_session(&self, sessio_runtime_session_id: &str) -> Result<bool> {
        let sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
        Ok(sessions.contains_key(sessio_runtime_session_id))
    }

    fn append_protocol_message(
        &self,
        sessio_runtime_session_id: &str,
        agent_session_id: &str,
        message: &crate::agents::runtime::types::AcpProtocolMessage,
        timestamp: i64,
    ) -> Result<()> {
        let should_upsert = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
            let entry = sessions
                .get_mut(sessio_runtime_session_id)
                .with_context(|| {
                    format!("missing pi transcript session for {sessio_runtime_session_id}")
                })?;
            if !is_fake_agent_session_id(agent_session_id) {
                entry.agent_session_id = agent_session_id.to_string();
            }
            entry.last_updated_at = timestamp.max(entry.last_updated_at);
            let had_first_user_message = entry.first_user_message.is_some();
            if entry.first_user_message.is_none() {
                entry.first_user_message = first_user_preview(message);
            }
            if entry.workspace_path.trim().is_empty() {
                if let Some(path) = session_workspace_path(message) {
                    entry.workspace_path = path;
                }
            }
            entry.transcript_lines.push(protocol_message_line(
                message,
                timestamp,
                &entry.workspace_path,
            )?);
            entry.message_count += 1;
            entry.is_auxiliary
                && !is_fake_agent_session_id(&entry.agent_session_id)
                && (!had_first_user_message && entry.first_user_message.is_some()
                    || is_turn_terminal_message(message))
        };
        if should_upsert {
            self.upsert_session_row(sessio_runtime_session_id)?;
        }
        Ok(())
    }

    fn remember_session_start(
        &self,
        sessio_runtime_session_id: &str,
        agent_session_id: &str,
        workspace_path: &str,
        timestamp: i64,
        is_auxiliary: bool,
    ) -> Result<()> {
        let should_upsert = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
            let entry = sessions
                .entry(sessio_runtime_session_id.to_string())
                .or_insert_with(|| PersistedPiSession {
                    agent_session_id: agent_session_id.to_string(),
                    file_path: String::new(),
                    workspace_path: String::new(),
                    first_user_message: None,
                    started_at: timestamp,
                    last_updated_at: timestamp,
                    message_count: 0,
                    transcript_lines: Vec::new(),
                    is_auxiliary,
                });
            if !is_fake_agent_session_id(agent_session_id) {
                entry.agent_session_id = agent_session_id.to_string();
            }
            entry.is_auxiliary |= is_auxiliary;
            if !workspace_path.trim().is_empty() {
                entry.workspace_path = workspace_path.to_string();
            }
            entry.started_at = entry.started_at.min(timestamp);
            entry.last_updated_at = timestamp.max(entry.last_updated_at);
            entry.is_auxiliary && !is_fake_agent_session_id(&entry.agent_session_id)
        };
        if should_upsert {
            self.upsert_session_row(sessio_runtime_session_id)?;
        }
        Ok(())
    }

    fn finish_session(&self, sessio_runtime_session_id: &str) -> Result<()> {
        let persisted = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
            sessions.remove(sessio_runtime_session_id)
        };
        let Some(persisted) = persisted else {
            return Ok(());
        };
        if persisted.transcript_lines.is_empty()
            || is_fake_agent_session_id(&persisted.agent_session_id)
        {
            return Ok(());
        }
        let store = self.inner.store.clone();
        thread::spawn(move || {
            if let Err(error) = persist_finished_session(store.as_ref(), persisted) {
                log::warn!("[pi-acp-session-store] persist finished transcript failed: {error}");
            }
        });
        Ok(())
    }

    fn upsert_session_row(&self, sessio_runtime_session_id: &str) -> Result<()> {
        let persisted = {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
            sessions
                .get(sessio_runtime_session_id)
                .cloned()
                .with_context(|| {
                    format!("missing pi transcript session for {sessio_runtime_session_id}")
                })?
        };

        let file_size = fs::metadata(&persisted.file_path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let project_path =
            (!persisted.workspace_path.trim().is_empty()).then(|| persisted.workspace_path.clone());
        let project_name = project_path.as_deref().and_then(|path| {
            Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .map(String::from)
        });
        let session = SessionInfo {
            id: persisted.agent_session_id.clone(),
            agent: Agent::AstraPi,
            forked_from_agent: None,
            forked_from_id: None,
            project_path,
            project_name,
            started_at: Some(persisted.started_at),
            updated_at: Some(persisted.last_updated_at),
            message_count: persisted.message_count,
            rename_title: None,
            title: persisted.first_user_message.clone(),
            first_user_message: persisted.first_user_message.clone(),
            file_path: persisted.file_path.clone(),
            file_size,
            partial: persisted.file_path.trim().is_empty(),
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            // pi fake sessions are runtime-internal helpers; mark them as
            // auxiliary so the new sidebar filter excludes them.
            is_auxiliary: persisted.is_auxiliary,
            subagents: Vec::new(),
        };
        let scope = Path::new(&session.file_path)
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| session.file_path.clone());
        // is_auxiliary is already encoded on the SessionInfo at construction
        // time, so the single upsert path is enough — sidebar filtering keeps
        // pi runtime helpers out without a special API.
        self.inner.store.upsert_session(&scope, &session)?;
        Ok(())
    }
}

fn persist_finished_session(store: &dyn SessionStore, persisted: PersistedPiSession) -> Result<()> {
    let transcript_dir = project_dir_for_workspace_path(Some(&persisted.workspace_path))?;
    fs::create_dir_all(&transcript_dir)
        .with_context(|| format!("create pi transcript dir {}", transcript_dir.display()))?;
    let file_path = transcript_dir.join(session_file_name(&persisted.agent_session_id));
    write_protocol_message_lines_to_file(&file_path, &persisted.transcript_lines)?;
    upsert_finished_session_file(store, persisted, &file_path)
}

fn upsert_finished_session_file(
    store: &dyn SessionStore,
    persisted: PersistedPiSession,
    file_path: &Path,
) -> Result<()> {
    let file_size = fs::metadata(file_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    let file_path = file_path.to_string_lossy().into_owned();
    let project_path =
        (!persisted.workspace_path.trim().is_empty()).then(|| persisted.workspace_path.clone());
    let project_name = project_path.as_deref().and_then(|path| {
        Path::new(path)
            .file_name()
            .and_then(|value| value.to_str())
            .map(String::from)
    });
    let session = SessionInfo {
        id: persisted.agent_session_id,
        agent: Agent::AstraPi,
        forked_from_agent: None,
        forked_from_id: None,
        project_path,
        project_name,
        started_at: Some(persisted.started_at),
        updated_at: Some(persisted.last_updated_at),
        message_count: persisted.message_count,
        rename_title: None,
        title: persisted.first_user_message.clone(),
        first_user_message: persisted.first_user_message,
        file_path: file_path.clone(),
        file_size,
        partial: false,
        available: true,
        archived: false,
        origin: crate::models::SessionOrigin::Chat,
        scheduled_task_id: None,
        is_auxiliary: persisted.is_auxiliary,
        subagents: Vec::new(),
    };
    let scope = Path::new(&file_path)
        .parent()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or(file_path);
    // Auxiliary status was carried on the SessionInfo (`is_auxiliary` reads
    // from `persisted.is_auxiliary`), so the regular upsert path
    // suffices.
    store.upsert_session(&scope, &session)
}

fn is_fake_agent_session_id(value: &str) -> bool {
    value.trim().is_empty() || value.starts_with("fake-agent-session")
}

fn is_turn_terminal_message(message: &crate::agents::runtime::types::AcpProtocolMessage) -> bool {
    message.method == "session/prompt" && message.direction == "agent_to_client"
}

fn first_user_preview(
    message: &crate::agents::runtime::types::AcpProtocolMessage,
) -> Option<String> {
    if message.method != "session/prompt"
        || message.direction != "client_to_agent"
        || message.message_kind != "request"
    {
        return None;
    }
    let prompt = message.data.get("prompt")?.as_array()?;
    let text = prompt
        .iter()
        .filter_map(|item| {
            let kind = item
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("text");
            if kind == "text" || kind == "input_text" {
                item.get("text").and_then(|value| value.as_str())
            } else {
                None
            }
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
}

fn session_workspace_path(message: &AcpProtocolMessage) -> Option<String> {
    match message.method.as_str() {
        "session/new" | "session/load" | "session/resume" | "session/fork" => message
            .data
            .get("workspacePath")
            .or_else(|| message.data.get("workspace_path"))
            .and_then(|value| value.as_str())
            .map(ToString::to_string),
        _ => None,
    }
}

fn protocol_message_line(
    message: &AcpProtocolMessage,
    timestamp: i64,
    workspace_path: &str,
) -> Result<String> {
    let message = message_with_persistence_meta(message, timestamp, workspace_path);
    serde_json::to_string(&message).map_err(anyhow::Error::from)
}

fn write_protocol_message_lines_to_file(file_path: &Path, lines: &[String]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file_path)
        .with_context(|| format!("open pi transcript {}", file_path.display()))?;
    for serialized in lines {
        file.write_all(serialized.as_bytes())
            .with_context(|| format!("write pi transcript {}", file_path.display()))?;
        file.write_all(b"\n")
            .with_context(|| format!("write newline to pi transcript {}", file_path.display()))?;
    }
    file.flush()
        .with_context(|| format!("flush pi transcript {}", file_path.display()))?;
    Ok(())
}

fn message_with_persistence_meta(
    message: &AcpProtocolMessage,
    timestamp: i64,
    workspace_path: &str,
) -> AcpProtocolMessage {
    let mut message = message.clone();
    let Some(data) = message.data.as_object_mut() else {
        return message;
    };

    let meta = data
        .entry("_meta".to_string())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    if !meta.is_object() {
        *meta = serde_json::Value::Object(Default::default());
    }
    let meta = meta.as_object_mut().expect("_meta just initialized");
    let sessio = meta
        .entry("sessio".to_string())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    if !sessio.is_object() {
        *sessio = serde_json::Value::Object(Default::default());
    }
    let sessio = sessio.as_object_mut().expect("sessio just initialized");
    sessio.insert("timestamp".to_string(), serde_json::json!(timestamp));
    if !workspace_path.trim().is_empty() {
        sessio.insert(
            "workspacePath".to_string(),
            serde_json::Value::String(workspace_path.to_string()),
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use super::{
        message_with_persistence_meta, protocol_message_line, write_protocol_message_lines_to_file,
    };
    use crate::agents::runtime::types::AcpProtocolMessage;
    use serde_json::json;

    #[test]
    fn persistence_meta_records_timestamp_and_workspace_without_changing_payload() {
        let message = AcpProtocolMessage {
            direction: "client_to_agent".to_string(),
            message_kind: "request".to_string(),
            method: "session/prompt".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("pi-session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: None,
            data: json!({
                "prompt": [{ "type": "text", "text": "hello" }],
                "_meta": { "existing": true }
            }),
        };

        let persisted = message_with_persistence_meta(&message, 1_780_912_800_000, "/tmp/demo");

        assert_eq!(persisted.data["prompt"], message.data["prompt"]);
        assert_eq!(persisted.data["_meta"]["existing"], true);
        assert_eq!(
            persisted.data["_meta"]["sessio"]["timestamp"],
            1_780_912_800_000i64
        );
        assert_eq!(
            persisted.data["_meta"]["sessio"]["workspacePath"],
            "/tmp/demo"
        );
    }

    #[test]
    fn transcript_write_keeps_one_json_message_per_line() {
        let path = std::env::temp_dir().join(format!(
            "sessio-pi-batched-transcript-{}.jsonl",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let message = AcpProtocolMessage {
            direction: "agent_to_client".to_string(),
            message_kind: "notification".to_string(),
            method: "session/update".to_string(),
            protocol_version: Some("1".to_string()),
            acp_session_id: Some("pi-session-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            request_id: None,
            update_type: Some("agent_message_chunk".to_string()),
            data: json!({
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": [{ "type": "text", "text": "hello" }]
                }
            }),
        };
        let lines = vec![
            protocol_message_line(&message, 1_780_912_800_000, "/tmp/demo").unwrap(),
            protocol_message_line(&message, 1_780_912_800_100, "/tmp/demo").unwrap(),
        ];

        write_protocol_message_lines_to_file(&path, &lines).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let rows = written.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        for row in rows {
            let parsed: serde_json::Value = serde_json::from_str(row).unwrap();
            assert_eq!(parsed["method"], "session/update");
            assert_eq!(
                parsed["data"]["_meta"]["sessio"]["workspacePath"],
                "/tmp/demo"
            );
        }
        let _ = std::fs::remove_file(&path);
    }
}
