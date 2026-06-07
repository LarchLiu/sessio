use std::collections::HashSet;

use super::{
    pick_stage_agent, rolling_stage_task_batch, short_hash, stage_label,
    task_blocked_by_thread_exception, AstraPlan, AstraRun, AstraTaskProposal,
    AstraTaskResultStatus, AstraTaskRisk,
};
use crate::models::{Agent, IssueStatus, StageStatus, ThreadInfo, ThreadKind};

pub(super) fn deterministic_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    if thread.kind == ThreadKind::Teamwork {
        return deterministic_teamwork_plan(run, thread, user_prompt, round_index);
    }

    let mut stages = thread.stages.clone();
    stages.sort_by_key(|stage| stage.order);
    let completed: HashSet<&str> = run.completed_task_ids.iter().map(String::as_str).collect();
    let tasks = rolling_stage_task_batch(stages
        .iter()
        .filter(|stage| {
            !matches!(
                stage.status,
                StageStatus::Completed | StageStatus::Skipped
            )
        })
        .filter(|stage| task_blocked_by_thread_exception(run, thread, Some(&stage.id)).is_none())
        .filter(|stage| {
            let attempts = run.stage_attempt_counts.get(&stage.id).copied().unwrap_or(0);
            !(stage.status == StageStatus::Blocked && attempts >= run.retry_limit)
        })
        .filter_map(|stage| {
            let target_agent = pick_stage_agent(stage)?;
            let blocked = stage.status == StageStatus::Blocked;
            let needs_review = stage.status == StageStatus::NeedsReview;
            let kind = if needs_review {
                "review"
            } else if blocked {
                "unblock"
            } else {
                "advance"
            };
            let task_id = format!(
                "task-{}",
                short_hash(&format!(
                    "{}:{}:{}:{}",
                    run.thread_id, stage.id, kind, round_index
                ))
            );
            if completed.contains(task_id.as_str())
                || run.task_results.iter().any(|result| {
                    result.task_id == task_id
                        && matches!(result.status, AstraTaskResultStatus::Completed)
                })
            {
                return None;
            }
            let instruction = if needs_review {
                "Review this stage's latest result, identify gaps or regressions, and either provide concrete corrections or verification notes."
            } else if blocked {
                "Identify the blocker, propose the smallest recovery step, and perform safe progress if possible."
            } else {
                "Work on this stage goal and return concrete progress with verification notes."
            };
            let prompt = [
                user_prompt
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!("User orchestration instruction: {value}")),
                Some(instruction.to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
            Some(AstraTaskProposal {
                id: task_id,
                plan_task_id: None,
                assistant_id: None,
                title: format!(
                    "{} {}",
                    if needs_review {
                        "Review"
                    } else if blocked {
                        "Unblock"
                    } else {
                        "Advance"
                    },
                    stage_label(stage)
                ),
                target_stage_id: Some(stage.id.clone()),
                target_agent,
                prompt,
                expected_output: if needs_review {
                    "Review findings, corrections if needed, and verification notes.".to_string()
                } else if blocked {
                    "Blocker diagnosis, recovery action, and verification notes.".to_string()
                } else {
                    "Stage progress summary, files or decisions changed, and verification notes."
                        .to_string()
                },
                risk: if blocked {
                    AstraTaskRisk::High
                } else if needs_review {
                    AstraTaskRisk::Medium
                } else if stage.issues.iter().any(|issue| issue.status == IssueStatus::Open) {
                    AstraTaskRisk::Medium
                } else {
                    AstraTaskRisk::Low
                },
            })
        })
        .collect::<Vec<_>>());
    AstraPlan {
        summary: format!(
            "Deterministic Astra Orchestrator selected {} rolling task{} for \"{}\".",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" },
            thread.goal
        ),
        tasks,
    }
}

fn deterministic_teamwork_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    let completed: HashSet<&str> = run.completed_task_ids.iter().map(String::as_str).collect();
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
            if completed.contains(task_id.as_str())
                || run.task_results.iter().any(|result| {
                    result.task_id == task_id
                        && matches!(result.status, AstraTaskResultStatus::Completed)
                })
            {
                return None;
            }
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
    run: &AstraRun,
    thread: &ThreadInfo,
) -> Vec<AstraTaskProposal> {
    run.proposed_tasks
        .iter()
        .filter(|task| {
            if thread.kind == ThreadKind::Teamwork && task.target_stage_id.is_none() {
                return true;
            }
            task_blocked_by_thread_exception(run, thread, task.target_stage_id.as_deref()).is_none()
        })
        .filter(|task| {
            task.assistant_id.as_deref().is_none_or(|assistant_id| {
                thread
                    .assistants
                    .iter()
                    .any(|assistant| assistant.assistant_id == assistant_id)
            })
        })
        .filter(|task| {
            task.target_stage_id.as_deref().is_none_or(|stage_id| {
                thread.stages.iter().any(|stage| {
                    stage.id == stage_id
                        && !matches!(stage.status, StageStatus::Completed | StageStatus::Skipped)
                })
            })
        })
        .filter(|task| {
            !run.task_results.iter().any(|result| {
                result.task_id == task.id
                    && matches!(
                        result.status,
                        AstraTaskResultStatus::Completed
                            | AstraTaskResultStatus::Failed
                            | AstraTaskResultStatus::Errored
                            | AstraTaskResultStatus::Cancelled
                    )
            })
        })
        .cloned()
        .collect()
}
