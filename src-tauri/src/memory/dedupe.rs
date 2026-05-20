use std::collections::HashSet;

use anyhow::Result;

use crate::memory::{MemoryStore, SessionTimeInfo, TurnFingerprint};
use crate::providers::types::SessionSource;

const MIN_SHARED_HASHES: usize = 3;
const MAX_SKIP_A: usize = 2;
const MAX_SKIP_B: usize = 2;
const MIN_MATCHED_TURNS: usize = 4;
const MIN_STRONG_PREFIX_COVERAGE: f64 = 0.85;
const MIN_STRONG_TOTAL_COVERAGE: f64 = 0.80;
const MIN_STRONG_CONTIGUOUS_RUN: usize = 4;
const MAX_STRONG_NEW_TAIL_WEIGHT: f64 = 2.0;
const MIN_WEAK_PREFIX_COVERAGE: f64 = 0.90;
const MIN_WEAK_TOTAL_COVERAGE: f64 = 0.75;
const MIN_WEAK_CONTIGUOUS_RUN: usize = 5;
const MAX_WEAK_NEW_TAIL_WEIGHT: f64 = 3.0;
const SHORT_USER_CHARS: usize = 12;
const SHORT_ASSISTANT_CHARS: usize = 16;
const LONG_USER_CHARS: usize = 24;
const LONG_ASSISTANT_CHARS: usize = 32;

#[derive(Debug, Clone)]
pub struct DedupeMatch {
    pub action: DedupeAction,
    pub source_agent: String,
    pub source_session_id: String,
    pub source_file_path: String,
    pub source_first_matched_turn_index: usize,
    pub source_first_matched_line_start: Option<u64>,
    pub source_first_matched_byte_start: Option<u64>,
    pub source_last_matched_turn_index: usize,
    pub source_last_matched_line_end: Option<u64>,
    pub source_last_matched_byte_end: Option<u64>,
    pub shared_hashes: usize,
    pub suffix_start_turn_index: usize,
    pub matched_turns: usize,
    pub prefix_coverage: f64,
    pub total_coverage: f64,
    pub longest_contiguous_run: usize,
    pub new_tail_weight: f64,
    pub new_tail_user_or_assistant_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupeAction {
    SuppressWholeSource,
    TrimPrefix,
}

#[derive(Debug, Clone)]
struct WeightedFingerprint {
    turn_index: usize,
    role: String,
    canonical_hash: String,
    text_len: usize,
    weight: f64,
    line_start: Option<u64>,
    line_end: Option<u64>,
    byte_start: Option<u64>,
    byte_end: Option<u64>,
}

#[derive(Debug, Clone)]
struct MatchScore {
    matched_turns: usize,
    source_first_matched_turn_index: usize,
    source_first_matched_line_start: Option<u64>,
    source_first_matched_byte_start: Option<u64>,
    source_last_matched_turn_index: usize,
    source_last_matched_line_end: Option<u64>,
    source_last_matched_byte_end: Option<u64>,
    last_matched_turn_index: usize,
    prefix_coverage: f64,
    total_coverage: f64,
    longest_contiguous_run: usize,
    new_tail_weight: f64,
    new_tail_user_or_assistant_count: usize,
}

pub fn should_suppress_source(
    store: &dyn MemoryStore,
    source: &SessionSource,
    fingerprints: &[TurnFingerprint],
) -> Result<Option<DedupeMatch>> {
    let Some(project) = &source.project else {
        return Ok(None);
    };
    let current = informative_fingerprints(fingerprints);
    if current.is_empty() {
        return Ok(None);
    }

    let candidate_hashes = current
        .iter()
        .map(|fp| fp.canonical_hash.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let candidates = store.find_turn_fingerprint_candidates(
        &project.project_key,
        source.agent.as_str(),
        &source.session_id,
        &candidate_hashes,
        20,
    )?;

    for candidate in candidates {
        let candidate_time = store.session_time_info(&candidate.agent, &candidate.session_id)?;
        if !is_allowed_candidate(
            source,
            &candidate.agent,
            &candidate.session_id,
            candidate_time,
        ) {
            continue;
        }
        let records = store.list_records_for_source(
            &candidate.agent,
            &candidate.session_id,
            &candidate.file_path,
        )?;
        if !records.iter().any(|record| record.available) {
            continue;
        }

        let existing = store.list_turn_fingerprints(
            &project.project_key,
            &candidate.agent,
            &candidate.session_id,
        )?;
        let existing = informative_fingerprints(&existing);
        if existing.is_empty() {
            continue;
        }

        let shared_hashes = shared_hash_count(&current, &existing);
        if shared_hashes < MIN_SHARED_HASHES {
            continue;
        }

        if let Some(score) = best_confirmed_prefix_alignment(&existing, &current) {
            let suppress = should_suppress_score(&score);
            let trim = should_trim_score(&score);
            if suppress || trim {
                return Ok(Some(DedupeMatch {
                    action: if suppress {
                        DedupeAction::SuppressWholeSource
                    } else {
                        DedupeAction::TrimPrefix
                    },
                    source_agent: candidate.agent,
                    source_session_id: candidate.session_id,
                    source_file_path: candidate.file_path,
                    source_first_matched_turn_index: score.source_first_matched_turn_index,
                    source_first_matched_line_start: score.source_first_matched_line_start,
                    source_first_matched_byte_start: score.source_first_matched_byte_start,
                    source_last_matched_turn_index: score.source_last_matched_turn_index,
                    source_last_matched_line_end: score.source_last_matched_line_end,
                    source_last_matched_byte_end: score.source_last_matched_byte_end,
                    shared_hashes,
                    suffix_start_turn_index: score.last_matched_turn_index.saturating_add(1),
                    matched_turns: score.matched_turns,
                    prefix_coverage: score.prefix_coverage,
                    total_coverage: score.total_coverage,
                    longest_contiguous_run: score.longest_contiguous_run,
                    new_tail_weight: score.new_tail_weight,
                    new_tail_user_or_assistant_count: score.new_tail_user_or_assistant_count,
                }));
            }
        }
    }

    Ok(None)
}

fn is_allowed_candidate(
    source: &SessionSource,
    candidate_agent: &str,
    candidate_session_id: &str,
    candidate_time: Option<SessionTimeInfo>,
) -> bool {
    if candidate_agent != source.agent.as_str() {
        return false;
    }
    // Codex sessions carry an explicit fork lineage. When present, the
    // only valid base is the parent — never a sibling, regardless of
    // timestamps. When absent, fall through to the same time-based
    // ordering used by other agents.
    if source.agent.as_str() == "codex" {
        if let Some(forked_from_id) = source
            .metadata
            .get("forked_from_id")
            .and_then(|value| value.as_str())
        {
            return candidate_session_id == forked_from_id;
        }
    }

    let candidate_started_at = candidate_time.as_ref().and_then(|info| info.started_at);
    let candidate_updated_at = candidate_time.as_ref().and_then(|info| info.updated_at);
    let source_started_at = metadata_i64(&source.metadata, "started_at");
    let source_updated_at = metadata_i64(&source.metadata, "updated_at");

    if let (Some(candidate_started_at), Some(source_started_at)) =
        (candidate_started_at, source_started_at)
    {
        if candidate_started_at != source_started_at {
            return candidate_started_at < source_started_at;
        }
    }
    if let (Some(candidate_updated_at), Some(source_updated_at)) =
        (candidate_updated_at, source_updated_at)
    {
        if candidate_updated_at != source_updated_at {
            return candidate_updated_at < source_updated_at;
        }
    }

    candidate_session_id < source.session_id.as_str()
}

fn metadata_i64(
    metadata: &std::collections::BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Option<i64> {
    metadata.get(key).and_then(|value| value.as_i64())
}

fn informative_fingerprints(fingerprints: &[TurnFingerprint]) -> Vec<WeightedFingerprint> {
    fingerprints
        .iter()
        .filter_map(|fp| {
            let normalized_role = normalize_role_name(&fp.role);
            let weight = role_weight(normalized_role, fp.text_len);
            if weight <= 0.0 {
                None
            } else {
                Some(WeightedFingerprint {
                    turn_index: fp.turn_index,
                    role: normalized_role.to_string(),
                    canonical_hash: fp.canonical_hash.clone(),
                    text_len: fp.text_len,
                    weight,
                    line_start: fp.location.line_start,
                    line_end: fp.location.line_end,
                    byte_start: fp.location.byte_start,
                    byte_end: fp.location.byte_end,
                })
            }
        })
        .collect()
}

fn best_confirmed_prefix_alignment(
    existing: &[WeightedFingerprint],
    candidate: &[WeightedFingerprint],
) -> Option<MatchScore> {
    let candidate = filter_informative(candidate);
    if candidate.is_empty() {
        return None;
    }

    let mut best: Option<MatchScore> = None;

    for (a_idx, anchor_a) in existing.iter().enumerate() {
        if anchor_a.role != candidate[0].role
            || anchor_a.canonical_hash != candidate[0].canonical_hash
        {
            continue;
        }
        let score = align_from_prefix_start(existing, &candidate, a_idx);
        best = Some(best_score(best, score));
    }

    best
}

fn align_from_prefix_start(
    existing: &[WeightedFingerprint],
    candidate: &[WeightedFingerprint],
    anchor_a: usize,
) -> MatchScore {
    let mut matched_candidate = vec![false; candidate.len()];
    let mut matched_weight = 0.0;
    let mut matched_turns = 1;
    let mut longest_contiguous_run = 1;
    let mut current_run = 1;
    let mut last_matched_a = anchor_a;
    let mut last_matched_b = 0;

    matched_candidate[0] = true;
    matched_weight += candidate[0].weight;

    let mut ai = anchor_a + 1;
    let mut bi = 1;
    while ai < existing.len() && bi < candidate.len() {
        let mut found = false;
        let a_end = (ai + MAX_SKIP_A + 1).min(existing.len());
        let b_end = (bi + MAX_SKIP_B + 1).min(candidate.len());
        // The inner loops break out of `'search` on the first match. Assigning
        // `ai` / `bi` is for the *next* iteration of the outer `while`, where
        // the ranges are recomputed — the rebinding doesn't affect the range
        // currently being iterated.
        #[allow(clippy::mut_range_bound)]
        'search: for (next_a, existing_item) in existing.iter().enumerate().take(a_end).skip(ai) {
            for (next_b, candidate_item) in candidate.iter().enumerate().take(b_end).skip(bi) {
                if existing_item.role == candidate_item.role
                    && existing_item.canonical_hash == candidate_item.canonical_hash
                {
                    ai = next_a;
                    bi = next_b;
                    found = true;
                    break 'search;
                }
            }
        }
        if !found {
            break;
        }

        matched_candidate[bi] = true;
        matched_weight += candidate[bi].weight;
        matched_turns += 1;
        if bi == last_matched_b + 1 {
            current_run += 1;
        } else {
            current_run = 1;
        }
        longest_contiguous_run = longest_contiguous_run.max(current_run);
        last_matched_a = ai;
        last_matched_b = bi;
        ai += 1;
        bi += 1;
    }

    let total_weight: f64 = candidate.iter().map(|fp| fp.weight).sum();
    let confirmed_prefix_weight: f64 = candidate[..=last_matched_b]
        .iter()
        .map(|fp| fp.weight)
        .sum();
    let matched_prefix_weight: f64 = candidate[..=last_matched_b]
        .iter()
        .enumerate()
        .filter_map(|(idx, fp)| matched_candidate[idx].then_some(fp.weight))
        .sum();
    let mut new_tail_weight = 0.0;
    let mut new_tail_user_or_assistant_count = 0;
    for (idx, fp) in candidate.iter().enumerate().skip(last_matched_b + 1) {
        if matched_candidate[idx] {
            continue;
        }
        new_tail_weight += fp.weight;
        if is_long_user_or_assistant(fp) {
            new_tail_user_or_assistant_count += 1;
        }
    }

    MatchScore {
        matched_turns,
        source_first_matched_turn_index: existing[anchor_a].turn_index,
        source_first_matched_line_start: existing[anchor_a].line_start,
        source_first_matched_byte_start: existing[anchor_a].byte_start,
        source_last_matched_turn_index: existing[last_matched_a].turn_index,
        source_last_matched_line_end: existing[last_matched_a].line_end,
        source_last_matched_byte_end: existing[last_matched_a].byte_end,
        last_matched_turn_index: candidate[last_matched_b].turn_index,
        prefix_coverage: if confirmed_prefix_weight > 0.0 {
            matched_prefix_weight / confirmed_prefix_weight
        } else {
            0.0
        },
        total_coverage: if total_weight > 0.0 {
            matched_weight / total_weight
        } else {
            0.0
        },
        longest_contiguous_run,
        new_tail_weight,
        new_tail_user_or_assistant_count,
    }
}

fn best_score(current: Option<MatchScore>, next: MatchScore) -> MatchScore {
    let Some(current) = current else {
        return next;
    };
    if next.last_matched_turn_index > current.last_matched_turn_index {
        return next;
    }
    if next.last_matched_turn_index == current.last_matched_turn_index
        && next.prefix_coverage > current.prefix_coverage
    {
        return next;
    }
    if next.last_matched_turn_index == current.last_matched_turn_index
        && (next.prefix_coverage - current.prefix_coverage).abs() < f64::EPSILON
        && next.matched_turns > current.matched_turns
    {
        return next;
    }
    current
}

fn should_suppress_score(score: &MatchScore) -> bool {
    score.matched_turns >= MIN_MATCHED_TURNS
        && score.longest_contiguous_run >= MIN_MATCHED_TURNS
        && ((score.prefix_coverage >= MIN_STRONG_PREFIX_COVERAGE
            && score.total_coverage >= MIN_STRONG_TOTAL_COVERAGE
            && score.longest_contiguous_run >= MIN_STRONG_CONTIGUOUS_RUN
            && score.new_tail_weight <= MAX_STRONG_NEW_TAIL_WEIGHT
            && score.new_tail_user_or_assistant_count == 0)
            || (score.prefix_coverage >= MIN_WEAK_PREFIX_COVERAGE
                && score.total_coverage >= MIN_WEAK_TOTAL_COVERAGE
                && score.longest_contiguous_run >= MIN_WEAK_CONTIGUOUS_RUN
                && score.new_tail_weight <= MAX_WEAK_NEW_TAIL_WEIGHT
                && score.new_tail_user_or_assistant_count == 0))
}

fn should_trim_score(score: &MatchScore) -> bool {
    score.matched_turns >= MIN_MATCHED_TURNS
        && score.longest_contiguous_run >= MIN_STRONG_CONTIGUOUS_RUN
        && score.prefix_coverage >= MIN_STRONG_PREFIX_COVERAGE
        && score.new_tail_weight > 0.0
}

fn filter_informative(fingerprints: &[WeightedFingerprint]) -> Vec<WeightedFingerprint> {
    fingerprints
        .iter()
        .filter(|fp| fp.weight > 0.0)
        .cloned()
        .collect()
}

fn shared_hash_count(a: &[WeightedFingerprint], b: &[WeightedFingerprint]) -> usize {
    let hashes: HashSet<&str> = a.iter().map(|fp| fp.canonical_hash.as_str()).collect();
    let mut count = 0;
    let mut seen = HashSet::new();
    for fp in b {
        if hashes.contains(fp.canonical_hash.as_str()) && seen.insert(fp.canonical_hash.as_str()) {
            count += 1;
        }
    }
    count
}

fn is_long_user_or_assistant(fp: &WeightedFingerprint) -> bool {
    match fp.role.as_str() {
        "user" => fp.text_len >= LONG_USER_CHARS,
        "assistant" => fp.text_len >= LONG_ASSISTANT_CHARS,
        _ => false,
    }
}

fn normalize_role_name(role: &str) -> &str {
    match role {
        "tooluse" | "tool_use" | "toolcall" | "tool_call" => "tool_use",
        "toolresult" | "tool_result" | "function_call_output" => "tool_result",
        other => other,
    }
}

fn role_weight(role: &str, text_len: usize) -> f64 {
    match role {
        "user" if text_len < SHORT_USER_CHARS => 0.25,
        "user" => 3.0,
        "assistant" if text_len < SHORT_ASSISTANT_CHARS => 0.25,
        "assistant" => 3.0,
        "thinking" => 2.0,
        "tool_use" => 1.5,
        "tool_result" => 1.0,
        _ => 0.0,
    }
}
