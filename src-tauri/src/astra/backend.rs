use anyhow::Result;
use serde_json::Value;

use super::{AstraOrchestration, AstraPlannerContext, AstraRun, AstraTaskCompletion};
use crate::models::ThreadInfo;

#[derive(Debug, Clone)]
pub struct BackendResponse<T> {
    pub data: T,
    pub session_id: String,
    pub backend_type: String,
}

#[derive(Debug, Clone)]
pub struct BackendFailure {
    pub code: &'static str,
    pub message: String,
    pub session_id: Option<String>,
    pub backend_type: String,
    pub raw_response_snippet: Option<String>,
}

impl BackendFailure {
    pub fn new(
        backend_type: impl Into<String>,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            session_id: None,
            backend_type: backend_type.into(),
            raw_response_snippet: None,
        }
    }

    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }

    pub fn with_raw_response(mut self, response: &str) -> Self {
        self.raw_response_snippet = raw_response_snippet(response);
        self
    }
}

pub fn raw_response_snippet(response: &str) -> Option<String> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut snippet = trimmed.chars().take(800).collect::<String>();
    if trimmed.chars().count() > snippet.chars().count() {
        snippet.push_str("...");
    }
    Some(snippet)
}

/// Trait for Astra Orchestrator backends that can do both initial rolling
/// planning and post-result planning in a single model call.
pub trait OrchestratorBackend: Send + Sync {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        planner_context: &AstraPlannerContext,
        config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure>;
}
