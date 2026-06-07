use serde_json::{json, Value};

use super::{
    dedupe_session_ref_values, pick_stage_agent, stage_label, status_label, AstraRun,
    AstraTaskCompletion, AstraTaskProposal, StageTaskContext,
};
use crate::models::{IssueStatus, SessionInfo, StageStatus, ThreadInfo, ThreadKind};

const ASTRA_STAGE_ORCHESTRATION_RESPONSE_CONTRACT: &str = r#"You are Astra Orchestrator.

Return only one complete valid JSON object. Do not return markdown, code fences, comments, prose, trailing commas, partial JSON, or multiple JSON values.

Required top-level response:
{
  "summary": "string",
  "decisions": [],
  "tasks": []
}

For each completedTasks item, include at least one decisions item:
{
  "taskId": "completedTasks[n].task.id",
  "decision": { "action": "update_stage|add_or_update_issue|retry_stage|plan_next_round|complete_run|error_run", "...": "..." }
}
If one task result needs multiple state changes, return multiple decisions with the same taskId. For example, when a retry resolves an existing open issue and completes the stage, return one add_or_update_issue decision with issue.status "resolved" plus one update_stage decision.

Use these exact decision shapes.

update_stage:
{
  "taskId": "task-id",
  "decision": {
    "action": "update_stage",
    "stage": {
      "threadStageId": "thread-stage-id",
      "status": "not_started|in_progress|blocked|needs_review|completed|skipped",
      "summary": "string",
      "outcome": "string"
    },
    "reason": "string"
  }
}
Do not put stageId, threadStageId, status, summary, or outcome directly on decision for update_stage. Put those fields under decision.stage. Use threadStageId, not stageId.

add_or_update_issue:
{
  "taskId": "task-id",
  "decision": {
    "action": "add_or_update_issue",
    "issue": {
      "id": "optional-existing-issue-id",
      "threadStageId": "thread-stage-id",
      "title": "string",
      "description": "string",
      "status": "open|resolved|dismissed",
      "severity": "low|medium|high|critical"
    },
    "reason": "string"
  }
}
Use add_or_update_issue for issue lifecycle decisions, not only for new failures. When a task output resolves an existing open issue, return add_or_update_issue with the existing issue id if known, the same title, status "resolved", and a short resolution description. Use status "dismissed" only when the issue is invalid or no longer relevant. Use status "open" for unresolved findings that still need follow-up.

retry_stage:
{
  "taskId": "task-id",
  "decision": {
    "action": "retry_stage",
    "retry": { "reason": "string" },
    "reason": "string"
  }
}

plan_next_round:
{
  "taskId": "task-id",
  "decision": {
    "action": "plan_next_round",
    "reason": "string"
  }
}

complete_run:
{
  "taskId": "task-id",
  "decision": {
    "action": "complete_run",
    "reason": "string"
  }
}

error_run:
{
  "taskId": "task-id",
  "decision": {
    "action": "error_run",
    "reason": "string"
  }
}

Tasks must be planned against the thread state after applying all decisions in this same response. If the thread still has dispatchable stages after applying decisions and no exceptional stage blocks dispatch, tasks must include the next rolling task batch.

Task shape:
{
  "title": "string",
  "targetStageId": "thread-stage-id",
  "targetAgent": "codex|claude|gemini|astra-pi",
  "prompt": "string",
  "expectedOutput": "string",
  "risk": "low|medium|high"
}

Use rolling planning: return only the next safe task batch for one target stage. A batch may include multiple tasks only when every task has the same targetStageId and the tasks can run safely in parallel; never mix targetStageIds in one response. Stage order is context, not a strict dependency chain, so choose any appropriate stage when there are no exceptional stages.

Exceptional stages take priority: if any non-human stage is needs_review, return only review tasks targeting needs_review non-human stages. Human needs_review stages are waiting for human review and should receive no agent tasks. If no agent-review stage exists and any human stage is needs_review, tasks may be empty to stop for human review. If no review exception exists and any stage is blocked, return only recovery tasks for blocked stages."#;

const ASTRA_TEAMWORK_ORCHESTRATION_RESPONSE_CONTRACT: &str = r#"You are Astra Teamwork Orchestrator.

Return only one complete valid JSON object. Do not return markdown, code fences, comments, prose, trailing commas, partial JSON, or multiple JSON values.

Required top-level response:
{
  "summary": "string",
  "decisions": [],
  "tasks": []
}

Teamwork uses shared thread context plus Astra task orchestration. It does not use workflow stage scheduling.

For each completedTasks item, include at least one decisions item. Use only these decision actions for teamwork:
- plan_next_round
- complete_run
- error_run

Do not return update_stage, retry_stage, add_or_update_issue, stage mutation, issue mutation, or targetStageId for teamwork.

Teamwork task shape:
{
  "title": "string",
  "assistantId": "thread-assistant-id",
  "targetAgent": "codex|claude|gemini|astra-pi",
  "prompt": "string",
  "expectedOutput": "string",
  "risk": "low|medium|high"
}

assistantId must reference one of thread.assistants. targetAgent should match that assistant's runtime agent. If you create an agent-level task without assistantId, targetAgent is required, but prefer assistantId so Sessio can preserve team-member history and assistant snapshots.

Plan the next useful batch from the shared thread goal, userPrompt, thread.assistants, completedTasks, and prior session refs. Tasks in a batch may target different assistants and may run in parallel when independent."#;

fn astra_orchestration_response_contract(kind: ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Teamwork => ASTRA_TEAMWORK_ORCHESTRATION_RESPONSE_CONTRACT,
        ThreadKind::Workflow | ThreadKind::Brainstorm | ThreadKind::Debate => {
            ASTRA_STAGE_ORCHESTRATION_RESPONSE_CONTRACT
        }
    }
}

pub(super) fn build_astra_orchestration_prompt(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    completions: &[AstraTaskCompletion],
) -> String {
    let stages = if thread.kind == ThreadKind::Teamwork {
        Vec::new()
    } else {
        thread
            .stages
            .iter()
            .map(|stage| {
                let agent = pick_stage_agent(stage).map(|agent| agent.as_str().to_string());
                json!({
                    "id": stage.id,
                    "title": stage_label(stage),
                    "order": stage.order,
                    "kind": stage.kind,
                    "status": stage.status,
                    "assignableAgent": agent,
                    "summary": stage.summary,
                    "issues": stage.issues,
                })
            })
            .collect::<Vec<_>>()
    };
    let assistants = thread
        .assistants
        .iter()
        .map(|assistant| {
            json!({
                "assistantId": assistant.assistant_id,
                "name": assistant.name,
                "order": assistant.order,
                "agent": assistant.agent,
                "systemPrompt": assistant.system_prompt,
            })
        })
        .collect::<Vec<_>>();
    let completed_tasks = completions
        .iter()
        .map(super::filtered_task_completion_value)
        .collect::<Vec<_>>();

    json!({
        "instruction": astra_orchestration_response_contract(thread.kind),
        "thread": {
            "id": thread.id,
            "kind": thread.kind,
            "goal": thread.goal,
            "description": thread.description,
            "assistants": assistants,
            "stages": stages,
        },
        "run": {
            "id": run.run_id,
            "roundIndex": round_index,
            "retryLimit": run.retry_limit,
            "completedTaskIds": run.completed_task_ids,
            "stageAttemptCounts": run.stage_attempt_counts,
        },
        "userPrompt": user_prompt.unwrap_or(""),
        "completedTasks": completed_tasks,
    })
    .to_string()
}

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

pub(super) fn build_teamwork_task_context(
    thread: &ThreadInfo,
    assistant_id: &str,
    task: &AstraTaskProposal,
) -> anyhow::Result<StageTaskContext> {
    let assistant = thread
        .assistants
        .iter()
        .find(|assistant| assistant.assistant_id == assistant_id)
        .ok_or_else(|| {
            anyhow::anyhow!("assistant does not belong to Astra run thread: {assistant_id}")
        })?;
    let snapshot = build_teamwork_task_snapshot(thread, assistant_id);
    let prompt = render_teamwork_task_prompt(thread, assistant, &snapshot, task);
    Ok(StageTaskContext {
        thread_id: thread.id.clone(),
        thread_goal: thread.goal.clone(),
        stage_name: assistant.name.clone(),
        snapshot,
        prompt,
    })
}

fn build_teamwork_task_snapshot(thread: &ThreadInfo, focused_assistant_id: &str) -> Value {
    let mut assistants = thread.assistants.clone();
    assistants.sort_by_key(|assistant| assistant.order);
    let thread_session_refs = thread
        .sessions
        .iter()
        .map(|session| session_ref_json(session, "thread"))
        .collect::<Vec<_>>();
    json!({
        "threadId": thread.id,
        "projectId": thread.project_id,
        "kind": thread.kind,
        "goal": thread.goal,
        "description": thread.description,
        "focusedAssistantId": focused_assistant_id,
        "assistants": assistants,
        "threadSessionRefs": thread_session_refs,
        "relatedContext": {
            "sessionExcerptRefs": dedupe_session_ref_values(thread_session_refs.clone()),
        },
        "detailRefs": {
            "threadId": thread.id,
            "assistantIds": thread
                .assistants
                .iter()
                .map(|assistant| assistant.assistant_id.clone())
                .collect::<Vec<_>>(),
            "sessionRefs": thread_session_refs,
        },
        "capturedAt": super::now_ms(),
    })
}

fn render_teamwork_task_prompt(
    thread: &ThreadInfo,
    assistant: &crate::models::ThreadAssistantInfo,
    snapshot: &Value,
    task: &AstraTaskProposal,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio teamwork task".to_string());
    lines.push(String::new());
    lines.push("You are working as a thread-level assistant delegated by Astra. Treat this as shared-context teamwork, not a workflow stage chat.".to_string());
    lines.push(format!("Thread goal: {}", thread.goal));
    if let Some(description) = thread
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thread description: {description}"));
    }
    lines.push(format!("Assistant: {}", assistant.name));
    lines.push(format!("Assistant id: {}", assistant.assistant_id));
    lines.push(format!("Runtime agent: {}", task.target_agent.as_str()));
    if let Some(system_prompt) = assistant
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push("## Assistant instructions".to_string());
        lines.push(system_prompt.to_string());
    }
    if let Some(session_refs) = snapshot
        .pointer("/relatedContext/sessionExcerptRefs")
        .and_then(Value::as_array)
        .filter(|refs| !refs.is_empty())
    {
        lines.push(String::new());
        lines.push("## Shared session refs".to_string());
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
                format!("- [{agent}:{session_id}] {title}")
                    .trim_end()
                    .to_string(),
            );
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
    lines.push("Return a concise final result for Astra with concrete progress, decisions, blockers, and verification notes.".to_string());
    lines.push(
        "Do not update workflow stage state or create stage issues from this teamwork task."
            .to_string(),
    );
    lines.join("\n")
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
        .filter(|stage| {
            stage.id == focused_stage.id
                || stage.status == StageStatus::Blocked
                || stage.status == StageStatus::NeedsReview
                || stage
                    .issues
                    .iter()
                    .any(|issue| issue.status == IssueStatus::Open)
        })
        .map(|stage| {
            json!({
                "threadStageId": stage.id,
                "projectStageId": stage.stage_id,
                "name": stage_label(stage),
                "kind": stage.kind,
                "icon": stage.icon,
                "status": stage.status,
                "summary": if stage.id == focused_stage.id { stage.summary.clone() } else { None },
                "outcome": if stage.id == focused_stage.id { stage.outcome.clone() } else { None },
                "assistants": stage.assistants,
                "issues": stage.issues,
                "sessionRefs": if stage.id == focused_stage.id {
                    stage
                        .sessions
                        .iter()
                        .map(|session| session_ref_json(session, "stage"))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                },
            })
        })
        .collect::<Vec<_>>();
    let stage_session_refs = focused_stage
        .sessions
        .iter()
        .map(|session| session_ref_json(session, "stage"))
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
            "stageIds": stage_values
                .iter()
                .filter_map(|stage| stage.get("threadStageId").and_then(Value::as_str).map(str::to_string))
                .collect::<Vec<_>>(),
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::astra::{AstraRunStatus, AstraTaskResult, AstraTaskResultStatus, AstraTaskRisk};
    use crate::models::{
        Agent, AssistantAgentInfo, ProjectStageType, StageAssistantInfo, StageInfo,
        ThreadAssistantInfo, ThreadKind,
    };

    fn run() -> AstraRun {
        AstraRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp/project".to_string(),
            status: AstraRunStatus::Planning,
            proposed_tasks: Vec::new(),
            approved_task_ids: Vec::new(),
            delegated_session_ids: Vec::new(),
            task_results: Vec::new(),
            mode: "auto".to_string(),
            current_stage_id: None,
            completed_task_ids: Vec::new(),
            stage_attempt_counts: HashMap::new(),
            retry_limit: 3,
            planner_backend: None,
            decision_backend: None,
            round_index: None,
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn thread() -> ThreadInfo {
        let assistant = StageAssistantInfo {
            assistant_id: "assistant-1".to_string(),
            name: "Codex".to_string(),
            color: None,
            agent: AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: String::new(),
                mode: String::new(),
                effort: String::new(),
            },
            system_prompt: None,
            order: 0,
        };
        ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Ship the thread".to_string(),
            description: None,
            stage_id: None,
            kind: crate::models::ThreadKind::Workflow,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            assistants: Vec::new(),
            stages: vec![StageInfo {
                id: "stage-1".to_string(),
                thread_id: "thread-1".to_string(),
                stage_id: "project-stage-1".to_string(),
                project_id: "project-1".to_string(),
                assistant_ids: Vec::new(),
                assistants: vec![assistant],
                stage_type: ProjectStageType::Custom,
                workflow_id: None,
                kind: None,
                name: Some("Research".to_string()),
                description: None,
                icon: None,
                order: 0,
                status: StageStatus::InProgress,
                summary: None,
                outcome: None,
                enabled: true,
                allow_empty_assistants: false,
                created_at: 1,
                updated_at: 1,
                sessions: Vec::new(),
                issues: Vec::new(),
            }],
            sessions: Vec::new(),
        }
    }

    fn task() -> AstraTaskProposal {
        AstraTaskProposal {
            id: "task-1".to_string(),
            plan_task_id: None,
            assistant_id: None,
            title: "Research".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Do the stage work.".to_string(),
            expected_output: "Research notes.".to_string(),
            risk: AstraTaskRisk::Low,
        }
    }

    fn teamwork_thread() -> ThreadInfo {
        let mut thread = thread();
        thread.kind = ThreadKind::Teamwork;
        thread.description = Some("Coordinate shared context work.".to_string());
        thread.assistants = vec![
            ThreadAssistantInfo {
                assistant_id: "assistant-codex".to_string(),
                name: "Builder".to_string(),
                color: None,
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-write".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Build carefully from shared context.".to_string()),
                order: 0,
            },
            ThreadAssistantInfo {
                assistant_id: "assistant-claude".to_string(),
                name: "Reviewer".to_string(),
                color: None,
                agent: AssistantAgentInfo {
                    id: "claude".to_string(),
                    name: "Claude".to_string(),
                    model: "claude-sonnet-4-5".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Review carefully from shared context.".to_string()),
                order: 1,
            },
        ];
        thread
    }

    fn teamwork_task() -> AstraTaskProposal {
        AstraTaskProposal {
            id: "task-teamwork-1".to_string(),
            plan_task_id: None,
            assistant_id: Some("assistant-codex".to_string()),
            title: "Build shared task".to_string(),
            target_stage_id: None,
            target_agent: Agent::Codex,
            prompt: "Implement the shared-context task.".to_string(),
            expected_output: "Implementation result and verification.".to_string(),
            risk: AstraTaskRisk::Medium,
        }
    }

    fn task_result() -> AstraTaskResult {
        AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: AstraTaskResultStatus::Completed,
            output: "done".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        }
    }

    #[test]
    fn orchestration_prompt_uses_explicit_response_contract() {
        let task = task();
        let completion = AstraTaskCompletion {
            task,
            result: task_result(),
        };

        let prompt = build_astra_orchestration_prompt(
            &run(),
            &thread(),
            Some("user request"),
            2,
            &[completion],
        );
        let value: Value = serde_json::from_str(&prompt).unwrap();
        let instruction = value["instruction"].as_str().unwrap();

        assert!(instruction.contains(r#""summary": "string""#));
        assert!(instruction.contains(r#""decisions": []"#));
        assert!(instruction.contains(r#""tasks": []"#));
        assert!(instruction.contains(r#""stage": {"#));
        assert!(instruction.contains(r#""threadStageId": "thread-stage-id""#));
        assert!(instruction.contains("include at least one decisions item"));
        assert!(instruction.contains("return multiple decisions with the same taskId"));
        assert!(instruction.contains(r#""status": "open|resolved|dismissed""#));
        assert!(instruction.contains("Use add_or_update_issue for issue lifecycle decisions"));
        assert!(instruction.contains(r#"status "resolved""#));
        assert!(instruction.contains(
            "Do not put stageId, threadStageId, status, summary, or outcome directly on decision",
        ));
        assert!(instruction.contains("Use threadStageId, not stageId"));
        assert!(instruction.contains(
            "Tasks must be planned against the thread state after applying all decisions"
        ));
        assert!(instruction.contains(r#""targetAgent": "codex|claude|gemini|astra-pi""#));
        assert_eq!(value["thread"]["id"], "thread-1");
        assert_eq!(value["run"]["roundIndex"], 2);
        assert_eq!(value["userPrompt"], "user request");
        assert_eq!(value["completedTasks"][0]["task"]["id"], "task-1");
    }

    #[test]
    fn teamwork_orchestration_prompt_uses_assistants_without_stage_contract() {
        let prompt = build_astra_orchestration_prompt(
            &run(),
            &teamwork_thread(),
            Some("split work"),
            1,
            &[],
        );
        let value: Value = serde_json::from_str(&prompt).unwrap();
        let instruction = value["instruction"].as_str().unwrap();

        assert!(instruction.contains("Astra Teamwork Orchestrator"));
        assert!(instruction.contains("Teamwork uses shared thread context"));
        assert!(instruction.contains(r#""assistantId": "thread-assistant-id""#));
        assert!(instruction.contains("Do not return update_stage"));
        assert!(!instruction.contains(r#""stage": {"#));
        assert_eq!(value["thread"]["kind"], "teamwork");
        assert_eq!(
            value["thread"]["assistants"][0]["assistantId"],
            "assistant-codex"
        );
        assert_eq!(
            value["thread"]["assistants"][1]["assistantId"],
            "assistant-claude"
        );
        assert!(value["thread"]["stages"].as_array().unwrap().is_empty());
    }

    #[test]
    fn teamwork_task_context_uses_assistant_instructions_and_shared_context() {
        let thread = teamwork_thread();
        let task = teamwork_task();

        let context = build_teamwork_task_context(&thread, "assistant-codex", &task).unwrap();

        assert_eq!(
            context.snapshot["focusedAssistantId"],
            Value::String("assistant-codex".to_string())
        );
        assert_eq!(
            context.snapshot["kind"],
            Value::String("teamwork".to_string())
        );
        assert_eq!(
            context.snapshot["assistants"][0]["assistantId"],
            Value::String("assistant-codex".to_string())
        );
        assert!(context
            .prompt
            .contains("Treat this as shared-context teamwork, not a workflow stage chat."));
        assert!(context.prompt.contains("## Assistant instructions"));
        assert!(context
            .prompt
            .contains("Build carefully from shared context."));
        assert!(context.prompt.contains("## Astra task"));
        assert!(context
            .prompt
            .contains("Implement the shared-context task."));
        assert!(context
            .prompt
            .contains("Do not update workflow stage state or create stage issues"));
    }
}
