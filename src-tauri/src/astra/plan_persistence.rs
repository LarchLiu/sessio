use anyhow::Result;
use serde_json::json;

use crate::models::{
    Agent, PlanRoundMode, PlanRoundSource, PlanRoundStatus, PlanTaskInfo, PlanTaskRisk,
    PlanTaskSessionRole, PlanTaskStatus, StageStatus, ThreadInfo, ThreadKind,
};
use crate::store::{
    AstraRunRecord, AstraRunSessionRecord, NewPlanRound, NewPlanTask, NewPlanTaskSession,
    PlanTaskStatusPatch, SessionStore,
};

use super::{
    delegated_attempt_id, final_task_output, is_runtime_placeholder_session_id, short_hash,
    summarize_task_output, AstraRun, AstraRunStatus, AstraTaskProposal, AstraTaskResult,
    AstraTaskResultStatus, AstraTaskRisk, RUST_NATIVE_ROUND_LIMIT,
};

struct OwnedAstraPlanTask {
    thread_stage_id: Option<String>,
    assistant_id: Option<String>,
    agent_participant_id: Option<String>,
    target_agent: Agent,
    stage_snapshot_json: Option<String>,
    assistant_snapshot_json: Option<String>,
    agent_snapshot_json: String,
    title: String,
    prompt: String,
    expected_output: Option<String>,
    artifact_role: Option<String>,
    uses_artifact_roles: Vec<String>,
    risk: PlanTaskRisk,
    sort_order: i64,
    status: PlanTaskStatus,
}

pub(super) fn run_to_record(run: &AstraRun) -> AstraRunRecord {
    let planner_agent = run
        .planner_backend
        .as_deref()
        .and_then(record_agent_for_backend)
        .unwrap_or(Agent::Codex);
    AstraRunRecord {
        run_id: run.run_id.clone(),
        thread_id: run.thread_id.clone(),
        project_id: run.project_id.clone(),
        project_path: run.project_path.clone(),
        continued_from_run_id: run.continued_from_run_id.clone(),
        status: run.status.as_str().to_string(),
        mode: run.mode.clone(),
        planner_backend: run.planner_backend.clone(),
        round_index: run.round_index.map(i64::from),
        round_limit: i64::from(run.round_limit),
        terminal_reason: run.terminal_reason.clone(),
        last_error_code: run.last_error_code.clone(),
        last_error_message: run.last_error_message.clone(),
        internal_planner_sessions: run
            .internal_planner_session_ids
            .iter()
            .enumerate()
            .map(|(index, session_id)| AstraRunSessionRecord {
                run_id: run.run_id.clone(),
                agent: planner_agent,
                session_id: session_id.clone(),
                role: PlanTaskSessionRole::Planner,
                sort_order: index as i64,
                created_at: run.created_at,
                updated_at: run.updated_at,
            })
            .collect(),
        run_diagnostics_json: serde_json::to_string(&run.run_diagnostics)
            .unwrap_or_else(|_| "[]".to_string()),
        error: run.error.clone(),
        created_at: run.created_at,
        updated_at: run.updated_at,
    }
}

pub(super) fn record_to_run(record: AstraRunRecord) -> Result<AstraRun> {
    Ok(AstraRun {
        run_id: record.run_id,
        thread_id: record.thread_id,
        project_id: record.project_id,
        project_path: record.project_path,
        continued_from_run_id: record.continued_from_run_id,
        status: AstraRunStatus::from_db_str(&record.status).unwrap_or(AstraRunStatus::Errored),
        mode: record.mode,
        planner_backend: record.planner_backend,
        round_index: record
            .round_index
            .and_then(|value| u32::try_from(value).ok()),
        round_limit: u32::try_from(record.round_limit)
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or(RUST_NATIVE_ROUND_LIMIT),
        terminal_reason: record.terminal_reason,
        last_error_code: record.last_error_code,
        last_error_message: record.last_error_message,
        internal_planner_session_ids: record
            .internal_planner_sessions
            .into_iter()
            .map(|session| session.session_id)
            .collect(),
        run_diagnostics: serde_json::from_str(&record.run_diagnostics_json).unwrap_or_default(),
        error: record.error,
        created_at: record.created_at,
        updated_at: record.updated_at,
    })
}

fn record_agent_for_backend(backend: &str) -> Option<Agent> {
    let backend = backend.trim();
    if backend.is_empty() {
        return None;
    }
    if let Some(agent) = backend.strip_prefix("runtime_agent_") {
        return Agent::from_db_str(agent);
    }
    Agent::from_db_str(backend)
}

fn plan_task_risk_from_astra(risk: AstraTaskRisk) -> PlanTaskRisk {
    match risk {
        AstraTaskRisk::Low => PlanTaskRisk::Low,
        AstraTaskRisk::Medium => PlanTaskRisk::Medium,
        AstraTaskRisk::High => PlanTaskRisk::High,
    }
}

fn plan_task_status_from_astra(status: AstraTaskResultStatus) -> PlanTaskStatus {
    match status {
        AstraTaskResultStatus::Completed => PlanTaskStatus::Completed,
        AstraTaskResultStatus::Failed => PlanTaskStatus::Failed,
        AstraTaskResultStatus::Errored => PlanTaskStatus::Errored,
        AstraTaskResultStatus::Cancelled => PlanTaskStatus::Cancelled,
    }
}

fn astra_task_risk_from_plan(risk: PlanTaskRisk) -> AstraTaskRisk {
    match risk {
        PlanTaskRisk::Low => AstraTaskRisk::Low,
        PlanTaskRisk::Medium => AstraTaskRisk::Medium,
        PlanTaskRisk::High => AstraTaskRisk::High,
    }
}

pub(crate) fn astra_task_from_plan_task(task: &PlanTaskInfo) -> AstraTaskProposal {
    AstraTaskProposal {
        id: task.id.clone(),
        plan_task_id: Some(task.id.clone()),
        assistant_id: task.assistant_id.clone(),
        agent_participant_id: task.agent_participant_id.clone(),
        title: task.title.clone(),
        target_stage_id: task.thread_stage_id.clone(),
        target_agent: task.target_agent,
        prompt: task.prompt.clone(),
        expected_output: task
            .expected_output
            .clone()
            .unwrap_or_else(|| "Task result.".to_string()),
        risk: astra_task_risk_from_plan(task.risk),
        depends_on: Vec::new(),
        artifact_role: task.artifact_role.clone(),
        uses_artifact_roles: task.uses_artifact_roles.clone(),
    }
}

pub(super) fn stable_run_id(thread_id: &str, now: i64) -> String {
    format!("astra-{}-{}", short_hash(thread_id), now)
}

pub(super) fn create_plan_round_for_astra_tasks_in_store(
    store: &dyn SessionStore,
    run: &AstraRun,
    thread: &ThreadInfo,
    summary: &str,
    mode: PlanRoundMode,
    _round_index: u32,
    tasks: Vec<AstraTaskProposal>,
) -> Result<Vec<AstraTaskProposal>> {
    let mut owned_tasks = tasks
        .iter()
        .enumerate()
        .map(|(idx, task)| astra_task_to_plan_task(store, thread, task, idx))
        .collect::<Result<Vec<_>>>()?;
    if mode == PlanRoundMode::Sequential {
        if let Some(first_task) = owned_tasks.first_mut() {
            first_task.status = PlanTaskStatus::Running;
        }
    }
    let new_tasks = owned_tasks
        .iter()
        .map(|task| NewPlanTask {
            thread_stage_id: task.thread_stage_id.as_deref(),
            assistant_id: task.assistant_id.as_deref(),
            agent_participant_id: task.agent_participant_id.as_deref(),
            target_agent: task.target_agent,
            stage_snapshot_json: task.stage_snapshot_json.as_deref(),
            assistant_snapshot_json: task.assistant_snapshot_json.as_deref(),
            agent_snapshot_json: &task.agent_snapshot_json,
            title: &task.title,
            prompt: &task.prompt,
            expected_output: task.expected_output.as_deref(),
            artifact_role: task.artifact_role.as_deref(),
            uses_artifact_roles: &task.uses_artifact_roles,
            risk: task.risk,
            sort_order: task.sort_order,
            status: task.status,
        })
        .collect::<Vec<_>>();
    let round = store.create_plan_round(NewPlanRound {
        thread_id: &run.thread_id,
        astra_run_id: Some(&run.run_id),
        round_index: None,
        summary: Some(summary),
        mode,
        source: PlanRoundSource::Astra,
        status: if new_tasks.is_empty() {
            PlanRoundStatus::Completed
        } else {
            PlanRoundStatus::Planned
        },
        tasks: new_tasks,
    })?;

    let mut next_tasks = tasks;
    let mut id_map = std::collections::HashMap::with_capacity(next_tasks.len());
    for (task, plan_task) in next_tasks.iter_mut().zip(round.tasks.iter()) {
        let old_id = std::mem::replace(&mut task.id, plan_task.id.clone());
        id_map.insert(old_id, plan_task.id.clone());
        task.plan_task_id = Some(plan_task.id.clone());
    }
    for task in next_tasks.iter_mut() {
        for dep in task.depends_on.iter_mut() {
            if let Some(new_id) = id_map.get(dep) {
                *dep = new_id.clone();
            }
        }
    }
    Ok(next_tasks)
}

fn astra_task_to_plan_task(
    store: &dyn SessionStore,
    thread: &ThreadInfo,
    task: &AstraTaskProposal,
    idx: usize,
) -> Result<OwnedAstraPlanTask> {
    let thread_assistant = if task.target_stage_id.is_none() {
        task.assistant_id
            .as_deref()
            .map(|assistant_id| {
                thread
                    .assistants
                    .iter()
                    .find(|assistant| assistant.assistant_id == assistant_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "assistant does not belong to Astra run thread: {assistant_id}"
                        )
                    })
            })
            .transpose()?
    } else {
        None
    };
    let agent_participant = task
        .agent_participant_id
        .as_deref()
        .map(|participant_id| {
            thread
                .agent_participants
                .iter()
                .find(|participant| participant.participant_id == participant_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "agent participant does not belong to Astra run thread: {participant_id}"
                    )
                })
        })
        .transpose()?;
    let stage = task.target_stage_id.as_deref().and_then(|stage_id| {
        thread
            .stages
            .iter()
            .find(|stage| stage.id == stage_id || stage.stage_id == stage_id)
    });
    let stage_assistant = stage.and_then(|stage| {
        task.assistant_id
            .as_deref()
            .and_then(|assistant_id| {
                stage
                    .assistants
                    .iter()
                    .find(|assistant| assistant.assistant_id == assistant_id)
            })
            .or_else(|| {
                stage
                    .assistants
                    .iter()
                    .find(|assistant| assistant.agent.id == task.target_agent.as_str())
            })
            .or_else(|| stage.assistants.first())
    });
    let agent_snapshot = store
        .list_agents()?
        .into_iter()
        .find(|agent| agent.id == task.target_agent.as_str());

    let assistant_id = thread_assistant
        .as_ref()
        .map(|assistant| assistant.assistant_id.clone())
        .or_else(|| stage_assistant.map(|assistant| assistant.assistant_id.clone()));
    let assistant_snapshot_json = thread_assistant
        .as_ref()
        .map(serde_json::to_string)
        .or_else(|| stage_assistant.map(serde_json::to_string))
        .transpose()?;
    let agent_participant_id = agent_participant
        .as_ref()
        .map(|participant| participant.participant_id.clone());
    let participant_snapshot = agent_participant.cloned();

    Ok(OwnedAstraPlanTask {
        thread_stage_id: stage.map(|stage| stage.id.clone()),
        assistant_id,
        agent_participant_id,
        target_agent: task.target_agent,
        stage_snapshot_json: stage.map(serde_json::to_string).transpose()?,
        assistant_snapshot_json,
        agent_snapshot_json: serde_json::to_string(&json!({
            "agent": task.target_agent,
            "participant": participant_snapshot,
            "agentInfo": agent_snapshot,
        }))?,
        title: task.title.clone(),
        prompt: task.prompt.clone(),
        expected_output: Some(task.expected_output.clone()),
        artifact_role: task.artifact_role.clone(),
        uses_artifact_roles: task.uses_artifact_roles.clone(),
        risk: plan_task_risk_from_astra(task.risk),
        sort_order: i64::try_from(idx).unwrap_or(i64::MAX),
        status: PlanTaskStatus::Planned,
    })
}

pub(super) fn mark_astra_plan_tasks_running_in_store(
    store: &dyn SessionStore,
    tasks: &[AstraTaskProposal],
) -> Result<()> {
    for task in tasks {
        if let Some(plan_task_id) = task.plan_task_id.as_deref() {
            store.update_plan_task_status(
                plan_task_id,
                PlanTaskStatusPatch {
                    status: PlanTaskStatus::Running,
                    result_summary: None,
                    error: None,
                },
            )?;
        }
    }
    Ok(())
}

pub(super) fn link_astra_plan_task_session_in_store(
    store: &dyn SessionStore,
    task: &AstraTaskProposal,
    agent: Agent,
    session_id: &str,
    role: PlanTaskSessionRole,
    attempt_count: u32,
) -> Result<()> {
    if let Some(plan_task_id) = task.plan_task_id.as_deref() {
        let attempt_id = delegated_attempt_id(plan_task_id, attempt_count);
        store.link_plan_task_session(NewPlanTaskSession {
            task_id: plan_task_id,
            agent,
            session_id,
            role,
            attempt_id: Some(&attempt_id),
            attempt_count: i64::from(attempt_count),
        })?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn relink_astra_plan_task_session_in_store(
    store: &dyn SessionStore,
    task: &AstraTaskProposal,
    agent: Agent,
    from_session_id: &str,
    from_role: PlanTaskSessionRole,
    to_session_id: &str,
    to_role: PlanTaskSessionRole,
    attempt_count: u32,
) -> Result<()> {
    if let Some(plan_task_id) = task.plan_task_id.as_deref() {
        let attempt_id = delegated_attempt_id(plan_task_id, attempt_count);
        store.relink_plan_task_session(
            NewPlanTaskSession {
                task_id: plan_task_id,
                agent,
                session_id: from_session_id,
                role: from_role,
                attempt_id: Some(&attempt_id),
                attempt_count: i64::from(attempt_count),
            },
            to_session_id,
            to_role,
        )?;
    }
    Ok(())
}

pub(super) fn record_plan_task_result_in_store(
    store: &dyn SessionStore,
    run: &AstraRun,
    result: &AstraTaskResult,
) -> Result<()> {
    let plan_task_id = result.task_id.trim();
    if plan_task_id.is_empty() {
        return Ok(());
    }
    let summary = summarize_task_output(&final_task_output(&result.output));
    let result_summary = if summary.trim().is_empty() {
        result.error.as_deref()
    } else {
        Some(summary.as_str())
    };
    let patch = PlanTaskStatusPatch {
        status: plan_task_status_from_astra(result.status),
        result_summary: Some(result_summary),
        error: Some(result.error.as_deref()),
    };
    let rounds = store.list_plan_rounds(&run.thread_id)?;
    let round = rounds
        .iter()
        .find(|round| round.tasks.iter().any(|task| task.id == plan_task_id));
    let Some(task) =
        round.and_then(|round| round.tasks.iter().find(|task| task.id == plan_task_id))
    else {
        return Ok(());
    };
    if round.is_some_and(|round| round.mode == PlanRoundMode::Sequential)
        && patch.status == PlanTaskStatus::Completed
    {
        store.complete_plan_task_and_start_next(plan_task_id, patch)?;
    } else {
        store.update_plan_task_status(plan_task_id, patch)?;
    }
    let session_id = result.sessio_runtime_session_id.trim();
    let result_session_already_linked = task
        .sessions
        .iter()
        .any(|session| session.agent == task.target_agent && session.session_id == session_id);
    if !session_id.is_empty()
        && !is_runtime_placeholder_session_id(session_id)
        && !result_session_already_linked
    {
        let attempt_id = delegated_attempt_id(plan_task_id, result.attempt_count);
        store.link_plan_task_session(NewPlanTaskSession {
            task_id: plan_task_id,
            agent: task.target_agent,
            session_id,
            role: PlanTaskSessionRole::Runtime,
            attempt_id: Some(&attempt_id),
            attempt_count: i64::from(result.attempt_count),
        })?;
    }
    Ok(())
}

pub(super) fn update_process_stage_after_task_result_in_store(
    store: &dyn SessionStore,
    run: &AstraRun,
    result: &AstraTaskResult,
) -> Result<bool> {
    let thread = store.get_thread_work_state(&run.thread_id)?;
    if thread.kind != ThreadKind::Process {
        return Ok(false);
    }
    let rounds = store.list_plan_rounds(&run.thread_id)?;
    let stage_id = result
        .thread_stage_id
        .as_deref()
        .map(str::to_string)
        .or_else(|| {
            rounds
                .iter()
                .flat_map(|round| round.tasks.iter())
                .find(|task| task.id == result.task_id)
                .and_then(|task| task.thread_stage_id.clone())
        });
    let Some(stage_id) = stage_id else {
        return Ok(false);
    };

    if matches!(
        result.status,
        AstraTaskResultStatus::Failed
            | AstraTaskResultStatus::Errored
            | AstraTaskResultStatus::Cancelled
    ) {
        store.update_thread_stage_state(
            &stage_id,
            Some(StageStatus::Blocked),
            None,
            result.error.clone().map(Some),
        )?;
        return Ok(true);
    }

    let stage_tasks = rounds
        .iter()
        .filter(|round| round.astra_run_id.as_deref() == Some(run.run_id.as_str()))
        .flat_map(|round| round.tasks.iter())
        .filter(|task| task.thread_stage_id.as_deref() == Some(stage_id.as_str()))
        .collect::<Vec<_>>();
    if stage_tasks.is_empty()
        || !stage_tasks
            .iter()
            .all(|task| task.status == PlanTaskStatus::Completed)
    {
        return Ok(false);
    }

    let summary = stage_tasks
        .iter()
        .filter_map(|task| task.result_summary.as_deref())
        .filter(|summary| !summary.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    store.update_thread_stage_state(
        &stage_id,
        Some(StageStatus::Completed),
        Some(if summary.trim().is_empty() {
            None
        } else {
            Some(summary)
        }),
        None,
    )?;
    Ok(true)
}
