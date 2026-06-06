use serde_json::Value;

use super::backend::{BackendFailure, BackendResponse, DecisionBackend, PlannerBackend};
use super::decision::deterministic_decision;
use super::planner::deterministic_plan;
use super::{AstraDecision, AstraPlan, AstraRun, AstraTaskProposal, AstraTaskResult};
use crate::models::ThreadInfo;

/// Deterministic planner backend (rule-based, no external agent)
pub struct DeterministicPlannerBackend;

impl PlannerBackend for DeterministicPlannerBackend {
    fn plan(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        _config: &Value,
    ) -> Result<BackendResponse<AstraPlan>, BackendFailure> {
        let plan = deterministic_plan(run, thread, user_prompt, round_index);
        Ok(BackendResponse {
            data: plan,
            session_id: format!("deterministic-planner-{}-{}", run.run_id, round_index),
            backend_type: "deterministic".to_string(),
        })
    }

    fn supports_fallback(&self) -> bool {
        false
    }
}

/// Deterministic decision backend (rule-based, no external agent)
pub struct DeterministicDecisionBackend;

impl DecisionBackend for DeterministicDecisionBackend {
    fn decide(
        &self,
        _run: &AstraRun,
        thread: &ThreadInfo,
        result: &AstraTaskResult,
        task: &AstraTaskProposal,
        _config: &Value,
    ) -> Result<BackendResponse<AstraDecision>, BackendFailure> {
        let decision = deterministic_decision(thread, result, task);
        Ok(BackendResponse {
            data: decision,
            session_id: format!("deterministic-decision-{}", result.task_id),
            backend_type: "deterministic".to_string(),
        })
    }

    fn supports_fallback(&self) -> bool {
        false
    }
}
