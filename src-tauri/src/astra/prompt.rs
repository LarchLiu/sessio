use serde_json::{json, Value};

use super::{
    dedupe_session_ref_values, stage_label, status_label, AstraTaskProposal, StageTaskContext,
};
use crate::models::{IssueStatus, SessionInfo, StageStatus, ThreadInfo};

pub(super) fn build_stage_task_context(
    thread: &ThreadInfo,
    thread_stage_id: &str,
    task: &AstraTaskProposal,
) -> anyhow::Result<StageTaskContext> {
    let stage = thread
        .stages
        .iter()
        .find(|stage| stage.id == thread_stage_id)
        .ok_or_else(|| {
            anyhow::anyhow!("stage does not belong to Astra run thread: {thread_stage_id}")
        })?;
    let snapshot = build_stage_task_snapshot(thread, stage);
    let prompt = render_stage_task_prompt(thread, stage, &snapshot, task);
    Ok(StageTaskContext {
        thread_id: thread.id.clone(),
        thread_goal: thread.goal.clone(),
        stage_name: stage_label(stage),
        snapshot,
        prompt,
    })
}

fn build_stage_task_snapshot(
    thread: &ThreadInfo,
    focused_stage: &crate::models::StageInfo,
) -> Value {
    let mut stages = thread.stages.clone();
    stages.sort_by_key(|stage| stage.order);
    let current_stage_label = stages
        .iter()
        .find(|stage| stage.id == focused_stage.id)
        .map(stage_label)
        .unwrap_or_else(|| stage_label(focused_stage));
    let completed = stages
        .iter()
        .filter(|stage| matches!(stage.status, StageStatus::Completed | StageStatus::Skipped))
        .count();
    let blocked = stages
        .iter()
        .filter(|stage| stage.status == StageStatus::Blocked)
        .count();
    let open_issues = stages
        .iter()
        .map(|stage| {
            stage
                .issues
                .iter()
                .filter(|issue| issue.status == IssueStatus::Open)
                .count()
        })
        .sum::<usize>();
    let thread_session_refs = thread
        .sessions
        .iter()
        .map(|session| session_ref_json(session, "thread"))
        .collect::<Vec<_>>();
    let stage_values = stages
        .iter()
        .map(|stage| {
            let session_refs = stage
                .sessions
                .iter()
                .map(|session| session_ref_json(session, "stage"))
                .collect::<Vec<_>>();
            json!({
                "threadStageId": stage.id,
                "projectStageId": stage.stage_id,
                "name": stage_label(stage),
                "kind": stage.kind,
                "icon": stage.icon,
                "status": stage.status,
                "summary": stage.summary,
                "outcome": stage.outcome,
                "assistants": stage.assistants,
                "issues": stage.issues,
                "sessionRefs": session_refs,
            })
        })
        .collect::<Vec<_>>();
    let stage_session_refs = stages
        .iter()
        .flat_map(|stage| {
            stage
                .sessions
                .iter()
                .map(|session| session_ref_json(session, "stage"))
        })
        .collect::<Vec<_>>();
    let all_session_refs = dedupe_session_ref_values(
        thread_session_refs
            .iter()
            .cloned()
            .chain(stage_session_refs.iter().cloned())
            .collect(),
    );
    json!({
        "threadId": thread.id,
        "projectId": thread.project_id,
        "goal": thread.goal,
        "description": thread.description,
        "activeStageId": thread.stage_id,
        "focusedStageId": focused_stage.id,
        "stages": stage_values,
        "threadSessionRefs": thread_session_refs,
        "relatedContext": {
            "sessionExcerptRefs": all_session_refs,
        },
        "detailRefs": {
            "threadId": thread.id,
            "focusedStageId": focused_stage.id,
            "stageIds": stages.iter().map(|stage| stage.id.clone()).collect::<Vec<_>>(),
            "issueIds": stages
                .iter()
                .flat_map(|stage| stage.issues.iter().map(|issue| issue.id.clone()))
                .collect::<Vec<_>>(),
            "sessionRefs": all_session_refs,
        },
        "rollup": {
            "completed": completed,
            "incomplete": stages.len().saturating_sub(completed),
            "blocked": blocked,
            "openIssues": open_issues,
            "currentStage": current_stage_label,
            "total": stages.len(),
        },
        "capturedAt": super::now_ms(),
    })
}

fn render_stage_task_prompt(
    thread: &ThreadInfo,
    focused_stage: &crate::models::StageInfo,
    snapshot: &Value,
    task: &AstraTaskProposal,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio stage task".to_string());
    lines.push(String::new());
    lines.push("You are working on a delegated stage task from Astra. Treat this as a Sessio stage chat, not a general thread chat.".to_string());
    lines.push(format!("Thread goal: {}", thread.goal));
    if let Some(description) = thread
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thread description: {description}"));
    }
    lines.push(format!("Target threadStageId: {}", focused_stage.id));
    lines.push(format!("Target stage: {}", stage_label(focused_stage)));
    if let Some(description) = focused_stage
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Stage description: {description}"));
    }
    if let Some(summary) = focused_stage
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Current stage summary: {summary}"));
    }
    if let Some(outcome) = focused_stage
        .outcome
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Current stage outcome: {outcome}"));
    }
    let assistant_instructions =
        stage_assistant_system_prompts(focused_stage, task.target_agent.as_str());
    if !assistant_instructions.is_empty() {
        lines.push(String::new());
        lines.push("## Stage assistant instructions".to_string());
        for (name, prompt) in assistant_instructions {
            lines.push(format!("### {name}"));
            lines.push(prompt);
        }
    }
    let completed = snapshot
        .pointer("/rollup/completed")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = snapshot
        .pointer("/rollup/total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let blocked = snapshot
        .pointer("/rollup/blocked")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let open_issues = snapshot
        .pointer("/rollup/openIssues")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    lines.push(format!(
        "Thread progress: {completed}/{total} stages complete, {blocked} blocked, {open_issues} open issues"
    ));
    lines.push(String::new());
    lines.push("## Stage work snapshot".to_string());
    if let Some(stages) = snapshot.get("stages").and_then(Value::as_array) {
        for stage in stages {
            let id = stage
                .get("threadStageId")
                .and_then(Value::as_str)
                .unwrap_or("");
            let name = stage.get("name").and_then(Value::as_str).unwrap_or(id);
            let status = stage
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("not_started");
            let focus = if id == focused_stage.id {
                " <- you are here"
            } else {
                ""
            };
            lines.push(format!("- [{}] {name}{focus}", status_label(status)));
            if let Some(summary) = stage
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    summary: {summary}"));
            }
            if let Some(outcome) = stage
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                lines.push(format!("    outcome: {outcome}"));
            }
            if let Some(issues) = stage.get("issues").and_then(Value::as_array) {
                for issue in issues {
                    if issue.get("status").and_then(Value::as_str) != Some("open") {
                        continue;
                    }
                    let severity = issue
                        .get("severity")
                        .and_then(Value::as_str)
                        .unwrap_or("medium");
                    let title = issue
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("issue");
                    lines.push(format!("    issue [{severity}] {title}"));
                    if let Some(description) = issue
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        lines.push(format!("      {description}"));
                    }
                }
            }
            if let Some(session_refs) = stage.get("sessionRefs").and_then(Value::as_array) {
                for reference in session_refs {
                    let agent = reference
                        .get("agent")
                        .and_then(Value::as_str)
                        .unwrap_or("agent");
                    let session_id = reference
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .unwrap_or("session");
                    let title = reference.get("title").and_then(Value::as_str).unwrap_or("");
                    lines.push(
                        format!("    [{agent}:{session_id}] {title}")
                            .trim_end()
                            .to_string(),
                    );
                }
            }
        }
    }
    lines.push(String::new());
    lines.push("## Astra task".to_string());
    lines.push(format!("Task title: {}", task.title));
    lines.push(format!("Expected output: {}", task.expected_output));
    lines.push(String::new());
    lines.push(task.prompt.clone());
    lines.push(String::new());
    lines.push("## Reporting".to_string());
    lines.push("Return a concise final result for Astra. Astra will decide status, summary, and outcome, then ask Sessio to update thread_stage_states.".to_string());
    lines.push("Do not mark unrelated stages complete.".to_string());
    lines.join("\n")
}

fn stage_assistant_system_prompts(
    stage: &crate::models::StageInfo,
    agent_id: &str,
) -> Vec<(String, String)> {
    stage
        .assistants
        .iter()
        .filter(|assistant| assistant.agent.id == agent_id)
        .filter_map(|assistant| {
            let prompt = assistant.system_prompt.as_deref()?.trim();
            if prompt.is_empty() {
                None
            } else {
                Some((assistant.name.clone(), prompt.to_string()))
            }
        })
        .collect()
}

fn session_ref_json(session: &SessionInfo, source_kind: &str) -> Value {
    json!({
        "agent": session.agent,
        "sessionId": session.id,
        "title": session
            .rename_title
            .as_deref()
            .or(session.title.as_deref())
            .or(session.first_user_message.as_deref()),
        "filePath": if session.file_path.is_empty() { None::<&str> } else { Some(session.file_path.as_str()) },
        "sourceKind": source_kind,
    })
}
