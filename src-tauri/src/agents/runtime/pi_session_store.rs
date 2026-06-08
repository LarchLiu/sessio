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
use tauri::{AppHandle, Emitter};

#[derive(Clone)]
pub struct PiAcpSessionStore {
    inner: Arc<PiAcpSessionStoreInner>,
}

struct PiAcpSessionStoreInner {
    app: AppHandle,
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
}

impl PiAcpSessionStore {
    pub fn new(app: AppHandle, store: Arc<dyn SessionStore>) -> Self {
        Self {
            inner: Arc::new(PiAcpSessionStoreInner {
                app,
                store,
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn watch_runtime_events(
        &self,
        runtime: crate::agents::runtime::RuntimeManager,
    ) -> Result<()> {
        let receiver = runtime.subscribe_events()?;
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
                ..
            } => {
                if *agent != Agent::AstraPi {
                    return Ok(());
                }
                self.ensure_session_file(
                    sessio_runtime_session_id,
                    agent_runtime_session_id,
                    workspace_path,
                    event.timestamp,
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
                self.ensure_session_file(
                    sessio_runtime_session_id,
                    agent_session_id,
                    workspace_path.as_deref().unwrap_or(""),
                    event.timestamp,
                )?;
                self.append_protocol_message(
                    sessio_runtime_session_id,
                    agent_session_id,
                    message,
                    event.timestamp,
                )?;
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

    fn ensure_session_file(
        &self,
        sessio_runtime_session_id: &str,
        agent_session_id: &str,
        workspace_path: &str,
        timestamp: i64,
    ) -> Result<()> {
        if agent_session_id.trim().is_empty() || agent_session_id.starts_with("fake-agent-session")
        {
            self.remember_session_start(
                sessio_runtime_session_id,
                agent_session_id,
                workspace_path,
                timestamp,
            )?;
            return Ok(());
        }
        let workspace_path =
            self.effective_workspace_path(sessio_runtime_session_id, workspace_path)?;
        let transcript_dir = project_dir_for_workspace_path(Some(&workspace_path))?;
        fs::create_dir_all(&transcript_dir)
            .with_context(|| format!("create pi transcript dir {}", transcript_dir.display()))?;
        let file_path = transcript_dir.join(session_file_name(agent_session_id));
        if !file_path.exists() {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file_path)
                .with_context(|| format!("create pi transcript {}", file_path.display()))?;
        }
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
            let entry = sessions
                .entry(sessio_runtime_session_id.to_string())
                .or_insert_with(|| PersistedPiSession {
                    agent_session_id: agent_session_id.to_string(),
                    file_path: file_path.to_string_lossy().to_string(),
                    workspace_path: workspace_path.clone(),
                    first_user_message: None,
                    started_at: timestamp,
                    last_updated_at: timestamp,
                    message_count: 0,
                });
            entry.agent_session_id = agent_session_id.to_string();
            entry.file_path = file_path.to_string_lossy().to_string();
            if !workspace_path.trim().is_empty() {
                entry.workspace_path = workspace_path.clone();
            }
            entry.started_at = entry.started_at.min(timestamp);
            entry.last_updated_at = timestamp.max(entry.last_updated_at);
        }
        self.upsert_session_row(sessio_runtime_session_id)
    }

    fn append_protocol_message(
        &self,
        sessio_runtime_session_id: &str,
        agent_session_id: &str,
        message: &crate::agents::runtime::types::AcpProtocolMessage,
        timestamp: i64,
    ) -> Result<()> {
        let (file_path, workspace_path) = {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
            sessions
                .get(sessio_runtime_session_id)
                .map(|entry| (entry.file_path.clone(), entry.workspace_path.clone()))
                .with_context(|| {
                    format!("missing pi transcript session for {sessio_runtime_session_id}")
                })?
        };

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .with_context(|| format!("open pi transcript {}", file_path))?;
        append_protocol_message_to_file(
            Path::new(&file_path),
            &mut file,
            message,
            timestamp,
            &workspace_path,
        )?;

        let should_refresh_index = {
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
            entry.agent_session_id = agent_session_id.to_string();
            entry.last_updated_at = timestamp.max(entry.last_updated_at);
            entry.message_count += 1;
            let had_first_user_message = entry.first_user_message.is_some();
            if entry.first_user_message.is_none() {
                entry.first_user_message = first_user_preview(message);
            }
            if entry.workspace_path.trim().is_empty() {
                if let Some(path) = session_workspace_path(message) {
                    entry.workspace_path = path;
                }
            }
            !had_first_user_message && entry.first_user_message.is_some()
                || is_turn_terminal_message(message)
        };

        if should_refresh_index {
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
    ) -> Result<()> {
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
            });
        entry.agent_session_id = agent_session_id.to_string();
        if !workspace_path.trim().is_empty() {
            entry.workspace_path = workspace_path.to_string();
        }
        entry.started_at = entry.started_at.min(timestamp);
        entry.last_updated_at = timestamp.max(entry.last_updated_at);
        Ok(())
    }

    fn effective_workspace_path(
        &self,
        sessio_runtime_session_id: &str,
        workspace_path: &str,
    ) -> Result<String> {
        if !workspace_path.trim().is_empty() {
            return Ok(workspace_path.to_string());
        }
        let sessions = self
            .inner
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("pi session persistence lock poisoned"))?;
        Ok(sessions
            .get(sessio_runtime_session_id)
            .map(|entry| entry.workspace_path.clone())
            .unwrap_or_default())
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
            partial: persisted.message_count == 0,
            available: true,
            archived: false,
            subagents: Vec::new(),
        };
        let scope = Path::new(&session.file_path)
            .parent()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| session.file_path.clone());
        self.inner.store.upsert_session(&scope, &session)?;
        self.inner
            .app
            .emit("sessions_index_updated", ())
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(())
    }
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

fn append_protocol_message_to_file(
    file_path: &Path,
    file: &mut std::fs::File,
    message: &AcpProtocolMessage,
    timestamp: i64,
    workspace_path: &str,
) -> Result<()> {
    let message = message_with_persistence_meta(message, timestamp, workspace_path);
    let serialized = serde_json::to_string(&message)?;
    file.write_all(serialized.as_bytes())
        .with_context(|| format!("write pi transcript {}", file_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("write newline to pi transcript {}", file_path.display()))?;
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
    use super::message_with_persistence_meta;
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
}
