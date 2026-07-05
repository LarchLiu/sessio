use std::collections::{HashMap, HashSet};

use anyhow::Result;
use serde_json::{json, Value};

use super::{
    artifact_roles::built_in_artifact_roles, astra_task_from_plan_task, final_task_output,
    is_runtime_placeholder_session_id, next_dispatchable_tasks, now_ms, summarize_task_output,
    validate_astra_tasks_for_thread, AstraBackendConfig, AstraOrchestration,
    AstraPlannerCanonicalArtifact, AstraPlannerContext, AstraRun, AstraRunIntent, AstraRunStatus,
    AstraService, AstraTaskCompletion, ASTRA_ORCHESTRATOR_TIMEOUT_MS,
};
use crate::astra::backend::{BackendFailure, OrchestratorBackend};
use crate::astra::brainstorm_backend::BrainstormBackend;
use crate::astra::brainstorm_facilitator::{
    BrainstormFacilitator, HeuristicFacilitator, RuntimeAgentFacilitator,
};
use crate::astra::debate_backend::DebateBackend;
use crate::astra::debate_judge::{DebateJudge, HeuristicJudge, RuntimeAgentJudge};
use crate::astra::deterministic_backend::DeterministicOrchestratorBackend;
use crate::astra::runtime_agent_backend::{RuntimeAgentBackendConfig, RuntimeAgentOrchestrator};
use crate::astra::types::{AstraTaskResult, AstraTaskResultStatus};
use crate::models::{
    PlanRoundInfo, PlanRoundMode, PlanRoundSource, PlanTaskInfo, PlanTaskStatus, StageStatus,
    ThreadAstraArtifactInfo, ThreadInfo, ThreadKind,
};
use crate::store::{NewThreadAstraArtifact, SessionStore};

pub(super) const MAX_INTERNAL_ASTRA_PI_ACP_SESSION_IDS: usize = 50;
const MAX_RUN_DIAGNOSTICS: usize = 100;
const THREAD_PROGRESS_DETAILED_ROUND_LIMIT: usize = 8;
const THREAD_PROGRESS_INTERRUPTED_TASK_LIMIT: usize = 24;
const THREAD_PROGRESS_SUMMARY_CHAR_LIMIT: usize = 700;
const THREAD_PROGRESS_TASK_TEXT_CHAR_LIMIT: usize = 900;
const THREAD_PROGRESS_TASK_ERROR_CHAR_LIMIT: usize = 500;
const THREAD_PROGRESS_TASK_PROMPT_CHAR_LIMIT: usize = 1200;

fn trim_vec_front<T>(values: &mut Vec<T>, max_len: usize) {
    if values.len() > max_len {
        values.drain(0..values.len() - max_len);
    }
}

fn astra_thread_round_progress_value(thread_astra_round_count: usize) -> Value {
    json!({
        "roundLimitsDisabled": true,
        "threadAstraRoundCount": thread_astra_round_count,
    })
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
            self.error_run(&current_run.run_id, reason, code, message)?;
            return Ok(RustNativeWorkerOutcome::Claimed);
        }
        let mut dispatch_batch = Vec::new();
        let mut completions = Vec::new();
        // Tasks of the current parallel round whose dependsOn is not yet
        // satisfied; dispatched in waves as their dependencies complete.
        let mut pending_wave_tasks: Vec<super::AstraTaskProposal> = Vec::new();
        // Round index and planner summary of the last dispatched teamwork
        // round, journaled once its completions arrive at the next planning.
        let mut pending_journal: Option<(u32, String)> = None;

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
                let current_round_index = round_index;
                round_index += 1;
                let latest_thread = self
                    .inner
                    .store
                    .get_thread_work_state(&current_run.thread_id)?;
                if latest_thread.kind == ThreadKind::Teamwork && !completions.is_empty() {
                    if let Some((journal_round_index, planner_summary)) = pending_journal.take() {
                        let entry = super::artifacts::teamwork_round_journal_entry(
                            &current_run.run_id,
                            journal_round_index,
                            &planner_summary,
                            &completions,
                            now_ms(),
                        );
                        current_run = self.record_round_journal(&current_run.run_id, entry)?;
                    }
                }
                let (orchestration, orchestrator_backend) = match self.orchestrate_astra_round(
                    &current_run,
                    &latest_thread,
                    prompt.as_deref(),
                    current_round_index,
                    &completions,
                ) {
                    Ok(orchestration) => orchestration,
                    Err(error) => {
                        self.error_run(
                            &current_run.run_id,
                            "orchestrator_backend_failure",
                            error.code,
                            error.message,
                        )?;
                        return Ok(RustNativeWorkerOutcome::Claimed);
                    }
                };
                let plan_summary = orchestration.summary.clone();
                let applied = self.apply_orchestration_intent(
                    &current_run,
                    orchestration,
                    &orchestrator_backend,
                    current_round_index,
                )?;
                current_run = applied.run;
                if !current_run.status.active() {
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
                dispatch_batch = applied.dispatch_batch;
                pending_wave_tasks = applied.deferred;
                if !dispatch_batch.is_empty() {
                    pending_journal = Some((current_round_index, plan_summary));
                }
                completions.clear();
                if dispatch_batch.is_empty() {
                    self.error_run(
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
            let results = match self.dispatch_task_batch_and_wait(
                &current_run,
                &dispatch_batch,
                &completions,
            ) {
                Ok(results) => results,
                Err(error) => {
                    self.fail_run(&current_run.run_id, error.to_string())?;
                    return Ok(RustNativeWorkerOutcome::Claimed);
                }
            };
            let batch_completions = results
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
            // Persist full outputs as workspace artifacts so later rounds can
            // read them on demand. Debate is skipped to preserve lane isolation.
            if matches!(thread.kind, ThreadKind::Teamwork | ThreadKind::Brainstorm) {
                let artifact_paths = super::artifacts::write_task_artifacts(
                    &current_run.project_path,
                    &current_run.run_id,
                    &batch_completions,
                );
                if thread.kind == ThreadKind::Teamwork {
                    self.register_canonical_artifacts_for_completions(
                        &current_run,
                        &batch_completions,
                        &artifact_paths,
                    );
                }
            }
            // Accumulate across sequential batches: the whole round's results
            // must reach the next planning, not just the last task's.
            completions.extend(batch_completions);
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
            if dispatch_batch.is_empty() && !pending_wave_tasks.is_empty() {
                let transition = next_wave(&mut pending_wave_tasks, &completions);
                for (task, reason) in transition.cancelled {
                    let completion =
                        self.cancel_undispatched_wave_task(&current_run, task, &reason);
                    completions.push(completion);
                }
                dispatch_batch = transition.ready;
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
            self.create_orchestrator_backend(thread, &backend_config);
        let config_value = json!({});
        let planner_context = self
            .build_astra_planner_context(run, thread, round_index)
            .map_err(|error| {
                BackendFailure::new(
                    "planner_context",
                    "planner_context_failed",
                    error.to_string(),
                )
            })?;

        match orchestrator_backend.orchestrate(
            run,
            thread,
            prompt,
            round_index,
            completions,
            &planner_context,
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

    fn build_astra_planner_context(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        run_round_index: u32,
    ) -> Result<AstraPlannerContext> {
        let current_artifacts = if thread.kind == ThreadKind::Teamwork {
            self.inner.store.list_current_astra_artifacts(&thread.id)?
        } else {
            Vec::new()
        };
        let canonical_artifacts = current_artifacts
            .iter()
            .map(|artifact| AstraPlannerCanonicalArtifact {
                role: artifact.role.clone(),
                title: artifact.title.clone(),
                path: artifact.path.clone(),
                summary: artifact.summary.clone(),
                source_task_id: artifact.source_task_id.clone(),
                updated_at: artifact.updated_at,
            })
            .collect::<Vec<_>>();
        let (continuation, thread_progress, interrupted_tasks) =
            if thread.kind == ThreadKind::Teamwork {
                self.build_teamwork_continuation_context(
                    run,
                    thread,
                    run_round_index,
                    &current_artifacts,
                )?
            } else {
                (None, None, Vec::new())
            };
        Ok(AstraPlannerContext {
            canonical_artifacts,
            artifact_role_catalog: planner_artifact_role_catalog(thread),
            continuation,
            thread_progress,
            interrupted_tasks,
        })
    }

    fn build_teamwork_continuation_context(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        run_round_index: u32,
        current_artifacts: &[ThreadAstraArtifactInfo],
    ) -> Result<(Option<Value>, Option<Value>, Vec<Value>)> {
        let mut rounds = self.inner.store.list_plan_rounds(&thread.id)?;
        rounds.sort_by_key(|round| (round.round_index, round.created_at));
        let runs = self.inner.store.list_astra_runs(&thread.id)?;
        let run_records_by_id = runs
            .iter()
            .map(|record| (record.run_id.clone(), record))
            .collect::<HashMap<_, _>>();
        let continuation = Some(continuation_context_value(run, &run_records_by_id));
        let current_artifacts_by_role = current_artifacts
            .iter()
            .map(|artifact| (artifact.role.as_str(), artifact))
            .collect::<HashMap<_, _>>();
        let journal_by_run_round = teamwork_journal_by_run_round(&runs);
        let run_round_index_by_round_id = run_round_indices_for_rounds(&rounds);
        let astra_round_count = rounds
            .iter()
            .filter(|round| round.source == PlanRoundSource::Astra)
            .count();
        let detailed_start = rounds
            .len()
            .saturating_sub(THREAD_PROGRESS_DETAILED_ROUND_LIMIT);
        let older_rounds = rounds
            .iter()
            .take(detailed_start)
            .map(compact_thread_round_value)
            .collect::<Vec<_>>();
        let recent_rounds = rounds
            .iter()
            .skip(detailed_start)
            .map(|round| {
                detailed_thread_round_value(
                    round,
                    &run_round_index_by_round_id,
                    &journal_by_run_round,
                    &current_artifacts_by_role,
                )
            })
            .collect::<Vec<_>>();
        let thread_progress = Some(json!({
            "runRoundIndex": run_round_index,
            "roundProgress": astra_thread_round_progress_value(astra_round_count),
            "olderRounds": older_rounds,
            "recentRounds": recent_rounds,
        }));
        let interrupted_tasks = interrupted_thread_tasks(
            &rounds,
            &run_round_index_by_round_id,
            &current_artifacts_by_role,
        );
        Ok((continuation, thread_progress, interrupted_tasks))
    }

    fn register_canonical_artifacts_for_completions(
        &self,
        run: &AstraRun,
        completions: &[AstraTaskCompletion],
        artifact_paths: &HashMap<String, String>,
    ) {
        for completion in completions {
            if completion.result.status != AstraTaskResultStatus::Completed {
                continue;
            }
            let Some(role) = completion.task.artifact_role.as_deref() else {
                continue;
            };
            let Some(plan_task_id) = completion.task.plan_task_id.as_deref() else {
                log::warn!(
                    "[astra:artifacts] skipped canonical artifact without plan task id task={}",
                    completion.task.id
                );
                continue;
            };
            let Some(path) = artifact_paths.get(&completion.task.id) else {
                log::warn!(
                    "[astra:artifacts] skipped canonical artifact without written path task={}",
                    completion.task.id
                );
                continue;
            };
            let summary = self.canonical_artifact_summary(run, completion);
            if let Err(error) =
                self.inner
                    .store
                    .register_current_astra_artifact(NewThreadAstraArtifact {
                        thread_id: &run.thread_id,
                        astra_run_id: &run.run_id,
                        source_task_id: plan_task_id,
                        role,
                        title: &completion.task.title,
                        path,
                        summary: &summary,
                    })
            {
                log::warn!(
                    "[astra:artifacts] failed to register canonical artifact task={} role={role}: {error}",
                    completion.task.id
                );
            }
        }
    }

    fn canonical_artifact_summary(
        &self,
        run: &AstraRun,
        completion: &AstraTaskCompletion,
    ) -> String {
        let persisted_summary = self
            .load_astra_plan_task(&run.thread_id, completion.task.plan_task_id.as_deref())
            .ok()
            .flatten()
            .and_then(|task| task.result_summary)
            .map(|summary| summary.trim().to_string())
            .filter(|summary| !summary.is_empty());
        let summary = persisted_summary.unwrap_or_else(|| {
            summarize_task_output(&final_task_output(&completion.result.output))
        });
        let char_count = summary.chars().count();
        let mut truncated = if char_count > 600 {
            summary.chars().take(597).collect::<String>()
        } else {
            summary
        };
        if char_count > 600 {
            truncated.push_str("...");
        }
        if truncated.trim().is_empty() {
            "Astra delegated task completed.".to_string()
        } else {
            truncated
        }
    }

    fn create_orchestrator_backend(
        &self,
        thread: &ThreadInfo,
        config: &AstraBackendConfig,
    ) -> Box<dyn OrchestratorBackend> {
        if thread.kind == ThreadKind::Brainstorm {
            let facilitator: Box<dyn BrainstormFacilitator> = if let Some(agent) = config.agent {
                log::info!(
                    "[astra:orchestrator:backend] using brainstorm_backend with runtime_agent facilitator agent={}",
                    agent.as_str()
                );
                Box::new(RuntimeAgentFacilitator::new(
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
                log::info!(
                    "[astra:orchestrator:backend] using brainstorm_backend with heuristic facilitator"
                );
                Box::new(HeuristicFacilitator)
            };
            return Box::new(BrainstormBackend::new(facilitator));
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
                log::info!(
                    "[astra:orchestrator:backend] using debate_backend with heuristic judge"
                );
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

        log::info!("[astra:orchestrator:backend] using deterministic backend");
        Box::new(DeterministicOrchestratorBackend)
    }

    fn apply_orchestration_intent(
        &self,
        run: &AstraRun,
        orchestration: AstraOrchestration,
        orchestrator_backend: &str,
        round_index: u32,
    ) -> Result<AppliedOrchestration> {
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
                Ok(AppliedOrchestration::terminal(self.load_run(&run.run_id)?))
            }
            AstraRunIntent::WaitForHuman => {
                self.mark_process_manual_checkpoint(run)?;
                self.complete_run(&run.run_id, &orchestration.reason)?;
                Ok(AppliedOrchestration::terminal(self.load_run(&run.run_id)?))
            }
            AstraRunIntent::Error => {
                self.error_run(
                    &run.run_id,
                    &orchestration.reason,
                    "orchestrator_error",
                    orchestration.summary,
                )?;
                Ok(AppliedOrchestration::terminal(self.load_run(&run.run_id)?))
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

    /// Marks a never-dispatched wave task as cancelled in the plan tables and
    /// synthesizes its completion so the planner and round journal see why it
    /// was skipped. Bypassing `finish_delegated_task` is safe here: a task
    /// that was never dispatched has no delegated session, waiter, or runtime
    /// resource to release.
    fn cancel_undispatched_wave_task(
        &self,
        run: &AstraRun,
        task: super::AstraTaskProposal,
        reason: &str,
    ) -> AstraTaskCompletion {
        let result = AstraTaskResult {
            task_id: task.id.clone(),
            thread_stage_id: None,
            sessio_runtime_session_id: String::new(),
            turn_id: None,
            status: AstraTaskResultStatus::Cancelled,
            output: String::new(),
            error: Some(reason.to_string()),
            attempt_count: 0,
            retry_limit_reached: false,
            completed_at: now_ms(),
        };
        if let Err(error) = self.record_task_result(&run.run_id, result.clone()) {
            log::warn!(
                "[astra:task:wave-cancel-record-failed] runId={} taskId={} message={}",
                run.run_id,
                task.id,
                error
            );
        }
        match serde_json::to_value(&result) {
            Ok(value) => self.emit(run, "task_result", value),
            Err(error) => log::warn!(
                "[astra:task:wave-cancel-emit-failed] runId={} taskId={} message={}",
                run.run_id,
                task.id,
                error
            ),
        }
        AstraTaskCompletion { task, result }
    }

    fn apply_orchestration_tasks(
        &self,
        run: &AstraRun,
        orchestration: AstraOrchestration,
        orchestrator_backend: &str,
        round_index: u32,
    ) -> Result<AppliedOrchestration> {
        let mode = orchestration
            .mode
            .ok_or_else(|| anyhow::anyhow!("continue runIntent requires a plan round mode"))?;
        let tasks = orchestration.tasks;
        let summary = orchestration.summary;
        let latest_thread = self.inner.store.get_thread_work_state(&run.thread_id)?;
        if let Err(error) = validate_astra_tasks_for_thread(&latest_thread, &tasks) {
            self.error_run(
                &run.run_id,
                "orchestrator_unsupported_stage_task",
                "orchestrator_unsupported_stage_task",
                error.to_string(),
            )?;
            return Ok(AppliedOrchestration::terminal(self.load_run(&run.run_id)?));
        }
        if tasks.is_empty() {
            self.error_run(
                &run.run_id,
                "orchestrator_missing_next_tasks",
                "orchestrator_missing_next_tasks",
                "Astra Orchestrator returned continue without tasks".to_string(),
            )?;
            return Ok(AppliedOrchestration::terminal(self.load_run(&run.run_id)?));
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
        let (dispatch_batch, deferred) = match mode {
            PlanRoundMode::Parallel => next_dispatchable_tasks(&tasks, &latest_thread)
                .into_iter()
                .partition(|task| task.depends_on.is_empty()),
            PlanRoundMode::Sequential => (tasks.into_iter().take(1).collect(), Vec::new()),
        };
        Ok(AppliedOrchestration {
            run: planned,
            dispatch_batch,
            deferred,
        })
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
            self.reconcile_terminal_plan_work_best_effort(&errored);
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

    fn record_round_journal(&self, run_id: &str, entry: serde_json::Value) -> Result<AstraRun> {
        let (run, ()) = self.mutate_run(run_id, move |next| {
            next.run_diagnostics.push(entry);
            trim_vec_front(&mut next.run_diagnostics, MAX_RUN_DIAGNOSTICS);
            Ok(())
        })?;
        Ok(run)
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

struct AppliedOrchestration {
    run: AstraRun,
    dispatch_batch: Vec<super::AstraTaskProposal>,
    deferred: Vec<super::AstraTaskProposal>,
}

impl AppliedOrchestration {
    fn terminal(run: AstraRun) -> Self {
        Self {
            run,
            dispatch_batch: Vec::new(),
            deferred: Vec::new(),
        }
    }
}

struct WaveTransition {
    ready: Vec<super::AstraTaskProposal>,
    cancelled: Vec<(super::AstraTaskProposal, String)>,
}

/// Computes the next wave of a dependsOn round. Tasks whose dependencies all
/// completed move to `ready`; tasks blocked by a failed, errored, or cancelled
/// dependency (directly or transitively) move to `cancelled`. When nothing can
/// ever become ready (a dependency id that was never dispatched), the whole
/// remaining pool is cancelled so the orchestration loop always progresses
/// instead of spinning on empty batches.
fn next_wave(
    pending: &mut Vec<super::AstraTaskProposal>,
    completions: &[AstraTaskCompletion],
) -> WaveTransition {
    let mut completed: HashSet<String> = HashSet::new();
    let mut failed: HashMap<String, String> = HashMap::new();
    for completion in completions {
        if completion.result.status == AstraTaskResultStatus::Completed {
            completed.insert(completion.task.id.clone());
        } else {
            failed.insert(completion.task.id.clone(), completion.task.title.clone());
        }
    }

    let mut ready = Vec::new();
    let mut cancelled: Vec<(super::AstraTaskProposal, String)> = Vec::new();
    loop {
        let mut progressed = false;
        let mut index = 0;
        while index < pending.len() {
            let task = &pending[index];
            if let Some(blocked_title) = task
                .depends_on
                .iter()
                .find_map(|dep| failed.get(dep))
                .cloned()
            {
                let task = pending.remove(index);
                failed.insert(task.id.clone(), task.title.clone());
                cancelled.push((
                    task,
                    format!("dependency task \"{blocked_title}\" did not complete"),
                ));
                progressed = true;
                continue;
            }
            if task.depends_on.iter().all(|dep| completed.contains(dep)) {
                ready.push(pending.remove(index));
                progressed = true;
                continue;
            }
            index += 1;
        }
        if !progressed {
            break;
        }
    }

    if ready.is_empty() && cancelled.is_empty() && !pending.is_empty() {
        for task in pending.drain(..) {
            let missing_dep = task
                .depends_on
                .iter()
                .find(|dep| !completed.contains(*dep))
                .cloned()
                .unwrap_or_default();
            cancelled.push((
                task,
                format!("dependency task \"{missing_dep}\" was never dispatched in this round"),
            ));
        }
    }

    WaveTransition { ready, cancelled }
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

fn planner_artifact_role_catalog(thread: &ThreadInfo) -> Vec<String> {
    let mut artifact_role_catalog = built_in_artifact_roles();
    for role in &thread.artifact_role_catalog {
        if !artifact_role_catalog
            .iter()
            .any(|existing| existing == role)
        {
            artifact_role_catalog.push(role.clone());
        }
    }
    artifact_role_catalog
}

fn continuation_context_value(
    run: &AstraRun,
    run_records_by_id: &HashMap<String, &crate::store::AstraRunRecord>,
) -> Value {
    let interrupted_run = run
        .continued_from_run_id
        .as_ref()
        .and_then(|run_id| run_records_by_id.get(run_id))
        .map(|record| {
            json!({
                "runId": record.run_id,
                "status": record.status,
                "terminalReason": record.terminal_reason,
                "lastErrorCode": record.last_error_code,
                "lastErrorMessage": record.last_error_message,
                "error": record.error,
                "updatedAt": record.updated_at,
            })
        });
    json!({
        "continuedFromRunId": run.continued_from_run_id.as_deref(),
        "isContinuation": run.continued_from_run_id.is_some(),
        "interruptedRun": interrupted_run,
    })
}

fn teamwork_journal_by_run_round(
    runs: &[crate::store::AstraRunRecord],
) -> HashMap<(String, u32), Value> {
    let mut journal = HashMap::new();
    for run in runs {
        let Ok(diagnostics) = serde_json::from_str::<Vec<Value>>(&run.run_diagnostics_json) else {
            continue;
        };
        for diagnostic in diagnostics {
            if diagnostic.get("kind").and_then(Value::as_str)
                != Some(super::artifacts::TEAMWORK_ROUND_JOURNAL_KIND)
            {
                continue;
            }
            let Some(round_index) = diagnostic
                .get("roundIndex")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
            else {
                continue;
            };
            journal.insert((run.run_id.clone(), round_index), diagnostic);
        }
    }
    journal
}

fn run_round_indices_for_rounds(rounds: &[PlanRoundInfo]) -> HashMap<String, u32> {
    let mut counters = HashMap::<String, u32>::new();
    let mut indices = HashMap::new();
    for round in rounds {
        let Some(run_id) = round.astra_run_id.as_ref() else {
            continue;
        };
        let index = counters.entry(run_id.clone()).or_insert(0);
        indices.insert(round.id.clone(), *index);
        *index = index.saturating_add(1);
    }
    indices
}

fn compact_thread_round_value(round: &PlanRoundInfo) -> Value {
    json!({
        "id": round.id,
        "astraRunId": round.astra_run_id,
        "threadRoundIndex": round.round_index,
        "mode": round.mode.as_str(),
        "source": round.source.as_str(),
        "status": round.status.as_str(),
        "summary": round.summary.as_deref().map(|summary| {
            super::structured_response::truncate_chars(summary, THREAD_PROGRESS_SUMMARY_CHAR_LIMIT)
        }),
        "taskCount": round.tasks.len(),
        "taskStatusCounts": task_status_counts(&round.tasks),
    })
}

fn detailed_thread_round_value(
    round: &PlanRoundInfo,
    run_round_index_by_round_id: &HashMap<String, u32>,
    journal_by_run_round: &HashMap<(String, u32), Value>,
    current_artifacts_by_role: &HashMap<&str, &ThreadAstraArtifactInfo>,
) -> Value {
    let run_round_index = run_round_index_by_round_id.get(&round.id).copied();
    let journal =
        round
            .astra_run_id
            .as_ref()
            .zip(run_round_index)
            .and_then(|(run_id, run_round_index)| {
                journal_by_run_round.get(&(run_id.clone(), run_round_index))
            });
    let journal_tasks = journal
        .and_then(|value| value.get("tasks"))
        .and_then(Value::as_array);
    let tasks = round
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            let journal_task = journal_tasks.and_then(|tasks| tasks.get(index));
            thread_task_value(round, task, journal_task, current_artifacts_by_role, false)
        })
        .collect::<Vec<_>>();
    json!({
        "id": round.id,
        "astraRunId": round.astra_run_id,
        "threadRoundIndex": round.round_index,
        "runRoundIndex": run_round_index,
        "mode": round.mode.as_str(),
        "source": round.source.as_str(),
        "status": round.status.as_str(),
        "summary": journal
            .and_then(|value| value.get("plannerSummary"))
            .and_then(Value::as_str)
            .or(round.summary.as_deref())
            .map(|summary| {
                super::structured_response::truncate_chars(summary, THREAD_PROGRESS_SUMMARY_CHAR_LIMIT)
            }),
        "taskStatusCounts": task_status_counts(&round.tasks),
        "tasks": tasks,
        "createdAt": round.created_at,
        "updatedAt": round.updated_at,
    })
}

fn thread_task_value(
    round: &PlanRoundInfo,
    task: &PlanTaskInfo,
    journal_task: Option<&Value>,
    current_artifacts_by_role: &HashMap<&str, &ThreadAstraArtifactInfo>,
    include_prompt: bool,
) -> Value {
    let mut value = json!({
        "id": task.id,
        "roundId": round.id,
        "threadRoundIndex": round.round_index,
        "astraRunId": round.astra_run_id,
        "title": super::structured_response::truncate_chars(&task.title, THREAD_PROGRESS_TASK_TEXT_CHAR_LIMIT),
        "status": task.status.as_str(),
        "resultSummary": task.result_summary.as_deref().map(|summary| {
            super::structured_response::truncate_chars(summary, THREAD_PROGRESS_TASK_TEXT_CHAR_LIMIT)
        }),
        "error": task.error.as_deref().map(|error| {
            super::structured_response::truncate_chars(error, THREAD_PROGRESS_TASK_ERROR_CHAR_LIMIT)
        }),
        "risk": task.risk.as_str(),
        "assistantId": task.assistant_id,
        "agentParticipantId": task.agent_participant_id,
        "targetAgent": task.target_agent.as_str(),
        "artifactRole": task.artifact_role,
        "usesArtifactRoles": task.uses_artifact_roles,
        "sortOrder": task.sort_order,
        "startedAt": task.started_at,
        "completedAt": task.completed_at,
        "createdAt": task.created_at,
        "updatedAt": task.updated_at,
    });
    let Some(record) = value.as_object_mut() else {
        return value;
    };
    if include_prompt {
        record.insert(
            "prompt".to_string(),
            json!(super::structured_response::truncate_chars(
                &task.prompt,
                THREAD_PROGRESS_TASK_PROMPT_CHAR_LIMIT,
            )),
        );
    }
    if let Some(excerpt) = journal_task
        .and_then(|task| task.get("outputExcerpt"))
        .and_then(Value::as_str)
        .map(|excerpt| {
            super::structured_response::truncate_chars(
                excerpt,
                THREAD_PROGRESS_TASK_TEXT_CHAR_LIMIT,
            )
        })
    {
        record.insert("outputExcerpt".to_string(), json!(excerpt));
    }
    if let Some(path) =
        ordinary_task_artifact_path(round, task, journal_task, current_artifacts_by_role)
    {
        record.insert("artifactPath".to_string(), json!(path));
    }
    value
}

fn ordinary_task_artifact_path(
    round: &PlanRoundInfo,
    task: &PlanTaskInfo,
    journal_task: Option<&Value>,
    current_artifacts_by_role: &HashMap<&str, &ThreadAstraArtifactInfo>,
) -> Option<String> {
    if task.status != PlanTaskStatus::Completed {
        return None;
    }
    if task
        .artifact_role
        .as_deref()
        .and_then(|role| current_artifacts_by_role.get(role))
        .is_some()
    {
        return None;
    }
    journal_task
        .and_then(|task| task.get("outputPath"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            round.astra_run_id.as_deref().map(|run_id| {
                super::artifacts::task_artifact_relative_path(run_id, &task.id, &task.title)
            })
        })
}

fn interrupted_thread_tasks(
    rounds: &[PlanRoundInfo],
    run_round_index_by_round_id: &HashMap<String, u32>,
    current_artifacts_by_role: &HashMap<&str, &ThreadAstraArtifactInfo>,
) -> Vec<Value> {
    let mut latest_completed_by_key = HashMap::<String, i64>::new();
    for round in rounds {
        for task in &round.tasks {
            if task.status != PlanTaskStatus::Completed {
                continue;
            }
            latest_completed_by_key
                .entry(task_replacement_key(task))
                .and_modify(|round_index| *round_index = (*round_index).max(round.round_index))
                .or_insert(round.round_index);
        }
    }

    let mut values = Vec::new();
    for round in rounds {
        let run_round_index = run_round_index_by_round_id.get(&round.id).copied();
        for task in &round.tasks {
            if !matches!(
                task.status,
                PlanTaskStatus::Planned
                    | PlanTaskStatus::Running
                    | PlanTaskStatus::Failed
                    | PlanTaskStatus::Errored
                    | PlanTaskStatus::Cancelled
            ) {
                continue;
            }
            if latest_completed_by_key
                .get(&task_replacement_key(task))
                .is_some_and(|completed_round| *completed_round > round.round_index)
            {
                continue;
            }
            let mut value = thread_task_value(round, task, None, current_artifacts_by_role, true);
            if let Some(record) = value.as_object_mut() {
                record.insert("runRoundIndex".to_string(), json!(run_round_index));
            }
            values.push(value);
            if values.len() >= THREAD_PROGRESS_INTERRUPTED_TASK_LIMIT {
                return values;
            }
        }
    }
    values
}

fn task_status_counts(tasks: &[PlanTaskInfo]) -> Value {
    let mut counts = serde_json::Map::new();
    for task in tasks {
        let key = task.status.as_str().to_string();
        let current = counts.get(&key).and_then(Value::as_u64).unwrap_or(0);
        counts.insert(key, json!(current + 1));
    }
    Value::Object(counts)
}

fn task_replacement_key(task: &PlanTaskInfo) -> String {
    format!(
        "{}\0{}\0{}\0{}\0{}",
        task.title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase(),
        task.assistant_id.as_deref().unwrap_or(""),
        task.agent_participant_id.as_deref().unwrap_or(""),
        task.target_agent.as_str(),
        task.artifact_role.as_deref().unwrap_or(""),
    )
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
    fn planner_artifact_role_catalog_merges_thread_custom_roles() {
        let mut thread = test_thread(Vec::new());
        thread.artifact_role_catalog = vec![
            "story_bible".to_string(),
            "outline".to_string(),
            "character_sheet".to_string(),
        ];

        let catalog = planner_artifact_role_catalog(&thread);

        assert!(catalog.starts_with(&built_in_artifact_roles()));
        assert_eq!(
            catalog,
            vec![
                "plan",
                "outline",
                "research_brief",
                "draft",
                "synthesis",
                "story_bible",
                "character_sheet",
            ]
        );
    }

    #[test]
    fn thread_round_progress_reports_counts_without_limits() {
        let progress = astra_thread_round_progress_value(125);

        assert_eq!(progress["roundLimitsDisabled"].as_bool(), Some(true));
        assert_eq!(progress["threadAstraRoundCount"].as_u64(), Some(125));
        assert!(progress.get("roundLimit").is_none());
        assert!(progress.get("runRoundLimit").is_none());
        assert!(progress.get("threadRoundLimit").is_none());
        assert!(progress.get("remainingAutomaticRounds").is_none());
    }

    #[test]
    fn thread_progress_round_value_includes_artifact_paths_and_round_indices() {
        let round = PlanRoundInfo {
            id: "round-1".to_string(),
            thread_id: "thread-1".to_string(),
            astra_run_id: Some("run-old".to_string()),
            round_index: 7,
            summary: Some("persisted summary".to_string()),
            mode: PlanRoundMode::Parallel,
            source: PlanRoundSource::Astra,
            status: crate::models::PlanRoundStatus::Completed,
            created_at: 10,
            updated_at: 20,
            tasks: vec![PlanTaskInfo {
                id: "task-done".to_string(),
                round_id: "round-1".to_string(),
                thread_stage_id: None,
                assistant_id: Some("assistant-codex".to_string()),
                agent_participant_id: None,
                target_agent: Agent::Codex,
                stage_snapshot_json: None,
                assistant_snapshot_json: None,
                agent_snapshot_json: r#"{"agent":"codex"}"#.to_string(),
                title: "完成需求分析".to_string(),
                prompt: "Analyse requirements".to_string(),
                expected_output: Some("Requirements".to_string()),
                artifact_role: None,
                uses_artifact_roles: Vec::new(),
                risk: crate::models::PlanTaskRisk::Low,
                sort_order: 0,
                status: PlanTaskStatus::Completed,
                result_summary: Some("需求已明确".to_string()),
                error: None,
                started_at: Some(11),
                completed_at: Some(12),
                created_at: 10,
                updated_at: 20,
                sessions: Vec::new(),
            }],
        };
        let mut run_round_indices = HashMap::new();
        run_round_indices.insert("round-1".to_string(), 2);
        let mut journals = HashMap::new();
        journals.insert(
            ("run-old".to_string(), 2),
            json!({
                "plannerSummary": "journal summary",
                "tasks": [{
                    "outputExcerpt": "journal excerpt",
                    "outputPath": ".sessio/astra/run-old/tasks/完成需求分析--task-done.md",
                }],
            }),
        );
        let current_artifacts = HashMap::new();

        let value =
            detailed_thread_round_value(&round, &run_round_indices, &journals, &current_artifacts);

        assert_eq!(value["threadRoundIndex"], 7);
        assert_eq!(value["runRoundIndex"], 2);
        assert_eq!(value["summary"], "journal summary");
        assert_eq!(value["tasks"][0]["outputExcerpt"], "journal excerpt");
        assert_eq!(
            value["tasks"][0]["artifactPath"],
            ".sessio/astra/run-old/tasks/完成需求分析--task-done.md"
        );
    }

    #[test]
    fn thread_progress_omits_ordinary_artifact_path_when_current_canonical_exists() {
        let round = PlanRoundInfo {
            id: "round-1".to_string(),
            thread_id: "thread-1".to_string(),
            astra_run_id: Some("run-old".to_string()),
            round_index: 7,
            summary: Some("persisted summary".to_string()),
            mode: PlanRoundMode::Parallel,
            source: PlanRoundSource::Astra,
            status: crate::models::PlanRoundStatus::Completed,
            created_at: 10,
            updated_at: 20,
            tasks: vec![PlanTaskInfo {
                id: "task-outline".to_string(),
                round_id: "round-1".to_string(),
                thread_stage_id: None,
                assistant_id: Some("assistant-codex".to_string()),
                agent_participant_id: None,
                target_agent: Agent::Codex,
                stage_snapshot_json: None,
                assistant_snapshot_json: None,
                agent_snapshot_json: r#"{"agent":"codex"}"#.to_string(),
                title: "更新大纲".to_string(),
                prompt: "Update outline".to_string(),
                expected_output: Some("Outline".to_string()),
                artifact_role: Some("outline".to_string()),
                uses_artifact_roles: Vec::new(),
                risk: crate::models::PlanTaskRisk::Low,
                sort_order: 0,
                status: PlanTaskStatus::Completed,
                result_summary: Some("大纲已更新".to_string()),
                error: None,
                started_at: Some(11),
                completed_at: Some(12),
                created_at: 10,
                updated_at: 20,
                sessions: Vec::new(),
            }],
        };
        let artifact = ThreadAstraArtifactInfo {
            id: "artifact-outline".to_string(),
            thread_id: "thread-1".to_string(),
            astra_run_id: "run-latest".to_string(),
            source_task_id: "task-outline-latest".to_string(),
            role: "outline".to_string(),
            title: "Current outline".to_string(),
            path: ".sessio/astra/run-latest/tasks/current-outline--task-outline-latest.md"
                .to_string(),
            summary: "Canonical outline summary".to_string(),
            is_current: true,
            created_at: 30,
            updated_at: 40,
            superseded_at: None,
        };
        let mut current_artifacts = HashMap::new();
        current_artifacts.insert(artifact.role.as_str(), &artifact);
        let run_round_indices = HashMap::new();
        let journals = HashMap::new();

        let value =
            detailed_thread_round_value(&round, &run_round_indices, &journals, &current_artifacts);

        assert!(value["tasks"][0].get("artifactPath").is_none());
    }

    #[test]
    fn rolling_batch_discards_only_undispatched_tasks() {
        let dispatchable = [
            test_task("task-1"),
            test_task("task-2"),
            test_task("task-3"),
        ];

        let dispatched = [test_task("task-1"), test_task("task-2")];
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
        let failure = BackendFailure::new("runtime_agent_pi", "invalid_yaml", "not valid YAML")
            .with_session_id(Some("planner-session-1".to_string()))
            .with_raw_response("  summary: bad\n```yaml\nnope\n```  ");

        let diagnostic = orchestrator_backend_failure_diagnostic(&failure, 3, 2);

        assert_eq!(diagnostic["kind"], "orchestrator_backend_failure");
        assert_eq!(diagnostic["backend"], "runtime_agent_pi");
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
                depends_on: Vec::new(),
                artifact_role: None,
                uses_artifact_roles: Vec::new(),
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
                depends_on: Vec::new(),
                artifact_role: None,
                uses_artifact_roles: Vec::new(),
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
                depends_on: Vec::new(),
                artifact_role: None,
                uses_artifact_roles: Vec::new(),
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
                depends_on: Vec::new(),
                artifact_role: None,
                uses_artifact_roles: Vec::new(),
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
            depends_on: Vec::new(),
            artifact_role: None,
            uses_artifact_roles: Vec::new(),
        }
    }

    fn dep_task(id: &str, deps: &[&str]) -> super::super::AstraTaskProposal {
        let mut task = test_task(id);
        task.depends_on = deps.iter().map(ToString::to_string).collect();
        task
    }

    fn wave_completion(id: &str, status: AstraTaskResultStatus) -> AstraTaskCompletion {
        AstraTaskCompletion {
            task: test_task(id),
            result: AstraTaskResult {
                task_id: id.to_string(),
                thread_stage_id: None,
                sessio_runtime_session_id: "session".to_string(),
                turn_id: None,
                status,
                output: String::new(),
                error: None,
                attempt_count: 1,
                retry_limit_reached: false,
                completed_at: 1,
            },
        }
    }

    #[test]
    fn parallel_first_wave_partitions_by_empty_depends_on() {
        let tasks = vec![
            test_task("a"),
            dep_task("b", &["a"]),
            test_task("c"),
            dep_task("d", &["b", "c"]),
        ];

        let (ready, deferred): (Vec<_>, Vec<_>) = tasks
            .into_iter()
            .partition(|task| task.depends_on.is_empty());

        assert_eq!(
            ready
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "c"]
        );
        assert_eq!(
            deferred
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "d"]
        );
    }

    #[test]
    fn next_wave_unlocks_tasks_whose_deps_completed_and_keeps_waiting_ones() {
        let mut pending = vec![dep_task("b", &["a"]), dep_task("c", &["a", "b"])];

        let first = next_wave(
            &mut pending,
            &[wave_completion("a", AstraTaskResultStatus::Completed)],
        );
        assert_eq!(
            first
                .ready
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );
        assert!(first.cancelled.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "c");

        let second = next_wave(
            &mut pending,
            &[
                wave_completion("a", AstraTaskResultStatus::Completed),
                wave_completion("b", AstraTaskResultStatus::Completed),
            ],
        );
        assert_eq!(
            second
                .ready
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c"]
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn next_wave_cancels_dependents_of_failed_deps_recursively() {
        let mut pending = vec![
            dep_task("b", &["a"]),
            dep_task("c", &["b"]),
            dep_task("d", &["ok"]),
        ];

        let transition = next_wave(
            &mut pending,
            &[
                wave_completion("a", AstraTaskResultStatus::Failed),
                wave_completion("ok", AstraTaskResultStatus::Completed),
            ],
        );

        assert_eq!(
            transition
                .ready
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["d"]
        );
        let cancelled = transition
            .cancelled
            .iter()
            .map(|(task, reason)| (task.id.as_str(), reason.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(cancelled.len(), 2);
        assert_eq!(cancelled[0].0, "b");
        assert!(cancelled[0].1.contains("\"a\" did not complete"));
        assert_eq!(cancelled[1].0, "c");
        assert!(cancelled[1].1.contains("\"b\" did not complete"));
        assert!(pending.is_empty());
    }

    #[test]
    fn next_wave_cascades_from_cancelled_dependencies_too() {
        let mut pending = vec![dep_task("b", &["a"])];

        let transition = next_wave(
            &mut pending,
            &[wave_completion("a", AstraTaskResultStatus::Cancelled)],
        );

        assert!(transition.ready.is_empty());
        assert_eq!(transition.cancelled.len(), 1);
        assert_eq!(transition.cancelled[0].0.id, "b");
    }

    #[test]
    fn next_wave_cancels_all_when_no_progress_possible() {
        let mut pending = vec![dep_task("b", &["ghost"]), dep_task("c", &["b"])];

        let transition = next_wave(
            &mut pending,
            &[wave_completion("a", AstraTaskResultStatus::Completed)],
        );

        assert!(transition.ready.is_empty());
        assert_eq!(transition.cancelled.len(), 2);
        assert!(transition.cancelled[0].1.contains("never dispatched"));
        assert!(pending.is_empty());
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
            selected_skill_ids: Vec::new(),
            selected_mcp_ids: Vec::new(),
            order: 0,
        }
    }
}
