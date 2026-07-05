use serde_json::Value;

use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::planner::{deterministic_plan, remaining_process_stages};
use super::{
    AstraOrchestration, AstraPlannerContext, AstraRun, AstraRunIntent, AstraTaskCompletion,
};
use crate::models::{PlanRoundMode, ThreadInfo, ThreadKind};

/// Deterministic Orchestrator backend (rule-based, no external agent).
pub struct DeterministicOrchestratorBackend;

impl OrchestratorBackend for DeterministicOrchestratorBackend {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        _completion_artifact_paths: &std::collections::HashMap<String, String>,
        _planner_context: &AstraPlannerContext,
        _config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure> {
        let orchestration =
            deterministic_orchestration(run, thread, user_prompt, round_index, completions)?;
        Ok(BackendResponse {
            data: orchestration,
            session_id: format!("deterministic-orchestrator-{}-{}", run.run_id, round_index),
            backend_type: "deterministic".to_string(),
        })
    }
}

fn deterministic_orchestration(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    completions: &[AstraTaskCompletion],
) -> Result<AstraOrchestration, BackendFailure> {
    if thread.kind == ThreadKind::Process {
        return Ok(deterministic_process_orchestration(
            run,
            thread,
            user_prompt,
            round_index,
            completions,
        ));
    }

    if thread.kind != ThreadKind::Teamwork {
        return Ok(AstraOrchestration {
            summary: "Astra automatic orchestration is only supported for teamwork threads."
                .to_string(),
            run_intent: AstraRunIntent::Error,
            reason: "astra_orchestration_unsupported_for_thread_kind".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        });
    }

    if completions.iter().any(|completion| {
        matches!(
            completion.result.status,
            super::AstraTaskResultStatus::Failed
                | super::AstraTaskResultStatus::Errored
                | super::AstraTaskResultStatus::Cancelled
        )
    }) {
        return Ok(AstraOrchestration {
            summary: format!(
                "Deterministic Astra Orchestrator received {} completion(s) with a terminal failure.",
                completions.len()
            ),
            run_intent: AstraRunIntent::Error,
            reason: "teamwork_task_failed".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        });
    }

    if !completions.is_empty() {
        return Ok(AstraOrchestration {
            summary: format!(
                "Deterministic Astra Orchestrator completed after {} teamwork completion(s).",
                completions.len()
            ),
            run_intent: AstraRunIntent::Complete,
            reason: "teamwork_round_completed".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        });
    }

    let plan = deterministic_plan(run, thread, user_prompt, round_index);
    if plan.tasks.is_empty() {
        return Ok(AstraOrchestration {
            summary: plan.summary,
            run_intent: AstraRunIntent::WaitForHuman,
            reason: "teamwork_no_dispatchable_tasks".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        });
    }

    Ok(AstraOrchestration {
        summary: plan.summary,
        run_intent: AstraRunIntent::Continue,
        reason: "continue_with_teamwork_tasks".to_string(),
        mode: Some(PlanRoundMode::Parallel),
        tasks: plan.tasks,
        diagnostics: Vec::new(),
    })
}

fn deterministic_process_orchestration(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    completions: &[AstraTaskCompletion],
) -> AstraOrchestration {
    if let Some(completion) = completions.iter().find(|completion| {
        matches!(
            completion.result.status,
            super::AstraTaskResultStatus::Failed
                | super::AstraTaskResultStatus::Errored
                | super::AstraTaskResultStatus::Cancelled
        )
    }) {
        return AstraOrchestration {
            summary: completion
                .result
                .error
                .clone()
                .unwrap_or_else(|| format!("Process task failed: {}", completion.task.title)),
            run_intent: AstraRunIntent::Error,
            reason: "process_task_failed".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    if remaining_process_stages(thread).is_empty() {
        return AstraOrchestration {
            summary: format!("Process completed for \"{}\".", thread.goal),
            run_intent: AstraRunIntent::Complete,
            reason: "process_completed".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    let plan = deterministic_plan(run, thread, user_prompt, round_index);
    if plan.tasks.is_empty() {
        return AstraOrchestration {
            summary: plan.summary,
            run_intent: AstraRunIntent::WaitForHuman,
            reason: "process_manual_checkpoint".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    AstraOrchestration {
        summary: plan.summary,
        run_intent: AstraRunIntent::Continue,
        reason: "continue_with_process_tasks".to_string(),
        mode: Some(PlanRoundMode::Sequential),
        tasks: plan.tasks,
        diagnostics: Vec::new(),
    }
}
