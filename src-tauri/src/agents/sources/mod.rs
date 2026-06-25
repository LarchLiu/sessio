pub mod claude;
pub mod codex;
pub mod gemini;
pub mod opencode;
pub mod pi;
pub mod registry;
pub mod shared;
pub mod types;

use anyhow::Result;
use std::collections::HashSet;
use std::time::SystemTime;

use crate::models::{Agent, AgentInfo, SessionInfo};

pub fn list_all() -> Vec<SessionInfo> {
    let mut out = Vec::new();
    for f in [
        codex::parser::list_sessions as fn() -> Result<Vec<SessionInfo>>,
        claude::parser::list_sessions,
        gemini::parser::list_sessions,
        pi::parser::list_sessions,
        opencode::parser::list_sessions,
    ] {
        match f() {
            Ok(mut v) => out.append(&mut v),
            Err(e) => log::warn!("source parser failed: {e}"),
        }
    }
    out
}

pub fn system_time_to_millis(t: SystemTime) -> Option<i64> {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
}

pub fn builtin_agent_sources() -> registry::AgentSourceRegistry {
    builtin_agent_sources_for(Agent::ALL.iter().copied())
}

pub fn builtin_agent_sources_for<I>(agents: I) -> registry::AgentSourceRegistry
where
    I: IntoIterator<Item = Agent>,
{
    let enabled: HashSet<Agent> = agents.into_iter().collect();
    let mut registry = registry::AgentSourceRegistry::new();
    if enabled.contains(&Agent::Pi) {
        registry.register(pi::PiSource);
    }
    if enabled.contains(&Agent::Codex) {
        registry.register(codex::CodexSource);
    }
    if enabled.contains(&Agent::Claude) {
        registry.register(claude::ClaudeSource);
    }
    if enabled.contains(&Agent::Gemini) {
        registry.register(gemini::GeminiSource);
    }
    if enabled.contains(&Agent::Opencode) {
        registry.register(opencode::OpencodeSource);
    }
    registry
}

pub fn enabled_builtin_agents(agent_rows: &[AgentInfo]) -> HashSet<Agent> {
    agent_rows
        .iter()
        .filter(|agent| agent.enabled)
        .filter_map(|agent| Agent::from_db_str(&agent.id))
        .collect()
}
