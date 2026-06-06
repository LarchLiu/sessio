use std::collections::HashSet;

use anyhow::Result;
use serde_json::json;

use super::{
    next_dispatchable_tasks, now_ms, thread_all_stages_terminal, thread_waiting_for_review,
    AstraBackendConfig, AstraDecision, AstraRun, AstraRunStatus, AstraService,
    AstraStageMutationResult, ASTRA_PI_ACP_TIMEOUT_MS,
};
use crate::astra::astra_pi_acp_adapter::{AstraPiAcpDecisionEngine, AstraPiAcpPlanner};
use crate::astra::backend::{BackendFailure, DecisionBackend, PlannerBackend};
use crate::astra::deterministic_backend::{
    DeterministicDecisionBackend, DeterministicPlannerBackend,
};
use crate::astra::runtime_agent_backend::{
    RuntimeAgentBackendConfig, RuntimeAgentDecisionEngine, RuntimeAgentPlanner,
};
use crate::models::{IssueStatus, StageStatus, StageType, ThreadInfo};

#[cfg(test)]
const MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS: usize = 50;
#[cfg(test)]
const MAX_RUN_DIAGNOSTICS: usize = 100;

enum DecisionOutcome {
    Continue,
    RetryTask { reason: String },
    PlanNextRound { reason: String },
    Terminal,
}

fn internal_failure_diagnostic(
    kind: &'static str,
    backend_type: &str,
    failure: &BackendFailure,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut diagnostic = json!({
        "kind": kind,
        "backendType": backend_type,
        "code": failure.code,
        "message": redact_diagnostic_message(&failure.message),
    });
    if let Some(session_id) = failure.session_id.as_ref() {
        diagnostic["sessionId"] = json!(session_id);
    }
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            diagnostic[key] = value.clone();
        }
    }
    diagnostic
}

fn redact_diagnostic_message(message: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut redacted = message.chars().take(MAX_CHARS).collect::<String>();
    if message.chars().count() > MAX_CHARS {
        redacted.push_str("...");
    }
    redacted
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

            run = self.mark_run_status(
                &run.run_id,
                AstraRunStatus::Planning,
                None,
                "planning_round",
            )?;
            if !run.status.active() {
                return Ok(RustNativeWorkerOutcome::Claimed);
            }
            let (plan, planner_backend, planner_fallback) =
                match self.plan_astra_round(&run, &thread, prompt.as_deref(), round_index) {
                    Ok(plan) => plan,
                    Err(error) => {
                        let _ = self.error_run(
                            &run.run_id,
                            "planner_policy_denied",
                            error.code,
                            error.message,
                        )?;
                        return Ok(RustNativeWorkerOutcome::Claimed);
                    }
                };
            let planner_backend_for_run = planner_backend.to_string();
            let (planned, ()) = self.mutate_run(&run.run_id, {
                let tasks = plan.tasks.clone();
                move |next| {
                    if !next.status.active() {
                        return Ok(());
                    }
                    next.status = AstraRunStatus::Dispatching;
                    next.planner_backend = Some(planner_backend_for_run);
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
                    "plannerBackend": planner_backend,
                    "fallback": planner_fallback,
                    "roundIndex": round_index,
                }),
            );

            let dispatchable = next_dispatchable_tasks(&planned);
            if dispatchable.is_empty() {
                let mut latest = self.inner.store.get_thread_work_state(&planned.thread_id)?;
                if self.auto_complete_empty_done_stages(&planned, &latest)? {
                    latest = self.inner.store.get_thread_work_state(&planned.thread_id)?;
                }
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
            for (task_index, task) in dispatchable.iter().cloned().enumerate() {
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
                run = self.mark_run_status(
                    run_id,
                    AstraRunStatus::Thinking,
                    result.thread_stage_id.clone(),
                    "task_result_received",
                )?;
                if !run.status.active() {
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                match self.decide_and_apply_task_result(&run, &result, &task)? {
                    DecisionOutcome::Continue => continue_rounds = true,
                    DecisionOutcome::RetryTask { reason } => {
                        match self.retry_task_until_settled(run_id, &task, reason)? {
                            DecisionOutcome::Continue => continue_rounds = true,
                            DecisionOutcome::PlanNextRound { reason } => {
                                let discarded = self.discard_remaining_dispatchable_tasks(
                                    &run.run_id,
                                    &dispatchable,
                                    task_index,
                                )?;
                                self.emit(
                                    &run,
                                    "plan_next_round",
                                    json!({
                                        "taskId": task.id,
                                        "reason": reason,
                                        "discardedTaskIds": discarded,
                                    }),
                                );
                                continue_rounds = true;
                                break;
                            }
                            DecisionOutcome::RetryTask { reason } => {
                                let _ = self.error_run(
                                    &run.run_id,
                                    "nested_retry_stage",
                                    "nested_retry_stage",
                                    format!(
                                        "Astra decision requested retry_stage after retry: {reason}"
                                    ),
                                )?;
                                return Ok(RustNativeWorkerOutcome::Claimed);
                            }
                            DecisionOutcome::Terminal => {
                                return Ok(RustNativeWorkerOutcome::Claimed)
                            }
                        }
                    }
                    DecisionOutcome::PlanNextRound { reason } => {
                        let discarded = self.discard_remaining_dispatchable_tasks(
                            &run.run_id,
                            &dispatchable,
                            task_index,
                        )?;
                        self.emit(
                            &run,
                            "plan_next_round",
                            json!({
                                "taskId": task.id,
                                "reason": reason,
                                "discardedTaskIds": discarded,
                            }),
                        );
                        continue_rounds = true;
                        break;
                    }
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

    fn plan_astra_round(
        &self,
        run: &AstraRun,
        thread: &crate::models::ThreadInfo,
        prompt: Option<&str>,
        round_index: u32,
    ) -> std::result::Result<(super::AstraPlan, String, Option<serde_json::Value>), BackendFailure>
    {
        let backend_config = self.astra_backend_config();
        let planner_backend: Box<dyn PlannerBackend> = self.create_planner_backend(&backend_config);
        let config_value = json!(backend_config.provider_config);

        match planner_backend.plan(run, thread, prompt, round_index, &config_value) {
            Ok(response) => {
                log::info!(
                    "[astra:planner:success] run={} backend={} sessionId={}",
                    run.run_id,
                    response.backend_type,
                    response.session_id
                );
                Ok((response.data, response.backend_type, None))
            }
            Err(failure) => {
                if failure.code == "policy_denied" {
                    log::warn!(
                        "[astra:planner:policy-denied] run={} backend={} code={}",
                        run.run_id,
                        failure.backend_type,
                        failure.code
                    );
                    return Err(failure);
                }
                if !planner_backend.supports_fallback() {
                    return Err(failure);
                }

                log::warn!(
                    "[astra:planner:fallback] run={} backend={} code={} message={}",
                    run.run_id,
                    failure.backend_type,
                    failure.code,
                    failure.message
                );

                let fallback_diagnostic = internal_failure_diagnostic(
                    "planner_failure",
                    &failure.backend_type,
                    &failure,
                    json!({ "roundIndex": round_index }),
                );

                // Fallback to deterministic
                let deterministic = DeterministicPlannerBackend;
                match deterministic.plan(run, thread, prompt, round_index, &json!({})) {
                    Ok(response) => Ok((
                        response.data,
                        response.backend_type,
                        Some(fallback_diagnostic),
                    )),
                    Err(err) => Err(err), // Should never happen for deterministic
                }
            }
        }
    }

    fn create_planner_backend(&self, config: &AstraBackendConfig) -> Box<dyn PlannerBackend> {
        // If an Astra agent is configured, use it for planning.
        if let Some(agent) = config.agent {
            log::info!(
                "[astra:planner:backend] using runtime_agent backend with agent={}",
                agent.as_str()
            );
            let runtime_config = RuntimeAgentBackendConfig {
                agent,
                timeout_ms: ASTRA_PI_ACP_TIMEOUT_MS,
                model: config.model.clone(),
                effort: config.effort.clone(),
                permission_mode: config.permission_mode.clone(),
            };
            return Box::new(RuntimeAgentPlanner::new(
                self.inner.runtime.clone(),
                runtime_config,
            ));
        }

        // Try Astra Pi ACP if available
        if let Some(astra_pi_acp_config) = self.inner.astra_pi_acp_config.clone() {
            log::info!("[astra:planner:backend] using astra_pi_acp backend");
            return Box::new(AstraPiAcpPlanner::new(astra_pi_acp_config));
        }

        // Default to deterministic
        log::info!("[astra:planner:backend] using deterministic backend");
        Box::new(DeterministicPlannerBackend)
    }

    fn decide_astra_task(
        &self,
        run: &AstraRun,
        thread: &crate::models::ThreadInfo,
        result: &super::AstraTaskResult,
        task: &super::AstraTaskProposal,
    ) -> std::result::Result<(AstraDecision, String, Option<serde_json::Value>), BackendFailure>
    {
        let backend_config = self.astra_backend_config();
        let decision_backend: Box<dyn DecisionBackend> =
            self.create_decision_backend(&backend_config);
        let config_value = json!(backend_config.provider_config);

        match decision_backend.decide(run, thread, result, task, &config_value) {
            Ok(response) => {
                log::info!(
                    "[astra:decision:success] run={} task={} backend={} sessionId={}",
                    run.run_id,
                    task.id,
                    response.backend_type,
                    response.session_id
                );
                Ok((response.data, response.backend_type, None))
            }
            Err(failure) => {
                if failure.code == "policy_denied" {
                    log::warn!(
                        "[astra:decision:policy-denied] run={} task={} backend={} code={}",
                        run.run_id,
                        task.id,
                        failure.backend_type,
                        failure.code
                    );
                    return Err(failure);
                }
                if !decision_backend.supports_fallback() {
                    return Err(failure);
                }

                log::warn!(
                    "[astra:decision:fallback] run={} task={} backend={} code={} message={}",
                    run.run_id,
                    task.id,
                    failure.backend_type,
                    failure.code,
                    failure.message
                );

                let fallback_diagnostic = internal_failure_diagnostic(
                    "decision_failure",
                    &failure.backend_type,
                    &failure,
                    json!({ "taskId": task.id }),
                );

                // Fallback to deterministic
                let deterministic = DeterministicDecisionBackend;
                match deterministic.decide(run, thread, result, task, &json!({})) {
                    Ok(response) => Ok((
                        response.data,
                        response.backend_type,
                        Some(fallback_diagnostic),
                    )),
                    Err(err) => Err(err), // Should never happen for deterministic
                }
            }
        }
    }

    fn create_decision_backend(&self, config: &AstraBackendConfig) -> Box<dyn DecisionBackend> {
        // If an Astra agent is configured, use it for decisions.
        if let Some(agent) = config.agent {
            log::info!(
                "[astra:decision:backend] using runtime_agent backend with agent={}",
                agent.as_str()
            );
            let runtime_config = RuntimeAgentBackendConfig {
                agent,
                timeout_ms: ASTRA_PI_ACP_TIMEOUT_MS,
                model: config.model.clone(),
                effort: config.effort.clone(),
                permission_mode: config.permission_mode.clone(),
            };
            return Box::new(RuntimeAgentDecisionEngine::new(
                self.inner.runtime.clone(),
                runtime_config,
            ));
        }

        // Try Astra Pi ACP if available
        if let Some(astra_pi_acp_config) = self.inner.astra_pi_acp_config.clone() {
            log::info!("[astra:decision:backend] using astra_pi_acp backend");
            return Box::new(AstraPiAcpDecisionEngine::new(astra_pi_acp_config));
        }

        // Default to deterministic
        log::info!("[astra:decision:backend] using deterministic backend");
        Box::new(DeterministicDecisionBackend)
    }

    fn decide_and_apply_task_result(
        &self,
        run: &AstraRun,
        result: &super::AstraTaskResult,
        task: &super::AstraTaskProposal,
    ) -> Result<DecisionOutcome> {
        let latest_thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        let (decision, decision_backend, decision_fallback) =
            match self.decide_astra_task(run, &latest_thread, result, task) {
                Ok(decision) => decision,
                Err(error) => {
                    return self.error_run(
                        &run.run_id,
                        "decision_policy_denied",
                        error.code,
                        error.message,
                    );
                }
            };
        if let Some(fallback) = decision_fallback {
            self.emit(
                run,
                "diagnostic",
                json!({
                    "kind": "decision_fallback",
                    "decisionBackend": decision_backend,
                    "fallback": fallback,
                    "taskId": task.id,
                }),
            );
        }
        let decision_backend_for_run = decision_backend.to_string();
        let _ = self.mutate_run(&run.run_id, |next| {
            if next.status.active() {
                next.decision_backend = Some(decision_backend_for_run);
            }
            Ok(())
        })?;
        self.apply_astra_decision(run, decision)
    }

    fn retry_task_until_settled(
        &self,
        run_id: &str,
        task: &super::AstraTaskProposal,
        mut reason: String,
    ) -> Result<DecisionOutcome> {
        let mut retry_count = 0u32;
        loop {
            let run = self.load_run(run_id)?;
            if !run.status.active() {
                return Ok(DecisionOutcome::Terminal);
            }
            if thread_level_retry_limit_reached(task, retry_count, run.retry_limit) {
                return self.error_run(
                    &run.run_id,
                    "thread_level_retry_limit_reached",
                    "retry_limit_reached",
                    format!(
                        "Astra thread-level task {} requested retry_stage {} time(s)",
                        task.id, retry_count
                    ),
                );
            }
            retry_count += 1;
            self.emit(
                &run,
                "retry_stage",
                json!({
                    "taskId": task.id,
                    "threadStageId": task.target_stage_id,
                    "reason": reason,
                }),
            );
            let retry_result = match self.dispatch_task_and_wait(&run, task) {
                Ok(result) => result,
                Err(error) => {
                    return self.fail_run(&run.run_id, error.to_string());
                }
            };
            let run = self.mark_run_status(
                run_id,
                AstraRunStatus::Thinking,
                retry_result.thread_stage_id.clone(),
                "task_result_received",
            )?;
            if !run.status.active() {
                return Ok(DecisionOutcome::Terminal);
            }
            match self.decide_and_apply_task_result(&run, &retry_result, task)? {
                DecisionOutcome::RetryTask {
                    reason: next_reason,
                } => {
                    reason = next_reason;
                }
                other => return Ok(other),
            }
        }
    }

    fn discard_remaining_dispatchable_tasks(
        &self,
        run_id: &str,
        dispatchable: &[super::AstraTaskProposal],
        current_index: usize,
    ) -> Result<Vec<String>> {
        let discard_ids = remaining_dispatchable_task_ids(dispatchable, current_index);
        if discard_ids.is_empty() {
            return Ok(Vec::new());
        }
        let discarded = discard_ids.iter().cloned().collect::<Vec<_>>();
        let _ = self.mutate_run(run_id, |next| {
            next.proposed_tasks
                .retain(|task| !discard_ids.contains(&task.id));
            Ok(())
        })?;
        Ok(discarded)
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
            AstraDecision::RetryStage { reason } => {
                self.record_latest_task_decision(run, "retry_stage", &reason)?;
                Ok(DecisionOutcome::RetryTask { reason })
            }
            AstraDecision::PlanNextRound { reason } => {
                self.record_latest_task_decision(run, "plan_next_round", &reason)?;
                Ok(DecisionOutcome::PlanNextRound { reason })
            }
            AstraDecision::CancelRun { reason } => self.cancel_run(&run.run_id, reason),
            AstraDecision::CompleteRun { reason } => self.complete_run(&run.run_id, &reason),
            AstraDecision::ErrorRun { reason } => self.fail_run(&run.run_id, reason),
            AstraDecision::Composite { decisions } => {
                for decision in decisions {
                    match self.apply_astra_decision(run, decision)? {
                        DecisionOutcome::Continue => {}
                        other => return Ok(other),
                    }
                }
                Ok(DecisionOutcome::Continue)
            }
        }
    }

    fn record_latest_task_decision(
        &self,
        run: &AstraRun,
        action: &str,
        reason: &str,
    ) -> Result<()> {
        let _ = self.mutate_run(&run.run_id, |next| {
            if let Some(result) = next.task_results.last_mut() {
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
            .filter(|stage| auto_completable_empty_done_stage(stage))
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

fn remaining_dispatchable_task_ids(
    dispatchable: &[super::AstraTaskProposal],
    current_index: usize,
) -> HashSet<String> {
    dispatchable
        .iter()
        .skip(current_index + 1)
        .map(|task| task.id.clone())
        .collect()
}

fn thread_level_retry_limit_reached(
    task: &super::AstraTaskProposal,
    retry_count: u32,
    retry_limit: u32,
) -> bool {
    task.target_stage_id.is_none() && retry_count >= retry_limit
}

fn auto_completable_empty_done_stage(stage: &crate::models::StageInfo) -> bool {
    matches!(stage.kind, Some(StageType::Done))
        && stage.allow_empty_assistants
        && stage.assistant_ids.is_empty()
        && stage.assistants.is_empty()
        && matches!(
            stage.status,
            StageStatus::NotStarted | StageStatus::InProgress
        )
        && !stage
            .issues
            .iter()
            .any(|issue| issue.status == IssueStatus::Open)
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
    fn plan_next_round_discards_remaining_dispatchable_tail() {
        let dispatchable = vec![
            test_task("task-1"),
            test_task("task-2"),
            test_task("task-3"),
        ];

        let remaining = remaining_dispatchable_task_ids(&dispatchable, 0);

        assert!(!remaining.contains("task-1"));
        assert!(remaining.contains("task-2"));
        assert!(remaining.contains("task-3"));
    }

    #[test]
    fn thread_level_retry_guard_stops_at_retry_limit() {
        let task = test_task("thread-task");

        assert!(!thread_level_retry_limit_reached(&task, 2, 3));
        assert!(thread_level_retry_limit_reached(&task, 3, 3));

        let mut stage_task = task;
        stage_task.target_stage_id = Some("stage-1".to_string());
        assert!(!thread_level_retry_limit_reached(&stage_task, 3, 3));
    }

    #[test]
    fn empty_done_stage_without_assistant_is_auto_completable() {
        let mut stage = test_stage("done-stage", StageStatus::NotStarted);
        stage.kind = Some(StageType::Done);
        stage.allow_empty_assistants = true;

        assert!(auto_completable_empty_done_stage(&stage));

        stage.kind = Some(StageType::Human);
        assert!(!auto_completable_empty_done_stage(&stage));

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
        assert!(!auto_completable_empty_done_stage(&stage));
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
            title: id.to_string(),
            target_stage_id: None,
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: super::super::AstraTaskRisk::Low,
        }
    }
}
