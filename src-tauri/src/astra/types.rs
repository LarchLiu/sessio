use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::Agent;
use crate::models::PlanRoundMode;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AstraRunStatus {
    Planning,
    Thinking,
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
            Self::Thinking => "thinking",
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
            "thinking" => Some(Self::Thinking),
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
            Self::Planning
                | Self::Thinking
                | Self::AwaitingApproval
                | Self::Dispatching
                | Self::Running
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_id: Option<String>,
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
    pub mode: String,
    pub planner_backend: Option<String>,
    pub round_index: Option<u32>,
    pub round_limit: u32,
    pub terminal_reason: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub internal_planner_session_ids: Vec<String>,
    pub run_diagnostics: Vec<Value>,
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
    pub mode: String,
    pub planner_backend: Option<String>,
    pub round_index: Option<u32>,
    pub round_limit: u32,
    pub terminal_reason: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub internal_planner_session_ids: Vec<String>,
    pub run_diagnostics: Vec<Value>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
pub struct AstraPlan {
    pub summary: String,
    pub tasks: Vec<AstraTaskProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraTaskCompletion {
    pub task: AstraTaskProposal,
    pub result: AstraTaskResult,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AstraRunIntent {
    Continue,
    Complete,
    WaitForHuman,
    Error,
}

impl AstraRunIntent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Complete => "complete",
            Self::WaitForHuman => "wait_for_human",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AstraOrchestration {
    pub summary: String,
    pub run_intent: AstraRunIntent,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<PlanRoundMode>,
    #[serde(default)]
    pub tasks: Vec<AstraTaskProposal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Value>,
}
