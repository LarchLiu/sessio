use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::agents::runtime::types::{
    AgentInput, AgentRuntimeEvent, AgentRuntimeEventPayload, AgentSessionHandle, RuntimeMetadata,
    StartAgentSession,
};
use crate::agents::runtime::RuntimeManager;
use crate::models::{
    Agent, AgentInfo, IssueSeverity, IssueStatus, SessionInfo, StageIssueInfo, StageStatus,
    ThreadInfo,
};
use crate::store::{AstraRunRecord, SessionStore, ThreadWorkSnapshotRecord};

mod backend;
mod decision;
mod deterministic_backend;
mod orchestrator;
mod pi_acp_adapter;
mod planner;
mod prompt;
mod runtime_agent_backend;
mod types;

use orchestrator::RustNativeWorkerOutcome;
use pi_acp_adapter::{
    prepare_pi_agent_config, AstraPiConfig, AstraPiProviderConfig, AstraPiPurposeConfig,
};
use planner::next_dispatchable_tasks;
use prompt::build_stage_task_context;
use types::AstraDecision;
pub use types::{
    AstraHandle, AstraPlan, AstraRun, AstraRunStatus, AstraStageMutationResult, AstraTaskProposal,
    AstraTaskResult, AstraTaskResultStatus, AstraTaskRisk,
};

pub const ASTRA_EVENT_NAME: &str = "astra-run-event";
const RUST_NATIVE_ROUND_LIMIT: u32 = 3;
const ASTRA_DEFAULT_RETRY_LIMIT: u32 = 3;
const ASTRA_PI_TIMEOUT_MS: u64 = 30_000;
const ASTRA_SESSION_DIR_NAME: &str = "astra-sessions";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAstraRunRequest {
    pub thread_id: String,
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelAstraRunRequest {
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

#[derive(Clone)]
pub struct AstraService {
    inner: Arc<AstraServiceInner>,
}

struct AstraServiceInner {
    app: AppHandle,
    store: Arc<dyn SessionStore>,
    runtime: RuntimeManager,
    pi_config: Option<AstraPiConfig>,
    astra_preferences: Mutex<AstraBackendConfig>,
    delegated_sessions: Mutex<HashMap<String, DelegatedSessionState>>,
    task_waiters: Mutex<HashMap<String, mpsc::Sender<AstraTaskResult>>>,
    orchestrator_workers: Mutex<HashMap<String, AstraWorkerState>>,
    // Serializes read-modify-write cycles on a single run row (see mutate_run).
    run_write_lock: Mutex<()>,
}

#[derive(Debug, Clone)]
struct AstraBackendConfig {
    pub planner_agent: Option<Agent>,
    pub decision_agent: Option<Agent>,
    pub provider_config: AstraPiProviderConfig,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AstraWorkerState {
    Pending,
    Running,
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

/// The stage/attempt coordinates that always travel together when dispatching
/// or tracking a delegated session.
#[derive(Clone, Copy)]
struct DelegatedAttempt<'a> {
    thread_stage_id: Option<&'a str>,
    attempt_count: u32,
    retry_limit_reached: bool,
}

fn bundled_pi_config() -> Option<AstraPiConfig> {
    let command = bundled_pi_command()?;
    log::info!("[astra:pi-acp] using bundled Pi sidecar");
    Some(AstraPiConfig {
        command,
        session_dir: astra_session_dir().to_string_lossy().to_string(),
        agent_dir: astra_agent_dir().to_string_lossy().to_string(),
        planner: AstraPiPurposeConfig {
            timeout_ms: ASTRA_PI_TIMEOUT_MS,
        },
        decision: AstraPiPurposeConfig {
            timeout_ms: ASTRA_PI_TIMEOUT_MS,
        },
    })
}

pub fn bundled_pi_acp_command() -> Option<String> {
    bundled_pi_command()
}

fn astra_session_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".sessio")
        .join(ASTRA_SESSION_DIR_NAME)
}

fn astra_agent_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".sessio")
        .join("astra-pi-agent")
}

fn bundled_pi_command() -> Option<String> {
    let executable = bundled_pi_path()?;
    if !executable.exists() {
        return None;
    }
    let session_dir = astra_session_dir();
    let agent_dir = astra_agent_dir();
    if let Err(error) = std::fs::create_dir_all(&session_dir) {
        log::warn!(
            "[astra:pi-acp] failed to create session dir {}: {error}",
            session_dir.display()
        );
        return None;
    }
    if let Err(error) = std::fs::create_dir_all(&agent_dir) {
        log::warn!(
            "[astra:pi-acp] failed to create agent dir {}: {error}",
            agent_dir.display()
        );
        return None;
    }
    Some(pi_stdio_command_json(&executable, &session_dir, &agent_dir))
}

fn pi_stdio_command_json(
    executable: &std::path::Path,
    session_dir: &std::path::Path,
    agent_dir: &std::path::Path,
) -> String {
    json!({
        "type": "stdio",
        "name": "astra",
        "command": executable,
        "args": [
            "--session-dir",
            session_dir,
            "--session-durability",
            "strict",
            "--acp",
        ],
        "env": [
            {
                "name": "PI_CODING_AGENT_DIR",
                "value": agent_dir,
            },
            {
                "name": "PI_SESSIONS_DIR",
                "value": session_dir,
            },
        ],
    })
    .to_string()
}

fn bundled_pi_path() -> Option<PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;
    let base_dir = if exe_dir.ends_with("deps") {
        exe_dir.parent().unwrap_or(exe_dir)
    } else {
        exe_dir
    };
    let binary_name = astra_binary_name();
    [
        base_dir.join(binary_name),
        base_dir.join("binaries").join(binary_name),
        exe_dir.join(binary_name),
    ]
    .into_iter()
    .find(|path| path.exists())
}

fn astra_binary_name() -> &'static str {
    if cfg!(windows) {
        "astra.exe"
    } else {
        "astra"
    }
}

fn load_astra_backend_config(store: &dyn SessionStore) -> AstraBackendConfig {
    // Load from astra_config table
    match store.get_astra_config() {
        Ok(config) => {
            let planner_agent = config.planner_agent.as_deref().and_then(Agent::from_db_str);
            let decision_agent = config
                .decision_agent
                .as_deref()
                .and_then(Agent::from_db_str);

            // Astra Pi is the runtime agent backing the bundled Pi provider config.
            let provider_config = match store.list_agents() {
                Ok(agents) => agents
                    .iter()
                    .find(|agent| agent.id == Agent::AstraPi.as_str())
                    .map(|agent| astra_provider_config_from_agent(agent.clone()))
                    .unwrap_or_default(),
                Err(_) => AstraPiProviderConfig::default(),
            };

            AstraBackendConfig {
                planner_agent,
                decision_agent,
                provider_config,
            }
        }
        Err(error) => {
            log::warn!("[astra:config] failed to load Astra config: {error}");
            AstraBackendConfig {
                planner_agent: None,
                decision_agent: None,
                provider_config: AstraPiProviderConfig::default(),
            }
        }
    }
}

fn load_astra_agent_preferences(store: &dyn SessionStore) -> AstraBackendConfig {
    load_astra_backend_config(store)
}

fn astra_provider_config_from_agent(agent: AgentInfo) -> AstraPiProviderConfig {
    let selected = agent
        .ai_provider
        .as_deref()
        .and_then(|id| agent.ai_providers.iter().find(|provider| provider.id == id))
        .or_else(|| agent.ai_providers.iter().find(|provider| provider.enabled))
        .or_else(|| agent.ai_providers.first());
    AstraPiProviderConfig {
        provider: selected.map(|provider| provider.provider.clone()),
        api: selected.and_then(|provider| provider.api.clone()),
        base_url: selected.and_then(|provider| provider.base_url.clone()),
        api_key: selected.and_then(|provider| provider.api_key.clone()),
        model: selected
            .and_then(|provider| provider.model.clone())
            .or(agent.model),
        thinking_level: agent.effort,
    }
}

fn sync_pi_agent_config(config: &AstraPiConfig, preferences: &AstraPiProviderConfig) {
    if let Err(error) = prepare_pi_agent_config(config, preferences) {
        log::warn!(
            "[astra:pi-acp:config] failed to write bundled Pi config code={} message={}",
            error.code,
            error.message
        );
    }
}

impl AstraService {
    pub fn new(app: AppHandle, store: Arc<dyn SessionStore>, runtime: RuntimeManager) -> Self {
        let astra_preferences = load_astra_agent_preferences(store.as_ref());
        let pi_config = bundled_pi_config();
        if let Some(config) = pi_config.as_ref() {
            sync_pi_agent_config(config, &astra_preferences.provider_config);
        }
        Self {
            inner: Arc::new(AstraServiceInner {
                app,
                store,
                runtime,
                pi_config,
                astra_preferences: Mutex::new(astra_preferences),
                delegated_sessions: Mutex::new(HashMap::new()),
                task_waiters: Mutex::new(HashMap::new()),
                orchestrator_workers: Mutex::new(HashMap::new()),
                run_write_lock: Mutex::new(()),
            }),
        }
    }

    pub fn update_astra_preferences_cache(&self, agent: AgentInfo) {
        // Only update provider config from agent, planner/decision come from astra_config table
        let provider_config = astra_provider_config_from_agent(agent.clone());

        match self.inner.astra_preferences.lock() {
            Ok(mut preferences) => {
                preferences.provider_config = provider_config.clone();
            }
            Err(_) => log::warn!("[astra:preferences] cache lock poisoned"),
        }
        if let Some(config) = self.inner.pi_config.as_ref() {
            sync_pi_agent_config(config, &provider_config);
        }
    }

    pub fn reload_config(&self) {
        let config = load_astra_backend_config(self.inner.store.as_ref());
        match self.inner.astra_preferences.lock() {
            Ok(mut preferences) => {
                *preferences = config;
            }
            Err(_) => log::warn!("[astra:preferences] reload config lock poisoned"),
        }
    }

    pub fn watch_runtime_events(&self) -> Result<()> {
        let receiver = self.inner.runtime.subscribe_events()?;
        let service = self.clone();
        thread::spawn(move || {
            for event in receiver {
                if let Err(error) = service.handle_runtime_event(event) {
                    log::warn!("[astra:runtime-event] {error}");
                }
            }
        });
        Ok(())
    }

    pub fn recover_interrupted_runs(&self) -> Result<()> {
        let interrupted = self.inner.store.interrupt_active_astra_runs()?;
        for record in &interrupted {
            let session_ids =
                serde_json::from_str::<Vec<String>>(&record.delegated_session_ids_json)
                    .unwrap_or_default();
            let cleaned = self
                .inner
                .store
                .cleanup_partial_astra_sessions(&session_ids)?;
            if cleaned > 0 {
                log::info!(
                    "[astra:recover:cleanup-partial] runId={} cleanedSessions={}",
                    record.run_id,
                    cleaned
                );
            }
        }
        Ok(())
    }

    pub fn create_astra_run(&self, req: CreateAstraRunRequest) -> Result<AstraHandle> {
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
                let run = record_to_run(active)?;
                if self.is_worker_registered(&run.run_id) {
                    return Ok(self.run_to_handle(run));
                }
                let mut interrupted = run.clone();
                let message =
                    "Astra run was active but no rust-native worker is registered".to_string();
                interrupted.status = AstraRunStatus::Interrupted;
                interrupted.terminal_reason = Some("zombie_active_run".to_string());
                interrupted.last_error_code = Some("worker_missing".to_string());
                interrupted.last_error_message = Some(message.clone());
                interrupted.error = Some(message);
                interrupted.updated_at = now_ms();
                self.inner
                    .store
                    .upsert_astra_run(&run_to_record(&interrupted))?;
                self.emit(
                    &interrupted,
                    "interrupted",
                    json!({
                        "reason": interrupted.terminal_reason,
                        "errorCode": interrupted.last_error_code,
                        "message": interrupted.last_error_message,
                    }),
                );
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
                mode: "rust_native".to_string(),
                current_stage_id: None,
                completed_task_ids: Vec::new(),
                stage_attempt_counts: HashMap::new(),
                retry_limit: ASTRA_DEFAULT_RETRY_LIMIT,
                planner_backend: Some("deterministic".to_string()),
                decision_backend: Some("deterministic".to_string()),
                round_index: None,
                round_limit: RUST_NATIVE_ROUND_LIMIT,
                terminal_reason: None,
                last_error_code: None,
                last_error_message: None,
                internal_planner_session_ids: Vec::new(),
                internal_decision_session_ids: Vec::new(),
                run_diagnostics: Vec::new(),
                error: None,
                created_at: now,
                updated_at: now,
            };
            self.inner.store.upsert_astra_run(&run_to_record(&run))?;
            self.register_pending_worker(&run.run_id)?;
            run
        };
        log::info!(
            "[astra:run:start] runId={} threadId={} projectId={}",
            run.run_id,
            run.thread_id,
            run.project_id
        );
        self.emit(&run, "status", json!({ "status": run.status.as_str() }));

        let service = self.clone();
        let run_id = run.run_id.clone();
        let prompt = req.prompt.clone();
        thread::spawn(move || {
            service.run_rust_native_worker(&run_id, prompt);
        });

        Ok(self.run_to_handle(run))
    }

    pub fn cancel_astra_run(&self, req: CancelAstraRunRequest) -> Result<AstraHandle> {
        let run = self.load_run(&req.run_id)?;
        let (run, changed) = self.mark_run_cancelled(&run.run_id, "user_cancelled")?;
        if changed {
            self.emit(
                &run,
                "cancelled",
                json!({ "status": run.status.as_str(), "reason": run.terminal_reason }),
            );
        }
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
        log::info!(
            "[astra:run:cancel] runId={} threadId={} delegatedSessions={}",
            run.run_id,
            run.thread_id,
            delegated_sessions.len()
        );
        Ok(self.run_to_handle(run))
    }

    pub fn list_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraHandle>> {
        self.inner
            .store
            .list_astra_runs(thread_id)?
            .into_iter()
            .map(record_to_run)
            .map(|result| result.map(|run| self.run_to_handle(run)))
            .collect()
    }

    pub fn get_astra_run(&self, run_id: &str) -> Result<AstraHandle> {
        self.load_run(run_id).map(|run| self.run_to_handle(run))
    }

    fn dispatch_task(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        task: &AstraTaskProposal,
        attempt: DelegatedAttempt<'_>,
        task_waiter: Option<mpsc::Sender<AstraTaskResult>>,
    ) -> Result<AgentSessionHandle> {
        let stage_context = attempt
            .thread_stage_id
            .map(|stage_id| build_stage_task_context(thread, stage_id, task))
            .transpose()?;
        let mut options = RuntimeMetadata::default();
        options.insert("astraRunId".to_string(), Value::String(run.run_id.clone()));
        options.insert("astraTaskId".to_string(), Value::String(task.id.clone()));
        if let Some(stage_id) = attempt.thread_stage_id {
            options.insert(
                "astraThreadStageId".to_string(),
                Value::String(stage_id.to_string()),
            );
        }
        options.insert(
            "astraAttemptCount".to_string(),
            Value::Number(serde_json::Number::from(attempt.attempt_count)),
        );
        options.insert(
            "astraRetryLimitReached".to_string(),
            Value::Bool(attempt.retry_limit_reached),
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
            attempt,
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
            if let Err(error) = self.inner.runtime.send_input(
                &handle.sessio_runtime_session_id,
                AgentInput {
                    text: initial_prompt,
                    attachments: Vec::new(),
                    options: RuntimeMetadata::default(),
                },
            ) {
                self.abort_delegated_session(&handle.sessio_runtime_session_id);
                return Err(error);
            }
        }
        Ok(handle)
    }

    pub(super) fn astra_backend_config(&self) -> AstraBackendConfig {
        match self.inner.astra_preferences.lock() {
            Ok(preferences) => preferences.clone(),
            Err(_) => {
                log::warn!("[astra:preferences] cache lock poisoned");
                AstraBackendConfig {
                    planner_agent: None,
                    decision_agent: None,
                    provider_config: AstraPiProviderConfig::default(),
                }
            }
        }
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
                    decision_action: None,
                    decision_reason: None,
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
            DelegatedAttempt {
                thread_stage_id: stage_id.as_deref(),
                attempt_count,
                retry_limit_reached: false,
            },
            Some(sender),
        )?;
        self.emit(
            &next,
            "task_dispatch",
            json!({
                "taskId": task.id,
                "threadStageId": stage_id,
                "sessioRuntimeSessionId": handle.sessio_runtime_session_id,
                "attemptCount": attempt_count,
            }),
        );
        log::info!(
            "[astra:task:dispatch] runId={} threadId={} taskId={} threadStageId={:?} runtimeSessionId={} attemptCount={}",
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
        attempt: DelegatedAttempt<'_>,
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
                state.thread_stage_id = attempt.thread_stage_id.map(ToString::to_string);
                state.attempt_count = attempt.attempt_count;
                state.retry_limit_reached = attempt.retry_limit_reached;
                if stage_task_context.is_some() {
                    state.stage_task_context = stage_task_context.clone();
                }
            })
            .or_insert_with(|| DelegatedSessionState {
                run_id: run_id.to_string(),
                task_id: task_id.to_string(),
                thread_stage_id: attempt.thread_stage_id.map(ToString::to_string),
                agent_session_id: None,
                stage_task_context,
                session_recorded: false,
                attempt_count: attempt.attempt_count,
                retry_limit_reached: attempt.retry_limit_reached,
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
                        DelegatedAttempt {
                            thread_stage_id,
                            attempt_count,
                            retry_limit_reached,
                        },
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
        let (state_context, already_recorded, already_finished) = {
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
            let finished = state.map(|state| state.finished).unwrap_or(true);
            (context, recorded, finished)
        };
        if already_recorded || already_finished {
            return Ok(());
        }
        let sessio_runtime_session_id_for_run = sessio_runtime_session_id.to_string();
        let agent_session_id_for_run = agent_session_id.to_string();
        let (run, task) = {
            let _guard = self
                .inner
                .run_write_lock
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra run write lock poisoned"))?;
            let mut run = self.load_run(run_id)?;
            if !run.status.active() {
                return Ok(());
            }
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
            run.delegated_session_ids
                .retain(|id| id != &sessio_runtime_session_id_for_run);
            if !run
                .delegated_session_ids
                .iter()
                .any(|id| id == &agent_session_id_for_run)
            {
                run.delegated_session_ids.push(agent_session_id_for_run);
            }
            run.updated_at = now_ms();
            self.inner.store.upsert_astra_run(&run_to_record(&run))?;
            (run, task)
        };
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
            "[astra:task:delegated] runId={} threadId={} taskId={} threadStageId={:?} agentSessionId={} runtimeSessionId={}",
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
            decision_action: None,
            decision_reason: None,
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
            "[astra:task:result] runId={} threadId={} taskId={} sessioRuntimeSessionId={} status={}",
            run.run_id,
            run.thread_id,
            result.task_id,
            result.sessio_runtime_session_id,
            result.status.as_str()
        );
        Ok(())
    }

    fn record_task_result(&self, run_id: &str, result: AstraTaskResult) -> Result<AstraRun> {
        let (run, _) = self.mutate_run(run_id, move |run| {
            upsert_task_result_in_run(run, result);
            Ok(())
        })?;
        Ok(run)
    }

    fn apply_stage_update_decision(
        &self,
        run: &AstraRun,
        args: &Value,
    ) -> Result<AstraStageMutationResult> {
        let stage_id = args
            .get("threadStageId")
            .or_else(|| args.get("stageId"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("threadStageId is required"))?;
        let status = args
            .get("status")
            .and_then(Value::as_str)
            .map(parse_stage_status)
            .transpose()?;
        let summary = optional_string_patch(args, "summary");
        let outcome = optional_string_patch(args, "outcome");

        let result: Result<(AstraRun, AstraStageMutationResult)> = {
            let _guard = self
                .inner
                .run_write_lock
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra run write lock poisoned"))?;
            let mut latest = self.load_run(&run.run_id)?;
            if !latest.status.active() {
                return Ok(inactive_run_mutation_result(&latest));
            }
            let thread = self.inner.store.get_thread_work_state(&latest.thread_id)?;
            let stage_id = resolve_thread_stage_id(&thread, stage_id)?;
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
                    if stage.status == StageStatus::Completed {
                        if let Some(task_id) = task_id.as_deref() {
                            if !latest.completed_task_ids.iter().any(|id| id == task_id) {
                                latest.completed_task_ids.push(task_id.to_string());
                            }
                        }
                    }
                    latest.current_stage_id = Some(stage_id);
                    latest.updated_at = now_ms();
                    self.inner.store.upsert_astra_run(&run_to_record(&latest))?;
                    let result = AstraStageMutationResult {
                        ok: true,
                        stage: Some(serde_json::to_value(stage)?),
                        issue: None,
                        error: None,
                        applied_at: now_ms(),
                    };
                    Ok((latest, result))
                }
                Err(error) => Ok((
                    latest,
                    AstraStageMutationResult {
                        ok: false,
                        stage: None,
                        issue: None,
                        error: Some(error.to_string()),
                        applied_at: now_ms(),
                    },
                )),
            }
        };
        let (next, result) = result?;
        if result.ok {
            self.emit(&next, "stage_update_result", serde_json::to_value(&result)?);
        }
        Ok(result)
    }

    fn apply_issue_decision(
        &self,
        run: &AstraRun,
        args: &Value,
    ) -> Result<AstraStageMutationResult> {
        let result: Result<(AstraRun, AstraStageMutationResult)> = {
            let _guard = self
                .inner
                .run_write_lock
                .lock()
                .map_err(|_| anyhow::anyhow!("Astra run write lock poisoned"))?;
            let latest = self.load_run(&run.run_id)?;
            if !latest.status.active() {
                return Ok(inactive_run_mutation_result(&latest));
            }
            match self.add_or_update_issue(&latest, args) {
                Ok(issue) => {
                    let result = AstraStageMutationResult {
                        ok: true,
                        stage: None,
                        issue: Some(serde_json::to_value(issue)?),
                        error: None,
                        applied_at: now_ms(),
                    };
                    Ok((latest, result))
                }
                Err(error) => Ok((
                    latest,
                    AstraStageMutationResult {
                        ok: false,
                        stage: None,
                        issue: None,
                        error: Some(error.to_string()),
                        applied_at: now_ms(),
                    },
                )),
            }
        };
        let (latest, result) = result?;
        if result.ok {
            self.emit(
                &latest,
                "issue_update_result",
                serde_json::to_value(&result)?,
            );
        }
        Ok(result)
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

    fn update_active_terminal_status(
        &self,
        run_id: &str,
        status: AstraRunStatus,
        terminal_reason: Option<String>,
        last_error_code: Option<String>,
        last_error_message: Option<String>,
        error: Option<String>,
    ) -> Result<(AstraRun, bool)> {
        self.mutate_run(run_id, move |run| {
            Ok(apply_active_status(
                run,
                status,
                error,
                terminal_reason,
                last_error_code,
                last_error_message,
            ))
        })
    }

    pub(super) fn mark_run_completed(
        &self,
        run_id: &str,
        reason: &str,
    ) -> Result<(AstraRun, bool)> {
        self.update_active_terminal_status(
            run_id,
            AstraRunStatus::Completed,
            Some(reason.to_string()),
            None,
            None,
            None,
        )
    }

    pub(super) fn mark_run_cancelled(
        &self,
        run_id: &str,
        reason: &str,
    ) -> Result<(AstraRun, bool)> {
        self.update_active_terminal_status(
            run_id,
            AstraRunStatus::Cancelled,
            Some(reason.to_string()),
            None,
            None,
            None,
        )
    }

    pub(super) fn mark_run_errored(
        &self,
        run_id: &str,
        reason: &str,
        code: &str,
        message: String,
    ) -> Result<(AstraRun, bool)> {
        self.update_active_terminal_status(
            run_id,
            AstraRunStatus::Errored,
            Some(reason.to_string()),
            Some(code.to_string()),
            Some(message.clone()),
            Some(message),
        )
    }

    pub(super) fn mark_run_interrupted(
        &self,
        run_id: &str,
        reason: &str,
        code: &str,
        message: &str,
    ) -> Result<(AstraRun, bool)> {
        self.update_active_terminal_status(
            run_id,
            AstraRunStatus::Interrupted,
            Some(reason.to_string()),
            Some(code.to_string()),
            Some(message.to_string()),
            Some(message.to_string()),
        )
    }

    /// Serialize a read-modify-write cycle on a single Astra run row. The
    /// confirm worker thread and the runtime-event thread both mutate the same
    /// run; without this lock they each load, change
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

    fn run_to_handle(&self, run: AstraRun) -> AstraHandle {
        let current_task_id = self.current_task_id_for_run(&run.run_id);
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
            current_task_id,
            completed_task_ids: run.completed_task_ids,
            stage_attempt_counts: run.stage_attempt_counts,
            retry_limit: run.retry_limit,
            planner_backend: run.planner_backend,
            decision_backend: run.decision_backend,
            round_index: run.round_index,
            round_limit: run.round_limit,
            terminal_reason: run.terminal_reason,
            last_error_code: run.last_error_code,
            last_error_message: run.last_error_message,
            internal_planner_session_ids: run.internal_planner_session_ids,
            internal_decision_session_ids: run.internal_decision_session_ids,
            run_diagnostics: run.run_diagnostics,
            error: run.error,
            created_at: run.created_at,
            updated_at: run.updated_at,
        }
    }

    fn current_task_id_for_run(&self, run_id: &str) -> Option<String> {
        self.inner
            .delegated_sessions
            .lock()
            .ok()?
            .values()
            .find(|state| state.run_id == run_id && !state.finished)
            .map(|state| state.task_id.clone())
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

impl AstraService {
    fn run_rust_native_worker(&self, run_id: &str, prompt: Option<String>) {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.run_rust_native_orchestrator(run_id, prompt)
        }));
        match outcome {
            Ok(Ok(RustNativeWorkerOutcome::Duplicate)) => {}
            Ok(Ok(RustNativeWorkerOutcome::Claimed)) => {
                if let Ok(run) = self.load_run(run_id) {
                    if run.status.active() {
                        let message =
                            "Astra rust-native worker stopped before reaching a terminal state";
                        if let Ok((interrupted, changed)) = self.mark_run_interrupted(
                            run_id,
                            "worker_stopped_without_terminal_state",
                            "worker_stopped",
                            message,
                        ) {
                            if changed {
                                self.emit(
                                    &interrupted,
                                    "interrupted",
                                    json!({
                                        "reason": interrupted.terminal_reason,
                                        "errorCode": interrupted.last_error_code,
                                        "message": interrupted.last_error_message,
                                    }),
                                );
                            }
                        }
                    }
                }
            }
            Ok(Err(error)) => {
                let message = error.to_string();
                log::warn!("[astra:run:rust-native-worker] {message}");
                let changed = self
                    .mark_run_errored(
                        run_id,
                        "worker_top_level_error",
                        "worker_error",
                        message.clone(),
                    )
                    .map(|(_, changed)| changed)
                    .unwrap_or(false);
                if changed {
                    if let Ok(run) = self.load_run(run_id) {
                        self.emit(
                            &run,
                            "error",
                            json!({
                                "message": message,
                                "reason": run.terminal_reason,
                                "errorCode": run.last_error_code,
                            }),
                        );
                    }
                }
            }
            Err(payload) => {
                let message = panic_payload_message(&payload);
                log::warn!("[astra:run:rust-native-worker:panic] {message}");
                let changed = self
                    .mark_run_errored(run_id, "worker_panic", "worker_panic", message.clone())
                    .map(|(_, changed)| changed)
                    .unwrap_or(false);
                if changed {
                    if let Ok(run) = self.load_run(run_id) {
                        self.emit(
                            &run,
                            "error",
                            json!({
                                "message": message,
                                "reason": run.terminal_reason,
                                "errorCode": run.last_error_code,
                            }),
                        );
                    }
                }
            }
        }
    }

    fn is_worker_registered(&self, run_id: &str) -> bool {
        self.inner
            .orchestrator_workers
            .lock()
            .map(|workers| workers.contains_key(run_id))
            .unwrap_or(false)
    }

    fn register_pending_worker(&self, run_id: &str) -> Result<()> {
        let mut workers = self
            .inner
            .orchestrator_workers
            .lock()
            .map_err(|_| anyhow::anyhow!("Astra worker registry lock poisoned"))?;
        workers.insert(run_id.to_string(), AstraWorkerState::Pending);
        Ok(())
    }

    pub(super) fn claim_pending_worker(&self, run_id: &str) -> Result<bool> {
        let mut workers = self
            .inner
            .orchestrator_workers
            .lock()
            .map_err(|_| anyhow::anyhow!("Astra worker registry lock poisoned"))?;
        match workers.get_mut(run_id) {
            Some(state @ AstraWorkerState::Pending) => {
                *state = AstraWorkerState::Running;
                Ok(true)
            }
            Some(AstraWorkerState::Running) => Ok(false),
            None => {
                workers.insert(run_id.to_string(), AstraWorkerState::Running);
                Ok(true)
            }
        }
    }
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

fn inactive_run_mutation_result(run: &AstraRun) -> AstraStageMutationResult {
    AstraStageMutationResult {
        ok: false,
        stage: None,
        issue: None,
        error: Some(format!("Astra run is not active: {}", run.status.as_str())),
        applied_at: now_ms(),
    }
}

fn is_persistable_agent_session_id(agent_runtime_session_id: &str) -> bool {
    !agent_runtime_session_id.trim().is_empty()
        && !agent_runtime_session_id.starts_with("fake-agent-session")
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

pub(crate) fn stage_label(stage: &crate::models::StageInfo) -> String {
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

fn apply_active_status(
    run: &mut AstraRun,
    status: AstraRunStatus,
    error: Option<String>,
    terminal_reason: Option<String>,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
) -> bool {
    if !run.status.active() {
        return false;
    }
    run.status = status;
    run.terminal_reason = terminal_reason;
    run.last_error_code = last_error_code;
    run.last_error_message = last_error_message;
    run.error = error;
    true
}

fn panic_payload_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Astra rust-native worker panicked".to_string())
}

pub(crate) fn summarize_task_output(output: &str) -> String {
    let text = output.trim();
    if text.is_empty() {
        return "Astra delegated task completed.".to_string();
    }
    if text.chars().count() <= 1000 {
        text.to_string()
    } else {
        text.chars().take(997).collect::<String>() + "..."
    }
}

pub(crate) fn pick_stage_agent(stage: &crate::models::StageInfo) -> Option<Agent> {
    stage
        .assistants
        .iter()
        .find_map(|assistant| Agent::from_db_str(&assistant.agent.id))
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
        crate::agents::runtime::types::RuntimeTransportKind::Sidecar => "sidecar",
        crate::agents::runtime::types::RuntimeTransportKind::Fake => "fake",
    }
    .to_string()
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
        planner_backend: run.planner_backend.clone(),
        decision_backend: run.decision_backend.clone(),
        round_index: run.round_index.map(i64::from),
        round_limit: i64::from(run.round_limit),
        terminal_reason: run.terminal_reason.clone(),
        last_error_code: run.last_error_code.clone(),
        last_error_message: run.last_error_message.clone(),
        internal_planner_session_ids_json: serde_json::to_string(&run.internal_planner_session_ids)
            .unwrap_or_else(|_| "[]".to_string()),
        internal_decision_session_ids_json: serde_json::to_string(
            &run.internal_decision_session_ids,
        )
        .unwrap_or_else(|_| "[]".to_string()),
        run_diagnostics_json: serde_json::to_string(&run.run_diagnostics)
            .unwrap_or_else(|_| "[]".to_string()),
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
            .unwrap_or(ASTRA_DEFAULT_RETRY_LIMIT),
        planner_backend: record.planner_backend,
        decision_backend: record.decision_backend,
        round_index: record
            .round_index
            .and_then(|value| u32::try_from(value).ok()),
        round_limit: u32::try_from(record.round_limit)
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or(RUST_NATIVE_ROUND_LIMIT),
        terminal_reason: record.terminal_reason,
        last_error_code: record.last_error_code,
        last_error_message: record.last_error_message,
        internal_planner_session_ids: serde_json::from_str(
            &record.internal_planner_session_ids_json,
        )
        .unwrap_or_default(),
        internal_decision_session_ids: serde_json::from_str(
            &record.internal_decision_session_ids_json,
        )
        .unwrap_or_default(),
        run_diagnostics: serde_json::from_str(&record.run_diagnostics_json).unwrap_or_default(),
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

pub(crate) fn thread_all_stages_terminal(thread: &ThreadInfo) -> bool {
    !thread.stages.is_empty()
        && thread
            .stages
            .iter()
            .all(|stage| matches!(stage.status, StageStatus::Completed | StageStatus::Skipped))
}

pub(crate) fn thread_waiting_for_review(thread: &ThreadInfo) -> bool {
    !thread.stages.is_empty()
        && thread
            .stages
            .iter()
            .any(|stage| stage.status == StageStatus::NeedsReview)
        && thread.stages.iter().all(|stage| {
            matches!(
                stage.status,
                StageStatus::Completed | StageStatus::Skipped | StageStatus::NeedsReview
            )
        })
}

pub(crate) fn short_hash(input: &str) -> String {
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
    use super::decision::{AstraDecisionEngine, DeterministicDecisionEngine};
    use super::planner::{AstraPlanner, DeterministicPlanner};
    use super::*;
    use crate::models::{AssistantAgentInfo, StageAssistantInfo};
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
    fn astra_provider_config_uses_selected_db_provider() {
        let agent = AgentInfo {
            id: Agent::AstraPi.as_str().to_string(),
            name: "Astra Pi".to_string(),
            display_name: "Astra Pi".to_string(),
            icon: None,
            ai_provider: Some("switch".to_string()),
            ai_providers: vec![
                crate::models::AgentAiProviderInfo {
                    id: "fallback".to_string(),
                    display_name: "Fallback".to_string(),
                    provider: "openai".to_string(),
                    api: Some("chat-completions".to_string()),
                    base_url: Some("https://fallback.invalid/v1".to_string()),
                    api_key: Some("fallback-key".to_string()),
                    model: Some("fallback-model".to_string()),
                    models: Vec::new(),
                    enabled: true,
                    order: 0,
                },
                crate::models::AgentAiProviderInfo {
                    id: "switch".to_string(),
                    display_name: "Switch".to_string(),
                    provider: "custom-endpoint".to_string(),
                    api: Some("openai-responses".to_string()),
                    base_url: Some("http://127.0.0.1:15721/v1".to_string()),
                    api_key: Some("ccw".to_string()),
                    model: Some("gpt-5.5".to_string()),
                    models: Vec::new(),
                    enabled: true,
                    order: 1,
                },
            ],
            model: Some("agent-level-model".to_string()),
            models: Vec::new(),
            effort: Some("off".to_string()),
            efforts: Vec::new(),
            permission_mode: None,
            permission_modes: Vec::new(),
            agent_type: crate::models::AgentType::Builtin,
            enabled: true,
            transport: crate::agents::runtime::types::RuntimeTransportKind::Acp,
            commands: crate::models::AgentCommandsInfo::default(),
            order: 0,
            created_at: 1,
            updated_at: 1,
        };

        let config = astra_provider_config_from_agent(agent);

        assert_eq!(config.provider.as_deref(), Some("custom-endpoint"));
        assert_eq!(config.api.as_deref(), Some("openai-responses"));
        assert_eq!(
            config.base_url.as_deref(),
            Some("http://127.0.0.1:15721/v1")
        );
        assert_eq!(config.api_key.as_deref(), Some("ccw"));
        assert_eq!(config.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(config.thinking_level.as_deref(), Some("off"));
    }

    #[test]
    fn pi_stdio_command_sets_session_dir_env_and_strict_durability() {
        let command = pi_stdio_command_json(
            std::path::Path::new("/tmp/astra"),
            std::path::Path::new("/tmp/sessions"),
            std::path::Path::new("/tmp/agent"),
        );
        let value: Value = serde_json::from_str(&command).unwrap();

        assert_eq!(value["command"], "/tmp/astra");
        assert_eq!(
            value["args"],
            json!([
                "--session-dir",
                "/tmp/sessions",
                "--session-durability",
                "strict",
                "--acp"
            ])
        );
        assert_eq!(
            value["env"],
            json!([
                {
                    "name": "PI_CODING_AGENT_DIR",
                    "value": "/tmp/agent",
                },
                {
                    "name": "PI_SESSIONS_DIR",
                    "value": "/tmp/sessions",
                },
            ])
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
            "astra-stage-link-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "astra-stage-project-{}",
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
            .update_project_stage(
                &stage_option.id,
                crate::store::ProjectStagePatch {
                    allow_empty_assistants: Some(true),
                    ..Default::default()
                },
            )
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
            retry_limit: ASTRA_DEFAULT_RETRY_LIMIT,
            planner_backend: Some("deterministic".to_string()),
            decision_backend: Some("deterministic".to_string()),
            round_index: None,
            round_limit: RUST_NATIVE_ROUND_LIMIT,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
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
            "astra-work-snapshot-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "astra-work-snapshot-project-{}",
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
            .update_project_stage(
                &stage_option.id,
                crate::store::ProjectStagePatch {
                    allow_empty_assistants: Some(true),
                    ..Default::default()
                },
            )
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
            "astra-session-promote-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "astra-session-promote-project-{}",
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
            .update_project_stage(
                &stage_option.id,
                crate::store::ProjectStagePatch {
                    allow_empty_assistants: Some(true),
                    ..Default::default()
                },
            )
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
            retry_limit: ASTRA_DEFAULT_RETRY_LIMIT,
            planner_backend: Some("deterministic".to_string()),
            decision_backend: Some("deterministic".to_string()),
            round_index: None,
            round_limit: RUST_NATIVE_ROUND_LIMIT,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
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
            assistant_ids: vec!["assistant-codex".to_string()],
            assistants: vec![
                StageAssistantInfo {
                    assistant_id: "assistant-codex".to_string(),
                    name: "Builder".to_string(),
                    color: None,
                    agent: AssistantAgentInfo {
                        id: "codex".to_string(),
                        name: "Codex".to_string(),
                        model: "gpt-5.3-codex".to_string(),
                        mode: "read-write".to_string(),
                        effort: "medium".to_string(),
                    },
                    system_prompt: Some("Use the stage builder instructions.".to_string()),
                    order: 0,
                },
                StageAssistantInfo {
                    assistant_id: "assistant-claude".to_string(),
                    name: "Reviewer".to_string(),
                    color: None,
                    agent: AssistantAgentInfo {
                        id: "claude".to_string(),
                        name: "Claude".to_string(),
                        model: "claude-sonnet-4-5".to_string(),
                        mode: "read-only".to_string(),
                        effort: "medium".to_string(),
                    },
                    system_prompt: Some("Do not include these review instructions.".to_string()),
                    order: 1,
                },
            ],
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
        assert!(context.prompt.contains("## Stage assistant instructions"));
        assert!(context.prompt.contains("### Builder"));
        assert!(context
            .prompt
            .contains("Use the stage builder instructions."));
        assert!(!context
            .prompt
            .contains("Do not include these review instructions."));
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

    #[test]
    fn needs_review_pauses_run_without_being_terminal() {
        let thread = test_thread(vec![
            test_stage("stage-1", StageStatus::Completed),
            test_stage("stage-2", StageStatus::NeedsReview),
        ]);

        assert!(!thread_all_stages_terminal(&thread));
        assert!(thread_waiting_for_review(&thread));

        let mut active = thread.clone();
        active
            .stages
            .push(test_stage("stage-3", StageStatus::InProgress));
        assert!(!thread_waiting_for_review(&active));
    }

    #[test]
    fn deterministic_plan_targets_incomplete_stage_agent() {
        let mut thread = test_thread(vec![test_stage("stage-1", StageStatus::NotStarted)]);
        thread.stages[0].assistants = vec![StageAssistantInfo {
            assistant_id: "assistant-claude".to_string(),
            name: "Reviewer".to_string(),
            color: None,
            agent: AssistantAgentInfo {
                id: "claude".to_string(),
                name: "Claude".to_string(),
                model: "claude-sonnet-4-5".to_string(),
                mode: "read-only".to_string(),
                effort: "medium".to_string(),
            },
            system_prompt: None,
            order: 0,
        }];
        let run = test_run("run-plan");

        let plan = DeterministicPlanner.plan(&run, &thread, Some("Focus the API"), 0);

        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].target_stage_id.as_deref(), Some("stage-1"));
        assert_eq!(plan.tasks[0].target_agent, Agent::Claude);
        assert!(plan.tasks[0].prompt.contains("Focus the API"));
    }

    #[test]
    fn deterministic_plan_skips_blocked_stage_after_retry_limit() {
        let mut thread = test_thread(vec![test_stage("stage-1", StageStatus::Blocked)]);
        thread.stages[0].assistants = vec![StageAssistantInfo {
            assistant_id: "assistant-codex".to_string(),
            name: "Builder".to_string(),
            color: None,
            agent: AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: "gpt-5.3-codex".to_string(),
                mode: "read-write".to_string(),
                effort: "medium".to_string(),
            },
            system_prompt: None,
            order: 0,
        }];
        let mut run = test_run("run-plan-retry");
        run.stage_attempt_counts
            .insert("stage-1".to_string(), run.retry_limit);

        let plan = DeterministicPlanner.plan(&run, &thread, None, 0);

        assert!(plan.tasks.is_empty());
    }

    #[test]
    fn deterministic_plan_skips_needs_review_stage() {
        let mut thread = test_thread(vec![test_stage("stage-1", StageStatus::NeedsReview)]);
        thread.stages[0].assistants = vec![StageAssistantInfo {
            assistant_id: "assistant-codex".to_string(),
            name: "Builder".to_string(),
            color: None,
            agent: AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: "gpt-5.3-codex".to_string(),
                mode: "read-write".to_string(),
                effort: "medium".to_string(),
            },
            system_prompt: None,
            order: 0,
        }];
        let run = test_run("run-plan-review");

        let plan = DeterministicPlanner.plan(&run, &thread, None, 0);

        assert!(plan.tasks.is_empty());
    }

    #[test]
    fn deterministic_decision_completed_stage_updates_stage() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);
        let task = test_task("task-1", "stage-1");
        let result = test_task_result("task-1", "session-1", "Implemented and verified.");

        let decision = DeterministicDecisionEngine.decide(&thread, &result, &task);

        match decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], Value::String("stage-1".to_string()));
                assert_eq!(args["status"], Value::String("completed".to_string()));
                assert_eq!(
                    args["summary"],
                    Value::String("Implemented and verified.".to_string())
                );
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn deterministic_decision_completed_without_signal_needs_review() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);
        let task = test_task("task-1", "stage-1");
        let result = test_task_result("task-1", "session-1", "I made some progress.");

        let decision = DeterministicDecisionEngine.decide(&thread, &result, &task);

        match decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], Value::String("stage-1".to_string()));
                assert_eq!(args["status"], Value::String("needs_review".to_string()));
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn deterministic_decision_negative_signal_wins_over_completion_substring() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);
        let task = test_task("task-1", "stage-1");
        let result = test_task_result(
            "task-1",
            "session-1",
            "Not completed successfully because more information is needed.",
        );

        let decision = DeterministicDecisionEngine.decide(&thread, &result, &task);

        match decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], Value::String("stage-1".to_string()));
                assert_eq!(args["status"], Value::String("blocked".to_string()));
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn deterministic_retry_limit_blocks_stage_and_creates_issue() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);
        let task = test_task("task-1", "stage-1");
        let mut result = test_task_result("task-1", "session-1", "");
        result.status = AstraTaskResultStatus::Failed;
        result.error = Some("retry limit reached".to_string());
        result.retry_limit_reached = true;
        result.attempt_count = 3;

        let decision = DeterministicDecisionEngine.decide(&thread, &result, &task);

        match decision {
            AstraDecision::Composite { decisions } => {
                assert_eq!(decisions.len(), 2);
                assert!(matches!(
                    decisions[0],
                    AstraDecision::AddOrUpdateIssue { .. }
                ));
                match &decisions[1] {
                    AstraDecision::UpdateStage { args } => {
                        assert_eq!(args["status"], Value::String("blocked".to_string()));
                    }
                    other => panic!("unexpected second decision: {other:?}"),
                }
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn deterministic_decision_failed_stage_creates_issue() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);
        let task = test_task("task-1", "stage-1");
        let mut result = test_task_result("task-1", "session-1", "failed output");
        result.status = AstraTaskResultStatus::Errored;
        result.error = Some("agent crashed".to_string());

        let decision = DeterministicDecisionEngine.decide(&thread, &result, &task);

        match decision {
            AstraDecision::AddOrUpdateIssue { args } => {
                assert_eq!(args["threadStageId"], Value::String("stage-1".to_string()));
                assert_eq!(args["severity"], Value::String("high".to_string()));
                assert_eq!(
                    args["description"],
                    Value::String("agent crashed".to_string())
                );
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn deterministic_decision_cancelled_task_cancels_run() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);
        let task = test_task("task-1", "stage-1");
        let mut result = test_task_result("task-1", "session-1", "");
        result.status = AstraTaskResultStatus::Cancelled;
        result.error = Some("turn cancelled".to_string());

        let decision = DeterministicDecisionEngine.decide(&thread, &result, &task);

        match decision {
            AstraDecision::CancelRun { reason } => {
                assert_eq!(reason, "turn cancelled");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn dispatch_failure_marks_only_active_run_errored() {
        let mut running = test_run("active-run");

        assert!(apply_active_status(
            &mut running,
            AstraRunStatus::Errored,
            Some("runtime start failed".to_string()),
            Some("runtime_failure".to_string()),
            Some("runtime_error".to_string()),
            Some("runtime start failed".to_string()),
        ));
        assert_eq!(running.status, AstraRunStatus::Errored);
        assert_eq!(running.error.as_deref(), Some("runtime start failed"));
        assert_eq!(running.terminal_reason.as_deref(), Some("runtime_failure"));
        assert_eq!(running.last_error_code.as_deref(), Some("runtime_error"));

        let mut cancelled = test_run("cancelled-run");
        cancelled.status = AstraRunStatus::Cancelled;

        assert!(!apply_active_status(
            &mut cancelled,
            AstraRunStatus::Errored,
            Some("waiter disconnected".to_string()),
            Some("waiter_disconnected".to_string()),
            Some("waiter_disconnected".to_string()),
            Some("waiter disconnected".to_string()),
        ));
        assert_eq!(cancelled.status, AstraRunStatus::Cancelled);
        assert_eq!(cancelled.error, None);
    }

    #[test]
    fn terminal_run_status_is_not_overwritten() {
        let mut completed = test_run("completed-run");
        completed.status = AstraRunStatus::Completed;

        assert!(!apply_active_status(
            &mut completed,
            AstraRunStatus::Cancelled,
            None,
            Some("late_cancel".to_string()),
            None,
            None,
        ));
        assert_eq!(completed.status, AstraRunStatus::Completed);

        let mut interrupted = test_run("interrupted-run");
        interrupted.status = AstraRunStatus::Interrupted;

        assert!(!apply_active_status(
            &mut interrupted,
            AstraRunStatus::Errored,
            Some("late error".to_string()),
            Some("late_error".to_string()),
            Some("late_error".to_string()),
            Some("late error".to_string()),
        ));
        assert_eq!(interrupted.status, AstraRunStatus::Interrupted);
        assert_eq!(interrupted.error, None);
    }

    #[test]
    fn deterministic_stage_update_decision_mutates_store() {
        let db_path = std::env::temp_dir().join(format!(
            "astra-decision-store-{}.sqlite",
            short_hash(&now_ms().to_string())
        ));
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let parent = std::env::temp_dir().join(format!(
            "astra-decision-project-{}",
            short_hash(&db_path.to_string_lossy())
        ));
        std::fs::create_dir_all(&parent).unwrap();
        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "Astra Decision Store",
                "general".to_string(),
                None,
            )
            .unwrap();
        let stage_option = store
            .create_project_stage(&project.id, None, "Implement", None, None)
            .unwrap();
        store
            .update_project_stage(
                &stage_option.id,
                crate::store::ProjectStagePatch {
                    allow_empty_assistants: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Ship deterministic decision", None)
            .unwrap();
        let stage = store
            .add_thread_stage(&thread.id, &stage_option.id, &[])
            .unwrap();
        let run = AstraRun {
            run_id: "astra-run-decision-store".to_string(),
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
            retry_limit: ASTRA_DEFAULT_RETRY_LIMIT,
            planner_backend: Some("deterministic".to_string()),
            decision_backend: Some("deterministic".to_string()),
            round_index: None,
            round_limit: RUST_NATIVE_ROUND_LIMIT,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
            error: None,
            created_at: 1,
            updated_at: 1,
        };

        let args = json!({
            "taskId": "task-1",
            "threadStageId": stage.id,
            "status": "completed",
            "summary": "Done",
            "outcome": "Verified",
        });
        let status = args
            .get("status")
            .and_then(Value::as_str)
            .map(parse_stage_status)
            .transpose()
            .unwrap();
        let updated = store
            .update_thread_stage_state(
                args["threadStageId"].as_str().unwrap(),
                status,
                optional_string_patch(&args, "summary"),
                optional_string_patch(&args, "outcome"),
            )
            .unwrap();
        let mut next = run.clone();
        if updated.status == StageStatus::Completed {
            next.completed_task_ids.push("task-1".to_string());
        }

        assert_eq!(updated.status, StageStatus::Completed);
        assert_eq!(updated.summary.as_deref(), Some("Done"));
        assert_eq!(updated.outcome.as_deref(), Some("Verified"));
        assert_eq!(next.completed_task_ids, vec!["task-1"]);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(Path::new(&project.path));
    }

    pub(super) fn test_thread(stages: Vec<crate::models::StageInfo>) -> ThreadInfo {
        ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Ship the thread".to_string(),
            description: None,
            stage_id: None,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            stages,
            sessions: Vec::new(),
        }
    }

    pub(super) fn test_stage(id: &str, status: StageStatus) -> crate::models::StageInfo {
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

    fn test_task(task_id: &str, stage_id: &str) -> AstraTaskProposal {
        AstraTaskProposal {
            id: task_id.to_string(),
            title: "Advance stage".to_string(),
            target_stage_id: Some(stage_id.to_string()),
            target_agent: Agent::Codex,
            prompt: "Do the stage work.".to_string(),
            expected_output: "Stage progress.".to_string(),
            risk: AstraTaskRisk::Low,
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
            retry_limit: ASTRA_DEFAULT_RETRY_LIMIT,
            planner_backend: Some("deterministic".to_string()),
            decision_backend: Some("deterministic".to_string()),
            round_index: None,
            round_limit: RUST_NATIVE_ROUND_LIMIT,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
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
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        }
    }
}
