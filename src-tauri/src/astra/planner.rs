use super::{short_hash, AstraPlan, AstraRun, AstraTaskProposal, AstraTaskRisk};
use crate::models::{Agent, ThreadInfo, ThreadKind};

pub(super) fn deterministic_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    if thread.kind != ThreadKind::Teamwork {
        return AstraPlan {
            summary: "Deterministic Astra Orchestrator only plans assistant-routed teamwork tasks."
                .to_string(),
            tasks: Vec::new(),
        };
    }

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
                title: format!("{} teamwork task", assistant.name),
                target_stage_id: None,
                target_agent,
                prompt,
                expected_output:
                    "Teamwork task result, concrete progress, decisions, and verification notes."
                        .to_string(),
                risk: AstraTaskRisk::Low,
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
        .cloned()
        .collect()
}
