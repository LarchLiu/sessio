pub mod cached;
pub mod sqlite;

use anyhow::Result;
use std::collections::HashSet;

use crate::agents::runtime::types::RuntimeTransportKind;
use crate::models::{
    Agent, KanbanItem, KanbanStatus, ProjectInfo, ProjectType, SessionInfo, SubagentInfo,
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

pub trait SessionStore: Send + Sync {
    fn init(&self) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_all_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>>;
    fn list_projects(&self) -> Result<Vec<ProjectInfo>>;
    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        project_type: ProjectType,
    ) -> Result<ProjectInfo>;
    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        project_type: ProjectType,
    ) -> Result<ProjectInfo>;
    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        project_type: Option<ProjectType>,
    ) -> Result<ProjectInfo>;
    fn archive_project(&self, project_id: &str) -> Result<()>;
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

impl Agent {
    pub fn from_db_str(s: &str) -> Option<Agent> {
        match s {
            "codex" => Some(Agent::Codex),
            "claude" => Some(Agent::Claude),
            "gemini" => Some(Agent::Gemini),
            _ => None,
        }
    }
}
