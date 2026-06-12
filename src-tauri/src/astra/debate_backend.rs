use serde_json::{json, Value};

use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::debate_judge::{
    DebateJudge, JudgeLaneArtifact, JudgeStatus, JudgeVerdict, HEURISTIC_JUDGE_BACKEND_TYPE,
};
use super::prompt::wrap_thread_prompt;
use super::{
    final_task_output, short_hash, summarize_task_output, AstraOrchestration, AstraRun,
    AstraRunIntent, AstraTaskCompletion, AstraTaskProposal, AstraTaskResultStatus, AstraTaskRisk,
};
use crate::models::{PlanRoundMode, ThreadAgentInfo, ThreadInfo, ThreadKind};

const DEBATE_BACKEND_TYPE: &str = "debate_backend";
const CROSS_CHECK_MARKER: &str = "## Cross-check artifacts";
const JUDGE_FEEDBACK_MARKER: &str = "## Judge feedback";

pub struct DebateBackend {
    judge: Box<dyn DebateJudge>,
}

impl DebateBackend {
    pub fn new(judge: Box<dyn DebateJudge>) -> Self {
        Self { judge }
    }
}

impl OrchestratorBackend for DebateBackend {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        _config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure> {
        if thread.kind != ThreadKind::Debate {
            return Err(BackendFailure::new(
                DEBATE_BACKEND_TYPE,
                "unsupported_thread_kind",
                "debate backend only supports debate threads",
            ));
        }

        let (orchestration, judge_session_id) = debate_orchestration(
            run,
            thread,
            user_prompt,
            round_index,
            completions,
            self.judge.as_ref(),
        );
        Ok(BackendResponse {
            data: orchestration,
            session_id: judge_session_id
                .unwrap_or_else(|| format!("debate-backend-{}-{}", run.run_id, round_index)),
            backend_type: DEBATE_BACKEND_TYPE.to_string(),
        })
    }
}

fn debate_orchestration(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    completions: &[AstraTaskCompletion],
    judge: &dyn DebateJudge,
) -> (AstraOrchestration, Option<String>) {
    if let Some(failure) = completions.iter().find(|completion| {
        matches!(
            completion.result.status,
            AstraTaskResultStatus::Failed
                | AstraTaskResultStatus::Errored
                | AstraTaskResultStatus::Cancelled
        )
    }) {
        return (
            AstraOrchestration {
                summary: format!(
                    "Debate stopped because lane task {} ended with {}.",
                    failure.task.title,
                    failure.result.status.as_str()
                ),
                run_intent: AstraRunIntent::Error,
                reason: "debate_task_failed".to_string(),
                mode: None,
                tasks: Vec::new(),
                diagnostics: Vec::new(),
            },
            None,
        );
    }

    if completions.is_empty() {
        let tasks = debate_lane_tasks(run, thread, user_prompt, round_index, None, None);
        return if tasks.len() < 2 {
            (
                AstraOrchestration {
                    summary: "Debate requires at least two agent participants.".to_string(),
                    run_intent: AstraRunIntent::WaitForHuman,
                    reason: "debate_needs_two_lanes".to_string(),
                    mode: None,
                    tasks,
                    diagnostics: Vec::new(),
                },
                None,
            )
        } else {
            (
                AstraOrchestration {
                    summary: format!(
                        "Debate round {} starts {} isolated lane{}.",
                        round_index + 1,
                        tasks.len(),
                        if tasks.len() == 1 { "" } else { "s" }
                    ),
                    run_intent: AstraRunIntent::Continue,
                    reason: "debate_isolated_lanes".to_string(),
                    mode: Some(PlanRoundMode::Parallel),
                    tasks,
                    diagnostics: Vec::new(),
                },
                None,
            )
        };
    }

    let artifact_set = lane_artifact_set(thread, round_index.saturating_sub(1), completions);
    if has_cross_check_marker(completions) {
        let judge_artifacts = judge_lane_artifacts(completions);
        let (verdict, meta) = match judge.judge(
            run,
            thread,
            user_prompt,
            round_index.saturating_sub(1),
            &judge_artifacts,
        ) {
            Ok(response) => {
                let meta = JudgeMeta {
                    backend: response.backend_type.clone(),
                    session_id: Some(response.session_id.clone()),
                    error: None,
                };
                (response.data, meta)
            }
            Err(failure) => {
                log::warn!(
                    "[astra:debate:judge-failure] run={} backend={} code={} message={}",
                    run.run_id,
                    failure.backend_type,
                    failure.code,
                    failure.message
                );
                let meta = JudgeMeta {
                    backend: failure.backend_type.clone(),
                    session_id: failure.session_id.clone(),
                    error: Some((failure.code, failure.message.clone())),
                };
                (degraded_judge_verdict(), meta)
            }
        };
        let judge_session_id = (meta.backend != HEURISTIC_JUDGE_BACKEND_TYPE
            && meta.error.is_none())
        .then(|| meta.session_id.clone())
        .flatten();
        let convergence = convergence_diagnostic(
            thread,
            round_index.saturating_sub(1),
            &artifact_set,
            &verdict,
            &meta,
        );
        if verdict.status == JudgeStatus::Converged {
            return (
                AstraOrchestration {
                    summary: "Debate cross-check converged; convergence diagnostics are recorded."
                        .to_string(),
                    run_intent: AstraRunIntent::Complete,
                    reason: "debate_cross_check_converged".to_string(),
                    mode: None,
                    tasks: Vec::new(),
                    diagnostics: vec![artifact_set, convergence],
                },
                judge_session_id,
            );
        }

        let status = verdict.status.as_str();
        if !has_room_for_terminal_after_followup(run, round_index) {
            let mut terminal = convergence;
            if let Some(record) = terminal.as_object_mut() {
                record.insert(
                    "terminalReason".to_string(),
                    Value::String("round_limit_reached".to_string()),
                );
                record.insert("roundLimit".to_string(), json!(run.round_limit));
                record.insert(
                    "nextAction".to_string(),
                    Value::String(
                        "Preserve consensus, disagreements, and arbitration recommendation for human review."
                            .to_string(),
                    ),
                );
            }
            return (
                AstraOrchestration {
                    summary: format!(
                        "Debate reached the round limit with {} cross-check status; diagnostics preserve the remaining disagreements.",
                        status
                    ),
                    run_intent: AstraRunIntent::Complete,
                    reason: "debate_round_limit_reached".to_string(),
                    mode: None,
                    tasks: Vec::new(),
                    diagnostics: vec![artifact_set, terminal],
                },
                judge_session_id,
            );
        }

        let tasks = debate_lane_tasks(
            run,
            thread,
            user_prompt,
            round_index,
            Some(&artifact_set),
            Some(&verdict),
        );
        if tasks.is_empty() {
            return (
                AstraOrchestration {
                    summary:
                        "Debate cross-check needs another pass, but no participants are available."
                            .to_string(),
                    run_intent: AstraRunIntent::WaitForHuman,
                    reason: "debate_no_cross_check_lanes".to_string(),
                    mode: None,
                    tasks,
                    diagnostics: vec![artifact_set, convergence],
                },
                judge_session_id,
            );
        }

        return (
            AstraOrchestration {
                summary: format!(
                    "Debate cross-check is {}; another cross-check round will exchange stage artifacts only.",
                    status
                ),
                run_intent: AstraRunIntent::Continue,
                reason: "debate_need_more_cross_check".to_string(),
                mode: Some(PlanRoundMode::Parallel),
                tasks,
                diagnostics: vec![artifact_set, convergence],
            },
            judge_session_id,
        );
    }

    if !has_room_for_terminal_after_followup(run, round_index) {
        let verdict = JudgeVerdict {
            status: JudgeStatus::NeedsReview,
            agreements: Vec::new(),
            disagreements: Vec::new(),
            arbitration: None,
            rationale:
                "Round limit reached before a cross-check round; no judge verdict was produced."
                    .to_string(),
            attempts: 0,
        };
        let meta = JudgeMeta {
            backend: "none".to_string(),
            session_id: None,
            error: None,
        };
        let mut convergence = convergence_diagnostic(
            thread,
            round_index.saturating_sub(1),
            &artifact_set,
            &verdict,
            &meta,
        );
        if let Some(record) = convergence.as_object_mut() {
            record.insert(
                "terminalReason".to_string(),
                Value::String("round_limit_reached_before_cross_check".to_string()),
            );
            record.insert("roundLimit".to_string(), json!(run.round_limit));
            record.insert(
                "nextAction".to_string(),
                Value::String(
                    "Review isolated lane artifacts manually; no round budget remains for cross-check."
                        .to_string(),
                ),
            );
        }
        return (
            AstraOrchestration {
                summary:
                    "Debate reached the round limit before a cross-check round; isolated lane artifacts are recorded."
                        .to_string(),
                run_intent: AstraRunIntent::Complete,
                reason: "debate_round_limit_reached".to_string(),
                mode: None,
                tasks: Vec::new(),
                diagnostics: vec![artifact_set, convergence],
            },
            None,
        );
    }

    let tasks = debate_lane_tasks(
        run,
        thread,
        user_prompt,
        round_index,
        Some(&artifact_set),
        None,
    );
    if tasks.is_empty() {
        return (
            AstraOrchestration {
                summary:
                    "Debate lane artifacts are ready, but no participants are available for cross-check."
                        .to_string(),
                run_intent: AstraRunIntent::WaitForHuman,
                reason: "debate_no_cross_check_lanes".to_string(),
                mode: None,
                tasks,
                diagnostics: vec![artifact_set],
            },
            None,
        );
    }

    (
        AstraOrchestration {
            summary: "Debate lane artifacts generated; next round cross-checks stage artifacts only."
                .to_string(),
            run_intent: AstraRunIntent::Continue,
            reason: "debate_cross_check_ready".to_string(),
            mode: Some(PlanRoundMode::Parallel),
            tasks,
            diagnostics: vec![artifact_set],
        },
        None,
    )
}

fn has_room_for_terminal_after_followup(run: &AstraRun, round_index: u32) -> bool {
    round_index.saturating_add(1) < run.round_limit
}

fn debate_lane_tasks(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    artifact_set: Option<&Value>,
    judge_verdict: Option<&JudgeVerdict>,
) -> Vec<AstraTaskProposal> {
    let mut participants = thread.agent_participants.clone();
    participants.sort_by_key(|participant| participant.order);
    participants
        .iter()
        .map(|participant| {
            let target_agent = participant.agent;
            let cross_check_round = artifact_set.is_some();
            let lane_id = lane_id(participant);
            let task_id = format!(
                "task-{}",
                short_hash(&format!(
                    "{}:{}:{}:{}:{}",
                    run.thread_id,
                    participant.participant_id,
                    target_agent.as_str(),
                    round_index,
                    if cross_check_round { "cross-check" } else { "lane" }
                ))
            );
            let mut task = AstraTaskProposal {
                id: task_id,
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: Some(participant.participant_id.clone()),
                title: if cross_check_round {
                    format!("{} debate cross-check", participant_label(participant))
                } else {
                    format!("{} debate lane", participant_label(participant))
                },
                target_stage_id: None,
                target_agent,
                prompt: debate_task_prompt(
                    thread,
                    user_prompt,
                    participant,
                    artifact_set,
                    judge_verdict,
                ),
                expected_output: if cross_check_round {
                    "Cross-check report with challenged claims, agreement points, disagreements, and convergence recommendation."
                        .to_string()
                } else {
                    "Isolated lane artifact with answer, evidence, assumptions, confidence, and falsification criteria."
                        .to_string()
                },
                risk: AstraTaskRisk::Low,
                depends_on: Vec::new(),
            };
            task.prompt.push_str(&format!("\n\nLane id: {lane_id}"));
            task.prompt = wrap_thread_prompt(
                "astra_debate_participant_task",
                thread,
                task.prompt,
                &[
                    ("participant_id", participant.participant_id.clone()),
                    ("target_agent", participant.agent.as_str().to_string()),
                    (
                        "round_role",
                        if cross_check_round { "cross_check" } else { "lane" }.to_string(),
                    ),
                ],
            );
            task
        })
        .collect()
}

fn debate_task_prompt(
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    participant: &ThreadAgentInfo,
    artifact_set: Option<&Value>,
    judge_verdict: Option<&JudgeVerdict>,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio debate task".to_string());
    lines.push(String::new());
    lines.push(format!("Thread goal: {}", thread.goal));
    if let Some(description) = thread
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Thread description: {description}"));
    }
    if let Some(prompt) = user_prompt.map(str::trim).filter(|value| !value.is_empty()) {
        lines.push(format!("User debate instruction: {prompt}"));
    }
    lines.push(format!(
        "Lane participant: {}",
        participant_label(participant)
    ));
    lines.push(format!("Participant id: {}", participant.participant_id));
    lines.push(format!("Runtime agent: {}", participant.agent.as_str()));
    if !participant.model.trim().is_empty() {
        lines.push(format!("Model: {}", participant.model));
    }
    if !participant.effort.trim().is_empty() {
        lines.push(format!("Effort: {}", participant.effort));
    }
    if !participant.permission_mode.trim().is_empty() {
        lines.push(format!("Permission mode: {}", participant.permission_mode));
    }
    lines.push(format!("Lane id: {}", lane_id(participant)));
    lines.push(String::new());

    if let Some(artifact_set) = artifact_set {
        let visible = visible_artifacts_for_participant(artifact_set, &participant.participant_id);
        lines.push(CROSS_CHECK_MARKER.to_string());
        lines.push(board_text(&json!({
            "visibleArtifacts": visible,
            "visibilityRule": "Only stage artifacts are visible. Do not infer or request another lane's full transcript.",
        })));
        lines.push(String::new());
        lines.push("## Task".to_string());
        lines.push("Cross-check the visible stage artifacts. Identify agreement, disagreement, unsupported assumptions, and what would be required to converge. Do not use hidden lane transcripts.".to_string());
        if let Some(feedback) = judge_feedback_section(judge_verdict) {
            lines.push(String::new());
            lines.push(feedback);
        }
    } else {
        lines.push("## Isolation rule".to_string());
        lines.push("Work only from the initial thread goal, description, and user instruction. Treat this as an isolated lane: do not assume access to any other participant's reasoning or transcript.".to_string());
        lines.push(String::new());
        lines.push("## Task".to_string());
        lines.push("Produce your lane artifact: answer, evidence, assumptions, confidence, likely failure modes, and what evidence would change your view.".to_string());
    }
    lines.join("\n")
}

fn lane_artifact_set(
    thread: &ThreadInfo,
    source_round_index: u32,
    completions: &[AstraTaskCompletion],
) -> Value {
    let artifacts = completions
        .iter()
        .map(|completion| {
            let assistant_id = completion.task.assistant_id.as_deref().unwrap_or("");
            let participant_id = completion
                .task
                .agent_participant_id
                .as_deref()
                .unwrap_or("");
            json!({
                "laneId": if participant_id.is_empty() {
                    lane_id_for_participant_id(assistant_id)
                } else {
                    lane_id_for_participant_id(participant_id)
                },
                "participantId": completion.task.agent_participant_id,
                "assistantId": completion.task.assistant_id,
                "agent": completion.task.target_agent.as_str(),
                "taskId": completion.task.id,
                "title": completion.task.title,
                "status": completion.result.status.as_str(),
                "stageArtifact": summarize_task_output(&final_task_output(&completion.result.output)),
                "visibility": "stage_artifact_only",
            })
        })
        .collect::<Vec<_>>();
    json!({
        "kind": "debate_lane_artifacts",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "artifacts": artifacts,
        "isolationPolicy": {
            "laneContext": "isolated",
            "sharedInput": "thread_goal_and_user_instruction",
            "crossCheckVisibility": "stage_artifact_only",
            "hidden": "full_lane_transcripts"
        },
        "recordedAt": super::now_ms(),
    })
}

struct JudgeMeta {
    backend: String,
    session_id: Option<String>,
    error: Option<(&'static str, String)>,
}

fn degraded_judge_verdict() -> JudgeVerdict {
    JudgeVerdict {
        status: JudgeStatus::NeedsReview,
        agreements: Vec::new(),
        disagreements: Vec::new(),
        arbitration: None,
        rationale: "Convergence judge unavailable; defaulting to needs_review.".to_string(),
        attempts: 0,
    }
}

fn judge_lane_artifacts(completions: &[AstraTaskCompletion]) -> Vec<JudgeLaneArtifact> {
    completions
        .iter()
        .map(|completion| {
            let participant_id = completion
                .task
                .agent_participant_id
                .clone()
                .or_else(|| completion.task.assistant_id.clone());
            JudgeLaneArtifact {
                lane_id: lane_id_for_participant_id(participant_id.as_deref().unwrap_or("")),
                participant_id,
                agent: completion.task.target_agent.as_str().to_string(),
                output: final_task_output(&completion.result.output),
            }
        })
        .collect()
}

fn judge_feedback_section(judge_verdict: Option<&JudgeVerdict>) -> Option<String> {
    let verdict = judge_verdict?;
    if verdict.agreements.is_empty() && verdict.disagreements.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    lines.push(JUDGE_FEEDBACK_MARKER.to_string());
    if !verdict.agreements.is_empty() {
        lines.push("Settled points (do not relitigate):".to_string());
        for agreement in &verdict.agreements {
            lines.push(format!("- {agreement}"));
        }
    }
    if !verdict.disagreements.is_empty() {
        lines.push(
            "Unresolved disagreements — address each item explicitly: provide new evidence, concede, or propose a converging position:"
                .to_string(),
        );
        for (index, disagreement) in verdict.disagreements.iter().enumerate() {
            lines.push(format!("{}. {disagreement}", index + 1));
        }
    }
    if let Some(arbitration) = verdict
        .arbitration
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("Arbitration note: {arbitration}"));
    }
    Some(lines.join("\n"))
}

fn convergence_diagnostic(
    thread: &ThreadInfo,
    source_round_index: u32,
    artifact_set: &Value,
    verdict: &JudgeVerdict,
    meta: &JudgeMeta,
) -> Value {
    let mut diagnostic = json!({
        "kind": "debate_convergence",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "status": verdict.status.as_str(),
        "artifactCount": artifact_set
            .get("artifacts")
            .and_then(Value::as_array)
            .map(|values| values.len())
            .unwrap_or(0),
        "decision": match verdict.status {
            JudgeStatus::Converged => "All visible cross-check artifacts indicate agreement.",
            JudgeStatus::Diverged => "At least one visible cross-check artifact reports disagreement.",
            JudgeStatus::NeedsReview => "Cross-check completed without a clear convergence signal.",
        },
        "agreements": &verdict.agreements,
        "disagreements": &verdict.disagreements,
        "arbitration": &verdict.arbitration,
        "rationale": &verdict.rationale,
        "judgeBackend": &meta.backend,
        "judgeSessionId": &meta.session_id,
        "judgeAttempts": verdict.attempts,
        "recordedAt": super::now_ms(),
    });
    if let Some((code, message)) = &meta.error {
        if let Some(record) = diagnostic.as_object_mut() {
            record.insert(
                "judgeError".to_string(),
                json!({ "code": code, "message": message }),
            );
        }
    }
    diagnostic
}

fn visible_artifacts_for_participant(artifact_set: &Value, participant_id: &str) -> Vec<Value> {
    artifact_set
        .get("artifacts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|artifact| {
            artifact
                .get("participantId")
                .and_then(Value::as_str)
                .is_none_or(|value| value != participant_id)
        })
        .cloned()
        .collect()
}

fn has_cross_check_marker(completions: &[AstraTaskCompletion]) -> bool {
    completions
        .iter()
        .any(|completion| completion.task.prompt.contains(CROSS_CHECK_MARKER))
}

fn lane_id(participant: &ThreadAgentInfo) -> String {
    lane_id_for_participant_id(&participant.participant_id)
}

fn lane_id_for_participant_id(participant_id: &str) -> String {
    format!("lane-{}", short_hash(participant_id))
}

fn participant_label(participant: &ThreadAgentInfo) -> String {
    if participant.model.trim().is_empty() {
        participant.agent.as_str().to_string()
    } else {
        format!("{} {}", participant.agent.as_str(), participant.model)
    }
}

fn board_text(board: &Value) -> String {
    serde_json::to_string_pretty(board).unwrap_or_else(|_| board.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::astra::debate_judge::HeuristicJudge;
    use crate::astra::types::{AstraTaskResult, AstraTaskResultStatus};
    use crate::models::{Agent, ThreadAgentInfo};

    struct FakeJudge {
        result: Result<BackendResponse<JudgeVerdict>, BackendFailure>,
    }

    impl DebateJudge for FakeJudge {
        fn judge(
            &self,
            _run: &AstraRun,
            _thread: &ThreadInfo,
            _user_prompt: Option<&str>,
            _source_round_index: u32,
            _artifacts: &[JudgeLaneArtifact],
        ) -> Result<BackendResponse<JudgeVerdict>, BackendFailure> {
            self.result.clone()
        }
    }

    fn judge_verdict(status: JudgeStatus) -> JudgeVerdict {
        JudgeVerdict {
            status,
            agreements: Vec::new(),
            disagreements: Vec::new(),
            arbitration: None,
            rationale: "test rationale".to_string(),
            attempts: 1,
        }
    }

    fn runtime_judge_response(verdict: JudgeVerdict) -> BackendResponse<JudgeVerdict> {
        BackendResponse {
            data: verdict,
            session_id: "agent-session-x".to_string(),
            backend_type: "runtime_agent_claude".to_string(),
        }
    }

    fn orchestrate_with_heuristic(
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
    ) -> AstraOrchestration {
        debate_orchestration(
            run,
            thread,
            user_prompt,
            round_index,
            completions,
            &HeuristicJudge,
        )
        .0
    }

    fn cross_check_completions(run: &AstraRun, output: &str) -> Vec<AstraTaskCompletion> {
        let first = orchestrate_with_heuristic(run, &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = orchestrate_with_heuristic(run, &thread(), None, 1, &first_completions);
        cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, output))
            .collect()
    }

    fn thread() -> ThreadInfo {
        ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Choose the safer architecture".to_string(),
            description: Some("Compare two proposals with evidence.".to_string()),
            stage_id: None,
            kind: ThreadKind::Debate,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            assistants: Vec::new(),
            agent_participants: vec![
                ThreadAgentInfo {
                    participant_id: "participant-a".to_string(),
                    agent: Agent::Codex,
                    model: "gpt-5.3-codex".to_string(),
                    effort: "medium".to_string(),
                    permission_mode: "read-write".to_string(),
                    order: 0,
                    created_at: 1,
                    updated_at: 1,
                },
                ThreadAgentInfo {
                    participant_id: "participant-b".to_string(),
                    agent: Agent::Claude,
                    model: "claude-sonnet-4-5".to_string(),
                    effort: "medium".to_string(),
                    permission_mode: "read-only".to_string(),
                    order: 1,
                    created_at: 1,
                    updated_at: 1,
                },
            ],
            stages: Vec::new(),
            sessions: Vec::new(),
        }
    }

    fn run() -> AstraRun {
        AstraRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp".to_string(),
            status: super::super::AstraRunStatus::Planning,
            mode: "rust_native".to_string(),
            planner_backend: Some(DEBATE_BACKEND_TYPE.to_string()),
            round_index: None,
            round_limit: 12,
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

    fn run_with_limit(round_limit: u32) -> AstraRun {
        AstraRun {
            round_limit,
            ..run()
        }
    }

    fn completion(task: AstraTaskProposal, output: &str) -> AstraTaskCompletion {
        AstraTaskCompletion {
            result: AstraTaskResult {
                task_id: task.id.clone(),
                thread_stage_id: None,
                sessio_runtime_session_id: "session-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                status: AstraTaskResultStatus::Completed,
                output: output.to_string(),
                error: None,
                attempt_count: 1,
                retry_limit_reached: false,
                completed_at: 2,
            },
            task,
        }
    }

    #[test]
    fn first_round_creates_isolated_lane_tasks() {
        let orchestration = orchestrate_with_heuristic(&run(), &thread(), Some("Be strict"), 0, &[]);

        assert_eq!(orchestration.run_intent, AstraRunIntent::Continue);
        assert_eq!(orchestration.mode, Some(PlanRoundMode::Parallel));
        assert_eq!(orchestration.tasks.len(), 2);
        assert_eq!(
            orchestration.tasks[0].agent_participant_id.as_deref(),
            Some("participant-a")
        );
        assert!(orchestration.tasks[0].assistant_id.is_none());
        assert!(orchestration.tasks[0].prompt.contains("## Isolation rule"));
        assert!(!orchestration.tasks[0].prompt.contains(CROSS_CHECK_MARKER));
    }

    #[test]
    fn completions_generate_artifacts_and_cross_check_tasks() {
        let first = orchestrate_with_heuristic(&run(), &thread(), None, 0, &[]);
        let completions = first
            .tasks
            .into_iter()
            .enumerate()
            .map(|(index, task)| {
                completion(
                    task,
                    if index == 0 {
                        "Final result: Proposal A is safer."
                    } else {
                        "Final result: Proposal B has better failure isolation."
                    },
                )
            })
            .collect::<Vec<_>>();

        let next = orchestrate_with_heuristic(&run(), &thread(), None, 1, &completions);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "debate_cross_check_ready");
        assert_eq!(next.diagnostics[0]["kind"], "debate_lane_artifacts");
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains(CROSS_CHECK_MARKER)));
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains("stage_artifact_only")));
        assert!(next
            .tasks
            .iter()
            .all(|task| !task.prompt.contains(JUDGE_FEEDBACK_MARKER)));
        let visible_for_a =
            visible_artifacts_for_participant(&next.diagnostics[0], "participant-a");
        assert!(visible_for_a
            .iter()
            .all(|artifact| artifact["participantId"] != "participant-a"));
        assert!(visible_for_a
            .iter()
            .any(|artifact| artifact["participantId"] == "participant-b"));
    }

    #[test]
    fn cross_check_completions_finish_with_convergence_diagnostic() {
        let first = orchestrate_with_heuristic(&run(), &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = orchestrate_with_heuristic(&run(), &thread(), None, 1, &first_completions);
        let cross_check_completions = cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: agree with caveats."))
            .collect::<Vec<_>>();

        let terminal = orchestrate_with_heuristic(&run(), &thread(), None, 2, &cross_check_completions);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "debate_cross_check_converged");
        assert!(terminal
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic["kind"] == "debate_convergence"));
        assert!(terminal.diagnostics.iter().any(|diagnostic| {
            diagnostic["kind"] == "debate_convergence" && diagnostic["status"] == "converged"
        }));
    }

    #[test]
    fn divergent_cross_check_continues_when_round_budget_remains() {
        let first = orchestrate_with_heuristic(&run(), &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = orchestrate_with_heuristic(&run(), &thread(), None, 1, &first_completions);
        let cross_check_completions = cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: disagree; claims conflict."))
            .collect::<Vec<_>>();

        let next = orchestrate_with_heuristic(&run(), &thread(), None, 2, &cross_check_completions);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "debate_need_more_cross_check");
        assert_eq!(next.mode, Some(PlanRoundMode::Parallel));
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains(CROSS_CHECK_MARKER)));
        assert!(next.diagnostics.iter().any(|diagnostic| {
            diagnostic["kind"] == "debate_convergence" && diagnostic["status"] == "diverged"
        }));
    }

    #[test]
    fn divergent_cross_check_finishes_with_round_limit_diagnostic() {
        let run = run_with_limit(3);
        let first = orchestrate_with_heuristic(&run, &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = orchestrate_with_heuristic(&run, &thread(), None, 1, &first_completions);
        let cross_check_completions = cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: disagree; assumptions conflict."))
            .collect::<Vec<_>>();

        let terminal = orchestrate_with_heuristic(&run, &thread(), None, 2, &cross_check_completions);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "debate_round_limit_reached");
        assert!(terminal.tasks.is_empty());
        assert!(terminal.diagnostics.iter().any(|diagnostic| {
            diagnostic["kind"] == "debate_convergence"
                && diagnostic["status"] == "diverged"
                && diagnostic["terminalReason"] == "round_limit_reached"
        }));
    }

    #[test]
    fn llm_judge_converged_completes_with_structured_diagnostic() {
        let run = run();
        let completions = cross_check_completions(&run, "Final result: 我们已达成一致。");
        let judge = FakeJudge {
            result: Ok(runtime_judge_response(JudgeVerdict {
                agreements: vec!["双方都接受方案A。".to_string()],
                rationale: "双方明确接受同一结论。".to_string(),
                ..judge_verdict(JudgeStatus::Converged)
            })),
        };

        let (terminal, judge_session_id) =
            debate_orchestration(&run, &thread(), None, 2, &completions, &judge);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "debate_cross_check_converged");
        assert_eq!(judge_session_id.as_deref(), Some("agent-session-x"));
        let convergence = terminal
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic["kind"] == "debate_convergence")
            .unwrap();
        assert_eq!(convergence["status"], "converged");
        assert_eq!(convergence["agreements"][0], "双方都接受方案A。");
        assert_eq!(convergence["rationale"], "双方明确接受同一结论。");
        assert_eq!(convergence["judgeBackend"], "runtime_agent_claude");
        assert_eq!(convergence["judgeSessionId"], "agent-session-x");
        assert_eq!(convergence["judgeAttempts"], 1);
        assert!(convergence.get("judgeError").is_none());
    }

    #[test]
    fn chinese_diverged_verdict_injects_judge_feedback_into_next_round() {
        let run = run();
        let completions = cross_check_completions(&run, "我不接受对方的延迟结论。");
        let disagreement = "方案A的延迟数据缺乏基准来源，需要补充测量方法。";
        let judge = FakeJudge {
            result: Ok(runtime_judge_response(JudgeVerdict {
                agreements: vec!["双方都认可需要异步架构。".to_string()],
                disagreements: vec![disagreement.to_string()],
                arbitration: Some("建议由人工复核延迟基准。".to_string()),
                rationale: "延迟证据仍有实质分歧。".to_string(),
                ..judge_verdict(JudgeStatus::Diverged)
            })),
        };

        let (next, judge_session_id) =
            debate_orchestration(&run, &thread(), None, 2, &completions, &judge);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "debate_need_more_cross_check");
        assert_eq!(judge_session_id.as_deref(), Some("agent-session-x"));
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains(JUDGE_FEEDBACK_MARKER)));
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains(&format!("1. {disagreement}"))));
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains("Arbitration note: 建议由人工复核延迟基准。")));
        assert!(next.diagnostics.iter().any(|diagnostic| {
            diagnostic["kind"] == "debate_convergence"
                && diagnostic["status"] == "diverged"
                && diagnostic["disagreements"][0] == disagreement
        }));
    }

    #[test]
    fn judge_failure_degrades_to_needs_review_and_continues() {
        let run = run();
        let completions = cross_check_completions(&run, "Final result: 还需要继续讨论。");
        let judge = FakeJudge {
            result: Err(BackendFailure::new(
                "runtime_agent_claude",
                "timeout",
                "judge timed out",
            )
            .with_session_id(Some("judge-session-err".to_string()))),
        };

        let (next, judge_session_id) =
            debate_orchestration(&run, &thread(), None, 2, &completions, &judge);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "debate_need_more_cross_check");
        assert!(judge_session_id.is_none());
        assert!(next
            .tasks
            .iter()
            .all(|task| !task.prompt.contains(JUDGE_FEEDBACK_MARKER)));
        let convergence = next
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic["kind"] == "debate_convergence")
            .unwrap();
        assert_eq!(convergence["status"], "needs_review");
        assert_eq!(convergence["judgeError"]["code"], "timeout");
        assert_eq!(convergence["judgeError"]["message"], "judge timed out");
        assert_eq!(convergence["judgeSessionId"], "judge-session-err");
        assert_eq!(convergence["judgeAttempts"], 0);
    }

    #[test]
    fn diverged_round_limit_terminal_preserves_judge_disagreements() {
        let run = run_with_limit(3);
        let completions = cross_check_completions(&run, "我不接受对方结论。");
        let disagreement = "对方未回应安全性质疑。";
        let judge = FakeJudge {
            result: Ok(runtime_judge_response(JudgeVerdict {
                disagreements: vec![disagreement.to_string()],
                ..judge_verdict(JudgeStatus::Diverged)
            })),
        };

        let (terminal, _) = debate_orchestration(&run, &thread(), None, 2, &completions, &judge);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "debate_round_limit_reached");
        assert!(terminal.tasks.is_empty());
        assert!(terminal.diagnostics.iter().any(|diagnostic| {
            diagnostic["kind"] == "debate_convergence"
                && diagnostic["terminalReason"] == "round_limit_reached"
                && diagnostic["disagreements"][0] == disagreement
        }));
    }

    #[test]
    fn orchestrate_propagates_runtime_judge_session_id_only() {
        let run = run();
        let completions = cross_check_completions(&run, "Final result: agree.");

        let runtime_backend = DebateBackend::new(Box::new(FakeJudge {
            result: Ok(runtime_judge_response(judge_verdict(JudgeStatus::Converged))),
        }));
        let response = runtime_backend
            .orchestrate(&run, &thread(), None, 2, &completions, &json!({}))
            .unwrap();
        assert_eq!(response.session_id, "agent-session-x");
        assert_eq!(response.backend_type, DEBATE_BACKEND_TYPE);

        let heuristic_backend = DebateBackend::new(Box::new(HeuristicJudge));
        let response = heuristic_backend
            .orchestrate(&run, &thread(), None, 2, &completions, &json!({}))
            .unwrap();
        assert_eq!(response.session_id, "debate-backend-run-1-2");
    }
}
