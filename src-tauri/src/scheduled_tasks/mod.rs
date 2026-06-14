//! Scheduled ("auto") tasks: fire a saved prompt at a configured time and send
//! it to a local runtime session or an IM chat.
//!
//! Mirrors the IM bridge's shape: a [`ScheduledTasksService`] owns the live task
//! list plus the handles it needs to execute (runtime, store, optional bridge),
//! and runs a single background worker thread (see [`scheduler`]). Tasks persist
//! to `~/.sessio/scheduled-tasks.yaml` via [`config`].

mod config;
mod schedule;
mod scheduler;

pub use config::{Schedule, ScheduledTask, ScheduledTasksConfig, TaskTarget};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use serde_json::Value;

use crate::agents::runtime::types::StartAgentSession;
use crate::agents::runtime::RuntimeManager;
use crate::im_bridge::ImBridgeService;
use crate::models::Agent;
use crate::store::SessionStore;

use self::config::now_ms;
use self::schedule::next_run_after;

/// Shared scheduler state, held behind an `Arc` so the worker thread and the
/// Tauri command handlers operate on the same task list.
pub(crate) struct SchedulerState {
    tasks: Mutex<Vec<ScheduledTask>>,
    runtime: RuntimeManager,
    store: Arc<dyn SessionStore>,
    /// Present only when the IM bridge started this session. `Im`-targeted tasks
    /// require it; without it they error (and are logged) at execution time.
    bridge: Option<ImBridgeService>,
}

impl SchedulerState {
    /// Snapshot of the current task list.
    fn snapshot(&self) -> Vec<ScheduledTask> {
        self.tasks.lock().map(|t| t.clone()).unwrap_or_default()
    }

    /// Replace the in-memory list and persist it.
    fn replace_tasks(&self, mut config: ScheduledTasksConfig) -> Result<Vec<ScheduledTask>> {
        self.normalize_task_timestamps(&mut config);
        config::ensure_ids(&mut config);
        config::validate_config(&config)?;
        self.validate_local_target_agents(&config.tasks)?;
        config::save(&mut config)?;
        let tasks = config.tasks.clone();
        if let Ok(mut guard) = self.tasks.lock() {
            *guard = tasks.clone();
        }
        Ok(tasks)
    }

    /// Record a task's last run time in memory and persist the whole list.
    fn mark_ran(&self, task_id: &str, when_ms: i64) {
        let snapshot = {
            let Ok(mut guard) = self.tasks.lock() else {
                return;
            };
            if let Some(task) = guard.iter_mut().find(|t| t.id == task_id) {
                task.last_run_at_ms = Some(when_ms);
            }
            guard.clone()
        };
        let mut config = ScheduledTasksConfig { tasks: snapshot };
        if let Err(error) = config::save(&mut config) {
            log::warn!("[scheduled-tasks] failed to persist last-run time: {error:#}");
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

    /// Fire any task whose next run (after its last run/creation) is `<= now`.
    /// Called once per worker tick.
    fn tick(&self, now: i64) {
        for task in self.snapshot() {
            if !task.enabled {
                continue;
            }
            let after = task.last_run_at_ms.unwrap_or(task.created_at_ms);
            let Some(fire_at) = next_run_after(&task.schedule, after) else {
                continue;
            };
            if fire_at > now {
                continue;
            }
            self.run(&task, now);
        }
    }

    /// Execute one task immediately and record its run time.
    fn run(&self, task: &ScheduledTask, now: i64) {
        match self.execute(task) {
            Ok(()) => {
                log::info!("[scheduled-tasks] ran task {} ({})", task.id, task.name);
                self.mark_ran(&task.id, now);
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

    /// Dispatch a task's prompt to its target.
    fn execute(&self, task: &ScheduledTask) -> Result<()> {
        if task.prompt.trim().is_empty() {
            bail!("task prompt is empty");
        }
        match &task.target {
            TaskTarget::Local {
                workspace_path,
                agent,
                model,
                effort,
                permission_mode,
            } => {
                self.ensure_local_agent_available(*agent)?;
                let mut req = StartAgentSession {
                    agent: *agent,
                    workspace_path: workspace_path.clone(),
                    initial_prompt: Some(task.prompt.clone()),
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
                self.runtime.start_session(req)?;
                Ok(())
            }
            TaskTarget::Im { platform, chat_id } => {
                let Some(bridge) = &self.bridge else {
                    bail!("IM bridge is not running; cannot deliver task to {platform}");
                };
                bridge.submit_prompt_to_chat(platform, chat_id, &task.prompt)
            }
        }
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
            if let TaskTarget::Local { agent, .. } = &task.target {
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
}

fn task_config_changed(next: &ScheduledTask, previous: &ScheduledTask) -> bool {
    next.name != previous.name
        || next.enabled != previous.enabled
        || next.prompt != previous.prompt
        || next.schedule != previous.schedule
        || next.target != previous.target
}

/// Owns the scheduler worker. Held in Tauri managed state for the app lifetime.
#[derive(Clone)]
pub struct ScheduledTasksService {
    state: Arc<SchedulerState>,
}

impl ScheduledTasksService {
    /// Construct the service, loading the persisted task list. Logs and falls
    /// back to an empty list on a malformed config file.
    pub fn new(
        store: Arc<dyn SessionStore>,
        runtime: RuntimeManager,
        bridge: Option<ImBridgeService>,
    ) -> Self {
        let tasks = match config::load() {
            Ok(config) => config.tasks,
            Err(error) => {
                log::warn!(
                    "[scheduled-tasks] failed to load {}: {error:#}",
                    config::config_path_display()
                );
                Vec::new()
            }
        };
        Self {
            state: Arc::new(SchedulerState {
                tasks: Mutex::new(tasks),
                runtime,
                store,
                bridge,
            }),
        }
    }

    /// Start the background worker thread. Returns immediately.
    pub fn start(&self) -> Result<()> {
        scheduler::spawn(self.state.clone())?;
        log::info!("[scheduled-tasks] started");
        Ok(())
    }

    /// Current task list.
    pub fn list(&self) -> Vec<ScheduledTask> {
        self.state.snapshot()
    }

    /// Replace the whole task list (the frontend always saves the full set).
    /// Returns the normalized list (ids/timestamps filled in).
    pub fn save(&self, config: ScheduledTasksConfig) -> Result<Vec<ScheduledTask>> {
        self.state.replace_tasks(config)
    }

    /// Run a single task right now by id, regardless of its schedule.
    pub fn run_now(&self, id: &str) -> Result<()> {
        let task = self
            .state
            .snapshot()
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow::anyhow!("no scheduled task with id {id}"))?;
        self.state.execute(&task)?;
        self.state.mark_ran(id, now_ms());
        Ok(())
    }
}
