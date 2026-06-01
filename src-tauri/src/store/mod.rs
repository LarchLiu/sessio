pub mod cached;
pub mod sqlite;

use anyhow::Result;
use std::collections::HashSet;

use crate::agents::runtime::types::RuntimeTransportKind;
use crate::models::{
    Agent, AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType, KanbanItem, KanbanStatus,
    ProjectInfo, ProjectStageInfo, RuntimeAgentOptionMetadata, SessionHistoryTurn, SessionInfo,
    StageInfo, SubagentInfo, ThreadInfo, WorkflowInfo,
};

#[derive(Debug, Clone)]
pub struct IndexedSubagentRecord {
    pub parent_agent: Agent,
    pub parent_session_id: String,
    pub parent_scope: String,
    pub subagent_id: String,
    pub file_path: String,
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct IndexedSessionRecord {
    pub agent: Agent,
    pub session_id: String,
    pub scope: String,
    pub file_path: String,
    pub forked_from_agent: Option<Agent>,
    pub forked_from_id: Option<String>,
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub last_indexed_at: i64,
    pub available: bool,
    pub archived: bool,
    pub subagents: Vec<IndexedSubagentRecord>,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentCapabilityRecord {
    pub agent: Agent,
    pub transport: RuntimeTransportKind,
    pub version: Option<String>,
    pub protocol_version: Option<String>,
    pub raw_initialize_response_json: String,
    pub raw_capabilities_json: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentSelection {
    pub agent: Agent,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionHistoryRecord {
    pub agent: Agent,
    pub session_id: String,
    pub file_path: String,
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub history_cache_version: i64,
    pub message_count: usize,
    pub indexed_through: Option<i64>,
    pub updated_at: i64,
    pub turns: Vec<SessionHistoryTurn>,
}

#[derive(Debug, Clone)]
pub struct SessionHistorySnapshotRecord {
    pub child_agent: Agent,
    pub child_session_id: String,
    pub ancestor_agent: Agent,
    pub ancestor_session_id: String,
    pub ancestor_index: i64,
    pub history_cache_version: i64,
    pub created_at: i64,
    pub turns: Vec<SessionHistoryTurn>,
}

pub trait SessionStore: Send + Sync {
    fn init(&self) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_all_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>>;
    fn list_workflows(&self) -> Result<Vec<WorkflowInfo>>;
    fn create_workflow(&self, name: &str, description: Option<&str>) -> Result<WorkflowInfo>;
    fn update_workflow(
        &self,
        workflow_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<WorkflowInfo>;
    fn delete_workflow(&self, workflow_id: &str) -> Result<()>;
    fn list_projects(&self) -> Result<Vec<ProjectInfo>>;
    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        workflow_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo>;
    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        workflow_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo>;
    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        workflow_id: Option<String>,
    ) -> Result<ProjectInfo>;
    fn archive_project(&self, project_id: &str) -> Result<()>;
    fn list_agents(&self) -> Result<Vec<AgentInfo>>;
    fn update_builtin_agent_preferences(
        &self,
        agent: Agent,
        display_name: Option<&str>,
        enabled: Option<bool>,
        order: Option<i64>,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        models: Option<&[RuntimeAgentOptionMetadata]>,
        efforts: Option<&[RuntimeAgentOptionMetadata]>,
        permission_modes: Option<&[RuntimeAgentOptionMetadata]>,
    ) -> Result<AgentInfo>;
    fn get_last_runtime_agent_selection(&self) -> Result<Option<RuntimeAgentSelection>>;
    fn set_last_runtime_agent_selection(
        &self,
        agent: Agent,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<RuntimeAgentSelection>;
    fn list_assistants(&self, project_id: Option<&str>) -> Result<Vec<AssistantInfo>>;
    fn create_assistant(
        &self,
        name: &str,
        agent: AssistantAgentInfo,
        system_prompt: Option<&str>,
        color: Option<&str>,
        assistant_type: AssistantType,
        workflow_id: Option<String>,
        project_id: Option<&str>,
    ) -> Result<AssistantInfo>;
    fn update_assistant(
        &self,
        assistant_id: &str,
        name: Option<&str>,
        agent: Option<AssistantAgentInfo>,
        system_prompt: Option<Option<&str>>,
        color: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<AssistantInfo>;
    fn delete_assistant(&self, assistant_id: &str) -> Result<()>;
    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>>;
    fn create_thread(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
    ) -> Result<ThreadInfo>;
    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<ThreadInfo>;
    fn delete_thread(&self, thread_id: &str) -> Result<()>;
    fn list_project_stages(&self, project_id: &str) -> Result<Vec<ProjectStageInfo>>;
    fn list_workflow_stages(&self, workflow_id: &str) -> Result<Vec<ProjectStageInfo>>;
    fn create_project_stage(
        &self,
        project_id: &str,
        workflow_id: Option<String>,
        name: &str,
        description: Option<&str>,
        icon: Option<&str>,
    ) -> Result<ProjectStageInfo>;
    fn update_project_stage(
        &self,
        stage_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        icon: Option<Option<&str>>,
        order: Option<i64>,
        enabled: Option<bool>,
        allow_empty_assistants: Option<bool>,
    ) -> Result<ProjectStageInfo>;
    fn update_project_stage_assistants(
        &self,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<ProjectStageInfo>;
    fn delete_project_stage(&self, stage_id: &str) -> Result<()>;
    fn add_thread_stage(
        &self,
        thread_id: &str,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<StageInfo>;
    fn update_thread_stage(
        &self,
        thread_stage_id: &str,
        assistant_ids: Option<&[String]>,
        order: Option<i64>,
        enabled: Option<bool>,
    ) -> Result<StageInfo>;
    fn update_thread_stage_assistant_agent(
        &self,
        thread_stage_id: &str,
        assistant_id: &str,
        agent: AssistantAgentInfo,
    ) -> Result<StageInfo>;
    fn delete_thread_stage(&self, thread_stage_id: &str) -> Result<()>;
    fn set_thread_stage(&self, thread_id: &str, thread_stage_id: &str) -> Result<ThreadInfo>;
    fn link_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo>;
    fn unlink_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo>;
    fn link_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo>;
    fn unlink_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo>;
    fn list_kanban_items(&self, project_id: &str) -> Result<Vec<KanbanItem>>;
    fn create_kanban_item(
        &self,
        project_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<KanbanItem>;
    fn update_kanban_item(
        &self,
        item_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<KanbanStatus>,
    ) -> Result<KanbanItem>;
    fn delete_kanban_item(&self, item_id: &str) -> Result<()>;
    fn link_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem>;
    fn unlink_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem>;
    fn get_runtime_agent_capability(
        &self,
        agent: Agent,
    ) -> Result<Option<RuntimeAgentCapabilityRecord>>;
    fn upsert_runtime_agent_capability(&self, record: &RuntimeAgentCapabilityRecord) -> Result<()>;
    fn get_session_history(
        &self,
        agent: Agent,
        session_id: &str,
        file_path: &str,
    ) -> Result<Option<SessionHistoryRecord>>;
    fn replace_session_history(&self, record: &SessionHistoryRecord) -> Result<()>;
    fn get_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Vec<SessionHistorySnapshotRecord>>;
    fn replace_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
        snapshots: &[SessionHistorySnapshotRecord],
    ) -> Result<()>;
    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()>;
    fn replace_by_scope(&self, scope: &str, agent: Agent, sessions: &[SessionInfo]) -> Result<()>;
    fn upsert_subagent(
        &self,
        parent_agent: Agent,
        parent_scope: &str,
        parent_session_id: &str,
        subagent: &SubagentInfo,
    ) -> Result<()>;
    fn update_message_count(
        &self,
        agent: Agent,
        session_id: Option<&str>,
        file_path: &str,
        message_count: usize,
    ) -> Result<()>;
    fn mark_file_path_unavailable(&self, file_path: &str) -> Result<()>;
    fn mark_subagent_file_unavailable(&self, file_path: &str) -> Result<()>;
    fn mark_file_path_unindexable(&self, agent: Agent, file_path: &str) -> Result<()>;
    fn mark_missing_scopes_unavailable(
        &self,
        agent: Agent,
        present: &HashSet<String>,
    ) -> Result<()>;
}
