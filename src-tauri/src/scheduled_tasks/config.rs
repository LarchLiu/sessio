//! Scheduled (auto) tasks configuration, loaded from
//! `~/.sessio/scheduled-tasks.yaml`.
//!
//! Mirrors the `im_bridge::config` approach: a nested, user-editable schema
//! backed by `serde_yaml`. Kept in a dedicated file so a later migration to the
//! rusqlite store only has to replace [`load`]/[`save`] without touching the
//! data model or the scheduler.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use croner::Cron;
use serde::{Deserialize, Serialize};

use crate::models::Agent;

/// Top-level scheduled-tasks document persisted to YAML.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTasksConfig {
    #[serde(default)]
    pub tasks: Vec<ScheduledTask>,
}

/// A single auto task: a prompt plus when to run it and where to send it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledTask {
    /// Stable identifier. Filled in on save when empty (see [`ensure_ids`]).
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
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

/// Where a fired task's prompt goes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TaskTarget {
    /// Start a local runtime session; its output persists as a normal session.
    Local {
        workspace_path: String,
        agent: Agent,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        effort: Option<String>,
        #[serde(default)]
        permission_mode: Option<String>,
    },
    /// Deliver the prompt to a bound IM chat via the running bridge.
    Im { platform: String, chat_id: String },
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
    if task.prompt.trim().is_empty() {
        bail!("task prompt is empty");
    }
    validate_schedule(&task.schedule)?;
    match &task.target {
        TaskTarget::Local { workspace_path, .. } => {
            if workspace_path.trim().is_empty() {
                bail!("local target workspace path is empty");
            }
        }
        TaskTarget::Im { platform, chat_id } => {
            if platform.trim().is_empty() {
                bail!("IM target platform is empty");
            }
            if chat_id.trim().is_empty() {
                bail!("IM target chat id is empty");
            }
        }
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

/// Path to the config file: `~/.sessio/scheduled-tasks.yaml`.
fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".sessio").join("scheduled-tasks.yaml"))
}

/// Best-effort display string for the config path, for log messages.
pub fn config_path_display() -> String {
    config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.sessio/scheduled-tasks.yaml".to_string())
}

/// Load the config file. Returns an empty config when the file is absent or
/// blank, so callers can treat "no tasks configured" as a normal state.
pub fn load() -> Result<ScheduledTasksConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(ScheduledTasksConfig::default());
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("read scheduled-tasks config {}", path.display()))?;
    if contents.trim().is_empty() {
        return Ok(ScheduledTasksConfig::default());
    }
    let config: ScheduledTasksConfig = serde_yaml::from_str(&contents)
        .with_context(|| format!("parse scheduled-tasks config {}", path.display()))?;
    Ok(config)
}

/// Persist the config, filling in any missing ids/timestamps first.
pub fn save(config: &mut ScheduledTasksConfig) -> Result<()> {
    ensure_ids(config);
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create scheduled-tasks config dir {}", parent.display()))?;
    }
    let contents = serde_yaml::to_string(config)
        .with_context(|| format!("serialize scheduled-tasks config {}", path.display()))?;
    std::fs::write(&path, contents)
        .with_context(|| format!("write scheduled-tasks config {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_task(schedule: Schedule) -> ScheduledTask {
        ScheduledTask {
            id: "task-test".to_string(),
            name: "Test task".to_string(),
            enabled: true,
            prompt: "Do the thing".to_string(),
            schedule,
            target: TaskTarget::Local {
                workspace_path: "/tmp".to_string(),
                agent: Agent::Codex,
                model: None,
                effort: None,
                permission_mode: None,
            },
            created_at_ms: 1,
            updated_at_ms: 1,
            last_run_at_ms: None,
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
    fn validate_config_rejects_empty_local_workspace() {
        let mut task = valid_task(Schedule::Daily { hour: 9, minute: 0 });
        task.target = TaskTarget::Local {
            workspace_path: " ".to_string(),
            agent: Agent::Codex,
            model: None,
            effort: None,
            permission_mode: None,
        };
        assert!(validate_config(&ScheduledTasksConfig { tasks: vec![task] }).is_err());
    }
}
