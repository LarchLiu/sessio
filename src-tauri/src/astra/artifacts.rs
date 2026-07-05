use serde_json::{json, Value};

use super::{final_task_output, structured_response, AstraTaskCompletion, AstraTaskResultStatus};

pub(crate) const ASTRA_ARTIFACT_ROOT_DIR: &str = ".sessio/astra";
pub(crate) const TEAMWORK_ROUND_JOURNAL_KIND: &str = "teamwork_round_journal";
pub(super) const TEAMWORK_PLANNER_OUTPUT_CHAR_LIMIT: usize = 6000;
pub(super) const TEAMWORK_JOURNAL_SUMMARY_CHAR_LIMIT: usize = 600;
pub(super) const TEAMWORK_JOURNAL_TASK_EXCERPT_CHAR_LIMIT: usize = 400;
pub(super) const TEAMWORK_JOURNAL_TASK_ERROR_CHAR_LIMIT: usize = 240;
pub(super) const TEAMWORK_JOURNAL_PROMPT_ROUNDS: usize = 8;

fn task_completion_value(completion: &AstraTaskCompletion, output_char_limit: usize) -> Value {
    let output = final_task_output(&completion.result.output);
    let final_output = if output.trim().is_empty() {
        "Astra delegated task completed.".to_string()
    } else {
        structured_response::truncate_chars(&output, output_char_limit)
    };
    json!({
        "task": {
            "id": completion.task.id,
            "title": completion.task.title,
            "assistantId": completion.task.assistant_id,
            "targetAgent": completion.task.target_agent,
            "expectedOutput": completion.task.expected_output,
            "risk": completion.task.risk,
        },
        "result": {
            "taskId": completion.result.task_id,
            "status": completion.result.status,
            "finalOutput": final_output,
            "error": completion.result.error,
            "attemptCount": completion.result.attempt_count,
            "retryLimitReached": completion.result.retry_limit_reached,
            "completedAt": completion.result.completed_at,
        },
    })
}

pub(super) fn filtered_task_completion_value(completion: &AstraTaskCompletion) -> Value {
    task_completion_value(completion, 1000)
}

/// Teamwork planner variant: keeps far more of each task output than the
/// generic 1000-char excerpt and points at the on-disk artifact with the
/// complete output so the planner can read details on demand.
pub(super) fn planner_task_completion_value(
    run_id: &str,
    completion: &AstraTaskCompletion,
) -> Value {
    let mut value = task_completion_value(completion, TEAMWORK_PLANNER_OUTPUT_CHAR_LIMIT);
    if completion.result.status != AstraTaskResultStatus::Cancelled {
        if let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) {
            result.insert(
                "fullOutputPath".to_string(),
                json!(task_artifact_relative_path(
                    run_id,
                    &completion.task.id,
                    &completion.task.title
                )),
            );
        }
    }
    value
}

/// Workspace-relative path of the markdown artifact holding a task's complete
/// final output. The scheme is deterministic so prompt builders never need IO.
pub(super) fn task_artifact_relative_path(run_id: &str, task_id: &str, task_title: &str) -> String {
    format!(
        "{ASTRA_ARTIFACT_ROOT_DIR}/{}/tasks/{}--{}.md",
        artifact_path_component(run_id),
        artifact_title_component(task_title),
        artifact_path_component(task_id)
    )
}

fn artifact_path_component(value: &str) -> String {
    let component = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if component.is_empty() {
        "unknown".to_string()
    } else {
        component
    }
}

fn artifact_title_component(value: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = false;
    for ch in value.trim().chars() {
        let is_word = ch.is_alphanumeric() || ch == '_' || ch == '-';
        if is_word {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash && !out.is_empty() {
            out.push('-');
            last_was_dash = true;
        }
        if out.chars().count() >= 80 {
            break;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "task".to_string()
    } else {
        out
    }
}

/// Persists each completion's full final output under the project workspace so
/// later rounds (planner sessions and participant tasks run in the same
/// workspace) can read complete outputs on demand. Best-effort: failures are
/// logged and skipped, and consumers treat the files as optional.
pub(super) fn write_task_artifacts(
    project_path: &str,
    run_id: &str,
    completions: &[AstraTaskCompletion],
) {
    if completions.is_empty() {
        return;
    }
    let root = std::path::Path::new(project_path).join(ASTRA_ARTIFACT_ROOT_DIR);
    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        if let Err(error) =
            std::fs::create_dir_all(&root).and_then(|()| std::fs::write(&gitignore, "*\n"))
        {
            log::warn!(
                "[astra:artifacts] failed to prepare artifact root {}: {error}",
                root.display()
            );
            return;
        }
    }
    for completion in completions {
        let relative =
            task_artifact_relative_path(run_id, &completion.task.id, &completion.task.title);
        let path = std::path::Path::new(project_path).join(&relative);
        let result = path
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
            .and_then(|_| std::fs::write(&path, task_artifact_markdown(completion)));
        if let Err(error) = result {
            log::warn!(
                "[astra:artifacts] failed to write task artifact {}: {error}",
                path.display()
            );
        }
    }
}

fn task_artifact_markdown(completion: &AstraTaskCompletion) -> String {
    let mut lines = Vec::new();
    lines.push(format!("# {}", completion.task.title));
    lines.push(String::new());
    lines.push(format!("- Task id: {}", completion.task.id));
    if let Some(assistant_id) = completion.task.assistant_id.as_deref() {
        lines.push(format!("- Assistant: {assistant_id}"));
    }
    if let Some(participant_id) = completion.task.agent_participant_id.as_deref() {
        lines.push(format!("- Participant: {participant_id}"));
    }
    lines.push(format!(
        "- Agent: {}",
        completion.task.target_agent.as_str()
    ));
    lines.push(format!("- Status: {}", completion.result.status.as_str()));
    lines.push(format!(
        "- Completed at: {}",
        completion.result.completed_at
    ));
    if let Some(error) = completion
        .result
        .error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("- Error: {error}"));
    }
    lines.push(String::new());
    lines.push("## Final output".to_string());
    lines.push(String::new());
    lines.push(final_task_output(&completion.result.output));
    lines.push(String::new());
    lines.join("\n")
}

/// Compact per-round journal entry persisted into run diagnostics so the next
/// planning rounds keep memory of earlier work after `completions` is cleared.
pub(super) fn teamwork_round_journal_entry(
    run_id: &str,
    round_index: u32,
    planner_summary: &str,
    completions: &[AstraTaskCompletion],
    recorded_at: i64,
) -> Value {
    let tasks = completions
        .iter()
        .map(|completion| {
            let mut task = json!({
                "title": completion.task.title,
                "assistantId": completion.task.assistant_id,
                "risk": completion.task.risk,
                "status": completion.result.status,
                "outputExcerpt": structured_response::truncate_chars(
                    &final_task_output(&completion.result.output),
                    TEAMWORK_JOURNAL_TASK_EXCERPT_CHAR_LIMIT,
                ),
            });
            if let Some(record) = task.as_object_mut() {
                if completion.result.status != AstraTaskResultStatus::Cancelled {
                    record.insert(
                        "outputPath".to_string(),
                        json!(task_artifact_relative_path(
                            run_id,
                            &completion.task.id,
                            &completion.task.title
                        )),
                    );
                }
                if let Some(error) = completion
                    .result
                    .error
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    record.insert(
                        "error".to_string(),
                        json!(structured_response::truncate_chars(
                            error,
                            TEAMWORK_JOURNAL_TASK_ERROR_CHAR_LIMIT,
                        )),
                    );
                }
            }
            task
        })
        .collect::<Vec<_>>();
    json!({
        "kind": TEAMWORK_ROUND_JOURNAL_KIND,
        "roundIndex": round_index,
        "plannerSummary": structured_response::truncate_chars(
            planner_summary,
            TEAMWORK_JOURNAL_SUMMARY_CHAR_LIMIT,
        ),
        "tasks": tasks,
        "recordedAt": recorded_at,
    })
}

/// Extracts journal entries for the teamwork planner prompt. The round whose
/// completions are already passed verbatim as `completedTasks` is excluded by
/// equality (`roundIndex + 1 == current_round_index`) rather than a range so
/// that history survives worker restarts, where round indices start over.
pub(super) fn previous_rounds_from_diagnostics(
    diagnostics: &[Value],
    current_round_index: u32,
) -> Vec<Value> {
    let rounds = diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.get("kind").and_then(Value::as_str) == Some(TEAMWORK_ROUND_JOURNAL_KIND)
        })
        .filter(|diagnostic| {
            diagnostic
                .get("roundIndex")
                .and_then(Value::as_u64)
                .is_none_or(|round_index| {
                    round_index.saturating_add(1) != u64::from(current_round_index)
                })
        })
        .map(|diagnostic| {
            json!({
                "roundIndex": diagnostic.get("roundIndex"),
                "plannerSummary": diagnostic.get("plannerSummary"),
                "tasks": diagnostic.get("tasks"),
            })
        })
        .collect::<Vec<_>>();
    let skip = rounds.len().saturating_sub(TEAMWORK_JOURNAL_PROMPT_ROUNDS);
    rounds.into_iter().skip(skip).collect()
}
