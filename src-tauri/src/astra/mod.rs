use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::agents::runtime::types::{
    AgentInput, AgentRuntimeEvent, AgentRuntimeEventPayload, AgentSessionHandle, RuntimeMetadata,
    StartAgentSession,
};
use crate::agents::runtime::RuntimeManager;
use crate::memory::service::MemoryService;
use crate::memory::{MemorySearchOptions, MemoryStore};
use crate::models::{
    Agent, IssueSeverity, IssueStatus, ProjectInfo, SessionInfo, StageIssueInfo, StageStatus,
    ThreadInfo,
};
use crate::store::{AstraRunRecord, SessionStore, ThreadWorkSnapshotRecord};

pub const ASTRA_EVENT_NAME: &str = "thread-astra-event";
const PROTOCOL_VERSION: u64 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AstraRunStatus {
    Planning,
    AwaitingApproval,
    Dispatching,
    Running,
    Completed,
    Cancelled,
    Errored,
    Interrupted,
}

impl AstraRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Dispatching => "dispatching",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Errored => "errored",
            Self::Interrupted => "interrupted",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "planning" => Some(Self::Planning),
            "awaiting_approval" => Some(Self::AwaitingApproval),
            "dispatching" => Some(Self::Dispatching),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::Cancelled),
            "errored" => Some(Self::Errored),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    pub fn active(&self) -> bool {
        matches!(
            self,
            Self::Planning | Self::AwaitingApproval | Self::Dispatching | Self::Running
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AstraTaskRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AstraTaskProposal {
    pub id: String,
    pub title: String,
    pub target_stage_id: Option<String>,
    pub target_agent: Agent,
    pub prompt: String,
    pub expected_output: String,
    pub risk: AstraTaskRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraHandle {
    pub run_id: String,
    pub thread_id: String,
    pub project_id: String,
    pub status: AstraRunStatus,
    pub proposed_tasks: Vec<AstraTaskProposal>,
    pub approved_task_ids: Vec<String>,
    pub delegated_session_ids: Vec<String>,
    pub task_results: Vec<AstraTaskResult>,
    pub mode: String,
    pub current_stage_id: Option<String>,
    pub completed_task_ids: Vec<String>,
    pub stage_attempt_counts: HashMap<String, u32>,
    pub retry_limit: u32,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraRun {
    pub run_id: String,
    pub thread_id: String,
    pub project_id: String,
    pub project_path: String,
    pub status: AstraRunStatus,
    pub proposed_tasks: Vec<AstraTaskProposal>,
    pub approved_task_ids: Vec<String>,
    pub delegated_session_ids: Vec<String>,
    pub task_results: Vec<AstraTaskResult>,
    pub mode: String,
    pub current_stage_id: Option<String>,
    pub completed_task_ids: Vec<String>,
    pub stage_attempt_counts: HashMap<String, u32>,
    pub retry_limit: u32,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AstraTaskResultStatus {
    Completed,
    Failed,
    Errored,
    Cancelled,
}

impl AstraTaskResultStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Errored => "errored",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AstraTaskResult {
    pub task_id: String,
    pub thread_stage_id: Option<String>,
    pub sessio_runtime_session_id: String,
    pub turn_id: Option<String>,
    pub status: AstraTaskResultStatus,
    pub output: String,
    pub error: Option<String>,
    pub attempt_count: u32,
    pub retry_limit_reached: bool,
    pub completed_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraStageMutationResult {
    pub ok: bool,
    #[serde(default)]
    pub stage: Option<Value>,
    #[serde(default)]
    pub issue: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    pub applied_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartThreadAstraRequest {
    pub thread_id: String,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelThreadAstraRequest {
    pub run_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraEvent {
    pub run_id: String,
    pub thread_id: String,
    pub status: AstraRunStatus,
    pub event_type: String,
    pub data: Value,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraProtocolRequest {
    pub protocol_version: u64,
    pub id: String,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraProtocolResponse {
    pub protocol_version: u64,
    pub id: String,
    pub result: Option<Value>,
    pub error: Option<AstraProtocolError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraProtocolEvent {
    pub protocol_version: u64,
    pub method: String,
    pub params: AstraProtocolEventParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraProtocolEventParams {
    pub run_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraProtocolError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraToolCall {
    pub run_id: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraToolResult {
    pub ok: bool,
    #[serde(default)]
    pub result: Value,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct AstraService {
    inner: Arc<AstraServiceInner>,
}

struct AstraServiceInner {
    app: AppHandle,
    store: Arc<dyn SessionStore>,
    memory_store: Arc<dyn MemoryStore>,
    runtime: RuntimeManager,
    sequence: AtomicU64,
    sidecar: Mutex<Option<SidecarHandle>>,
    delegated_sessions: Mutex<HashMap<String, DelegatedSessionState>>,
    task_waiters: Mutex<HashMap<String, mpsc::Sender<AstraTaskResult>>>,
    // Serializes read-modify-write cycles on a single run row (see mutate_run).
    run_write_lock: Mutex<()>,
}

#[derive(Debug, Clone)]
struct DelegatedSessionState {
    run_id: String,
    task_id: String,
    thread_stage_id: Option<String>,
    agent_session_id: Option<String>,
    stage_task_context: Option<StageTaskContext>,
    session_recorded: bool,
    attempt_count: u32,
    retry_limit_reached: bool,
    text: String,
    last_turn_id: Option<String>,
    finished: bool,
}

enum DispatchTaskDecision {
    RetryLimit {
        result: AstraTaskResult,
        retry_limit: u32,
    },
    Dispatch {
        attempt_count: u32,
    },
}

struct SidecarHandle {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    pending: Arc<Mutex<HashMap<String, mpsc::Sender<AstraProtocolResponse>>>>,
}

impl Drop for SidecarHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

impl AstraService {
    pub fn new(
        app: AppHandle,
        store: Arc<dyn SessionStore>,
        memory_store: Arc<dyn MemoryStore>,
        runtime: RuntimeManager,
    ) -> Self {
        Self {
            inner: Arc::new(AstraServiceInner {
                app,
                store,
                memory_store,
                runtime,
                sequence: AtomicU64::new(1),
                sidecar: Mutex::new(None),
                delegated_sessions: Mutex::new(HashMap::new()),
                task_waiters: Mutex::new(HashMap::new()),
                run_write_lock: Mutex::new(()),
            }),
        }
    }

    pub fn watch_runtime_events(&self) -> Result<()> {
        let receiver = self.inner.runtime.subscribe_events()?;
        let service = self.clone();
        thread::spawn(move || {
            for event in receiver {
                if let Err(error) = service.handle_runtime_event(event) {
                    log::warn!("[sessio-astra:runtime-event] {error}");
                }
            }
        });
        Ok(())
    }

    pub fn recover_interrupted_runs(&self) -> Result<()> {
        self.inner.store.interrupt_active_astra_runs()
    }

    pub fn start_thread_astra(&self, req: StartThreadAstraRequest) -> Result<AstraHandle> {
        if req.thread_id.trim().is_empty() {
            bail!("threadId is required");
        }
        let thread = self.inner.store.get_thread_work_state(&req.thread_id)?;
        let project = self
            .inner
            .store
            .list_projects()?
            .into_iter()
            .find(|project| project.id == thread.project_id)
            .ok_or_else(|| anyhow::anyhow!("project not found: {}", thread.project_id))?;
        let run = {
            let _guard = self
                .inner
                .run_write_lock
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra run write lock poisoned"))?;
            if let Some(active) = self.inner.store.get_active_astra_run(&req.thread_id)? {
                return Ok(run_to_handle(record_to_run(active)?));
            }

            let now = now_ms();
            let run = AstraRun {
                run_id: stable_run_id(&thread.id, now),
                thread_id: thread.id.clone(),
                project_id: thread.project_id.clone(),
                project_path: project.path.clone(),
                status: AstraRunStatus::Planning,
                proposed_tasks: Vec::new(),
                approved_task_ids: Vec::new(),
                delegated_session_ids: Vec::new(),
                task_results: Vec::new(),
                mode: "auto".to_string(),
                current_stage_id: None,
                completed_task_ids: Vec::new(),
                stage_attempt_counts: HashMap::new(),
                retry_limit: 3,
                error: None,
                created_at: now,
                updated_at: now,
            };
            self.inner.store.upsert_astra_run(&run_to_record(&run))?;
            run
        };
        log::info!(
            "[sessio-astra:run:start] runId={} threadId={} projectId={}",
            run.run_id,
            run.thread_id,
            run.project_id
        );
        self.emit(&run, "status", json!({ "status": run.status.as_str() }));

        let astra_agent = self
            .inner
            .store
            .list_agents()?
            .into_iter()
            .find(|agent| agent.id == "astra");
        let model_config = astra_agent.and_then(|agent| {
            let selected_provider_id = agent.ai_provider.as_deref().unwrap_or("");
            let provider = agent
                .ai_providers
                .iter()
                .find(|provider| provider.id == selected_provider_id && provider.enabled)
                .or_else(|| agent.ai_providers.iter().find(|provider| provider.enabled))
                .or_else(|| agent.ai_providers.first())?;
            let model_id = provider.model.clone().or(agent.model).or_else(|| {
                provider
                    .models
                    .iter()
                    .find(|model| model.enabled)
                    .map(|model| model.value.clone())
            });
            Some(json!({
                "provider": provider.provider,
                "api": provider.api,
                "baseUrl": provider.base_url,
                "apiKey": provider.api_key,
                "modelId": model_id,
                "thinkingLevel": agent.effort,
            }))
        });
        let params = json!({
            "runId": run.run_id,
            "thread": thread,
            "snapshot": project_snapshot(&project, &thread),
            "prompt": req.prompt,
            "modelConfig": model_config,
        });

        let service = self.clone();
        let run_id = run.run_id.clone();
        thread::spawn(move || {
            if let Err(error) = service.run_start_planning(&run_id, params) {
                log::warn!("[sessio-astra:run:start-worker] {error}");
            }
        });

        Ok(run_to_handle(run))
    }

    fn run_start_planning(&self, run_id: &str, params: Value) -> Result<()> {
        let response = match self.request("astra/start", params, Some(Duration::from_secs(20))) {
            Ok(response) => response,
            Err(error) => {
                let (failed, changed) = self.update_active_status(
                    run_id,
                    AstraRunStatus::Errored,
                    Some(error.to_string()),
                )?;
                if changed {
                    self.emit(&failed, "error", json!({ "message": error.to_string() }));
                }
                return Err(error);
            }
        };
        if let Some(error) = response.error {
            let (failed, changed) = self.update_active_status(
                run_id,
                AstraRunStatus::Errored,
                Some(error.message.clone()),
            )?;
            if changed {
                self.emit(&failed, "error", json!({ "message": error.message }));
            }
            bail!("Astra start failed: {}", error.code);
        }
        log::info!(
            "[sessio-astra:run:orchestrator-started] runId={} response={:?}",
            run_id,
            response.result
        );
        Ok(())
    }

    pub fn cancel_thread_astra(&self, req: CancelThreadAstraRequest) -> Result<AstraHandle> {
        let run = self.load_run(&req.run_id)?;
        // Abort every delegated session this run launched: interrupt the ACP
        // agents and release any blocked dispatch waiter. Sessions started by
        // other runs or by the user are left untouched.
        let delegated_sessions: Vec<String> = match self.inner.delegated_sessions.lock() {
            Ok(delegated) => delegated
                .iter()
                .filter(|(_, state)| state.run_id == run.run_id && !state.finished)
                .map(|(session_id, _)| session_id.clone())
                .collect(),
            Err(_) => Vec::new(),
        };
        for session_id in &delegated_sessions {
            self.abort_delegated_session(session_id);
        }
        let _ = self.request(
            "astra/cancel",
            json!({ "runId": run.run_id }),
            Some(Duration::from_secs(3)),
        );
        let run = self.update_status(&run.run_id, AstraRunStatus::Cancelled, None)?;
        log::info!(
            "[sessio-astra:run:cancel] runId={} threadId={} delegatedSessions={}",
            run.run_id,
            run.thread_id,
            delegated_sessions.len()
        );
        self.emit(&run, "cancelled", json!({ "status": run.status.as_str() }));
        Ok(run_to_handle(run))
    }

    pub fn get_thread_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraHandle>> {
        self.inner
            .store
            .list_astra_runs(thread_id)?
            .into_iter()
            .map(record_to_run)
            .map(|result| result.map(run_to_handle))
            .collect()
    }

    fn dispatch_task(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        task: &AstraTaskProposal,
        resolved_thread_stage_id: Option<&str>,
        attempt_count: u32,
        retry_limit_reached: bool,
        task_waiter: Option<mpsc::Sender<AstraTaskResult>>,
    ) -> Result<AgentSessionHandle> {
        let stage_context = resolved_thread_stage_id
            .map(|stage_id| build_stage_task_context(thread, stage_id, task))
            .transpose()?;
        let mut options = RuntimeMetadata::default();
        options.insert("astraRunId".to_string(), Value::String(run.run_id.clone()));
        options.insert("astraTaskId".to_string(), Value::String(task.id.clone()));
        if let Some(stage_id) = resolved_thread_stage_id {
            options.insert(
                "astraThreadStageId".to_string(),
                Value::String(stage_id.to_string()),
            );
        }
        options.insert(
            "astraAttemptCount".to_string(),
            Value::Number(serde_json::Number::from(attempt_count)),
        );
        options.insert(
            "astraRetryLimitReached".to_string(),
            Value::Bool(retry_limit_reached),
        );
        let initial_prompt = stage_context
            .as_ref()
            .map(|context| context.prompt.clone())
            .unwrap_or_else(|| task.prompt.clone());
        let mut req = StartAgentSession {
            agent: task.target_agent,
            workspace_path: run.project_path.clone(),
            initial_prompt: None,
            source_session_id: None,
            source_agent: None,
            options,
        };
        hydrate_start_request_for_astra(&mut req, self.inner.store.as_ref())?;
        let handle = self.inner.runtime.start_session(req)?;
        self.track_delegated_session(
            &run.run_id,
            &task.id,
            resolved_thread_stage_id,
            attempt_count,
            retry_limit_reached,
            &handle.sessio_runtime_session_id,
            stage_context,
        )?;
        // Register the result waiter before sending the prompt: a fast (or
        // synchronous fake) turn can reach a terminal state inside send_input,
        // so the waiter must already be in place or its wakeup is lost.
        if let Some(waiter) = task_waiter {
            self.inner
                .task_waiters
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra task waiter lock poisoned"))?
                .insert(handle.sessio_runtime_session_id.clone(), waiter);
        }
        if !initial_prompt.trim().is_empty() {
            self.inner.runtime.send_input(
                &handle.sessio_runtime_session_id,
                AgentInput {
                    text: initial_prompt,
                    attachments: Vec::new(),
                    options: RuntimeMetadata::default(),
                },
            )?;
        }
        Ok(handle)
    }

    fn dispatch_task_and_wait(
        &self,
        run: &AstraRun,
        task: &AstraTaskProposal,
    ) -> Result<AstraTaskResult> {
        let mut thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        let stage_id = task
            .target_stage_id
            .as_deref()
            .map(|id| resolve_thread_stage_id(&thread, id))
            .transpose()?;
        if let Some(stage_id) = stage_id.as_deref() {
            thread = self.prepare_stage_for_delegated_task(&run.thread_id, stage_id)?;
        }
        let task_id = task.id.clone();
        let stage_id_for_run = stage_id.clone();
        let (next, decision) = self.mutate_run(&run.run_id, move |next| {
            if !next.status.active() {
                bail!("Astra run is not active: {}", next.run_id);
            }

            next.status = AstraRunStatus::Running;
            next.current_stage_id = stage_id_for_run.clone();
            let prior_attempt_count = stage_id_for_run
                .as_ref()
                .map(|id| *next.stage_attempt_counts.entry(id.clone()).or_insert(0))
                .unwrap_or(0);
            let retry_limit_reached = stage_id_for_run
                .as_ref()
                .map(|_| prior_attempt_count >= next.retry_limit)
                .unwrap_or(false);
            if retry_limit_reached {
                let result = AstraTaskResult {
                    task_id: task_id.clone(),
                    thread_stage_id: stage_id_for_run.clone(),
                    sessio_runtime_session_id: String::new(),
                    turn_id: None,
                    status: AstraTaskResultStatus::Failed,
                    output: String::new(),
                    error: Some("retry limit reached".to_string()),
                    attempt_count: prior_attempt_count,
                    retry_limit_reached: true,
                    completed_at: now_ms(),
                };
                upsert_task_result_in_run(next, result.clone());
                return Ok(DispatchTaskDecision::RetryLimit {
                    result,
                    retry_limit: next.retry_limit,
                });
            }
            let attempt_count = stage_id_for_run
                .as_ref()
                .map(|id| {
                    let count = next.stage_attempt_counts.entry(id.clone()).or_insert(0);
                    *count += 1;
                    *count
                })
                .unwrap_or(1);
            Ok(DispatchTaskDecision::Dispatch { attempt_count })
        })?;

        let attempt_count = match decision {
            DispatchTaskDecision::RetryLimit {
                result,
                retry_limit,
            } => {
                self.emit(
                    &next,
                    "retry_limit",
                    json!({
                        "taskId": task.id,
                        "threadStageId": stage_id,
                        "attemptCount": result.attempt_count,
                        "retryLimit": retry_limit,
                    }),
                );
                return Ok(result);
            }
            DispatchTaskDecision::Dispatch { attempt_count } => attempt_count,
        };

        // Create the waiter channel up front and hand the sender to dispatch_task
        // so it is registered before the prompt is sent (avoids the lost-wakeup
        // race where a synchronous turn finishes before we start waiting).
        let (sender, receiver) = mpsc::channel();
        let handle = self.dispatch_task(
            &next,
            &thread,
            task,
            stage_id.as_deref(),
            attempt_count,
            false,
            Some(sender),
        )?;
        log::info!(
            "[sessio-astra:task:dispatch] runId={} threadId={} taskId={} threadStageId={:?} runtimeSessionId={} attemptCount={}",
            next.run_id,
            next.thread_id,
            task.id,
            stage_id,
            handle.sessio_runtime_session_id,
            attempt_count
        );

        match receiver.recv_timeout(Duration::from_secs(60 * 60)) {
            Ok(result) => Ok(result),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.abort_delegated_session(&handle.sessio_runtime_session_id);
                bail!("delegated task timed out: {}", task.id)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.abort_delegated_session(&handle.sessio_runtime_session_id);
                bail!("delegated task waiter disconnected: {}", task.id)
            }
        }
    }

    /// Best-effort termination of a delegated runtime session. Marks the
    /// delegated state finished so a late runtime event cannot double-record a
    /// result, cancels any active turn (which also interrupts the real ACP
    /// agent), disposes the runtime session, and drops the task waiter so a
    /// blocked `dispatch_task_and_wait` is released.
    fn abort_delegated_session(&self, sessio_runtime_session_id: &str) {
        let turn_id = match self.inner.delegated_sessions.lock() {
            Ok(mut delegated) => match delegated.get_mut(sessio_runtime_session_id) {
                Some(state) => {
                    state.finished = true;
                    state.last_turn_id.clone()
                }
                None => None,
            },
            Err(_) => None,
        };
        if let Some(turn_id) = turn_id.as_deref() {
            let _ = self
                .inner
                .runtime
                .cancel_turn(sessio_runtime_session_id, turn_id);
        }
        let _ = self
            .inner
            .runtime
            .dispose_session_silent(sessio_runtime_session_id);
        if let Ok(mut waiters) = self.inner.task_waiters.lock() {
            waiters.remove(sessio_runtime_session_id);
        }
    }

    fn track_delegated_session(
        &self,
        run_id: &str,
        task_id: &str,
        thread_stage_id: Option<&str>,
        attempt_count: u32,
        retry_limit_reached: bool,
        sessio_runtime_session_id: &str,
        stage_task_context: Option<StageTaskContext>,
    ) -> Result<()> {
        let mut delegated = self
            .inner
            .delegated_sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("Astra delegated session lock poisoned"))?;
        delegated
            .entry(sessio_runtime_session_id.to_string())
            .and_modify(|state| {
                state.run_id = run_id.to_string();
                state.task_id = task_id.to_string();
                state.thread_stage_id = thread_stage_id.map(ToString::to_string);
                state.attempt_count = attempt_count;
                state.retry_limit_reached = retry_limit_reached;
                if stage_task_context.is_some() {
                    state.stage_task_context = stage_task_context.clone();
                }
            })
            .or_insert_with(|| DelegatedSessionState {
                run_id: run_id.to_string(),
                task_id: task_id.to_string(),
                thread_stage_id: thread_stage_id.map(ToString::to_string),
                agent_session_id: None,
                stage_task_context,
                session_recorded: false,
                attempt_count,
                retry_limit_reached,
                text: String::new(),
                last_turn_id: None,
                finished: false,
            });
        Ok(())
    }

    fn handle_runtime_event(&self, event: AgentRuntimeEvent) -> Result<()> {
        match event.payload {
            AgentRuntimeEventPayload::SessionStarted {
                sessio_runtime_session_id,
                agent,
                agent_runtime_session_id,
                metadata,
                ..
            } => {
                let Some(run_id) = metadata.get("astraRunId").and_then(Value::as_str) else {
                    return Ok(());
                };
                let Some(task_id) = metadata.get("astraTaskId").and_then(Value::as_str) else {
                    return Ok(());
                };
                let thread_stage_id = metadata.get("astraThreadStageId").and_then(Value::as_str);
                let attempt_count = metadata
                    .get("astraAttemptCount")
                    .and_then(Value::as_u64)
                    .unwrap_or(1) as u32;
                let retry_limit_reached = metadata
                    .get("astraRetryLimitReached")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if is_persistable_agent_session_id(&agent_runtime_session_id) {
                    self.track_delegated_session(
                        run_id,
                        task_id,
                        thread_stage_id,
                        attempt_count,
                        retry_limit_reached,
                        &sessio_runtime_session_id,
                        None,
                    )?;
                    self.record_ready_delegated_session(
                        run_id,
                        agent,
                        task_id,
                        thread_stage_id,
                        &agent_runtime_session_id,
                        &sessio_runtime_session_id,
                    )?;
                }
            }
            AgentRuntimeEventPayload::TurnStarted {
                sessio_runtime_session_id,
                turn_id,
            } => {
                self.update_delegated_state(&sessio_runtime_session_id, |state| {
                    state.last_turn_id = Some(turn_id);
                })?;
            }
            AgentRuntimeEventPayload::TextDelta {
                sessio_runtime_session_id,
                turn_id,
                text,
            } => {
                self.update_delegated_state(&sessio_runtime_session_id, |state| {
                    state.last_turn_id = Some(turn_id);
                    state.text.push_str(&text);
                })?;
            }
            AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id,
                turn_id,
                result,
            } => {
                let output = result
                    .as_ref()
                    .and_then(extract_result_text)
                    .unwrap_or_else(|| {
                        self.delegated_output(&sessio_runtime_session_id)
                            .unwrap_or_default()
                    });
                self.finish_delegated_task(
                    &sessio_runtime_session_id,
                    Some(turn_id),
                    AstraTaskResultStatus::Completed,
                    output,
                    None,
                )?;
            }
            AgentRuntimeEventPayload::TurnError {
                sessio_runtime_session_id,
                turn_id,
                error,
            } => {
                let output = self
                    .delegated_output(&sessio_runtime_session_id)
                    .unwrap_or_default();
                self.finish_delegated_task(
                    &sessio_runtime_session_id,
                    Some(turn_id),
                    AstraTaskResultStatus::Errored,
                    output,
                    Some(format!("{}: {}", error.code, error.message)),
                )?;
            }
            AgentRuntimeEventPayload::TurnCancelled {
                sessio_runtime_session_id,
                turn_id,
            } => {
                let output = self
                    .delegated_output(&sessio_runtime_session_id)
                    .unwrap_or_default();
                self.finish_delegated_task(
                    &sessio_runtime_session_id,
                    Some(turn_id),
                    AstraTaskResultStatus::Cancelled,
                    output,
                    Some("turn cancelled".to_string()),
                )?;
            }
            AgentRuntimeEventPayload::SessionEnded {
                sessio_runtime_session_id,
            } => {
                // A normally-completed turn already finished the task via
                // TurnCompleted (finished=true). Reaching here unfinished means
                // the delegated session ended without a terminal turn (crash or
                // abnormal exit), so record it as errored rather than completed.
                let should_finish = {
                    let delegated =
                        self.inner.delegated_sessions.lock().map_err(|_| {
                            anyhow::anyhow!("Astra delegated session lock poisoned")
                        })?;
                    delegated
                        .get(&sessio_runtime_session_id)
                        .map(|state| !state.finished)
                        .unwrap_or(false)
                };
                if should_finish {
                    let output = self
                        .delegated_output(&sessio_runtime_session_id)
                        .unwrap_or_default();
                    self.finish_delegated_task(
                        &sessio_runtime_session_id,
                        None,
                        AstraTaskResultStatus::Errored,
                        output,
                        Some("delegated session ended before turn completion".to_string()),
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn update_delegated_state(
        &self,
        sessio_runtime_session_id: &str,
        update: impl FnOnce(&mut DelegatedSessionState),
    ) -> Result<()> {
        let mut delegated = self
            .inner
            .delegated_sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("Astra delegated session lock poisoned"))?;
        if let Some(state) = delegated.get_mut(sessio_runtime_session_id) {
            update(state);
        }
        Ok(())
    }

    fn delegated_output(&self, sessio_runtime_session_id: &str) -> Option<String> {
        self.inner
            .delegated_sessions
            .lock()
            .ok()
            .and_then(|delegated| {
                delegated
                    .get(sessio_runtime_session_id)
                    .map(|state| state.text.clone())
            })
    }

    fn record_ready_delegated_session(
        &self,
        run_id: &str,
        agent: Agent,
        task_id: &str,
        thread_stage_id: Option<&str>,
        agent_session_id: &str,
        sessio_runtime_session_id: &str,
    ) -> Result<()> {
        let (state_context, already_recorded) = {
            let delegated = self
                .inner
                .delegated_sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra delegated session lock poisoned"))?;
            let state = delegated.get(sessio_runtime_session_id);
            let context = state.and_then(|state| state.stage_task_context.clone());
            let recorded = state
                .map(|state| {
                    state.session_recorded
                        && state.agent_session_id.as_deref() == Some(agent_session_id)
                })
                .unwrap_or(false);
            (context, recorded)
        };
        if already_recorded {
            return Ok(());
        }
        let run = self.load_run(run_id)?;
        let task = run
            .proposed_tasks
            .iter()
            .find(|task| task.id == task_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Astra task not found: {task_id}"))?;
        let context = match (state_context, thread_stage_id) {
            (Some(context), _) => Some(context),
            (None, Some(stage_id)) => {
                let thread = self.prepare_stage_for_delegated_task(&run.thread_id, stage_id)?;
                Some(build_stage_task_context(&thread, stage_id, &task)?)
            }
            (None, None) => None,
        };
        record_and_link_ready_delegated_session(
            self.inner.store.as_ref(),
            &run,
            agent,
            agent_session_id,
            &task,
            thread_stage_id,
            context.as_ref(),
        )?;
        let sessio_runtime_session_id_for_run = sessio_runtime_session_id.to_string();
        let agent_session_id_for_run = agent_session_id.to_string();
        let (run, _) = self.mutate_run(run_id, move |run| {
            run.delegated_session_ids
                .retain(|id| id != &sessio_runtime_session_id_for_run);
            if !run
                .delegated_session_ids
                .iter()
                .any(|id| id == &agent_session_id_for_run)
            {
                run.delegated_session_ids.push(agent_session_id_for_run);
            }
            Ok(())
        })?;
        self.update_delegated_state(sessio_runtime_session_id, |state| {
            state.agent_session_id = Some(agent_session_id.to_string());
            state.session_recorded = true;
        })?;
        self.emit(
            &run,
            "delegated",
            json!({
                "taskId": task.id,
                "threadStageId": thread_stage_id,
                "targetStageId": task.target_stage_id,
                "sessioRuntimeSessionId": agent_session_id,
                "agentRuntimeSessionId": agent_session_id,
                "liveRuntimeSessionId": sessio_runtime_session_id,
                "agent": agent,
            }),
        );
        log::info!(
            "[sessio-astra:task:delegated] runId={} threadId={} taskId={} threadStageId={:?} agentSessionId={} runtimeSessionId={}",
            run.run_id,
            run.thread_id,
            task.id,
            thread_stage_id,
            agent_session_id,
            sessio_runtime_session_id
        );
        Ok(())
    }

    fn prepare_stage_for_delegated_task(
        &self,
        thread_id: &str,
        thread_stage_id: &str,
    ) -> Result<ThreadInfo> {
        let thread = self.inner.store.get_thread_work_state(thread_id)?;
        let Some(stage) = thread
            .stages
            .iter()
            .find(|stage| stage.id == thread_stage_id)
        else {
            return Ok(thread);
        };
        if stage.status == StageStatus::NotStarted {
            self.inner.store.update_thread_stage_state(
                thread_stage_id,
                Some(StageStatus::InProgress),
                None,
                None,
            )?;
            return self.inner.store.get_thread_work_state(thread_id);
        }
        Ok(thread)
    }

    fn finish_delegated_task(
        &self,
        sessio_runtime_session_id: &str,
        turn_id: Option<String>,
        status: AstraTaskResultStatus,
        output: String,
        error: Option<String>,
    ) -> Result<()> {
        let state = {
            let mut delegated = self
                .inner
                .delegated_sessions
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra delegated session lock poisoned"))?;
            let Some(state) = delegated.get_mut(sessio_runtime_session_id) else {
                return Ok(());
            };
            if state.finished {
                return Ok(());
            }
            state.finished = true;
            if turn_id.is_some() {
                state.last_turn_id = turn_id.clone();
            }
            state.clone()
        };
        let result = AstraTaskResult {
            task_id: state.task_id.clone(),
            thread_stage_id: state.thread_stage_id.clone(),
            sessio_runtime_session_id: state
                .agent_session_id
                .clone()
                .unwrap_or_else(|| sessio_runtime_session_id.to_string()),
            turn_id: turn_id.or(state.last_turn_id),
            status,
            output,
            error,
            attempt_count: state.attempt_count,
            retry_limit_reached: state.retry_limit_reached,
            completed_at: now_ms(),
        };
        let run = self.record_task_result(&state.run_id, result.clone())?;
        if let Some(sender) = self
            .inner
            .task_waiters
            .lock()
            .ok()
            .and_then(|mut waiters| waiters.remove(sessio_runtime_session_id))
        {
            let _ = sender.send(result.clone());
        }
        self.emit(&run, "task_result", serde_json::to_value(&result)?);
        log::info!(
            "[sessio-astra:task:result] runId={} threadId={} taskId={} sessioRuntimeSessionId={} status={}",
            run.run_id,
            run.thread_id,
            result.task_id,
            result.sessio_runtime_session_id,
            result.status.as_str()
        );

        let service = self.clone();
        let run_id = run.run_id.clone();
        let result_for_notify = result.clone();
        thread::spawn(move || {
            if let Err(error) = service.notify_astra_task_result(&run_id, &result_for_notify) {
                log::warn!("[sessio-astra:task:result:notify] runId={run_id} {error}");
            }
        });
        Ok(())
    }

    fn record_task_result(&self, run_id: &str, result: AstraTaskResult) -> Result<AstraRun> {
        let (run, _) = self.mutate_run(run_id, move |run| {
            upsert_task_result_in_run(run, result);
            Ok(())
        })?;
        Ok(run)
    }

    fn notify_astra_task_result(&self, run_id: &str, result: &AstraTaskResult) -> Result<()> {
        let response = self.request(
            "astra/task_result",
            json!({ "runId": run_id, "result": result }),
            Some(Duration::from_secs(5)),
        )?;
        if let Some(error) = response.error {
            bail!("Astra task result rejected: {}", error.message);
        }
        Ok(())
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<AstraProtocolResponse> {
        let id = self
            .inner
            .sequence
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        let (receiver, pending, message) = {
            let mut sidecar_guard = self
                .inner
                .sidecar
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra sidecar lock poisoned"))?;
            if sidecar_guard.is_none() {
                *sidecar_guard = Some(self.spawn_sidecar()?);
            }
            let sidecar = sidecar_guard
                .as_mut()
                .context("Astra sidecar unavailable")?;
            let (sender, receiver) = mpsc::channel();
            sidecar
                .pending
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra pending lock poisoned"))?
                .insert(id.clone(), sender);
            let message = AstraProtocolRequest {
                protocol_version: PROTOCOL_VERSION,
                id: id.clone(),
                method: method.to_string(),
                params,
            };
            let line = serde_json::to_string(&message)?;
            let write_result = {
                let mut stdin = sidecar
                    .stdin
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Astra stdin lock poisoned"))?;
                writeln!(stdin, "{line}").and_then(|_| stdin.flush())
            };
            if let Err(error) = write_result {
                sidecar
                    .pending
                    .lock()
                    .ok()
                    .and_then(|mut pending| pending.remove(&id));
                *sidecar_guard = None;
                return Err(error).context("write Astra request");
            }
            (receiver, sidecar.pending.clone(), message)
        };
        let response = match timeout {
            Some(timeout) => receiver
                .recv_timeout(timeout)
                .map_err(|_| anyhow::anyhow!("Astra request timed out: {}", message.method)),
            None => receiver
                .recv()
                .map_err(|_| anyhow::anyhow!("Astra response channel closed")),
        };
        pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&id));
        Ok(response?)
    }

    fn spawn_sidecar(&self) -> Result<SidecarHandle> {
        let mut command = astra_command(&self.inner.app)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().context("spawn Astra sidecar")?;
        let stdin = Arc::new(Mutex::new(
            child
                .stdin
                .take()
                .context("Astra sidecar stdin unavailable")?,
        ));
        let stdout = child
            .stdout
            .take()
            .context("Astra sidecar stdout unavailable")?;
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    log::warn!("[sessio-astra:stderr] {line}");
                }
            });
        }

        let pending: Arc<Mutex<HashMap<String, mpsc::Sender<AstraProtocolResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = pending.clone();
        let stdin_reader = stdin.clone();
        let service = self.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                match serde_json::from_str::<Value>(&line) {
                    Ok(value) if value.get("method").and_then(Value::as_str) == Some("event") => {
                        if let Ok(event) = serde_json::from_value::<AstraProtocolEvent>(value) {
                            let _ = service.apply_protocol_event(event);
                        }
                    }
                    Ok(value)
                        if value.get("method").and_then(Value::as_str) == Some("tool/call") =>
                    {
                        let service = service.clone();
                        let stdin_writer = stdin_reader.clone();
                        thread::spawn(move || {
                            let response =
                                match serde_json::from_value::<AstraProtocolRequest>(value) {
                                    Ok(request) => service.handle_tool_request(request),
                                    Err(error) => AstraProtocolResponse {
                                        protocol_version: PROTOCOL_VERSION,
                                        id: "unknown".to_string(),
                                        result: None,
                                        error: Some(AstraProtocolError {
                                            code: "invalid_request".to_string(),
                                            message: error.to_string(),
                                            data: None,
                                        }),
                                    },
                                };
                            if let Ok(line) = serde_json::to_string(&response) {
                                if let Ok(mut stdin) = stdin_writer.lock() {
                                    let _ = writeln!(stdin, "{line}");
                                    let _ = stdin.flush();
                                }
                            }
                        });
                    }
                    Ok(value) => match serde_json::from_value::<AstraProtocolResponse>(value) {
                        Ok(response) => {
                            let sender = pending_reader
                                .lock()
                                .ok()
                                .and_then(|mut pending| pending.remove(&response.id));
                            if let Some(sender) = sender {
                                let _ = sender.send(response);
                            }
                        }
                        Err(error) => {
                            log::warn!("[sessio-astra:protocol] invalid response: {error}")
                        }
                    },
                    Err(error) => {
                        log::warn!("[sessio-astra:protocol] invalid json: {error}: {line}")
                    }
                }
            }
        });

        Ok(SidecarHandle {
            child,
            stdin,
            pending,
        })
    }

    fn handle_tool_request(&self, request: AstraProtocolRequest) -> AstraProtocolResponse {
        let id = request.id.clone();
        match self.execute_tool_request(request) {
            Ok(result) => AstraProtocolResponse {
                protocol_version: PROTOCOL_VERSION,
                id,
                result: Some(result),
                error: None,
            },
            Err(error) => AstraProtocolResponse {
                protocol_version: PROTOCOL_VERSION,
                id,
                result: None,
                error: Some(AstraProtocolError {
                    code: "tool_error".to_string(),
                    message: error.to_string(),
                    data: None,
                }),
            },
        }
    }

    fn execute_tool_request(&self, request: AstraProtocolRequest) -> Result<Value> {
        if request.protocol_version != PROTOCOL_VERSION {
            bail!(
                "unsupported Astra protocol version: {}",
                request.protocol_version
            );
        }
        if request.method != "tool/call" {
            bail!("unsupported Astra request method: {}", request.method);
        }
        let call: AstraToolCall = serde_json::from_value(request.params)?;
        let run = self.load_run(&call.run_id)?;
        match call.name.as_str() {
            "sessio.project.snapshot" => {
                let thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
                let project = self
                    .inner
                    .store
                    .list_projects()?
                    .into_iter()
                    .find(|project| project.id == run.project_id)
                    .ok_or_else(|| anyhow::anyhow!("project not found: {}", run.project_id))?;
                Ok(project_snapshot(&project, &thread))
            }
            "sessio.memory.search" => {
                let query = call
                    .args
                    .get("query")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("query is required"))?;
                let project_key = call
                    .args
                    .get("projectKey")
                    .and_then(Value::as_str)
                    .unwrap_or(&run.project_path);
                let service = MemoryService::new(
                    self.inner.memory_store.clone(),
                    Arc::new(crate::agents::sources::builtin_agent_sources()),
                )?;
                let result = service.search_full(
                    project_key,
                    query,
                    MemorySearchOptions { include_raw: false },
                )?;
                Ok(json!({
                    "backend": result.backend,
                    "hits": result.hits.into_iter().map(|hit| json!({
                        "record": hit.record,
                        "score": hit.score,
                        "snippet": hit.snippet,
                        "sources": hit.sources,
                        "continuation": hit.continuation,
                    })).collect::<Vec<_>>()
                }))
            }
            "sessio.agent.plan_task" => {
                let task: AstraTaskProposal = serde_json::from_value(call.args)?;
                let (next, _) = self.mutate_run(&run.run_id, move |next| {
                    if !next
                        .proposed_tasks
                        .iter()
                        .any(|existing| existing.id == task.id)
                    {
                        next.proposed_tasks.push(task);
                    }
                    Ok(())
                })?;
                Ok(
                    json!({ "taskIds": next.proposed_tasks.iter().map(|task| task.id.clone()).collect::<Vec<_>>() }),
                )
            }
            "sessio.agent.dispatch_task" => {
                let task_id = call
                    .args
                    .get("taskId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("taskId is required"))?;
                let run = self.load_run(&call.run_id)?;
                let task = run
                    .proposed_tasks
                    .iter()
                    .find(|task| task.id == task_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;
                let result = self.dispatch_task_and_wait(&run, &task)?;
                Ok(serde_json::to_value(result)?)
            }
            "sessio.stage.update" => Ok(serde_json::to_value(
                self.apply_stage_update_decision(&run, &call.args)?,
            )?),
            "sessio.stage.issue.add_or_update" => Ok(serde_json::to_value(
                self.apply_issue_decision(&run, &call.args)?,
            )?),
            other => bail!("unknown Astra tool: {other}"),
        }
    }

    fn apply_stage_update_decision(
        &self,
        run: &AstraRun,
        args: &Value,
    ) -> Result<AstraStageMutationResult> {
        if let Some(rejection) = inactive_run_mutation_error(run) {
            return Ok(rejection);
        }
        let stage_id = args
            .get("threadStageId")
            .or_else(|| args.get("stageId"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("threadStageId is required"))?;
        let thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        let stage_id = resolve_thread_stage_id(&thread, stage_id)?;
        let status = args
            .get("status")
            .and_then(Value::as_str)
            .map(parse_stage_status)
            .transpose()?;
        let summary = optional_string_patch(args, "summary");
        let outcome = optional_string_patch(args, "outcome");
        match self
            .inner
            .store
            .update_thread_stage_state(&stage_id, status, summary, outcome)
        {
            Ok(stage) => {
                let task_id = args
                    .get("taskId")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
                let stage_id = stage.id.clone();
                let stage_completed = stage.status == StageStatus::Completed;
                let (next, _) = self.mutate_run(&run.run_id, move |next| {
                    if stage_completed {
                        if let Some(task_id) = task_id.as_deref() {
                            if !next.completed_task_ids.iter().any(|id| id == task_id) {
                                next.completed_task_ids.push(task_id.to_string());
                            }
                        }
                    }
                    next.current_stage_id = Some(stage_id);
                    Ok(())
                })?;
                let result = AstraStageMutationResult {
                    ok: true,
                    stage: Some(serde_json::to_value(stage)?),
                    issue: None,
                    error: None,
                    applied_at: now_ms(),
                };
                self.emit(&next, "stage_update_result", serde_json::to_value(&result)?);
                Ok(result)
            }
            Err(error) => Ok(AstraStageMutationResult {
                ok: false,
                stage: None,
                issue: None,
                error: Some(error.to_string()),
                applied_at: now_ms(),
            }),
        }
    }

    fn apply_issue_decision(
        &self,
        run: &AstraRun,
        args: &Value,
    ) -> Result<AstraStageMutationResult> {
        if let Some(rejection) = inactive_run_mutation_error(run) {
            return Ok(rejection);
        }
        match self.add_or_update_issue(run, args) {
            Ok(issue) => {
                let result = AstraStageMutationResult {
                    ok: true,
                    stage: None,
                    issue: Some(serde_json::to_value(issue)?),
                    error: None,
                    applied_at: now_ms(),
                };
                self.emit(run, "stage_update_result", serde_json::to_value(&result)?);
                Ok(result)
            }
            Err(error) => Ok(AstraStageMutationResult {
                ok: false,
                stage: None,
                issue: None,
                error: Some(error.to_string()),
                applied_at: now_ms(),
            }),
        }
    }

    fn add_or_update_issue(&self, run: &AstraRun, args: &Value) -> Result<StageIssueInfo> {
        let stage_id = args
            .get("threadStageId")
            .or_else(|| args.get("stageId"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("threadStageId is required"))?;
        let thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        let stage_id = resolve_thread_stage_id(&thread, stage_id)?;
        let title = args
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("issue title is required"))?;
        let description = args.get("description").and_then(Value::as_str);
        let severity = args
            .get("severity")
            .and_then(Value::as_str)
            .map(parse_issue_severity)
            .transpose()?
            .unwrap_or(IssueSeverity::Medium);
        let existing = self
            .inner
            .store
            .list_thread_stage_issues(&stage_id)?
            .into_iter()
            .find(|issue| issue.title.eq_ignore_ascii_case(title));
        if let Some(existing) = existing {
            self.inner.store.update_thread_stage_issue(
                &existing.id,
                Some(title),
                Some(description),
                Some(IssueStatus::Open),
                Some(severity),
            )
        } else {
            self.inner
                .store
                .create_thread_stage_issue(&stage_id, title, description, severity)
        }
    }

    fn apply_protocol_event(&self, event: AstraProtocolEvent) -> Result<()> {
        let event_type = event.params.event_type.clone();
        let event_data = event.params.data.clone();
        let Some((run, emit)) = self.mutate_existing_run(&event.params.run_id, move |run| {
            let mut emit = Some((event_type.clone(), event_data.clone()));
            match event_type.as_str() {
                "plan" => {
                    if !run.status.active() {
                        return Ok(None);
                    }
                    if let Some(tasks) = event_data.get("tasks") {
                        let tasks = serde_json::from_value::<Vec<AstraTaskProposal>>(tasks.clone())
                            .unwrap_or_default();
                        for task in tasks {
                            if !run
                                .proposed_tasks
                                .iter()
                                .any(|existing| existing.id == task.id)
                            {
                                run.proposed_tasks.push(task);
                            }
                        }
                    }
                }
                "status" => {
                    if !run.status.active() {
                        return Ok(None);
                    }
                    if let Some(status) = event_data
                        .get("status")
                        .and_then(Value::as_str)
                        .and_then(AstraRunStatus::from_db_str)
                    {
                        run.status = status;
                        if run.status.active() {
                            run.error = None;
                        }
                    }
                }
                "cancelled" => run.status = AstraRunStatus::Cancelled,
                "error" => {
                    run.status = AstraRunStatus::Errored;
                    run.error = event_data
                        .get("message")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                }
                "complete" => {
                    let status = event_data.get("status").and_then(Value::as_str);
                    if status == Some("cancelled") {
                        run.status = AstraRunStatus::Cancelled;
                    } else {
                        run.status = AstraRunStatus::Completed;
                        run.error = None;
                    }
                }
                _ => emit = None,
            }
            Ok(emit)
        })?
        else {
            return Ok(());
        };
        if let Some((emit_type, emit_data)) = emit {
            self.emit(&run, &emit_type, emit_data);
        }
        Ok(())
    }

    fn update_status(
        &self,
        run_id: &str,
        status: AstraRunStatus,
        error: Option<String>,
    ) -> Result<AstraRun> {
        let (run, _) = self.mutate_run(run_id, move |run| {
            run.status = status;
            run.error = error;
            Ok(())
        })?;
        Ok(run)
    }

    fn update_active_status(
        &self,
        run_id: &str,
        status: AstraRunStatus,
        error: Option<String>,
    ) -> Result<(AstraRun, bool)> {
        self.mutate_run(run_id, move |run| {
            if !run.status.active() {
                return Ok(false);
            }
            run.status = status;
            run.error = error;
            Ok(true)
        })
    }

    fn mutate_existing_run<F, T>(&self, run_id: &str, mutate: F) -> Result<Option<(AstraRun, T)>>
    where
        F: FnOnce(&mut AstraRun) -> Result<T>,
    {
        let _guard = self
            .inner
            .run_write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Astra run write lock poisoned"))?;
        let Some(record) = self.inner.store.get_astra_run(run_id)? else {
            return Ok(None);
        };
        let mut run = record_to_run(record)?;
        let value = mutate(&mut run)?;
        run.updated_at = now_ms();
        self.inner.store.upsert_astra_run(&run_to_record(&run))?;
        Ok(Some((run, value)))
    }

    /// Serialize a read-modify-write cycle on a single Astra run row. The
    /// confirm worker thread, the sidecar tool thread, and the runtime-event
    /// thread all mutate the same run; without this lock they each load, change
    /// different fields, and clobber one another on the full-row upsert (losing
    /// attempt counts, delegated session ids, or task results). The closure runs
    /// against the freshly loaded run; updated_at and the upsert are handled here.
    fn mutate_run<F, T>(&self, run_id: &str, mutate: F) -> Result<(AstraRun, T)>
    where
        F: FnOnce(&mut AstraRun) -> Result<T>,
    {
        let _guard = self
            .inner
            .run_write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("Astra run write lock poisoned"))?;
        let mut run = self.load_run(run_id)?;
        let value = mutate(&mut run)?;
        run.updated_at = now_ms();
        self.inner.store.upsert_astra_run(&run_to_record(&run))?;
        Ok((run, value))
    }

    fn load_run(&self, run_id: &str) -> Result<AstraRun> {
        self.inner
            .store
            .get_astra_run(run_id)?
            .map(record_to_run)
            .transpose()?
            .ok_or_else(|| anyhow::anyhow!("Astra run not found: {run_id}"))
    }

    fn emit(&self, run: &AstraRun, event_type: &str, data: Value) {
        let _ = self.inner.app.emit(
            ASTRA_EVENT_NAME,
            AstraEvent {
                run_id: run.run_id.clone(),
                thread_id: run.thread_id.clone(),
                status: run.status,
                event_type: event_type.to_string(),
                data,
                timestamp: now_ms(),
            },
        );
    }
}

fn astra_command(app: &AppHandle) -> Result<Command> {
    if cfg!(debug_assertions) {
        let sidecar = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("sidecars")
            .join("sessio-astra");
        let mut command = Command::new("bun");
        command
            .arg("run")
            .arg("src/main.ts")
            .arg("--stdio")
            .current_dir(sidecar);
        return Ok(command);
    }

    let sidecar_name = if cfg!(windows) {
        "sessio-astra.exe"
    } else {
        "sessio-astra"
    };
    let exe = app
        .path()
        .resolve(sidecar_name, tauri::path::BaseDirectory::Resource)
        .context("resolve Astra sidecar resource")?;
    let mut command = Command::new(exe);
    command.arg("--stdio");
    Ok(command)
}

fn record_and_link_ready_delegated_session(
    store: &dyn SessionStore,
    run: &AstraRun,
    agent: Agent,
    agent_session_id: &str,
    task: &AstraTaskProposal,
    resolved_thread_stage_id: Option<&str>,
    stage_task_context: Option<&StageTaskContext>,
) -> Result<()> {
    let project_name = store
        .list_projects()?
        .into_iter()
        .find(|project| project.id == run.project_id)
        .map(|project| project.name)
        .unwrap_or_else(|| run.project_id.clone());
    let now = now_ms();
    let first_user_message = stage_task_context
        .map(|context| context.prompt.clone())
        .or_else(|| Some(task.prompt.clone()));
    let rename_title = stage_task_context
        .map(|context| context.session_title())
        .unwrap_or_else(|| format!("Astra: {}", task.title));
    let session = SessionInfo {
        id: agent_session_id.to_string(),
        agent,
        forked_from_agent: None,
        forked_from_id: None,
        project_path: Some(run.project_path.clone()),
        project_name: Some(project_name),
        started_at: Some(now),
        updated_at: Some(now),
        message_count: 0,
        rename_title: Some(rename_title),
        title: None,
        first_user_message,
        file_path: String::new(),
        file_size: 0,
        partial: true,
        available: true,
        archived: false,
        subagents: Vec::new(),
    };
    store.upsert_session(&session.file_path, &session)?;

    if let Some(stage_id) = resolved_thread_stage_id {
        store
            .link_stage_session(stage_id, agent, agent_session_id)
            .with_context(|| {
                format!(
                    "link Astra delegated session {} to stage {stage_id}",
                    agent_session_id
                )
            })?;
    } else {
        store
            .link_thread_session(&run.thread_id, agent, agent_session_id)
            .with_context(|| {
                format!(
                    "link Astra delegated session {} to thread {}",
                    agent_session_id, run.thread_id
                )
            })?;
    }
    if let Some(context) = stage_task_context {
        save_stage_task_work_snapshot(
            store,
            agent,
            agent_session_id,
            context,
            resolved_thread_stage_id,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct StageTaskContext {
    thread_id: String,
    thread_goal: String,
    stage_name: String,
    snapshot: Value,
    prompt: String,
}

impl StageTaskContext {
    fn session_title(&self) -> String {
        format!("Astra: {}-{}", self.stage_name, self.thread_goal)
    }
}

fn build_stage_task_context(
    thread: &ThreadInfo,
    thread_stage_id: &str,
    task: &AstraTaskProposal,
) -> Result<StageTaskContext> {
    let stage = thread
        .stages
        .iter()
        .find(|stage| stage.id == thread_stage_id)
        .ok_or_else(|| {
            anyhow::anyhow!("stage does not belong to Astra run thread: {thread_stage_id}")
        })?;
    let snapshot = build_stage_task_snapshot(thread, stage);
    let prompt = render_stage_task_prompt(thread, stage, &snapshot, task);
    Ok(StageTaskContext {
        thread_id: thread.id.clone(),
        thread_goal: thread.goal.clone(),
        stage_name: stage_label(stage),
        snapshot,
        prompt,
    })
}

fn save_stage_task_work_snapshot(
    store: &dyn SessionStore,
    agent: Agent,
    agent_session_id: &str,
    context: &StageTaskContext,
    resolved_thread_stage_id: Option<&str>,
) -> Result<()> {
    let snapshot_json = serde_json::to_string(&context.snapshot)?;
    store.save_thread_work_snapshot(&ThreadWorkSnapshotRecord {
        child_agent: agent,
        child_session_id: agent_session_id.to_string(),
        thread_id: context.thread_id.clone(),
        stage_id: resolved_thread_stage_id.map(ToString::to_string),
        snapshot_json,
        version: 1,
        created_at: now_ms(),
    })
}

/// Mutation tools may only change state while the run is actively orchestrating.
/// A cancelled, completed, errored, or interrupted run must not have its stages
/// or issues mutated by a late or stray sidecar tool call.
fn inactive_run_mutation_error(run: &AstraRun) -> Option<AstraStageMutationResult> {
    if matches!(
        run.status,
        AstraRunStatus::Dispatching | AstraRunStatus::Running
    ) {
        None
    } else {
        Some(AstraStageMutationResult {
            ok: false,
            stage: None,
            issue: None,
            error: Some(format!("Astra run is not active: {}", run.status.as_str())),
            applied_at: now_ms(),
        })
    }
}

fn is_persistable_agent_session_id(agent_runtime_session_id: &str) -> bool {
    !agent_runtime_session_id.trim().is_empty()
        && !agent_runtime_session_id.starts_with("fake-agent-session")
}

fn build_stage_task_snapshot(
    thread: &ThreadInfo,
    focused_stage: &crate::models::StageInfo,
) -> Value {
    let mut stages = thread.stages.clone();
    stages.sort_by_key(|stage| stage.order);
    let current_stage_label = stages
        .iter()
        .find(|stage| stage.id == focused_stage.id)
        .map(stage_label)
        .unwrap_or_else(|| stage_label(focused_stage));
    let completed = stages
        .iter()
        .filter(|stage| matches!(stage.status, StageStatus::Completed | StageStatus::Skipped))
        .count();
    let blocked = stages
        .iter()
        .filter(|stage| stage.status == StageStatus::Blocked)
        .count();
    let open_issues = stages
        .iter()
        .map(|stage| {
            stage
                .issues
                .iter()
                .filter(|issue| issue.status == IssueStatus::Open)
                .count()
        })
        .sum::<usize>();
    let thread_session_refs = thread
        .sessions
        .iter()
        .map(|session| session_ref_json(session, "thread"))
        .collect::<Vec<_>>();
    let stage_values = stages
        .iter()
        .map(|stage| {
            let session_refs = stage
                .sessions
                .iter()
                .map(|session| session_ref_json(session, "stage"))
                .collect::<Vec<_>>();
            json!({
                "threadStageId": stage.id,
                "projectStageId": stage.stage_id,
                "name": stage_label(stage),
                "kind": stage.kind,
                "icon": stage.icon,
                "status": stage.status,
                "summary": stage.summary,
                "outcome": stage.outcome,
                "assistants": stage.assistants,
                "issues": stage.issues,
                "sessionRefs": session_refs,
            })
        })
        .collect::<Vec<_>>();
    let stage_session_refs = stages
        .iter()
        .flat_map(|stage| {
            stage
                .sessions
                .iter()
                .map(|session| session_ref_json(session, "stage"))
        })
        .collect::<Vec<_>>();
    let all_session_refs = dedupe_session_ref_values(
        thread_session_refs
            .iter()
            .cloned()
            .chain(stage_session_refs.iter().cloned())
            .collect(),
    );
    json!({
        "threadId": thread.id,
        "projectId": thread.project_id,
        "goal": thread.goal,
        "description": thread.description,
        "activeStageId": thread.stage_id,
        "focusedStageId": focused_stage.id,
        "stages": stage_values,
        "threadSessionRefs": thread_session_refs,
        "relatedContext": {
            "sessionExcerptRefs": all_session_refs,
        },
        "detailRefs": {
            "threadId": thread.id,
            "focusedStageId": focused_stage.id,
            "stageIds": stages.iter().map(|stage| stage.id.clone()).collect::<Vec<_>>(),
            "issueIds": stages
                .iter()
                .flat_map(|stage| stage.issues.iter().map(|issue| issue.id.clone()))
                .collect::<Vec<_>>(),
            "sessionRefs": all_session_refs,
        },
        "rollup": {
            "completed": completed,
            "incomplete": stages.len().saturating_sub(completed),
            "blocked": blocked,
            "openIssues": open_issues,
            "currentStage": current_stage_label,
            "total": stages.len(),
        },
        "capturedAt": now_ms(),
    })
}

fn render_stage_task_prompt(
    thread: &ThreadInfo,
    focused_stage: &crate::models::StageInfo,
    snapshot: &Value,
    task: &AstraTaskProposal,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio stage task".to_string());
    lines.push(String::new());
    lines.push("You are working on a delegated stage task from Astra. Treat this as a Sessio stage chat, not a general thread chat.".to_string());
    lines.push(format!("Thread goal: {}", thread.goal));
    if let Some(description) = thread
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thread description: {description}"));
    }
    lines.push(format!("Target threadStageId: {}", focused_stage.id));
    lines.push(format!("Target stage: {}", stage_label(focused_stage)));
    if let Some(description) = focused_stage
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Stage description: {description}"));
    }
    if let Some(summary) = focused_stage
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Current stage summary: {summary}"));
    }
    if let Some(outcome) = focused_stage
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Current stage outcome: {outcome}"));
    }
    let completed = snapshot
        .pointer("/rollup/completed")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = snapshot
        .pointer("/rollup/total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let blocked = snapshot
        .pointer("/rollup/blocked")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let open_issues = snapshot
        .pointer("/rollup/openIssues")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    lines.push(format!(
        "Thread progress: {completed}/{total} stages complete, {blocked} blocked, {open_issues} open issues"
    ));
    lines.push(String::new());
    lines.push("## Stage work snapshot".to_string());
    if let Some(stages) = snapshot.get("stages").and_then(Value::as_array) {
        for stage in stages {
            let id = stage
                .get("threadStageId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let name = stage.get("name").and_then(Value::as_str).unwrap_or(id);
            let status = stage
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("not_started");
            let focus = if id == focused_stage.id {
                " <- you are here"
            } else {
                ""
            };
            lines.push(format!("- [{}] {name}{focus}", status_label(status)));
            if let Some(summary) = stage
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    summary: {summary}"));
            }
            if let Some(outcome) = stage
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    outcome: {outcome}"));
            }
            if let Some(issues) = stage.get("issues").and_then(Value::as_array) {
                for issue in issues {
                    if issue.get("status").and_then(Value::as_str) != Some("open") {
                        continue;
                    }
                    let severity = issue
                        .get("severity")
                        .and_then(Value::as_str)
                        .unwrap_or("medium");
                    let title = issue
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("issue");
                    lines.push(format!("    issue [{severity}] {title}"));
                    if let Some(description) = issue
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        lines.push(format!("      {description}"));
                    }
                }
            }
            if let Some(session_refs) = stage.get("sessionRefs").and_then(Value::as_array) {
                for reference in session_refs {
                    let agent = reference
                        .get("agent")
                        .and_then(Value::as_str)
                        .unwrap_or("agent");
                    let session_id = reference
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or("session");
                    let title = reference.get("title").and_then(Value::as_str).unwrap_or("");
                    lines.push(
                        format!("    [{agent}:{session_id}] {title}")
                            .trim_end()
                            .to_string(),
                    );
                }
            }
        }
    }
    lines.push(String::new());
    lines.push("## Astra task".to_string());
    lines.push(format!("Task title: {}", task.title));
    lines.push(format!("Expected output: {}", task.expected_output));
    lines.push(String::new());
    lines.push(task.prompt.clone());
    lines.push(String::new());
    lines.push("## Reporting".to_string());
    lines.push("Return a concise final result for Astra. Astra will decide status, summary, and outcome, then ask Sessio to update thread_stage_states.".to_string());
    lines.push("Do not mark unrelated stages complete.".to_string());
    lines.join("\n")
}

fn session_ref_json(session: &SessionInfo, source_kind: &str) -> Value {
    json!({
        "agent": session.agent,
        "sessionId": session.id,
        "title": session
            .rename_title
            .as_deref()
            .or(session.title.as_deref())
            .or(session.first_user_message.as_deref()),
        "filePath": if session.file_path.is_empty() { None::<&str> } else { Some(session.file_path.as_str()) },
        "sourceKind": source_kind,
    })
}

fn dedupe_session_ref_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let Some(agent) = value.get("agent").and_then(Value::as_str) else {
            continue;
        };
        let Some(session_id) = value.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        if seen.insert(format!("{agent}:{session_id}")) {
            out.push(value);
        }
    }
    out
}

fn stage_label(stage: &crate::models::StageInfo) -> String {
    stage
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| stage.kind.map(|kind| kind.as_str().to_string()))
        .unwrap_or_else(|| stage.stage_id.clone())
}

fn status_label(status: &str) -> &'static str {
    match status {
        "completed" => "done",
        "in_progress" => "active",
        "blocked" => "blocked",
        "needs_review" => "needs review",
        "skipped" => "skipped",
        _ => "not started",
    }
}

fn hydrate_start_request_for_astra(
    req: &mut StartAgentSession,
    store: &dyn SessionStore,
) -> Result<()> {
    let Some(agent) = store
        .list_agents()?
        .into_iter()
        .find(|agent| agent.id == req.agent.as_str())
    else {
        return Ok(());
    };
    insert_option_if_missing(&mut req.options, "model", agent.model);
    insert_option_if_missing(&mut req.options, "effort", agent.effort);
    insert_option_if_missing(&mut req.options, "permissionMode", agent.permission_mode);
    insert_option_if_missing(
        &mut req.options,
        "transport",
        Some(runtime_transport_option(agent.transport)),
    );
    if !req.options.contains_key("command") && !req.options.contains_key("acpCommand") {
        if let Some(command) = agent.commands.session.first().cloned() {
            insert_option_if_missing(&mut req.options, "command", Some(command));
        }
    }
    Ok(())
}

fn insert_option_if_missing(options: &mut RuntimeMetadata, key: &str, value: Option<String>) {
    if options.contains_key(key) {
        return;
    }
    if let Some(value) = value.map(|value| value.trim().to_string()) {
        if !value.is_empty() {
            options.insert(key.to_string(), Value::String(value));
        }
    }
}

fn runtime_transport_option(
    transport: crate::agents::runtime::types::RuntimeTransportKind,
) -> String {
    match transport {
        crate::agents::runtime::types::RuntimeTransportKind::Acp => "acp",
        crate::agents::runtime::types::RuntimeTransportKind::CliStreamJson => "cliStreamJson",
        crate::agents::runtime::types::RuntimeTransportKind::PlainCli => "plainCli",
        crate::agents::runtime::types::RuntimeTransportKind::Fake => "fake",
    }
    .to_string()
}

fn run_to_handle(run: AstraRun) -> AstraHandle {
    AstraHandle {
        run_id: run.run_id,
        thread_id: run.thread_id,
        project_id: run.project_id,
        status: run.status,
        proposed_tasks: run.proposed_tasks,
        approved_task_ids: run.approved_task_ids,
        delegated_session_ids: run.delegated_session_ids,
        task_results: run.task_results,
        mode: run.mode,
        current_stage_id: run.current_stage_id,
        completed_task_ids: run.completed_task_ids,
        stage_attempt_counts: run.stage_attempt_counts,
        retry_limit: run.retry_limit,
        error: run.error,
        created_at: run.created_at,
        updated_at: run.updated_at,
    }
}

fn run_to_record(run: &AstraRun) -> AstraRunRecord {
    AstraRunRecord {
        run_id: run.run_id.clone(),
        thread_id: run.thread_id.clone(),
        project_id: run.project_id.clone(),
        project_path: run.project_path.clone(),
        status: run.status.as_str().to_string(),
        mode: run.mode.clone(),
        proposed_tasks_json: serde_json::to_string(&run.proposed_tasks)
            .unwrap_or_else(|_| "[]".to_string()),
        approved_task_ids_json: serde_json::to_string(&run.approved_task_ids)
            .unwrap_or_else(|_| "[]".to_string()),
        delegated_session_ids_json: serde_json::to_string(&run.delegated_session_ids)
            .unwrap_or_else(|_| "[]".to_string()),
        task_results_json: serde_json::to_string(&run.task_results)
            .unwrap_or_else(|_| "[]".to_string()),
        current_stage_id: run.current_stage_id.clone(),
        completed_task_ids_json: serde_json::to_string(&run.completed_task_ids)
            .unwrap_or_else(|_| "[]".to_string()),
        stage_attempt_counts_json: serde_json::to_string(&run.stage_attempt_counts)
            .unwrap_or_else(|_| "{}".to_string()),
        retry_limit: i64::from(run.retry_limit),
        error: run.error.clone(),
        created_at: run.created_at,
        updated_at: run.updated_at,
    }
}

fn record_to_run(record: AstraRunRecord) -> Result<AstraRun> {
    Ok(AstraRun {
        run_id: record.run_id,
        thread_id: record.thread_id,
        project_id: record.project_id,
        project_path: record.project_path,
        status: AstraRunStatus::from_db_str(&record.status).unwrap_or(AstraRunStatus::Errored),
        proposed_tasks: serde_json::from_str(&record.proposed_tasks_json).unwrap_or_default(),
        approved_task_ids: serde_json::from_str(&record.approved_task_ids_json).unwrap_or_default(),
        delegated_session_ids: serde_json::from_str(&record.delegated_session_ids_json)
            .unwrap_or_default(),
        task_results: serde_json::from_str(&record.task_results_json).unwrap_or_default(),
        mode: record.mode,
        current_stage_id: record.current_stage_id,
        completed_task_ids: serde_json::from_str(&record.completed_task_ids_json)
            .unwrap_or_default(),
        stage_attempt_counts: serde_json::from_str(&record.stage_attempt_counts_json)
            .unwrap_or_default(),
        retry_limit: u32::try_from(record.retry_limit)
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or(3),
        error: record.error,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn upsert_task_result_in_run(run: &mut AstraRun, result: AstraTaskResult) {
    let key = (
        result.task_id.clone(),
        result.sessio_runtime_session_id.clone(),
    );
    if let Some(existing) = run
        .task_results
        .iter_mut()
        .find(|existing| existing.task_id == key.0 && existing.sessio_runtime_session_id == key.1)
    {
        *existing = result;
    } else {
        run.task_results.push(result);
    }
}

fn project_snapshot(project: &ProjectInfo, thread: &ThreadInfo) -> Value {
    json!({
        "project": project,
        "thread": thread,
        "activeStage": thread.stage_id.as_ref().and_then(|id| thread.stages.iter().find(|stage| stage.id == *id || stage.stage_id == *id)),
        "tools": [
            "sessio.project.snapshot",
            "sessio.memory.search",
            "sessio.agent.plan_task",
            "sessio.agent.dispatch_task",
            "sessio.stage.update",
            "sessio.stage.issue.add_or_update"
        ]
    })
}

fn resolve_thread_stage_id(thread: &ThreadInfo, stage_id: &str) -> Result<String> {
    thread
        .stages
        .iter()
        .find(|stage| stage.id == stage_id || stage.stage_id == stage_id)
        .map(|stage| stage.id.clone())
        .ok_or_else(|| anyhow::anyhow!("stage does not belong to Astra run thread: {stage_id}"))
}

fn optional_string_patch(args: &Value, key: &str) -> Option<Option<String>> {
    if !args
        .as_object()
        .map(|object| object.contains_key(key))
        .unwrap_or(false)
    {
        return None;
    }
    Some(
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    )
}

fn parse_stage_status(value: &str) -> Result<StageStatus> {
    StageStatus::from_db_str(value).ok_or_else(|| anyhow::anyhow!("unknown stage status: {value}"))
}

fn parse_issue_severity(value: &str) -> Result<IssueSeverity> {
    IssueSeverity::from_db_str(value)
        .ok_or_else(|| anyhow::anyhow!("unknown issue severity: {value}"))
}

fn stable_run_id(thread_id: &str, now: i64) -> String {
    format!("astra-{}-{}", short_hash(thread_id), now)
}

fn extract_result_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Array(items) => {
            let text = items
                .iter()
                .filter_map(extract_result_text)
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Value::Object(object) => {
            for key in ["text", "content", "output", "message", "summary"] {
                if let Some(text) = object.get(key).and_then(extract_result_text) {
                    return Some(text);
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
fn thread_all_stages_terminal(thread: &ThreadInfo) -> bool {
    !thread.stages.is_empty()
        && thread
            .stages
            .iter()
            .all(|stage| matches!(stage.status, StageStatus::Completed | StageStatus::Skipped))
}

fn short_hash(input: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut hasher);
    format!("{:x}", hasher.finish())
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
    use crate::store::sqlite::SqliteStore;
    use std::path::Path;

    #[test]
    fn astra_status_roundtrip() {
        for status in [
            AstraRunStatus::Planning,
            AstraRunStatus::AwaitingApproval,
            AstraRunStatus::Dispatching,
            AstraRunStatus::Running,
            AstraRunStatus::Completed,
            AstraRunStatus::Cancelled,
            AstraRunStatus::Errored,
            AstraRunStatus::Interrupted,
        ] {
            assert_eq!(AstraRunStatus::from_db_str(status.as_str()), Some(status));
        }
    }

    #[test]
    fn protocol_decoding_rejects_missing_version() {
        let value = json!({ "id": "1", "method": "astra/start", "params": {} });
        assert!(serde_json::from_value::<AstraProtocolRequest>(value).is_err());
    }

    #[test]
    fn extracts_runtime_result_text_from_common_shapes() {
        assert_eq!(
            extract_result_text(&json!({ "content": [{ "text": "hello" }, { "text": " world" }] })),
            Some("hello world".to_string())
        );
        assert_eq!(
            extract_result_text(&json!({ "output": "done" })),
            Some("done".to_string())
        );
    }

    #[test]
    fn task_result_upsert_preserves_unrelated_run_metadata() {
        let mut run = test_run("run-merge");
        run.delegated_session_ids = vec!["session-1".to_string()];
        run.stage_attempt_counts = HashMap::from([("stage-1".to_string(), 2)]);
        run.task_results = vec![test_task_result("task-1", "session-1", "first")];

        upsert_task_result_in_run(&mut run, test_task_result("task-2", "session-2", "second"));

        assert_eq!(run.delegated_session_ids, vec!["session-1"]);
        assert_eq!(run.stage_attempt_counts["stage-1"], 2);
        assert_eq!(run.task_results.len(), 2);
        assert!(run
            .task_results
            .iter()
            .any(|result| result.task_id == "task-1"));
        assert!(run
            .task_results
            .iter()
            .any(|result| result.task_id == "task-2"));
    }

    #[test]
    fn task_result_upsert_replaces_same_task_session_pair() {
        let mut run = test_run("run-upsert");
        run.task_results = vec![test_task_result("task-1", "session-1", "old")];

        upsert_task_result_in_run(&mut run, test_task_result("task-1", "session-1", "new"));

        assert_eq!(run.task_results.len(), 1);
        assert_eq!(run.task_results[0].output, "new");
    }

    #[test]
    fn resolves_project_or_thread_stage_id_to_thread_stage_id() {
        let thread = ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Ship".to_string(),
            description: None,
            stage_id: None,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            stages: vec![crate::models::StageInfo {
                id: "thread-stage-1".to_string(),
                thread_id: "thread-1".to_string(),
                stage_id: "project-stage-1".to_string(),
                project_id: "project-1".to_string(),
                assistant_ids: Vec::new(),
                assistants: Vec::new(),
                stage_type: crate::models::ProjectStageType::Custom,
                workflow_id: None,
                kind: None,
                name: Some("Build".to_string()),
                description: None,
                icon: None,
                order: 0,
                status: StageStatus::NotStarted,
                summary: None,
                outcome: None,
                enabled: true,
                allow_empty_assistants: true,
                created_at: 1,
                updated_at: 1,
                sessions: Vec::new(),
                issues: Vec::new(),
            }],
            sessions: Vec::new(),
        };

        assert_eq!(
            resolve_thread_stage_id(&thread, "thread-stage-1").unwrap(),
            "thread-stage-1"
        );
        assert_eq!(
            resolve_thread_stage_id(&thread, "project-stage-1").unwrap(),
            "thread-stage-1"
        );
        assert!(resolve_thread_stage_id(&thread, "other-stage").is_err());
    }

    #[test]
    fn records_delegated_stage_session_under_stage() {
        let db_path = std::env::temp_dir().join(format!(
            "sessio-astra-stage-link-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "sessio-astra-stage-project-{}",
            short_hash(&db_path.to_string_lossy())
        ));
        std::fs::create_dir_all(&parent).unwrap();
        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "Astra Stage Link",
                "general".to_string(),
                None,
            )
            .unwrap();
        let stage_option = store
            .create_project_stage(&project.id, None, "Build", None, None)
            .unwrap();
        store
            .update_project_stage(&stage_option.id, None, None, None, None, None, Some(true))
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Ship Astra stage task", None)
            .unwrap();
        let stage = store
            .add_thread_stage(&thread.id, &stage_option.id, &[])
            .unwrap();
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Build stage worker".to_string(),
            target_stage_id: Some(stage.id.clone()),
            target_agent: Agent::Codex,
            prompt: "Do the stage work.".to_string(),
            expected_output: "Stage result.".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let run = AstraRun {
            run_id: "astra-run-stage-link".to_string(),
            thread_id: thread.id.clone(),
            project_id: project.id.clone(),
            project_path: project.path.clone(),
            status: AstraRunStatus::Running,
            proposed_tasks: Vec::new(),
            approved_task_ids: Vec::new(),
            delegated_session_ids: Vec::new(),
            task_results: Vec::new(),
            mode: "auto".to_string(),
            current_stage_id: Some(stage.id.clone()),
            completed_task_ids: Vec::new(),
            stage_attempt_counts: HashMap::new(),
            retry_limit: 3,
            error: None,
            created_at: 1,
            updated_at: 1,
        };

        record_and_link_ready_delegated_session(
            &store,
            &run,
            Agent::Codex,
            "agent-session-real",
            &task,
            Some(&stage.id),
            None,
        )
        .unwrap();

        let updated = store.get_thread_work_state(&thread.id).unwrap();
        assert!(updated.sessions.is_empty());
        let updated_stage = updated
            .stages
            .iter()
            .find(|item| item.id == stage.id)
            .unwrap();
        assert_eq!(updated_stage.sessions.len(), 1);
        assert_eq!(updated_stage.sessions[0].id, "agent-session-real");
        assert_eq!(
            updated_stage.sessions[0].rename_title.as_deref(),
            Some("Astra: Build stage worker")
        );
        assert_eq!(updated_stage.sessions[0].title, None);
        assert_eq!(updated_stage.sessions[0].file_path, "");
        assert!(updated_stage.sessions[0].partial);
        assert!(store
            .get_thread_work_snapshot(Agent::Codex, "runtime-stage-session")
            .unwrap()
            .is_none());

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(Path::new(&project.path));
    }

    #[test]
    fn saves_stage_task_thread_work_snapshot_for_delegated_session() {
        let db_path = std::env::temp_dir().join(format!(
            "sessio-astra-work-snapshot-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "sessio-astra-work-snapshot-project-{}",
            short_hash(&db_path.to_string_lossy())
        ));
        std::fs::create_dir_all(&parent).unwrap();
        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "Astra Work Snapshot",
                "general".to_string(),
                None,
            )
            .unwrap();
        let stage_option = store
            .create_project_stage(
                &project.id,
                None,
                "Implement",
                Some("Build the focused feature."),
                None,
            )
            .unwrap();
        store
            .update_project_stage(&stage_option.id, None, None, None, None, None, Some(true))
            .unwrap();
        let thread = store
            .create_thread(
                &project.id,
                "Ship focused worker",
                Some("Keep Astra stage scoped."),
            )
            .unwrap();
        let stage = store
            .add_thread_stage(&thread.id, &stage_option.id, &[])
            .unwrap();
        let thread = store.get_thread_work_state(&thread.id).unwrap();
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Implement focused worker".to_string(),
            target_stage_id: Some(stage.id.clone()),
            target_agent: Agent::Codex,
            prompt: "Do the stage work.".to_string(),
            expected_output: "Focused worker result.".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let context = build_stage_task_context(&thread, &stage.id, &task).unwrap();

        save_stage_task_work_snapshot(
            &store,
            Agent::Codex,
            "agent-stage-snapshot",
            &context,
            Some(&stage.id),
        )
        .unwrap();

        let saved = store
            .get_thread_work_snapshot(Agent::Codex, "agent-stage-snapshot")
            .unwrap()
            .unwrap();
        assert_eq!(saved.thread_id, thread.id);
        assert_eq!(saved.stage_id.as_deref(), Some(stage.id.as_str()));
        let snapshot: Value = serde_json::from_str(&saved.snapshot_json).unwrap();
        assert_eq!(snapshot["focusedStageId"], Value::String(stage.id.clone()));
        assert_eq!(
            snapshot["description"],
            Value::String("Keep Astra stage scoped.".to_string())
        );
        assert_eq!(
            snapshot["rollup"]["currentStage"],
            Value::String("Implement".to_string())
        );

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(Path::new(&project.path));
    }

    #[test]
    fn promotes_delegated_runtime_session_to_agent_session_identity() {
        let db_path = std::env::temp_dir().join(format!(
            "sessio-astra-session-promote-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "sessio-astra-session-promote-project-{}",
            short_hash(&db_path.to_string_lossy())
        ));
        std::fs::create_dir_all(&parent).unwrap();
        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "Astra Session Promote",
                "general".to_string(),
                None,
            )
            .unwrap();
        let stage_option = store
            .create_project_stage(&project.id, None, "Research", None, None)
            .unwrap();
        store
            .update_project_stage(&stage_option.id, None, None, None, None, None, Some(true))
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Promote session", None)
            .unwrap();
        let stage = store
            .add_thread_stage(&thread.id, &stage_option.id, &[])
            .unwrap();
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Research stage".to_string(),
            target_stage_id: Some(stage.id.clone()),
            target_agent: Agent::Codex,
            prompt: "Research the stage.".to_string(),
            expected_output: "Research summary.".to_string(),
            risk: AstraTaskRisk::Low,
        };
        store
            .update_thread_stage_state(&stage.id, Some(StageStatus::InProgress), None, None)
            .unwrap();
        let thread_state = store.get_thread_work_state(&thread.id).unwrap();
        let context = build_stage_task_context(&thread_state, &stage.id, &task).unwrap();
        assert_eq!(
            context.snapshot["stages"][0]["status"],
            Value::String("in_progress".to_string())
        );
        let run = AstraRun {
            run_id: "astra-run-ready".to_string(),
            thread_id: thread.id.clone(),
            project_id: project.id.clone(),
            project_path: project.path.clone(),
            status: AstraRunStatus::Running,
            proposed_tasks: vec![task.clone()],
            approved_task_ids: Vec::new(),
            delegated_session_ids: Vec::new(),
            task_results: Vec::new(),
            mode: "auto".to_string(),
            current_stage_id: Some(stage.id.clone()),
            completed_task_ids: Vec::new(),
            stage_attempt_counts: HashMap::new(),
            retry_limit: 3,
            error: None,
            created_at: 1,
            updated_at: 1,
        };
        store.upsert_astra_run(&run_to_record(&run)).unwrap();
        assert!(store
            .get_thread_work_snapshot(Agent::Codex, "runtime-stage-session")
            .unwrap()
            .is_none());

        record_and_link_ready_delegated_session(
            &store,
            &run,
            Agent::Codex,
            "agent-session-real",
            &task,
            Some(&stage.id),
            Some(&context),
        )
        .unwrap();

        let updated = store.get_thread_work_state(&thread.id).unwrap();
        let updated_stage = updated
            .stages
            .iter()
            .find(|item| item.id == stage.id)
            .unwrap();
        assert_eq!(updated_stage.sessions.len(), 1);
        assert_eq!(updated_stage.sessions[0].id, "agent-session-real");
        assert_eq!(updated_stage.sessions[0].file_path, "");
        assert!(updated_stage.sessions[0].partial);
        assert!(store
            .get_thread_work_snapshot(Agent::Codex, "runtime-stage-session")
            .unwrap()
            .is_none());
        assert_eq!(
            updated_stage.sessions[0].rename_title.as_deref(),
            Some("Astra: Research-Promote session")
        );
        assert_eq!(updated_stage.sessions[0].title, None);
        assert!(store
            .get_thread_work_snapshot(Agent::Codex, "agent-session-real")
            .unwrap()
            .is_some());

        let real_session = SessionInfo {
            id: "agent-session-real".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(2),
            updated_at: Some(3),
            message_count: 4,
            rename_title: None,
            title: Some("# Sessio stage task".to_string()),
            first_user_message: Some("# Sessio stage task".to_string()),
            file_path: Path::new(&project.path)
                .join("real-session.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            subagents: Vec::new(),
        };
        store
            .upsert_session(&real_session.file_path, &real_session)
            .unwrap();
        let indexed = store.get_thread_work_state(&thread.id).unwrap();
        let indexed_stage = indexed
            .stages
            .iter()
            .find(|item| item.id == stage.id)
            .unwrap();
        assert_eq!(indexed_stage.sessions.len(), 1);
        assert_eq!(indexed_stage.sessions[0].id, "agent-session-real");
        assert_eq!(indexed_stage.sessions[0].file_path, real_session.file_path);
        assert!(!indexed_stage.sessions[0].partial);
        assert_eq!(
            indexed_stage.sessions[0].rename_title.as_deref(),
            Some("Astra: Research-Promote session")
        );
        assert_eq!(
            indexed_stage.sessions[0].title.as_deref(),
            Some("# Sessio stage task")
        );
        store
            .upsert_session(&real_session.file_path, &real_session)
            .unwrap();
        let reindexed = store.get_thread_work_state(&thread.id).unwrap();
        let reindexed_stage = reindexed
            .stages
            .iter()
            .find(|item| item.id == stage.id)
            .unwrap();
        assert_eq!(
            reindexed_stage.sessions[0].file_path,
            real_session.file_path
        );
        assert!(!reindexed_stage.sessions[0].partial);
        assert_eq!(
            reindexed_stage.sessions[0].rename_title.as_deref(),
            Some("Astra: Research-Promote session")
        );
        assert_eq!(
            reindexed_stage.sessions[0].title.as_deref(),
            Some("# Sessio stage task")
        );
        let session_row_count = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .filter(|session| session.agent == Agent::Codex && session.id == "agent-session-real")
            .count();
        assert_eq!(session_row_count, 1);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(Path::new(&project.path));
    }

    #[test]
    fn stage_task_context_renders_stage_chat_snapshot() {
        let stage = crate::models::StageInfo {
            id: "thread-stage-1".to_string(),
            thread_id: "thread-1".to_string(),
            stage_id: "project-stage-1".to_string(),
            project_id: "project-1".to_string(),
            assistant_ids: Vec::new(),
            assistants: Vec::new(),
            stage_type: crate::models::ProjectStageType::Custom,
            workflow_id: None,
            kind: None,
            name: Some("Build API".to_string()),
            description: Some("Implement the API surface.".to_string()),
            icon: None,
            order: 0,
            status: StageStatus::NotStarted,
            summary: Some("Routes are scaffolded.".to_string()),
            outcome: None,
            enabled: true,
            allow_empty_assistants: true,
            created_at: 1,
            updated_at: 1,
            sessions: Vec::new(),
            issues: vec![StageIssueInfo {
                id: "issue-1".to_string(),
                thread_stage_id: "thread-stage-1".to_string(),
                title: "Need persistence check".to_string(),
                description: Some("Confirm state is stored.".to_string()),
                status: IssueStatus::Open,
                severity: IssueSeverity::High,
                created_at: 1,
                updated_at: 1,
            }],
        };
        let thread = ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Ship Astra".to_string(),
            description: Some("Close the orchestration loop.".to_string()),
            stage_id: Some(stage.id.clone()),
            enabled: true,
            created_at: 1,
            updated_at: 1,
            stages: vec![stage],
            sessions: Vec::new(),
        };
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Advance Build API".to_string(),
            target_stage_id: Some("thread-stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Implement and verify the missing API.".to_string(),
            expected_output: "Implementation summary and verification.".to_string(),
            risk: AstraTaskRisk::Medium,
        };

        let context = build_stage_task_context(&thread, "thread-stage-1", &task).unwrap();

        assert_eq!(
            context.snapshot["focusedStageId"],
            Value::String("thread-stage-1".to_string())
        );
        assert_eq!(
            context.snapshot["stages"][0]["status"],
            Value::String("not_started".to_string())
        );
        assert_eq!(
            context.snapshot["goal"],
            Value::String("Ship Astra".to_string())
        );
        assert!(context
            .prompt
            .contains("Treat this as a Sessio stage chat, not a general thread chat."));
        assert!(context.prompt.contains("Thread goal: Ship Astra"));
        assert!(context
            .prompt
            .contains("Thread description: Close the orchestration loop."));
        assert!(context
            .prompt
            .contains("Stage description: Implement the API surface."));
        assert!(context
            .prompt
            .contains("issue [high] Need persistence check"));
        assert!(context.prompt.contains("## Astra task"));
        assert!(context
            .prompt
            .contains("Implement and verify the missing API."));
    }

    #[test]
    fn thread_completion_requires_all_stages_terminal() {
        let mut thread = ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Ship the thread".to_string(),
            description: None,
            stage_id: None,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            stages: Vec::new(),
            sessions: Vec::new(),
        };
        assert!(!thread_all_stages_terminal(&thread));

        thread
            .stages
            .push(test_stage("stage-1", StageStatus::Completed));
        thread
            .stages
            .push(test_stage("stage-2", StageStatus::InProgress));
        assert!(!thread_all_stages_terminal(&thread));

        thread.stages[1].status = StageStatus::Skipped;
        assert!(thread_all_stages_terminal(&thread));
    }

    fn test_stage(id: &str, status: StageStatus) -> crate::models::StageInfo {
        crate::models::StageInfo {
            id: id.to_string(),
            thread_id: "thread-1".to_string(),
            stage_id: format!("project-{id}"),
            project_id: "project-1".to_string(),
            assistant_ids: Vec::new(),
            assistants: Vec::new(),
            stage_type: crate::models::ProjectStageType::Custom,
            workflow_id: None,
            kind: None,
            name: Some(id.to_string()),
            description: None,
            icon: None,
            order: 0,
            status,
            summary: None,
            outcome: None,
            enabled: true,
            allow_empty_assistants: true,
            created_at: 1,
            updated_at: 1,
            sessions: Vec::new(),
            issues: Vec::new(),
        }
    }

    fn test_run(run_id: &str) -> AstraRun {
        AstraRun {
            run_id: run_id.to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp".to_string(),
            status: AstraRunStatus::Running,
            proposed_tasks: Vec::new(),
            approved_task_ids: Vec::new(),
            delegated_session_ids: Vec::new(),
            task_results: Vec::new(),
            mode: "auto".to_string(),
            current_stage_id: None,
            completed_task_ids: Vec::new(),
            stage_attempt_counts: HashMap::new(),
            retry_limit: 3,
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn test_task_result(task_id: &str, session_id: &str, output: &str) -> AstraTaskResult {
        AstraTaskResult {
            task_id: task_id.to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: session_id.to_string(),
            turn_id: Some("turn-1".to_string()),
            status: AstraTaskResultStatus::Completed,
            output: output.to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            completed_at: 1,
        }
    }
}
