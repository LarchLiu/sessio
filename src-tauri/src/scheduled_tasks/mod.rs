//! Scheduled ("auto") tasks: persist New Chat templates in SQLite and fire
//! active tasks at configured times.

mod config;
mod schedule;
mod scheduler;

pub use config::{
    ImPushTarget, Schedule, ScheduledTask, ScheduledTaskPushStatus, ScheduledTaskRun,
    ScheduledTaskRunStatus, ScheduledTaskRunTrigger, ScheduledTaskStatus, ScheduledTasksConfig,
    TaskMode, TaskTarget,
};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::agents::runtime::manager::RuntimeCleanupReport;
use crate::agents::runtime::types::RuntimeSessionStatus;
use crate::agents::runtime::types::{AgentSessionHandle, RuntimeTransportKind, StartAgentSession};
use crate::agents::runtime::RuntimeManager;
use crate::astra::{
    AstraHandle, AstraRunStatus, AstraService, CancelAstraRunRequest, CreateAstraRunRequest,
};
use crate::im_bridge::ImBridgeService;
use crate::models::{
    Agent, ProjectInfo, SessionInfo, StageStatus, ThreadAgentInfo, ThreadInfo, ThreadKind,
};
use crate::store::{
    ScheduledTaskRecord, ScheduledTaskRunRecord, SessionStore,
    SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK,
};

use self::config::{now_ms, task_chat_prompt};
use self::schedule::next_run_after;

const COMPLETION_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const PUSH_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const SUMMARY_SOURCE_CHAR_LIMIT: usize = 14_000;
/// A run still `Running` after this long is force-failed by the completion
/// watcher, so a stuck underlying session/Astra run can never lock its task
/// forever. The primary unlock path remains the manual force-unlock command.
const RUN_MAX_DURATION_MS: i64 = 6 * 60 * 60 * 1000;
/// A thread run whose Astra run reports no progress (its `updated_at` stops
/// advancing) for this long is treated as stalled and failed, unlocking the
/// task well before the absolute cap above. Chat runs have no equivalent
/// fine-grained progress signal and rely on `RUN_MAX_DURATION_MS`.
const RUN_STALL_TIMEOUT_MS: i64 = 60 * 60 * 1000;
/// Bounded wait when freeing a finished chat run's runtime session. Short so it
/// never meaningfully delays the watcher/push threads that call it.
const CHAT_SESSION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for the runtime to surface a real agent session id before we stamp it
/// with the scheduled task lineage on the request path. If startup is slower
/// than this, a background waiter keeps watching so Run Now does not look
/// stuck in the UI.
const SCHEDULED_CHAT_STARTUP_INLINE_TIMEOUT: Duration = Duration::from_secs(3);
/// Upper bound for the background waiter that stamps scheduled task lineage
/// after a chat session eventually publishes its real agent session id.
const SCHEDULED_CHAT_STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

fn log_scheduled_chat_cleanup_issue(session_id: &str, report: &RuntimeCleanupReport) {
    if report.cancel_error.is_some()
        || report.dispose_error.is_some()
        || report.timed_out
        || report.force_detached
    {
        log::warn!(
            "[scheduled-tasks] cleanup after failed chat session startup {} reported {:?}",
            session_id,
            report
        );
    }
}

fn is_runtime_startup_timeout(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains("startup timed out"))
}

fn stamp_started_chat_session(
    runtime: &RuntimeManager,
    store: &dyn SessionStore,
    handle: &AgentSessionHandle,
    project: &ProjectInfo,
    task_id: &str,
    task_name: &str,
    prompt: &str,
    run_id: &str,
    timeout: Duration,
) -> Result<()> {
    let real_agent_session_id = if transport_publishes_real_agent_session_id(handle.transport) {
        wait_for_real_agent_session_id(runtime, &handle.sessio_runtime_session_id, timeout)?
    } else {
        handle.agent_runtime_session_id.trim().to_string()
    };
    if real_agent_session_id.is_empty() {
        bail!(
            "chat session {} has empty agent session id",
            handle.sessio_runtime_session_id
        );
    }
    stamp_chat_session(
        store,
        handle,
        project,
        task_id,
        task_name,
        prompt,
        &real_agent_session_id,
    )?;
    // Backfill the run row's agent_session_id so the run survives a restart
    // (its `session_id` carries the runtime's internal handle, dead after
    // shutdown). Best-effort: a logged warning is fine, run already ran.
    if let Err(error) =
        store.update_scheduled_task_run_agent_session_id(run_id, &real_agent_session_id)
    {
        log::warn!(
            "[scheduled-tasks] failed to stamp run {run_id} with agent session id {real_agent_session_id}: {error:#}"
        );
    }
    Ok(())
}

fn transport_publishes_real_agent_session_id(transport: RuntimeTransportKind) -> bool {
    matches!(
        transport,
        RuntimeTransportKind::Acp | RuntimeTransportKind::PiRpc
    )
}

fn wait_for_real_agent_session_id(
    runtime: &RuntimeManager,
    sessio_runtime_session_id: &str,
    timeout: Duration,
) -> Result<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(agent_session_id) = runtime
            .agent_runtime_session_id_for_session(sessio_runtime_session_id)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && !value.starts_with("fake-agent-session"))
        {
            return Ok(agent_session_id);
        }

        match runtime
            .status_for_session(sessio_runtime_session_id)
            .with_context(|| format!("unknown runtime session: {sessio_runtime_session_id}"))?
        {
            RuntimeSessionStatus::Starting
            | RuntimeSessionStatus::Active
            | RuntimeSessionStatus::Idle
            | RuntimeSessionStatus::Cancelling => {}
            RuntimeSessionStatus::Errored
            | RuntimeSessionStatus::Disconnected
            | RuntimeSessionStatus::Ended
            | RuntimeSessionStatus::Completed => {
                runtime.wait_for_session_startup(sessio_runtime_session_id, Duration::ZERO)?;
                bail!(
                    "runtime session {} ended before publishing a real agent session id",
                    sessio_runtime_session_id
                );
            }
        }

        let now = Instant::now();
        if now >= deadline {
            bail!(
                "runtime session startup timed out after {}ms: {}",
                timeout.as_millis(),
                sessio_runtime_session_id
            );
        }
        let remaining = deadline.saturating_duration_since(now);
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

fn stamp_chat_session(
    store: &dyn SessionStore,
    handle: &AgentSessionHandle,
    project: &ProjectInfo,
    task_id: &str,
    task_name: &str,
    prompt: &str,
    real_agent_session_id: &str,
) -> Result<()> {
    // The indexer may not have picked up the new jsonl yet, so the row
    // mark_session_scheduled_task targets might not exist. Write a
    // placeholder SessionInfo first; sticky merge in insert_session preserves
    // origin/scheduled_task_id when indexer later reindexes.
    let now = now_ms();
    let placeholder = SessionInfo {
        id: real_agent_session_id.to_string(),
        agent: handle.agent,
        forked_from_agent: None,
        forked_from_id: None,
        project_path: Some(project.path.clone()),
        project_name: Some(project.name.clone()),
        started_at: Some(now),
        updated_at: Some(now),
        message_count: 0,
        rename_title: None,
        title: Some(task_name.to_string()),
        first_user_message: Some(prompt.to_string()),
        file_path: String::new(),
        file_size: 0,
        partial: true,
        available: true,
        archived: false,
        origin: crate::models::SessionOrigin::Chat,
        scheduled_task_id: Some(task_id.to_string()),
        is_auxiliary: false,
        subagents: Vec::new(),
    };
    if let Err(error) = store.upsert_session("", &placeholder) {
        log::warn!(
            "[scheduled-tasks] failed to upsert placeholder for chat session {} task {}: {error:#}",
            real_agent_session_id,
            task_id
        );
    }
    // Defensive mark: covers the case where the indexer raced ahead and
    // already wrote a row before our placeholder upsert ran.
    if let Err(error) =
        store.mark_session_scheduled_task(handle.agent, real_agent_session_id, task_id, false)
    {
        log::warn!(
            "[scheduled-tasks] failed to mark chat session {} for task {}: {error:#}",
            real_agent_session_id,
            task_id
        );
    }
    Ok(())
}

pub(crate) struct SchedulerState {
    tasks: Mutex<Vec<ScheduledTask>>,
    runtime: RuntimeManager,
    astra: AstraService,
    store: Arc<dyn SessionStore>,
    bridge: Option<ImBridgeService>,
}

impl SchedulerState {
    fn snapshot(&self) -> Vec<ScheduledTask> {
        self.tasks.lock().map(|t| t.clone()).unwrap_or_default()
    }

    fn replace_tasks(&self, mut config: ScheduledTasksConfig) -> Result<Vec<ScheduledTask>> {
        self.normalize_task_timestamps(&mut config);
        config::ensure_ids(&mut config);
        config::validate_config(&config)?;
        self.ensure_running_tasks_are_not_edited(&config.tasks)?;
        self.validate_task_references(&config.tasks)?;
        let records = tasks_to_records(&config.tasks)?;
        self.store.replace_scheduled_tasks(&records)?;
        let tasks = load_tasks_from_store(self.store.as_ref())?;
        if let Ok(mut guard) = self.tasks.lock() {
            *guard = tasks.clone();
        }
        Ok(tasks)
    }

    fn mark_ran(&self, task_id: &str, when_ms: i64) {
        if let Ok(mut guard) = self.tasks.lock() {
            if let Some(task) = guard.iter_mut().find(|t| t.id == task_id) {
                task.last_run_at_ms = Some(when_ms);
            }
        }
        if let Err(error) = self.store.update_scheduled_task_last_run(task_id, when_ms) {
            log::warn!("[scheduled-tasks] failed to persist last-run time: {error:#}");
        }
    }

    fn record_run(
        &self,
        task: &ScheduledTask,
        outcome: &TaskRunOutcome,
        when_ms: i64,
        trigger: ScheduledTaskRunTrigger,
        scheduled_for_ms: Option<i64>,
    ) -> Option<ScheduledTaskRun> {
        let run =
            scheduled_task_run_from_outcome(task, outcome, when_ms, trigger, scheduled_for_ms);
        match scheduled_task_run_to_record(&run).and_then(|record| {
            self.store.insert_scheduled_task_run(&record)?;
            Ok(record)
        }) {
            Ok(_) => {
                if let Ok(mut guard) = self.tasks.lock() {
                    if let Some(task) = guard.iter_mut().find(|candidate| candidate.id == task.id) {
                        task.runs.insert(0, run.clone());
                        task.runs
                            .truncate(SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK);
                    }
                }
                Some(run)
            }
            Err(error) => {
                log::warn!("[scheduled-tasks] failed to record task run: {error:#}");
                None
            }
        }
    }

    fn normalize_task_timestamps(&self, config: &mut ScheduledTasksConfig) {
        let previous = self
            .snapshot()
            .into_iter()
            .map(|task| (task.id.clone(), task))
            .collect::<HashMap<_, _>>();
        let now = now_ms();
        for task in &mut config.tasks {
            if let Some(existing) = previous.get(&task.id) {
                if task.created_at_ms == 0 {
                    task.created_at_ms = existing.created_at_ms;
                }
                if task_config_changed(task, existing) {
                    task.updated_at_ms = now;
                } else if task.updated_at_ms == 0 {
                    task.updated_at_ms = existing.updated_at_ms.max(existing.created_at_ms);
                }
            } else {
                if task.created_at_ms == 0 {
                    task.created_at_ms = now;
                }
                if task.updated_at_ms == 0 {
                    task.updated_at_ms = now;
                }
            }
        }
    }

    fn tick(self: &Arc<Self>, now: i64) {
        for task in self.snapshot() {
            if task.status != ScheduledTaskStatus::Active {
                continue;
            }
            let after = task.last_run_at_ms.unwrap_or(task.created_at_ms);
            let Some(fire_at) = next_run_after(&task.schedule, after) else {
                continue;
            };
            if fire_at > now {
                continue;
            }
            self.run(&task, now, fire_at);
        }
    }

    fn run(self: &Arc<Self>, task: &ScheduledTask, now: i64, scheduled_for_ms: i64) {
        if task_has_running_run(task) {
            log::info!(
                "[scheduled-tasks] skipped task {} ({}) because a previous run is still active",
                task.id,
                task.name
            );
            return;
        }
        match self.execute(task) {
            Ok(outcome) => {
                log::info!("[scheduled-tasks] ran task {} ({})", task.id, task.name);
                let recorded = self.record_run(
                    task,
                    &outcome,
                    now,
                    ScheduledTaskRunTrigger::Scheduled,
                    Some(scheduled_for_ms),
                );
                if let Some(run) = recorded {
                    self.mark_ran(&task.id, now);
                    self.kick_off_chat_stamp(task, &outcome, &run.id);
                } else {
                    self.cleanup_unrecorded_outcome(&outcome);
                }
            }
            Err(error) => {
                log::warn!(
                    "[scheduled-tasks] task {} ({}) failed: {error:#}",
                    task.id,
                    task.name
                );
            }
        }
    }

    fn execute(&self, task: &ScheduledTask) -> Result<TaskRunOutcome> {
        Ok(match &task.target {
            TaskTarget::Chat { .. } => TaskRunOutcome::Chat(self.start_chat_session(task)?),
            _ => TaskRunOutcome::Thread(self.start_thread_run(task)?),
        })
    }

    fn start_chat_session(&self, task: &ScheduledTask) -> Result<AgentSessionHandle> {
        let TaskTarget::Chat {
            project_id,
            agent,
            model,
            effort,
            permission_mode,
            ..
        } = &task.target
        else {
            bail!("chat task target is required");
        };
        let prompt = task_chat_prompt(task).to_string();
        if prompt.is_empty() {
            bail!("task prompt is empty");
        }
        self.ensure_local_agent_available(*agent)?;
        let project = self.project_by_id(project_id)?;
        let mut req = StartAgentSession {
            agent: *agent,
            workspace_path: project.path.clone(),
            initial_prompt: Some(prompt.clone()),
            source_session_id: None,
            source_agent: None,
            options: Default::default(),
        };
        crate::hydrate_start_request_from_db(&mut req, &self.store)?;
        if let Some(model) = model {
            req.options
                .insert("model".to_string(), Value::String(model.clone()));
        }
        if let Some(effort) = effort {
            req.options
                .insert("effort".to_string(), Value::String(effort.clone()));
        }
        if let Some(permission_mode) = permission_mode {
            req.options.insert(
                "permissionMode".to_string(),
                Value::String(permission_mode.clone()),
            );
        }
        // Stamping (waiting for a real agent session id, then writing the
        // sessions placeholder + run.agent_session_id backfill) happens after
        // record_run lands; callers invoke `kick_off_chat_stamp` with the
        // freshly-recorded run id.
        self.runtime.start_session(req)
    }

    /// Drive the post-record stamp: try inline first, fall back to a
    /// background waiter on startup-timeout, and on any other failure cancel
    /// the runtime session + mark the run failed with the error reason.
    fn kick_off_chat_stamp(
        self: &Arc<Self>,
        task: &ScheduledTask,
        outcome: &TaskRunOutcome,
        run_id: &str,
    ) {
        let TaskRunOutcome::Chat(handle) = outcome else {
            return;
        };
        let TaskTarget::Chat { project_id, .. } = &task.target else {
            return;
        };
        let project = match self.project_by_id(project_id) {
            Ok(project) => project,
            Err(error) => {
                self.fail_run_with_stamp_error(run_id, &format!("{error:#}"));
                return;
            }
        };
        let prompt = task_chat_prompt(task).to_string();
        match stamp_started_chat_session(
            &self.runtime,
            self.store.as_ref(),
            handle,
            &project,
            &task.id,
            &task.name,
            &prompt,
            run_id,
            SCHEDULED_CHAT_STARTUP_INLINE_TIMEOUT,
        ) {
            Ok(()) => {}
            Err(error) if is_runtime_startup_timeout(&error) => {
                if let Err(spawn_error) = self.spawn_chat_session_stamp_waiter(
                    handle.clone(),
                    project,
                    task.id.clone(),
                    task.name.clone(),
                    prompt,
                    run_id.to_string(),
                ) {
                    let cleanup = self.runtime.cleanup_session_bounded(
                        &handle.sessio_runtime_session_id,
                        CHAT_SESSION_CLEANUP_TIMEOUT,
                    );
                    log_scheduled_chat_cleanup_issue(&handle.sessio_runtime_session_id, &cleanup);
                    self.fail_run_with_stamp_error(
                        run_id,
                        &format!(
                            "failed to spawn stamp waiter: {spawn_error:#}; cleanup={cleanup:?}"
                        ),
                    );
                }
            }
            Err(error) => {
                let cleanup = self.runtime.cleanup_session_bounded(
                    &handle.sessio_runtime_session_id,
                    CHAT_SESSION_CLEANUP_TIMEOUT,
                );
                log_scheduled_chat_cleanup_issue(&handle.sessio_runtime_session_id, &cleanup);
                let reason = format!(
                    "chat session {} did not finish startup before stamping task {}: {error:#}; cleanup={cleanup:?}",
                    handle.sessio_runtime_session_id, task.id
                );
                log::warn!("[scheduled-tasks] {reason}");
                self.fail_run_with_stamp_error(run_id, &reason);
            }
        }
    }

    fn fail_run_with_stamp_error(&self, run_id: &str, reason: &str) {
        if let Err(error) = self.update_run_status(
            run_id,
            ScheduledTaskRunStatus::Failed,
            now_ms(),
            Some(reason),
        ) {
            log::warn!(
                "[scheduled-tasks] failed to mark run {run_id} failed after stamp error: {error:#}"
            );
        }
    }

    fn spawn_chat_session_stamp_waiter(
        self: &Arc<Self>,
        handle: AgentSessionHandle,
        project: ProjectInfo,
        task_id: String,
        task_name: String,
        prompt: String,
        run_id: String,
    ) -> Result<()> {
        let state = Arc::clone(self);
        let thread_name = format!("scheduled-chat-stamp-{task_id}");
        let session_id = handle.sessio_runtime_session_id.clone();
        let context_task_id = task_id.clone();
        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                if let Err(error) = stamp_started_chat_session(
                    &state.runtime,
                    state.store.as_ref(),
                    &handle,
                    &project,
                    &task_id,
                    &task_name,
                    &prompt,
                    &run_id,
                    SCHEDULED_CHAT_STARTUP_TIMEOUT,
                ) {
                    let cleanup = state.runtime.cleanup_session_bounded(
                        &handle.sessio_runtime_session_id,
                        CHAT_SESSION_CLEANUP_TIMEOUT,
                    );
                    log_scheduled_chat_cleanup_issue(&handle.sessio_runtime_session_id, &cleanup);
                    let reason = format!(
                        "chat session {} did not finish startup before stamping task {task_id}: {error:#}; cleanup={cleanup:?}",
                        handle.sessio_runtime_session_id,
                    );
                    log::warn!("[scheduled-tasks] {reason}");
                    // Route through update_run_status so the in-memory snapshot
                    // (consulted by task_has_running_run on the next tick) is
                    // updated together with the DB row. Bypassing this and
                    // writing the status straight to the store would leave the
                    // task locked as "running" until the next list/UI refresh.
                    state.fail_run_with_stamp_error(&run_id, &reason);
                }
            })
            .with_context(|| {
                format!(
                    "spawn background waiter for scheduled chat session {session_id} task {context_task_id}"
                )
            })?;
        Ok(())
    }

    fn start_thread_run(&self, task: &ScheduledTask) -> Result<ThreadRunOutcome> {
        let (project_id, goal, description, kind, assistant_ids, agent_participants, stage_ids) =
            match &task.target {
                TaskTarget::Process {
                    project_id,
                    goal,
                    description,
                    stage_ids,
                    ..
                } => (
                    project_id,
                    goal,
                    description,
                    ThreadKind::Process,
                    Vec::new(),
                    Vec::new(),
                    stage_ids.clone(),
                ),
                TaskTarget::Teamwork {
                    project_id,
                    goal,
                    description,
                    assistant_ids,
                    ..
                } => (
                    project_id,
                    goal,
                    description,
                    ThreadKind::Teamwork,
                    assistant_ids.clone(),
                    Vec::new(),
                    Vec::new(),
                ),
                TaskTarget::Brainstorm {
                    project_id,
                    goal,
                    description,
                    agent_participants,
                    ..
                } => (
                    project_id,
                    goal,
                    description,
                    ThreadKind::Brainstorm,
                    Vec::new(),
                    agent_participants.clone(),
                    Vec::new(),
                ),
                TaskTarget::Debate {
                    project_id,
                    goal,
                    description,
                    agent_participants,
                    ..
                } => (
                    project_id,
                    goal,
                    description,
                    ThreadKind::Debate,
                    Vec::new(),
                    agent_participants.clone(),
                    Vec::new(),
                ),
                TaskTarget::Chat { .. } => bail!("thread task target is required"),
            };
        let project = self.project_by_id(project_id)?;
        let goal = goal.trim();
        if goal.is_empty() {
            bail!("thread goal is empty");
        }
        validate_thread_defaults(kind, &stage_ids, &assistant_ids, &agent_participants)?;
        let description = description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let thread = self.store.create_thread_with_origin(
            &project.id,
            goal,
            description,
            kind,
            &assistant_ids,
            &agent_participants,
            crate::models::ThreadOrigin::ScheduledTask,
            Some(task.id.as_str()),
        )?;
        let start_result = (|| -> Result<_> {
            if kind == ThreadKind::Process {
                for stage_id in &stage_ids {
                    self.store.add_thread_stage(&thread.id, stage_id, &[])?;
                }
            }
            self.astra.create_astra_run(CreateAstraRunRequest {
                thread_id: thread.id.clone(),
                prompt: None,
            })
        })();
        let run = match start_result {
            Ok(run) => run,
            Err(error) => {
                if let Err(cleanup_error) = self.store.delete_thread(&thread.id) {
                    log::warn!(
                        "[scheduled-tasks] failed to clean up thread {} after failed task start: {cleanup_error:#}",
                        thread.id
                    );
                }
                return Err(error);
            }
        };
        Ok(ThreadRunOutcome { thread, run })
    }

    fn validate_task_references(&self, tasks: &[ScheduledTask]) -> Result<()> {
        let projects = self.store.list_projects()?;
        let project_ids = projects
            .iter()
            .map(|project| project.id.as_str())
            .collect::<HashSet<_>>();
        for task in tasks {
            if !project_ids.contains(task.target.project_id()) {
                bail!("task project not found: {}", task.target.project_id());
            }
        }
        self.validate_local_target_agents(tasks)?;
        self.validate_thread_task_templates(tasks)
    }

    fn ensure_running_tasks_are_not_edited(&self, next_tasks: &[ScheduledTask]) -> Result<()> {
        let previous = self
            .snapshot()
            .into_iter()
            .filter(|task| task_has_running_run(task))
            .map(|task| (task.id.clone(), task))
            .collect::<HashMap<_, _>>();
        if previous.is_empty() {
            return Ok(());
        }
        let next_by_id = next_tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect::<HashMap<_, _>>();
        for (task_id, previous_task) in previous {
            let Some(next_task) = next_by_id.get(task_id.as_str()) else {
                bail!(
                    "scheduled task {} is still running and cannot be deleted",
                    previous_task.name
                );
            };
            if previous_task.name != next_task.name
                || previous_task.schedule != next_task.schedule
                || previous_task.target != next_task.target
            {
                bail!(
                    "scheduled task {} is still running and cannot be edited",
                    previous_task.name
                );
            }
        }
        Ok(())
    }

    fn validate_local_target_agents(&self, tasks: &[ScheduledTask]) -> Result<()> {
        let enabled_agents = self
            .store
            .list_agents()?
            .into_iter()
            .filter(|agent| agent.enabled)
            .map(|agent| agent.id)
            .collect::<HashSet<_>>();
        for task in tasks {
            if let TaskTarget::Chat { agent, .. } = &task.target {
                if !enabled_agents.contains(agent.as_str()) {
                    bail!(
                        "local target agent {} is not enabled or configured",
                        agent.as_str()
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_thread_task_templates(&self, tasks: &[ScheduledTask]) -> Result<()> {
        let enabled_agents = self
            .store
            .list_agents()?
            .into_iter()
            .filter(|agent| agent.enabled && Agent::from_db_str(&agent.id).is_some())
            .map(|agent| agent.id)
            .collect::<HashSet<_>>();
        for task in tasks {
            match &task.target {
                TaskTarget::Chat { .. } => {}
                TaskTarget::Process {
                    project_id,
                    stage_ids,
                    ..
                } => {
                    let valid_stage_ids = self
                        .store
                        .list_project_stages(project_id)?
                        .into_iter()
                        .filter(|stage| {
                            stage.enabled
                                && (stage.allow_empty_assistants || !stage.assistants.is_empty())
                        })
                        .map(|stage| stage.id)
                        .collect::<HashSet<_>>();
                    ensure_unique(stage_ids, "process stage")?;
                    for stage_id in stage_ids {
                        if !valid_stage_ids.contains(stage_id) {
                            bail!("process stage {stage_id} is not enabled or not in task project");
                        }
                    }
                }
                TaskTarget::Teamwork {
                    project_id,
                    assistant_ids,
                    ..
                } => {
                    let valid_assistant_ids = self
                        .store
                        .list_assistants(Some(project_id))?
                        .into_iter()
                        .filter(|assistant| assistant.enabled)
                        .map(|assistant| assistant.id)
                        .collect::<HashSet<_>>();
                    ensure_unique(assistant_ids, "teamwork assistant")?;
                    for assistant_id in assistant_ids {
                        if !valid_assistant_ids.contains(assistant_id) {
                            bail!(
                                "teamwork assistant {assistant_id} is not enabled or not in task project"
                            );
                        }
                    }
                }
                TaskTarget::Brainstorm {
                    agent_participants, ..
                }
                | TaskTarget::Debate {
                    agent_participants, ..
                } => {
                    // De-dupe by (agent, model): participant_id is empty for
                    // freshly-built templates, so keying on it would silently
                    // disable the uniqueness check.
                    let participant_keys = agent_participants
                        .iter()
                        .map(|participant| {
                            format!(
                                "{}:{}",
                                participant.agent.as_str(),
                                participant.model.trim()
                            )
                        })
                        .collect::<Vec<_>>();
                    ensure_unique(&participant_keys, "thread participant")?;
                    for participant in agent_participants {
                        if !enabled_agents.contains(participant.agent.as_str()) {
                            bail!(
                                "thread participant agent {} is not enabled or configured",
                                participant.agent.as_str()
                            );
                        }
                        if participant.model.trim().is_empty() {
                            bail!("thread participant model is empty");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn ensure_local_agent_available(&self, agent: Agent) -> Result<()> {
        let available = self
            .store
            .list_agents()?
            .into_iter()
            .any(|candidate| candidate.id == agent.as_str() && candidate.enabled);
        if !available {
            bail!(
                "local target agent {} is not enabled or configured",
                agent.as_str()
            );
        }
        Ok(())
    }

    fn project_by_id(&self, project_id: &str) -> Result<ProjectInfo> {
        self.store
            .list_projects()?
            .into_iter()
            .find(|project| project.id == project_id)
            .ok_or_else(|| anyhow::anyhow!("project not found for {project_id}"))
    }

    fn load_runs_requiring_update(&self) -> Option<Vec<ScheduledTaskRun>> {
        match self
            .store
            .list_scheduled_task_runs_requiring_update()
            .and_then(|records| {
                records
                    .into_iter()
                    .map(scheduled_task_run_from_record)
                    .collect::<Result<Vec<_>>>()
            }) {
            Ok(runs) => Some(runs),
            Err(error) => {
                log::warn!("[scheduled-tasks] failed to load task runs: {error:#}");
                None
            }
        }
    }

    /// Detect terminal status for still-running runs and apply the TTL backstop.
    /// Deliberately free of the (potentially minutes-long) channel push so the
    /// watcher never stalls other runs' completion detection.
    fn process_run_completions(&self) {
        let Some(runs) = self.load_runs_requiring_update() else {
            return;
        };
        for run in runs {
            if run.status != ScheduledTaskRunStatus::Running {
                continue;
            }
            let now = now_ms();
            // TTL backstop: fail a run that is either stalled (no progress) or
            // simply too old, and stop its underlying work, so a hung session /
            // Astra run can't lock the task or keep consuming resources.
            if let Some(reason) = self.run_expiry_reason(&run, now) {
                self.cancel_run_underlying(&run);
                if let Err(error) = self.update_run_status(
                    &run.id,
                    ScheduledTaskRunStatus::Failed,
                    now,
                    Some(reason),
                ) {
                    log::warn!(
                        "[scheduled-tasks] failed to fail run {} after TTL: {error:#}",
                        run.id
                    );
                } else {
                    log::warn!(
                        "[scheduled-tasks] run {} backstop-failed ({reason}); cancelled underlying work",
                        run.id
                    );
                }
                self.cleanup_chat_session_if_unpushed(&run);
                continue;
            }
            match self.detect_run_terminal_status(&run) {
                Ok(Some(status)) => {
                    if let Err(error) = self.update_run_status(&run.id, status, now, None) {
                        log::warn!(
                            "[scheduled-tasks] failed to mark run {} terminal: {error:#}",
                            run.id
                        );
                    }
                    self.cleanup_chat_session_if_unpushed(&run);
                }
                Ok(None) => {}
                Err(error) => {
                    let message = error.to_string();
                    if let Err(update_error) = self.update_run_status(
                        &run.id,
                        ScheduledTaskRunStatus::Failed,
                        now,
                        Some(&message),
                    ) {
                        log::warn!(
                            "[scheduled-tasks] failed to mark run {} failed after detect error: {update_error:#}",
                            run.id
                        );
                    }
                    log::warn!(
                        "[scheduled-tasks] run {} completion detection failed: {error:#}",
                        run.id
                    );
                    self.cleanup_chat_session_if_unpushed(&run);
                }
            }
        }
    }

    /// Why a still-running run should be backstop-failed, or `None` if it may
    /// keep running. Combines an absolute cap (all modes) with a stall cap for
    /// thread runs, which expose Astra `updated_at` as a progress signal; chat
    /// runs have no comparable signal and rely on the absolute cap alone.
    fn run_expiry_reason(&self, run: &ScheduledTaskRun, now: i64) -> Option<&'static str> {
        if now.saturating_sub(run.started_at_ms) > RUN_MAX_DURATION_MS {
            return Some("exceeded max duration");
        }
        if run.mode != TaskMode::Chat {
            if let Some(updated_at) = self.astra_run_last_update(run) {
                if now.saturating_sub(updated_at) > RUN_STALL_TIMEOUT_MS {
                    return Some("no progress within stall timeout");
                }
            }
        }
        None
    }

    fn astra_run_last_update(&self, run: &ScheduledTaskRun) -> Option<i64> {
        let astra_run_id = run.astra_run_id.as_deref()?;
        self.store
            .get_astra_run(astra_run_id)
            .ok()
            .flatten()
            .map(|record| record.updated_at)
    }

    /// Push channel summaries for finished runs. Runs on its own worker thread
    /// because `push_run_summary` makes a blocking summarization call that must
    /// not delay completion detection.
    fn process_run_pushes(&self) {
        let Some(runs) = self.load_runs_requiring_update() else {
            return;
        };
        for run in runs {
            if run.status == ScheduledTaskRunStatus::Running {
                continue;
            }
            if !matches!(
                run.push_status,
                Some(ScheduledTaskPushStatus::Pending | ScheduledTaskPushStatus::Summarizing)
            ) {
                continue;
            }
            if let Err(error) = self.push_run_summary(&run) {
                log::warn!(
                    "[scheduled-tasks] run {} channel push failed: {error:#}",
                    run.id
                );
            }
            // The push consumed the transcript; free the chat session now.
            self.cleanup_chat_session(&run);
        }
    }

    fn detect_run_terminal_status(
        &self,
        run: &ScheduledTaskRun,
    ) -> Result<Option<ScheduledTaskRunStatus>> {
        match run.mode {
            TaskMode::Chat => self.detect_chat_run_terminal_status(run),
            TaskMode::Process | TaskMode::Teamwork | TaskMode::Brainstorm | TaskMode::Debate => {
                self.detect_thread_run_terminal_status(run)
            }
        }
    }

    fn detect_chat_run_terminal_status(
        &self,
        run: &ScheduledTaskRun,
    ) -> Result<Option<ScheduledTaskRunStatus>> {
        let Some(session_id) = run.session_id.as_deref() else {
            bail!("chat run has no session id");
        };
        if self.runtime.active_turn_id(session_id).is_some() {
            return Ok(None);
        }
        let Some(runtime_status) = self.runtime.status_for_session(session_id) else {
            return Ok(Some(ScheduledTaskRunStatus::Failed));
        };
        match self.runtime.latest_turn_status(session_id).as_deref() {
            Some("completed") => Ok(Some(ScheduledTaskRunStatus::Completed)),
            Some("cancelled") => Ok(Some(ScheduledTaskRunStatus::Cancelled)),
            Some("failed") => Ok(Some(ScheduledTaskRunStatus::Failed)),
            Some(_) => Ok(None),
            None => match runtime_status {
                RuntimeSessionStatus::Starting
                | RuntimeSessionStatus::Active
                | RuntimeSessionStatus::Idle => Ok(None),
                RuntimeSessionStatus::Completed | RuntimeSessionStatus::Ended => {
                    Ok(Some(ScheduledTaskRunStatus::Completed))
                }
                RuntimeSessionStatus::Cancelling => Ok(None),
                RuntimeSessionStatus::Errored | RuntimeSessionStatus::Disconnected => {
                    Ok(Some(ScheduledTaskRunStatus::Failed))
                }
            },
        }
    }

    fn detect_thread_run_terminal_status(
        &self,
        run: &ScheduledTaskRun,
    ) -> Result<Option<ScheduledTaskRunStatus>> {
        let Some(astra_run_id) = run.astra_run_id.as_deref() else {
            bail!("thread run has no Astra run id");
        };
        let Some(record) = self.store.get_astra_run(astra_run_id)? else {
            bail!("Astra run not found: {astra_run_id}");
        };
        let status = AstraRunStatus::from_db_str(&record.status)
            .ok_or_else(|| anyhow::anyhow!("invalid Astra run status {}", record.status))?;
        if status.active() {
            return Ok(None);
        }
        Ok(Some(match status {
            AstraRunStatus::Completed => ScheduledTaskRunStatus::Completed,
            AstraRunStatus::Cancelled => ScheduledTaskRunStatus::Cancelled,
            AstraRunStatus::Errored | AstraRunStatus::Interrupted => ScheduledTaskRunStatus::Failed,
            AstraRunStatus::Planning
            | AstraRunStatus::Thinking
            | AstraRunStatus::AwaitingApproval
            | AstraRunStatus::Dispatching
            | AstraRunStatus::Running => unreachable!("active Astra status handled above"),
        }))
    }

    fn update_run_status(
        &self,
        run_id: &str,
        status: ScheduledTaskRunStatus,
        completed_at_ms: i64,
        error: Option<&str>,
    ) -> Result<()> {
        self.store.update_scheduled_task_run_status(
            run_id,
            status.as_str(),
            Some(completed_at_ms),
            error,
        )?;
        self.update_run_in_memory(run_id, |run| {
            run.status = status;
            run.completed_at_ms = Some(completed_at_ms);
            if let Some(error) = error {
                run.error = Some(error.to_string());
            }
        });
        Ok(())
    }

    fn update_run_push_status(
        &self,
        run_id: &str,
        push_status: ScheduledTaskPushStatus,
        push_summary: Option<&str>,
        push_error: Option<&str>,
        push_sent_at_ms: Option<i64>,
    ) -> Result<()> {
        self.store.update_scheduled_task_run_push(
            run_id,
            push_status.as_str(),
            push_summary,
            push_error,
            push_sent_at_ms,
        )?;
        self.update_run_in_memory(run_id, |run| {
            run.push_status = Some(push_status);
            if let Some(summary) = push_summary {
                run.push_summary = Some(summary.to_string());
            }
            run.push_error = push_error.map(ToOwned::to_owned);
            if let Some(sent_at) = push_sent_at_ms {
                run.push_sent_at_ms = Some(sent_at);
            }
        });
        Ok(())
    }

    fn update_run_in_memory(&self, run_id: &str, mut update: impl FnMut(&mut ScheduledTaskRun)) {
        if let Ok(mut guard) = self.tasks.lock() {
            for task in guard.iter_mut() {
                if let Some(run) = task.runs.iter_mut().find(|run| run.id == run_id) {
                    update(run);
                    return;
                }
            }
        }
    }

    fn cleanup_unrecorded_outcome(&self, outcome: &TaskRunOutcome) {
        match outcome {
            TaskRunOutcome::Chat(handle) => {
                let report = self.runtime.cleanup_session_bounded(
                    &handle.sessio_runtime_session_id,
                    CHAT_SESSION_CLEANUP_TIMEOUT,
                );
                log_scheduled_chat_cleanup_issue(&handle.sessio_runtime_session_id, &report);
                log::warn!(
                    "[scheduled-tasks] cleaned up unrecorded chat session {} after run record failure: {:?}",
                    handle.sessio_runtime_session_id,
                    report
                );
            }
            TaskRunOutcome::Thread(outcome) => {
                if let Err(error) = self.astra.cancel_astra_run(CancelAstraRunRequest {
                    run_id: outcome.run.run_id.clone(),
                }) {
                    log::warn!(
                        "[scheduled-tasks] failed to cancel unrecorded Astra run {} after run record failure: {error:#}",
                        outcome.run.run_id
                    );
                }
                if let Err(error) = self.store.delete_thread(&outcome.thread.id) {
                    log::warn!(
                        "[scheduled-tasks] failed to delete unrecorded thread {} after run record failure: {error:#}",
                        outcome.thread.id
                    );
                }
            }
        }
    }

    fn push_run_summary(&self, run: &ScheduledTaskRun) -> Result<()> {
        let (Some(platform), Some(chat_id)) =
            (run.push_platform.as_deref(), run.push_chat_id.as_deref())
        else {
            return Ok(());
        };
        let platform = platform.trim();
        let chat_id = chat_id.trim();
        if platform.is_empty() || chat_id.is_empty() {
            self.update_run_push_status(
                &run.id,
                ScheduledTaskPushStatus::Failed,
                None,
                Some("channel push target is incomplete"),
                None,
            )?;
            return Ok(());
        }

        self.update_run_push_status(
            &run.id,
            ScheduledTaskPushStatus::Summarizing,
            None,
            None,
            None,
        )?;

        if self.bridge.is_none() {
            let error = "IM bridge is not running; cannot push task notification";
            self.update_run_push_status(
                &run.id,
                ScheduledTaskPushStatus::Failed,
                None,
                Some(error),
                None,
            )?;
            bail!(error);
        }

        let result = (|| -> Result<Option<String>> {
            let source = self.build_run_summary_source(run)?;
            let workspace_path = self.summary_workspace_path(run)?;
            let outcome = self
                .astra
                .summarize_auto_task_notification(&workspace_path, &source)?;
            // The summary helper runtime session is task-internal and must
            // not appear in the sidebar. We can't rely on the indexer having
            // written a sessions row yet — mark_session_scheduled_task is a
            // pure UPDATE — so write a placeholder first, then re-mark
            // defensively in case the indexer raced ahead. Sticky merge in
            // insert_session preserves origin / scheduled_task_id /
            // is_auxiliary across any later reindex.
            let now = now_ms();
            let placeholder = SessionInfo {
                id: outcome.agent_session_id.clone(),
                agent: outcome.agent,
                forked_from_agent: None,
                forked_from_id: None,
                project_path: Some(workspace_path.clone()),
                project_name: None,
                started_at: Some(now),
                updated_at: Some(now),
                message_count: 0,
                rename_title: Some(format!("Auto task summary: {}", run.task_id)),
                title: None,
                first_user_message: None,
                file_path: String::new(),
                file_size: 0,
                partial: true,
                available: true,
                archived: false,
                origin: crate::models::SessionOrigin::Chat,
                scheduled_task_id: Some(run.task_id.clone()),
                is_auxiliary: true,
                subagents: Vec::new(),
            };
            if let Err(error) = self.store.upsert_session("", &placeholder) {
                log::warn!(
                    "[scheduled-tasks] failed to upsert placeholder for summary session {} task {}: {error:#}",
                    outcome.agent_session_id,
                    run.task_id
                );
            }
            if let Err(error) = self.store.mark_session_scheduled_task(
                outcome.agent,
                &outcome.agent_session_id,
                &run.task_id,
                true,
            ) {
                log::warn!(
                    "[scheduled-tasks] failed to mark summary session {} for task {}: {error:#}",
                    outcome.agent_session_id,
                    run.task_id
                );
            }
            let summary = outcome.summary;
            // Cancellation checkpoint: a concurrent force-unlock flips this run's
            // push out of `summarizing`. Stop before sending so "force-unlock
            // means no more pushes" holds. This shrinks the race window from the
            // whole (minutes-long) summarization to just here→the send call.
            if self.push_superseded(&run.id) {
                return Ok(None);
            }
            let bridge = self.bridge.as_ref().expect("bridge checked before summary");
            let title = run
                .task_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Auto task");
            let text = format!("Auto task finished: {title}\n\n{}", summary.trim());
            bridge.send_text_to_chat(platform, chat_id, &text)?;
            let attachment_refs = format!("{summary}\n\n{source}");
            let attachment_count = bridge.send_referenced_attachments_to_chat(
                platform,
                chat_id,
                &attachment_refs,
                &workspace_path,
            )?;
            if attachment_count > 0 {
                log::info!(
                    "[scheduled-tasks] pushed {attachment_count} attachment(s) for run {}",
                    run.id
                );
            }
            Ok(Some(summary))
        })();

        match result {
            Ok(Some(summary)) => {
                self.update_run_push_status(
                    &run.id,
                    ScheduledTaskPushStatus::Sent,
                    Some(summary.trim()),
                    None,
                    Some(now_ms()),
                )?;
            }
            Ok(None) => {
                log::info!(
                    "[scheduled-tasks] skipped channel push for run {} because it was force-unlocked during summarization",
                    run.id
                );
            }
            Err(error) => {
                self.update_run_push_status(
                    &run.id,
                    ScheduledTaskPushStatus::Failed,
                    None,
                    Some(&error.to_string()),
                    None,
                )?;
                return Err(error);
            }
        }
        Ok(())
    }

    /// True once a concurrent action (force-unlock) moved this run's push out of
    /// the `summarizing` state set at the start of `push_run_summary`.
    fn push_superseded(&self, run_id: &str) -> bool {
        self.tasks.lock().ok().map_or(false, |guard| {
            guard.iter().any(|task| {
                task.runs.iter().any(|run| {
                    run.id == run_id
                        && run.push_status != Some(ScheduledTaskPushStatus::Summarizing)
                })
            })
        })
    }

    fn summary_workspace_path(&self, run: &ScheduledTaskRun) -> Result<String> {
        if let Some(target) = &run.task_target {
            return self
                .project_by_id(target.project_id())
                .map(|project| project.path);
        }
        if let Some(thread_id) = run.thread_id.as_deref() {
            let thread = self.store.get_thread_work_state(thread_id)?;
            return self
                .project_by_id(&thread.project_id)
                .map(|project| project.path);
        }
        bail!("run has no task target or thread project for summary")
    }

    fn build_run_summary_source(&self, run: &ScheduledTaskRun) -> Result<String> {
        let source = match run.mode {
            TaskMode::Chat => self.build_chat_run_summary_source(run)?,
            TaskMode::Process | TaskMode::Teamwork | TaskMode::Brainstorm | TaskMode::Debate => {
                self.build_thread_run_summary_source(run)?
            }
        };
        Ok(truncate_chars(&source, SUMMARY_SOURCE_CHAR_LIMIT))
    }

    fn build_chat_run_summary_source(&self, run: &ScheduledTaskRun) -> Result<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "Task: {}",
            run.task_name.as_deref().unwrap_or("Auto task")
        ));
        lines.push("Mode: chat".to_string());
        lines.push(format!("Run status: {}", run.status.as_str()));
        if let Some(TaskTarget::Chat { prompt, agent, .. }) = &run.task_target {
            lines.push(format!("Target agent: {}", agent.as_str()));
            lines.push(format!("Prompt:\n{}", prompt.trim()));
        }
        if let Some(session_id) = run.session_id.as_deref() {
            lines.push(format!("Session: {session_id}"));
            let transcript = self
                .runtime
                .session_transcript_text(session_id)
                .unwrap_or_default();
            if transcript.trim().is_empty() {
                lines.push("Transcript: unavailable from live runtime state.".to_string());
            } else {
                lines.push(format!("Transcript:\n{transcript}"));
            }
        }
        Ok(lines.join("\n\n"))
    }

    fn build_thread_run_summary_source(&self, run: &ScheduledTaskRun) -> Result<String> {
        let Some(thread_id) = run.thread_id.as_deref() else {
            bail!("thread run has no thread id");
        };
        let thread = self.store.get_thread_work_state(thread_id)?;
        let rounds = self.store.list_plan_rounds(thread_id)?;
        let mut lines = Vec::new();
        lines.push(format!(
            "Task: {}",
            run.task_name.as_deref().unwrap_or("Auto task")
        ));
        lines.push(format!("Mode: {}", run.mode.as_str()));
        lines.push(format!("Run status: {}", run.status.as_str()));
        lines.push(format!("Thread goal: {}", thread.goal));
        if let Some(description) = thread.description.as_deref() {
            if !description.trim().is_empty() {
                lines.push(format!("Thread description:\n{}", description.trim()));
            }
        }
        if let Some(astra_run_id) = run.astra_run_id.as_deref() {
            if let Some(record) = self.store.get_astra_run(astra_run_id)? {
                lines.push(format!("Astra status: {}", record.status));
                if let Some(error) = record.last_error_message.or(record.error) {
                    lines.push(format!("Astra error: {error}"));
                }
            }
        }
        if !thread.stages.is_empty() {
            lines.push("Stages:".to_string());
            for stage in &thread.stages {
                let label = stage
                    .name
                    .as_deref()
                    .or_else(|| stage.kind.map(|kind| kind.as_str()))
                    .unwrap_or(stage.stage_id.as_str());
                lines.push(format!(
                    "- {} [{}]{}{}",
                    label,
                    stage_status_str(stage.status),
                    stage
                        .summary
                        .as_deref()
                        .map(|summary| format!(" summary: {}", summary.trim()))
                        .unwrap_or_default(),
                    stage
                        .outcome
                        .as_deref()
                        .map(|outcome| format!(" outcome: {}", outcome.trim()))
                        .unwrap_or_default()
                ));
            }
        }
        for round in rounds {
            lines.push(format!(
                "Round {} [{}]{}",
                round.round_index,
                round.status.as_str(),
                round
                    .summary
                    .as_deref()
                    .map(|summary| format!(" summary: {}", summary.trim()))
                    .unwrap_or_default()
            ));
            for task in round.tasks {
                lines.push(format!(
                    "- Task {} [{}]{}{}",
                    task.title,
                    task.status.as_str(),
                    task.result_summary
                        .as_deref()
                        .map(|summary| format!(" result: {}", summary.trim()))
                        .unwrap_or_default(),
                    task.error
                        .as_deref()
                        .map(|error| format!(" error: {}", error.trim()))
                        .unwrap_or_default()
                ));
            }
        }
        Ok(lines.join("\n"))
    }

    /// Cancel all still-running runs of a task and unlock it. Best-effort stops
    /// the underlying work (chat turn / Astra run) before marking each run
    /// `Cancelled`. Returns how many runs were unlocked.
    fn force_unlock(&self, task_id: &str) -> Result<usize> {
        // Take any run that either locks the task (Running) or has a push still
        // queued/in-flight (Pending/Summarizing) — the latter covers a run that
        // already finished but whose channel push is mid-summarization.
        let targets: Vec<ScheduledTaskRun> = {
            let guard = self
                .tasks
                .lock()
                .map_err(|_| anyhow::anyhow!("scheduled task lock poisoned"))?;
            guard
                .iter()
                .find(|task| task.id == task_id)
                .map(|task| {
                    task.runs
                        .iter()
                        .filter(|run| {
                            run.status == ScheduledTaskRunStatus::Running
                                || matches!(
                                    run.push_status,
                                    Some(
                                        ScheduledTaskPushStatus::Pending
                                            | ScheduledTaskPushStatus::Summarizing
                                    )
                                )
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        for run in &targets {
            if run.status == ScheduledTaskRunStatus::Running {
                self.cancel_run_underlying(run);
                let now = now_ms();
                if let Err(error) =
                    self.update_run_status(&run.id, ScheduledTaskRunStatus::Cancelled, now, None)
                {
                    log::warn!(
                        "[scheduled-tasks] failed to cancel run {} during force-unlock: {error:#}",
                        run.id
                    );
                }
            }
            // Cancel any queued or in-flight push. Combined with the checkpoint
            // in push_run_summary, this stops a mid-summarization push from
            // still delivering to the channel.
            if matches!(
                run.push_status,
                Some(ScheduledTaskPushStatus::Pending | ScheduledTaskPushStatus::Summarizing)
            ) {
                let _ = self.update_run_push_status(
                    &run.id,
                    ScheduledTaskPushStatus::Failed,
                    None,
                    Some("task force-unlocked"),
                    None,
                );
            }
        }
        Ok(targets.len())
    }

    /// Best-effort stop of the underlying work a run launched. Failures are
    /// logged but never block the unlock — the goal is to release the task.
    fn cancel_run_underlying(&self, run: &ScheduledTaskRun) {
        match run.mode {
            TaskMode::Chat => {
                if let Some(session_id) = run.session_id.as_deref() {
                    if let Some(turn_id) = self.runtime.active_turn_id(session_id) {
                        if let Err(error) = self.runtime.cancel_turn(session_id, &turn_id) {
                            log::warn!(
                                "[scheduled-tasks] failed to cancel chat turn for run {}: {error:#}",
                                run.id
                            );
                        }
                    }
                }
            }
            TaskMode::Process | TaskMode::Teamwork | TaskMode::Brainstorm | TaskMode::Debate => {
                if let Some(astra_run_id) = run.astra_run_id.as_deref() {
                    if let Err(error) = self.astra.cancel_astra_run(CancelAstraRunRequest {
                        run_id: astra_run_id.to_string(),
                    }) {
                        log::warn!(
                            "[scheduled-tasks] failed to cancel Astra run for run {}: {error:#}",
                            run.id
                        );
                    }
                }
            }
        }
    }

    /// Free the runtime session backing a finished chat run so unattended runs
    /// don't accumulate in memory. Best-effort and time-bounded; no-op for
    /// thread runs (which have no runtime session of their own).
    fn cleanup_chat_session(&self, run: &ScheduledTaskRun) {
        if run.mode != TaskMode::Chat {
            return;
        }
        let Some(session_id) = run.session_id.as_deref() else {
            return;
        };
        let report = self
            .runtime
            .cleanup_session_bounded(session_id, CHAT_SESSION_CLEANUP_TIMEOUT);
        if let Some(error) = report.dispose_error {
            log::warn!(
                "[scheduled-tasks] failed to free chat session for run {}: {error}",
                run.id
            );
        }
    }

    /// Free a finished chat run's session only when no channel push is pending —
    /// a pending push still needs the transcript, so the push worker frees the
    /// session after sending instead.
    fn cleanup_chat_session_if_unpushed(&self, run: &ScheduledTaskRun) {
        if run.push_status.is_none() {
            self.cleanup_chat_session(run);
        }
    }
}

fn task_config_changed(next: &ScheduledTask, previous: &ScheduledTask) -> bool {
    next.name != previous.name
        || next.status != previous.status
        || next.schedule != previous.schedule
        || next.target != previous.target
}

fn task_has_running_run(task: &ScheduledTask) -> bool {
    task.runs
        .iter()
        .any(|run| run.status == ScheduledTaskRunStatus::Running)
}

fn stage_status_str(status: StageStatus) -> &'static str {
    status.as_str()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let head_len = max_chars.saturating_sub(200);
    let head = trimmed.chars().take(head_len).collect::<String>();
    let tail = trimmed
        .chars()
        .rev()
        .take(180)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}\n\n...[truncated]...\n\n{tail}")
}

fn spawn_completion_watcher(state: Arc<SchedulerState>) -> Result<()> {
    thread::Builder::new()
        .name("scheduled-task-completions".to_string())
        .spawn({
            let state = state.clone();
            move || loop {
                state.process_run_completions();
                thread::sleep(COMPLETION_CHECK_INTERVAL);
            }
        })?;
    thread::Builder::new()
        .name("scheduled-task-pushes".to_string())
        .spawn(move || loop {
            state.process_run_pushes();
            thread::sleep(PUSH_CHECK_INTERVAL);
        })?;
    Ok(())
}

enum TaskRunOutcome {
    Chat(AgentSessionHandle),
    Thread(ThreadRunOutcome),
}

struct ThreadRunOutcome {
    thread: ThreadInfo,
    run: AstraHandle,
}

fn validate_thread_defaults(
    kind: ThreadKind,
    process_stage_ids: &[String],
    assistant_ids: &[String],
    agent_participants: &[ThreadAgentInfo],
) -> Result<()> {
    match kind {
        ThreadKind::Process => {
            if process_stage_ids.is_empty() {
                bail!("process thread requires at least one stage");
            }
        }
        ThreadKind::Teamwork => {
            if assistant_ids.is_empty() {
                bail!("teamwork thread requires at least one assistant");
            }
        }
        ThreadKind::Brainstorm => {
            if agent_participants.len() < 2 {
                bail!("brainstorm thread requires at least two participants");
            }
        }
        ThreadKind::Debate => {
            if agent_participants.len() != 2 {
                bail!("debate thread requires exactly two participants");
            }
        }
    }
    Ok(())
}

fn ensure_unique(values: &[String], label: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for value in values {
        if !seen.insert(value) {
            bail!("duplicate {label}: {value}");
        }
    }
    Ok(())
}

fn load_tasks_from_store(store: &dyn SessionStore) -> Result<Vec<ScheduledTask>> {
    let mut runs = store
        .list_scheduled_task_runs()?
        .into_iter()
        .map(scheduled_task_run_from_record)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .fold(
            HashMap::<String, Vec<ScheduledTaskRun>>::new(),
            |mut acc, run| {
                acc.entry(run.task_id.clone()).or_default().push(run);
                acc
            },
        );
    store
        .list_scheduled_tasks()?
        .into_iter()
        .map(|record| scheduled_task_from_record(record, &mut runs))
        .collect()
}

fn scheduled_task_from_record(
    record: ScheduledTaskRecord,
    runs: &mut HashMap<String, Vec<ScheduledTaskRun>>,
) -> Result<ScheduledTask> {
    let schedule = serde_json::from_str::<Schedule>(&record.schedule_json)
        .with_context(|| format!("parse scheduled task schedule {}", record.id))?;
    let target = serde_json::from_str::<TaskTarget>(&record.target_json)
        .with_context(|| format!("parse scheduled task target {}", record.id))?;
    let status = ScheduledTaskStatus::from_db_str(&record.status)
        .ok_or_else(|| anyhow::anyhow!("invalid scheduled task status {}", record.status))?;
    Ok(ScheduledTask {
        id: record.id.clone(),
        name: record.name,
        status,
        enabled: true,
        prompt: String::new(),
        schedule,
        target,
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
        last_run_at_ms: record.last_run_at_ms,
        runs: runs.remove(&record.id).unwrap_or_default(),
    })
}

fn tasks_to_records(tasks: &[ScheduledTask]) -> Result<Vec<ScheduledTaskRecord>> {
    tasks
        .iter()
        .enumerate()
        .map(|(index, task)| scheduled_task_to_record(task, index as i64))
        .collect()
}

fn scheduled_task_to_record(task: &ScheduledTask, sort_order: i64) -> Result<ScheduledTaskRecord> {
    Ok(ScheduledTaskRecord {
        id: task.id.clone(),
        name: task.name.clone(),
        status: task.status.as_str().to_string(),
        schedule_json: serde_json::to_string(&task.schedule)?,
        target_json: serde_json::to_string(&task.target)?,
        project_id: task.target.project_id().to_string(),
        mode: task.target.mode().as_str().to_string(),
        sort_order,
        created_at_ms: task.created_at_ms,
        updated_at_ms: task.updated_at_ms,
        last_run_at_ms: task.last_run_at_ms,
    })
}

fn scheduled_task_run_from_record(record: ScheduledTaskRunRecord) -> Result<ScheduledTaskRun> {
    let mode = TaskMode::from_db_str(&record.mode)
        .ok_or_else(|| anyhow::anyhow!("invalid scheduled task run mode {}", record.mode))?;
    let trigger = ScheduledTaskRunTrigger::from_db_str(&record.trigger)
        .ok_or_else(|| anyhow::anyhow!("invalid scheduled task run trigger {}", record.trigger))?;
    let status = ScheduledTaskRunStatus::from_db_str(&record.status)
        .ok_or_else(|| anyhow::anyhow!("invalid scheduled task run status {}", record.status))?;
    let push_status = record
        .push_status
        .as_deref()
        .map(|value| {
            ScheduledTaskPushStatus::from_db_str(value)
                .ok_or_else(|| anyhow::anyhow!("invalid scheduled task push status {value}"))
        })
        .transpose()?;
    let task_target = record
        .target_json
        .as_deref()
        .map(serde_json::from_str::<TaskTarget>)
        .transpose()
        .with_context(|| format!("parse scheduled task run target {}", record.id))?;
    Ok(ScheduledTaskRun {
        id: record.id,
        task_id: record.task_id,
        mode,
        trigger,
        status,
        started_at_ms: record.started_at_ms,
        scheduled_for_ms: record.scheduled_for_ms,
        completed_at_ms: record.completed_at_ms,
        task_name: record.task_name,
        task_target,
        session_agent: record.session_agent,
        session_id: record.session_id,
        agent_session_id: record.agent_session_id,
        thread_id: record.thread_id,
        astra_run_id: record.astra_run_id,
        push_platform: record.push_platform,
        push_chat_id: record.push_chat_id,
        push_status,
        push_summary: record.push_summary,
        push_error: record.push_error,
        push_sent_at_ms: record.push_sent_at_ms,
        error: record.error,
    })
}

fn scheduled_task_run_to_record(run: &ScheduledTaskRun) -> Result<ScheduledTaskRunRecord> {
    Ok(ScheduledTaskRunRecord {
        id: run.id.clone(),
        task_id: run.task_id.clone(),
        mode: run.mode.as_str().to_string(),
        trigger: run.trigger.as_str().to_string(),
        status: run.status.as_str().to_string(),
        started_at_ms: run.started_at_ms,
        scheduled_for_ms: run.scheduled_for_ms,
        completed_at_ms: run.completed_at_ms,
        task_name: run.task_name.clone(),
        target_json: run
            .task_target
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?,
        session_agent: run.session_agent,
        session_id: run.session_id.clone(),
        agent_session_id: run.agent_session_id.clone(),
        thread_id: run.thread_id.clone(),
        astra_run_id: run.astra_run_id.clone(),
        push_platform: run.push_platform.clone(),
        push_chat_id: run.push_chat_id.clone(),
        push_status: run.push_status.map(|status| status.as_str().to_string()),
        push_summary: run.push_summary.clone(),
        push_error: run.push_error.clone(),
        push_sent_at_ms: run.push_sent_at_ms,
        error: run.error.clone(),
    })
}

fn scheduled_task_run_from_outcome(
    task: &ScheduledTask,
    outcome: &TaskRunOutcome,
    when_ms: i64,
    trigger: ScheduledTaskRunTrigger,
    scheduled_for_ms: Option<i64>,
) -> ScheduledTaskRun {
    let (push_platform, push_chat_id, push_status) = task
        .target
        .im_push()
        .filter(|target| target.enabled)
        .map(|target| {
            (
                Some(target.platform.trim().to_string()),
                Some(target.chat_id.trim().to_string()),
                Some(ScheduledTaskPushStatus::Pending),
            )
        })
        .unwrap_or((None, None, None));
    match outcome {
        TaskRunOutcome::Chat(handle) => ScheduledTaskRun {
            id: stable_task_run_id(
                &task.id,
                TaskMode::Chat,
                when_ms,
                &handle.sessio_runtime_session_id,
            ),
            task_id: task.id.clone(),
            mode: TaskMode::Chat,
            trigger,
            status: ScheduledTaskRunStatus::Running,
            started_at_ms: when_ms,
            scheduled_for_ms,
            completed_at_ms: None,
            task_name: Some(task.name.clone()),
            task_target: Some(task.target.clone()),
            session_agent: Some(handle.agent),
            session_id: Some(handle.sessio_runtime_session_id.clone()),
            // Real ACP id is unknown until stamp completes; the stamp helper
            // backfills this column via update_scheduled_task_run_agent_session_id.
            agent_session_id: None,
            thread_id: None,
            astra_run_id: None,
            push_platform,
            push_chat_id,
            push_status,
            push_summary: None,
            push_error: None,
            push_sent_at_ms: None,
            error: None,
        },
        TaskRunOutcome::Thread(outcome) => {
            let mode = task.target.mode();
            ScheduledTaskRun {
                id: stable_task_run_id(&task.id, mode, when_ms, &outcome.thread.id),
                task_id: task.id.clone(),
                mode,
                trigger,
                status: ScheduledTaskRunStatus::Running,
                started_at_ms: when_ms,
                scheduled_for_ms,
                completed_at_ms: None,
                task_name: Some(task.name.clone()),
                task_target: Some(task.target.clone()),
                session_agent: None,
                session_id: None,
                agent_session_id: None,
                thread_id: Some(outcome.thread.id.clone()),
                astra_run_id: Some(outcome.run.run_id.clone()),
                push_platform,
                push_chat_id,
                push_status,
                push_summary: None,
                push_error: None,
                push_sent_at_ms: None,
                error: None,
            }
        }
    }
}

fn stable_task_run_id(task_id: &str, mode: TaskMode, when_ms: i64, output_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(task_id.as_bytes());
    hasher.update(mode.as_str().as_bytes());
    hasher.update(when_ms.to_le_bytes());
    hasher.update(output_id.as_bytes());
    let digest = hasher.finalize();
    format!("task-run-{}", hex::encode(&digest[..8]))
}

#[derive(Clone)]
pub struct ScheduledTasksService {
    state: Arc<SchedulerState>,
}

impl ScheduledTasksService {
    pub fn new(
        store: Arc<dyn SessionStore>,
        runtime: RuntimeManager,
        astra: AstraService,
        bridge: Option<ImBridgeService>,
    ) -> Self {
        // A push interrupted by a previous shutdown may have already delivered
        // its notification; mark it failed rather than re-summarizing and
        // double-notifying on restart.
        if let Err(error) = store.fail_interrupted_task_run_pushes() {
            log::warn!("[scheduled-tasks] failed to recover interrupted pushes: {error:#}");
        }
        let tasks = match load_tasks_from_store(store.as_ref()) {
            Ok(tasks) => tasks,
            Err(error) => {
                log::warn!("[scheduled-tasks] failed to load from sqlite: {error:#}");
                Vec::new()
            }
        };
        Self {
            state: Arc::new(SchedulerState {
                tasks: Mutex::new(tasks),
                runtime,
                astra,
                store,
                bridge,
            }),
        }
    }

    pub fn start(&self) -> Result<()> {
        scheduler::spawn(self.state.clone())?;
        spawn_completion_watcher(self.state.clone())?;
        log::info!("[scheduled-tasks] started");
        Ok(())
    }

    pub fn list(&self) -> Vec<ScheduledTask> {
        match load_tasks_from_store(self.state.store.as_ref()) {
            Ok(tasks) => {
                if let Ok(mut guard) = self.state.tasks.lock() {
                    *guard = tasks.clone();
                }
                tasks
            }
            Err(error) => {
                log::warn!("[scheduled-tasks] failed to refresh list from sqlite: {error:#}");
                self.state.snapshot()
            }
        }
    }

    pub fn save(&self, config: ScheduledTasksConfig) -> Result<Vec<ScheduledTask>> {
        self.state.replace_tasks(config)
    }

    pub fn run_now(&self, id: &str) -> Result<()> {
        let task = self
            .state
            .snapshot()
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("no scheduled task with id {id}"))?;
        if task_has_running_run(&task) {
            bail!("scheduled task {id} is already running");
        }
        let now = now_ms();
        let outcome = self.state.execute(&task)?;
        let recorded =
            self.state
                .record_run(&task, &outcome, now, ScheduledTaskRunTrigger::Manual, None);
        if let Some(run) = recorded {
            self.state.mark_ran(id, now);
            self.state.kick_off_chat_stamp(&task, &outcome, &run.id);
        } else {
            self.state.cleanup_unrecorded_outcome(&outcome);
        }
        Ok(())
    }

    /// Cancel a stuck task's running runs and unlock it for editing/deletion.
    pub fn force_unlock(&self, id: &str) -> Result<()> {
        let count = self.state.force_unlock(id)?;
        log::info!("[scheduled-tasks] force-unlocked task {id}: cancelled {count} run(s)");
        Ok(())
    }
}
