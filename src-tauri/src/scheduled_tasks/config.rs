use anyhow::{bail, Context, Result};
use croner::Cron;
use serde::{Deserialize, Serialize};

use crate::models::{Agent, ThreadAgentInfo, ThreadKind};

/// Top-level scheduled-tasks document persisted to YAML.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTasksConfig {
    #[serde(default)]
    pub tasks: Vec<ScheduledTask>,
}

/// A single auto task: a scheduled New Chat template plus when to run it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    /// Stable identifier. Filled in on save when empty (see [`ensure_ids`]).
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_task_status")]
    pub status: ScheduledTaskStatus,
    /// Legacy pre-status flag. Deserialized only so old callers/configs that
    /// send `enabled: false` are normalized to `status: paused`.
    #[serde(default = "default_true")]
    #[serde(skip_serializing)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    pub schedule: Schedule,
    pub target: TaskTarget,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
    /// Last successful trigger time. Persisted so a restart does not re-fire a
    /// task that already ran inside its current window.
    #[serde(default)]
    pub last_run_at_ms: Option<i64>,
    #[serde(default)]
    pub runs: Vec<ScheduledTaskRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTaskRun {
    pub id: String,
    pub task_id: String,
    pub mode: TaskMode,
    #[serde(default = "default_run_trigger")]
    pub trigger: ScheduledTaskRunTrigger,
    #[serde(default = "default_run_status")]
    pub status: ScheduledTaskRunStatus,
    pub started_at_ms: i64,
    #[serde(default)]
    pub scheduled_for_ms: Option<i64>,
    #[serde(default)]
    pub completed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_target: Option<TaskTarget>,
    #[serde(default)]
    pub session_agent: Option<Agent>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub astra_run_id: Option<String>,
    #[serde(default)]
    pub push_platform: Option<String>,
    #[serde(default)]
    pub push_chat_id: Option<String>,
    #[serde(default)]
    pub push_status: Option<ScheduledTaskPushStatus>,
    #[serde(default)]
    pub push_summary: Option<String>,
    #[serde(default)]
    pub push_error: Option<String>,
    #[serde(default)]
    pub push_sent_at_ms: Option<i64>,
    /// Failure reason for a `failed` run (TTL/stall/detect error). `None` for
    /// runs that completed or were cancelled normally.
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledTaskRunTrigger {
    Scheduled,
    Manual,
}

impl ScheduledTaskRunTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduledTaskRunTrigger::Scheduled => "scheduled",
            ScheduledTaskRunTrigger::Manual => "manual",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "scheduled" => Some(ScheduledTaskRunTrigger::Scheduled),
            "manual" => Some(ScheduledTaskRunTrigger::Manual),
            _ => None,
        }
    }
}

fn default_run_trigger() -> ScheduledTaskRunTrigger {
    ScheduledTaskRunTrigger::Scheduled
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledTaskRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl ScheduledTaskRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduledTaskRunStatus::Running => "running",
            ScheduledTaskRunStatus::Completed => "completed",
            ScheduledTaskRunStatus::Failed => "failed",
            ScheduledTaskRunStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "running" => Some(ScheduledTaskRunStatus::Running),
            "completed" => Some(ScheduledTaskRunStatus::Completed),
            "failed" => Some(ScheduledTaskRunStatus::Failed),
            "cancelled" => Some(ScheduledTaskRunStatus::Cancelled),
            _ => None,
        }
    }
}

fn default_run_status() -> ScheduledTaskRunStatus {
    // Missing status should be terminal so a malformed config never blocks
    // task edits.
    ScheduledTaskRunStatus::Completed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledTaskPushStatus {
    Pending,
    Summarizing,
    Sent,
    Failed,
}

impl ScheduledTaskPushStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduledTaskPushStatus::Pending => "pending",
            ScheduledTaskPushStatus::Summarizing => "summarizing",
            ScheduledTaskPushStatus::Sent => "sent",
            ScheduledTaskPushStatus::Failed => "failed",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(ScheduledTaskPushStatus::Pending),
            "summarizing" => Some(ScheduledTaskPushStatus::Summarizing),
            "sent" => Some(ScheduledTaskPushStatus::Sent),
            "failed" => Some(ScheduledTaskPushStatus::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledTaskStatus {
    Active,
    Paused,
}

impl ScheduledTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ScheduledTaskStatus::Active => "active",
            ScheduledTaskStatus::Paused => "paused",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "active" => Some(ScheduledTaskStatus::Active),
            "paused" => Some(ScheduledTaskStatus::Paused),
            _ => None,
        }
    }
}

fn default_task_status() -> ScheduledTaskStatus {
    ScheduledTaskStatus::Active
}

/// When a task should fire. Simple recurrences are computed in-house; `Cron`
/// delegates to the `cron` crate.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    /// Every `every_secs` seconds, measured from the last run (or task creation).
    Interval { every_secs: u64 },
    /// Every day at `hour:minute` (local time).
    Daily { hour: u8, minute: u8 },
    /// Every week on `weekday` (0 = Sunday .. 6 = Saturday) at `hour:minute`.
    Weekly { weekday: u8, hour: u8, minute: u8 },
    /// Standard 5-field cron expression, evaluated in local time.
    Cron { expr: String },
}

/// How a fired task starts local work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskMode {
    Chat,
    Process,
    Teamwork,
    Brainstorm,
    Debate,
}

impl TaskMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskMode::Chat => "chat",
            TaskMode::Process => "process",
            TaskMode::Teamwork => "teamwork",
            TaskMode::Brainstorm => "brainstorm",
            TaskMode::Debate => "debate",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "chat" => Some(TaskMode::Chat),
            "process" => Some(TaskMode::Process),
            "teamwork" => Some(TaskMode::Teamwork),
            "brainstorm" => Some(TaskMode::Brainstorm),
            "debate" => Some(TaskMode::Debate),
            _ => None,
        }
    }

    pub fn as_thread_kind(self) -> Option<ThreadKind> {
        match self {
            TaskMode::Chat => None,
            TaskMode::Process => Some(ThreadKind::Process),
            TaskMode::Teamwork => Some(ThreadKind::Teamwork),
            TaskMode::Brainstorm => Some(ThreadKind::Brainstorm),
            TaskMode::Debate => Some(ThreadKind::Debate),
        }
    }
}

/// Optional notification destination. This is outbound-only: it must not create
/// or drive an IM-owned runtime session.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImPushTarget {
    #[serde(default)]
    pub enabled: bool,
    pub platform: String,
    pub chat_id: String,
}

/// Where a fired task's local work starts. `mode = chat` starts a normal runtime
/// session; thread modes create a fresh thread on every run from this template.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum TaskTarget {
    Chat {
        #[serde(rename = "projectId", alias = "project_id")]
        project_id: String,
        #[serde(default)]
        prompt: String,
        agent: Agent,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default, rename = "permissionMode", alias = "permission_mode")]
        permission_mode: Option<String>,
        #[serde(default, rename = "imPush", alias = "im_push")]
        im_push: Option<ImPushTarget>,
    },
    Process {
        #[serde(rename = "projectId", alias = "project_id")]
        project_id: String,
        goal: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default, rename = "stageIds", alias = "stage_ids")]
        stage_ids: Vec<String>,
        #[serde(default, rename = "imPush", alias = "im_push")]
        im_push: Option<ImPushTarget>,
    },
    Teamwork {
        #[serde(rename = "projectId", alias = "project_id")]
        project_id: String,
        goal: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default, rename = "assistantIds", alias = "assistant_ids")]
        assistant_ids: Vec<String>,
        #[serde(default, rename = "imPush", alias = "im_push")]
        im_push: Option<ImPushTarget>,
    },
    Brainstorm {
        #[serde(rename = "projectId", alias = "project_id")]
        project_id: String,
        goal: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default, rename = "agentParticipants", alias = "agent_participants")]
        agent_participants: Vec<ThreadAgentInfo>,
        #[serde(default, rename = "imPush", alias = "im_push")]
        im_push: Option<ImPushTarget>,
    },
    Debate {
        #[serde(rename = "projectId", alias = "project_id")]
        project_id: String,
        goal: String,
        #[serde(default)]
        description: Option<String>,
        #[serde(default, rename = "agentParticipants", alias = "agent_participants")]
        agent_participants: Vec<ThreadAgentInfo>,
        #[serde(default, rename = "imPush", alias = "im_push")]
        im_push: Option<ImPushTarget>,
    },
}

impl TaskTarget {
    pub fn mode(&self) -> TaskMode {
        match self {
            TaskTarget::Chat { .. } => TaskMode::Chat,
            TaskTarget::Process { .. } => TaskMode::Process,
            TaskTarget::Teamwork { .. } => TaskMode::Teamwork,
            TaskTarget::Brainstorm { .. } => TaskMode::Brainstorm,
            TaskTarget::Debate { .. } => TaskMode::Debate,
        }
    }

    pub fn project_id(&self) -> &str {
        match self {
            TaskTarget::Chat { project_id, .. }
            | TaskTarget::Process { project_id, .. }
            | TaskTarget::Teamwork { project_id, .. }
            | TaskTarget::Brainstorm { project_id, .. }
            | TaskTarget::Debate { project_id, .. } => project_id,
        }
    }

    pub fn im_push(&self) -> Option<&ImPushTarget> {
        match self {
            TaskTarget::Chat { im_push, .. }
            | TaskTarget::Process { im_push, .. }
            | TaskTarget::Teamwork { im_push, .. }
            | TaskTarget::Brainstorm { im_push, .. }
            | TaskTarget::Debate { im_push, .. } => im_push.as_ref(),
        }
    }

    pub fn thread_goal(&self) -> Option<&str> {
        match self {
            TaskTarget::Chat { .. } => None,
            TaskTarget::Process { goal, .. }
            | TaskTarget::Teamwork { goal, .. }
            | TaskTarget::Brainstorm { goal, .. }
            | TaskTarget::Debate { goal, .. } => Some(goal),
        }
    }

    pub fn thread_description(&self) -> Option<&str> {
        match self {
            TaskTarget::Chat { .. } => None,
            TaskTarget::Process { description, .. }
            | TaskTarget::Teamwork { description, .. }
            | TaskTarget::Brainstorm { description, .. }
            | TaskTarget::Debate { description, .. } => description.as_deref(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Current wall-clock in epoch millis. Uses chrono (already a dependency).
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl ScheduledTask {
    /// Derive a stable id from name + creation time when one is missing.
    fn fill_id(&mut self, index: usize) {
        if !self.id.trim().is_empty() {
            return;
        }
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.name.as_bytes());
        hasher.update(self.created_at_ms.to_le_bytes());
        hasher.update((index as u64).to_le_bytes());
        let digest = hasher.finalize();
        self.id = format!("task-{}", hex::encode(&digest[..8]));
    }
}

/// Ensure every task has a non-empty id and timestamps. Mutates in place; called
/// before persisting a config that may have been built by the frontend.
pub fn ensure_ids(config: &mut ScheduledTasksConfig) {
    let now = now_ms();
    for (index, task) in config.tasks.iter_mut().enumerate() {
        if let TaskTarget::Chat { prompt, .. } = &mut task.target {
            if prompt.trim().is_empty() && !task.prompt.trim().is_empty() {
                *prompt = task.prompt.trim().to_string();
            }
        }
        if !task.enabled {
            task.status = ScheduledTaskStatus::Paused;
            task.enabled = true;
        }
        if task.created_at_ms == 0 {
            task.created_at_ms = now;
        }
        if task.updated_at_ms == 0 {
            task.updated_at_ms = now;
        }
        task.fill_id(index);
    }
}

/// Validate user-editable task settings before replacing the persisted config.
pub fn validate_config(config: &ScheduledTasksConfig) -> Result<()> {
    for (index, task) in config.tasks.iter().enumerate() {
        validate_task(task).with_context(|| {
            let label = task.name.trim();
            if label.is_empty() {
                format!("invalid scheduled task at index {index}")
            } else {
                format!("invalid scheduled task {label:?}")
            }
        })?;
    }
    Ok(())
}

fn validate_task(task: &ScheduledTask) -> Result<()> {
    if task.name.trim().is_empty() {
        bail!("task name is empty");
    }
    validate_schedule(&task.schedule)?;
    if task.target.project_id().trim().is_empty() {
        bail!("task project is empty");
    }
    match &task.target {
        TaskTarget::Chat { .. } if task_chat_prompt(task).is_empty() => {
            bail!("task prompt is empty");
        }
        TaskTarget::Process {
            goal, stage_ids, ..
        } => {
            validate_thread_goal(goal)?;
            if stage_ids.is_empty() {
                bail!("process thread requires at least one stage");
            }
        }
        TaskTarget::Teamwork {
            goal,
            assistant_ids,
            ..
        } => {
            validate_thread_goal(goal)?;
            if assistant_ids.is_empty() {
                bail!("teamwork thread requires at least one assistant");
            }
        }
        TaskTarget::Brainstorm {
            goal,
            agent_participants,
            ..
        } => {
            validate_thread_goal(goal)?;
            if agent_participants.len() < 2 {
                bail!("brainstorm thread requires at least two participants");
            }
        }
        TaskTarget::Debate {
            goal,
            agent_participants,
            ..
        } => {
            validate_thread_goal(goal)?;
            if agent_participants.len() != 2 {
                bail!("debate thread requires exactly two participants");
            }
        }
        TaskTarget::Chat { .. } => {}
    }
    if let Some(im_push) = task.target.im_push() {
        if im_push.enabled {
            let platform = im_push.platform.trim();
            let chat_id = im_push.chat_id.trim();
            if platform.trim().is_empty() {
                bail!("IM push platform is empty");
            }
            if chat_id.trim().is_empty() {
                bail!("IM push chat id is empty");
            }
        }
    }
    Ok(())
}

pub fn task_chat_prompt(task: &ScheduledTask) -> &str {
    if let TaskTarget::Chat { prompt, .. } = &task.target {
        let prompt = prompt.trim();
        if !prompt.is_empty() {
            return prompt;
        }
    }
    task.prompt.trim()
}

fn validate_thread_goal(goal: &str) -> Result<()> {
    if goal.trim().is_empty() {
        bail!("thread goal is empty");
    }
    Ok(())
}

pub fn validate_schedule(schedule: &Schedule) -> Result<()> {
    match schedule {
        Schedule::Interval { every_secs } => {
            if *every_secs == 0 {
                bail!("interval must be greater than zero");
            }
        }
        Schedule::Daily { hour, minute } => {
            validate_time(*hour, *minute)?;
        }
        Schedule::Weekly {
            weekday,
            hour,
            minute,
        } => {
            if *weekday > 6 {
                bail!("weekday must be between 0 and 6");
            }
            validate_time(*hour, *minute)?;
        }
        Schedule::Cron { expr } => {
            let trimmed = expr.trim();
            if trimmed.split_whitespace().count() != 5 {
                bail!("cron expression must have exactly 5 fields");
            }
            Cron::new(trimmed)
                .parse()
                .context("cron expression is invalid")?;
        }
    }
    Ok(())
}

fn validate_time(hour: u8, minute: u8) -> Result<()> {
    if hour > 23 {
        bail!("hour must be between 0 and 23");
    }
    if minute > 59 {
        bail!("minute must be between 0 and 59");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_task(schedule: Schedule) -> ScheduledTask {
        ScheduledTask {
            id: "task-test".to_string(),
            name: "Test task".to_string(),
            status: ScheduledTaskStatus::Active,
            enabled: true,
            prompt: String::new(),
            schedule,
            target: TaskTarget::Chat {
                project_id: "project-test".to_string(),
                prompt: "Do the thing".to_string(),
                agent: Agent::Codex,
                model: None,
                effort: None,
                permission_mode: None,
                im_push: None,
            },
            created_at_ms: 1,
            updated_at_ms: 1,
            last_run_at_ms: None,
            runs: Vec::new(),
        }
    }

    #[test]
    fn validate_config_accepts_valid_cron_task() {
        let config = ScheduledTasksConfig {
            tasks: vec![valid_task(Schedule::Cron {
                expr: "*/5 * * * *".to_string(),
            })],
        };
        validate_config(&config).unwrap();
    }

    #[test]
    fn validate_config_rejects_invalid_cron_task() {
        let config = ScheduledTasksConfig {
            tasks: vec![valid_task(Schedule::Cron {
                expr: "not a cron".to_string(),
            })],
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn validate_config_rejects_empty_project() {
        let mut task = valid_task(Schedule::Daily { hour: 9, minute: 0 });
        task.target = TaskTarget::Chat {
            project_id: " ".to_string(),
            prompt: "Do the thing".to_string(),
            agent: Agent::Codex,
            model: None,
            effort: None,
            permission_mode: None,
            im_push: None,
        };
        assert!(validate_config(&ScheduledTasksConfig { tasks: vec![task] }).is_err());
    }

    #[test]
    fn validate_config_rejects_thread_mode_without_goal() {
        let mut task = valid_task(Schedule::Daily { hour: 9, minute: 0 });
        task.target = TaskTarget::Teamwork {
            project_id: "project-test".to_string(),
            goal: " ".to_string(),
            description: None,
            assistant_ids: vec!["assistant-test".to_string()],
            im_push: None,
        };
        assert!(validate_config(&ScheduledTasksConfig { tasks: vec![task] }).is_err());
    }

    #[test]
    fn validate_config_rejects_thread_mode_without_configuration() {
        let mut task = valid_task(Schedule::Daily { hour: 9, minute: 0 });
        task.target = TaskTarget::Process {
            project_id: "project-test".to_string(),
            goal: "Ship it".to_string(),
            description: None,
            stage_ids: Vec::new(),
            im_push: None,
        };
        assert!(validate_config(&ScheduledTasksConfig { tasks: vec![task] }).is_err());
    }

    #[test]
    fn task_target_deserializes_frontend_camel_case_fields() {
        let target = serde_json::from_value::<TaskTarget>(serde_json::json!({
            "mode": "chat",
            "projectId": "project-test",
            "prompt": "Do the thing",
            "agent": "codex",
            "model": "gpt-5",
            "effort": "medium",
            "permissionMode": "workspace-write",
            "imPush": {
                "enabled": true,
                "platform": "telegram",
                "chatId": "123456"
            }
        }))
        .unwrap();

        assert_eq!(
            target,
            TaskTarget::Chat {
                project_id: "project-test".to_string(),
                prompt: "Do the thing".to_string(),
                agent: Agent::Codex,
                model: Some("gpt-5".to_string()),
                effort: Some("medium".to_string()),
                permission_mode: Some("workspace-write".to_string()),
                im_push: Some(ImPushTarget {
                    enabled: true,
                    platform: "telegram".to_string(),
                    chat_id: "123456".to_string(),
                }),
            }
        );
    }

    #[test]
    fn task_target_serializes_variant_fields_as_camel_case() {
        let value = serde_json::to_value(TaskTarget::Process {
            project_id: "project-test".to_string(),
            goal: "Ship it".to_string(),
            description: Some("Run the release workflow".to_string()),
            stage_ids: vec!["stage-a".to_string(), "stage-b".to_string()],
            im_push: None,
        })
        .unwrap();

        assert_eq!(
            value.get("projectId").and_then(serde_json::Value::as_str),
            Some("project-test")
        );
        assert_eq!(
            value
                .get("stageIds")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(2)
        );
        assert!(value.get("project_id").is_none());
        assert!(value.get("stage_ids").is_none());
    }
}
