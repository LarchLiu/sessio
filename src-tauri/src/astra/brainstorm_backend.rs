use serde_json::{json, Value};

use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::prompt::wrap_thread_prompt;
use super::{
    final_task_output, short_hash, summarize_task_output, AstraOrchestration, AstraRun,
    AstraRunIntent, AstraTaskCompletion, AstraTaskProposal, AstraTaskResultStatus, AstraTaskRisk,
};
use crate::models::{PlanRoundMode, ThreadAgentInfo, ThreadInfo, ThreadKind};

const BRAINSTORM_BACKEND_TYPE: &str = "brainstorm_backend";

pub struct BrainstormBackend;

impl OrchestratorBackend for BrainstormBackend {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        _config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure> {
        if thread.kind != ThreadKind::Brainstorm {
            return Err(BackendFailure::new(
                BRAINSTORM_BACKEND_TYPE,
                "unsupported_thread_kind",
                "brainstorm backend only supports brainstorm threads",
            ));
        }

        let orchestration =
            brainstorm_orchestration(run, thread, user_prompt, round_index, completions);
        Ok(BackendResponse {
            data: orchestration,
            session_id: format!("brainstorm-backend-{}-{}", run.run_id, round_index),
            backend_type: BRAINSTORM_BACKEND_TYPE.to_string(),
        })
    }
}

fn brainstorm_orchestration(
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
                "Brainstorm stopped because task {} ended with {}.",
                failure.task.title,
                failure.result.status.as_str()
            ),
            run_intent: AstraRunIntent::Error,
            reason: "brainstorm_task_failed".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: Vec::new(),
        };
    }

    if completions.is_empty() {
        let tasks = brainstorm_divergence_tasks(run, thread, user_prompt, round_index, None, false);
        return if tasks.is_empty() {
            AstraOrchestration {
                summary: "Brainstorm needs at least one agent participant.".to_string(),
                run_intent: AstraRunIntent::WaitForHuman,
                reason: "brainstorm_no_participants".to_string(),
                mode: None,
                tasks,
                diagnostics: Vec::new(),
            }
        } else {
            AstraOrchestration {
                summary: format!(
                    "Brainstorm round {} asks {} participant{} for independent opinions.",
                    round_index + 1,
                    tasks.len(),
                    if tasks.len() == 1 { "" } else { "s" }
                ),
                run_intent: AstraRunIntent::Continue,
                reason: "brainstorm_parallel_opinions".to_string(),
                mode: Some(PlanRoundMode::Parallel),
                tasks,
                diagnostics: Vec::new(),
            }
        };
    }

    let board = shared_board_value(thread, round_index.saturating_sub(1), completions);
    if has_board_injection(completions) {
        return AstraOrchestration {
            summary: "Brainstorm synthesis completed from the shared board.".to_string(),
            run_intent: AstraRunIntent::Complete,
            reason: "brainstorm_synthesis_complete".to_string(),
            mode: None,
            tasks: Vec::new(),
            diagnostics: vec![
                board.clone(),
                synthesis_diagnostic(thread, round_index.saturating_sub(1), &board),
            ],
        };
    }

    let tasks =
        brainstorm_divergence_tasks(run, thread, user_prompt, round_index, Some(&board), true);
    if tasks.is_empty() {
        return AstraOrchestration {
            summary:
                "Brainstorm shared board is ready, but no participants are available for synthesis."
                    .to_string(),
            run_intent: AstraRunIntent::WaitForHuman,
            reason: "brainstorm_no_synthesis_participants".to_string(),
            mode: None,
            tasks,
            diagnostics: vec![board],
        };
    }

    AstraOrchestration {
        summary: "Brainstorm shared board generated; next round injects the board for synthesis."
            .to_string(),
        run_intent: AstraRunIntent::Continue,
        reason: "brainstorm_shared_board_ready".to_string(),
        mode: Some(PlanRoundMode::Parallel),
        tasks,
        diagnostics: vec![board],
    }
}

fn brainstorm_divergence_tasks(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    shared_board: Option<&Value>,
    synthesis_round: bool,
) -> Vec<AstraTaskProposal> {
    let mut participants = thread.agent_participants.clone();
    participants.sort_by_key(|participant| participant.order);
    participants
        .into_iter()
        .map(|participant| {
            let target_agent = participant.agent;
            let task_id = format!(
                "task-{}",
                short_hash(&format!(
                    "{}:{}:{}:{}:{}",
                    run.thread_id,
                    participant.participant_id,
                    target_agent.as_str(),
                    round_index,
                    if synthesis_round { "synthesis" } else { "diverge" }
                ))
            );
            let prompt = brainstorm_task_prompt(
                thread,
                user_prompt,
                &participant,
                shared_board,
                synthesis_round,
            );
            AstraTaskProposal {
                id: task_id,
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: Some(participant.participant_id.clone()),
                title: if synthesis_round {
                    format!("{} brainstorm synthesis", participant_label(&participant))
                } else {
                    format!("{} brainstorm opinion", participant_label(&participant))
                },
                target_stage_id: None,
                target_agent,
                prompt,
                expected_output: if synthesis_round {
                    "Synthesis result with candidates, consensus, disagreements, and recommendation."
                        .to_string()
                } else {
                    "Independent brainstorm opinion with rationale, opportunities, risks, and questions."
                        .to_string()
                },
                risk: AstraTaskRisk::Low,
            }
        })
        .collect()
}

fn brainstorm_task_prompt(
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    participant: &ThreadAgentInfo,
    shared_board: Option<&Value>,
    synthesis_round: bool,
) -> String {
    let mut lines = Vec::new();
    lines.push("# Sessio brainstorm task".to_string());
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
        lines.push(format!("User brainstorm instruction: {prompt}"));
    }
    lines.push(format!("Participant: {}", participant_label(participant)));
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
    lines.push(String::new());
    if let Some(board) = shared_board {
        lines.push("## Shared board from previous round".to_string());
        lines.push(board_text(board));
        lines.push(String::new());
    }
    lines.push("## Task".to_string());
    if synthesis_round {
        lines.push("Use the shared board above as explicit context. Synthesize the candidates, consensus, disagreements, risks, and a recommendation. Extend or challenge the board where useful.".to_string());
    } else {
        lines.push("Produce an independent opinion. Offer concrete ideas, rationale, risks, conflicts, and questions. Do not wait for other participants.".to_string());
    }
    wrap_thread_prompt(
        "astra_brainstorm_participant_task",
        thread,
        lines.join("\n"),
        &[
            ("participant_id", participant.participant_id.clone()),
            ("target_agent", participant.agent.as_str().to_string()),
            (
                "round_role",
                if synthesis_round {
                    "synthesis"
                } else {
                    "divergence"
                }
                .to_string(),
            ),
        ],
    )
}

fn participant_label(participant: &ThreadAgentInfo) -> String {
    if participant.model.trim().is_empty() {
        participant.agent.as_str().to_string()
    } else {
        format!("{} {}", participant.agent.as_str(), participant.model)
    }
}

fn shared_board_value(
    thread: &ThreadInfo,
    source_round_index: u32,
    completions: &[AstraTaskCompletion],
) -> Value {
    let opinions = completions
        .iter()
        .map(|completion| {
            let output = summarize_task_output(&final_task_output(&completion.result.output));
            json!({
                "taskId": completion.task.id,
                "participantId": completion.task.agent_participant_id,
                "agent": completion.task.target_agent.as_str(),
                "title": completion.task.title,
                "status": completion.result.status.as_str(),
                "opinion": output,
            })
        })
        .collect::<Vec<_>>();
    let highlights = opinions
        .iter()
        .filter_map(|opinion| opinion.get("opinion").and_then(Value::as_str))
        .take(6)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    json!({
        "kind": "brainstorm_shared_board",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "opinions": opinions,
        "highlights": highlights,
        "conflicts": ["Compare assumptions and tradeoffs across the opinions."],
        "openQuestions": ["Which candidate best satisfies the thread goal?", "What evidence would change the recommendation?"],
        "recordedAt": super::now_ms(),
    })
}

fn synthesis_diagnostic(thread: &ThreadInfo, source_round_index: u32, board: &Value) -> Value {
    json!({
        "kind": "brainstorm_synthesis",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "sharedBoardOpinionCount": board
            .get("opinions")
            .and_then(Value::as_array)
            .map(|values| values.len())
            .unwrap_or(0),
        "recordedAt": super::now_ms(),
    })
}

fn board_text(board: &Value) -> String {
    serde_json::to_string_pretty(board).unwrap_or_else(|_| board.to_string())
}

fn has_board_injection(completions: &[AstraTaskCompletion]) -> bool {
    completions.iter().any(|completion| {
        completion
            .task
            .prompt
            .contains("## Shared board from previous round")
    })
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
            goal: "Choose a product direction".to_string(),
            description: Some("Explore and synthesize options.".to_string()),
            stage_id: None,
            kind: ThreadKind::Brainstorm,
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
            planner_backend: Some(BRAINSTORM_BACKEND_TYPE.to_string()),
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
    fn first_round_creates_parallel_opinion_tasks() {
        let orchestration =
            brainstorm_orchestration(&run(), &thread(), Some("Be practical"), 0, &[]);

        assert_eq!(orchestration.run_intent, AstraRunIntent::Continue);
        assert_eq!(orchestration.mode, Some(PlanRoundMode::Parallel));
        assert_eq!(orchestration.tasks.len(), 2);
        assert_eq!(
            orchestration.tasks[0].agent_participant_id.as_deref(),
            Some("participant-a")
        );
        assert!(orchestration.tasks[0].assistant_id.is_none());
        assert!(orchestration.diagnostics.is_empty());
        assert!(orchestration.tasks[0]
            .prompt
            .contains("Produce an independent opinion"));
    }

    #[test]
    fn completions_generate_shared_board_and_injected_next_round() {
        let first = brainstorm_orchestration(&run(), &thread(), None, 0, &[]);
        let completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Option A has strong upside."))
            .collect::<Vec<_>>();

        let next = brainstorm_orchestration(&run(), &thread(), None, 1, &completions);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "brainstorm_shared_board_ready");
        assert_eq!(next.diagnostics[0]["kind"], "brainstorm_shared_board");
        assert_eq!(
            next.diagnostics[0]["opinions"][0]["participantId"],
            "participant-a"
        );
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains("## Shared board from previous round")));
    }

    #[test]
    fn injected_round_completions_finish_with_synthesis_diagnostic() {
        let first = brainstorm_orchestration(&run(), &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Option A."))
            .collect::<Vec<_>>();
        let synthesis = brainstorm_orchestration(&run(), &thread(), None, 1, &first_completions);
        let synthesis_completions = synthesis
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Recommend Option A."))
            .collect::<Vec<_>>();

        let terminal = brainstorm_orchestration(&run(), &thread(), None, 2, &synthesis_completions);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "brainstorm_synthesis_complete");
        assert!(terminal
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic["kind"] == "brainstorm_synthesis"));
    }
}
