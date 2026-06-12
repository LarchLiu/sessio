use serde::Deserialize;

use super::backend::{BackendFailure, BackendResponse};
use super::prompt::wrap_thread_prompt;
use super::runtime_agent_backend::{execute_agent_session, RuntimeAgentBackendConfig};
use super::structured_response::{
    clean_string_list, execute_structured_with_retry, parse_yaml_mapping, truncate_chars,
};
use super::AstraRun;
use crate::agents::runtime::RuntimeManager;
use crate::models::ThreadInfo;

pub(super) const HEURISTIC_FACILITATOR_BACKEND_TYPE: &str = "brainstorm_heuristic_facilitator";
const FACILITATOR_OPINION_CHAR_LIMIT: usize = 6000;
const FACILITATOR_LIST_ITEM_CHAR_LIMIT: usize = 600;
const FACILITATOR_LIST_LEN_LIMIT: usize = 12;
const FACILITATOR_IDEA_TITLE_CHAR_LIMIT: usize = 120;
const REPORT_RECOMMENDATION_CHAR_LIMIT: usize = 2000;

pub(super) const HEURISTIC_BOARD_CONFLICT: &str =
    "Compare assumptions and tradeoffs across the opinions.";
pub(super) const HEURISTIC_BOARD_OPEN_QUESTIONS: [&str; 2] = [
    "Which candidate best satisfies the thread goal?",
    "What evidence would change the recommendation?",
];

const BRAINSTORM_BOARD_CONTRACT: &str = r#"You are the Sessio brainstorm facilitator.

Read the independent participant opinions and build the shared board for the next round.

Return only one complete YAML mapping. Do not return JSON, markdown, code fences, comments, prose, or multiple YAML documents.

Required top-level YAML response:
ideas: []
conflicts: []
openQuestions: []
readyToSynthesize: true|false

ideas item shape (every key required):
- id: short kebab-case identifier, unique within the list
  title: one-line idea name
  summary: one or two sentences capturing the idea
  sources: [participant ids that proposed or support it]

Field rules:
- ideas: cluster and deduplicate the concrete ideas across all opinions. Merge near-duplicate proposals into a single idea and list every contributing participant id in sources. ideas must be non-empty when at least one opinion is provided.
- conflicts: real, substantive disagreements between the opinions, one sentence each naming the conflicting positions. Leave empty when there is no genuine conflict; do not invent one.
- openQuestions: unresolved questions whose answers would most change the recommendation.
- readyToSynthesize: false when a critique round would add real value because substantive conflicts or open questions remain that participants should debate; true when the opinions already align or further debate is unlikely to change the recommendation.
- When a previous shared board is provided, update it instead of starting over: keep existing idea ids stable, merge newly supported ideas, refine summaries from the critiques, and drop ideas that were convincingly rebutted.
- Write title, summary, conflicts, and openQuestions in the same language as the opinions (for example, respond in Chinese when participants brainstorm in Chinese). Keep ids ASCII kebab-case.
- Facilitate only from the opinions below. Do not invent ideas that no participant stated."#;

const BRAINSTORM_REPORT_CONTRACT: &str = r#"You are the Sessio brainstorm facilitator writing the final synthesis report.

Read the shared board context and every participant synthesis, then produce one final report for the whole brainstorm.

Return only one complete YAML mapping. Do not return JSON, markdown, code fences, comments, prose, or multiple YAML documents.

Required top-level YAML response:
recommendation: string
consensus: []
disagreements: []
nextSteps: []
rationale: string

Field rules:
- recommendation: the single concrete recommendation for the thread goal. It must be non-empty and self-contained: state what to do and the key reason.
- consensus: points that the syntheses genuinely share, one sentence each.
- disagreements: substantive disagreements that remain across the syntheses, each self-contained and actionable.
- nextSteps: concrete follow-up actions, ordered by priority.
- rationale: one short paragraph explaining how the recommendation follows from the syntheses.
- Write every field except YAML keys in the same language as the syntheses (for example, respond in Chinese when participants write in Chinese).
- Synthesize only from the board context and syntheses below. Do not invent positions that no participant stated."#;

#[derive(Debug, Clone)]
pub(super) struct BoardIdea {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct FacilitatorBoard {
    pub ideas: Vec<BoardIdea>,
    pub conflicts: Vec<String>,
    pub open_questions: Vec<String>,
    /// false asks the backend for another critique round before synthesis.
    pub ready_to_synthesize: bool,
    pub attempts: u32,
}

#[derive(Debug, Clone)]
pub(super) struct FacilitatorReport {
    pub recommendation: String,
    pub consensus: Vec<String>,
    pub disagreements: Vec<String>,
    pub next_steps: Vec<String>,
    pub rationale: String,
    pub attempts: u32,
}

/// One participant's full output as facilitator input. `output` carries the
/// full final task output, not the 1000-char board excerpt shared with the
/// other participants; the facilitator prompt applies its own larger limit.
#[derive(Debug, Clone)]
pub(super) struct FacilitatorOpinion {
    pub participant_id: Option<String>,
    pub agent: String,
    pub title: String,
    pub output: String,
}

pub(super) trait BrainstormFacilitator: Send + Sync {
    fn build_board(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        source_round_index: u32,
        board_context: Option<&str>,
        opinions: &[FacilitatorOpinion],
    ) -> Result<BackendResponse<FacilitatorBoard>, BackendFailure>;

    fn synthesize(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        source_round_index: u32,
        board_context: Option<&str>,
        syntheses: &[FacilitatorOpinion],
    ) -> Result<BackendResponse<FacilitatorReport>, BackendFailure>;
}

pub(super) fn heuristic_board(attempts: u32) -> FacilitatorBoard {
    FacilitatorBoard {
        ideas: Vec::new(),
        conflicts: vec![HEURISTIC_BOARD_CONFLICT.to_string()],
        open_questions: HEURISTIC_BOARD_OPEN_QUESTIONS
            .iter()
            .map(ToString::to_string)
            .collect(),
        // The keyword fallback cannot judge convergence; keep the short
        // diverge -> synthesize flow instead of spending critique rounds.
        ready_to_synthesize: true,
        attempts,
    }
}

pub(super) fn heuristic_report(
    syntheses: &[FacilitatorOpinion],
    attempts: u32,
) -> FacilitatorReport {
    let recommendation = syntheses
        .iter()
        .map(|synthesis| synthesis.output.trim())
        .find(|output| !output.is_empty())
        .map(|output| truncate_chars(output, FACILITATOR_LIST_ITEM_CHAR_LIMIT))
        .unwrap_or_default();
    FacilitatorReport {
        recommendation,
        consensus: Vec::new(),
        disagreements: Vec::new(),
        next_steps: Vec::new(),
        rationale:
            "Heuristic facilitator report; configure an Astra backend agent for structured synthesis."
                .to_string(),
        attempts,
    }
}

/// Fallback used when no Astra backend agent is configured: preserves the
/// historical static board content and aggregates the first synthesis output
/// as the recommendation.
pub(super) struct HeuristicFacilitator;

impl BrainstormFacilitator for HeuristicFacilitator {
    fn build_board(
        &self,
        run: &AstraRun,
        _thread: &ThreadInfo,
        _user_prompt: Option<&str>,
        source_round_index: u32,
        _board_context: Option<&str>,
        _opinions: &[FacilitatorOpinion],
    ) -> Result<BackendResponse<FacilitatorBoard>, BackendFailure> {
        Ok(BackendResponse {
            data: heuristic_board(1),
            session_id: format!(
                "brainstorm-facilitator-heuristic-{}-{}",
                run.run_id, source_round_index
            ),
            backend_type: HEURISTIC_FACILITATOR_BACKEND_TYPE.to_string(),
        })
    }

    fn synthesize(
        &self,
        run: &AstraRun,
        _thread: &ThreadInfo,
        _user_prompt: Option<&str>,
        source_round_index: u32,
        _board_context: Option<&str>,
        syntheses: &[FacilitatorOpinion],
    ) -> Result<BackendResponse<FacilitatorReport>, BackendFailure> {
        Ok(BackendResponse {
            data: heuristic_report(syntheses, 1),
            session_id: format!(
                "brainstorm-facilitator-heuristic-{}-{}",
                run.run_id, source_round_index
            ),
            backend_type: HEURISTIC_FACILITATOR_BACKEND_TYPE.to_string(),
        })
    }
}

pub(super) struct RuntimeAgentFacilitator {
    runtime: RuntimeManager,
    config: RuntimeAgentBackendConfig,
}

impl RuntimeAgentFacilitator {
    pub(super) fn new(runtime: RuntimeManager, config: RuntimeAgentBackendConfig) -> Self {
        Self { runtime, config }
    }

    fn backend_type(&self) -> String {
        format!("runtime_agent_{}", self.config.agent.as_str())
    }
}

impl BrainstormFacilitator for RuntimeAgentFacilitator {
    fn build_board(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        source_round_index: u32,
        board_context: Option<&str>,
        opinions: &[FacilitatorOpinion],
    ) -> Result<BackendResponse<FacilitatorBoard>, BackendFailure> {
        let prompt = build_board_prompt(
            run,
            thread,
            user_prompt,
            source_round_index,
            board_context,
            opinions,
        );
        let backend_type = self.backend_type();
        let require_ideas = !opinions.is_empty();
        let (mut board, session_id, attempts) = execute_structured_with_retry(
            &prompt,
            |text| parse_facilitator_board(&backend_type, text, require_ideas),
            |attempt_prompt| {
                execute_agent_session(
                    &self.runtime,
                    &self.config,
                    run,
                    thread,
                    &run.project_path,
                    attempt_prompt,
                    "brainstorm_facilitator_board",
                )
            },
        )?;
        board.attempts = attempts;
        Ok(BackendResponse {
            data: board,
            session_id,
            backend_type,
        })
    }

    fn synthesize(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        source_round_index: u32,
        board_context: Option<&str>,
        syntheses: &[FacilitatorOpinion],
    ) -> Result<BackendResponse<FacilitatorReport>, BackendFailure> {
        let prompt = build_report_prompt(
            run,
            thread,
            user_prompt,
            source_round_index,
            board_context,
            syntheses,
        );
        let backend_type = self.backend_type();
        let (mut report, session_id, attempts) = execute_structured_with_retry(
            &prompt,
            |text| parse_facilitator_report(&backend_type, text),
            |attempt_prompt| {
                execute_agent_session(
                    &self.runtime,
                    &self.config,
                    run,
                    thread,
                    &run.project_path,
                    attempt_prompt,
                    "brainstorm_facilitator_report",
                )
            },
        )?;
        report.attempts = attempts;
        Ok(BackendResponse {
            data: report,
            session_id,
            backend_type,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawFacilitatorBoard {
    #[serde(default)]
    ideas: Option<Vec<RawBoardIdea>>,
    #[serde(default)]
    conflicts: Option<Vec<String>>,
    #[serde(default)]
    open_questions: Option<Vec<String>>,
    #[serde(default)]
    ready_to_synthesize: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBoardIdea {
    id: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    sources: Option<Vec<String>>,
}

fn parse_facilitator_board(
    backend_type: &str,
    response: &str,
    require_ideas: bool,
) -> Result<FacilitatorBoard, BackendFailure> {
    let raw: RawFacilitatorBoard =
        parse_yaml_mapping(backend_type, "brainstorm facilitator board", response)?;
    let mut ideas = Vec::new();
    for (index, idea) in raw.ideas.unwrap_or_default().into_iter().enumerate() {
        let id = idea.id.as_deref().map(str::trim).unwrap_or_default();
        let title = idea.title.as_deref().map(str::trim).unwrap_or_default();
        let summary = idea.summary.as_deref().map(str::trim).unwrap_or_default();
        if id.is_empty() || title.is_empty() || summary.is_empty() {
            return Err(BackendFailure::new(
                backend_type.to_string(),
                "validation_failed",
                format!(
                    "board idea {} is missing a non-empty id, title, or summary",
                    index + 1
                ),
            )
            .with_raw_response(response));
        }
        ideas.push(BoardIdea {
            id: truncate_chars(id, FACILITATOR_IDEA_TITLE_CHAR_LIMIT),
            title: truncate_chars(title, FACILITATOR_IDEA_TITLE_CHAR_LIMIT),
            summary: truncate_chars(summary, FACILITATOR_LIST_ITEM_CHAR_LIMIT),
            sources: clean_string_list(
                idea.sources.unwrap_or_default(),
                FACILITATOR_IDEA_TITLE_CHAR_LIMIT,
                FACILITATOR_LIST_LEN_LIMIT,
            ),
        });
        if ideas.len() >= FACILITATOR_LIST_LEN_LIMIT {
            break;
        }
    }
    if require_ideas && ideas.is_empty() {
        return Err(BackendFailure::new(
            backend_type.to_string(),
            "validation_failed",
            "board ideas must be non-empty when participant opinions are provided",
        )
        .with_raw_response(response));
    }
    Ok(FacilitatorBoard {
        ideas,
        conflicts: clean_string_list(
            raw.conflicts.unwrap_or_default(),
            FACILITATOR_LIST_ITEM_CHAR_LIMIT,
            FACILITATOR_LIST_LEN_LIMIT,
        ),
        open_questions: clean_string_list(
            raw.open_questions.unwrap_or_default(),
            FACILITATOR_LIST_ITEM_CHAR_LIMIT,
            FACILITATOR_LIST_LEN_LIMIT,
        ),
        // Missing flag defaults to ready: a forgetful facilitator should not
        // silently spend critique rounds.
        ready_to_synthesize: raw.ready_to_synthesize.unwrap_or(true),
        attempts: 1,
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawFacilitatorReport {
    recommendation: Option<String>,
    #[serde(default)]
    consensus: Option<Vec<String>>,
    #[serde(default)]
    disagreements: Option<Vec<String>>,
    #[serde(default)]
    next_steps: Option<Vec<String>>,
    #[serde(default)]
    rationale: Option<String>,
}

fn parse_facilitator_report(
    backend_type: &str,
    response: &str,
) -> Result<FacilitatorReport, BackendFailure> {
    let raw: RawFacilitatorReport =
        parse_yaml_mapping(backend_type, "brainstorm facilitator report", response)?;
    let recommendation = raw
        .recommendation
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if recommendation.is_empty() {
        return Err(BackendFailure::new(
            backend_type.to_string(),
            "validation_failed",
            "report recommendation must be non-empty",
        )
        .with_raw_response(response));
    }
    Ok(FacilitatorReport {
        recommendation: truncate_chars(recommendation, REPORT_RECOMMENDATION_CHAR_LIMIT),
        consensus: clean_string_list(
            raw.consensus.unwrap_or_default(),
            FACILITATOR_LIST_ITEM_CHAR_LIMIT,
            FACILITATOR_LIST_LEN_LIMIT,
        ),
        disagreements: clean_string_list(
            raw.disagreements.unwrap_or_default(),
            FACILITATOR_LIST_ITEM_CHAR_LIMIT,
            FACILITATOR_LIST_LEN_LIMIT,
        ),
        next_steps: clean_string_list(
            raw.next_steps.unwrap_or_default(),
            FACILITATOR_LIST_ITEM_CHAR_LIMIT,
            FACILITATOR_LIST_LEN_LIMIT,
        ),
        rationale: raw
            .rationale
            .map(|value| value.trim().to_string())
            .unwrap_or_default(),
        attempts: 1,
    })
}

fn push_prompt_header(
    lines: &mut Vec<String>,
    contract: &str,
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    source_round_index: u32,
) {
    lines.push(contract.to_string());
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
    lines.push(format!("Source round: {source_round_index}"));
    lines.push(format!("Round limit: {}", run.round_limit));
}

fn push_opinion_sections(lines: &mut Vec<String>, heading: &str, opinions: &[FacilitatorOpinion]) {
    lines.push(String::new());
    lines.push(heading.to_string());
    for opinion in opinions {
        lines.push(String::new());
        lines.push(format!(
            "### From participant {} (agent {}, task \"{}\")",
            opinion.participant_id.as_deref().unwrap_or("unknown"),
            opinion.agent,
            opinion.title
        ));
        lines.push(truncate_chars(
            &opinion.output,
            FACILITATOR_OPINION_CHAR_LIMIT,
        ));
    }
}

fn build_board_prompt(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    source_round_index: u32,
    board_context: Option<&str>,
    opinions: &[FacilitatorOpinion],
) -> String {
    let mut lines = Vec::new();
    push_prompt_header(
        &mut lines,
        BRAINSTORM_BOARD_CONTRACT,
        run,
        thread,
        user_prompt,
        source_round_index,
    );
    if let Some(board_context) = board_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push("## Previous shared board".to_string());
        lines.push(truncate_chars(
            board_context,
            FACILITATOR_OPINION_CHAR_LIMIT,
        ));
    }
    push_opinion_sections(&mut lines, "## Participant opinions", opinions);
    wrap_thread_prompt(
        "astra_brainstorm_facilitator_board",
        thread,
        lines.join("\n"),
        &[
            ("run_id", run.run_id.clone()),
            ("source_round_index", source_round_index.to_string()),
        ],
    )
}

fn build_report_prompt(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    source_round_index: u32,
    board_context: Option<&str>,
    syntheses: &[FacilitatorOpinion],
) -> String {
    let mut lines = Vec::new();
    push_prompt_header(
        &mut lines,
        BRAINSTORM_REPORT_CONTRACT,
        run,
        thread,
        user_prompt,
        source_round_index,
    );
    if let Some(board_context) = board_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(String::new());
        lines.push("## Shared board context".to_string());
        lines.push(truncate_chars(
            board_context,
            FACILITATOR_OPINION_CHAR_LIMIT,
        ));
    }
    push_opinion_sections(&mut lines, "## Participant syntheses", syntheses);
    wrap_thread_prompt(
        "astra_brainstorm_facilitator_report",
        thread,
        lines.join("\n"),
        &[
            ("run_id", run.run_id.clone()),
            ("source_round_index", source_round_index.to_string()),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ThreadKind;

    fn run() -> AstraRun {
        AstraRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp".to_string(),
            status: super::super::AstraRunStatus::Planning,
            mode: "rust_native".to_string(),
            planner_backend: None,
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
            agent_participants: Vec::new(),
            stages: Vec::new(),
            sessions: Vec::new(),
        }
    }

    fn opinion(participant: &str, output: &str) -> FacilitatorOpinion {
        FacilitatorOpinion {
            participant_id: Some(format!("participant-{participant}")),
            agent: "codex".to_string(),
            title: format!("{participant} brainstorm opinion"),
            output: output.to_string(),
        }
    }

    #[test]
    fn parse_board_accepts_valid_yaml_with_chinese_ideas() {
        let response = r#"ideas:
  - id: idea-async-core
    title: 异步内核改造
    summary: 把同步主循环替换为异步任务调度，降低阻塞。
    sources:
      - participant-a
      - participant-b
conflicts:
  - A 主张全量重写，B 主张渐进迁移。
openQuestions:
  - 迁移期间如何保证数据一致性？"#;

        let board = parse_facilitator_board("runtime_agent_claude", response, true).unwrap();

        assert_eq!(board.ideas.len(), 1);
        assert_eq!(board.ideas[0].id, "idea-async-core");
        assert_eq!(board.ideas[0].title, "异步内核改造");
        assert_eq!(
            board.ideas[0].sources,
            vec!["participant-a", "participant-b"]
        );
        assert_eq!(board.conflicts, vec!["A 主张全量重写，B 主张渐进迁移。"]);
        assert_eq!(board.open_questions, vec!["迁移期间如何保证数据一致性？"]);
    }

    #[test]
    fn parse_board_rejects_code_fence_json_and_empty() {
        let fenced = parse_facilitator_board("f", "```yaml\nideas: []\n```", false).unwrap_err();
        assert_eq!(fenced.code, "invalid_yaml");

        let json = parse_facilitator_board("f", "{\"ideas\": []}", false).unwrap_err();
        assert_eq!(json.code, "invalid_yaml");

        let empty = parse_facilitator_board("f", "  ", false).unwrap_err();
        assert_eq!(empty.code, "empty_response");
    }

    #[test]
    fn parse_board_requires_ideas_only_when_required() {
        let response = "ideas: []\nconflicts: []\nopenQuestions: []";

        let failure = parse_facilitator_board("f", response, true).unwrap_err();
        assert_eq!(failure.code, "validation_failed");
        assert!(failure.message.contains("ideas"));

        let board = parse_facilitator_board("f", response, false).unwrap();
        assert!(board.ideas.is_empty());
    }

    #[test]
    fn parse_board_rejects_idea_missing_fields_and_unknown_keys() {
        let missing = parse_facilitator_board(
            "f",
            "ideas:\n  - id: idea-1\n    title: ok\n    summary: ''",
            true,
        )
        .unwrap_err();
        assert_eq!(missing.code, "validation_failed");
        assert!(missing.message.contains("idea 1"));

        let unknown = parse_facilitator_board("f", "ideas: []\nscore: 3", false).unwrap_err();
        assert_eq!(unknown.code, "invalid_yaml");
    }

    #[test]
    fn parse_board_bounds_idea_count_and_summary_length() {
        let long_summary = "细".repeat(700);
        let items = (0..15)
            .map(|index| {
                format!(
                    "  - id: idea-{index}\n    title: t{index}\n    summary: {long_summary}\n    sources: []"
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let response = format!("ideas:\n{items}");

        let board = parse_facilitator_board("f", &response, true).unwrap();

        assert_eq!(board.ideas.len(), FACILITATOR_LIST_LEN_LIMIT);
        assert!(board.ideas[0].summary.chars().count() <= FACILITATOR_LIST_ITEM_CHAR_LIMIT);
        assert!(board.ideas[0].summary.ends_with("..."));
    }

    #[test]
    fn parse_report_accepts_chinese_and_rejects_empty_recommendation() {
        let response = r#"recommendation: 优先做异步内核改造，渐进迁移以控制风险。
consensus:
  - 双方都同意异步化方向。
disagreements:
  - 迁移节奏仍有分歧。
nextSteps:
  - 先做基准测试。
rationale: 渐进迁移兼顾速度与稳定性。"#;

        let report = parse_facilitator_report("f", response).unwrap();
        assert_eq!(
            report.recommendation,
            "优先做异步内核改造，渐进迁移以控制风险。"
        );
        assert_eq!(report.consensus, vec!["双方都同意异步化方向。"]);
        assert_eq!(report.next_steps, vec!["先做基准测试。"]);
        assert_eq!(report.rationale, "渐进迁移兼顾速度与稳定性。");

        let failure = parse_facilitator_report("f", "recommendation: ''").unwrap_err();
        assert_eq!(failure.code, "validation_failed");
        assert!(failure.message.contains("recommendation"));
    }

    #[test]
    fn parse_report_rejects_unknown_keys_and_bounds_recommendation() {
        let unknown =
            parse_facilitator_report("f", "recommendation: ok\nverdict: yes").unwrap_err();
        assert_eq!(unknown.code, "invalid_yaml");

        let long = "荐".repeat(2100);
        let report = parse_facilitator_report("f", &format!("recommendation: {long}")).unwrap();
        assert_eq!(
            report.recommendation.chars().count(),
            REPORT_RECOMMENDATION_CHAR_LIMIT
        );
        assert!(report.recommendation.ends_with("..."));
    }

    #[test]
    fn heuristic_facilitator_board_keeps_static_content() {
        let response = HeuristicFacilitator
            .build_board(&run(), &thread(), None, 0, None, &[opinion("a", "想法一")])
            .unwrap();

        assert_eq!(response.backend_type, HEURISTIC_FACILITATOR_BACKEND_TYPE);
        assert_eq!(
            response.session_id,
            "brainstorm-facilitator-heuristic-run-1-0"
        );
        assert!(response.data.ideas.is_empty());
        assert!(response.data.ready_to_synthesize);
        assert_eq!(response.data.conflicts, vec![HEURISTIC_BOARD_CONFLICT]);
        assert_eq!(response.data.open_questions.len(), 2);
    }

    #[test]
    fn heuristic_facilitator_synthesize_uses_first_output_excerpt() {
        let response = HeuristicFacilitator
            .synthesize(
                &run(),
                &thread(),
                None,
                1,
                None,
                &[
                    opinion("a", "   "),
                    opinion("b", "最终建议：采用渐进迁移方案。"),
                ],
            )
            .unwrap();

        assert_eq!(response.data.recommendation, "最终建议：采用渐进迁移方案。");
        assert!(response.data.consensus.is_empty());
        assert!(response.data.rationale.contains("Heuristic facilitator"));
    }

    #[test]
    fn board_prompt_includes_contract_full_opinions_and_wrapping() {
        let long_opinion = "观点细节 ".repeat(300);
        assert!(long_opinion.chars().count() > 1000);

        let prompt = build_board_prompt(
            &run(),
            &thread(),
            Some("Be practical"),
            0,
            None,
            &[opinion("a", &long_opinion), opinion("b", "short")],
        );

        assert!(prompt.contains("ideas must be non-empty"));
        assert!(prompt.contains("readyToSynthesize: true|false"));
        assert!(prompt.contains("Thread goal: Choose a product direction"));
        assert!(prompt.contains("User brainstorm instruction: Be practical"));
        assert!(prompt.contains("### From participant participant-a (agent codex"));
        assert!(prompt.contains("in the same language as the opinions"));
        assert!(prompt.contains(long_opinion.trim()));
        assert!(prompt.contains("sessio-thread-prompt:start"));
        assert!(!prompt.contains("## Previous shared board"));
    }

    #[test]
    fn board_prompt_includes_previous_board_for_critique_rounds() {
        let prompt = build_board_prompt(
            &run(),
            &thread(),
            None,
            2,
            Some("### Ideas\n- [idea-1] 异步内核改造"),
            &[opinion("a", "支持 idea-1，但迁移节奏要放缓。")],
        );

        assert!(prompt.contains("## Previous shared board"));
        assert!(prompt.contains("[idea-1] 异步内核改造"));
        assert!(prompt.contains("keep existing idea ids stable"));
    }

    #[test]
    fn parse_board_reads_ready_to_synthesize_and_defaults_to_true() {
        let unready = parse_facilitator_board(
            "f",
            "ideas:\n  - id: idea-1\n    title: t\n    summary: s\n    sources: []\nreadyToSynthesize: false",
            true,
        )
        .unwrap();
        assert!(!unready.ready_to_synthesize);

        let omitted = parse_facilitator_board(
            "f",
            "ideas:\n  - id: idea-1\n    title: t\n    summary: s\n    sources: []",
            true,
        )
        .unwrap();
        assert!(omitted.ready_to_synthesize);
    }

    #[test]
    fn report_prompt_includes_board_context_and_syntheses() {
        let prompt = build_report_prompt(
            &run(),
            &thread(),
            None,
            1,
            Some("### Ideas\n- [idea-1] 异步内核改造"),
            &[opinion("a", "同意 idea-1，建议先做基准。")],
        );

        assert!(prompt.contains("recommendation: the single concrete recommendation"));
        assert!(prompt.contains("## Shared board context"));
        assert!(prompt.contains("[idea-1] 异步内核改造"));
        assert!(prompt.contains("## Participant syntheses"));
        assert!(prompt.contains("同意 idea-1，建议先做基准。"));

        let without_board = build_report_prompt(&run(), &thread(), None, 1, None, &[]);
        assert!(!without_board.contains("## Shared board context"));
    }
}
