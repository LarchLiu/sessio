use anyhow::Result;
use serde_json::Value;

use super::{AstraOrchestration, AstraRun, AstraTaskCompletion};
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
        }
    }

    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }
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
        config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure>;
}
