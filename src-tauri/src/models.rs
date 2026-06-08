use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agents::runtime::types::{RuntimeCapabilitySet, RuntimeTransportKind};

const SESSIO_ATTACHMENT_MARKER: &str = "__sessio_attachment__:";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    #[serde(rename = "astra-pi")]
    AstraPi,
    Codex,
    Claude,
    Gemini,
}

impl Agent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Agent::AstraPi => "astra-pi",
            Agent::Codex => "codex",
            Agent::Claude => "claude",
            Agent::Gemini => "gemini",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "astra-pi" => Some(Agent::AstraPi),
            "codex" => Some(Agent::Codex),
            "claude" => Some(Agent::Claude),
            "gemini" => Some(Agent::Gemini),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub agent: Agent,
    pub forked_from_agent: Option<Agent>,
    pub forked_from_id: Option<String>,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub message_count: usize,
    #[serde(default)]
    pub rename_title: Option<String>,
    pub title: Option<String>,
    pub first_user_message: Option<String>,
    pub file_path: String,
    pub file_size: u64,
    pub partial: bool,
    pub available: bool,
    pub archived: bool,
    #[serde(default)]
    pub subagents: Vec<SubagentInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfo {
    pub id: String,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub message_count: usize,
    pub first_user_message: Option<String>,
    pub file_path: String,
    pub file_size: u64,
    pub partial: bool,
    #[serde(default = "default_available")]
    pub available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowType {
    Builtin,
    Custom,
}

impl WorkflowType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowType::Builtin => "builtin",
            WorkflowType::Custom => "custom",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "builtin" => Some(WorkflowType::Builtin),
            "custom" => Some(WorkflowType::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub workflow_type: WorkflowType,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub workflow_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub session_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    Builtin,
    Custom,
}

impl AgentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentType::Builtin => "builtin",
            AgentType::Custom => "custom",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "builtin" => Some(AgentType::Builtin),
            "custom" => Some(AgentType::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub icon: Option<String>,
    pub ai_provider: Option<String>,
    pub ai_providers: Vec<AgentAiProviderInfo>,
    pub model: Option<String>,
    pub models: Vec<RuntimeAgentOptionMetadata>,
    pub effort: Option<String>,
    pub efforts: Vec<RuntimeAgentOptionMetadata>,
    pub permission_mode: Option<String>,
    pub permission_modes: Vec<RuntimeAgentOptionMetadata>,
    #[serde(rename = "type")]
    pub agent_type: AgentType,
    pub enabled: bool,
    pub transport: RuntimeTransportKind,
    pub commands: AgentCommandsInfo,
    pub order: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraConfig {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAiProviderInfo {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub api: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub models: Vec<RuntimeAgentOptionMetadata>,
    pub enabled: bool,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandsInfo {
    pub session: Vec<String>,
    pub version: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AssistantType {
    Builtin,
    Custom,
}

impl AssistantType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AssistantType::Builtin => "builtin",
            AssistantType::Custom => "custom",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "builtin" => Some(AssistantType::Builtin),
            "custom" => Some(AssistantType::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAgentInfo {
    pub id: String,
    pub name: String,
    pub model: String,
    pub mode: String,
    pub effort: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantInfo {
    pub id: String,
    pub name: String,
    pub agent: AssistantAgentInfo,
    pub system_prompt: Option<String>,
    pub color: Option<String>,
    #[serde(rename = "type")]
    pub assistant_type: AssistantType,
    pub workflow_id: Option<String>,
    pub project_id: Option<String>,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageAssistantInfo {
    pub assistant_id: String,
    pub name: String,
    pub color: Option<String>,
    pub agent: AssistantAgentInfo,
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub order: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StageType {
    Research,
    Plan,
    Develop,
    Build,
    Writing,
    Editing,
    Review,
    Proofreading,
    Screenplay,
    Storyboard,
    Design,
    Production,
    Human,
    Done,
}

impl StageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StageType::Research => "research",
            StageType::Plan => "plan",
            StageType::Develop => "develop",
            StageType::Build => "build",
            StageType::Writing => "writing",
            StageType::Editing => "editing",
            StageType::Review => "review",
            StageType::Proofreading => "proofreading",
            StageType::Screenplay => "screenplay",
            StageType::Storyboard => "storyboard",
            StageType::Design => "design",
            StageType::Production => "production",
            StageType::Human => "human",
            StageType::Done => "done",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "research" => Some(StageType::Research),
            "plan" => Some(StageType::Plan),
            "develop" => Some(StageType::Develop),
            "build" => Some(StageType::Build),
            "writing" => Some(StageType::Writing),
            "editing" => Some(StageType::Editing),
            "review" => Some(StageType::Review),
            "proofreading" => Some(StageType::Proofreading),
            "screenplay" => Some(StageType::Screenplay),
            "storyboard" => Some(StageType::Storyboard),
            "design" => Some(StageType::Design),
            "production" => Some(StageType::Production),
            "human" => Some(StageType::Human),
            "done" => Some(StageType::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    NotStarted,
    InProgress,
    Blocked,
    NeedsReview,
    Completed,
    Skipped,
}

impl StageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            StageStatus::NotStarted => "not_started",
            StageStatus::InProgress => "in_progress",
            StageStatus::Blocked => "blocked",
            StageStatus::NeedsReview => "needs_review",
            StageStatus::Completed => "completed",
            StageStatus::Skipped => "skipped",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "not_started" => Some(StageStatus::NotStarted),
            "in_progress" => Some(StageStatus::InProgress),
            "blocked" => Some(StageStatus::Blocked),
            "needs_review" => Some(StageStatus::NeedsReview),
            "completed" => Some(StageStatus::Completed),
            "skipped" => Some(StageStatus::Skipped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStageType {
    Builtin,
    Custom,
}

impl ProjectStageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectStageType::Builtin => "builtin",
            ProjectStageType::Custom => "custom",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "builtin" => Some(ProjectStageType::Builtin),
            "custom" => Some(ProjectStageType::Custom),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStageInfo {
    pub id: String,
    pub project_id: Option<String>,
    #[serde(rename = "type")]
    pub stage_type: ProjectStageType,
    pub workflow_id: Option<String>,
    pub kind: Option<StageType>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub order: i64,
    pub enabled: bool,
    pub allow_empty_assistants: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub assistants: Vec<StageAssistantInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub id: String,
    pub thread_id: String,
    pub stage_id: String,
    pub project_id: String,
    pub assistant_ids: Vec<String>,
    #[serde(default)]
    pub assistants: Vec<StageAssistantInfo>,
    #[serde(rename = "type")]
    pub stage_type: ProjectStageType,
    pub workflow_id: Option<String>,
    pub kind: Option<StageType>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub order: i64,
    pub status: StageStatus,
    pub summary: Option<String>,
    pub outcome: Option<String>,
    pub enabled: bool,
    pub allow_empty_assistants: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
    #[serde(default)]
    pub issues: Vec<StageIssueInfo>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Open,
    Resolved,
    Dismissed,
}

impl IssueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueStatus::Open => "open",
            IssueStatus::Resolved => "resolved",
            IssueStatus::Dismissed => "dismissed",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "open" => Some(IssueStatus::Open),
            "resolved" => Some(IssueStatus::Resolved),
            "dismissed" => Some(IssueStatus::Dismissed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl IssueSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            IssueSeverity::Low => "low",
            IssueSeverity::Medium => "medium",
            IssueSeverity::High => "high",
            IssueSeverity::Critical => "critical",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "low" => Some(IssueSeverity::Low),
            "medium" => Some(IssueSeverity::Medium),
            "high" => Some(IssueSeverity::High),
            "critical" => Some(IssueSeverity::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageIssueInfo {
    pub id: String,
    pub thread_stage_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: IssueStatus,
    pub severity: IssueSeverity,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThreadKind {
    #[default]
    Workflow,
    Teamwork,
    Brainstorm,
    Debate,
}

impl ThreadKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThreadKind::Workflow => "workflow",
            ThreadKind::Teamwork => "teamwork",
            ThreadKind::Brainstorm => "brainstorm",
            ThreadKind::Debate => "debate",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "workflow" => Some(ThreadKind::Workflow),
            "teamwork" => Some(ThreadKind::Teamwork),
            "brainstorm" => Some(ThreadKind::Brainstorm),
            "debate" => Some(ThreadKind::Debate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAssistantInfo {
    pub assistant_id: String,
    pub name: String,
    pub color: Option<String>,
    pub agent: AssistantAgentInfo,
    #[serde(default)]
    pub system_prompt: Option<String>,
    pub order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    pub id: String,
    pub project_id: String,
    pub goal: String,
    pub description: Option<String>,
    pub stage_id: Option<String>,
    #[serde(default)]
    pub kind: ThreadKind,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub assistants: Vec<ThreadAssistantInfo>,
    #[serde(default)]
    pub stages: Vec<StageInfo>,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReplayInfo {
    pub thread_id: String,
    pub kind: ThreadKind,
    pub sessions: Vec<ThreadReplaySessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadChatSummaryInfo {
    pub thread_id: String,
    pub project_id: String,
    pub goal: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub time: i64,
    pub sessions: Vec<SessionInfo>,
    pub session_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReplaySessionInfo {
    pub agent: Agent,
    pub session_id: String,
    pub session: Option<SessionInfo>,
    #[serde(default)]
    pub sources: Vec<ThreadReplaySessionSourceInfo>,
    pub first_seen_at: Option<i64>,
    pub last_seen_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReplaySessionSourceInfo {
    pub kind: ThreadReplaySessionSourceKind,
    pub thread_id: Option<String>,
    pub stage_id: Option<String>,
    pub plan_round_id: Option<String>,
    pub plan_task_id: Option<String>,
    pub astra_run_id: Option<String>,
    pub role: Option<PlanTaskSessionRole>,
    pub label: Option<String>,
    pub stage_snapshot_json: Option<String>,
    pub assistant_snapshot_json: Option<String>,
    pub agent_snapshot_json: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ThreadReplaySessionSourceKind {
    Thread,
    Stage,
    PlanTask,
    AstraInternal,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanRoundMode {
    Parallel,
    Sequential,
}

impl PlanRoundMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanRoundMode::Parallel => "parallel",
            PlanRoundMode::Sequential => "sequential",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "parallel" => Some(PlanRoundMode::Parallel),
            "sequential" => Some(PlanRoundMode::Sequential),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanRoundSource {
    Astra,
    Manual,
    Agent,
}

impl PlanRoundSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanRoundSource::Astra => "astra",
            PlanRoundSource::Manual => "manual",
            PlanRoundSource::Agent => "agent",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "astra" => Some(PlanRoundSource::Astra),
            "manual" => Some(PlanRoundSource::Manual),
            "agent" => Some(PlanRoundSource::Agent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanRoundStatus {
    Planned,
    Running,
    Completed,
    Cancelled,
    Errored,
}

impl PlanRoundStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanRoundStatus::Planned => "planned",
            PlanRoundStatus::Running => "running",
            PlanRoundStatus::Completed => "completed",
            PlanRoundStatus::Cancelled => "cancelled",
            PlanRoundStatus::Errored => "errored",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "planned" => Some(PlanRoundStatus::Planned),
            "running" => Some(PlanRoundStatus::Running),
            "completed" => Some(PlanRoundStatus::Completed),
            "cancelled" => Some(PlanRoundStatus::Cancelled),
            "errored" => Some(PlanRoundStatus::Errored),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanTaskStatus {
    Planned,
    Running,
    Completed,
    Failed,
    Errored,
    Cancelled,
}

impl PlanTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanTaskStatus::Planned => "planned",
            PlanTaskStatus::Running => "running",
            PlanTaskStatus::Completed => "completed",
            PlanTaskStatus::Failed => "failed",
            PlanTaskStatus::Errored => "errored",
            PlanTaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "planned" => Some(PlanTaskStatus::Planned),
            "running" => Some(PlanTaskStatus::Running),
            "completed" => Some(PlanTaskStatus::Completed),
            "failed" => Some(PlanTaskStatus::Failed),
            "errored" => Some(PlanTaskStatus::Errored),
            "cancelled" => Some(PlanTaskStatus::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            PlanTaskStatus::Completed
                | PlanTaskStatus::Failed
                | PlanTaskStatus::Errored
                | PlanTaskStatus::Cancelled
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum PlanTaskRisk {
    Low,
    Medium,
    High,
}

impl PlanTaskRisk {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanTaskRisk::Low => "low",
            PlanTaskRisk::Medium => "medium",
            PlanTaskRisk::High => "high",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "low" => Some(PlanTaskRisk::Low),
            "medium" => Some(PlanTaskRisk::Medium),
            "high" => Some(PlanTaskRisk::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PlanTaskSessionRole {
    Primary,
    Delegated,
    Runtime,
    Planner,
    Synthesis,
    CrossCheck,
    Diagnostic,
}

impl PlanTaskSessionRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanTaskSessionRole::Primary => "primary",
            PlanTaskSessionRole::Delegated => "delegated",
            PlanTaskSessionRole::Runtime => "runtime",
            PlanTaskSessionRole::Planner => "planner",
            PlanTaskSessionRole::Synthesis => "synthesis",
            PlanTaskSessionRole::CrossCheck => "cross_check",
            PlanTaskSessionRole::Diagnostic => "diagnostic",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "primary" => Some(PlanTaskSessionRole::Primary),
            "delegated" => Some(PlanTaskSessionRole::Delegated),
            "runtime" => Some(PlanTaskSessionRole::Runtime),
            "planner" => Some(PlanTaskSessionRole::Planner),
            "synthesis" => Some(PlanTaskSessionRole::Synthesis),
            "cross_check" => Some(PlanTaskSessionRole::CrossCheck),
            "diagnostic" => Some(PlanTaskSessionRole::Diagnostic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskSessionInfo {
    pub task_id: String,
    pub agent: Agent,
    pub session_id: String,
    pub role: PlanTaskSessionRole,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanTaskInfo {
    pub id: String,
    pub round_id: String,
    pub thread_stage_id: Option<String>,
    pub assistant_id: Option<String>,
    pub target_agent: Agent,
    pub stage_snapshot_json: Option<String>,
    pub assistant_snapshot_json: Option<String>,
    pub agent_snapshot_json: String,
    pub title: String,
    pub prompt: String,
    pub expected_output: Option<String>,
    pub risk: PlanTaskRisk,
    pub sort_order: i64,
    pub status: PlanTaskStatus,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub sessions: Vec<PlanTaskSessionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanRoundInfo {
    pub id: String,
    pub thread_id: String,
    pub astra_run_id: Option<String>,
    pub round_index: i64,
    pub summary: Option<String>,
    pub mode: PlanRoundMode,
    pub source: PlanRoundSource,
    pub status: PlanRoundStatus,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub tasks: Vec<PlanTaskInfo>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KanbanStatus {
    Todo,
    InProgress,
    Canceled,
    AgentReview,
    HumanReview,
    Done,
}

impl KanbanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KanbanStatus::Todo => "todo",
            KanbanStatus::InProgress => "in_progress",
            KanbanStatus::Canceled => "canceled",
            KanbanStatus::AgentReview => "agent_review",
            KanbanStatus::HumanReview => "human_review",
            KanbanStatus::Done => "done",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(KanbanStatus::Todo),
            "in_progress" => Some(KanbanStatus::InProgress),
            "canceled" => Some(KanbanStatus::Canceled),
            "agent_review" => Some(KanbanStatus::AgentReview),
            "human_review" => Some(KanbanStatus::HumanReview),
            "done" => Some(KanbanStatus::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KanbanItem {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: KanbanStatus,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

fn default_available() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl SessionContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".to_string(),
            text: Some(text.into()),
            uri: None,
            data: None,
            mime_type: None,
            name: None,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: None,
            meta: None,
        }
    }

    pub fn image(uri: impl Into<String>, mime_type: Option<String>) -> Self {
        Self {
            kind: "image".to_string(),
            text: None,
            uri: Some(uri.into()),
            data: None,
            mime_type,
            name: None,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: None,
            meta: None,
        }
    }

    pub fn resource(uri: Option<String>, name: Option<String>, mime_type: Option<String>) -> Self {
        Self {
            kind: "resource".to_string(),
            text: None,
            uri,
            data: None,
            mime_type,
            name,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: None,
            meta: None,
        }
    }
}

pub fn sessio_attachment_marker_name(name: &str) -> String {
    format!("{SESSIO_ATTACHMENT_MARKER}{name}")
}

pub fn strip_sessio_attachment_marker_name(name: &str) -> Option<String> {
    name.strip_prefix(SESSIO_ATTACHMENT_MARKER)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryTurn {
    pub turn_id: String,
    pub status: String,
    pub blocks: Vec<SessionHistoryBlock>,
    pub tools: Vec<SessionHistoryToolCall>,
    pub permissions: Vec<SessionHistoryPermissionRequest>,
    pub protocol_messages: Vec<Value>,
    pub stop_reason: Option<String>,
    pub error: Option<Value>,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryBlock {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<SessionContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryToolCall {
    pub tool_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default)]
    pub locations: Vec<Value>,
    pub raw_input: Value,
    pub raw_output: Value,
    pub meta: Value,
    pub raw: Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryPermissionRequest {
    pub request_id: String,
    pub tool_call: Value,
    pub tool_name: String,
    pub input: Value,
    pub options: Vec<SessionHistoryPermissionOption>,
    pub selected_option_id: Option<String>,
    pub cancelled: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryPermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
    pub meta: Value,
}

pub fn text_content_blocks(text: &str) -> Vec<SessionContentBlock> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    let code_ranges = markdown_code_ranges(text);
    let mut cursor = 0usize;
    while cursor < text.len() {
        let next_image = find_markdown_image_marker(text, cursor, &code_ranges);
        let next_file = find_file_marker(text, cursor, &code_ranges);
        let next = match (next_image, next_file) {
            (Some(image), Some(file)) => Some(image.min(file)),
            (Some(image), None) => Some(image),
            (None, Some(file)) => Some(file),
            (None, None) => None,
        };
        let Some(start) = next else {
            push_text_block(&mut blocks, &text[cursor..]);
            break;
        };
        push_text_block(&mut blocks, &text[cursor..start]);
        if text[start..].starts_with("![") {
            if let Some((block, end)) = parse_markdown_image(text, start) {
                blocks.push(block);
                cursor = end;
                continue;
            }
        } else if let Some((block, end)) = parse_file_marker(text, start) {
            blocks.push(block);
            cursor = end;
            continue;
        }
        push_text_block(&mut blocks, &text[start..start + 1]);
        cursor = start + 1;
    }
    if blocks.is_empty() {
        vec![SessionContentBlock::text(text.to_string())]
    } else {
        merge_adjacent_text_blocks(blocks)
    }
}

fn push_text_block(blocks: &mut Vec<SessionContentBlock>, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    blocks.push(SessionContentBlock::text(text.to_string()));
}

fn merge_adjacent_text_blocks(blocks: Vec<SessionContentBlock>) -> Vec<SessionContentBlock> {
    let mut merged: Vec<SessionContentBlock> = Vec::new();
    for block in blocks {
        if block.kind == "text" {
            let text = block.text.clone().unwrap_or_default();
            if let Some(previous) = merged.last_mut().filter(|previous| previous.kind == "text") {
                let previous_text = previous.text.get_or_insert_with(String::new);
                previous_text.push_str(&text);
                continue;
            }
        }
        merged.push(block);
    }
    merged
}

fn find_markdown_image_marker(
    text: &str,
    start: usize,
    code_ranges: &[(usize, usize)],
) -> Option<usize> {
    let mut cursor = start;
    while cursor < text.len() {
        let found = text.get(cursor..)?.find("![").map(|idx| cursor + idx)?;
        if !is_in_range(found, code_ranges) {
            return Some(found);
        }
        cursor = found + 1;
    }
    None
}

fn find_file_marker(text: &str, start: usize, code_ranges: &[(usize, usize)]) -> Option<usize> {
    let haystack = text.get(start..)?.to_ascii_lowercase();
    let mut cursor = 0usize;
    while cursor < haystack.len() {
        let found = haystack[cursor..]
            .find("[file:")
            .map(|idx| start + cursor + idx)?;
        if !is_in_range(found, code_ranges) {
            return Some(found);
        }
        cursor = found - start + 1;
    }
    None
}

fn parse_markdown_image(text: &str, start: usize) -> Option<(SessionContentBlock, usize)> {
    let after_open = start + 2;
    let label_end = text[after_open..].find("](").map(|idx| after_open + idx)?;
    let target_start = label_end + 2;
    let target_end = text[target_start..]
        .find(')')
        .map(|idx| target_start + idx)?;
    let alt = text[after_open..label_end].trim();
    let uri = text[target_start..target_end]
        .trim()
        .trim_matches(['<', '>']);
    if uri.is_empty() {
        return None;
    }
    let display_name = strip_sessio_attachment_marker_name(alt)?;
    let mime_type = if display_name.contains('/') {
        Some(display_name.clone())
    } else {
        None
    };
    let mut block = SessionContentBlock::image(uri.to_string(), mime_type);
    block.name = Some(display_name);
    Some((block, target_end + 1))
}

fn parse_file_marker(text: &str, start: usize) -> Option<(SessionContentBlock, usize)> {
    let marker = text.get(start..)?;
    if !marker
        .get(..6)
        .map(|value| value.eq_ignore_ascii_case("[file:"))
        .unwrap_or(false)
    {
        return None;
    }
    let close = marker.find(']')?;
    let body = marker[6..close].trim();
    if body.is_empty() {
        return None;
    }
    let mut parts = body.splitn(2, '|');
    let name = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let display_name = name.and_then(strip_sessio_attachment_marker_name)?;
    let uri = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Some((
        SessionContentBlock::resource(uri.map(ToOwned::to_owned), Some(display_name), None),
        start + close + 1,
    ))
}

fn markdown_code_ranges(text: &str) -> Vec<(usize, usize)> {
    let fence_ranges = markdown_fenced_code_ranges(text);
    let mut ranges = fence_ranges.clone();
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        if is_in_range(cursor, &fence_ranges) {
            cursor = range_end(cursor, &fence_ranges).unwrap_or(cursor + 1);
            continue;
        }
        if bytes[cursor] != b'`' {
            cursor += 1;
            continue;
        }
        let tick_count = backtick_run_len(bytes, cursor);
        let search_from = cursor + tick_count;
        if let Some(end) = find_closing_backtick_run(bytes, search_from, tick_count, &fence_ranges)
        {
            ranges.push((cursor, end + tick_count));
            cursor = end + tick_count;
        } else {
            cursor = search_from;
        }
    }
    ranges.sort_by_key(|(start, _)| *start);
    ranges
}

fn markdown_fenced_code_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut fence: Option<(usize, u8, usize)> = None;
    let mut line_start = 0usize;
    while line_start < text.len() {
        let line_end = text[line_start..]
            .find('\n')
            .map(|idx| line_start + idx)
            .unwrap_or(text.len());
        let line = &text[line_start..line_end];
        if let Some((marker, marker_len, rest)) = fenced_code_marker(line) {
            if let Some((start, open_marker, open_marker_len)) = fence {
                if marker == open_marker && marker_len >= open_marker_len && rest.trim().is_empty()
                {
                    fence = None;
                    let end = if line_end < text.len() {
                        line_end + 1
                    } else {
                        line_end
                    };
                    ranges.push((start, end));
                }
            } else {
                fence = Some((line_start, marker, marker_len));
            }
        }
        if line_end == text.len() {
            break;
        }
        line_start = line_end + 1;
    }
    if let Some((start, _, _)) = fence {
        ranges.push((start, text.len()));
    }
    ranges
}

fn fenced_code_marker(line: &str) -> Option<(u8, usize, &str)> {
    let trimmed = line.trim_start();
    let bytes = trimmed.as_bytes();
    let marker = *bytes.first()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let marker_len = bytes.iter().take_while(|byte| **byte == marker).count();
    if marker_len < 3 {
        return None;
    }
    let rest = &trimmed[marker_len..];
    Some((marker, marker_len, rest))
}

fn find_closing_backtick_run(
    bytes: &[u8],
    start: usize,
    tick_count: usize,
    excluded_ranges: &[(usize, usize)],
) -> Option<usize> {
    let mut cursor = start;
    while cursor < bytes.len() {
        if is_in_range(cursor, excluded_ranges) {
            cursor = range_end(cursor, excluded_ranges).unwrap_or(cursor + 1);
            continue;
        }
        if bytes[cursor] == b'`' && backtick_run_len(bytes, cursor) == tick_count {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

fn backtick_run_len(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && bytes[end] == b'`' {
        end += 1;
    }
    end - start
}

fn is_in_range(index: usize, ranges: &[(usize, usize)]) -> bool {
    ranges
        .iter()
        .any(|(start, end)| index >= *start && index < *end)
}

fn range_end(index: usize, ranges: &[(usize, usize)]) -> Option<usize> {
    ranges
        .iter()
        .find(|(start, end)| index >= *start && index < *end)
        .map(|(_, end)| *end)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAgentMetadata {
    pub agent: Agent,
    pub enabled: bool,
    pub configured: bool,
    pub order: i64,
    pub transport: RuntimeTransportKind,
    pub model: Option<String>,
    pub models: Vec<RuntimeAgentOptionMetadata>,
    pub effort: Option<String>,
    pub efforts: Vec<RuntimeAgentOptionMetadata>,
    pub permission_mode: Option<String>,
    pub permission_modes: Vec<RuntimeAgentOptionMetadata>,
    pub session_command: Option<String>,
    pub version_command: Option<String>,
    pub detected_version: Option<String>,
    pub capabilities: Option<RuntimeCapabilitySet>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAgentOptionMetadata {
    pub value: String,
    pub label: String,
    pub display_name: String,
    pub enabled: bool,
    pub order: i64,
}

pub fn normalize_preview(s: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 50;
    let trimmed = s.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch == '\n' || ch == '\r' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    let mut chars = out.chars();
    let truncated: String = chars.by_ref().take(MAX_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub fn is_system_noise(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<INSTRUCTIONS>")
        || t.starts_with("# AGENTS.md")
        || t.starts_with("<system-reminder>")
        || t.starts_with("<command-name>")
        || t.starts_with("<command-message>")
        || t.starts_with("<command-args>")
        || t.starts_with("<local-command-stdout>")
        || t.starts_with("<local-command-caveat>")
        || t.starts_with("<bash-input>")
        || t.starts_with("<bash-stdout>")
        || t.starts_with("<bash-stderr>")
        || t.starts_with("<user-memory-input>")
        || t.starts_with("<turn_aborted>")
        || t.starts_with("Caveat:")
        || t.starts_with("Warning: apply_patch was requested via exec_command")
}

// Strip IDE-injected context blocks that some agents prepend to the real user
// message. Returns the underlying request text, or empty if the message is
// entirely context.
pub fn strip_injected_context(s: &str) -> String {
    let mut text: &str = s;

    // Claude-style: leading <ide_*>...</ide_*> wrapper blocks (e.g.
    // <ide_opened_file>, <ide_selection>).
    loop {
        let trimmed = text.trim_start();
        let after_lt = match trimmed.strip_prefix("<ide_") {
            Some(rest) => rest,
            None => break,
        };
        let close_idx = match after_lt.find('>') {
            Some(i) => i,
            None => break,
        };
        let tag = &after_lt[..close_idx];
        let close = format!("</ide_{}>", tag);
        let after_open = &after_lt[close_idx + 1..];
        match after_open.find(close.as_str()) {
            Some(i) => {
                text = &after_open[i + close.len()..];
            }
            None => break,
        }
    }

    // Codex-style: strip any preamble before the "## My request for Codex:"
    // header (e.g. "# Context from my IDE setup:", "# Files mentioned by the
    // user:"); the real user input follows the marker.
    const MARKER: &str = "## My request for Codex:";
    if let Some(i) = text.find(MARKER) {
        text = &text[i + MARKER.len()..];
    }

    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{normalize_preview, sessio_attachment_marker_name, text_content_blocks};

    #[test]
    fn normalize_preview_limits_to_50_chars_plus_ellipsis() {
        let exact = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十";
        assert_eq!(exact.chars().count(), 50);
        assert_eq!(normalize_preview(exact), exact);

        let long = format!("{exact}超出");
        let preview = normalize_preview(&long);
        assert_eq!(preview, format!("{exact}..."));
        assert_eq!(preview.chars().count(), 53);
    }

    #[test]
    fn normalize_preview_flattens_newlines_before_truncating() {
        assert_eq!(
            normalize_preview(" hello\nworld\ragain "),
            "hello world again"
        );
    }

    #[test]
    fn text_content_blocks_parse_markdown_images_and_file_markers() {
        let file_name = sessio_attachment_marker_name("spec.md");
        let image_name = sessio_attachment_marker_name("screen.png");
        let blocks = text_content_blocks(&format!(
            "review\n[file: {file_name}|file:///tmp/spec.md]\n![{image_name}](file:///tmp/screen.png)"
        ));
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some("review\n"));
        assert_eq!(blocks[1].kind, "resource");
        assert_eq!(blocks[1].name.as_deref(), Some("spec.md"));
        assert_eq!(blocks[1].uri.as_deref(), Some("file:///tmp/spec.md"));
        assert_eq!(blocks[2].kind, "image");
        assert_eq!(blocks[2].name.as_deref(), Some("screen.png"));
        assert_eq!(blocks[2].uri.as_deref(), Some("file:///tmp/screen.png"));
    }

    #[test]
    fn text_content_blocks_keeps_unmarked_attachment_examples_as_text() {
        let text = "A hook can inject examples like [file: name|uri], [file: ...], and ![alt](file://...).";
        let blocks = text_content_blocks(text);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some(text));
    }

    #[test]
    fn text_content_blocks_keeps_marked_attachments_inside_code_as_text() {
        let file_name = sessio_attachment_marker_name("spec.md");
        let image_name = sessio_attachment_marker_name("screen.png");
        let text = format!(
            "inline `[file: {file_name}|file:///tmp/spec.md]` and `![{image_name}](file:///tmp/screen.png)`\n```md\n[file: {file_name}|file:///tmp/spec.md]\n![{image_name}](file:///tmp/screen.png)\n```\noutside [file: {file_name}|file:///tmp/spec.md]"
        );
        let blocks = text_content_blocks(&text);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "text");
        let display_text = blocks[0].text.as_deref().unwrap_or_default();
        assert!(display_text.contains("inline `[file:"));
        assert!(display_text.contains("```md\n[file:"));
        assert_eq!(blocks[1].kind, "resource");
        assert_eq!(blocks[1].name.as_deref(), Some("spec.md"));
        assert_eq!(blocks[1].uri.as_deref(), Some("file:///tmp/spec.md"));
    }

    #[test]
    fn text_content_blocks_does_not_extract_when_marked_attachments_are_only_code() {
        let text = "`[file: __sessio_attachment__:test.md|file:///x]`\n`![__sessio_attachment__:image/png](file:///x.png)`\n\n```md\n[file: __sessio_attachment__:test.md|file:///x]\n![__sessio_attachment__:image/png](file:///x.png)\n```";
        let blocks = text_content_blocks(text);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some(text));
    }

    #[test]
    fn text_content_blocks_keeps_marked_attachments_inside_unclosed_outer_fence_as_text() {
        let text = "```\n`[file: __sessio_attachment__:test.md|file:///x]`\n`![__sessio_attachment__:image/png](file:///x.png)`\n\n```md\n[file: __sessio_attachment__:test.md|file:///x]\n![__sessio_attachment__:image/png](file:///x.png)\n```  这种下面的图片还是会被提取\n";
        let blocks = text_content_blocks(text);

        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some(text));
    }
}
