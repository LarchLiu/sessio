use anyhow::Result;
use serde_json::json;

use super::{
    next_dispatchable_tasks, now_ms, rolling_stage_task_batch, thread_all_stages_terminal,
    thread_waiting_for_review, AstraBackendConfig, AstraDecision, AstraOrchestration, AstraRun,
    AstraRunStatus, AstraService, AstraStageMutationResult, AstraTaskCompletion,
    ASTRA_ORCHESTRATOR_TIMEOUT_MS,
};
use crate::astra::astra_pi_acp_adapter::AstraPiAcpOrchestrator;
use crate::astra::backend::{BackendFailure, OrchestratorBackend};
use crate::astra::deterministic_backend::DeterministicOrchestratorBackend;
use crate::astra::runtime_agent_backend::{RuntimeAgentBackendConfig, RuntimeAgentOrchestrator};
use crate::models::{IssueStatus, StageStatus, StageType, ThreadInfo};

#[cfg(test)]
const MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS: usize = 50;
#[cfg(test)]
const MAX_RUN_DIAGNOSTICS: usize = 100;

enum DecisionOutcome {
    Continue,
    RetryTask,
    PlanNextRound,
    Terminal,
}

#[cfg(test)]
fn trim_vec_front<T>(values: &mut Vec<T>, max_len: usize) {
    if values.len() > max_len {
        values.drain(0..values.len() - max_len);
    }
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
        let mut round_index = 0u32;

        let mut run = self.load_run(run_id)?;
        if !run.status.active() {
            return Ok(RustNativeWorkerOutcome::Claimed);
        }
        let thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        if thread_all_stages_terminal(&thread) {
            let _ = self.complete_run(&run.run_id, "all_stages_terminal")?;
            return Ok(RustNativeWorkerOutcome::Claimed);
        }

        run = self.mark_run_status(
            &run.run_id,
            AstraRunStatus::Planning,
            None,
            "planning_round",
        )?;
        if !run.status.active() {
            return Ok(RustNativeWorkerOutcome::Claimed);
        }
        if round_index >= round_limit {
            let _ = self.error_run(
                run_id,
                "round_limit_reached",
                "round_limit_reached",
                "Astra round limit reached before initial planning".to_string(),
            )?;
            return Ok(RustNativeWorkerOutcome::Claimed);
        }
        let current_round_index = round_index;
        round_index += 1;
        let (orchestration, orchestrator_backend) = match self.orchestrate_astra_round(
            &run,
            &thread,
            prompt.as_deref(),
            current_round_index,
            &[],
        ) {
            Ok(orchestration) => orchestration,
            Err(error) => {
                let _ = self.error_run(
                    &run.run_id,
                    "orchestrator_policy_denied",
                    error.code,
                    error.message,
                )?;
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
        };
        let (planned, mut dispatch_batch) = self.apply_orchestration_tasks(
            &run,
            orchestration,
            &orchestrator_backend,
            current_round_index,
        )?;
        let mut current_run = planned;

        loop {
            if dispatch_batch.is_empty() {
                let mut latest = self
                    .inner
                    .store
                    .get_thread_work_state(&current_run.thread_id)?;
                if self.auto_complete_empty_done_stages(&current_run, &latest)? {
                    latest = self
                        .inner
                        .store
                        .get_thread_work_state(&current_run.thread_id)?;
                }
                if thread_all_stages_terminal(&latest) {
                    let _ = self.complete_run(
                        &current_run.run_id,
                        "no_dispatchable_tasks_all_stages_terminal",
                    )?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                if super::thread_waiting_for_review(&latest) {
                    let _ = self.complete_run(&current_run.run_id, "pending_human_review")?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                match classify_no_dispatchable_tasks(&latest) {
                    NoDispatchableOutcome::Completed(reason) => {
                        let _ = self.complete_run(&current_run.run_id, reason)?;
                    }
                    NoDispatchableOutcome::Errored {
                        reason,
                        code,
                        message,
                    } => {
                        let _ = self.error_run(&current_run.run_id, reason, code, message)?;
                    }
                }
                return Ok(RustNativeWorkerOutcome::Claimed);
            }

            current_run = self.load_run(run_id)?;
            if !current_run.status.active() {
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            let results = match self.dispatch_task_batch_and_wait(&current_run, &dispatch_batch) {
                Ok(results) => results,
                Err(error) => {
                    let _ = self.fail_run(&current_run.run_id, error.to_string())?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
            };
            let completions = results
                .into_iter()
                .map(|result| {
                    let task = dispatch_batch
                        .iter()
                        .find(|task| task.id == result.task_id)
                        .cloned()
                        .ok_or_else(|| {
                            anyhow::anyhow!("Astra task result has no proposal: {}", result.task_id)
                        })?;
                    Ok(AstraTaskCompletion { task, result })
                })
                .collect::<Result<Vec<_>>>()?;
            current_run = self.mark_run_status(
                run_id,
                AstraRunStatus::Thinking,
                completions
                    .first()
                    .and_then(|completion| completion.result.thread_stage_id.clone()),
                "task_batch_result_received",
            )?;
            if !current_run.status.active() {
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            if round_index >= round_limit {
                let _ = self.error_run(
                    &current_run.run_id,
                    "round_limit_reached",
                    "round_limit_reached",
                    "Astra round limit reached before evaluating returned task results".to_string(),
                )?;
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            let current_round_index = round_index;
            round_index += 1;
            let latest_thread = self
                .inner
                .store
                .get_thread_work_state(&current_run.thread_id)?;
            let (orchestration, orchestrator_backend) = match self.orchestrate_astra_round(
                &current_run,
                &latest_thread,
                prompt.as_deref(),
                current_round_index,
                &completions,
            ) {
                Ok(orchestration) => orchestration,
                Err(error) => {
                    let _ = self.error_run(
                        &current_run.run_id,
                        "orchestrator_policy_denied",
                        error.code,
                        error.message,
                    )?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
            };
            current_run = self.apply_orchestration_decisions(
                &current_run,
                &orchestration,
                &completions,
                &orchestrator_backend,
            )?;
            if !current_run.status.active() {
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            if orchestration
                .decisions
                .iter()
                .any(|decision| matches!(decision.decision, AstraDecision::RetryStage { .. }))
            {
                let _ = self.error_run(
                    &current_run.run_id,
                    "batch_retry_stage_unsupported",
                    "batch_retry_stage_unsupported",
                    "Astra Orchestrator requested retry_stage after a batch result; return tasks for the next rolling batch instead.".to_string(),
                )?;
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            let (next_run, next_batch) = self.apply_orchestration_tasks(
                &current_run,
                orchestration,
                &orchestrator_backend,
                current_round_index,
            )?;
            current_run = next_run;
            dispatch_batch = next_batch;
            if !current_run.status.active() {
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            if dispatch_batch.is_empty()
                && thread_all_stages_terminal(
                    &self
                        .inner
                        .store
                        .get_thread_work_state(&current_run.thread_id)?,
                )
            {
                let _ = self.complete_run(&current_run.run_id, "all_stages_terminal")?;
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
        }
    }

    fn orchestrate_astra_round(
        &self,
        run: &AstraRun,
        thread: &crate::models::ThreadInfo,
        prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
    ) -> std::result::Result<(AstraOrchestration, String), BackendFailure> {
        let backend_config = self.astra_backend_config();
        let orchestrator_backend: Box<dyn OrchestratorBackend> =
            self.create_orchestrator_backend(&backend_config);
        let config_value = json!(backend_config.provider_config);

        match orchestrator_backend.orchestrate(
            run,
            thread,
            prompt,
            round_index,
            completions,
            &config_value,
        ) {
            Ok(response) => {
                log::info!(
                    "[astra:orchestrator:success] run={} backend={} sessionId={} completions={}",
                    run.run_id,
                    response.backend_type,
                    response.session_id,
                    completions.len()
                );
                Ok((response.data, response.backend_type))
            }
            Err(failure) => {
                log::warn!(
                    "[astra:orchestrator:failure] run={} backend={} code={} message={}",
                    run.run_id,
                    failure.backend_type,
                    failure.code,
                    failure.message
                );
                Err(failure)
            }
        }
    }

    fn create_orchestrator_backend(
        &self,
        config: &AstraBackendConfig,
    ) -> Box<dyn OrchestratorBackend> {
        if let Some(agent) = config.agent {
            log::info!(
                "[astra:orchestrator:backend] using runtime_agent backend with agent={}",
                agent.as_str()
            );
            let runtime_config = RuntimeAgentBackendConfig {
                agent,
                timeout_ms: ASTRA_ORCHESTRATOR_TIMEOUT_MS,
                model: config.model.clone(),
                effort: config.effort.clone(),
                permission_mode: config.permission_mode.clone(),
            };
            return Box::new(RuntimeAgentOrchestrator::new(
                self.inner.runtime.clone(),
                runtime_config,
            ));
        }

        if let Some(astra_pi_acp_config) = self.inner.astra_pi_acp_config.clone() {
            log::info!("[astra:orchestrator:backend] using astra_pi_acp backend");
            return Box::new(AstraPiAcpOrchestrator::new(astra_pi_acp_config));
        }

        log::info!("[astra:orchestrator:backend] using deterministic backend");
        Box::new(DeterministicOrchestratorBackend)
    }

    fn apply_orchestration_decisions(
        &self,
        run: &AstraRun,
        orchestration: &AstraOrchestration,
        completions: &[AstraTaskCompletion],
        orchestrator_backend: &str,
    ) -> Result<AstraRun> {
        let orchestrator_backend_for_run = orchestrator_backend.to_string();
        let _ = self.mutate_run(&run.run_id, |next| {
            if next.status.active() {
                next.decision_backend = Some(orchestrator_backend_for_run);
            }
            Ok(())
        })?;
        let mut latest = self.load_run(&run.run_id)?;
        for task_decision in &orchestration.decisions {
            if !completions
                .iter()
                .any(|completion| completion.task.id == task_decision.task_id)
            {
                let _ = self.error_run(
                    &latest.run_id,
                    "decision_without_completed_task",
                    "decision_without_completed_task",
                    format!(
                        "Astra Orchestrator returned a decision for unknown task {}",
                        task_decision.task_id
                    ),
                )?;
                return self.load_run(&run.run_id);
            }
            match self.apply_astra_decision(
                &latest,
                task_decision.decision.clone(),
                &task_decision.task_id,
            )? {
                DecisionOutcome::Continue
                | DecisionOutcome::PlanNextRound
                | DecisionOutcome::RetryTask => {
                    latest = self.load_run(&run.run_id)?;
                }
                DecisionOutcome::Terminal => return self.load_run(&run.run_id),
            }
        }
        Ok(latest)
    }

    fn apply_orchestration_tasks(
        &self,
        run: &AstraRun,
        orchestration: AstraOrchestration,
        orchestrator_backend: &str,
        round_index: u32,
    ) -> Result<(AstraRun, Vec<super::AstraTaskProposal>)> {
        let tasks = rolling_stage_task_batch(orchestration.tasks);
        let summary = orchestration.summary;
        let latest_thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        if tasks.is_empty() {
            if !thread_all_stages_terminal(&latest_thread)
                && !thread_waiting_for_review(&latest_thread)
                && thread_has_dispatchable_stage(run, &latest_thread)
            {
                let _ = self.error_run(
                    &run.run_id,
                    "orchestrator_missing_next_tasks",
                    "orchestrator_missing_next_tasks",
                    "Astra Orchestrator returned no next tasks while the thread still has dispatchable stages.".to_string(),
                )?;
                return Ok((self.load_run(&run.run_id)?, Vec::new()));
            }
        }
        let tasks = self.create_plan_round_for_astra_tasks(
            run,
            &latest_thread,
            &summary,
            round_index,
            tasks,
        )?;
        let orchestrator_backend_for_run = orchestrator_backend.to_string();
        let (planned, ()) = self.mutate_run(&run.run_id, {
            let tasks = tasks.clone();
            move |next| {
                if !next.status.active() {
                    return Ok(());
                }
                next.status = AstraRunStatus::Dispatching;
                next.planner_backend = Some(orchestrator_backend_for_run);
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
                "summary": summary,
                "tasks": tasks.clone(),
                "plannerBackend": orchestrator_backend,
                "roundIndex": round_index,
            }),
        );
        let dispatchable = next_dispatchable_tasks(&planned, &latest_thread);
        let dispatch_batch = rolling_stage_task_batch(dispatchable);
        Ok((planned, dispatch_batch))
    }

    fn emit_decision(&self, run: &AstraRun, decision: &AstraDecision) -> Result<()> {
        self.emit(run, "decision", serde_json::to_value(decision)?);
        Ok(())
    }

    fn apply_astra_decision(
        &self,
        run: &AstraRun,
        decision: AstraDecision,
        current_task_id: &str,
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
            AstraDecision::RetryStage { reason } => {
                self.record_task_decision(run, current_task_id, "retry_stage", &reason)?;
                Ok(DecisionOutcome::RetryTask)
            }
            AstraDecision::PlanNextRound { reason } => {
                self.record_task_decision(run, current_task_id, "plan_next_round", &reason)?;
                Ok(DecisionOutcome::PlanNextRound)
            }
            AstraDecision::CancelRun { reason } => self.cancel_run(&run.run_id, reason),
            AstraDecision::CompleteRun { reason } => self.complete_run(&run.run_id, &reason),
            AstraDecision::ErrorRun { reason } => self.fail_run(&run.run_id, reason),
            AstraDecision::Composite { decisions } => {
                for decision in decisions {
                    match self.apply_astra_decision(run, decision, current_task_id)? {
                        DecisionOutcome::Continue => {}
                        other => return Ok(other),
                    }
                }
                Ok(DecisionOutcome::Continue)
            }
        }
    }

    fn record_task_decision(
        &self,
        run: &AstraRun,
        task_id: &str,
        action: &str,
        reason: &str,
    ) -> Result<()> {
        let _ = self.mutate_run(&run.run_id, |next| {
            if let Some(result) = next
                .task_results
                .iter_mut()
                .rev()
                .find(|result| result.task_id == task_id)
            {
                result.decision_action = Some(action.to_string());
                result.decision_reason = Some(reason.to_string());
            }
            Ok(())
        })?;
        Ok(())
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
        let run = self.load_run(run_id)?;
        if run.status.active() {
            let thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
            self.auto_complete_empty_done_stages(&run, &thread)?;
        }
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

    fn mark_run_status(
        &self,
        run_id: &str,
        status: AstraRunStatus,
        thread_stage_id: Option<String>,
        reason: &'static str,
    ) -> Result<AstraRun> {
        let (run, changed) = self.mutate_run(run_id, move |next| {
            if !next.status.active() {
                return Ok(false);
            }
            next.status = status;
            if let Some(stage_id) = thread_stage_id {
                next.current_stage_id = Some(stage_id);
            }
            Ok(true)
        })?;
        if changed {
            self.emit(
                &run,
                "status",
                json!({ "status": run.status.as_str(), "reason": reason }),
            );
        }
        Ok(run)
    }

    fn auto_complete_empty_done_stages(&self, run: &AstraRun, thread: &ThreadInfo) -> Result<bool> {
        let stages = thread
            .stages
            .iter()
            .filter(|stage| auto_completable_empty_done_stage(thread, stage))
            .map(|stage| {
                (
                    stage.id.clone(),
                    stage.summary.is_none(),
                    stage.outcome.is_none(),
                )
            })
            .collect::<Vec<_>>();
        if stages.is_empty() {
            return Ok(false);
        }

        for (stage_id, needs_summary, needs_outcome) in stages {
            let stage = self.inner.store.update_thread_stage_state(
                &stage_id,
                Some(StageStatus::Completed),
                needs_summary
                    .then(|| Some("Astra auto-completed this empty final stage.".to_string())),
                needs_outcome.then(|| {
                    Some("No delegated work was required for this final stage.".to_string())
                }),
            )?;
            let stage_id_for_run = stage.id.clone();
            let (next, ()) = self.mutate_run(&run.run_id, move |next| {
                if next.status.active() {
                    next.current_stage_id = Some(stage_id_for_run);
                }
                Ok(())
            })?;
            let result = AstraStageMutationResult {
                ok: true,
                stage: Some(serde_json::to_value(stage)?),
                issue: None,
                error: None,
                applied_at: now_ms(),
            };
            self.emit(&next, "stage_update_result", serde_json::to_value(&result)?);
            log::info!(
                "[astra:stage:auto-complete-empty-done] runId={} threadId={} stageId={}",
                next.run_id,
                next.thread_id,
                stage_id
            );
        }

        Ok(true)
    }
}

fn auto_completable_empty_done_stage(
    thread: &crate::models::ThreadInfo,
    stage: &crate::models::StageInfo,
) -> bool {
    empty_done_stage_without_assistant(stage)
        && matches!(
            stage.status,
            StageStatus::NotStarted | StageStatus::InProgress
        )
        && !stage
            .issues
            .iter()
            .any(|issue| issue.status == IssueStatus::Open)
        && thread.stages.iter().all(|other| {
            other.id == stage.id
                || matches!(other.status, StageStatus::Completed | StageStatus::Skipped)
        })
}

fn empty_done_stage_without_assistant(stage: &crate::models::StageInfo) -> bool {
    matches!(stage.kind, Some(StageType::Done))
        && stage.allow_empty_assistants
        && stage.assistant_ids.is_empty()
        && stage.assistants.is_empty()
}

fn thread_has_dispatchable_stage(run: &AstraRun, thread: &crate::models::ThreadInfo) -> bool {
    thread.stages.iter().any(|stage| {
        !matches!(stage.status, StageStatus::Completed | StageStatus::Skipped)
            && super::pick_stage_agent(stage).is_some()
            && super::task_blocked_by_thread_exception(run, thread, Some(&stage.id)).is_none()
    })
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
    if super::thread_waiting_for_review(thread) {
        return NoDispatchableOutcome::Completed("pending_human_review");
    }
    if thread.stages.iter().any(|stage| {
        !matches!(stage.status, StageStatus::Completed | StageStatus::Skipped)
            && !empty_done_stage_without_assistant(stage)
            && super::pick_stage_agent(stage).is_none()
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
        message: "Astra Orchestrator produced no dispatchable tasks".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::astra::tests::{test_stage, test_thread};
    use crate::models::{Agent, IssueSeverity, StageIssueInfo, StageStatus, StageType};

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

    #[test]
    fn no_dispatchable_human_review_takes_priority_over_empty_done_stage() {
        let mut research = test_stage("research", StageStatus::NeedsReview);
        research.kind = Some(StageType::Human);
        let mut done = test_stage("done-stage", StageStatus::NotStarted);
        done.kind = Some(StageType::Done);
        done.allow_empty_assistants = true;
        let thread = test_thread(vec![research, done]);

        match classify_no_dispatchable_tasks(&thread) {
            NoDispatchableOutcome::Completed(reason) => {
                assert_eq!(reason, "pending_human_review")
            }
            NoDispatchableOutcome::Errored { reason, .. } => {
                panic!("unexpected error outcome: {reason}")
            }
        }
    }

    #[test]
    fn no_dispatchable_empty_done_stage_is_not_missing_assistant() {
        let mut done = test_stage("done-stage", StageStatus::NotStarted);
        done.kind = Some(StageType::Done);
        done.allow_empty_assistants = true;
        let thread = test_thread(vec![done]);

        match classify_no_dispatchable_tasks(&thread) {
            NoDispatchableOutcome::Errored { reason, .. } => {
                assert_eq!(reason, "no_dispatchable_tasks")
            }
            NoDispatchableOutcome::Completed(reason) => {
                panic!("unexpected completed outcome: {reason}")
            }
        }
    }

    #[test]
    fn empty_done_stage_auto_completes_after_other_stages_terminal() {
        let completed = test_stage("writing", StageStatus::Completed);
        let mut done = test_stage("done-stage", StageStatus::NotStarted);
        done.kind = Some(StageType::Done);
        done.allow_empty_assistants = true;
        let thread = test_thread(vec![completed, done.clone()]);

        assert!(auto_completable_empty_done_stage(&thread, &done));
    }

    #[test]
    fn rolling_batch_discards_only_undispatched_tasks() {
        let dispatchable = vec![
            test_task("task-1"),
            test_task("task-2"),
            test_task("task-3"),
        ];

        let dispatched = vec![test_task("task-1"), test_task("task-2")];
        let dispatched_ids = dispatched
            .iter()
            .map(|task| task.id.as_str())
            .collect::<HashSet<_>>();
        let remaining = dispatchable
            .iter()
            .filter(|task| !dispatched_ids.contains(task.id.as_str()))
            .map(|task| task.id.clone())
            .collect::<HashSet<_>>();

        assert!(!remaining.contains("task-1"));
        assert!(!remaining.contains("task-2"));
        assert!(remaining.contains("task-3"));
    }

    #[test]
    fn dispatchable_tasks_prioritize_agent_review_stage() {
        let mut run = test_run("run-dispatch-exception");
        let mut research = test_stage("research", StageStatus::NeedsReview);
        let mut plan = test_stage("plan", StageStatus::InProgress);
        research.assistants.push(stage_assistant());
        plan.assistants.push(stage_assistant());
        run.proposed_tasks.push(super::super::AstraTaskProposal {
            id: "task-plan".to_string(),
            plan_task_id: None,
            title: "Plan".to_string(),
            target_stage_id: Some("plan".to_string()),
            target_agent: Agent::Codex,
            prompt: "Plan next.".to_string(),
            expected_output: "Plan.".to_string(),
            risk: super::super::AstraTaskRisk::Low,
        });
        run.proposed_tasks.push(super::super::AstraTaskProposal {
            id: "task-review".to_string(),
            plan_task_id: None,
            title: "Review".to_string(),
            target_stage_id: Some("research".to_string()),
            target_agent: Agent::Codex,
            prompt: "Review research.".to_string(),
            expected_output: "Review notes.".to_string(),
            risk: super::super::AstraTaskRisk::Medium,
        });
        let thread = test_thread(vec![research, plan]);

        let dispatchable = next_dispatchable_tasks(&run, &thread);

        assert_eq!(dispatchable.len(), 1);
        assert_eq!(dispatchable[0].id, "task-review");
    }

    #[test]
    fn dispatchable_tasks_allow_blocked_stage_recovery_only() {
        let mut run = test_run("run-dispatch-blocked");
        let mut blocked = test_stage("blocked", StageStatus::Blocked);
        let mut plan = test_stage("plan", StageStatus::InProgress);
        blocked.assistants.push(stage_assistant());
        plan.assistants.push(stage_assistant());
        run.proposed_tasks.push(super::super::AstraTaskProposal {
            id: "task-plan".to_string(),
            plan_task_id: None,
            title: "Plan".to_string(),
            target_stage_id: Some("plan".to_string()),
            target_agent: Agent::Codex,
            prompt: "Plan next.".to_string(),
            expected_output: "Plan.".to_string(),
            risk: super::super::AstraTaskRisk::Low,
        });
        run.proposed_tasks.push(super::super::AstraTaskProposal {
            id: "task-blocked".to_string(),
            plan_task_id: None,
            title: "Unblock".to_string(),
            target_stage_id: Some("blocked".to_string()),
            target_agent: Agent::Codex,
            prompt: "Recover blocked stage.".to_string(),
            expected_output: "Recovery notes.".to_string(),
            risk: super::super::AstraTaskRisk::High,
        });
        let thread = test_thread(vec![blocked, plan]);

        let dispatchable = next_dispatchable_tasks(&run, &thread);

        assert_eq!(dispatchable.len(), 1);
        assert_eq!(dispatchable[0].id, "task-blocked");
    }

    #[test]
    fn empty_done_stage_without_assistant_is_auto_completable() {
        let mut stage = test_stage("done-stage", StageStatus::NotStarted);
        stage.kind = Some(StageType::Done);
        stage.allow_empty_assistants = true;
        let thread = test_thread(vec![stage.clone()]);

        assert!(auto_completable_empty_done_stage(&thread, &stage));

        stage.kind = Some(StageType::Human);
        let thread = test_thread(vec![stage.clone()]);
        assert!(!auto_completable_empty_done_stage(&thread, &stage));

        stage.kind = Some(StageType::Done);
        stage.issues.push(StageIssueInfo {
            id: "issue-1".to_string(),
            thread_stage_id: stage.id.clone(),
            title: "Needs final approval".to_string(),
            description: None,
            status: crate::models::IssueStatus::Open,
            severity: IssueSeverity::Medium,
            created_at: 1,
            updated_at: 1,
        });
        let thread = test_thread(vec![stage.clone()]);
        assert!(!auto_completable_empty_done_stage(&thread, &stage));
    }

    #[test]
    fn empty_done_stage_waits_for_all_other_stages_to_finish() {
        let mut work = test_stage("work-stage", StageStatus::NotStarted);
        work.assistants.push(stage_assistant());
        let mut done = test_stage("done-stage", StageStatus::NotStarted);
        done.kind = Some(StageType::Done);
        done.allow_empty_assistants = true;
        let thread = test_thread(vec![work.clone(), done.clone()]);

        assert!(!auto_completable_empty_done_stage(&thread, &done));

        work.status = StageStatus::Completed;
        let thread = test_thread(vec![work, done.clone()]);

        assert!(auto_completable_empty_done_stage(&thread, &done));
    }

    #[test]
    fn bounded_diagnostics_keep_recent_entries() {
        let mut values = (0..105).collect::<Vec<_>>();

        trim_vec_front(&mut values, MAX_RUN_DIAGNOSTICS);

        assert_eq!(values.len(), MAX_RUN_DIAGNOSTICS);
        assert_eq!(values[0], 5);
        assert_eq!(values[99], 104);
    }

    #[test]
    fn bounded_internal_sessions_keep_recent_entries() {
        let mut values = (0..55)
            .map(|idx| format!("session-{idx}"))
            .collect::<Vec<_>>();

        trim_vec_front(&mut values, MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS);

        assert_eq!(values.len(), MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS);
        assert_eq!(values[0], "session-5");
        assert_eq!(values[49], "session-54");
    }

    fn test_task(id: &str) -> super::super::AstraTaskProposal {
        super::super::AstraTaskProposal {
            id: id.to_string(),
            plan_task_id: None,
            title: id.to_string(),
            target_stage_id: None,
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: super::super::AstraTaskRisk::Low,
        }
    }

    fn test_run(run_id: &str) -> AstraRun {
        AstraRun {
            run_id: run_id.to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp".to_string(),
            status: AstraRunStatus::Running,
            proposed_tasks: Vec::new(),
            approved_task_ids: Vec::new(),
            delegated_session_ids: Vec::new(),
            task_results: Vec::new(),
            mode: "auto".to_string(),
            current_stage_id: None,
            completed_task_ids: Vec::new(),
            stage_attempt_counts: std::collections::HashMap::new(),
            retry_limit: super::super::ASTRA_DEFAULT_RETRY_LIMIT,
            planner_backend: Some("deterministic".to_string()),
            decision_backend: Some("deterministic".to_string()),
            round_index: None,
            round_limit: super::super::RUST_NATIVE_ROUND_LIMIT,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn stage_assistant() -> crate::models::StageAssistantInfo {
        crate::models::StageAssistantInfo {
            assistant_id: "assistant-codex".to_string(),
            name: "Codex".to_string(),
            color: None,
            agent: crate::models::AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: "gpt-5.3-codex".to_string(),
                mode: "read-write".to_string(),
                effort: "medium".to_string(),
            },
            system_prompt: None,
            order: 0,
        }
    }
}
