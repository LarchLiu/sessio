use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};

use anyhow::{bail, Context, Result};
use tauri::{AppHandle, Emitter};

use agent_client_protocol::schema::RequestPermissionRequest;

use super::acp::{
    convert_permission_request, fake_permission_request, permission_resolved_event,
    permission_response_from_decision,
};
use super::acp_transport::{self, AcpSessionController};
use super::fake;
use super::types::{
    AgentInput, AgentRuntimeEvent, AgentRuntimeEventPayload, AgentSessionHandle, AgentTurnHandle,
    EnsureAgentRuntimeSession, RuntimeCapabilitySet, RuntimeError, RuntimeSessionStatus,
    RuntimeStatus, RuntimeTransportKind, RuntimeTurnStatus, StartAgentSession,
};
use crate::config;
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
    permission_waiters: HashMap<String, mpsc::Sender<RuntimePermissionDecision>>,
    acp_controller: Option<AcpSessionController>,
}

pub(crate) enum RuntimePermissionDecision {
    Selected { approved: bool },
    Cancelled,
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

        let runtime_config = runtime_config(req.agent);
        let transport = if req.options.contains_key("transport") {
            acp_transport::transport_requested(&req.options)
        } else {
            runtime_config
                .as_ref()
                .map(acp_transport::transport_from_config)
                .unwrap_or(RuntimeTransportKind::Fake)
        };
        let id = self.next_id("runtime");
        let agent_session_id = self.next_id("fake-agent-session");
        let capabilities = RuntimeCapabilitySet::fake();
        let mut acp_controller = None;
        if transport == RuntimeTransportKind::Acp {
            let command = if req.options.contains_key("command")
                || req.options.contains_key("acpCommand")
            {
                acp_transport::command_from_options(req.agent, &req.options)
            } else {
                runtime_config
                    .as_ref()
                    .map(|config| acp_transport::command_from_config(req.agent, config))
                    .unwrap_or_else(|| acp_transport::command_from_options(req.agent, &req.options))
            };
            acp_controller = Some(acp_transport::spawn_session(
                self.clone(),
                id.clone(),
                req.agent,
                req.workspace_path.clone(),
                command,
                acp_transport::AcpSessionStart::New,
            ));
        }
        let handle = AgentSessionHandle {
            sessio_runtime_session_id: id.clone(),
            agent: req.agent,
            transport,
            agent_runtime_session_id: agent_session_id.clone(),
            workspace_path: req.workspace_path.clone(),
            status: if transport == RuntimeTransportKind::Acp {
                RuntimeSessionStatus::Starting
            } else {
                RuntimeSessionStatus::Active
            },
            capabilities,
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
                    acp_controller,
                },
            );
        };

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

    pub fn ensure_session(&self, req: EnsureAgentRuntimeSession) -> Result<AgentSessionHandle> {
        if req.sessio_runtime_session_id.trim().is_empty() {
            bail!("runtime session id is required");
        }
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

        {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            if let Some(existing) = sessions.get(&req.sessio_runtime_session_id) {
                return Ok(existing.handle.clone());
            }
        }

        let runtime_config = runtime_config(req.agent);
        let transport = runtime_config
            .as_ref()
            .map(acp_transport::transport_from_config)
            .unwrap_or(RuntimeTransportKind::Fake);
        let agent_session_id = req
            .agent_runtime_session_id
            .clone()
            .unwrap_or_else(|| req.sessio_runtime_session_id.clone());
        let capabilities = RuntimeCapabilitySet::fake();
        let mut acp_controller = None;
        if transport == RuntimeTransportKind::Acp {
            let start = req
                .agent_runtime_session_id
                .as_ref()
                .filter(|id| !id.trim().is_empty())
                .map(|agent_session_id| acp_transport::AcpSessionStart::Load {
                    agent_session_id: agent_session_id.clone(),
                })
                .unwrap_or(acp_transport::AcpSessionStart::New);
            acp_controller = Some(acp_transport::spawn_session(
                self.clone(),
                req.sessio_runtime_session_id.clone(),
                req.agent,
                req.workspace_path.clone(),
                runtime_config
                    .as_ref()
                    .map(|config| acp_transport::command_from_config(req.agent, config))
                    .unwrap_or_else(|| {
                        acp_transport::command_from_options(req.agent, &Default::default())
                    }),
                start,
            ));
        }

        let handle = AgentSessionHandle {
            sessio_runtime_session_id: req.sessio_runtime_session_id.clone(),
            agent: req.agent,
            transport,
            agent_runtime_session_id: agent_session_id,
            workspace_path: req.workspace_path,
            status: if transport == RuntimeTransportKind::Acp {
                RuntimeSessionStatus::Starting
            } else {
                RuntimeSessionStatus::Idle
            },
            capabilities,
        };

        {
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
                    acp_controller,
                },
            );
        }

        self.emit(AgentRuntimeEventPayload::SessionStarted {
            agent: handle.agent,
            sessio_runtime_session_id: handle.sessio_runtime_session_id.clone(),
            agent_runtime_session_id: handle.agent_runtime_session_id.clone(),
            transport: handle.transport,
            workspace_path: handle.workspace_path.clone(),
            capabilities: handle.capabilities.clone(),
        })?;

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
        let acp_controller = {
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
            state.acp_controller.clone()
        };

        self.emit(AgentRuntimeEventPayload::TurnStarted {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.clone(),
        })?;

        if let Some(controller) = acp_controller {
            if let Err(error) = controller.send_prompt(turn_id.clone(), input) {
                self.fail_turn(
                    sessio_runtime_session_id,
                    &turn_id,
                    RuntimeError::new("acp_send_error", error.to_string()),
                )?;
                return Err(error);
            }
        } else {
            fake::spawn_stream(
                self.clone(),
                sessio_runtime_session_id.to_string(),
                turn_id.clone(),
                input,
                cancel_token,
            );
        }

        Ok(AgentTurnHandle {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id,
            status: RuntimeTurnStatus::Streaming,
        })
    }

    pub fn cancel_turn(&self, sessio_runtime_session_id: &str, turn_id: &str) -> Result<()> {
        let acp_controller = {
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
                    let acp_controller = state.acp_controller.clone();
                    state.active_turn_id = None;
                    if let Some(token) = state.turn_cancellations.get(turn_id) {
                        token.store(true, Ordering::Relaxed);
                    }
                    for (_, sender) in state.permission_waiters.drain() {
                        let _ = sender.send(RuntimePermissionDecision::Cancelled);
                    }
                    state.handle.status = RuntimeSessionStatus::Idle;
                    acp_controller
                }
                Some(active) => bail!("active turn is {active}, not {turn_id}"),
                None => bail!("runtime session has no active turn"),
            }
        };

        if let Some(controller) = acp_controller {
            controller.cancel_turn(turn_id.to_string())?;
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
        log::info!(
            "[sessio-runtime:permission-response:user] session={} request={} approved={}",
            sessio_runtime_session_id,
            request_id,
            approved
        );
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
            .send(RuntimePermissionDecision::Selected { approved })
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
        let acp_request = fake_permission_request(
            sessio_runtime_session_id,
            request_id.to_string(),
            tool_name.to_string(),
            input,
        );
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
        log::info!(
            "[sessio-runtime:fake-acp:permission-request] {:?}",
            acp_request
        );
        self.emit(convert_permission_request(
            &acp_request,
            sessio_runtime_session_id,
            turn_id,
            request_id,
        )?)?;
        let decision = receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("permission response channel closed"))?;
        let approved = match decision {
            RuntimePermissionDecision::Selected { approved } => approved,
            RuntimePermissionDecision::Cancelled => false,
        };
        let acp_response = permission_response_from_decision(&acp_request, approved);
        log::info!(
            "[sessio-runtime:fake-acp:permission-response] {:?}",
            acp_response
        );
        self.emit(permission_resolved_event(
            sessio_runtime_session_id,
            turn_id,
            request_id,
            approved,
        ))?;
        Ok(approved)
    }

    pub(crate) fn request_permission_from_acp(
        &self,
        request: &RequestPermissionRequest,
        sessio_runtime_session_id: &str,
        turn_id: &str,
        request_id: &str,
    ) -> Result<RuntimePermissionDecision> {
        log::info!(
            "[sessio-runtime:permission-request:acp] session={} turn={} request={} tool={}",
            sessio_runtime_session_id,
            turn_id,
            request_id,
            request.tool_call.fields.title.as_deref().unwrap_or("tool")
        );
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
        self.emit(convert_permission_request(
            request,
            sessio_runtime_session_id,
            turn_id,
            request_id,
        )?)?;
        let decision = receiver
            .recv()
            .map_err(|_| anyhow::anyhow!("permission response channel closed"))?;
        log::info!(
            "[sessio-runtime:permission-decision:acp] session={} request={} approved={:?}",
            sessio_runtime_session_id,
            request_id,
            match decision {
                RuntimePermissionDecision::Selected { approved } => Some(approved),
                RuntimePermissionDecision::Cancelled => None,
            }
        );
        if let RuntimePermissionDecision::Selected { approved } = decision {
            self.emit(permission_resolved_event(
                sessio_runtime_session_id,
                turn_id,
                request_id,
                approved,
            ))?;
            Ok(RuntimePermissionDecision::Selected { approved })
        } else {
            self.emit(permission_resolved_event(
                sessio_runtime_session_id,
                turn_id,
                request_id,
                false,
            ))?;
            Ok(RuntimePermissionDecision::Cancelled)
        }
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

    pub(crate) fn active_turn_id(&self, sessio_runtime_session_id: &str) -> Option<String> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)?
                .active_turn_id
                .clone()
        })
    }

    pub(crate) fn cancel_turn_if_active(
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
            if state.active_turn_id.as_deref() == Some(turn_id) {
                state.active_turn_id = None;
                state.turn_cancellations.remove(turn_id);
                for (_, sender) in state.permission_waiters.drain() {
                    let _ = sender.send(RuntimePermissionDecision::Cancelled);
                }
                state.handle.status = RuntimeSessionStatus::Idle;
                true
            } else {
                false
            }
        };
        if should_emit {
            self.emit(AgentRuntimeEventPayload::TurnCancelled {
                sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                turn_id: turn_id.to_string(),
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

    pub(crate) fn mark_session_ready(
        &self,
        sessio_runtime_session_id: &str,
        agent_runtime_session_id: String,
        capabilities: RuntimeCapabilitySet,
    ) -> Result<()> {
        let handle = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let state = sessions
                .get_mut(sessio_runtime_session_id)
                .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?;
            state.handle.agent_runtime_session_id = agent_runtime_session_id;
            state.handle.capabilities = capabilities;
            if state.handle.status == RuntimeSessionStatus::Starting {
                state.handle.status = RuntimeSessionStatus::Idle;
            }
            state.handle.clone()
        };

        self.emit(AgentRuntimeEventPayload::SessionStarted {
            agent: handle.agent,
            sessio_runtime_session_id: handle.sessio_runtime_session_id,
            agent_runtime_session_id: handle.agent_runtime_session_id,
            transport: handle.transport,
            workspace_path: handle.workspace_path,
            capabilities: handle.capabilities,
        })
    }

    pub(crate) fn fail_session_start(
        &self,
        sessio_runtime_session_id: &str,
        message: String,
    ) -> Result<()> {
        let active_turn_id = {
            let mut sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let Some(state) = sessions.get_mut(sessio_runtime_session_id) else {
                return Ok(());
            };
            state.handle.status = RuntimeSessionStatus::Errored;
            for (_, sender) in state.permission_waiters.drain() {
                let _ = sender.send(RuntimePermissionDecision::Cancelled);
            }
            state.active_turn_id.clone()
        };
        if let Some(turn_id) = active_turn_id {
            self.fail_turn(
                sessio_runtime_session_id,
                &turn_id,
                RuntimeError::new("acp_runtime_error", message),
            )?;
        } else {
            self.emit(super::acp::turn_error_event(
                sessio_runtime_session_id,
                "startup",
                "acp_runtime_error",
                message,
            ))?;
        }
        Ok(())
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

fn runtime_config(agent: Agent) -> Option<config::AgentRuntimeConfig> {
    config::load_config()
        .ok()
        .map(|config| config.agents.runtime.get(agent).clone())
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
