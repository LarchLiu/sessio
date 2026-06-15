use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    dedupe_session_ref_values, pick_stage_agent, short_hash, stage_label, status_label, AstraRun,
    AstraTaskCompletion, AstraTaskProposal, StageTaskContext,
};
use crate::models::{IssueStatus, SessionInfo, StageStatus, ThreadInfo, ThreadKind};

const ASTRA_PROCESS_ORCHESTRATION_RESPONSE_CONTRACT: &str = r#"You are Astra Orchestrator.

Process threads are human-defined stages and do not use Astra automatic scheduling. Return an error terminal response if invoked for process.

Return only one complete YAML mapping. Do not return JSON, markdown, code fences, comments, prose, or multiple YAML documents.

Required top-level YAML response:
summary: string
runIntent: error
reason: process_astra_orchestration_unsupported
mode: null
tasks: []"#;

const ASTRA_TEAMWORK_ORCHESTRATION_RESPONSE_CONTRACT: &str = r#"You are Astra Teamwork Orchestrator.

Return only one complete YAML mapping. Do not return JSON, markdown, code fences, comments, prose, or multiple YAML documents.

Required top-level YAML response:
summary: string
runIntent: continue|complete|wait_for_human|error
reason: string
mode: parallel|sequential|null
tasks: []

Teamwork uses shared thread context plus Astra task orchestration. The response schema is closed: return only the top-level keys and task keys listed here.

Use runIntent:
- continue: create and dispatch one plan round. mode must be parallel or sequential, and tasks must be non-empty.
- complete: stop the Astra run successfully. mode must be null and tasks must be empty.
- wait_for_human: stop for human input or review. mode must be null and tasks must be empty.
- error: stop with a diagnostic error. mode must be null and tasks must be empty.

Teamwork task shape:
tasks:
  - id: short unique id within this response (t1, t2, ...)
    title: string
    assistantId: thread-assistant-id
    targetAgent: codex|claude|gemini|astra-pi
    prompt: string
    expectedOutput: string
    risk: low|medium|high
    dependsOn: [ids of other tasks in this response]

assistantId must reference one of thread.assistants. targetAgent should match that assistant's runtime agent. If you create an agent-level task without assistantId, targetAgent is required, but prefer assistantId so Sessio can preserve team-member history and assistant snapshots.

dependsOn declares execution-order dependencies inside one parallel round and is only valid with mode: parallel; a sequential round with dependsOn is rejected. Tasks with an empty or omitted dependsOn start immediately and run concurrently. A task starts only after every task it depends on completed successfully; if any dependency fails, errors, or is cancelled, the dependent task is cancelled automatically and reported as cancelled with the blocking dependency in its error. dependsOn must be a YAML list of task ids from this same response, with no self references, no unknown ids, and no cycles. id is required for any task that other tasks depend on, and a referenced id must be unique in the response. Prefer one parallel round with dependsOn over several single-task rounds when work fans out and joins (for example: t1 and t2 independent, then t3 with dependsOn: [t1, t2]).

Plan the next useful batch from the shared thread goal, userPrompt, thread.assistants, completedTasks, and prior session refs. Tasks in a parallel batch may target different assistants when independent. Use sequential only when the whole round is strictly linear; when a round mixes independent and dependent tasks, use parallel with dependsOn.

previousRounds is the run journal: one entry per earlier completed round, with the planner summary and each task's title, assistantId, risk, status, and outputExcerpt. completedTasks carries the full outputs of the most recent round only; the round already covered by completedTasks is not repeated in previousRounds. Use previousRounds to recall earlier results and decisions, avoid re-running finished work, and keep new tasks consistent with what was already built.

Full outputs on demand: each completedTasks result includes result.fullOutputPath and each previousRounds task includes outputPath - a workspace-relative markdown file containing that task's complete final output. finalOutput and outputExcerpt are truncated; read the file when planning needs details beyond the excerpt.

Write each expectedOutput as concrete acceptance criteria: the artifacts, behaviors, or checks a reviewer could verify, not a restatement of the prompt.

Review gate: when a completed task has risk medium|high, or later work depends on its output, schedule a review/verification task for a different assistant in a following round before building on it. If the review finds problems, re-dispatch the original work with the reviewer's concrete feedback included in the new task prompt. Do not re-review work that already passed review.

Synthesis gate: before returning complete, dispatch one final round containing a single synthesis task that consolidates the whole run's outputs into one deliverable satisfying the thread goal; the complete summary must then reference that deliverable's key points. If the run produced only a single completed task whose output already is the deliverable, you may skip the synthesis round.

Language: write summary and every task title, prompt, and expectedOutput in the language of the thread goal and userPrompt (for example, a Chinese-language thread gets Chinese tasks)."#;

const SESSIO_THREAD_PROMPT_START: &str = "<!-- sessio-thread-prompt:start";
const SESSIO_THREAD_PROMPT_END: &str = "<!-- sessio-thread-prompt:end";
static THREAD_PROMPT_NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn astra_orchestration_response_contract(kind: ThreadKind) -> &'static str {
    match kind {
        ThreadKind::Teamwork => ASTRA_TEAMWORK_ORCHESTRATION_RESPONSE_CONTRACT,
        ThreadKind::Process => ASTRA_PROCESS_ORCHESTRATION_RESPONSE_CONTRACT,
        ThreadKind::Brainstorm | ThreadKind::Debate => {
            ASTRA_PROCESS_ORCHESTRATION_RESPONSE_CONTRACT
        }
    }
}

fn html_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(super) fn wrap_thread_prompt(
    kind: &str,
    thread: &ThreadInfo,
    content: String,
    attrs: &[(&str, String)],
) -> String {
    let body = content.trim();
    if body.is_empty() {
        return String::new();
    }
    let nonce = thread_prompt_nonce(kind, thread, body);
    let mut attr_text = format!(
        " nonce=\"{}\" kind=\"{}\" thread_id=\"{}\" thread_kind=\"{}\"",
        html_attr(&nonce),
        html_attr(kind),
        html_attr(&thread.id),
        html_attr(thread.kind.as_str())
    );
    for (key, value) in attrs {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        attr_text.push_str(&format!(" {key}=\"{}\"", html_attr(value)));
    }
    format!(
        "{SESSIO_THREAD_PROMPT_START}{attr_text} -->\n\n{body}\n\n{SESSIO_THREAD_PROMPT_END} nonce=\"{}\" -->",
        html_attr(&nonce)
    )
}

fn thread_prompt_nonce(kind: &str, thread: &ThreadInfo, body: &str) -> String {
    let sequence = THREAD_PROMPT_NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    short_hash(&format!(
        "{kind}\0{}\0{}\0{sequence}\0{}",
        thread.id,
        super::now_ms(),
        body.len()
    ))
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
    let completed_tasks = if thread.kind == ThreadKind::Teamwork {
        completions
            .iter()
            .map(|completion| super::planner_task_completion_value(&run.run_id, completion))
            .collect::<Vec<_>>()
    } else {
        completions
            .iter()
            .map(super::filtered_task_completion_value)
            .collect::<Vec<_>>()
    };

    let mut body = json!({
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
        },
        "userPrompt": user_prompt.unwrap_or(""),
        "completedTasks": completed_tasks,
    });
    if thread.kind == ThreadKind::Teamwork {
        if let Some(record) = body.as_object_mut() {
            record.insert(
                "previousRounds".to_string(),
                Value::Array(super::previous_rounds_from_diagnostics(
                    &run.run_diagnostics,
                    round_index,
                )),
            );
        }
    }
    let body = body.to_string();
    wrap_thread_prompt(
        "astra_planner",
        thread,
        body,
        &[
            ("run_id", run.run_id.clone()),
            ("round_index", round_index.to_string()),
            (
                "prompt_summary",
                user_prompt.unwrap_or("Astra planner").to_string(),
            ),
        ],
    )
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

pub(super) fn build_thread_assistant_task_context(
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
    let snapshot = build_thread_assistant_task_snapshot(thread, assistant_id);
    let prompt = match thread.kind {
        ThreadKind::Teamwork => render_teamwork_task_prompt(thread, assistant, &snapshot, task),
        ThreadKind::Brainstorm => render_brainstorm_task_prompt(thread, assistant, task),
        ThreadKind::Debate => render_debate_task_prompt(thread, assistant, task),
        ThreadKind::Process => render_teamwork_task_prompt(thread, assistant, &snapshot, task),
    };
    Ok(StageTaskContext {
        thread_id: thread.id.clone(),
        thread_goal: thread.goal.clone(),
        stage_name: assistant.name.clone(),
        snapshot,
        prompt,
    })
}

pub(super) fn build_plan_task_snapshot_context(
    thread: &ThreadInfo,
    task: &AstraTaskProposal,
    stage_snapshot_json: Option<&str>,
    assistant_snapshot_json: Option<&str>,
    agent_snapshot_json: Option<&str>,
) -> anyhow::Result<Option<StageTaskContext>> {
    if stage_snapshot_json.is_none()
        && assistant_snapshot_json.is_none()
        && agent_snapshot_json.is_none()
    {
        return Ok(None);
    }

    let stage_snapshot = parse_task_snapshot(stage_snapshot_json, "stage")?;
    let assistant_snapshot = parse_task_snapshot(assistant_snapshot_json, "assistant")?;
    let agent_snapshot = parse_task_snapshot(agent_snapshot_json, "agent")?;
    let stage_name = task_snapshot_label(&stage_snapshot)
        .or_else(|| {
            task.target_stage_id.as_deref().and_then(|stage_id| {
                thread
                    .stages
                    .iter()
                    .find(|stage| stage.id == stage_id)
                    .map(stage_label)
            })
        })
        .or_else(|| task_snapshot_label(&assistant_snapshot))
        .unwrap_or_else(|| task.title.clone());
    let captured_at = super::now_ms();
    let mut snapshot = if thread.kind == ThreadKind::Process {
        build_process_thread_work_snapshot(thread, task.target_stage_id.as_deref(), captured_at)
    } else {
        json!({
            "threadId": thread.id,
            "projectId": thread.project_id,
            "kind": thread.kind,
            "goal": thread.goal,
            "description": thread.description,
            "capturedAt": captured_at,
        })
    };
    if let Some(snapshot_object) = snapshot.as_object_mut() {
        snapshot_object.insert("kind".to_string(), json!(thread.kind));
        snapshot_object.insert(
            "task".to_string(),
            json!({
                "id": task.id,
                "planTaskId": task.plan_task_id,
                "title": task.title,
                "assistantId": task.assistant_id,
                "threadStageId": task.target_stage_id,
                "targetAgent": task.target_agent,
                "expectedOutput": task.expected_output,
                "risk": task.risk,
            }),
        );
        snapshot_object.insert("stageSnapshot".to_string(), json!(stage_snapshot));
        snapshot_object.insert("assistantSnapshot".to_string(), json!(assistant_snapshot));
        snapshot_object.insert("agentSnapshot".to_string(), json!(agent_snapshot));
        snapshot_object.insert(
            "contextPolicy".to_string(),
            json!({
                "mode": "persisted_plan_task_snapshot",
                "source": "thread_plan_tasks",
            }),
        );
        snapshot_object.insert("capturedAt".to_string(), json!(captured_at));
    }
    let prompt = render_plan_task_snapshot_prompt(thread, &snapshot, task);
    Ok(Some(StageTaskContext {
        thread_id: thread.id.clone(),
        thread_goal: thread.goal.clone(),
        stage_name,
        snapshot,
        prompt,
    }))
}

fn parse_task_snapshot(value: Option<&str>, label: &str) -> anyhow::Result<Option<Value>> {
    value
        .map(|raw| {
            serde_json::from_str::<Value>(raw)
                .map_err(|err| anyhow::anyhow!("invalid plan task {label} snapshot json: {err}"))
        })
        .transpose()
}

fn build_process_thread_work_snapshot(
    thread: &ThreadInfo,
    focused_stage_id: Option<&str>,
    captured_at: i64,
) -> Value {
    let mut stages = thread.stages.clone();
    stages.sort_by_key(|stage| stage.order);
    let current_stage_label = focused_stage_id
        .and_then(|stage_id| stages.iter().find(|stage| stage.id == stage_id))
        .map(stage_label);
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
                "sessionRefs": stage
                    .sessions
                    .iter()
                    .map(|session| session_ref_json(session, "stage"))
                    .collect::<Vec<_>>(),
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
        "kind": thread.kind,
        "goal": thread.goal,
        "description": thread.description,
        "activeStageId": thread.stage_id,
        "focusedStageId": focused_stage_id,
        "stages": stage_values,
        "threadSessionRefs": thread_session_refs,
        "relatedContext": {
            "sessionExcerptRefs": all_session_refs,
        },
        "detailRefs": {
            "threadId": thread.id,
            "focusedStageId": focused_stage_id,
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
        "capturedAt": captured_at,
    })
}

fn task_snapshot_label(value: &Option<Value>) -> Option<String> {
    let value = value.as_ref()?.as_object()?;
    value
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("stageId").and_then(Value::as_str))
        .or_else(|| value.get("assistantId").and_then(Value::as_str))
        .or_else(|| value.get("id").and_then(Value::as_str))
        .map(ToString::to_string)
}

fn snapshot_text(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn build_thread_assistant_task_snapshot(thread: &ThreadInfo, focused_assistant_id: &str) -> Value {
    let mut assistants = thread.assistants.clone();
    assistants.sort_by_key(|assistant| assistant.order);
    let thread_session_refs = if thread.kind == ThreadKind::Debate {
        Vec::new()
    } else {
        thread
            .sessions
            .iter()
            .map(|session| session_ref_json(session, "thread"))
            .collect::<Vec<_>>()
    };
    let context_policy = match thread.kind {
        ThreadKind::Debate => json!({
            "mode": "isolated_lane",
            "laneId": format!("lane-{}", short_hash(focused_assistant_id)),
            "sharedInput": "thread_goal_and_user_instruction",
            "crossCheckVisibility": "stage_artifact_only",
            "hidden": "full_lane_transcripts",
        }),
        ThreadKind::Brainstorm => json!({
            "mode": "shared_board",
            "sharedInput": "thread_goal_user_instruction_and_shared_board",
        }),
        ThreadKind::Teamwork => json!({
            "mode": "shared_context_teamwork",
        }),
        ThreadKind::Process => json!({
            "mode": "process_stage",
        }),
    };
    let related_session_refs = if thread.kind == ThreadKind::Debate {
        Vec::new()
    } else {
        dedupe_session_ref_values(thread_session_refs.clone())
    };
    json!({
        "threadId": thread.id,
        "projectId": thread.project_id,
        "kind": thread.kind,
        "goal": thread.goal,
        "description": thread.description,
        "focusedAssistantId": focused_assistant_id,
        "assistants": assistants,
        "threadSessionRefs": thread_session_refs,
        "contextPolicy": context_policy,
        "relatedContext": {
            "sessionExcerptRefs": related_session_refs,
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

fn render_plan_task_snapshot_prompt(
    thread: &ThreadInfo,
    snapshot: &Value,
    task: &AstraTaskProposal,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio plan task".to_string());
    lines.push(String::new());
    lines.push("You are working on a delegated Astra plan task. Use the persisted task snapshots below as the execution context; they reflect the stage, assistant, and agent configuration captured when this task was planned.".to_string());
    lines.push(format!("Thread goal: {}", thread.goal));
    if let Some(description) = thread
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thread description: {description}"));
    }
    lines.push(format!("Task title: {}", task.title));
    lines.push(format!("Runtime agent: {}", task.target_agent.as_str()));
    if let Some(stage_id) = task.target_stage_id.as_deref() {
        lines.push(format!("Thread stage id: {stage_id}"));
    }
    if let Some(assistant_id) = task.assistant_id.as_deref() {
        lines.push(format!("Assistant id: {assistant_id}"));
    }
    lines.push(String::new());
    lines.push("## Persisted snapshots".to_string());
    if let Some(stage) = snapshot
        .get("stageSnapshot")
        .filter(|value| !value.is_null())
    {
        lines.push("### Stage snapshot".to_string());
        lines.push(snapshot_text(stage));
    }
    if let Some(assistant) = snapshot
        .get("assistantSnapshot")
        .filter(|value| !value.is_null())
    {
        lines.push("### Assistant snapshot".to_string());
        lines.push(snapshot_text(assistant));
    }
    if let Some(agent) = snapshot
        .get("agentSnapshot")
        .filter(|value| !value.is_null())
    {
        lines.push("### Agent snapshot".to_string());
        lines.push(snapshot_text(agent));
    }
    lines.push(String::new());
    lines.push("## Astra task".to_string());
    lines.push(format!("Expected output: {}", task.expected_output));
    lines.push(String::new());
    lines.push(task.prompt.clone());
    lines.push(String::new());
    lines.push("## Reporting".to_string());
    lines.push("Return a concise final result for Astra. Do not mutate process stages or issues unless this task explicitly asks for a separate manual action.".to_string());
    let mut attrs = vec![
        ("task_id", task.id.clone()),
        ("task_title", task.title.clone()),
        ("target_agent", task.target_agent.as_str().to_string()),
    ];
    if let Some(plan_task_id) = task.plan_task_id.as_deref() {
        attrs.push(("plan_task_id", plan_task_id.to_string()));
    }
    if let Some(stage_id) = task.target_stage_id.as_deref() {
        attrs.push(("thread_stage_id", stage_id.to_string()));
    }
    if let Some(assistant_id) = task.assistant_id.as_deref() {
        attrs.push(("assistant_id", assistant_id.to_string()));
    }
    wrap_thread_prompt("astra_plan_task", thread, lines.join("\n"), &attrs)
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
    lines.push("You are working as a thread-level assistant delegated by Astra. Treat this as shared-context teamwork, not a process stage chat.".to_string());
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
        "Do not update process stage state or create stage issues from this teamwork task."
            .to_string(),
    );
    wrap_thread_prompt(
        "astra_teamwork_task",
        thread,
        lines.join("\n"),
        &[
            ("task_id", task.id.clone()),
            ("task_title", task.title.clone()),
            ("assistant_id", assistant.assistant_id.clone()),
            ("assistant_name", assistant.name.clone()),
            ("target_agent", task.target_agent.as_str().to_string()),
        ],
    )
}

fn render_brainstorm_task_prompt(
    thread: &ThreadInfo,
    assistant: &crate::models::ThreadAssistantInfo,
    task: &AstraTaskProposal,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio brainstorm task".to_string());
    lines.push(String::new());
    lines.push("You are working as a thread-level assistant in shared-board brainstorm mode. Treat the task prompt as the source of truth for the current board or synthesis instruction.".to_string());
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
    lines.push(String::new());
    lines.push("## Astra task".to_string());
    lines.push(format!("Task title: {}", task.title));
    lines.push(format!("Expected output: {}", task.expected_output));
    lines.push(String::new());
    lines.push(task.prompt.clone());
    lines.push(String::new());
    lines.push("## Reporting".to_string());
    lines.push("Return a concise final result for Astra. Preserve concrete candidates, agreements, disagreements, risks, and questions.".to_string());
    lines.push(
        "Do not update process stage state or create stage issues from this brainstorm task."
            .to_string(),
    );
    wrap_thread_prompt(
        "astra_brainstorm_task",
        thread,
        lines.join("\n"),
        &[
            ("task_id", task.id.clone()),
            ("task_title", task.title.clone()),
            ("assistant_id", assistant.assistant_id.clone()),
            ("assistant_name", assistant.name.clone()),
            ("target_agent", task.target_agent.as_str().to_string()),
        ],
    )
}

fn render_debate_task_prompt(
    thread: &ThreadInfo,
    assistant: &crate::models::ThreadAssistantInfo,
    task: &AstraTaskProposal,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio debate lane task".to_string());
    lines.push(String::new());
    lines.push("You are working in an isolated debate lane delegated by Astra. Use only the input explicitly present in this prompt: the shared initial problem, your lane instruction, and any visible cross-check artifacts. Do not assume access to another lane's full transcript.".to_string());
    lines.push(format!("Thread goal: {}", thread.goal));
    if let Some(description) = thread
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thread description: {description}"));
    }
    lines.push(format!("Lane assistant: {}", assistant.name));
    lines.push(format!("Assistant id: {}", assistant.assistant_id));
    lines.push(format!("Runtime agent: {}", task.target_agent.as_str()));
    if let Some(system_prompt) = assistant
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push("## Lane instructions".to_string());
        lines.push(system_prompt.to_string());
    }
    lines.push(String::new());
    lines.push("## Astra task".to_string());
    lines.push(format!("Task title: {}", task.title));
    lines.push(format!("Expected output: {}", task.expected_output));
    lines.push(String::new());
    lines.push(task.prompt.clone());
    lines.push(String::new());
    lines.push("## Reporting".to_string());
    lines.push("Return a concise final result for Astra with answer, evidence, assumptions, confidence, disagreements, and convergence notes.".to_string());
    lines.push(
        "Do not update process stage state or create stage issues from this debate task."
            .to_string(),
    );
    wrap_thread_prompt(
        "astra_debate_task",
        thread,
        lines.join("\n"),
        &[
            ("task_id", task.id.clone()),
            ("task_title", task.title.clone()),
            ("assistant_id", assistant.assistant_id.clone()),
            ("assistant_name", assistant.name.clone()),
            ("target_agent", task.target_agent.as_str().to_string()),
        ],
    )
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
    wrap_thread_prompt(
        "astra_stage_task",
        thread,
        lines.join("\n"),
        &[
            ("task_id", task.id.clone()),
            ("task_title", task.title.clone()),
            ("thread_stage_id", focused_stage.id.clone()),
            ("stage_name", stage_label(focused_stage)),
            ("target_agent", task.target_agent.as_str().to_string()),
        ],
    )
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
            mode: "auto".to_string(),
            planner_backend: None,
            round_index: None,
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
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
            kind: crate::models::ThreadKind::Process,
            enabled: true,
            origin: crate::models::ThreadOrigin::Manual,
            scheduled_task_id: None,
            created_at: 1,
            updated_at: 1,
            assistants: Vec::new(),
            agent_participants: Vec::new(),
            stages: vec![StageInfo {
                id: "stage-1".to_string(),
                thread_id: "thread-1".to_string(),
                stage_id: "project-stage-1".to_string(),
                project_id: "project-1".to_string(),
                assistant_ids: Vec::new(),
                assistants: vec![assistant],
                stage_type: ProjectStageType::Custom,
                process_template_id: None,
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
            agent_participant_id: None,
            title: "Research".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Do the stage work.".to_string(),
            expected_output: "Research notes.".to_string(),
            risk: AstraTaskRisk::Low,
            depends_on: Vec::new(),
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
            agent_participant_id: None,
            title: "Build shared task".to_string(),
            target_stage_id: None,
            target_agent: Agent::Codex,
            prompt: "Implement the shared-context task.".to_string(),
            expected_output: "Implementation result and verification.".to_string(),
            risk: AstraTaskRisk::Medium,
            depends_on: Vec::new(),
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
            completed_at: 1,
        }
    }

    fn thread_prompt_body(prompt: &str) -> &str {
        let start = prompt.find("-->").map(|idx| idx + "-->".len()).unwrap_or(0);
        let end = prompt
            .find(SESSIO_THREAD_PROMPT_END)
            .unwrap_or(prompt.len());
        prompt[start..end].trim()
    }

    #[test]
    fn wrapped_thread_prompt_requires_matching_nonce_to_strip() {
        let prompt = wrap_thread_prompt(
            "test",
            &thread(),
            "before <!-- sessio-thread-prompt:end --> after".to_string(),
            &[],
        );

        assert!(prompt.contains(" nonce=\""));
        assert_eq!(
            crate::models::strip_sessio_thread_prompt_blocks(&format!("visible\n{prompt}\nrest")),
            "visible\n\nrest"
        );
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
        let value: Value = serde_json::from_str(thread_prompt_body(&prompt)).unwrap();
        let instruction = value["instruction"].as_str().unwrap();

        assert!(instruction.contains("Return only one complete YAML mapping"));
        assert!(instruction.contains("Do not return JSON"));
        assert!(instruction.contains("summary: string"));
        assert!(instruction.contains("runIntent: error"));
        assert!(instruction.contains("reason: process_astra_orchestration_unsupported"));
        assert!(instruction.contains("mode: null"));
        assert!(instruction.contains("tasks: []"));
        assert!(instruction.contains(
            "Process threads are human-defined stages and do not use Astra automatic scheduling"
        ));
        assert!(!instruction.contains(r#""decisions": []"#));
        assert!(!instruction.contains(r#""stage": {"#));
        assert_eq!(value["thread"]["id"], "thread-1");
        assert_eq!(value["run"]["roundIndex"], 2);
        assert_eq!(value["userPrompt"], "user request");
        assert_eq!(value["completedTasks"][0]["task"]["id"], "task-1");
        assert!(value.get("previousRounds").is_none());
    }

    #[test]
    fn teamwork_orchestration_prompt_uses_assistants_without_stage_contract() {
        let mut task = teamwork_task();
        task.target_stage_id = Some("stage-legacy-1".to_string());
        let mut result = task_result();
        result.task_id = task.id.clone();
        result.thread_stage_id = Some("stage-legacy-1".to_string());
        let completion = AstraTaskCompletion { task, result };

        let prompt = build_astra_orchestration_prompt(
            &run(),
            &teamwork_thread(),
            Some("split work"),
            1,
            &[completion],
        );
        let value: Value = serde_json::from_str(thread_prompt_body(&prompt)).unwrap();
        let instruction = value["instruction"].as_str().unwrap();

        assert!(instruction.contains("Astra Teamwork Orchestrator"));
        assert!(instruction.contains("Teamwork uses shared thread context"));
        assert!(instruction.contains("Return only one complete YAML mapping"));
        assert!(instruction.contains("Do not return JSON"));
        assert!(instruction.contains("runIntent: continue|complete|wait_for_human|error"));
        assert!(instruction.contains("mode: parallel|sequential|null"));
        assert!(instruction.contains("assistantId: thread-assistant-id"));
        assert!(instruction.contains("response schema is closed"));
        assert!(instruction.contains("dependsOn: [ids of other tasks in this response]"));
        assert!(instruction.contains("only valid with mode: parallel"));
        assert!(instruction.contains("previousRounds is the run journal"));
        assert!(instruction.contains("Full outputs on demand"));
        assert!(instruction.contains("acceptance criteria"));
        assert!(instruction.contains("Review gate:"));
        assert!(instruction.contains("Synthesis gate:"));
        assert!(instruction.contains("Language: write summary and every task title"));
        assert!(!instruction.contains("targetStageId"));
        assert!(!instruction.contains(r#""stage": {"#));
        assert!(!prompt.contains("targetStageId"));
        assert!(!prompt.contains("threadStageId"));
        assert!(!prompt.contains("stageAttemptCounts"));
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
        assert!(value["run"].get("stageAttemptCounts").is_none());
        assert_eq!(
            value["completedTasks"][0]["task"]["assistantId"],
            "assistant-codex"
        );
        assert_eq!(
            value["completedTasks"][0]["result"]["taskId"],
            "task-teamwork-1"
        );
    }

    #[test]
    fn teamwork_orchestration_prompt_injects_previous_rounds_and_full_outputs() {
        let mut task = teamwork_task();
        task.target_stage_id = None;
        let mut result = task_result();
        result.task_id = task.id.clone();
        result.output = format!("Final result: {}", "结论 ".repeat(700));
        let completion = AstraTaskCompletion { task, result };

        let mut run = run();
        run.run_diagnostics = vec![
            json!({
                "kind": "orchestrator_backend_failure",
                "code": "timeout",
            }),
            json!({
                "kind": crate::astra::TEAMWORK_ROUND_JOURNAL_KIND,
                "roundIndex": 0,
                "plannerSummary": "第 1 轮：完成需求分析。",
                "tasks": [{ "title": "需求分析", "status": "completed" }],
                "recordedAt": 1,
            }),
            json!({
                "kind": crate::astra::TEAMWORK_ROUND_JOURNAL_KIND,
                "roundIndex": 1,
                "plannerSummary": "第 2 轮：实现核心接口。",
                "tasks": [],
                "recordedAt": 2,
            }),
        ];

        let prompt =
            build_astra_orchestration_prompt(&run, &teamwork_thread(), None, 2, &[completion]);
        let value: Value = serde_json::from_str(thread_prompt_body(&prompt)).unwrap();

        // Round 1 feeds completedTasks and is excluded; the failure diagnostic
        // never leaks into the journal view.
        let rounds = value["previousRounds"].as_array().unwrap();
        assert_eq!(rounds.len(), 1);
        assert_eq!(rounds[0]["roundIndex"], 0);
        assert_eq!(rounds[0]["plannerSummary"], "第 1 轮：完成需求分析。");
        assert_eq!(rounds[0]["tasks"][0]["title"], "需求分析");
        assert!(rounds[0].get("kind").is_none());
        assert!(rounds[0].get("code").is_none());

        let final_output = value["completedTasks"][0]["result"]["finalOutput"]
            .as_str()
            .unwrap();
        assert!(final_output.chars().count() > 1000);
        assert!(final_output.contains("结论"));
        assert!(!final_output.contains("Final result:"));
        let full_output_path = value["completedTasks"][0]["result"]["fullOutputPath"]
            .as_str()
            .unwrap();
        assert!(full_output_path.starts_with(".sessio/astra/run-1/tasks/"));
        assert!(full_output_path.ends_with(".md"));
    }

    #[test]
    fn teamwork_task_context_uses_assistant_instructions_and_shared_context() {
        let thread = teamwork_thread();
        let task = teamwork_task();

        let context =
            build_thread_assistant_task_context(&thread, "assistant-codex", &task).unwrap();

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
            .contains("Treat this as shared-context teamwork, not a process stage chat."));
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
            .contains("Do not update process stage state or create stage issues"));
    }

    #[test]
    fn debate_task_context_uses_isolated_lane_policy_without_shared_refs() {
        let mut thread = teamwork_thread();
        thread.kind = ThreadKind::Debate;
        thread.sessions = vec![SessionInfo {
            id: "thread-session-1".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: None,
            project_name: None,
            started_at: Some(1),
            updated_at: Some(1),
            message_count: 1,
            rename_title: None,
            title: Some("Thread session".to_string()),
            first_user_message: None,
            file_path: "/tmp/session.jsonl".to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        }];
        let task = teamwork_task();

        let context =
            build_thread_assistant_task_context(&thread, "assistant-codex", &task).unwrap();

        assert_eq!(
            context.snapshot["contextPolicy"]["mode"],
            Value::String("isolated_lane".to_string())
        );
        assert_eq!(
            context.snapshot["contextPolicy"]["crossCheckVisibility"],
            Value::String("stage_artifact_only".to_string())
        );
        assert!(context.snapshot["threadSessionRefs"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(context.snapshot["relatedContext"]["sessionExcerptRefs"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(context.prompt.contains("isolated debate lane"));
        assert!(context
            .prompt
            .contains("Do not assume access to another lane's full transcript"));
        assert!(!context
            .prompt
            .contains("Treat this as shared-context teamwork"));
    }

    #[test]
    fn plan_task_snapshot_context_uses_persisted_task_snapshots() {
        let mut thread = teamwork_thread();
        thread.assistants[0].name = "Current Builder".to_string();
        thread.assistants[0].system_prompt =
            Some("Current instructions should not win.".to_string());
        let mut task = teamwork_task();
        task.plan_task_id = Some("plan-task-1".to_string());
        let context = build_plan_task_snapshot_context(
            &thread,
            &task,
            None,
            Some(r#"{"assistantId":"assistant-codex","name":"Original Builder","systemPrompt":"Original persisted instructions."}"#),
            Some(r#"{"agent":"codex","agentInfo":{"name":"Codex","model":"gpt-5.3-codex-original"}}"#),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            context.snapshot["contextPolicy"]["mode"],
            Value::String("persisted_plan_task_snapshot".to_string())
        );
        assert_eq!(
            context.snapshot["assistantSnapshot"]["name"],
            Value::String("Original Builder".to_string())
        );
        assert!(context.prompt.contains("Original Builder"));
        assert!(context.prompt.contains("Original persisted instructions."));
        assert!(context.prompt.contains("gpt-5.3-codex-original"));
        assert!(!context.prompt.contains("Current Builder"));
        assert!(!context
            .prompt
            .contains("Current instructions should not win."));
    }

    #[test]
    fn process_plan_task_snapshot_context_keeps_stage_rollup() {
        let thread = thread();
        let mut task = task();
        task.plan_task_id = Some("plan-task-1".to_string());
        task.title = "writing / Writer".to_string();
        let context = build_plan_task_snapshot_context(
            &thread,
            &task,
            Some(r#"{"id":"project-stage-1","name":"Writing"}"#),
            Some(r#"{"assistantId":"assistant-1","name":"Writer"}"#),
            Some(r#"{"agent":"codex","agentInfo":{"displayName":"Codex CLI"}}"#),
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            context.snapshot["contextPolicy"]["mode"],
            Value::String("persisted_plan_task_snapshot".to_string())
        );
        assert_eq!(
            context.snapshot["focusedStageId"],
            Value::String("stage-1".to_string())
        );
        assert_eq!(context.snapshot["rollup"]["total"], Value::from(1));
        assert_eq!(
            context.snapshot["rollup"]["currentStage"],
            Value::String("Research".to_string())
        );
        assert_eq!(
            context.snapshot["stages"][0]["threadStageId"],
            Value::String("stage-1".to_string())
        );
        assert_eq!(
            context.snapshot["task"]["title"],
            Value::String("writing / Writer".to_string())
        );
    }
}
