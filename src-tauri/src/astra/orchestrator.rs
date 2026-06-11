use anyhow::Result;
use serde_json::json;

use super::{
    astra_task_from_plan_task, is_runtime_placeholder_session_id, next_dispatchable_tasks, now_ms,
    validate_astra_tasks_for_thread, AstraBackendConfig, AstraOrchestration, AstraRun,
    AstraRunIntent, AstraRunStatus, AstraService, AstraTaskCompletion,
    ASTRA_ORCHESTRATOR_TIMEOUT_MS,
};
use crate::astra::astra_pi_acp_adapter::AstraPiAcpOrchestrator;
use crate::astra::backend::{BackendFailure, OrchestratorBackend};
use crate::astra::brainstorm_backend::BrainstormBackend;
use crate::astra::debate_backend::DebateBackend;
use crate::astra::debate_judge::{DebateJudge, HeuristicJudge, RuntimeAgentJudge};
use crate::astra::deterministic_backend::DeterministicOrchestratorBackend;
use crate::astra::runtime_agent_backend::{RuntimeAgentBackendConfig, RuntimeAgentOrchestrator};
use crate::models::{PlanRoundMode, PlanTaskStatus, StageStatus, ThreadInfo, ThreadKind};
use crate::store::SessionStore;

pub(super) const MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS: usize = 50;
const MAX_RUN_DIAGNOSTICS: usize = 100;

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

        let mut current_run = self.load_run(run_id)?;
        if !current_run.status.active() {
            return Ok(RustNativeWorkerOutcome::Claimed);
        }
        let thread = self
            .inner
            .store
            .get_thread_work_state(&current_run.thread_id)?;
        if let Some((reason, code, message)) = dedicated_backend_required_error(&thread) {
            let _ = self.error_run(&current_run.run_id, reason, code, message)?;
            return Ok(RustNativeWorkerOutcome::Claimed);
        }
        let mut dispatch_batch = Vec::new();
        let mut completions = Vec::new();

        loop {
            if dispatch_batch.is_empty() {
                current_run = self.mark_run_status(
                    &current_run.run_id,
                    AstraRunStatus::Planning,
                    None,
                    "planning_round",
                )?;
                if !current_run.status.active() {
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                if round_index >= round_limit {
                    let _ = self.error_run(
                        run_id,
                        "round_limit_reached",
                        "round_limit_reached",
                        "Astra round limit reached before planning the next round".to_string(),
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
                            "orchestrator_backend_failure",
                            error.code,
                            error.message,
                        )?;
                        return Ok(RustNativeWorkerOutcome::Claimed);
                    }
                };
                let (next_run, next_batch) = self.apply_orchestration_intent(
                    &current_run,
                    orchestration,
                    &orchestrator_backend,
                    current_round_index,
                )?;
                current_run = next_run;
                if !current_run.status.active() {
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                dispatch_batch = next_batch;
                completions.clear();
                if dispatch_batch.is_empty() {
                    let _ = self.error_run(
                        &current_run.run_id,
                        "orchestrator_missing_next_tasks",
                        "orchestrator_missing_next_tasks",
                        "Astra Orchestrator returned continue without dispatchable tasks"
                            .to_string(),
                    )?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
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
            completions = results
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
            dispatch_batch = self.next_running_sequential_task_batch(&current_run)?;
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
            self.create_orchestrator_backend(thread, &backend_config);
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
                if let Err(error) =
                    self.record_orchestrator_backend_session(&run.run_id, &response.session_id)
                {
                    return Err(BackendFailure::new(
                        response.backend_type,
                        "diagnostic_write_failed",
                        error.to_string(),
                    )
                    .with_session_id(Some(response.session_id)));
                }
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
                if let Err(error) = self.record_orchestrator_backend_failure(
                    &run.run_id,
                    round_index,
                    completions.len(),
                    &failure,
                ) {
                    log::warn!(
                        "[astra:orchestrator:diagnostic-failure] run={} backend={} code={} message={}",
                        run.run_id,
                        failure.backend_type,
                        failure.code,
                        error
                    );
                }
                Err(failure)
            }
        }
    }

    fn create_orchestrator_backend(
        &self,
        thread: &ThreadInfo,
        config: &AstraBackendConfig,
    ) -> Box<dyn OrchestratorBackend> {
        if thread.kind == ThreadKind::Brainstorm {
            log::info!("[astra:orchestrator:backend] using brainstorm_backend");
            return Box::new(BrainstormBackend);
        }
        if thread.kind == ThreadKind::Debate {
            let judge: Box<dyn DebateJudge> = if let Some(agent) = config.agent {
                log::info!(
                    "[astra:orchestrator:backend] using debate_backend with runtime_agent judge agent={}",
                    agent.as_str()
                );
                Box::new(RuntimeAgentJudge::new(
                    self.inner.runtime.clone(),
                    RuntimeAgentBackendConfig {
                        agent,
                        timeout_ms: ASTRA_ORCHESTRATOR_TIMEOUT_MS,
                        model: config.model.clone(),
                        effort: config.effort.clone(),
                        permission_mode: config.permission_mode.clone(),
                    },
                ))
            } else {
                log::info!("[astra:orchestrator:backend] using debate_backend with heuristic judge");
                Box::new(HeuristicJudge)
            };
            return Box::new(DebateBackend::new(judge));
        }
        if thread.kind == ThreadKind::Process {
            log::info!("[astra:orchestrator:backend] using deterministic backend for process");
            return Box::new(DeterministicOrchestratorBackend);
        }

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

    fn apply_orchestration_intent(
        &self,
        run: &AstraRun,
        orchestration: AstraOrchestration,
        orchestrator_backend: &str,
        round_index: u32,
    ) -> Result<(AstraRun, Vec<super::AstraTaskProposal>)> {
        let mut orchestration = orchestration;
        self.record_orchestration_diagnostics(
            &run.run_id,
            std::mem::take(&mut orchestration.diagnostics),
        )?;
        match orchestration.run_intent {
            AstraRunIntent::Continue => self.apply_orchestration_tasks(
                run,
                orchestration,
                orchestrator_backend,
                round_index,
            ),
            AstraRunIntent::Complete => {
                self.complete_run(&run.run_id, &orchestration.reason)?;
                Ok((self.load_run(&run.run_id)?, Vec::new()))
            }
            AstraRunIntent::WaitForHuman => {
                self.mark_process_manual_checkpoint(run)?;
                self.complete_run(&run.run_id, &orchestration.reason)?;
                Ok((self.load_run(&run.run_id)?, Vec::new()))
            }
            AstraRunIntent::Error => {
                self.error_run(
                    &run.run_id,
                    &orchestration.reason,
                    "orchestrator_error",
                    orchestration.summary,
                )?;
                Ok((self.load_run(&run.run_id)?, Vec::new()))
            }
        }
    }

    fn next_running_sequential_task_batch(
        &self,
        run: &AstraRun,
    ) -> Result<Vec<super::AstraTaskProposal>> {
        let rounds = self.inner.store.list_plan_rounds(&run.thread_id)?;
        for round in rounds
            .iter()
            .filter(|round| round.astra_run_id.as_deref() == Some(run.run_id.as_str()))
            .filter(|round| round.mode == PlanRoundMode::Sequential)
        {
            let Some(running_task) = round
                .tasks
                .iter()
                .find(|task| task.status == PlanTaskStatus::Running)
            else {
                continue;
            };
            return Ok(vec![astra_task_from_plan_task(running_task)]);
        }
        Ok(Vec::new())
    }

    fn apply_orchestration_tasks(
        &self,
        run: &AstraRun,
        orchestration: AstraOrchestration,
        orchestrator_backend: &str,
        round_index: u32,
    ) -> Result<(AstraRun, Vec<super::AstraTaskProposal>)> {
        let mode = orchestration
            .mode
            .ok_or_else(|| anyhow::anyhow!("continue runIntent requires a plan round mode"))?;
        let tasks = orchestration.tasks;
        let summary = orchestration.summary;
        let latest_thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        if let Err(error) = validate_astra_tasks_for_thread(&latest_thread, &tasks) {
            let _ = self.error_run(
                &run.run_id,
                "orchestrator_unsupported_stage_task",
                "orchestrator_unsupported_stage_task",
                error.to_string(),
            )?;
            return Ok((self.load_run(&run.run_id)?, Vec::new()));
        }
        if tasks.is_empty() {
            let _ = self.error_run(
                &run.run_id,
                "orchestrator_missing_next_tasks",
                "orchestrator_missing_next_tasks",
                "Astra Orchestrator returned continue without tasks".to_string(),
            )?;
            return Ok((self.load_run(&run.run_id)?, Vec::new()));
        }
        let tasks = self.create_plan_round_for_astra_tasks(
            run,
            &latest_thread,
            &summary,
            mode,
            round_index,
            tasks,
        )?;
        let orchestrator_backend_for_run = orchestrator_backend.to_string();
        let (planned, ()) = self.mutate_run(&run.run_id, move |next| {
            if !next.status.active() {
                return Ok(());
            }
            next.status = AstraRunStatus::Dispatching;
            next.planner_backend = Some(orchestrator_backend_for_run);
            next.round_index = Some(round_index);
            Ok(())
        })?;
        self.emit(
            &planned,
            "plan",
            json!({
                "summary": summary,
                "runIntent": "continue",
                "reason": orchestration.reason,
                "mode": mode.as_str(),
                "tasks": tasks.clone(),
                "plannerBackend": orchestrator_backend,
                "roundIndex": round_index,
            }),
        );
        let dispatch_batch = match mode {
            PlanRoundMode::Parallel => next_dispatchable_tasks(&tasks, &latest_thread),
            PlanRoundMode::Sequential => tasks.into_iter().take(1).collect(),
        };
        Ok((planned, dispatch_batch))
    }

    fn mark_process_manual_checkpoint(&self, run: &AstraRun) -> Result<()> {
        if mark_process_manual_checkpoint_in_store(self.inner.store.as_ref(), run)? {
            self.emit_threads_updated(run);
        }
        Ok(())
    }

    fn complete_run(&self, run_id: &str, reason: &str) -> Result<()> {
        let (completed, changed) = self.mark_run_completed(run_id, reason)?;
        if changed {
            self.emit(&completed, "completed", json!({ "reason": reason }));
        }
        Ok(())
    }

    fn error_run(&self, run_id: &str, reason: &str, code: &str, message: String) -> Result<()> {
        let (errored, changed) = self.mark_run_errored(run_id, reason, code, message.clone())?;
        if changed {
            self.emit(
                &errored,
                "error",
                json!({ "message": message, "reason": reason, "errorCode": code }),
            );
        }
        Ok(())
    }

    fn fail_run(&self, run_id: &str, message: String) -> Result<()> {
        self.error_run(
            run_id,
            "orchestrator_failure",
            "orchestrator_error",
            message,
        )
    }

    fn record_orchestrator_backend_session(&self, run_id: &str, session_id: &str) -> Result<()> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Ok(());
        }
        let session_id = session_id.to_string();
        self.mutate_run(run_id, move |next| {
            push_internal_planner_session_id(&mut next.internal_planner_session_ids, session_id);
            Ok(())
        })?;
        Ok(())
    }

    fn record_orchestrator_backend_failure(
        &self,
        run_id: &str,
        round_index: u32,
        completion_count: usize,
        failure: &BackendFailure,
    ) -> Result<()> {
        let diagnostic =
            orchestrator_backend_failure_diagnostic(failure, round_index, completion_count);
        let session_id = failure
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self.mutate_run(run_id, move |next| {
            if let Some(session_id) = session_id {
                push_internal_planner_session_id(
                    &mut next.internal_planner_session_ids,
                    session_id,
                );
            }
            next.run_diagnostics.push(diagnostic);
            trim_vec_front(&mut next.run_diagnostics, MAX_RUN_DIAGNOSTICS);
            Ok(())
        })?;
        Ok(())
    }

    fn record_orchestration_diagnostics(
        &self,
        run_id: &str,
        diagnostics: Vec<serde_json::Value>,
    ) -> Result<()> {
        if diagnostics.is_empty() {
            return Ok(());
        }
        self.mutate_run(run_id, move |next| {
            next.run_diagnostics.extend(diagnostics);
            trim_vec_front(&mut next.run_diagnostics, MAX_RUN_DIAGNOSTICS);
            Ok(())
        })?;
        Ok(())
    }

    fn mark_run_status(
        &self,
        run_id: &str,
        status: AstraRunStatus,
        _thread_stage_id: Option<String>,
        reason: &'static str,
    ) -> Result<AstraRun> {
        let (run, changed) = self.mutate_run(run_id, move |next| {
            if !next.status.active() {
                return Ok(false);
            }
            next.status = status;
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
}

pub(super) fn push_unique_bounded(values: &mut Vec<String>, value: String, max_len: usize) {
    if values.iter().any(|existing| existing == &value) {
        return;
    }
    values.push(value);
    trim_vec_front(values, max_len);
}

pub(super) fn push_internal_planner_session_id(values: &mut Vec<String>, value: String) {
    let value = value.trim().to_string();
    if is_runtime_placeholder_session_id(&value) {
        return;
    }
    values.retain(|existing| !is_runtime_placeholder_session_id(existing));
    push_unique_bounded(values, value, MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS);
}

fn orchestrator_backend_failure_diagnostic(
    failure: &BackendFailure,
    round_index: u32,
    completion_count: usize,
) -> serde_json::Value {
    json!({
        "kind": "orchestrator_backend_failure",
        "backend": failure.backend_type,
        "code": failure.code,
        "message": failure.message,
        "sessionId": failure.session_id,
        "roundIndex": round_index,
        "completionCount": completion_count,
        "rawResponseSnippet": failure.raw_response_snippet,
        "recordedAt": now_ms(),
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

pub(super) fn mark_process_manual_checkpoint_in_store(
    store: &dyn SessionStore,
    run: &AstraRun,
) -> Result<bool> {
    let thread = store.get_thread_work_state(&run.thread_id)?;
    if thread.kind != ThreadKind::Process {
        return Ok(false);
    }
    let mut stages = thread
        .stages
        .iter()
        .filter(|stage| stage.enabled)
        .filter(|stage| !matches!(stage.status, StageStatus::Completed | StageStatus::Skipped))
        .collect::<Vec<_>>();
    stages.sort_by_key(|stage| stage.order);
    let Some(stage) = stages.first() else {
        return Ok(false);
    };
    if !stage.assistants.is_empty() {
        return Ok(false);
    }
    store.update_thread_stage_state(&stage.id, Some(StageStatus::NeedsReview), None, None)?;
    Ok(true)
}

pub(super) fn dedicated_backend_required_error(
    thread: &crate::models::ThreadInfo,
) -> Option<(&'static str, &'static str, String)> {
    match thread.kind {
        ThreadKind::Process
        | ThreadKind::Teamwork
        | ThreadKind::Brainstorm
        | ThreadKind::Debate => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::astra::tests::{test_stage, test_thread};
    use crate::models::{Agent, StageStatus};

    #[test]
    fn process_uses_dedicated_deterministic_backend_path() {
        let thread = test_thread(Vec::new());

        assert!(dedicated_backend_required_error(&thread).is_none());
    }

    #[test]
    fn brainstorm_is_allowed_by_dedicated_backend_guard() {
        let mut thread = test_thread(Vec::new());
        thread.kind = ThreadKind::Brainstorm;

        assert!(dedicated_backend_required_error(&thread).is_none());
    }

    #[test]
    fn debate_is_allowed_by_dedicated_backend_guard() {
        let mut thread = test_thread(Vec::new());
        thread.kind = ThreadKind::Debate;

        assert!(dedicated_backend_required_error(&thread).is_none());
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
    fn backend_failure_diagnostic_records_backend_session_and_raw_snippet() {
        let failure = BackendFailure::new("astra_pi_acp", "invalid_yaml", "not valid YAML")
            .with_session_id(Some("planner-session-1".to_string()))
            .with_raw_response("  summary: bad\n```yaml\nnope\n```  ");

        let diagnostic = orchestrator_backend_failure_diagnostic(&failure, 3, 2);

        assert_eq!(diagnostic["kind"], "orchestrator_backend_failure");
        assert_eq!(diagnostic["backend"], "astra_pi_acp");
        assert_eq!(diagnostic["code"], "invalid_yaml");
        assert_eq!(diagnostic["message"], "not valid YAML");
        assert_eq!(diagnostic["sessionId"], "planner-session-1");
        assert_eq!(diagnostic["roundIndex"], 3);
        assert_eq!(diagnostic["completionCount"], 2);
        assert_eq!(
            diagnostic["rawResponseSnippet"],
            "summary: bad\n```yaml\nnope\n```"
        );
        assert!(diagnostic["recordedAt"].as_i64().unwrap() > 0);
    }

    #[test]
    fn bounded_internal_sessions_are_unique_and_keep_recent_entries() {
        let mut values = (0..55)
            .map(|idx| format!("session-{idx}"))
            .collect::<Vec<_>>();

        push_unique_bounded(
            &mut values,
            "session-54".to_string(),
            MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS,
        );
        push_unique_bounded(
            &mut values,
            "session-55".to_string(),
            MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS,
        );

        assert_eq!(values.len(), MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS);
        assert_eq!(values[0], "session-6");
        assert_eq!(values[49], "session-55");
        assert_eq!(
            values
                .iter()
                .filter(|value| value.as_str() == "session-54")
                .count(),
            1
        );
    }

    #[test]
    fn internal_planner_sessions_drop_runtime_placeholders() {
        let mut values = Vec::new();
        push_internal_planner_session_id(&mut values, "runtime-1".to_string());
        push_internal_planner_session_id(&mut values, "fake-agent-session-1".to_string());
        assert!(values.is_empty());

        values.push("runtime-2".to_string());
        values.push("planner-session-old".to_string());
        push_internal_planner_session_id(
            &mut values,
            "deterministic-orchestrator-astra-run-1-0".to_string(),
        );
        push_internal_planner_session_id(&mut values, "planner-session-real".to_string());

        assert_eq!(
            values,
            vec![
                "planner-session-old".to_string(),
                "deterministic-orchestrator-astra-run-1-0".to_string(),
                "planner-session-real".to_string()
            ]
        );
    }

    #[test]
    fn process_stage_tasks_are_not_automatically_dispatchable() {
        let mut research = test_stage("research", StageStatus::NeedsReview);
        let mut plan = test_stage("plan", StageStatus::InProgress);
        research.assistants.push(stage_assistant());
        plan.assistants.push(stage_assistant());
        let tasks = vec![
            super::super::AstraTaskProposal {
                id: "task-plan".to_string(),
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: None,
                title: "Plan".to_string(),
                target_stage_id: Some("plan".to_string()),
                target_agent: Agent::Codex,
                prompt: "Plan next.".to_string(),
                expected_output: "Plan.".to_string(),
                risk: super::super::AstraTaskRisk::Low,
            },
            super::super::AstraTaskProposal {
                id: "task-review".to_string(),
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: None,
                title: "Review".to_string(),
                target_stage_id: Some("research".to_string()),
                target_agent: Agent::Codex,
                prompt: "Review research.".to_string(),
                expected_output: "Review notes.".to_string(),
                risk: super::super::AstraTaskRisk::Medium,
            },
        ];
        let thread = test_thread(vec![research, plan]);

        let dispatchable = next_dispatchable_tasks(&tasks, &thread);

        assert!(dispatchable.is_empty());
    }

    #[test]
    fn process_blocked_stage_tasks_are_not_automatically_dispatchable() {
        let mut blocked = test_stage("blocked", StageStatus::Blocked);
        let mut plan = test_stage("plan", StageStatus::InProgress);
        blocked.assistants.push(stage_assistant());
        plan.assistants.push(stage_assistant());
        let tasks = vec![
            super::super::AstraTaskProposal {
                id: "task-plan".to_string(),
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: None,
                title: "Plan".to_string(),
                target_stage_id: Some("plan".to_string()),
                target_agent: Agent::Codex,
                prompt: "Plan next.".to_string(),
                expected_output: "Plan.".to_string(),
                risk: super::super::AstraTaskRisk::Low,
            },
            super::super::AstraTaskProposal {
                id: "task-blocked".to_string(),
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: None,
                title: "Unblock".to_string(),
                target_stage_id: Some("blocked".to_string()),
                target_agent: Agent::Codex,
                prompt: "Recover blocked stage.".to_string(),
                expected_output: "Recovery notes.".to_string(),
                risk: super::super::AstraTaskRisk::High,
            },
        ];
        let thread = test_thread(vec![blocked, plan]);

        let dispatchable = next_dispatchable_tasks(&tasks, &thread);

        assert!(dispatchable.is_empty());
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
            assistant_id: None,
            agent_participant_id: None,
            title: id.to_string(),
            target_stage_id: None,
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: super::super::AstraTaskRisk::Low,
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
