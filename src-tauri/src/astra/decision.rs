use serde_json::json;

use super::{
    summarize_task_output, thread_all_stages_terminal, AstraDecision, AstraTaskProposal,
    AstraTaskResult, AstraTaskResultStatus,
};
use crate::models::ThreadInfo;

pub(super) fn deterministic_decision(
    thread: &ThreadInfo,
    result: &AstraTaskResult,
    task: &AstraTaskProposal,
) -> AstraDecision {
    if result.retry_limit_reached {
        if let Some(stage_id) = result
            .thread_stage_id
            .as_deref()
            .or(task.target_stage_id.as_deref())
        {
            return AstraDecision::Composite {
                decisions: vec![
                    AstraDecision::AddOrUpdateIssue {
                        args: json!({
                            "taskId": task.id,
                            "threadStageId": stage_id,
                            "title": format!("Astra retry limit reached: {}", task.title),
                            "description": format!("Astra stopped retrying this stage after {} attempt(s). Manual intervention or a different strategy is required.", result.attempt_count),
                            "severity": "high",
                        }),
                    },
                    AstraDecision::UpdateStage {
                        args: json!({
                            "taskId": task.id,
                            "threadStageId": stage_id,
                            "status": "blocked",
                            "summary": format!("Astra retry limit reached for {}", task.title),
                            "outcome": "Blocked by repeated delegated task failure.",
                        }),
                    },
                ],
            };
        }
        return AstraDecision::ErrorRun {
            reason: "retry limit reached for thread-level Astra task".to_string(),
        };
    }
    match result.status {
        AstraTaskResultStatus::Completed => {
            if let Some(stage_id) = result
                .thread_stage_id
                .as_deref()
                .or(task.target_stage_id.as_deref())
            {
                let summary = summarize_task_output(&result.output);
                if has_explicit_incomplete_signal(&result.output) {
                    AstraDecision::UpdateStage {
                        args: json!({
                            "taskId": task.id,
                            "threadStageId": stage_id,
                            "status": "blocked",
                            "summary": summary,
                            "outcome": "Astra delegated task completed without satisfying the stage.",
                        }),
                    }
                } else if has_explicit_completion_signal(&result.output) {
                    AstraDecision::UpdateStage {
                        args: json!({
                            "taskId": task.id,
                            "threadStageId": stage_id,
                            "status": "completed",
                            "summary": summary,
                            "outcome": summary,
                        }),
                    }
                } else {
                    AstraDecision::UpdateStage {
                        args: json!({
                            "taskId": task.id,
                            "threadStageId": stage_id,
                            "status": "needs_review",
                            "summary": summary,
                            "outcome": "Astra delegated task returned a result that needs review before completion.",
                        }),
                    }
                }
            } else if thread_all_stages_terminal(thread) {
                AstraDecision::CompleteRun {
                    reason: "thread_level_task_completed".to_string(),
                }
            } else {
                AstraDecision::PlanNextRound {
                    reason: "thread_level_task_completed".to_string(),
                }
            }
        }
        AstraTaskResultStatus::Cancelled => AstraDecision::CancelRun {
            reason: result
                .error
                .clone()
                .unwrap_or_else(|| "delegated task was cancelled".to_string()),
        },
        AstraTaskResultStatus::Failed | AstraTaskResultStatus::Errored => {
            if let Some(stage_id) = result
                .thread_stage_id
                .as_deref()
                .or(task.target_stage_id.as_deref())
            {
                AstraDecision::AddOrUpdateIssue {
                    args: json!({
                        "taskId": task.id,
                        "threadStageId": stage_id,
                        "title": format!("Astra delegated task did not complete: {}", task.title),
                        "description": result.error.as_deref().unwrap_or_else(|| result.output.as_str()),
                        "severity": "high",
                    }),
                }
            } else {
                AstraDecision::ErrorRun {
                    reason: result
                        .error
                        .clone()
                        .unwrap_or_else(|| "thread-level delegated task failed".to_string()),
                }
            }
        }
    }
}

fn has_explicit_completion_signal(output: &str) -> bool {
    let text = output.to_ascii_lowercase();
    [
        "status: complete",
        "status: completed",
        "completed successfully",
        "done and verified",
        "implemented and verified",
        "verified complete",
        "stage complete",
        "stage completed",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn has_explicit_incomplete_signal(output: &str) -> bool {
    let text = output.to_ascii_lowercase();
    [
        "not complete",
        "not completed",
        "incomplete",
        "could not complete",
        "unable to complete",
        "need more information",
        "needs more information",
        "blocked",
        "cannot proceed",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}
