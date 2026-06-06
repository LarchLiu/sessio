use std::collections::HashSet;

use super::{
    pick_stage_agent, short_hash, stage_label, AstraPlan, AstraRun, AstraTaskProposal,
    AstraTaskResultStatus, AstraTaskRisk,
};
use crate::models::{IssueStatus, StageStatus, ThreadInfo};

pub(super) fn deterministic_plan(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> AstraPlan {
    let mut stages = thread.stages.clone();
    stages.sort_by_key(|stage| stage.order);
    let completed: HashSet<&str> = run.completed_task_ids.iter().map(String::as_str).collect();
    let tasks = stages
        .iter()
        .filter(|stage| {
            !matches!(
                stage.status,
                StageStatus::Completed | StageStatus::Skipped | StageStatus::NeedsReview
            )
        })
        .filter(|stage| {
            let attempts = run.stage_attempt_counts.get(&stage.id).copied().unwrap_or(0);
            !(stage.status == StageStatus::Blocked && attempts >= run.retry_limit)
        })
        .filter_map(|stage| {
            let target_agent = pick_stage_agent(stage)?;
            let blocked = stage.status == StageStatus::Blocked;
            let kind = if blocked { "unblock" } else { "advance" };
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
            let instruction = if blocked {
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
                title: format!(
                    "{} {}",
                    if blocked { "Unblock" } else { "Advance" },
                    stage_label(stage)
                ),
                target_stage_id: Some(stage.id.clone()),
                target_agent,
                prompt,
                expected_output: if blocked {
                    "Blocker diagnosis, recovery action, and verification notes.".to_string()
                } else {
                    "Stage progress summary, files or decisions changed, and verification notes."
                        .to_string()
                },
                risk: if blocked {
                    AstraTaskRisk::High
                } else if stage.issues.iter().any(|issue| issue.status == IssueStatus::Open) {
                    AstraTaskRisk::Medium
                } else {
                    AstraTaskRisk::Low
                },
            })
        })
        .take(20)
        .collect::<Vec<_>>();
    AstraPlan {
        summary: format!(
            "Deterministic Astra found {} task{} for \"{}\".",
            tasks.len(),
            if tasks.len() == 1 { "" } else { "s" },
            thread.goal
        ),
        tasks,
    }
}

pub(super) fn next_dispatchable_tasks(run: &AstraRun) -> Vec<AstraTaskProposal> {
    run.proposed_tasks
        .iter()
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
