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

pub(super) const HEURISTIC_JUDGE_BACKEND_TYPE: &str = "debate_heuristic_judge";
const JUDGE_LANE_OUTPUT_CHAR_LIMIT: usize = 6000;
const JUDGE_LIST_ITEM_CHAR_LIMIT: usize = 600;
const JUDGE_LIST_LEN_LIMIT: usize = 12;

const DEBATE_JUDGE_RESPONSE_CONTRACT: &str = r#"You are the Sessio debate convergence judge.

Read the cross-check artifacts from every debate lane and decide whether the debate has converged.

Return only one complete YAML mapping. Do not return JSON, markdown, code fences, comments, prose, or multiple YAML documents.

Required top-level YAML response:
status: converged|diverged|needs_review
agreements: []
disagreements: []
arbitration: string or null
rationale: string

Field rules:
- status: use converged only when every lane explicitly accepts a shared conclusion with no substantive open disagreement. Use diverged when at least one substantive disagreement remains. Use needs_review when the artifacts are ambiguous, incomplete, or talk past each other.
- agreements: settled points that all lanes accept, one sentence each.
- disagreements: every unresolved substantive disagreement. Each item must be self-contained and actionable: name the conflicting positions and what evidence or decision would resolve it. disagreements must be non-empty when status is diverged.
- arbitration: when the lanes are unlikely to converge on their own, a concrete recommendation for a human arbiter; otherwise null.
- rationale: one short paragraph explaining the status decision.
- Write agreements, disagreements, arbitration, and rationale in the same language as the debate artifacts (for example, respond in Chinese when the lanes debate in Chinese).
- Judge only from the artifacts below. Do not invent positions that no lane stated."#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum JudgeStatus {
    Converged,
    Diverged,
    NeedsReview,
}

impl JudgeStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Converged => "converged",
            Self::Diverged => "diverged",
            Self::NeedsReview => "needs_review",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct JudgeVerdict {
    pub status: JudgeStatus,
    pub agreements: Vec<String>,
    pub disagreements: Vec<String>,
    pub arbitration: Option<String>,
    pub rationale: String,
    pub attempts: u32,
}

/// One debate lane's cross-check output as judge input. `output` carries the
/// full final task output, not the 1000-char stage artifact shared with the
/// opposing lane; the judge prompt applies its own larger excerpt limit.
#[derive(Debug, Clone)]
pub(super) struct JudgeLaneArtifact {
    pub lane_id: String,
    pub participant_id: Option<String>,
    pub agent: String,
    pub output: String,
}

pub(super) trait DebateJudge: Send + Sync {
    fn judge(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        source_round_index: u32,
        artifacts: &[JudgeLaneArtifact],
    ) -> Result<BackendResponse<JudgeVerdict>, BackendFailure>;
}

/// Keyword fallback used when no Astra backend agent is configured. English
/// keywords only; non-English debates resolve to needs_review until the round
/// limit, matching the historical deterministic behavior.
pub(super) struct HeuristicJudge;

impl DebateJudge for HeuristicJudge {
    fn judge(
        &self,
        run: &AstraRun,
        _thread: &ThreadInfo,
        _user_prompt: Option<&str>,
        source_round_index: u32,
        artifacts: &[JudgeLaneArtifact],
    ) -> Result<BackendResponse<JudgeVerdict>, BackendFailure> {
        let verdict = JudgeVerdict {
            status: heuristic_status(artifacts),
            agreements: Vec::new(),
            disagreements: Vec::new(),
            arbitration: None,
            rationale:
                "Keyword heuristic verdict; configure an Astra backend agent for structured judging."
                    .to_string(),
            attempts: 1,
        };
        Ok(BackendResponse {
            data: verdict,
            session_id: format!(
                "debate-judge-heuristic-{}-{}",
                run.run_id, source_round_index
            ),
            backend_type: HEURISTIC_JUDGE_BACKEND_TYPE.to_string(),
        })
    }
}

fn heuristic_status(artifacts: &[JudgeLaneArtifact]) -> JudgeStatus {
    if artifacts.is_empty() {
        return JudgeStatus::NeedsReview;
    }
    let texts = artifacts
        .iter()
        .map(|artifact| artifact.output.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if texts.iter().any(|text| {
        text.contains("disagree")
            || text.contains("diverge")
            || text.contains("conflict")
            || text.contains("reject")
    }) {
        return JudgeStatus::Diverged;
    }
    if texts.iter().all(|text| {
        text.contains("agree") || text.contains("converge") || text.contains("consensus")
    }) {
        return JudgeStatus::Converged;
    }
    JudgeStatus::NeedsReview
}

pub(super) struct RuntimeAgentJudge {
    runtime: RuntimeManager,
    config: RuntimeAgentBackendConfig,
}

impl RuntimeAgentJudge {
    pub(super) fn new(runtime: RuntimeManager, config: RuntimeAgentBackendConfig) -> Self {
        Self { runtime, config }
    }

    fn backend_type(&self) -> String {
        format!("runtime_agent_{}", self.config.agent.as_str())
    }
}

impl DebateJudge for RuntimeAgentJudge {
    fn judge(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        source_round_index: u32,
        artifacts: &[JudgeLaneArtifact],
    ) -> Result<BackendResponse<JudgeVerdict>, BackendFailure> {
        let prompt =
            build_debate_judge_prompt(run, thread, user_prompt, source_round_index, artifacts);
        let backend_type = self.backend_type();
        let (verdict, session_id) =
            judge_with_attempts(&backend_type, &prompt, |attempt_prompt| {
                execute_agent_session(
                    &self.runtime,
                    &self.config,
                    run,
                    thread,
                    &run.project_path,
                    attempt_prompt,
                    "debate_judge",
                )
            })?;
        Ok(BackendResponse {
            data: verdict,
            session_id,
            backend_type,
        })
    }
}

/// Runs the judge once and retries exactly once on parse/validation failures
/// (the corrected prompt restates the schema). Transport-class failures from
/// `execute` (timeout, turn errors) are returned as-is: they are likely to
/// recur and a retry would double the worst-case latency.
fn judge_with_attempts(
    backend_type: &str,
    prompt: &str,
    execute: impl FnMut(&str) -> Result<(String, String), BackendFailure>,
) -> Result<(JudgeVerdict, String), BackendFailure> {
    let (mut verdict, session_id, attempts) = execute_structured_with_retry(
        prompt,
        |text| parse_judge_verdict(backend_type, text),
        execute,
    )?;
    verdict.attempts = attempts;
    Ok((verdict, session_id))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJudgeVerdict {
    status: Option<String>,
    #[serde(default)]
    agreements: Option<Vec<String>>,
    #[serde(default)]
    disagreements: Option<Vec<String>>,
    #[serde(default)]
    arbitration: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
}

fn parse_judge_verdict(backend_type: &str, response: &str) -> Result<JudgeVerdict, BackendFailure> {
    let raw: RawJudgeVerdict = parse_yaml_mapping(backend_type, "debate judge", response)?;

    let status = raw
        .status
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    let status = match status.as_deref() {
        Some("converged") => JudgeStatus::Converged,
        Some("diverged") => JudgeStatus::Diverged,
        Some("needs_review") => JudgeStatus::NeedsReview,
        other => {
            return Err(BackendFailure::new(
                backend_type.to_string(),
                "validation_failed",
                format!(
                    "debate judge status must be converged, diverged, or needs_review (got {})",
                    other.unwrap_or("nothing")
                ),
            )
            .with_raw_response(response));
        }
    };
    let agreements = clean_string_list(
        raw.agreements.unwrap_or_default(),
        JUDGE_LIST_ITEM_CHAR_LIMIT,
        JUDGE_LIST_LEN_LIMIT,
    );
    let disagreements = clean_string_list(
        raw.disagreements.unwrap_or_default(),
        JUDGE_LIST_ITEM_CHAR_LIMIT,
        JUDGE_LIST_LEN_LIMIT,
    );
    if status == JudgeStatus::Diverged && disagreements.is_empty() {
        return Err(BackendFailure::new(
            backend_type.to_string(),
            "validation_failed",
            "a diverged verdict requires a non-empty disagreements list",
        )
        .with_raw_response(response));
    }
    let arbitration = raw
        .arbitration
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| truncate_chars(&value, JUDGE_LIST_ITEM_CHAR_LIMIT));
    let rationale = raw
        .rationale
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    Ok(JudgeVerdict {
        status,
        agreements,
        disagreements,
        arbitration,
        rationale,
        attempts: 1,
    })
}

fn build_debate_judge_prompt(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    source_round_index: u32,
    artifacts: &[JudgeLaneArtifact],
) -> String {
    let mut lines = Vec::new();
    lines.push(DEBATE_JUDGE_RESPONSE_CONTRACT.to_string());
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
    lines.push(format!("Cross-check round: {source_round_index}"));
    lines.push(format!("Round limit: {}", run.round_limit));
    lines.push(String::new());
    lines.push("## Lane cross-check artifacts".to_string());
    for artifact in artifacts {
        lines.push(String::new());
        lines.push(format!(
            "### Lane {} (agent {}, participant {})",
            artifact.lane_id,
            artifact.agent,
            artifact.participant_id.as_deref().unwrap_or("unknown")
        ));
        lines.push(truncate_chars(
            &artifact.output,
            JUDGE_LANE_OUTPUT_CHAR_LIMIT,
        ));
    }
    wrap_thread_prompt(
        "astra_debate_judge",
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
            goal: "Choose the safer architecture".to_string(),
            description: Some("Compare two proposals with evidence.".to_string()),
            stage_id: None,
            kind: ThreadKind::Debate,
            enabled: true,
            created_at: 1,
            updated_at: 1,
            assistants: Vec::new(),
            agent_participants: Vec::new(),
            stages: Vec::new(),
            sessions: Vec::new(),
        }
    }

    fn artifact(lane: &str, output: &str) -> JudgeLaneArtifact {
        JudgeLaneArtifact {
            lane_id: format!("lane-{lane}"),
            participant_id: Some(format!("participant-{lane}")),
            agent: "codex".to_string(),
            output: output.to_string(),
        }
    }

    #[test]
    fn parse_judge_verdict_accepts_valid_yaml_with_chinese_items() {
        let response = r#"status: diverged
agreements:
  - 双方都认可需要异步架构。
disagreements:
  - 方案A的延迟数据缺乏基准来源，需要补充测量方法。
arbitration: 建议由人工复核延迟基准。
rationale: 双方在延迟证据上仍有实质分歧。"#;

        let verdict = parse_judge_verdict("runtime_agent_claude", response).unwrap();

        assert_eq!(verdict.status, JudgeStatus::Diverged);
        assert_eq!(verdict.agreements, vec!["双方都认可需要异步架构。"]);
        assert_eq!(
            verdict.disagreements,
            vec!["方案A的延迟数据缺乏基准来源，需要补充测量方法。"]
        );
        assert_eq!(
            verdict.arbitration.as_deref(),
            Some("建议由人工复核延迟基准。")
        );
        assert_eq!(verdict.rationale, "双方在延迟证据上仍有实质分歧。");
    }

    #[test]
    fn parse_judge_verdict_rejects_code_fence_json_and_empty() {
        let fenced = parse_judge_verdict("judge", "```yaml\nstatus: converged\n```").unwrap_err();
        assert_eq!(fenced.code, "invalid_yaml");
        assert!(fenced.raw_response_snippet.is_some());

        let json = parse_judge_verdict("judge", "{\"status\": \"converged\"}").unwrap_err();
        assert_eq!(json.code, "invalid_yaml");

        let empty = parse_judge_verdict("judge", "   ").unwrap_err();
        assert_eq!(empty.code, "empty_response");
    }

    #[test]
    fn parse_judge_verdict_rejects_missing_or_unknown_status() {
        let missing = parse_judge_verdict("judge", "rationale: no status").unwrap_err();
        assert_eq!(missing.code, "validation_failed");

        let unknown = parse_judge_verdict("judge", "status: maybe\nrationale: x").unwrap_err();
        assert_eq!(unknown.code, "validation_failed");
    }

    #[test]
    fn parse_judge_verdict_rejects_diverged_without_disagreements() {
        let response = "status: diverged\ndisagreements: []\nrationale: conflicting";

        let failure = parse_judge_verdict("judge", response).unwrap_err();

        assert_eq!(failure.code, "validation_failed");
        assert!(failure.message.contains("disagreements"));
    }

    #[test]
    fn parse_judge_verdict_rejects_unknown_keys() {
        let response = "status: converged\nverdictScore: 10";

        let failure = parse_judge_verdict("judge", response).unwrap_err();

        assert_eq!(failure.code, "invalid_yaml");
    }

    #[test]
    fn parse_judge_verdict_bounds_list_items_and_length() {
        let long_item = "a".repeat(700);
        let items = (0..15)
            .map(|index| format!("  - disagreement {index} {long_item}"))
            .collect::<Vec<_>>()
            .join("\n");
        let response = format!("status: diverged\ndisagreements:\n{items}");

        let verdict = parse_judge_verdict("judge", &response).unwrap();

        assert_eq!(verdict.disagreements.len(), JUDGE_LIST_LEN_LIMIT);
        assert!(verdict
            .disagreements
            .iter()
            .all(|item| item.chars().count() <= JUDGE_LIST_ITEM_CHAR_LIMIT));
        assert!(verdict.disagreements[0].ends_with("..."));
    }

    #[test]
    fn heuristic_judge_matches_english_keywords_only() {
        let judge = HeuristicJudge;

        let diverged = judge
            .judge(
                &run(),
                &thread(),
                None,
                1,
                &[
                    artifact("a", "I disagree with the latency claims."),
                    artifact("b", "We agree on the approach."),
                ],
            )
            .unwrap();
        assert_eq!(diverged.data.status, JudgeStatus::Diverged);
        assert_eq!(diverged.backend_type, HEURISTIC_JUDGE_BACKEND_TYPE);
        assert_eq!(diverged.session_id, "debate-judge-heuristic-run-1-1");

        let converged = judge
            .judge(
                &run(),
                &thread(),
                None,
                1,
                &[
                    artifact("a", "Final result: agree with caveats."),
                    artifact("b", "We reached consensus."),
                ],
            )
            .unwrap();
        assert_eq!(converged.data.status, JudgeStatus::Converged);

        // Chinese output never matches the English keywords: the fallback
        // stays needs_review, preserving the historical behavior baseline.
        let chinese = judge
            .judge(
                &run(),
                &thread(),
                None,
                1,
                &[
                    artifact("a", "我不同意对方的延迟结论。"),
                    artifact("b", "我们已经达成一致。"),
                ],
            )
            .unwrap();
        assert_eq!(chinese.data.status, JudgeStatus::NeedsReview);
    }

    #[test]
    fn judge_with_attempts_retries_once_on_parse_failure() {
        let mut prompts = Vec::new();
        let mut responses = vec![
            "not: valid\nstatus: maybe".to_string(),
            "status: converged\nrationale: all lanes accept the plan".to_string(),
        ]
        .into_iter();

        let (verdict, session_id) = judge_with_attempts("judge", "base prompt", |prompt| {
            prompts.push(prompt.to_string());
            Ok((
                responses.next().unwrap(),
                format!("session-{}", prompts.len()),
            ))
        })
        .unwrap();

        assert_eq!(verdict.status, JudgeStatus::Converged);
        assert_eq!(verdict.attempts, 2);
        assert_eq!(session_id, "session-2");
        assert_eq!(prompts.len(), 2);
        assert!(prompts[1].contains("## Correction"));
        assert!(prompts[1].contains("base prompt"));
    }

    #[test]
    fn judge_with_attempts_does_not_retry_transport_failures() {
        let mut calls = 0;

        let failure = judge_with_attempts("judge", "base prompt", |_| {
            calls += 1;
            Err(BackendFailure::new("judge", "timeout", "timed out"))
        })
        .unwrap_err();

        assert_eq!(calls, 1);
        assert_eq!(failure.code, "timeout");
    }

    #[test]
    fn judge_with_attempts_returns_last_parse_failure_with_session() {
        let failure = judge_with_attempts("judge", "base prompt", |_| {
            Ok(("status: maybe".to_string(), "session-x".to_string()))
        })
        .unwrap_err();

        assert_eq!(failure.code, "validation_failed");
        assert_eq!(failure.session_id.as_deref(), Some("session-x"));
        assert!(failure.raw_response_snippet.is_some());
    }

    #[test]
    fn judge_prompt_includes_contract_full_artifacts_and_wrapping() {
        let long_output = "观点细节 ".repeat(300);
        assert!(long_output.chars().count() > 1000);
        let prompt = build_debate_judge_prompt(
            &run(),
            &thread(),
            Some("Be strict"),
            2,
            &[artifact("a", &long_output), artifact("b", "short")],
        );

        assert!(prompt.contains("status: converged|diverged|needs_review"));
        assert!(prompt.contains("Thread goal: Choose the safer architecture"));
        assert!(prompt.contains("User debate instruction: Be strict"));
        assert!(prompt.contains("Cross-check round: 2"));
        assert!(prompt.contains("### Lane lane-a (agent codex, participant participant-a)"));
        assert!(prompt.contains("in the same language as the debate artifacts"));
        assert!(prompt.contains(long_output.trim()));
        assert!(prompt.contains("sessio-thread-prompt:start"));
    }

    #[test]
    fn judge_prompt_truncates_lane_output_beyond_limit() {
        let oversized = "x".repeat(JUDGE_LANE_OUTPUT_CHAR_LIMIT + 100);

        let prompt =
            build_debate_judge_prompt(&run(), &thread(), None, 1, &[artifact("a", &oversized)]);

        assert!(!prompt.contains(&oversized));
        assert!(prompt.contains(&format!(
            "{}...",
            "x".repeat(JUDGE_LANE_OUTPUT_CHAR_LIMIT - 3)
        )));
    }
}
