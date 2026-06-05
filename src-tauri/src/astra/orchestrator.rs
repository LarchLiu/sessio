use std::collections::HashSet;

use anyhow::Result;
use serde_json::json;

use super::{
    next_dispatchable_tasks, thread_all_stages_terminal, thread_waiting_for_review, AstraDecision,
    AstraRun, AstraRunStatus, AstraService,
};
use crate::astra::decision::{AstraDecisionEngine, DeterministicDecisionEngine};
use crate::astra::pi_acp_adapter::{PiAcpDecisionEngine, PiAcpFailure, PiAcpPlanner, PiAcpPurpose};
use crate::astra::planner::{AstraPlanner, DeterministicPlanner};
use crate::models::StageStatus;

const MAX_INTERNAL_PI_SESSION_IDS: usize = 50;
const MAX_RUN_DIAGNOSTICS: usize = 100;

enum DecisionOutcome {
    Continue,
    RetryTask { reason: String },
    PlanNextRound { reason: String },
    Terminal,
}

fn pi_failure_json(kind: &'static str, failure: PiAcpFailure) -> serde_json::Value {
    let mut diagnostic = json!({
        "kind": kind,
        "code": failure.code,
        "message": failure.message,
    });
    if let Some(session_id) = failure.session_id {
        diagnostic["sessionId"] = json!(session_id);
    }
    diagnostic
}

fn internal_pi_diagnostic(
    purpose: PiAcpPurpose,
    kind: &'static str,
    backend: &'static str,
    failure: &PiAcpFailure,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut diagnostic = json!({
        "kind": kind,
        "purpose": purpose.as_str(),
        "backend": backend,
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
                if !self.load_run(run_id)?.status.active() {
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
    ) -> std::result::Result<
        (super::AstraPlan, &'static str, Option<serde_json::Value>),
        PiAcpFailure,
    > {
        if let Some(config) = self.inner.config.pi.clone() {
            let planner = PiAcpPlanner::new(config);
            match planner.plan(run, thread, prompt, round_index) {
                Ok(response) => {
                    if let Err(error) = self.record_internal_pi_session(
                        &run.run_id,
                        PiAcpPurpose::Planning,
                        Some(response.session_id),
                    ) {
                        log::warn!(
                            "[sessio-astra:pi-acp:diagnostic] run={} purpose=planning error={}",
                            run.run_id,
                            error
                        );
                    }
                    return Ok((response.plan, "pi_acp", None));
                }
                Err(error) => {
                    if error.code == "policy_denied" {
                        let session_id = error.session_id.clone();
                        let _ = self.record_internal_pi_failure(
                            &run.run_id,
                            PiAcpPurpose::Planning,
                            "planner_policy_denied",
                            "pi_acp",
                            &error,
                            json!({ "roundIndex": round_index }),
                        );
                        let _ = self.record_internal_pi_session(
                            &run.run_id,
                            PiAcpPurpose::Planning,
                            session_id,
                        );
                        return Err(error);
                    }
                    log::warn!(
                        "[sessio-astra:pi-acp:planner-fallback] run={} code={} message={}",
                        run.run_id,
                        error.code,
                        error.message
                    );
                    let fallback = self
                        .record_internal_pi_failure(
                            &run.run_id,
                            PiAcpPurpose::Planning,
                            "planner_failure",
                            "deterministic",
                            &error,
                            json!({ "roundIndex": round_index }),
                        )
                        .unwrap_or_else(|_| pi_failure_json("planner_failure", error.clone()));
                    let planner = DeterministicPlanner;
                    return Ok((
                        planner.plan(run, thread, prompt, round_index),
                        "deterministic",
                        Some(fallback),
                    ));
                }
            }
        }
        let planner = DeterministicPlanner;
        Ok((
            planner.plan(run, thread, prompt, round_index),
            "deterministic",
            None,
        ))
    }

    fn decide_astra_task(
        &self,
        run: &AstraRun,
        thread: &crate::models::ThreadInfo,
        result: &super::AstraTaskResult,
        task: &super::AstraTaskProposal,
    ) -> std::result::Result<(AstraDecision, &'static str, Option<serde_json::Value>), PiAcpFailure>
    {
        if let Some(config) = self.inner.config.pi.clone() {
            let decision_engine = PiAcpDecisionEngine::new(config);
            match decision_engine.decide(&run.run_id, &run.project_path, thread, result, task) {
                Ok(response) => {
                    if let Err(error) = self.record_internal_pi_session(
                        &run.run_id,
                        PiAcpPurpose::Decision,
                        Some(response.session_id),
                    ) {
                        log::warn!(
                            "[sessio-astra:pi-acp:diagnostic] run={} purpose=decision task={} error={}",
                            run.run_id,
                            task.id,
                            error
                        );
                    }
                    return Ok((response.decision, "pi_acp", None));
                }
                Err(error) => {
                    if error.code == "policy_denied" {
                        let session_id = error.session_id.clone();
                        let _ = self.record_internal_pi_failure(
                            &run.run_id,
                            PiAcpPurpose::Decision,
                            "decision_policy_denied",
                            "pi_acp",
                            &error,
                            json!({ "taskId": task.id }),
                        );
                        let _ = self.record_internal_pi_session(
                            &run.run_id,
                            PiAcpPurpose::Decision,
                            session_id,
                        );
                        return Err(error);
                    }
                    log::warn!(
                        "[sessio-astra:pi-acp:decision-fallback] task={} code={} message={}",
                        task.id,
                        error.code,
                        error.message
                    );
                    let fallback = self
                        .record_internal_pi_failure(
                            &run.run_id,
                            PiAcpPurpose::Decision,
                            "decision_failure",
                            "deterministic",
                            &error,
                            json!({ "taskId": task.id }),
                        )
                        .unwrap_or_else(|_| pi_failure_json("decision_failure", error.clone()));
                    let decision_engine = DeterministicDecisionEngine;
                    return Ok((
                        decision_engine.decide(thread, result, task),
                        "deterministic",
                        Some(fallback),
                    ));
                }
            }
        }
        let decision_engine = DeterministicDecisionEngine;
        Ok((
            decision_engine.decide(thread, result, task),
            "deterministic",
            None,
        ))
    }

    fn record_internal_pi_session(
        &self,
        run_id: &str,
        purpose: PiAcpPurpose,
        session_id: Option<String>,
    ) -> Result<()> {
        let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) else {
            return Ok(());
        };
        let _ = self.mutate_run(run_id, |next| {
            let sessions = match purpose {
                PiAcpPurpose::Planning => &mut next.internal_planner_session_ids,
                PiAcpPurpose::Decision => &mut next.internal_decision_session_ids,
            };
            if !sessions.iter().any(|existing| existing == &session_id) {
                sessions.push(session_id);
                trim_vec_front(sessions, MAX_INTERNAL_PI_SESSION_IDS);
            }
            Ok(())
        })?;
        Ok(())
    }

    fn record_internal_pi_failure(
        &self,
        run_id: &str,
        purpose: PiAcpPurpose,
        kind: &'static str,
        backend: &'static str,
        failure: &PiAcpFailure,
        extra: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.record_internal_pi_session(run_id, purpose, failure.session_id.clone())?;
        let diagnostic = internal_pi_diagnostic(purpose, kind, backend, failure, extra);
        let _ = self.mutate_run(run_id, {
            let diagnostic = diagnostic.clone();
            move |next| {
                next.run_diagnostics.push(diagnostic);
                trim_vec_front(&mut next.run_diagnostics, MAX_RUN_DIAGNOSTICS);
                Ok(())
            }
        })?;
        Ok(diagnostic)
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
            let run = self.load_run(run_id)?;
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
    use crate::models::{Agent, StageStatus};

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

        trim_vec_front(&mut values, MAX_INTERNAL_PI_SESSION_IDS);

        assert_eq!(values.len(), MAX_INTERNAL_PI_SESSION_IDS);
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
