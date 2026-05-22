use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};

use anyhow::{bail, Context, Result};
use tauri::{AppHandle, Emitter};

use super::fake;
use super::types::{
    AgentInput, AgentRuntimeEvent, AgentRuntimeEventPayload, AgentSessionHandle, AgentTurnHandle,
    RuntimeCapabilitySet, RuntimeError, RuntimeSessionStatus, RuntimeStatus, RuntimeTransportKind,
    RuntimeTurnStatus, StartAgentSession,
};
use crate::models::Agent;

#[derive(Clone)]
pub struct RuntimeManager {
    inner: Arc<RuntimeManagerInner>,
}

struct RuntimeManagerInner {
    app: AppHandle,
    sequence: AtomicU64,
    id_counter: AtomicU64,
    sessions: Mutex<HashMap<String, RuntimeSessionState>>,
}

#[derive(Debug, Clone)]
struct RuntimeSessionState {
    handle: AgentSessionHandle,
    active_turn_id: Option<String>,
    turn_cancellations: HashMap<String, Arc<AtomicBool>>,
    permission_waiters: HashMap<String, mpsc::Sender<bool>>,
}

impl RuntimeManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            inner: Arc::new(RuntimeManagerInner {
                app,
                sequence: AtomicU64::new(1),
                id_counter: AtomicU64::new(1),
                sessions: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn status(&self, agent: Agent) -> RuntimeStatus {
        RuntimeStatus {
            agent,
            transport: RuntimeTransportKind::Fake,
            available: true,
            status: RuntimeSessionStatus::Idle,
            capabilities: RuntimeCapabilitySet::fake(),
            error: None,
            metadata: Default::default(),
        }
    }

    pub fn start_session(&self, req: StartAgentSession) -> Result<AgentSessionHandle> {
        if req.workspace_path.trim().is_empty() {
            bail!("workspace_path is required");
        }
        let workspace = Path::new(&req.workspace_path);
        if !workspace.is_absolute() {
            bail!("workspace_path must be absolute: {}", req.workspace_path);
        }
        if !workspace.exists() {
            bail!("workspace_path does not exist: {}", req.workspace_path);
        }

        let id = self.next_id("runtime");
        let agent_session_id = self.next_id("fake-agent-session");
        let handle = AgentSessionHandle {
            sessio_runtime_session_id: id.clone(),
            agent: req.agent,
            transport: RuntimeTransportKind::Fake,
            agent_runtime_session_id: agent_session_id.clone(),
            workspace_path: req.workspace_path.clone(),
            status: RuntimeSessionStatus::Active,
            capabilities: RuntimeCapabilitySet::fake(),
        };

        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            sessions.insert(
                id.clone(),
                RuntimeSessionState {
                    handle: handle.clone(),
                    active_turn_id: None,
                    turn_cancellations: HashMap::new(),
                    permission_waiters: HashMap::new(),
                },
            );
        }

        self.emit(AgentRuntimeEventPayload::SessionStarted {
            agent: handle.agent,
            sessio_runtime_session_id: id.clone(),
            agent_runtime_session_id: agent_session_id,
            transport: handle.transport,
            workspace_path: handle.workspace_path.clone(),
            capabilities: handle.capabilities.clone(),
        })?;

        if let Some(text) = req.initial_prompt {
            if !text.trim().is_empty() {
                let _ = self.send_input(
                    &id,
                    AgentInput {
                        text,
                        attachments: Vec::new(),
                        options: req.options,
                    },
                )?;
            }
        }

        Ok(handle)
    }

    pub fn load_session(
        &self,
        agent: Agent,
        agent_runtime_session_id: String,
        workspace_path: String,
    ) -> Result<AgentSessionHandle> {
        if agent_runtime_session_id.trim().is_empty() {
            bail!("runtime session id is required");
        }
        if workspace_path.trim().is_empty() {
            bail!("workspace_path is required");
        }
        let workspace = Path::new(&workspace_path);
        if !workspace.is_absolute() {
            bail!("workspace_path must be absolute: {}", workspace_path);
        }
        if !workspace.exists() {
            bail!("workspace_path does not exist: {}", workspace_path);
        }

        let handle = AgentSessionHandle {
            sessio_runtime_session_id: agent_runtime_session_id.clone(),
            agent,
            transport: RuntimeTransportKind::Fake,
            agent_runtime_session_id,
            workspace_path,
            status: RuntimeSessionStatus::Idle,
            capabilities: RuntimeCapabilitySet::fake(),
        };

        let inserted = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            if let Some(existing) = sessions.get(&handle.sessio_runtime_session_id) {
                return Ok(existing.handle.clone());
            }
            sessions.insert(
                handle.sessio_runtime_session_id.clone(),
                RuntimeSessionState {
                    handle: handle.clone(),
                    active_turn_id: None,
                    turn_cancellations: HashMap::new(),
                    permission_waiters: HashMap::new(),
                },
            );
            true
        };

        if inserted {
            self.emit(AgentRuntimeEventPayload::SessionStarted {
                agent: handle.agent,
                sessio_runtime_session_id: handle.sessio_runtime_session_id.clone(),
                agent_runtime_session_id: handle.agent_runtime_session_id.clone(),
                transport: handle.transport,
                workspace_path: handle.workspace_path.clone(),
                capabilities: handle.capabilities.clone(),
            })?;
        }

        Ok(handle)
    }

    pub fn send_input(
        &self,
        sessio_runtime_session_id: &str,
        input: AgentInput,
    ) -> Result<AgentTurnHandle> {
        if input.text.trim().is_empty() {
            bail!("input text is required");
        }

        let turn_id = self.next_id("turn");
        let cancel_token = Arc::new(AtomicBool::new(false));
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let state = sessions
                .get_mut(sessio_runtime_session_id)
                .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?;
            if let Some(active) = &state.active_turn_id {
                bail!("runtime session already has active turn: {active}");
            }
            state.active_turn_id = Some(turn_id.clone());
            state
                .turn_cancellations
                .insert(turn_id.clone(), cancel_token.clone());
            state.handle.status = RuntimeSessionStatus::Active;
        }

        self.emit(AgentRuntimeEventPayload::TurnStarted {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.clone(),
        })?;

        fake::spawn_stream(
            self.clone(),
            sessio_runtime_session_id.to_string(),
            turn_id.clone(),
            input,
            cancel_token,
        );

        Ok(AgentTurnHandle {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id,
            status: RuntimeTurnStatus::Streaming,
        })
    }

    pub fn cancel_turn(
        &self,
        sessio_runtime_session_id: &str,
        turn_id: &str,
    ) -> Result<()> {
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let state = sessions
                .get_mut(sessio_runtime_session_id)
                .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?;
            match state.active_turn_id.as_deref() {
                Some(active) if active == turn_id => {
                    state.active_turn_id = None;
                    if let Some(token) = state.turn_cancellations.get(turn_id) {
                        token.store(true, Ordering::Relaxed);
                    }
                    for (_, sender) in state.permission_waiters.drain() {
                        let _ = sender.send(false);
                    }
                    state.handle.status = RuntimeSessionStatus::Idle;
                }
                Some(active) => bail!("active turn is {active}, not {turn_id}"),
                None => bail!("runtime session has no active turn"),
            }
        }

        self.emit(AgentRuntimeEventPayload::TurnCancelled {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
        })
    }

    pub fn respond_permission(
        &self,
        sessio_runtime_session_id: &str,
        request_id: &str,
        approved: bool,
    ) -> Result<()> {
        let sender = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let state = sessions
                .get_mut(sessio_runtime_session_id)
                .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?;
            state
                .permission_waiters
                .remove(request_id)
                .with_context(|| format!("unknown permission request: {request_id}"))?
        };
        sender
            .send(approved)
            .map_err(|_| anyhow::anyhow!("permission request is no longer active"))
    }

    pub(crate) fn request_permission(
        &self,
        sessio_runtime_session_id: &str,
        turn_id: &str,
        request_id: &str,
        tool_name: &str,
        input: Option<serde_json::Value>,
    ) -> Result<bool> {
        let (sender, receiver) = mpsc::channel();
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let state = sessions
                .get_mut(sessio_runtime_session_id)
                .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?;
            state
                .permission_waiters
                .insert(request_id.to_string(), sender);
        }
        self.emit(AgentRuntimeEventPayload::PermissionRequested {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            request_id: request_id.to_string(),
            tool_name: tool_name.to_string(),
            input,
        })?;
        let approved = receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("permission response channel closed"))?;
        self.emit(AgentRuntimeEventPayload::PermissionResolved {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            request_id: request_id.to_string(),
            approved,
        })?;
        Ok(approved)
    }

    pub(crate) fn complete_turn(
        &self,
        sessio_runtime_session_id: &str,
        turn_id: &str,
    ) -> Result<()> {
        let should_emit = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let Some(state) = sessions.get_mut(sessio_runtime_session_id) else {
                return Ok(());
            };
            if state.active_turn_id.as_deref() != Some(turn_id) {
                false
            } else {
                state.active_turn_id = None;
                state.turn_cancellations.remove(turn_id);
                state.handle.status = RuntimeSessionStatus::Idle;
                true
            }
        };
        if should_emit {
            self.emit(AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                turn_id: turn_id.to_string(),
                result: None,
            })?;
        }
        Ok(())
    }

    pub(crate) fn fail_turn(
        &self,
        sessio_runtime_session_id: &str,
        turn_id: &str,
        error: RuntimeError,
    ) -> Result<()> {
        {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            if let Some(state) = sessions.get_mut(sessio_runtime_session_id) {
                if state.active_turn_id.as_deref() == Some(turn_id) {
                    state.active_turn_id = None;
                    state.turn_cancellations.remove(turn_id);
                    state.handle.status = RuntimeSessionStatus::Errored;
                }
            }
        }
        self.emit(AgentRuntimeEventPayload::TurnError {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
            error,
        })
    }

    pub(crate) fn emit(&self, payload: AgentRuntimeEventPayload) -> Result<()> {
        log::info!("[sessio-runtime:backend:event] {:?}", payload);
        let event = AgentRuntimeEvent {
            sequence: self.inner.sequence.fetch_add(1, Ordering::Relaxed),
            timestamp: now_ms(),
            payload,
        };
        self.inner
            .app
            .emit("agent-runtime-event", event)
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }

    fn next_id(&self, prefix: &str) -> String {
        let counter = self.inner.id_counter.fetch_add(1, Ordering::Relaxed);
        format!("{prefix}-{}-{counter}", now_ms())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_capabilities_are_stream_ready() {
        let caps = RuntimeCapabilitySet::fake();
        assert!(caps.supports_cancel);
        assert!(caps.supports_permissions);
        assert!(caps.supports_tool_deltas);
        assert!(!caps.supports_attachments);
    }
}
