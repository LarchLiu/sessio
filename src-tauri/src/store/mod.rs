pub mod cached;
pub mod sqlite;

use anyhow::Result;
use std::collections::HashSet;

use crate::models::{Agent, SessionInfo, SubagentInfo};

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
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub last_indexed_at: i64,
    pub available: bool,
    pub archived: bool,
    pub subagents: Vec<IndexedSubagentRecord>,
}

pub trait SessionStore: Send + Sync {
    fn init(&self) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>>;
    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()>;
    fn replace_by_scope(
        &self,
        scope: &str,
        agent: Agent,
        sessions: &[SessionInfo],
    ) -> Result<()>;
    fn upsert_subagent(
        &self,
        parent_agent: Agent,
        parent_scope: &str,
        parent_session_id: &str,
        subagent: &SubagentInfo,
    ) -> Result<()>;
    fn mark_file_path_unavailable(&self, file_path: &str) -> Result<()>;
    fn mark_subagent_file_unavailable(&self, file_path: &str) -> Result<()>;
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
