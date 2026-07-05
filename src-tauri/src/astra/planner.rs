use super::{short_hash, AstraPlan, AstraRun, AstraTaskProposal, AstraTaskRisk};
use crate::models::{Agent, StageInfo, StageStatus, ThreadInfo, ThreadKind};

pub(super) fn deterministic_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    match thread.kind {
        ThreadKind::Teamwork => deterministic_teamwork_plan(run, thread, user_prompt, round_index),
        ThreadKind::Process => deterministic_process_plan(run, thread, user_prompt, round_index),
        ThreadKind::Brainstorm | ThreadKind::Debate => AstraPlan {
            summary: "Deterministic Astra Orchestrator only plans teamwork or process tasks."
                .to_string(),
            tasks: Vec::new(),
        },
    }
}

fn deterministic_teamwork_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    let mut assistants = thread.assistants.clone();
    assistants.sort_by_key(|assistant| assistant.order);
    let tasks = assistants
        .into_iter()
        .filter_map(|assistant| {
            let target_agent = Agent::from_db_str(&assistant.agent.id)?;
            let task_id = format!(
                "task-{}",
                short_hash(&format!(
                    "{}:{}:{}:{}",
                    run.thread_id, assistant.assistant_id, target_agent.as_str(), round_index
                ))
            );
            let prompt = [
                user_prompt
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!("User orchestration instruction: {value}")),
                Some(format!(
                    "Work as the thread assistant \"{}\" on the shared thread goal. Return concrete progress, decisions, and verification notes.",
                    assistant.name
                )),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
            Some(AstraTaskProposal {
                id: task_id,
                plan_task_id: None,
                assistant_id: Some(assistant.assistant_id),
                agent_participant_id: None,
                title: format!("{} teamwork task", assistant.name),
                target_stage_id: None,
                target_agent,
                prompt,
                expected_output:
                    "Teamwork task result, concrete progress, decisions, and verification notes."
                        .to_string(),
                risk: AstraTaskRisk::Low,
                depends_on: Vec::new(),
                artifact_role: None,
                uses_artifact_roles: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    AstraPlan {
        summary: format!(
            "Deterministic Astra Orchestrator selected {} teamwork task{} for \"{}\".",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" },
            thread.goal
        ),
        tasks,
    }
}

fn deterministic_process_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    let mut remaining = remaining_process_stages(thread);
    remaining.sort_by_key(|stage| stage.order);

    let mut tasks = Vec::new();
    for stage in remaining {
        let mut assistants = stage.assistants.clone();
        assistants.sort_by_key(|assistant| assistant.order);
        let stage_tasks = assistants
            .into_iter()
            .filter_map(|assistant| {
                let target_agent = Agent::from_db_str(&assistant.agent.id)?;
                let task_id = format!(
                    "task-{}",
                    short_hash(&format!(
                        "{}:{}:{}:{}:{}",
                        run.thread_id,
                        stage.id,
                        assistant.assistant_id,
                        target_agent.as_str(),
                        round_index
                    ))
                );
                let stage_name = super::stage_label(stage);
                let prompt = [
                    user_prompt
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| format!("User orchestration instruction: {value}")),
                    Some(format!(
                        "Work on process stage \"{}\" as \"{}\". Return concrete progress, blockers, and verification notes for this stage.",
                        stage_name,
                        assistant.name
                    )),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("\n\n");
                Some(AstraTaskProposal {
                    id: task_id,
                    plan_task_id: None,
                    assistant_id: Some(assistant.assistant_id),
                    agent_participant_id: None,
                    title: format!("{} / {}", stage_name, assistant.name),
                    target_stage_id: Some(stage.id.clone()),
                    target_agent,
                    prompt,
                    expected_output:
                        "Process stage task result, concrete progress, blockers, and verification notes."
                            .to_string(),
                    risk: AstraTaskRisk::Low,
                    depends_on: Vec::new(),
                    artifact_role: None,
                    uses_artifact_roles: Vec::new(),
                })
            })
            .collect::<Vec<_>>();
        if stage_tasks.is_empty() {
            break;
        }
        tasks.extend(stage_tasks);
    }

    AstraPlan {
        summary: format!(
            "Deterministic Astra Orchestrator selected {} process task{} for \"{}\".",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" },
            thread.goal
        ),
        tasks,
    }
}

pub(super) fn remaining_process_stages(thread: &ThreadInfo) -> Vec<&StageInfo> {
    thread
        .stages
        .iter()
        .filter(|stage| stage.enabled)
        .filter(|stage| !matches!(stage.status, StageStatus::Completed | StageStatus::Skipped))
        .collect()
}

pub(super) fn next_dispatchable_tasks(
    tasks: &[AstraTaskProposal],
    thread: &ThreadInfo,
) -> Vec<AstraTaskProposal> {
    if !matches!(
        thread.kind,
        ThreadKind::Teamwork | ThreadKind::Brainstorm | ThreadKind::Debate
    ) {
        return Vec::new();
    }

    tasks
        .iter()
        .filter(|task| task.target_stage_id.is_none())
        .filter(|task| {
            task.assistant_id.as_deref().is_none_or(|assistant_id| {
                thread
                    .assistants
                    .iter()
                    .any(|assistant| assistant.assistant_id == assistant_id)
            })
        })
        .filter(|task| {
            task.agent_participant_id
                .as_deref()
                .is_none_or(|participant_id| {
                    thread
                        .agent_participants
                        .iter()
                        .any(|participant| participant.participant_id == participant_id)
                })
        })
        .cloned()
        .collect()
}
