use anyhow::Result;
use serde_json::json;

use super::{
    next_dispatchable_tasks, thread_all_stages_terminal, thread_waiting_for_review, AstraDecision,
    AstraRun, AstraRunStatus, AstraService,
};
use crate::astra::decision::{AstraDecisionEngine, DeterministicDecisionEngine};
use crate::astra::planner::{AstraPlanner, DeterministicPlanner};
use crate::models::StageStatus;

enum DecisionOutcome {
    Continue,
    Terminal,
}

pub(super) enum RustNativeWorkerOutcome {
    Claimed,
    Duplicate,
}

impl AstraService {
    pub(super) fn run_rust_native_orchestrator(
        &self,
        run_id: &str,
        prompt: Option<String>,
    ) -> Result<RustNativeWorkerOutcome> {
        let _worker = match AstraWorkerGuard::new(self, run_id)? {
            Some(worker) => worker,
            None => return Ok(RustNativeWorkerOutcome::Duplicate),
        };
        let round_limit = self.load_run(run_id)?.round_limit;
        for round_index in 0..round_limit {
            let mut run = self.load_run(run_id)?;
            if !run.status.active() {
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            let thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
            if thread_all_stages_terminal(&thread) {
                let _ = self.complete_run(&run.run_id, "all_stages_terminal")?;
                return Ok(RustNativeWorkerOutcome::Claimed);
            }

            let planner = DeterministicPlanner;
            let plan = planner.plan(&run, &thread, prompt.as_deref(), round_index);
            let (planned, ()) = self.mutate_run(&run.run_id, {
                let tasks = plan.tasks.clone();
                move |next| {
                    if !next.status.active() {
                        return Ok(());
                    }
                    next.status = AstraRunStatus::Dispatching;
                    next.planner_backend = Some("deterministic".to_string());
                    next.round_index = Some(round_index);
                    for task in tasks {
                        if !next
                            .proposed_tasks
                            .iter()
                            .any(|existing| existing.id == task.id)
                        {
                            next.proposed_tasks.push(task);
                        }
                    }
                    Ok(())
                }
            })?;
            self.emit(
                &planned,
                "plan",
                json!({
                    "summary": plan.summary,
                    "tasks": plan.tasks,
                    "plannerBackend": "deterministic",
                    "roundIndex": round_index,
                }),
            );

            let dispatchable = next_dispatchable_tasks(&planned);
            if dispatchable.is_empty() {
                let latest = self.inner.store.get_thread_work_state(&planned.thread_id)?;
                if thread_all_stages_terminal(&latest) {
                    let _ = self.complete_run(
                        &planned.run_id,
                        "no_dispatchable_tasks_all_stages_terminal",
                    )?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                if thread_waiting_for_review(&latest) {
                    let _ = self.complete_run(&planned.run_id, "pending_human_review")?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                match classify_no_dispatchable_tasks(&latest) {
                    NoDispatchableOutcome::Completed(reason) => {
                        let _ = self.complete_run(&planned.run_id, reason)?;
                    }
                    NoDispatchableOutcome::Errored {
                        reason,
                        code,
                        message,
                    } => {
                        let _ = self.error_run(&planned.run_id, reason, code, message)?;
                    }
                }
                return Ok(RustNativeWorkerOutcome::Claimed);
            }

            let mut continue_rounds = false;
            for task in dispatchable {
                run = self.load_run(run_id)?;
                if !run.status.active() {
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                let result = match self.dispatch_task_and_wait(&run, &task) {
                    Ok(result) => result,
                    Err(error) => {
                        let _ = self.fail_run(&run.run_id, error.to_string())?;
                        return Ok(RustNativeWorkerOutcome::Claimed);
                    }
                };
                if !self.load_run(run_id)?.status.active() {
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                let latest_thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
                let decision_engine = DeterministicDecisionEngine;
                let decision = decision_engine.decide(&latest_thread, &result, &task);
                let _ = self.mutate_run(&run.run_id, |next| {
                    if next.status.active() {
                        next.decision_backend = Some("deterministic".to_string());
                    }
                    Ok(())
                })?;
                match self.apply_astra_decision(&run, decision)? {
                    DecisionOutcome::Continue => continue_rounds = true,
                    DecisionOutcome::Terminal => return Ok(RustNativeWorkerOutcome::Claimed),
                }
            }
            if !continue_rounds {
                let _ = self.complete_run(run_id, "no_more_work")?;
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
        }

        let _ = self.error_run(
            run_id,
            "round_limit_reached",
            "round_limit_reached",
            "Astra round limit reached".to_string(),
        )?;
        Ok(RustNativeWorkerOutcome::Claimed)
    }

    fn emit_decision(&self, run: &AstraRun, decision: &AstraDecision) -> Result<()> {
        self.emit(run, "decision", serde_json::to_value(decision)?);
        Ok(())
    }

    fn apply_astra_decision(
        &self,
        run: &AstraRun,
        decision: AstraDecision,
    ) -> Result<DecisionOutcome> {
        self.emit_decision(run, &decision)?;
        match decision {
            AstraDecision::UpdateStage { args } => {
                let outcome = self.apply_stage_update_decision(run, &args)?;
                if !outcome.ok {
                    return self.fail_run(
                        &run.run_id,
                        outcome
                            .error
                            .unwrap_or_else(|| "stage update failed".to_string()),
                    );
                }
                Ok(DecisionOutcome::Continue)
            }
            AstraDecision::AddOrUpdateIssue { args } => {
                let outcome = self.apply_issue_decision(run, &args)?;
                if !outcome.ok {
                    return self.fail_run(
                        &run.run_id,
                        outcome
                            .error
                            .unwrap_or_else(|| "issue update failed".to_string()),
                    );
                }
                Ok(DecisionOutcome::Continue)
            }
            AstraDecision::RetryStage { .. } | AstraDecision::PlanNextRound { .. } => {
                Ok(DecisionOutcome::Continue)
            }
            AstraDecision::CancelRun { reason } => self.cancel_run(&run.run_id, reason),
            AstraDecision::CompleteRun { reason } => self.complete_run(&run.run_id, &reason),
            AstraDecision::ErrorRun { reason } => self.fail_run(&run.run_id, reason),
            AstraDecision::Composite { decisions } => {
                for decision in decisions {
                    if matches!(
                        self.apply_astra_decision(run, decision)?,
                        DecisionOutcome::Terminal
                    ) {
                        return Ok(DecisionOutcome::Terminal);
                    }
                }
                Ok(DecisionOutcome::Continue)
            }
        }
    }

    fn cancel_run(&self, run_id: &str, reason: String) -> Result<DecisionOutcome> {
        let (cancelled, changed) = self.mark_run_cancelled(run_id, &reason)?;
        if changed {
            self.emit(
                &cancelled,
                "cancelled",
                json!({ "status": cancelled.status.as_str(), "reason": reason }),
            );
        }
        Ok(DecisionOutcome::Terminal)
    }

    fn complete_run(&self, run_id: &str, reason: &str) -> Result<DecisionOutcome> {
        let (completed, changed) = self.mark_run_completed(run_id, reason)?;
        if changed {
            self.emit(&completed, "completed", json!({ "reason": reason }));
        }
        Ok(DecisionOutcome::Terminal)
    }

    fn error_run(
        &self,
        run_id: &str,
        reason: &str,
        code: &str,
        message: String,
    ) -> Result<DecisionOutcome> {
        let (errored, changed) = self.mark_run_errored(run_id, reason, code, message.clone())?;
        if changed {
            self.emit(
                &errored,
                "error",
                json!({ "message": message, "reason": reason, "errorCode": code }),
            );
        }
        Ok(DecisionOutcome::Terminal)
    }

    fn fail_run(&self, run_id: &str, message: String) -> Result<DecisionOutcome> {
        self.error_run(
            run_id,
            "orchestrator_failure",
            "orchestrator_error",
            message,
        )
    }
}

struct AstraWorkerGuard {
    service: AstraService,
    run_id: String,
}

impl AstraWorkerGuard {
    fn new(service: &AstraService, run_id: &str) -> Result<Option<Self>> {
        if !service.claim_pending_worker(run_id)? {
            return Ok(None);
        }
        Ok(Some(Self {
            service: service.clone(),
            run_id: run_id.to_string(),
        }))
    }
}

impl Drop for AstraWorkerGuard {
    fn drop(&mut self) {
        if let Ok(mut workers) = self.service.inner.orchestrator_workers.lock() {
            workers.remove(&self.run_id);
        }
    }
}

enum NoDispatchableOutcome<'a> {
    #[cfg_attr(not(test), allow(dead_code))]
    Completed(&'a str),
    #[cfg_attr(not(test), allow(dead_code))]
    Errored {
        reason: &'a str,
        code: &'a str,
        message: String,
    },
}

fn classify_no_dispatchable_tasks(thread: &crate::models::ThreadInfo) -> NoDispatchableOutcome<'_> {
    if thread.stages.is_empty() {
        return NoDispatchableOutcome::Completed("no_stages_to_orchestrate");
    }
    if thread.stages.iter().any(|stage| {
        !matches!(
            stage.status,
            StageStatus::Completed | StageStatus::Skipped | StageStatus::NeedsReview
        ) && super::pick_stage_agent(stage).is_none()
    }) {
        return NoDispatchableOutcome::Errored {
            reason: "stage_without_assignable_agent",
            code: "stage_without_assignable_agent",
            message: "Astra found non-terminal stages without an assignable assistant".to_string(),
        };
    }
    NoDispatchableOutcome::Errored {
        reason: "no_dispatchable_tasks",
        code: "no_dispatchable_tasks",
        message: "deterministic planner produced no dispatchable tasks".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astra::tests::{test_stage, test_thread};
    use crate::models::StageStatus;

    #[test]
    fn no_dispatchable_empty_stages_completes() {
        let thread = test_thread(Vec::new());

        match classify_no_dispatchable_tasks(&thread) {
            NoDispatchableOutcome::Completed(reason) => {
                assert_eq!(reason, "no_stages_to_orchestrate")
            }
            NoDispatchableOutcome::Errored { reason, .. } => {
                panic!("unexpected error outcome: {reason}")
            }
        }
    }

    #[test]
    fn no_dispatchable_stage_without_assistant_is_diagnostic_error() {
        let thread = test_thread(vec![test_stage("stage-1", StageStatus::InProgress)]);

        match classify_no_dispatchable_tasks(&thread) {
            NoDispatchableOutcome::Errored { reason, code, .. } => {
                assert_eq!(reason, "stage_without_assignable_agent");
                assert_eq!(code, "stage_without_assignable_agent");
            }
            NoDispatchableOutcome::Completed(reason) => {
                panic!("unexpected completed outcome: {reason}")
            }
        }
    }
}
