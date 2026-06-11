use serde_json::{json, Value};

use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::prompt::wrap_thread_prompt;
use super::{
    final_task_output, short_hash, summarize_task_output, AstraOrchestration, AstraRun,
    AstraRunIntent, AstraTaskCompletion, AstraTaskProposal, AstraTaskResultStatus, AstraTaskRisk,
};
use crate::models::{PlanRoundMode, ThreadAgentInfo, ThreadInfo, ThreadKind};

const DEBATE_BACKEND_TYPE: &str = "debate_backend";
const CROSS_CHECK_MARKER: &str = "## Cross-check artifacts";

pub struct DebateBackend;

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

        let orchestration =
            debate_orchestration(run, thread, user_prompt, round_index, completions);
        Ok(BackendResponse {
            data: orchestration,
            session_id: format!("debate-backend-{}-{}", run.run_id, round_index),
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
) -> AstraOrchestration {
    if let Some(failure) = completions.iter().find(|completion| {
        matches!(
            completion.result.status,
            AstraTaskResultStatus::Failed
                | AstraTaskResultStatus::Errored
                | AstraTaskResultStatus::Cancelled
        )
    }) {
        return AstraOrchestration {
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
        };
    }

    if completions.is_empty() {
        let tasks = debate_lane_tasks(run, thread, user_prompt, round_index, None);
        return if tasks.len() < 2 {
            AstraOrchestration {
                summary: "Debate requires at least two agent participants.".to_string(),
                run_intent: AstraRunIntent::WaitForHuman,
                reason: "debate_needs_two_lanes".to_string(),
                mode: None,
                tasks,
                diagnostics: Vec::new(),
            }
        } else {
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
            }
        };
    }

    let artifact_set = lane_artifact_set(thread, round_index.saturating_sub(1), completions);
    if has_cross_check_marker(completions) {
        let convergence =
            convergence_diagnostic(thread, round_index.saturating_sub(1), &artifact_set);
        let status = convergence
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("needs_review")
            .to_string();
        if status == "converged" {
            return AstraOrchestration {
                summary: "Debate cross-check converged; convergence diagnostics are recorded."
                    .to_string(),
                run_intent: AstraRunIntent::Complete,
                reason: "debate_cross_check_converged".to_string(),
                mode: None,
                tasks: Vec::new(),
                diagnostics: vec![artifact_set, convergence],
            };
        }

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
            return AstraOrchestration {
                summary: format!(
                    "Debate reached the round limit with {} cross-check status; diagnostics preserve the remaining disagreements.",
                    status
                ),
                run_intent: AstraRunIntent::Complete,
                reason: "debate_round_limit_reached".to_string(),
                mode: None,
                tasks: Vec::new(),
                diagnostics: vec![artifact_set, terminal],
            };
        }

        let tasks = debate_lane_tasks(run, thread, user_prompt, round_index, Some(&artifact_set));
        if tasks.is_empty() {
            return AstraOrchestration {
                summary:
                    "Debate cross-check needs another pass, but no participants are available."
                        .to_string(),
                run_intent: AstraRunIntent::WaitForHuman,
                reason: "debate_no_cross_check_lanes".to_string(),
                mode: None,
                tasks,
                diagnostics: vec![artifact_set, convergence],
            };
        }

        return AstraOrchestration {
            summary: format!(
                "Debate cross-check is {}; another cross-check round will exchange stage artifacts only.",
                status
            ),
            run_intent: AstraRunIntent::Continue,
            reason: "debate_need_more_cross_check".to_string(),
            mode: Some(PlanRoundMode::Parallel),
            tasks,
            diagnostics: vec![artifact_set, convergence],
        };
    }

    if !has_room_for_terminal_after_followup(run, round_index) {
        let mut convergence =
            convergence_diagnostic(thread, round_index.saturating_sub(1), &artifact_set);
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
        return AstraOrchestration {
            summary:
                "Debate reached the round limit before a cross-check round; isolated lane artifacts are recorded."
                    .to_string(),
            run_intent: AstraRunIntent::Complete,
            reason: "debate_round_limit_reached".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: vec![artifact_set, convergence],
        };
    }

    let tasks = debate_lane_tasks(run, thread, user_prompt, round_index, Some(&artifact_set));
    if tasks.is_empty() {
        return AstraOrchestration {
            summary:
                "Debate lane artifacts are ready, but no participants are available for cross-check."
                    .to_string(),
            run_intent: AstraRunIntent::WaitForHuman,
            reason: "debate_no_cross_check_lanes".to_string(),
            mode: None,
            tasks,
            diagnostics: vec![artifact_set],
        };
    }

    AstraOrchestration {
        summary: "Debate lane artifacts generated; next round cross-checks stage artifacts only."
            .to_string(),
        run_intent: AstraRunIntent::Continue,
        reason: "debate_cross_check_ready".to_string(),
        mode: Some(PlanRoundMode::Parallel),
        tasks,
        diagnostics: vec![artifact_set],
    }
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
                prompt: debate_task_prompt(thread, user_prompt, participant, artifact_set),
                expected_output: if cross_check_round {
                    "Cross-check report with challenged claims, agreement points, disagreements, and convergence recommendation."
                        .to_string()
                } else {
                    "Isolated lane artifact with answer, evidence, assumptions, confidence, and falsification criteria."
                        .to_string()
                },
                risk: AstraTaskRisk::Low,
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

fn convergence_diagnostic(
    thread: &ThreadInfo,
    source_round_index: u32,
    artifact_set: &Value,
) -> Value {
    let status = convergence_status(artifact_set);
    json!({
        "kind": "debate_convergence",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "status": status,
        "artifactCount": artifact_set
            .get("artifacts")
            .and_then(Value::as_array)
            .map(|values| values.len())
            .unwrap_or(0),
        "decision": match status {
            "converged" => "All visible cross-check artifacts indicate agreement.",
            "diverged" => "At least one visible cross-check artifact reports disagreement.",
            _ => "Cross-check completed without a clear convergence signal.",
        },
        "recordedAt": super::now_ms(),
    })
}

fn convergence_status(artifact_set: &Value) -> &'static str {
    let artifacts = artifact_set
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if artifacts.is_empty() {
        return "needs_review";
    }
    let texts = artifacts
        .iter()
        .filter_map(|artifact| artifact.get("stageArtifact").and_then(Value::as_str))
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if texts.iter().any(|text| {
        text.contains("disagree")
            || text.contains("diverge")
            || text.contains("conflict")
            || text.contains("reject")
    }) {
        return "diverged";
    }
    if !texts.is_empty()
        && texts.iter().all(|text| {
            text.contains("agree") || text.contains("converge") || text.contains("consensus")
        })
    {
        return "converged";
    }
    "needs_review"
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
    use crate::astra::types::{AstraTaskResult, AstraTaskResultStatus};
    use crate::models::{Agent, ThreadAgentInfo};

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
        let orchestration = debate_orchestration(&run(), &thread(), Some("Be strict"), 0, &[]);

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
        let first = debate_orchestration(&run(), &thread(), None, 0, &[]);
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

        let next = debate_orchestration(&run(), &thread(), None, 1, &completions);

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
        let first = debate_orchestration(&run(), &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = debate_orchestration(&run(), &thread(), None, 1, &first_completions);
        let cross_check_completions = cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: agree with caveats."))
            .collect::<Vec<_>>();

        let terminal = debate_orchestration(&run(), &thread(), None, 2, &cross_check_completions);

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
        let first = debate_orchestration(&run(), &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = debate_orchestration(&run(), &thread(), None, 1, &first_completions);
        let cross_check_completions = cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: disagree; claims conflict."))
            .collect::<Vec<_>>();

        let next = debate_orchestration(&run(), &thread(), None, 2, &cross_check_completions);

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
        let first = debate_orchestration(&run, &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Proposal A."))
            .collect::<Vec<_>>();
        let cross_check = debate_orchestration(&run, &thread(), None, 1, &first_completions);
        let cross_check_completions = cross_check
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: disagree; assumptions conflict."))
            .collect::<Vec<_>>();

        let terminal = debate_orchestration(&run, &thread(), None, 2, &cross_check_completions);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "debate_round_limit_reached");
        assert!(terminal.tasks.is_empty());
        assert!(terminal.diagnostics.iter().any(|diagnostic| {
            diagnostic["kind"] == "debate_convergence"
                && diagnostic["status"] == "diverged"
                && diagnostic["terminalReason"] == "round_limit_reached"
        }));
    }
}
