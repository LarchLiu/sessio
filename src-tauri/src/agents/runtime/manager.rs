use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use tauri::{AppHandle, Emitter, Manager};

use agent_client_protocol::schema::RequestPermissionRequest;

use super::acp::{
    convert_permission_request, fake_permission_request, permission_resolved_event,
    permission_response_from_decision,
};
use super::acp_transport::{self, AcpSessionController};
use super::fake;
use super::pi_rpc_transport::{self, PiRpcSessionController};
use super::types::{
    AgentInput, AgentRuntimeEvent, AgentRuntimeEventPayload, AgentRuntimeSessionConfig,
    AgentSessionConfigChange, AgentSessionHandle, AgentTurnHandle, EnsureAgentRuntimeSession,
    RuntimeCapabilitySet, RuntimeError, RuntimeMetadata, RuntimeSessionStatus, RuntimeStatus,
    RuntimeTransportKind, RuntimeTurnStatus, StartAgentSession,
};
use crate::models::Agent;
use crate::store::{RuntimeAgentSessionConfigRecord, SessionStore};
use crate::turns::{
    apply_optimistic_user_message, apply_runtime_event_to_state, AcpCanonicalSessionState,
    LiveRuntimeTurnSnapshotEvent, RuntimeTurnState,
};

#[derive(Clone)]
pub struct RuntimeManager {
    inner: Arc<RuntimeManagerInner>,
}

struct RuntimeManagerInner {
    app: AppHandle,
    sequence: AtomicU64,
    id_counter: AtomicU64,
    sessions: Mutex<HashMap<String, RuntimeSessionState>>,
    event_listeners: Mutex<Vec<RuntimeEventListener>>,
    snapshot_queue: Mutex<HashMap<String, PendingRuntimeSnapshot>>,
}

#[derive(Debug, Clone)]
struct RuntimeSessionState {
    handle: AgentSessionHandle,
    active_turn_id: Option<String>,
    turn_state: RuntimeTurnState,
    metadata: RuntimeMetadata,
    startup_error: Option<String>,
    turn_cancellations: HashMap<String, Arc<AtomicBool>>,
    permission_waiters: HashMap<String, mpsc::Sender<RuntimePermissionDecision>>,
    acp_controller: Option<AcpSessionController>,
    pi_rpc_controller: Option<PiRpcSessionController>,
}

type RuntimeEventFilter = Arc<dyn Fn(&AgentRuntimeEventPayload) -> bool + Send + Sync>;

struct RuntimeEventListener {
    sender: mpsc::Sender<AgentRuntimeEvent>,
    filter: RuntimeEventFilter,
}

#[derive(Debug, Clone, Copy)]
struct PendingRuntimeSnapshot {
    sequence: u64,
    timestamp: i64,
    scheduled: bool,
}

pub(crate) enum RuntimePermissionDecision {
    Selected { option_id: String },
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeCleanupReport {
    pub session_existed: bool,
    pub cancelled_turn_id: Option<String>,
    pub cancel_error: Option<String>,
    pub dispose_error: Option<String>,
    pub timed_out: bool,
    pub force_detached: bool,
}

const LIVE_RUNTIME_SNAPSHOT_THROTTLE_MS: u64 = 160;
const ENABLE_LIVE_RUNTIME_SNAPSHOTS: bool = true;
const RUNTIME_INPUT_DISPLAY_TEXT_OPTION: &str = "displayText";
const RUNTIME_INPUT_SUPPRESS_OPTIMISTIC_OPTION: &str = "suppressOptimisticUserMessage";

impl RuntimeManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            inner: Arc::new(RuntimeManagerInner {
                app,
                sequence: AtomicU64::new(1),
                id_counter: AtomicU64::new(1),
                sessions: Mutex::new(HashMap::new()),
                event_listeners: Mutex::new(Vec::new()),
                snapshot_queue: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn subscribe_events(&self) -> Result<mpsc::Receiver<AgentRuntimeEvent>> {
        self.subscribe_events_filtered(|_| true)
    }

    pub fn subscribe_events_filtered<F>(
        &self,
        filter: F,
    ) -> Result<mpsc::Receiver<AgentRuntimeEvent>>
    where
        F: Fn(&AgentRuntimeEventPayload) -> bool + Send + Sync + 'static,
    {
        let (sender, receiver) = mpsc::channel();
        self.inner
            .event_listeners
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime event listeners lock poisoned"))?
            .push(RuntimeEventListener {
                sender,
                filter: Arc::new(filter),
            });
        Ok(receiver)
    }

    pub(crate) fn event_session_metadata_has(
        &self,
        payload: &AgentRuntimeEventPayload,
        key: &str,
    ) -> bool {
        let session_id = event_session_id(payload);
        let Some(session_id) = session_id else {
            return false;
        };
        self.inner
            .sessions
            .lock()
            .ok()
            .and_then(|sessions| {
                sessions
                    .get(session_id)
                    .map(|state| state.metadata.contains_key(key))
            })
            .unwrap_or(false)
    }

    pub fn status(&self, agent: Agent) -> RuntimeStatus {
        let transport = self.configured_transport(agent);
        RuntimeStatus {
            agent,
            transport,
            available: true,
            status: RuntimeSessionStatus::Idle,
            capabilities: runtime_capabilities_for_transport(transport),
            error: None,
            metadata: Default::default(),
        }
    }

    pub fn configured_transport(&self, agent: Agent) -> RuntimeTransportKind {
        if agent == Agent::Pi {
            RuntimeTransportKind::PiRpc
        } else {
            RuntimeTransportKind::Acp
        }
    }

    pub fn configured_session_command(&self, agent: Agent) -> String {
        if self.configured_transport(agent) == RuntimeTransportKind::PiRpc {
            pi_rpc_transport::command_from_options(&Default::default())
        } else {
            acp_transport::command_from_options(agent, &Default::default())
        }
    }

    fn requested_transport(&self, agent: Agent, options: &RuntimeMetadata) -> RuntimeTransportKind {
        if agent == Agent::Pi {
            return RuntimeTransportKind::PiRpc;
        }
        if options.contains_key("transport") {
            acp_transport::transport_requested(options)
        } else {
            self.configured_transport(agent)
        }
    }

    pub fn status_for_session(
        &self,
        sessio_runtime_session_id: &str,
    ) -> Option<RuntimeSessionStatus> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)
                .map(|s| s.handle.status)
        })
    }

    pub fn agent_runtime_session_id_for_session(
        &self,
        sessio_runtime_session_id: &str,
    ) -> Option<String> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)
                .map(|s| s.handle.agent_runtime_session_id.clone())
        })
    }

    pub fn capabilities_for_session(
        &self,
        sessio_runtime_session_id: &str,
    ) -> Option<RuntimeCapabilitySet> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)
                .map(|s| s.handle.capabilities.clone())
        })
    }

    pub fn active_turn_id(&self, sessio_runtime_session_id: &str) -> Option<String> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)
                .and_then(|state| state.active_turn_id.clone())
        })
    }

    pub fn latest_turn_status(&self, sessio_runtime_session_id: &str) -> Option<String> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions.get(sessio_runtime_session_id).and_then(|state| {
                state
                    .turn_state
                    .turns
                    .last()
                    .map(|turn| turn.status.clone())
            })
        })
    }

    pub fn session_transcript_text(&self, sessio_runtime_session_id: &str) -> Option<String> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)
                .map(|state| session_turns_text(&state.turn_state.turns))
        })
    }

    pub fn has_active_runtime_turn(&self) -> bool {
        self.inner
            .sessions
            .lock()
            .map(|sessions| {
                sessions.values().any(|session| {
                    transport_requires_startup(session.handle.transport)
                        && session.active_turn_id.is_some()
                })
            })
            .unwrap_or(false)
    }

    pub fn dispose_session_silent(&self, sessio_runtime_session_id: &str) -> Result<()> {
        let should_emit = self
            .inner
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?
            .contains_key(sessio_runtime_session_id);

        let emit_error = if should_emit {
            self.emit(AgentRuntimeEventPayload::SessionEnded {
                sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            })
            .err()
        } else {
            None
        };

        let remove_error = self
            .inner
            .sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?
            .remove(sessio_runtime_session_id)
            .map(|_| ())
            .ok_or_else(|| anyhow::anyhow!("runtime session was already removed"))
            .err();

        if let Some(error) = emit_error {
            return Err(error);
        }
        if should_emit {
            if let Some(error) = remove_error {
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn wait_for_session_startup(
        &self,
        sessio_runtime_session_id: &str,
        timeout: Duration,
    ) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            let (status, startup_error) = {
                let sessions = self
                    .inner
                    .sessions
                    .lock()
                    .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
                let state = sessions.get(sessio_runtime_session_id).with_context(|| {
                    format!("unknown runtime session: {sessio_runtime_session_id}")
                })?;
                (state.handle.status, state.startup_error.clone())
            };

            match status {
                RuntimeSessionStatus::Starting => {
                    let now = Instant::now();
                    if now >= deadline {
                        bail!(
                            "runtime session startup timed out after {}ms: {}",
                            timeout.as_millis(),
                            sessio_runtime_session_id
                        );
                    }
                    let remaining = deadline.saturating_duration_since(now);
                    std::thread::sleep(remaining.min(Duration::from_millis(50)));
                }
                RuntimeSessionStatus::Errored
                | RuntimeSessionStatus::Disconnected
                | RuntimeSessionStatus::Ended
                | RuntimeSessionStatus::Completed => {
                    let detail = startup_error
                        .as_deref()
                        .unwrap_or("runtime session is not active");
                    bail!(
                        "runtime session {} failed during startup while {:?}: {}",
                        sessio_runtime_session_id,
                        status,
                        detail
                    );
                }
                RuntimeSessionStatus::Active
                | RuntimeSessionStatus::Idle
                | RuntimeSessionStatus::Cancelling => return Ok(()),
            }
        }
    }

    pub fn cleanup_session_bounded(
        &self,
        sessio_runtime_session_id: &str,
        timeout: Duration,
    ) -> RuntimeCleanupReport {
        let started = Instant::now();
        let mut report = RuntimeCleanupReport::default();
        let active_turn_id = match self.inner.sessions.lock() {
            Ok(sessions) => match sessions.get(sessio_runtime_session_id) {
                Some(state) => {
                    report.session_existed = true;
                    state.active_turn_id.clone()
                }
                None => None,
            },
            Err(_) => {
                report.dispose_error = Some("runtime session lock poisoned".to_string());
                report.timed_out = started.elapsed() >= timeout;
                return report;
            }
        };

        if let Some(turn_id) = active_turn_id {
            if let Err(error) = self.cancel_turn(sessio_runtime_session_id, &turn_id) {
                report.cancel_error = Some(error.to_string());
            }
            report.cancelled_turn_id = Some(turn_id);
            report.force_detached = true;
        }

        if let Err(error) = self.dispose_session_silent(sessio_runtime_session_id) {
            report.dispose_error = Some(error.to_string());
        }
        report.timed_out = started.elapsed() >= timeout;
        report
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

        let transport = self.requested_transport(req.agent, &req.options);
        let runtime_config = session_config_from_options(req.agent, &req.options);
        let id = self.next_id("runtime");
        let agent_session_id = self.next_id("fake-agent-session");
        let capabilities = runtime_capabilities_for_transport(transport);
        let mut acp_controller = None;
        let mut pi_rpc_controller = None;
        let mut pi_rpc_worker = None;
        match transport {
            RuntimeTransportKind::Acp => {
                let command = acp_transport::command_from_options(req.agent, &req.options);
                let start = match (&req.source_session_id, req.source_agent) {
                    (Some(source_session_id), Some(source_agent)) if source_agent == req.agent => {
                        acp_transport::AcpSessionStart::Fork {
                            source_session_id: source_session_id.clone(),
                        }
                    }
                    _ => acp_transport::AcpSessionStart::New,
                };
                acp_controller = Some(acp_transport::spawn_session(
                    self.clone(),
                    id.clone(),
                    req.agent,
                    req.workspace_path.clone(),
                    command,
                    Some(runtime_config),
                    start,
                ));
            }
            RuntimeTransportKind::PiRpc => {
                let start = match (&req.source_session_id, req.source_agent) {
                    (Some(source_session_id), Some(source_agent)) if source_agent == req.agent => {
                        pi_rpc_transport::PiRpcSessionStart::Fork {
                            source_session_id: source_session_id.clone(),
                        }
                    }
                    _ => pi_rpc_transport::PiRpcSessionStart::New,
                };
                let (controller, worker) = pi_rpc_transport::prepare_session(
                    self.clone(),
                    id.clone(),
                    pi_rpc_transport::PiRpcSessionSpec {
                        agent: req.agent,
                        workspace_path: req.workspace_path.clone(),
                        command: pi_rpc_transport::command_from_options(&req.options),
                        runtime_config: Some(runtime_config),
                        start,
                    },
                );
                pi_rpc_controller = Some(controller);
                pi_rpc_worker = Some(worker);
            }
            RuntimeTransportKind::Fake => {}
        }
        let handle = AgentSessionHandle {
            sessio_runtime_session_id: id.clone(),
            agent: req.agent,
            transport,
            agent_runtime_session_id: agent_session_id.clone(),
            workspace_path: req.workspace_path.clone(),
            status: if transport_requires_startup(transport) {
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
                    turn_state: RuntimeTurnState::new(
                        id.clone(),
                        handle.agent,
                        handle.agent_runtime_session_id.clone(),
                        handle.transport,
                        handle.workspace_path.clone(),
                        handle.capabilities.clone(),
                    ),
                    metadata: req.options.clone(),
                    startup_error: None,
                    turn_cancellations: HashMap::new(),
                    permission_waiters: HashMap::new(),
                    acp_controller,
                    pi_rpc_controller,
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
            metadata: req.options.clone(),
        })?;

        if let Some(worker) = pi_rpc_worker {
            worker.start();
        }

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

        let transport = self.requested_transport(req.agent, &req.options);
        let runtime_config = session_config_from_options(req.agent, &req.options);
        let agent_session_id = req
            .agent_runtime_session_id
            .clone()
            .unwrap_or_else(|| req.sessio_runtime_session_id.clone());
        let capabilities = runtime_capabilities_for_transport(transport);
        let mut acp_controller = None;
        let mut pi_rpc_controller = None;
        let mut pi_rpc_worker = None;
        match transport {
            RuntimeTransportKind::Acp => {
                let start = req
                    .agent_runtime_session_id
                    .as_ref()
                    .filter(|id| !id.trim().is_empty())
                    .map(|agent_session_id| {
                        if req.source_agent == Some(req.agent) {
                            acp_transport::AcpSessionStart::Resume {
                                agent_session_id: agent_session_id.clone(),
                            }
                        } else {
                            acp_transport::AcpSessionStart::Load {
                                agent_session_id: agent_session_id.clone(),
                            }
                        }
                    })
                    .unwrap_or(acp_transport::AcpSessionStart::New);
                acp_controller = Some(acp_transport::spawn_session(
                    self.clone(),
                    req.sessio_runtime_session_id.clone(),
                    req.agent,
                    req.workspace_path.clone(),
                    acp_transport::command_from_options(req.agent, &req.options),
                    Some(runtime_config),
                    start,
                ));
            }
            RuntimeTransportKind::PiRpc => {
                let start = req
                    .agent_runtime_session_id
                    .as_ref()
                    .filter(|id| !id.trim().is_empty())
                    .map(|agent_session_id| {
                        if req.source_agent == Some(req.agent) {
                            pi_rpc_transport::PiRpcSessionStart::Resume {
                                agent_session_id: agent_session_id.clone(),
                            }
                        } else {
                            pi_rpc_transport::PiRpcSessionStart::Load {
                                agent_session_id: agent_session_id.clone(),
                            }
                        }
                    })
                    .unwrap_or(pi_rpc_transport::PiRpcSessionStart::New);
                let (controller, worker) = pi_rpc_transport::prepare_session(
                    self.clone(),
                    req.sessio_runtime_session_id.clone(),
                    pi_rpc_transport::PiRpcSessionSpec {
                        agent: req.agent,
                        workspace_path: req.workspace_path.clone(),
                        command: pi_rpc_transport::command_from_options(&req.options),
                        runtime_config: Some(runtime_config),
                        start,
                    },
                );
                pi_rpc_controller = Some(controller);
                pi_rpc_worker = Some(worker);
            }
            RuntimeTransportKind::Fake => {}
        }

        let handle = AgentSessionHandle {
            sessio_runtime_session_id: req.sessio_runtime_session_id.clone(),
            agent: req.agent,
            transport,
            agent_runtime_session_id: agent_session_id,
            workspace_path: req.workspace_path,
            status: if transport_requires_startup(transport) {
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
                    turn_state: RuntimeTurnState::new(
                        handle.sessio_runtime_session_id.clone(),
                        handle.agent,
                        handle.agent_runtime_session_id.clone(),
                        handle.transport,
                        handle.workspace_path.clone(),
                        handle.capabilities.clone(),
                    ),
                    metadata: req.options.clone(),
                    startup_error: None,
                    turn_cancellations: HashMap::new(),
                    permission_waiters: HashMap::new(),
                    acp_controller,
                    pi_rpc_controller,
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
            metadata: req.options.clone(),
        })?;

        if let Some(worker) = pi_rpc_worker {
            worker.start();
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
        let (acp_controller, pi_rpc_controller) = {
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
            if matches!(
                state.handle.status,
                RuntimeSessionStatus::Errored
                    | RuntimeSessionStatus::Disconnected
                    | RuntimeSessionStatus::Ended
                    | RuntimeSessionStatus::Completed
            ) {
                let detail = state
                    .startup_error
                    .as_deref()
                    .unwrap_or("runtime session is not active");
                bail!(
                    "runtime session {} cannot receive input while {:?}: {}",
                    sessio_runtime_session_id,
                    state.handle.status,
                    detail
                );
            }
            state.active_turn_id = Some(turn_id.clone());
            if !input_option_bool(&input.options, RUNTIME_INPUT_SUPPRESS_OPTIMISTIC_OPTION) {
                let display_text =
                    input_option_string(&input.options, RUNTIME_INPUT_DISPLAY_TEXT_OPTION)
                        .unwrap_or(input.text.as_str());
                apply_optimistic_user_message(
                    &mut state.turn_state,
                    &turn_id,
                    display_text,
                    &input.attachments,
                    now_ms(),
                );
            }
            state
                .turn_cancellations
                .insert(turn_id.clone(), cancel_token.clone());
            state.handle.status = RuntimeSessionStatus::Active;
            (
                state.acp_controller.clone(),
                state.pi_rpc_controller.clone(),
            )
        };

        self.emit(AgentRuntimeEventPayload::TurnStarted {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.clone(),
        })?;

        if let Some(controller) = acp_controller {
            if let Err(error) = controller.send_prompt(turn_id.clone(), input) {
                let (code, message) = self
                    .session_startup_error(sessio_runtime_session_id)
                    .map(|message| ("acp_runtime_error", message))
                    .unwrap_or_else(|| ("acp_send_error", error.to_string()));
                self.fail_turn(
                    sessio_runtime_session_id,
                    &turn_id,
                    RuntimeError::new(code, message.clone()),
                )?;
                return Err(anyhow::anyhow!(message));
            }
        } else if let Some(controller) = pi_rpc_controller {
            if let Err(error) = controller.send_prompt(turn_id.clone(), input) {
                let (code, message) = self
                    .session_startup_error(sessio_runtime_session_id)
                    .map(|message| ("pi_rpc_runtime_error", message))
                    .unwrap_or_else(|| ("pi_rpc_send_error", error.to_string()));
                self.fail_turn(
                    sessio_runtime_session_id,
                    &turn_id,
                    RuntimeError::new(code, message.clone()),
                )?;
                return Err(anyhow::anyhow!(message));
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
        let (acp_controller, pi_rpc_controller) = {
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
                    let pi_rpc_controller = state.pi_rpc_controller.clone();
                    state.active_turn_id = None;
                    if let Some(token) = state.turn_cancellations.get(turn_id) {
                        token.store(true, Ordering::Relaxed);
                    }
                    for (_, sender) in state.permission_waiters.drain() {
                        let _ = sender.send(RuntimePermissionDecision::Cancelled);
                    }
                    state.handle.status = RuntimeSessionStatus::Idle;
                    (acp_controller, pi_rpc_controller)
                }
                Some(active) => bail!("active turn is {active}, not {turn_id}"),
                None => bail!("runtime session has no active turn"),
            }
        };

        if let Some(controller) = acp_controller {
            controller.cancel_turn(turn_id.to_string())?;
        }
        if let Some(controller) = pi_rpc_controller {
            controller.cancel_turn(turn_id.to_string())?;
        }

        self.emit(AgentRuntimeEventPayload::TurnCancelled {
            sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
            turn_id: turn_id.to_string(),
        })
    }

    pub fn set_config_option(
        &self,
        sessio_runtime_session_id: &str,
        change: AgentSessionConfigChange,
    ) -> Result<()> {
        if change.config_id.trim().is_empty() {
            bail!("config_id is required");
        }
        let (acp_controller, pi_rpc_controller) = {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let state = sessions
                .get(sessio_runtime_session_id)
                .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?;
            (
                state.acp_controller.clone(),
                state.pi_rpc_controller.clone(),
            )
        };
        if let Some(controller) = acp_controller {
            controller.set_config_option(change.config_id, change.value)
        } else if let Some(controller) = pi_rpc_controller {
            controller.set_config_option(change.config_id, change.value)
        } else {
            bail!("session config updates require a runtime session controller")
        }
    }

    pub fn respond_permission(
        &self,
        sessio_runtime_session_id: &str,
        request_id: &str,
        option_id: String,
    ) -> Result<()> {
        log::info!(
            "[sessio-runtime:permission-response:user] session={} request={} option={}",
            sessio_runtime_session_id,
            request_id,
            option_id
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
            .send(RuntimePermissionDecision::Selected { option_id })
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
        let option_id = match decision {
            RuntimePermissionDecision::Selected { option_id } => Some(option_id),
            RuntimePermissionDecision::Cancelled => None,
        };
        let approved = option_id
            .as_ref()
            .map(|id| id.to_ascii_lowercase().starts_with("allow"))
            .unwrap_or(false);
        let acp_response = option_id
            .as_deref()
            .map(|id| permission_response_from_decision(&acp_request, id))
            .unwrap_or_else(|| {
                agent_client_protocol::schema::RequestPermissionResponse::new(
                    agent_client_protocol::schema::RequestPermissionOutcome::Cancelled,
                )
            });
        log::info!(
            "[sessio-runtime:fake-acp:permission-response] {:?}",
            acp_response
        );
        self.emit(permission_resolved_event(
            sessio_runtime_session_id,
            turn_id,
            request_id,
            option_id,
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
                RuntimePermissionDecision::Selected { ref option_id } => Some(option_id.as_str()),
                RuntimePermissionDecision::Cancelled => None,
            }
        );
        if let RuntimePermissionDecision::Selected { option_id } = decision {
            self.emit(permission_resolved_event(
                sessio_runtime_session_id,
                turn_id,
                request_id,
                Some(option_id.clone()),
            ))?;
            Ok(RuntimePermissionDecision::Selected { option_id })
        } else {
            self.emit(permission_resolved_event(
                sessio_runtime_session_id,
                turn_id,
                request_id,
                None,
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
        let (handle, metadata) = {
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
            (state.handle.clone(), state.metadata.clone())
        };

        self.emit(AgentRuntimeEventPayload::SessionStarted {
            agent: handle.agent,
            sessio_runtime_session_id: handle.sessio_runtime_session_id,
            agent_runtime_session_id: handle.agent_runtime_session_id,
            transport: handle.transport,
            workspace_path: handle.workspace_path,
            capabilities: handle.capabilities,
            metadata,
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
            state.startup_error = Some(message.clone());
            state.acp_controller = None;
            state.pi_rpc_controller = None;
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
        log_runtime_event(&payload);
        let timestamp = now_ms();
        let sequence = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
        let should_flush_snapshot = should_emit_snapshot_immediately(&payload);
        let event = AgentRuntimeEvent {
            sequence,
            timestamp,
            payload,
        };
        let should_emit_runtime_event = should_emit_runtime_event_to_webview(&event.payload);
        let should_emit_turn_snapshot = should_emit_turn_snapshot_for_event(&event.payload);
        let snapshot_session_id = if ENABLE_LIVE_RUNTIME_SNAPSHOTS {
            self.apply_event_to_turn_state(&event)
        } else {
            None
        };
        if let Some(session_id) = snapshot_session_id.as_deref() {
            let _ = self.persist_runtime_session_config_if_needed(session_id);
        }
        self.notify_event_listeners(&event);
        if should_emit_runtime_event {
            self.inner
                .app
                .emit("agent-runtime-event", event)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        }
        if let Some(session_id) = snapshot_session_id {
            if should_emit_turn_snapshot {
                self.queue_turn_snapshot(session_id, timestamp, sequence, should_flush_snapshot)?;
            } else {
                self.clear_queued_turn_snapshot(&session_id)?;
            }
        }
        Ok(())
    }

    fn session_startup_error(&self, sessio_runtime_session_id: &str) -> Option<String> {
        self.inner.sessions.lock().ok().and_then(|sessions| {
            sessions
                .get(sessio_runtime_session_id)
                .and_then(|state| state.startup_error.clone())
        })
    }

    fn notify_event_listeners(&self, event: &AgentRuntimeEvent) {
        let Ok(mut listeners) = self.inner.event_listeners.lock() else {
            return;
        };
        listeners.retain(|listener| {
            if !(listener.filter)(&event.payload) {
                return true;
            }
            listener.sender.send(event.clone()).is_ok()
        });
    }

    fn apply_event_to_turn_state(&self, event: &AgentRuntimeEvent) -> Option<String> {
        let sessio_runtime_session_id = event_session_id(&event.payload)?;
        {
            let mut sessions = self.inner.sessions.lock().ok()?;
            let state = sessions.get_mut(sessio_runtime_session_id)?;
            apply_runtime_event_to_state(&mut state.turn_state, &event.payload, event.timestamp);
        }
        Some(sessio_runtime_session_id.to_string())
    }

    fn persist_runtime_session_config_if_needed(
        &self,
        sessio_runtime_session_id: &str,
    ) -> Result<()> {
        let Some(store) = self.inner.app.try_state::<Arc<dyn SessionStore>>() else {
            return Ok(());
        };
        let (agent, session_state) = {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let Some(state) = sessions.get(sessio_runtime_session_id) else {
                return Ok(());
            };
            (state.handle.agent, state.turn_state.session_state.clone())
        };
        if session_state.available_commands.is_empty() && session_state.config_options.is_empty() {
            return Ok(());
        }
        let Some(capability) = store.get_runtime_agent_capability(agent)? else {
            return Ok(());
        };
        let Some(adapter_version) = capability.version.as_deref().map(str::trim) else {
            return Ok(());
        };
        if adapter_version.is_empty() {
            return Ok(());
        }
        let current = store.get_runtime_agent_session_config(agent, adapter_version)?;
        let Some(record) = build_runtime_session_config_record(
            agent,
            adapter_version,
            &session_state,
            current.as_ref(),
            now_ms(),
        )?
        else {
            return Ok(());
        };
        store.upsert_runtime_agent_session_config(&record)
    }

    fn queue_turn_snapshot(
        &self,
        session_id: String,
        timestamp: i64,
        sequence: u64,
        immediate: bool,
    ) -> Result<()> {
        if immediate {
            if let Ok(mut queue) = self.inner.snapshot_queue.lock() {
                queue.remove(&session_id);
            }
            return self.emit_turn_snapshot(&session_id, sequence, timestamp);
        }

        let should_schedule = {
            let mut queue = self
                .inner
                .snapshot_queue
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime snapshot queue lock poisoned"))?;
            let pending = queue
                .entry(session_id.clone())
                .or_insert(PendingRuntimeSnapshot {
                    sequence,
                    timestamp,
                    scheduled: false,
                });
            pending.sequence = sequence;
            pending.timestamp = timestamp;
            if pending.scheduled {
                false
            } else {
                pending.scheduled = true;
                true
            }
        };

        if should_schedule {
            let manager = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(LIVE_RUNTIME_SNAPSHOT_THROTTLE_MS));
                let _ = manager.flush_queued_turn_snapshot(&session_id);
            });
        }
        Ok(())
    }

    fn flush_queued_turn_snapshot(&self, session_id: &str) -> Result<()> {
        let pending = {
            let mut queue = self
                .inner
                .snapshot_queue
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime snapshot queue lock poisoned"))?;
            queue.remove(session_id)
        };
        if let Some(pending) = pending {
            self.emit_turn_snapshot(session_id, pending.sequence, pending.timestamp)?;
        }
        Ok(())
    }

    fn clear_queued_turn_snapshot(&self, session_id: &str) -> Result<()> {
        let mut queue = self
            .inner
            .snapshot_queue
            .lock()
            .map_err(|_| anyhow::anyhow!("runtime snapshot queue lock poisoned"))?;
        queue.remove(session_id);
        Ok(())
    }

    fn emit_turn_snapshot(&self, session_id: &str, sequence: u64, timestamp: i64) -> Result<()> {
        let snapshot = {
            let sessions = self
                .inner
                .sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("runtime session lock poisoned"))?;
            let Some(state) = sessions.get(session_id) else {
                return Ok(());
            };
            state.turn_state.snapshot()
        };
        self.inner
            .app
            .emit(
                "agent-runtime-turn-snapshot",
                LiveRuntimeTurnSnapshotEvent {
                    sequence,
                    timestamp,
                    session: snapshot,
                },
            )
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(())
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

fn runtime_capabilities_for_transport(transport: RuntimeTransportKind) -> RuntimeCapabilitySet {
    match transport {
        RuntimeTransportKind::Acp => RuntimeCapabilitySet::fake(),
        RuntimeTransportKind::PiRpc => pi_rpc_transport::runtime_capabilities(),
        RuntimeTransportKind::Fake => RuntimeCapabilitySet::fake(),
    }
}

fn transport_requires_startup(transport: RuntimeTransportKind) -> bool {
    matches!(
        transport,
        RuntimeTransportKind::Acp | RuntimeTransportKind::PiRpc
    )
}

fn log_runtime_event(payload: &AgentRuntimeEventPayload) {
    if is_live_runtime_message(payload) {
        log::debug!("[sessio-runtime:backend:event] {:?}", payload);
    } else {
        log::info!("[sessio-runtime:backend:event] {:?}", payload);
    }
}

fn is_live_runtime_message(payload: &AgentRuntimeEventPayload) -> bool {
    matches!(
        payload,
        AgentRuntimeEventPayload::TextDelta { .. }
            | AgentRuntimeEventPayload::ReasoningDelta { .. }
            | AgentRuntimeEventPayload::ToolStarted { .. }
            | AgentRuntimeEventPayload::ToolInputDelta { .. }
            | AgentRuntimeEventPayload::ToolOutputDelta { .. }
            | AgentRuntimeEventPayload::ToolStatusChanged { .. }
            | AgentRuntimeEventPayload::SessionUpdate { .. }
            | AgentRuntimeEventPayload::AcpProtocolMessage { .. }
    )
}

fn should_emit_runtime_event_to_webview(payload: &AgentRuntimeEventPayload) -> bool {
    !is_live_runtime_message(payload)
}

fn should_emit_snapshot_immediately(payload: &AgentRuntimeEventPayload) -> bool {
    matches!(
        payload,
        AgentRuntimeEventPayload::SessionStarted { .. }
            | AgentRuntimeEventPayload::TurnStarted { .. }
            | AgentRuntimeEventPayload::PermissionRequested { .. }
            | AgentRuntimeEventPayload::PermissionResolved { .. }
            | AgentRuntimeEventPayload::TurnCompleted { .. }
            | AgentRuntimeEventPayload::TurnError { .. }
            | AgentRuntimeEventPayload::TurnCancelled { .. }
    )
}

fn should_emit_turn_snapshot_for_event(payload: &AgentRuntimeEventPayload) -> bool {
    !matches!(payload, AgentRuntimeEventPayload::SessionEnded { .. })
}

fn session_turns_text(turns: &[crate::models::SessionHistoryTurn]) -> String {
    let mut lines = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        lines.push(format!("Turn {} [{}]", index + 1, turn.status));
        for block in &turn.blocks {
            if !matches!(block.kind.as_str(), "user" | "assistant") {
                continue;
            }
            let text_parts = block
                .blocks
                .iter()
                .filter_map(session_content_block_summary)
                .collect::<Vec<_>>();
            let text = text_parts.join("\n");
            if !text.is_empty() {
                lines.push(format!("{}: {}", block.kind, text));
            }
        }
        if let Some(error) = &turn.error {
            lines.push(format!("error: {error}"));
        }
    }
    lines.join("\n")
}

fn session_content_block_summary(block: &crate::models::SessionContentBlock) -> Option<String> {
    if let Some(text) = block
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    let uri = block
        .uri
        .as_deref()
        .map(str::trim)
        .filter(|uri| !uri.is_empty())?;
    let label = block
        .name
        .as_deref()
        .or(block.title.as_deref())
        .or(block.description.as_deref())
        .unwrap_or(uri)
        .trim();
    if block.kind == "image" {
        Some(format!("![{}]({})", label, uri))
    } else {
        Some(format!("[{}]({})", label, uri))
    }
}

fn event_session_id(payload: &AgentRuntimeEventPayload) -> Option<&str> {
    match payload {
        AgentRuntimeEventPayload::SessionStarted {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::TurnStarted {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::TextDelta {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::ReasoningDelta {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::ToolStarted {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::ToolInputDelta {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::ToolOutputDelta {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::ToolStatusChanged {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::SessionUpdate {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::AcpProtocolMessage {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::PermissionRequested {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::PermissionResolved {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::TurnCompleted {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::TurnError {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::TurnCancelled {
            sessio_runtime_session_id,
            ..
        }
        | AgentRuntimeEventPayload::SessionEnded {
            sessio_runtime_session_id,
        } => Some(sessio_runtime_session_id.as_str()),
    }
}

fn session_config_from_options(
    agent: Agent,
    options: &super::types::RuntimeMetadata,
) -> AgentRuntimeSessionConfig {
    AgentRuntimeSessionConfig {
        model: option_string(options, "model"),
        effort: option_string(options, "effort")
            .or_else(|| option_string(options, "reasoningEffort"))
            .or_else(|| option_string(options, "reasoning_effort")),
        permission_mode: if agent == Agent::Opencode {
            // OpenCode `mode` selects a persona / agent mode, not Sessio's
            // permission-mode concept. Don't forward Sessio permissionMode
            // values into ACP `session/set_mode` for OpenCode.
            None
        } else {
            option_string(options, "permissionMode")
                .or_else(|| option_string(options, "permission_mode"))
                .map(|mode| normalize_runtime_permission_mode(agent, &mode))
        },
    }
}

fn normalize_runtime_permission_mode(agent: Agent, value: &str) -> String {
    if agent != Agent::Claude {
        return value.trim().to_string();
    }
    match value.trim().to_ascii_lowercase().as_str() {
        "default" => "default".to_string(),
        "acceptedits" => "acceptEdits".to_string(),
        "plan" => "plan".to_string(),
        "dontask" => "dontAsk".to_string(),
        _ => "default".to_string(),
    }
}

fn option_string(options: &super::types::RuntimeMetadata, key: &str) -> Option<String> {
    options
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn input_option_string<'a>(
    options: &'a super::types::RuntimeMetadata,
    key: &str,
) -> Option<&'a str> {
    options
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn input_option_bool(options: &super::types::RuntimeMetadata, key: &str) -> bool {
    options
        .get(key)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn build_runtime_session_config_record(
    agent: Agent,
    adapter_version: &str,
    session_state: &AcpCanonicalSessionState,
    current: Option<&RuntimeAgentSessionConfigRecord>,
    now: i64,
) -> Result<Option<RuntimeAgentSessionConfigRecord>> {
    if session_state.available_commands.is_empty() && session_state.config_options.is_empty() {
        return Ok(None);
    }

    let current_has_commands = current
        .map(|record| !json_array_is_empty(&record.available_commands_json))
        .unwrap_or(false);
    let current_has_config = current
        .map(|record| !json_array_is_empty(&record.config_options_json))
        .unwrap_or(false);
    let next_has_commands = !session_state.available_commands.is_empty();
    let next_has_config = !session_state.config_options.is_empty();
    if current_has_commands && current_has_config {
        return Ok(None);
    }

    let available_commands_json = if next_has_commands {
        serde_json::to_string(&session_state.available_commands)?
    } else {
        current
            .map(|record| record.available_commands_json.clone())
            .unwrap_or_else(|| "[]".to_string())
    };
    let config_options_json = if next_has_config {
        serde_json::to_string(&session_state.config_options)?
    } else {
        current
            .map(|record| record.config_options_json.clone())
            .unwrap_or_else(|| "[]".to_string())
    };

    Ok(Some(RuntimeAgentSessionConfigRecord {
        agent,
        adapter_version: adapter_version.to_string(),
        available_commands_json,
        config_options_json,
        created_at: current.map(|record| record.created_at).unwrap_or(now),
        updated_at: now,
    }))
}

fn json_array_is_empty(value: &str) -> bool {
    serde_json::from_str::<Vec<serde_json::Value>>(value)
        .map(|items| items.is_empty())
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fake_capabilities_are_stream_ready() {
        let caps = RuntimeCapabilitySet::fake();
        assert!(caps.supports_cancel);
        assert!(caps.supports_permissions);
        assert!(caps.supports_tool_deltas);
        assert!(!caps.supports_attachments);
    }

    #[test]
    fn claude_permission_mode_is_clamped_to_common_modes() {
        assert_eq!(
            normalize_runtime_permission_mode(Agent::Claude, "acceptEdits"),
            "acceptEdits"
        );
        assert_eq!(
            normalize_runtime_permission_mode(Agent::Claude, "bypassPermissions"),
            "default"
        );
        assert_eq!(
            normalize_runtime_permission_mode(Agent::Codex, "full-access"),
            "full-access"
        );
    }

    #[test]
    fn session_config_record_backfills_until_commands_and_config_exist() {
        let mut state = AcpCanonicalSessionState::default();
        let first = build_runtime_session_config_record(
            Agent::Codex,
            "codex-cli 0.134.0",
            &state,
            None,
            10,
        )
        .unwrap();
        assert!(first.is_none());

        state.config_options = vec![crate::turns::AcpSessionConfigOption {
            id: "mode".to_string(),
            name: "Mode".to_string(),
            description: None,
            category: Some("mode".to_string()),
            option_type: Some("select".to_string()),
            current_value: json!("auto"),
            options: Vec::new(),
            groups: Vec::new(),
            meta: json!(null),
            raw: json!({ "id": "mode" }),
        }];
        let second = build_runtime_session_config_record(
            Agent::Codex,
            "codex-cli 0.134.0",
            &state,
            None,
            11,
        )
        .unwrap()
        .unwrap();
        assert_eq!(second.available_commands_json, "[]");
        assert!(second.config_options_json.contains("\"mode\""));

        state.available_commands = vec![crate::turns::AcpAvailableCommand {
            name: "plan".to_string(),
            description: "Plan".to_string(),
            input: json!(null),
            meta: json!(null),
        }];
        let third = build_runtime_session_config_record(
            Agent::Codex,
            "codex-cli 0.134.0",
            &state,
            Some(&second),
            12,
        )
        .unwrap()
        .unwrap();
        assert!(third.available_commands_json.contains("\"plan\""));
        assert!(third.config_options_json.contains("\"mode\""));
        let fourth = build_runtime_session_config_record(
            Agent::Codex,
            "codex-cli 0.134.0",
            &state,
            Some(&third),
            13,
        )
        .unwrap();
        assert!(fourth.is_none());
    }
}
