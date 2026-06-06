use anyhow::Result;
use serde_json::Value;

use super::{AstraDecision, AstraPlan, AstraRun, AstraTaskProposal, AstraTaskResult};
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
    pub fn new(backend_type: impl Into<String>, code: &'static str, message: impl Into<String>) -> Self {
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

/// Trait for Astra planner backends that can generate execution plans
pub trait PlannerBackend: Send + Sync {
    /// Generate a plan for the given Astra run
    fn plan(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        config: &Value,
    ) -> Result<BackendResponse<AstraPlan>, BackendFailure>;

    /// Returns the backend type identifier (e.g., "pi_acp", "runtime_agent", "deterministic")
    fn backend_type(&self) -> &'static str;

    /// Returns true if this backend can fail and should have a fallback
    fn supports_fallback(&self) -> bool {
        true
    }
}

/// Trait for Astra decision engine backends that evaluate task results
pub trait DecisionBackend: Send + Sync {
    /// Make a decision based on task result
    fn decide(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        result: &AstraTaskResult,
        task: &AstraTaskProposal,
        config: &Value,
    ) -> Result<BackendResponse<AstraDecision>, BackendFailure>;

    /// Returns the backend type identifier
    fn backend_type(&self) -> &'static str;

    /// Returns true if this backend can fail and should have a fallback
    fn supports_fallback(&self) -> bool {
        true
    }
}
