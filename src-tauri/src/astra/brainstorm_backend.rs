use serde_json::{json, Value};

use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::brainstorm_facilitator::{
    heuristic_board, heuristic_report, BrainstormFacilitator, FacilitatorBoard, FacilitatorOpinion,
    FacilitatorReport, HEURISTIC_FACILITATOR_BACKEND_TYPE,
};
use super::prompt::wrap_thread_prompt;
use super::{
    final_task_output, short_hash, summarize_task_output, AstraOrchestration, AstraRun,
    AstraRunIntent, AstraTaskCompletion, AstraTaskProposal, AstraTaskResultStatus, AstraTaskRisk,
};
use crate::models::{PlanRoundMode, ThreadAgentInfo, ThreadInfo, ThreadKind};

const BRAINSTORM_BACKEND_TYPE: &str = "brainstorm_backend";
const BOARD_INJECTION_MARKER: &str = "## Shared board from previous round";
/// Hard cap on critique rounds even when the facilitator keeps asking for
/// more; the round budget check below bounds it further.
const MAX_BRAINSTORM_CRITIQUE_ROUNDS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrainstormRole {
    Divergence,
    Critique,
    Synthesis,
}

impl BrainstormRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Divergence => "divergence",
            Self::Critique => "critique",
            Self::Synthesis => "synthesis",
        }
    }
}

/// Recovers the role of the round that produced these completions from the
/// `round_role` attribute embedded in the wrapped task prompt.
fn round_role_of(completions: &[AstraTaskCompletion]) -> Option<BrainstormRole> {
    let prompt = &completions.first()?.task.prompt;
    for role in [
        BrainstormRole::Synthesis,
        BrainstormRole::Critique,
        BrainstormRole::Divergence,
    ] {
        if prompt.contains(&format!("round_role=\"{}\"", role.as_str())) {
            return Some(role);
        }
    }
    None
}

pub struct BrainstormBackend {
    facilitator: Box<dyn BrainstormFacilitator>,
}

impl BrainstormBackend {
    pub fn new(facilitator: Box<dyn BrainstormFacilitator>) -> Self {
        Self { facilitator }
    }
}

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

        let (orchestration, facilitator_session_id) = brainstorm_orchestration(
            run,
            thread,
            user_prompt,
            round_index,
            completions,
            self.facilitator.as_ref(),
        );
        Ok(BackendResponse {
            data: orchestration,
            session_id: facilitator_session_id
                .unwrap_or_else(|| format!("brainstorm-backend-{}-{}", run.run_id, round_index)),
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
    facilitator: &dyn BrainstormFacilitator,
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
                    "Brainstorm stopped because task {} ended with {}.",
                    failure.task.title,
                    failure.result.status.as_str()
                ),
                run_intent: AstraRunIntent::Error,
                reason: "brainstorm_task_failed".to_string(),
                mode: None,
                tasks: Vec::new(),
                diagnostics: Vec::new(),
            },
            None,
        );
    }

    if completions.is_empty() {
        let tasks = brainstorm_participant_tasks(
            run,
            thread,
            user_prompt,
            round_index,
            None,
            BrainstormRole::Divergence,
        );
        return if tasks.is_empty() {
            (
                AstraOrchestration {
                    summary: "Brainstorm needs at least one agent participant.".to_string(),
                    run_intent: AstraRunIntent::WaitForHuman,
                    reason: "brainstorm_no_participants".to_string(),
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
                },
                None,
            )
        };
    }

    if round_role_of(completions) == Some(BrainstormRole::Synthesis) {
        let board = shared_board_value(
            run,
            thread,
            round_index.saturating_sub(1),
            completions,
            None,
            None,
        );
        let board_context = extract_board_context(completions);
        let syntheses = facilitator_opinions(completions);
        let (report, meta) = match facilitator.synthesize(
            run,
            thread,
            user_prompt,
            round_index.saturating_sub(1),
            board_context.as_deref(),
            &syntheses,
        ) {
            Ok(response) => {
                let meta = FacilitatorMeta {
                    backend: response.backend_type.clone(),
                    session_id: Some(response.session_id.clone()),
                    error: None,
                };
                (response.data, meta)
            }
            Err(failure) => {
                log::warn!(
                    "[astra:brainstorm:facilitator-failure] run={} backend={} purpose=synthesize code={} message={}",
                    run.run_id,
                    failure.backend_type,
                    failure.code,
                    failure.message
                );
                let meta = FacilitatorMeta {
                    backend: failure.backend_type.clone(),
                    session_id: failure.session_id.clone(),
                    error: Some((failure.code, failure.message.clone())),
                };
                (heuristic_report(&syntheses, 0), meta)
            }
        };
        let facilitator_session_id = facilitator_session_to_propagate(&meta);
        let synthesis = synthesis_diagnostic(
            thread,
            round_index.saturating_sub(1),
            &board,
            &report,
            &meta,
        );
        return (
            AstraOrchestration {
                summary: "Brainstorm synthesis completed from the shared board.".to_string(),
                run_intent: AstraRunIntent::Complete,
                reason: "brainstorm_synthesis_complete".to_string(),
                mode: None,
                tasks: Vec::new(),
                diagnostics: vec![board, synthesis],
            },
            facilitator_session_id,
        );
    }

    let opinions = facilitator_opinions(completions);
    let board_context = extract_board_context(completions);
    let (facilitator_board, meta) = match facilitator.build_board(
        run,
        thread,
        user_prompt,
        round_index.saturating_sub(1),
        board_context.as_deref(),
        &opinions,
    ) {
        Ok(response) => {
            let meta = FacilitatorMeta {
                backend: response.backend_type.clone(),
                session_id: Some(response.session_id.clone()),
                error: None,
            };
            (response.data, meta)
        }
        Err(failure) => {
            log::warn!(
                "[astra:brainstorm:facilitator-failure] run={} backend={} purpose=build_board code={} message={}",
                run.run_id,
                failure.backend_type,
                failure.code,
                failure.message
            );
            let meta = FacilitatorMeta {
                backend: failure.backend_type.clone(),
                session_id: failure.session_id.clone(),
                error: Some((failure.code, failure.message.clone())),
            };
            (heuristic_board(0), meta)
        }
    };
    let facilitator_session_id = facilitator_session_to_propagate(&meta);
    let board = shared_board_value(
        run,
        thread,
        round_index.saturating_sub(1),
        completions,
        Some(&facilitator_board),
        Some(&meta),
    );

    // A critique round needs budget for itself, the synthesis round, and the
    // terminal synthesis planning that follows it.
    let critique_rounds_so_far = round_index.saturating_sub(1);
    let next_role = if !facilitator_board.ready_to_synthesize
        && critique_rounds_so_far < MAX_BRAINSTORM_CRITIQUE_ROUNDS
        && round_index.saturating_add(2) < run.round_limit
    {
        BrainstormRole::Critique
    } else {
        BrainstormRole::Synthesis
    };
    let tasks = brainstorm_participant_tasks(
        run,
        thread,
        user_prompt,
        round_index,
        Some(&board),
        next_role,
    );
    if tasks.is_empty() {
        return (
            AstraOrchestration {
                summary:
                    "Brainstorm shared board is ready, but no participants are available for synthesis."
                        .to_string(),
                run_intent: AstraRunIntent::WaitForHuman,
                reason: "brainstorm_no_synthesis_participants".to_string(),
                mode: None,
                tasks,
                diagnostics: vec![board],
            },
            facilitator_session_id,
        );
    }

    if next_role == BrainstormRole::Critique {
        return (
            AstraOrchestration {
                summary: format!(
                    "Brainstorm board still has open conflicts; round {} asks participants to critique and build on it.",
                    round_index + 1
                ),
                run_intent: AstraRunIntent::Continue,
                reason: "brainstorm_critique_round".to_string(),
                mode: Some(PlanRoundMode::Parallel),
                tasks,
                diagnostics: vec![board],
            },
            facilitator_session_id,
        );
    }

    (
        AstraOrchestration {
            summary:
                "Brainstorm shared board generated; next round injects the board for synthesis."
                    .to_string(),
            run_intent: AstraRunIntent::Continue,
            reason: "brainstorm_shared_board_ready".to_string(),
            mode: Some(PlanRoundMode::Parallel),
            tasks,
            diagnostics: vec![board],
        },
        facilitator_session_id,
    )
}

struct FacilitatorMeta {
    backend: String,
    session_id: Option<String>,
    error: Option<(&'static str, String)>,
}

fn facilitator_session_to_propagate(meta: &FacilitatorMeta) -> Option<String> {
    (meta.backend != HEURISTIC_FACILITATOR_BACKEND_TYPE && meta.error.is_none())
        .then(|| meta.session_id.clone())
        .flatten()
}

fn facilitator_opinions(completions: &[AstraTaskCompletion]) -> Vec<FacilitatorOpinion> {
    completions
        .iter()
        .map(|completion| FacilitatorOpinion {
            participant_id: completion
                .task
                .agent_participant_id
                .clone()
                .or_else(|| completion.task.assistant_id.clone()),
            agent: completion.task.target_agent.as_str().to_string(),
            title: completion.task.title.clone(),
            output: final_task_output(&completion.result.output),
        })
        .collect()
}

/// Recovers the board section injected into the previous synthesis round from
/// the task prompt; the backend is stateless across rounds, so the prompt is
/// the only place the dispatched board survives.
fn extract_board_context(completions: &[AstraTaskCompletion]) -> Option<String> {
    let prompt = &completions.first()?.task.prompt;
    let start = prompt.find(BOARD_INJECTION_MARKER)?;
    let after = &prompt[start + BOARD_INJECTION_MARKER.len()..];
    let end = after.find("\n## ").unwrap_or(after.len());
    let context = after[..end].trim();
    (!context.is_empty()).then(|| context.to_string())
}

fn brainstorm_participant_tasks(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
    shared_board: Option<&Value>,
    role: BrainstormRole,
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
                    role.as_str()
                ))
            );
            let prompt =
                brainstorm_task_prompt(thread, user_prompt, &participant, shared_board, role);
            AstraTaskProposal {
                id: task_id,
                plan_task_id: None,
                assistant_id: None,
                agent_participant_id: Some(participant.participant_id.clone()),
                title: format!(
                    "{} brainstorm {}",
                    participant_label(&participant),
                    match role {
                        BrainstormRole::Divergence => "opinion",
                        BrainstormRole::Critique => "critique",
                        BrainstormRole::Synthesis => "synthesis",
                    }
                ),
                target_stage_id: None,
                target_agent,
                prompt,
                expected_output: match role {
                    BrainstormRole::Divergence =>
                        "Independent brainstorm opinion with rationale, opportunities, risks, and questions."
                            .to_string(),
                    BrainstormRole::Critique =>
                        "Critique referencing board idea ids with explicit support, rebuttal, or extension, plus any genuinely new ideas."
                            .to_string(),
                    BrainstormRole::Synthesis =>
                        "Synthesis result with candidates, consensus, disagreements, and recommendation."
                            .to_string(),
                },
                risk: AstraTaskRisk::Low,
                depends_on: Vec::new(),
            }
        })
        .collect()
}

fn brainstorm_task_prompt(
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    participant: &ThreadAgentInfo,
    shared_board: Option<&Value>,
    role: BrainstormRole,
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
        lines.push(BOARD_INJECTION_MARKER.to_string());
        lines.push(board_injection_text(board));
        lines.push(String::new());
    }
    lines.push("## Task".to_string());
    match role {
        BrainstormRole::Divergence => {
            lines.push("Produce an independent opinion. Offer concrete ideas, rationale, risks, conflicts, and questions. Do not wait for other participants.".to_string());
        }
        BrainstormRole::Critique => {
            lines.push("Critique and build on the shared board above. For every idea you address, reference its id (for example [idea-x]) and explicitly support, rebut, or extend it with reasons and evidence. Address the listed conflicts and open questions directly. Add a genuinely new idea only if the critique surfaced one. End with your current preferred direction.".to_string());
            lines.push(String::new());
            lines.push("Board excerpts are truncated; read the full-output files listed on the board (paths are relative to this workspace) when you need a participant's complete opinion.".to_string());
        }
        BrainstormRole::Synthesis => {
            lines.push("Use the shared board above as explicit context. Synthesize the candidates, consensus, disagreements, risks, and a recommendation. Extend or challenge the board where useful.".to_string());
            lines.push(String::new());
            lines.push("Board excerpts are truncated; read the full-output files listed on the board (paths are relative to this workspace) when you need a participant's complete opinion.".to_string());
        }
    }
    wrap_thread_prompt(
        "astra_brainstorm_participant_task",
        thread,
        lines.join("\n"),
        &[
            ("participant_id", participant.participant_id.clone()),
            ("target_agent", participant.agent.as_str().to_string()),
            ("round_role", role.as_str().to_string()),
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
    run: &AstraRun,
    thread: &ThreadInfo,
    source_round_index: u32,
    completions: &[AstraTaskCompletion],
    facilitator_board: Option<&FacilitatorBoard>,
    meta: Option<&FacilitatorMeta>,
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
                "fullOutputPath": super::artifacts::task_artifact_relative_path(&run.run_id, &completion.task.id),
            })
        })
        .collect::<Vec<_>>();
    let highlights = opinions
        .iter()
        .filter_map(|opinion| opinion.get("opinion").and_then(Value::as_str))
        .take(6)
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let fallback_board;
    let board_data = match facilitator_board {
        Some(board) => board,
        None => {
            fallback_board = heuristic_board(0);
            &fallback_board
        }
    };
    let ideas = board_data
        .ideas
        .iter()
        .map(|idea| {
            json!({
                "id": idea.id,
                "title": idea.title,
                "summary": idea.summary,
                "sources": idea.sources,
            })
        })
        .collect::<Vec<_>>();
    let mut board = json!({
        "kind": "brainstorm_shared_board",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "opinions": opinions,
        "highlights": highlights,
        "ideas": ideas,
        "conflicts": &board_data.conflicts,
        "openQuestions": &board_data.open_questions,
        "readyToSynthesize": board_data.ready_to_synthesize,
        "recordedAt": super::now_ms(),
    });
    if let Some(meta) = meta {
        if let Some(record) = board.as_object_mut() {
            record.insert("facilitatorBackend".to_string(), json!(&meta.backend));
            record.insert("facilitatorSessionId".to_string(), json!(&meta.session_id));
            record.insert(
                "facilitatorAttempts".to_string(),
                json!(board_data.attempts),
            );
            if let Some((code, message)) = &meta.error {
                record.insert(
                    "facilitatorError".to_string(),
                    json!({ "code": code, "message": message }),
                );
            }
        }
    }
    board
}

fn synthesis_diagnostic(
    thread: &ThreadInfo,
    source_round_index: u32,
    board: &Value,
    report: &FacilitatorReport,
    meta: &FacilitatorMeta,
) -> Value {
    let mut diagnostic = json!({
        "kind": "brainstorm_synthesis",
        "threadId": thread.id,
        "sourceRoundIndex": source_round_index,
        "sharedBoardOpinionCount": board
            .get("opinions")
            .and_then(Value::as_array)
            .map(|values| values.len())
            .unwrap_or(0),
        "recommendation": &report.recommendation,
        "consensus": &report.consensus,
        "disagreements": &report.disagreements,
        "nextSteps": &report.next_steps,
        "rationale": &report.rationale,
        "facilitatorBackend": &meta.backend,
        "facilitatorSessionId": &meta.session_id,
        "facilitatorAttempts": report.attempts,
        "recordedAt": super::now_ms(),
    });
    if let Some((code, message)) = &meta.error {
        if let Some(record) = diagnostic.as_object_mut() {
            record.insert(
                "facilitatorError".to_string(),
                json!({ "code": code, "message": message }),
            );
        }
    }
    diagnostic
}

/// Renders the participant-facing markdown view of the shared board. Keeps
/// facilitator meta fields (session ids, timestamps) out of the prompt.
fn board_injection_text(board: &Value) -> String {
    let mut lines = Vec::new();
    lines.push("### Ideas".to_string());
    let ideas = board
        .get("ideas")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if ideas.is_empty() {
        lines.push("- (none yet)".to_string());
    }
    for idea in &ideas {
        let id = idea.get("id").and_then(Value::as_str).unwrap_or("idea");
        let title = idea.get("title").and_then(Value::as_str).unwrap_or("");
        let summary = idea.get("summary").and_then(Value::as_str).unwrap_or("");
        let sources = idea
            .get("sources")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let mut line = format!("- [{id}] {title} — {summary}");
        if !sources.is_empty() {
            line.push_str(&format!(" (sources: {sources})"));
        }
        lines.push(line);
    }
    push_board_list(&mut lines, "### Conflicts", board.get("conflicts"));
    push_board_list(&mut lines, "### Open questions", board.get("openQuestions"));
    lines.push(String::new());
    lines.push("### Participant opinions (excerpts)".to_string());
    for opinion in board
        .get("opinions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let participant = opinion
            .get("participantId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let agent = opinion
            .get("agent")
            .and_then(Value::as_str)
            .unwrap_or("agent");
        let text = opinion.get("opinion").and_then(Value::as_str).unwrap_or("");
        let mut line = format!("- {participant} ({agent}): {text}");
        if let Some(path) = opinion.get("fullOutputPath").and_then(Value::as_str) {
            line.push_str(&format!("\n  Full text: {path}"));
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn push_board_list(lines: &mut Vec<String>, heading: &str, values: Option<&Value>) {
    lines.push(String::new());
    lines.push(heading.to_string());
    let items = values
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if items.is_empty() {
        lines.push("- (none)".to_string());
    }
    for item in items {
        lines.push(format!("- {item}"));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::astra::brainstorm_facilitator::{
        BoardIdea, HeuristicFacilitator, HEURISTIC_BOARD_CONFLICT,
    };
    use crate::astra::types::{AstraTaskResult, AstraTaskResultStatus};
    use crate::models::{Agent, ThreadAgentInfo};

    struct FakeFacilitator {
        board: Result<BackendResponse<FacilitatorBoard>, BackendFailure>,
        report: Result<BackendResponse<FacilitatorReport>, BackendFailure>,
        seen_board_contexts: Mutex<Vec<Option<String>>>,
        seen_build_board_contexts: Mutex<Vec<Option<String>>>,
    }

    impl FakeFacilitator {
        fn new(
            board: Result<BackendResponse<FacilitatorBoard>, BackendFailure>,
            report: Result<BackendResponse<FacilitatorReport>, BackendFailure>,
        ) -> Self {
            Self {
                board,
                report,
                seen_board_contexts: Mutex::new(Vec::new()),
                seen_build_board_contexts: Mutex::new(Vec::new()),
            }
        }
    }

    impl BrainstormFacilitator for FakeFacilitator {
        fn build_board(
            &self,
            _run: &AstraRun,
            _thread: &ThreadInfo,
            _user_prompt: Option<&str>,
            _source_round_index: u32,
            board_context: Option<&str>,
            _opinions: &[FacilitatorOpinion],
        ) -> Result<BackendResponse<FacilitatorBoard>, BackendFailure> {
            self.seen_build_board_contexts
                .lock()
                .unwrap()
                .push(board_context.map(ToString::to_string));
            self.board.clone()
        }

        fn synthesize(
            &self,
            _run: &AstraRun,
            _thread: &ThreadInfo,
            _user_prompt: Option<&str>,
            _source_round_index: u32,
            board_context: Option<&str>,
            _syntheses: &[FacilitatorOpinion],
        ) -> Result<BackendResponse<FacilitatorReport>, BackendFailure> {
            self.seen_board_contexts
                .lock()
                .unwrap()
                .push(board_context.map(ToString::to_string));
            self.report.clone()
        }
    }

    fn fake_board() -> FacilitatorBoard {
        FacilitatorBoard {
            ideas: vec![BoardIdea {
                id: "idea-async".to_string(),
                title: "异步内核改造".to_string(),
                summary: "渐进迁移到异步任务调度。".to_string(),
                sources: vec!["participant-a".to_string()],
            }],
            conflicts: vec!["A 与 B 在迁移节奏上存在冲突。".to_string()],
            open_questions: vec!["如何保证迁移期一致性？".to_string()],
            ready_to_synthesize: true,
            attempts: 1,
        }
    }

    fn unready_board() -> FacilitatorBoard {
        FacilitatorBoard {
            ready_to_synthesize: false,
            ..fake_board()
        }
    }

    fn fake_report() -> FacilitatorReport {
        FacilitatorReport {
            recommendation: "采用渐进迁移方案。".to_string(),
            consensus: vec!["双方都同意异步化方向。".to_string()],
            disagreements: vec!["迁移节奏仍有分歧。".to_string()],
            next_steps: vec!["先做基准测试。".to_string()],
            rationale: "渐进迁移兼顾速度与稳定性。".to_string(),
            attempts: 1,
        }
    }

    fn runtime_response<T>(data: T) -> BackendResponse<T> {
        BackendResponse {
            data,
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
        brainstorm_orchestration(
            run,
            thread,
            user_prompt,
            round_index,
            completions,
            &HeuristicFacilitator,
        )
        .0
    }

    fn opinion_completions(run: &AstraRun) -> Vec<AstraTaskCompletion> {
        let first = orchestrate_with_heuristic(run, &thread(), None, 0, &[]);
        first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Option A has strong upside."))
            .collect()
    }

    fn synthesis_completions(
        run: &AstraRun,
        facilitator: &dyn BrainstormFacilitator,
        output: &str,
    ) -> Vec<AstraTaskCompletion> {
        let board_round = brainstorm_orchestration(
            run,
            &thread(),
            None,
            1,
            &opinion_completions(run),
            facilitator,
        )
        .0;
        board_round
            .tasks
            .into_iter()
            .map(|task| completion(task, output))
            .collect()
    }

    fn thread() -> ThreadInfo {
        ThreadInfo {
            id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            goal: "Choose a product direction".to_string(),
            description: Some("Explore and synthesize options.".to_string()),
            stage_id: None,
            kind: ThreadKind::Brainstorm,
            enabled: true,
            origin: crate::models::ThreadOrigin::Manual,
            scheduled_task_id: None,
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
            orchestrate_with_heuristic(&run(), &thread(), Some("Be practical"), 0, &[]);

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
        let first = orchestrate_with_heuristic(&run(), &thread(), None, 0, &[]);
        let completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Option A has strong upside."))
            .collect::<Vec<_>>();

        let next = orchestrate_with_heuristic(&run(), &thread(), None, 1, &completions);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "brainstorm_shared_board_ready");
        assert_eq!(next.diagnostics[0]["kind"], "brainstorm_shared_board");
        assert_eq!(
            next.diagnostics[0]["opinions"][0]["participantId"],
            "participant-a"
        );
        assert_eq!(
            next.diagnostics[0]["facilitatorBackend"],
            HEURISTIC_FACILITATOR_BACKEND_TYPE
        );
        assert_eq!(
            next.diagnostics[0]["conflicts"][0],
            HEURISTIC_BOARD_CONFLICT
        );
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains("## Shared board from previous round")));
    }

    #[test]
    fn injected_round_completions_finish_with_synthesis_diagnostic() {
        let first = orchestrate_with_heuristic(&run(), &thread(), None, 0, &[]);
        let first_completions = first
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Option A."))
            .collect::<Vec<_>>();
        let synthesis = orchestrate_with_heuristic(&run(), &thread(), None, 1, &first_completions);
        let synthesis_completions = synthesis
            .tasks
            .into_iter()
            .map(|task| completion(task, "Final result: Recommend Option A."))
            .collect::<Vec<_>>();

        let terminal =
            orchestrate_with_heuristic(&run(), &thread(), None, 2, &synthesis_completions);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "brainstorm_synthesis_complete");
        let report = terminal
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic["kind"] == "brainstorm_synthesis")
            .unwrap();
        assert_eq!(report["recommendation"], "Recommend Option A.");
        assert_eq!(
            report["facilitatorBackend"],
            HEURISTIC_FACILITATOR_BACKEND_TYPE
        );
    }

    #[test]
    fn runtime_facilitator_ideas_flow_into_board_and_injection() {
        let run = run();
        let fake = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );

        let (next, session_id) =
            brainstorm_orchestration(&run, &thread(), None, 1, &opinion_completions(&run), &fake);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "brainstorm_shared_board_ready");
        assert_eq!(session_id.as_deref(), Some("agent-session-x"));
        let board = &next.diagnostics[0];
        assert_eq!(board["ideas"][0]["id"], "idea-async");
        assert_eq!(board["ideas"][0]["title"], "异步内核改造");
        assert_eq!(board["conflicts"][0], "A 与 B 在迁移节奏上存在冲突。");
        assert_eq!(board["openQuestions"][0], "如何保证迁移期一致性？");
        assert_eq!(board["facilitatorBackend"], "runtime_agent_claude");
        assert_eq!(board["facilitatorSessionId"], "agent-session-x");
        assert_eq!(board["facilitatorAttempts"], 1);
        assert!(board.get("facilitatorError").is_none());
        assert!(next.tasks.iter().all(|task| {
            task.prompt.contains("[idea-async] 异步内核改造")
                && task.prompt.contains("A 与 B 在迁移节奏上存在冲突。")
        }));
    }

    #[test]
    fn facilitator_board_failure_degrades_to_static_board() {
        let run = run();
        let fake = FakeFacilitator::new(
            Err(
                BackendFailure::new("runtime_agent_claude", "timeout", "facilitator timed out")
                    .with_session_id(Some("facilitator-session-err".to_string())),
            ),
            Ok(runtime_response(fake_report())),
        );

        let (next, session_id) =
            brainstorm_orchestration(&run, &thread(), None, 1, &opinion_completions(&run), &fake);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert!(session_id.is_none());
        let board = &next.diagnostics[0];
        assert_eq!(board["conflicts"][0], HEURISTIC_BOARD_CONFLICT);
        assert_eq!(board["ideas"].as_array().map(Vec::len), Some(0));
        assert_eq!(board["facilitatorError"]["code"], "timeout");
        assert_eq!(board["facilitatorSessionId"], "facilitator-session-err");
        assert_eq!(board["facilitatorAttempts"], 0);
        assert!(!next.tasks.is_empty());
    }

    #[test]
    fn runtime_facilitator_synthesis_produces_final_report() {
        let run = run();
        let fake = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );
        let completions = synthesis_completions(&run, &fake, "Final result: 同意渐进迁移。");

        let (terminal, session_id) =
            brainstorm_orchestration(&run, &thread(), None, 2, &completions, &fake);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert_eq!(terminal.reason, "brainstorm_synthesis_complete");
        assert_eq!(session_id.as_deref(), Some("agent-session-x"));
        let report = terminal
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic["kind"] == "brainstorm_synthesis")
            .unwrap();
        assert_eq!(report["recommendation"], "采用渐进迁移方案。");
        assert_eq!(report["consensus"][0], "双方都同意异步化方向。");
        assert_eq!(report["disagreements"][0], "迁移节奏仍有分歧。");
        assert_eq!(report["nextSteps"][0], "先做基准测试。");
        assert_eq!(report["rationale"], "渐进迁移兼顾速度与稳定性。");
        assert_eq!(report["facilitatorBackend"], "runtime_agent_claude");
        assert!(report.get("facilitatorError").is_none());
    }

    #[test]
    fn facilitator_synthesis_failure_degrades_with_error_diagnostic() {
        let run = run();
        let board_fake = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );
        let completions = synthesis_completions(&run, &board_fake, "Final result: 同意渐进迁移。");
        let failing = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Err(BackendFailure::new(
                "runtime_agent_claude",
                "invalid_yaml",
                "report was not valid YAML",
            )),
        );

        let (terminal, session_id) =
            brainstorm_orchestration(&run, &thread(), None, 2, &completions, &failing);

        assert_eq!(terminal.run_intent, AstraRunIntent::Complete);
        assert!(session_id.is_none());
        let report = terminal
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic["kind"] == "brainstorm_synthesis")
            .unwrap();
        assert_eq!(report["facilitatorError"]["code"], "invalid_yaml");
        assert_eq!(report["recommendation"], "同意渐进迁移。");
        assert_eq!(report["facilitatorAttempts"], 0);
    }

    #[test]
    fn synthesize_receives_board_context_extracted_from_prompt() {
        let run = run();
        let fake = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );
        let completions = synthesis_completions(&run, &fake, "Final result: 同意。");

        let _ = brainstorm_orchestration(&run, &thread(), None, 2, &completions, &fake);

        let contexts = fake.seen_board_contexts.lock().unwrap();
        let context = contexts.last().unwrap().as_deref().unwrap();
        assert!(context.contains("[idea-async] 异步内核改造"));
        assert!(context.contains("### Conflicts"));
        assert!(!context.contains("facilitatorSessionId"));
    }

    #[test]
    fn orchestrate_propagates_runtime_facilitator_session_id_only() {
        let run = run();
        let board_fake = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );
        let completions = synthesis_completions(&run, &board_fake, "Final result: agree.");

        let runtime_backend = BrainstormBackend::new(Box::new(FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        )));
        let response = runtime_backend
            .orchestrate(&run, &thread(), None, 2, &completions, &json!({}))
            .unwrap();
        assert_eq!(response.session_id, "agent-session-x");
        assert_eq!(response.backend_type, BRAINSTORM_BACKEND_TYPE);

        let heuristic_backend = BrainstormBackend::new(Box::new(HeuristicFacilitator));
        let response = heuristic_backend
            .orchestrate(&run, &thread(), None, 2, &completions, &json!({}))
            .unwrap();
        assert_eq!(response.session_id, "brainstorm-backend-run-1-2");
    }

    #[test]
    fn unready_board_dispatches_critique_round() {
        let run = run();
        let fake = FakeFacilitator::new(
            Ok(runtime_response(unready_board())),
            Ok(runtime_response(fake_report())),
        );

        let (next, _) =
            brainstorm_orchestration(&run, &thread(), None, 1, &opinion_completions(&run), &fake);

        assert_eq!(next.run_intent, AstraRunIntent::Continue);
        assert_eq!(next.reason, "brainstorm_critique_round");
        assert_eq!(next.mode, Some(PlanRoundMode::Parallel));
        assert_eq!(next.diagnostics[0]["readyToSynthesize"], false);
        assert!(next.tasks.iter().all(|task| {
            task.prompt.contains("round_role=\"critique\"")
                && task.prompt.contains(BOARD_INJECTION_MARKER)
                && task.prompt.contains("support, rebut, or extend")
                && task.prompt.contains("[idea-async] 异步内核改造")
        }));
        assert!(next.tasks[0].title.contains("brainstorm critique"));
    }

    #[test]
    fn critique_completions_feed_previous_board_back_to_facilitator() {
        let run = run();
        let unready = FakeFacilitator::new(
            Ok(runtime_response(unready_board())),
            Ok(runtime_response(fake_report())),
        );
        let critique_round = brainstorm_orchestration(
            &run,
            &thread(),
            None,
            1,
            &opinion_completions(&run),
            &unready,
        )
        .0;
        let critique_completions = critique_round
            .tasks
            .into_iter()
            .map(|task| completion(task, "支持 [idea-async]，但建议放缓迁移节奏。"))
            .collect::<Vec<_>>();

        let ready = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );
        let (next, _) =
            brainstorm_orchestration(&run, &thread(), None, 2, &critique_completions, &ready);

        // The facilitator saw the previous board while rebuilding it, and the
        // now-ready board moves the flow to synthesis.
        let contexts = ready.seen_build_board_contexts.lock().unwrap();
        let context = contexts.last().unwrap().as_deref().unwrap();
        assert!(context.contains("[idea-async] 异步内核改造"));
        assert_eq!(next.reason, "brainstorm_shared_board_ready");
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains("round_role=\"synthesis\"")));
    }

    #[test]
    fn critique_rounds_are_capped_even_when_facilitator_stays_unready() {
        let run = run();
        let unready = FakeFacilitator::new(
            Ok(runtime_response(unready_board())),
            Ok(runtime_response(fake_report())),
        );
        let critique_round = brainstorm_orchestration(
            &run,
            &thread(),
            None,
            1,
            &opinion_completions(&run),
            &unready,
        )
        .0;
        let mut completions = critique_round
            .tasks
            .into_iter()
            .map(|task| completion(task, "仍有分歧。"))
            .collect::<Vec<_>>();

        // Round 2 may still critique (1 critique round so far); round 3 hits
        // the MAX_BRAINSTORM_CRITIQUE_ROUNDS cap and must synthesize.
        let second = brainstorm_orchestration(&run, &thread(), None, 2, &completions, &unready).0;
        assert_eq!(second.reason, "brainstorm_critique_round");
        completions = second
            .tasks
            .into_iter()
            .map(|task| completion(task, "仍有分歧。"))
            .collect();

        let third = brainstorm_orchestration(&run, &thread(), None, 3, &completions, &unready).0;
        assert_eq!(third.reason, "brainstorm_shared_board_ready");
        assert!(third
            .tasks
            .iter()
            .all(|task| task.prompt.contains("round_role=\"synthesis\"")));
    }

    #[test]
    fn critique_is_skipped_when_round_budget_is_tight() {
        let run = AstraRun {
            round_limit: 3,
            ..run()
        };
        let unready = FakeFacilitator::new(
            Ok(runtime_response(unready_board())),
            Ok(runtime_response(fake_report())),
        );

        let (next, _) = brainstorm_orchestration(
            &run,
            &thread(),
            None,
            1,
            &opinion_completions(&run),
            &unready,
        );

        // round_index 1 + critique + synthesis + terminal planning would
        // exceed round_limit 3, so the flow goes straight to synthesis.
        assert_eq!(next.reason, "brainstorm_shared_board_ready");
    }

    #[test]
    fn board_injection_lists_full_output_paths() {
        let run = run();
        let fake = FakeFacilitator::new(
            Ok(runtime_response(fake_board())),
            Ok(runtime_response(fake_report())),
        );

        let (next, _) =
            brainstorm_orchestration(&run, &thread(), None, 1, &opinion_completions(&run), &fake);

        let board = &next.diagnostics[0];
        let path = board["opinions"][0]["fullOutputPath"].as_str().unwrap();
        assert!(path.starts_with(".sessio/astra/run-1/tasks/"));
        assert!(path.ends_with(".md"));
        assert!(next
            .tasks
            .iter()
            .all(|task| task.prompt.contains(&format!("Full text: {path}"))));
    }
}
