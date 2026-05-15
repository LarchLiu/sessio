pub mod sqlite;

use anyhow::Result;
use std::collections::HashSet;

use crate::models::{Agent, SessionInfo};

pub trait SessionStore: Send + Sync {
    fn init(&self) -> Result<()>;
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()>;
    fn replace_by_scope(
        &self,
        scope: &str,
        agent: Agent,
        sessions: &[SessionInfo],
    ) -> Result<()>;
    fn delete_by_file_path(&self, file_path: &str) -> Result<()>;
    fn purge_missing_scopes(&self, agent: Agent, present: &HashSet<String>) -> Result<()>;
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
