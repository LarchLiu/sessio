use serde_json::Value;

use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::decision::deterministic_decision;
use super::planner::deterministic_plan;
use super::{AstraDecision, AstraOrchestration, AstraRun, AstraTaskCompletion, AstraTaskDecision};
use crate::models::{StageStatus, ThreadInfo};

/// Deterministic Orchestrator backend (rule-based, no external agent)
pub struct DeterministicOrchestratorBackend;

impl OrchestratorBackend for DeterministicOrchestratorBackend {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        _config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure> {
        let decisions = completions
            .iter()
            .map(|completion| AstraTaskDecision {
                task_id: completion.task.id.clone(),
                decision: deterministic_decision(thread, &completion.result, &completion.task),
            })
            .collect::<Vec<_>>();
        let mut planning_thread = thread.clone();
        for task_decision in &decisions {
            apply_decision_to_thread(&mut planning_thread, &task_decision.decision);
        }
        let tasks = if decisions
            .iter()
            .any(|task_decision| decision_is_terminal(&task_decision.decision))
        {
            Vec::new()
        } else {
            deterministic_plan(run, &planning_thread, user_prompt, round_index).tasks
        };
        Ok(BackendResponse {
            data: AstraOrchestration {
                summary: format!(
                    "Deterministic Astra Orchestrator handled {} completion(s) and selected {} task(s).",
                    completions.len(),
                    tasks.len()
                ),
                decisions,
                tasks,
            },
            session_id: format!("deterministic-orchestrator-{}-{}", run.run_id, round_index),
            backend_type: "deterministic".to_string(),
        })
    }
}

fn apply_decision_to_thread(thread: &mut ThreadInfo, decision: &AstraDecision) {
    match decision {
        AstraDecision::UpdateStage { args } => {
            let Some(stage_id) = args
                .get("threadStageId")
                .or_else(|| args.get("stageId"))
                .and_then(Value::as_str)
            else {
                return;
            };
            let Some(status) = args
                .get("status")
                .and_then(Value::as_str)
                .and_then(StageStatus::from_db_str)
            else {
                return;
            };
            if let Some(stage) = thread
                .stages
                .iter_mut()
                .find(|stage| stage.id == stage_id || stage.stage_id == stage_id)
            {
                stage.status = status;
            }
        }
        AstraDecision::Composite { decisions } => {
            for decision in decisions {
                apply_decision_to_thread(thread, decision);
            }
        }
        _ => {}
    }
}

fn decision_is_terminal(decision: &AstraDecision) -> bool {
    match decision {
        AstraDecision::CancelRun { .. }
        | AstraDecision::CompleteRun { .. }
        | AstraDecision::ErrorRun { .. } => true,
        AstraDecision::Composite { decisions } => decisions.iter().any(decision_is_terminal),
        _ => false,
    }
}
