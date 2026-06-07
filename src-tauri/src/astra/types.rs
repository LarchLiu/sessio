use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::Agent;

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
    pub proposed_tasks: Vec<AstraTaskProposal>,
    pub approved_task_ids: Vec<String>,
    pub delegated_session_ids: Vec<String>,
    pub task_results: Vec<AstraTaskResult>,
    pub mode: String,
    pub current_stage_id: Option<String>,
    pub current_task_id: Option<String>,
    pub completed_task_ids: Vec<String>,
    pub stage_attempt_counts: HashMap<String, u32>,
    pub retry_limit: u32,
    pub planner_backend: Option<String>,
    pub decision_backend: Option<String>,
    pub round_index: Option<u32>,
    pub round_limit: u32,
    pub terminal_reason: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub internal_planner_session_ids: Vec<String>,
    pub internal_decision_session_ids: Vec<String>,
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
    pub proposed_tasks: Vec<AstraTaskProposal>,
    pub approved_task_ids: Vec<String>,
    pub delegated_session_ids: Vec<String>,
    pub task_results: Vec<AstraTaskResult>,
    pub mode: String,
    pub current_stage_id: Option<String>,
    pub completed_task_ids: Vec<String>,
    pub stage_attempt_counts: HashMap<String, u32>,
    pub retry_limit: u32,
    pub planner_backend: Option<String>,
    pub decision_backend: Option<String>,
    pub round_index: Option<u32>,
    pub round_limit: u32,
    pub terminal_reason: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub internal_planner_session_ids: Vec<String>,
    pub internal_decision_session_ids: Vec<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_reason: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AstraTaskDecision {
    pub task_id: String,
    pub decision: AstraDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AstraOrchestration {
    pub summary: String,
    #[serde(default)]
    pub decisions: Vec<AstraTaskDecision>,
    #[serde(default)]
    pub tasks: Vec<AstraTaskProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "action")]
pub(crate) enum AstraDecision {
    UpdateStage { args: Value },
    AddOrUpdateIssue { args: Value },
    RetryStage { reason: String },
    PlanNextRound { reason: String },
    CancelRun { reason: String },
    CompleteRun { reason: String },
    ErrorRun { reason: String },
    Composite { decisions: Vec<AstraDecision> },
}
