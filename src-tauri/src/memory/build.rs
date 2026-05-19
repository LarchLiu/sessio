use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::memory::artifacts::{MarkdownArtifactSink, MemoryArtifactSink};
use crate::memory::cards::{cards_for_source, fingerprints_for_source, safe_id_part};
use crate::memory::dedupe::{should_suppress_source, DedupeAction, DedupeMatch};
use crate::memory::normalize::normalize_events;
use crate::memory::{CardContinuation, MemoryStore, TurnFingerprint};
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::{MessageEvent, MessageRole, SessionSource};

#[derive(Debug, Clone)]
pub struct MemoryBuildOptions {
    pub project_path: PathBuf,
    pub artifacts_root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBuildSummary {
    pub project_path: String,
    pub project_key: Option<String>,
    pub sources_seen: usize,
    pub sources_built: usize,
    pub sources_skipped: usize,
    pub cards_written: usize,
    pub artifacts_root: String,
    pub errors: Vec<String>,
    pub dependent_source_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct MemoryBuildSourceResult {
    pub project_key: Option<String>,
    pub cards_written: usize,
    pub cards_marked_unavailable: usize,
    pub dependent_source_paths: Vec<PathBuf>,
}

pub fn build_project_memory(
    registry: &ProviderRegistry,
    store: &dyn MemoryStore,
    options: &MemoryBuildOptions,
) -> Result<MemoryBuildSummary> {
    build_project_memory_with_backend(registry, store, "qmd", options)
}

pub fn build_project_memory_with_backend(
    registry: &ProviderRegistry,
    store: &dyn MemoryStore,
    backend: &str,
    options: &MemoryBuildOptions,
) -> Result<MemoryBuildSummary> {
    let wanted_project = normalize_path(&options.project_path);
    let mut summary = MemoryBuildSummary {
        project_path: wanted_project.clone(),
        project_key: None,
        sources_seen: 0,
        sources_built: 0,
        sources_skipped: 0,
        cards_written: 0,
        artifacts_root: options.artifacts_root.to_string_lossy().to_string(),
        dependent_source_paths: Vec::new(),
        errors: Vec::new(),
    };
    let mut seen_sources = HashSet::new();
    let artifact_sink = MarkdownArtifactSink::new(options.artifacts_root.clone());

    for provider in registry.providers() {
        let sources = match provider.discover() {
            Ok(sources) => sources,
            Err(e) => {
                summary.sources_skipped += 1;
                summary.errors.push(format!(
                    "discover {} sources failed: {e}",
                    provider.display_name()
                ));
                continue;
            }
        };
        for source in sources {
            summary.sources_seen += 1;
            let Some(project) = &source.project else {
                continue;
            };
            let Some(source_project_path) = project.project_path.as_deref() else {
                continue;
            };
            if normalize_path(source_project_path) != wanted_project {
                continue;
            }

            summary.project_key = Some(project.project_key.clone());
            seen_sources.insert(source_key(&source));
            let events = match provider.read_messages(&source) {
                Ok(events) => events,
                Err(e) => {
                    summary.sources_skipped += 1;
                    summary.errors.push(format!(
                        "read {} {} from {} failed: {e}",
                        source.agent.as_str(),
                        source.session_id,
                        source.file_path
                    ));
                    if let Err(mark_error) = store.mark_source_cards_unavailable(
                        source.agent.as_str(),
                        &source.session_id,
                        &source.file_path,
                    ) {
                        summary.errors.push(format!(
                            "mark source unavailable {} {} failed: {mark_error}",
                            source.agent.as_str(),
                            source.file_path
                        ));
                    }
                    if let Err(remove_error) =
                        remove_existing_source_card_files(store, &artifact_sink, backend, &source)
                    {
                        summary.errors.push(format!(
                            "remove source card files {} {} failed: {remove_error}",
                            source.agent.as_str(),
                            source.file_path
                        ));
                    }
                    if let Err(fp_error) = clear_source_fingerprints(store, &source) {
                        summary.errors.push(format!(
                            "clear fingerprints {} {} failed: {fp_error}",
                            source.agent.as_str(),
                            source.file_path
                        ));
                    }
                    continue;
                }
            };
            let events = normalize_events(events);
            if events.is_empty() {
                if let Err(mark_error) = store.mark_source_cards_unavailable(
                    source.agent.as_str(),
                    &source.session_id,
                    &source.file_path,
                ) {
                    summary.errors.push(format!(
                        "mark empty source unavailable {} {} failed: {mark_error}",
                        source.agent.as_str(),
                        source.file_path
                    ));
                }
                if let Err(remove_error) =
                    remove_existing_source_card_files(store, &artifact_sink, backend, &source)
                {
                    summary.errors.push(format!(
                        "remove empty source card files {} {} failed: {remove_error}",
                        source.agent.as_str(),
                        source.file_path
                    ));
                }
                if let Err(fp_error) = clear_source_fingerprints(store, &source) {
                    summary.errors.push(format!(
                        "clear fingerprints {} {} failed: {fp_error}",
                        source.agent.as_str(),
                        source.file_path
                    ));
                }
                continue;
            }

            let fingerprints = fingerprints_for_source(&source, &events);
            let plan =
                resolve_dedupe_plan(store, &project.project_key, &source, &events, &fingerprints)?;
            let (card_events, card_continuation) = match plan {
                DedupePlan::Pass => (events.as_slice(), None),
                DedupePlan::Trim {
                    offset,
                    continuation,
                } => (&events[offset..], Some(continuation)),
                DedupePlan::Suppress { reason } => {
                    summary.sources_skipped += 1;
                    summary.errors.push(reason);
                    if let Err(mark_error) = store.mark_source_cards_unavailable(
                        source.agent.as_str(),
                        &source.session_id,
                        &source.file_path,
                    ) {
                        summary.errors.push(format!(
                            "mark suppressed source unavailable {} {} failed: {mark_error}",
                            source.agent.as_str(),
                            source.file_path
                        ));
                    }
                    if let Err(remove_error) =
                        remove_existing_source_card_files(store, &artifact_sink, backend, &source)
                    {
                        summary.errors.push(format!(
                            "remove suppressed source card files {} {} failed: {remove_error}",
                            source.agent.as_str(),
                            source.file_path
                        ));
                    }
                    if let Err(fp_error) = clear_source_fingerprints(store, &source) {
                        summary.errors.push(format!(
                            "clear fingerprints {} {} failed: {fp_error}",
                            source.agent.as_str(),
                            source.file_path
                        ));
                    }
                    continue;
                }
            };

            let generated = cards_for_source(&source, card_events);
            if generated.is_empty() {
                if let Err(mark_error) = store.mark_source_cards_unavailable(
                    source.agent.as_str(),
                    &source.session_id,
                    &source.file_path,
                ) {
                    summary.errors.push(format!(
                        "mark source without cards unavailable {} {} failed: {mark_error}",
                        source.agent.as_str(),
                        source.file_path
                    ));
                }
                if let Err(remove_error) =
                    remove_existing_source_card_files(store, &artifact_sink, backend, &source)
                {
                    summary.errors.push(format!(
                        "remove source without cards files {} {} failed: {remove_error}",
                        source.agent.as_str(),
                        source.file_path
                    ));
                }
                if let Err(fp_error) = clear_source_fingerprints(store, &source) {
                    summary.errors.push(format!(
                        "clear fingerprints {} {} failed: {fp_error}",
                        source.agent.as_str(),
                        source.file_path
                    ));
                }
                continue;
            }
            summary.sources_built += 1;
            store.replace_turn_fingerprints(
                &project.project_key,
                source.agent.as_str(),
                &source.session_id,
                &fingerprints,
            )?;
            let dependent_source_paths = invalidate_dependent_continuations(
                store,
                &artifact_sink,
                backend,
                source.agent.as_str(),
                &source.session_id,
                &mut summary.errors,
            );
            extend_unique_paths(&mut summary.dependent_source_paths, dependent_source_paths);
            for (card, sources) in generated {
                store.upsert_card(&card)?;
                let artifact =
                    artifact_sink.write_record_artifact(backend, &card.project_key, &card)?;
                store.upsert_memory_artifact(&artifact)?;
                store.replace_card_sources(&card.record_id, &sources)?;
                store.replace_card_continuation(&card.record_id, card_continuation.as_ref())?;
                summary.cards_written += 1;
            }
        }
    }

    if let Some(project_key) = summary.project_key.as_deref() {
        for card in store.list_project_cards(project_key)? {
            if !card.available {
                continue;
            }
            let sources = store.sources_for_card(&card.record_id)?;
            if sources.is_empty() {
                continue;
            }
            let all_missing = sources.iter().all(|source| {
                !seen_sources.contains(&source_key_parts(
                    &source.agent,
                    &source.session_id,
                    &source.file_path,
                ))
            });
            if all_missing {
                store.mark_card_unavailable(&card.record_id)?;
                remove_record_artifact(
                    &artifact_sink,
                    backend,
                    &card.project_key,
                    &card.record_id,
                )?;
            }
        }
    }

    Ok(summary)
}

pub fn build_source_memory(
    registry: &ProviderRegistry,
    store: &dyn MemoryStore,
    artifacts_root: &Path,
    source: &SessionSource,
) -> Result<MemoryBuildSourceResult> {
    build_source_memory_with_backend(registry, store, "qmd", artifacts_root, source)
}

pub fn build_source_memory_with_backend(
    registry: &ProviderRegistry,
    store: &dyn MemoryStore,
    backend: &str,
    artifacts_root: &Path,
    source: &SessionSource,
) -> Result<MemoryBuildSourceResult> {
    let Some(project) = &source.project else {
        return Ok(MemoryBuildSourceResult {
            project_key: None,
            cards_written: 0,
            cards_marked_unavailable: 0,
            dependent_source_paths: Vec::new(),
        });
    };
    let Some(provider) = registry.provider_for_agent(&source.agent) else {
        anyhow::bail!("no provider for agent {}", source.agent.as_str());
    };
    let artifact_sink = MarkdownArtifactSink::new(artifacts_root.to_path_buf());

    let existing = store.list_cards_for_source(
        source.agent.as_str(),
        &source.session_id,
        &source.file_path,
    )?;
    let mut marked_unavailable = 0;

    let events = provider.read_messages(source)?;
    let events = normalize_events(events);
    if events.is_empty() {
        for card in existing {
            if card.available {
                store.mark_card_unavailable(&card.record_id)?;
                remove_record_artifact(
                    &artifact_sink,
                    backend,
                    &card.project_key,
                    &card.record_id,
                )?;
                marked_unavailable += 1;
            }
        }
        clear_source_fingerprints(store, source)?;
        return Ok(MemoryBuildSourceResult {
            project_key: Some(project.project_key.clone()),
            cards_written: 0,
            cards_marked_unavailable: marked_unavailable,
            dependent_source_paths: Vec::new(),
        });
    }

    let fingerprints = fingerprints_for_source(source, &events);
    let plan = resolve_dedupe_plan(store, &project.project_key, source, &events, &fingerprints)?;
    let (card_events, card_continuation) = match plan {
        DedupePlan::Pass => (events.as_slice(), None),
        DedupePlan::Trim {
            offset,
            continuation,
        } => (&events[offset..], Some(continuation)),
        DedupePlan::Suppress { reason: _ } => {
            for card in existing {
                if card.available {
                    store.mark_card_unavailable(&card.record_id)?;
                    remove_record_artifact(
                        &artifact_sink,
                        backend,
                        &card.project_key,
                        &card.record_id,
                    )?;
                    marked_unavailable += 1;
                }
            }
            clear_source_fingerprints(store, source)?;
            return Ok(MemoryBuildSourceResult {
                project_key: Some(project.project_key.clone()),
                cards_written: 0,
                cards_marked_unavailable: marked_unavailable,
                dependent_source_paths: Vec::new(),
            });
        }
    };

    let generated = cards_for_source(source, card_events);
    if generated.is_empty() {
        for card in existing {
            if card.available {
                store.mark_card_unavailable(&card.record_id)?;
                remove_record_artifact(
                    &artifact_sink,
                    backend,
                    &card.project_key,
                    &card.record_id,
                )?;
                marked_unavailable += 1;
            }
        }
        clear_source_fingerprints(store, source)?;
        return Ok(MemoryBuildSourceResult {
            project_key: Some(project.project_key.clone()),
            cards_written: 0,
            cards_marked_unavailable: marked_unavailable,
            dependent_source_paths: Vec::new(),
        });
    }

    let generated_ids = generated
        .iter()
        .map(|(card, _)| card.record_id.clone())
        .collect::<std::collections::HashSet<_>>();
    for card in existing {
        if card.available && !generated_ids.contains(&card.record_id) {
            store.mark_card_unavailable(&card.record_id)?;
            remove_record_artifact(&artifact_sink, backend, &card.project_key, &card.record_id)?;
            marked_unavailable += 1;
        }
    }

    store.replace_turn_fingerprints(
        &project.project_key,
        source.agent.as_str(),
        &source.session_id,
        &fingerprints,
    )?;
    let mut invalidation_errors: Vec<String> = Vec::new();
    let dependent_source_paths = invalidate_dependent_continuations(
        store,
        &artifact_sink,
        backend,
        source.agent.as_str(),
        &source.session_id,
        &mut invalidation_errors,
    );
    for error in invalidation_errors {
        log::warn!("{error}");
    }

    let mut cards_written = 0;
    for (card, sources) in generated {
        store.upsert_card(&card)?;
        let artifact = artifact_sink.write_record_artifact(backend, &card.project_key, &card)?;
        store.upsert_memory_artifact(&artifact)?;
        store.replace_card_sources(&card.record_id, &sources)?;
        store.replace_card_continuation(&card.record_id, card_continuation.as_ref())?;
        cards_written += 1;
    }

    Ok(MemoryBuildSourceResult {
        project_key: Some(project.project_key.clone()),
        cards_written,
        cards_marked_unavailable: marked_unavailable,
        dependent_source_paths,
    })
}

pub fn default_artifacts_root() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    remove_legacy_qmd_memory_root(&home)?;
    Ok(home
        .join(".sessio")
        .join("memory")
        .join("qmd")
        .join("projects"))
}

fn remove_existing_source_card_files(
    store: &dyn MemoryStore,
    artifact_sink: &dyn MemoryArtifactSink,
    backend: &str,
    source: &SessionSource,
) -> Result<()> {
    for card in
        store.list_cards_for_source(source.agent.as_str(), &source.session_id, &source.file_path)?
    {
        remove_record_artifact(artifact_sink, backend, &card.project_key, &card.record_id)?;
    }
    Ok(())
}

fn clear_source_fingerprints(store: &dyn MemoryStore, source: &SessionSource) -> Result<()> {
    let Some(project) = &source.project else {
        return Ok(());
    };
    store.replace_turn_fingerprints(
        &project.project_key,
        source.agent.as_str(),
        &source.session_id,
        &[],
    )
}

fn next_user_block_start(events: &[MessageEvent], suffix_start_turn_index: usize) -> Option<usize> {
    events.iter().position(|event| {
        event.turn_index >= suffix_start_turn_index && event.role == MessageRole::User
    })
}

// When a base session's turn fingerprints change, the byte/line ranges
// recorded in card_continuations rows that point at it may no longer be
// valid. Drop those continuation rows and mark the dependent candidate
// cards unavailable so the next build pass regenerates them from scratch.
fn invalidate_dependent_continuations(
    store: &dyn MemoryStore,
    artifact_sink: &dyn MemoryArtifactSink,
    backend: &str,
    base_agent: &str,
    base_session_id: &str,
    errors: &mut Vec<String>,
) -> Vec<PathBuf> {
    let continuations =
        match store.invalidate_continuations_referencing_base(base_agent, base_session_id) {
            Ok(continuations) => continuations,
            Err(e) => {
                errors.push(format!(
                "invalidate dependent continuations for {base_agent} {base_session_id} failed: {e}"
            ));
                return Vec::new();
            }
        };

    let mut dependent_source_paths = Vec::new();
    let mut seen = HashSet::new();
    for continuation in continuations {
        let path = PathBuf::from(&continuation.candidate_file_path);
        if seen.insert(path.clone()) {
            dependent_source_paths.push(path);
        }
        match store.card_by_id(&continuation.card_id) {
            Ok(Some(card)) if card.available => {
                if let Err(e) = store.mark_card_unavailable(&card.record_id) {
                    errors.push(format!(
                        "mark dependent card {} unavailable failed: {e}",
                        card.record_id
                    ));
                    continue;
                }
                if let Err(e) = remove_record_artifact(
                    artifact_sink,
                    backend,
                    &card.project_key,
                    &card.record_id,
                ) {
                    errors.push(format!(
                        "remove dependent card markdown {} failed: {e}",
                        card.record_id
                    ));
                }
            }
            Ok(_) => {}
            Err(e) => errors.push(format!(
                "load dependent card {} failed: {e}",
                continuation.card_id
            )),
        }
    }
    dependent_source_paths
}
fn extend_unique_paths(existing: &mut Vec<PathBuf>, additions: Vec<PathBuf>) {
    if additions.is_empty() {
        return;
    }
    // Seed dedupe set borrowing from `existing` so we don't clone the
    // already-collected paths. New `additions` still cost one clone each
    // because the set must outlive the borrow of `path`.
    let mut seen: HashSet<PathBuf> = existing.iter().cloned().collect();
    existing.reserve(additions.len());
    for path in additions {
        if seen.insert(path.clone()) {
            existing.push(path);
        }
    }
}

enum DedupePlan {
    Pass,
    Suppress {
        reason: String,
    },
    Trim {
        offset: usize,
        continuation: CardContinuation,
    },
}

fn resolve_dedupe_plan(
    store: &dyn MemoryStore,
    project_key: &str,
    source: &SessionSource,
    events: &[MessageEvent],
    fingerprints: &[TurnFingerprint],
) -> Result<DedupePlan> {
    let Some(dedupe_match) = should_suppress_source(store, source, fingerprints)? else {
        return Ok(DedupePlan::Pass);
    };
    match dedupe_match.action {
        DedupeAction::SuppressWholeSource => Ok(DedupePlan::Suppress {
            reason: suppress_reason(source, &dedupe_match),
        }),
        DedupeAction::TrimPrefix => {
            let Some(trim_at) = next_user_block_start(events, dedupe_match.suffix_start_turn_index)
            else {
                // No user-block boundary after the matched prefix means the
                // candidate has no fresh conversation of its own — whatever
                // sits after the replay is either empty, dangling tool work,
                // or assistant noise without a follow-up question. Treat the
                // whole source as covered by the base instead of writing a
                // card that re-states the prefix.
                return Ok(DedupePlan::Suppress {
                    reason: suppress_reason(source, &dedupe_match),
                });
            };
            let Some(trim_event) = events.get(trim_at) else {
                return Ok(DedupePlan::Suppress {
                    reason: suppress_reason(source, &dedupe_match),
                });
            };
            let continuation = CardContinuation {
                card_id: format!(
                    "sessio-{}-{}",
                    safe_id_part(source.agent.as_str()),
                    safe_id_part(&source.session_id)
                ),
                project_key: project_key.to_string(),
                candidate_agent: source.agent.as_str().to_string(),
                candidate_session_id: source.session_id.clone(),
                candidate_file_path: source.file_path.clone(),
                base_agent: dedupe_match.source_agent.clone(),
                base_session_id: dedupe_match.source_session_id.clone(),
                base_file_path: dedupe_match.source_file_path.clone(),
                base_start_turn_index: dedupe_match.source_first_matched_turn_index,
                base_start_line_start: dedupe_match.source_first_matched_line_start,
                base_start_byte_start: dedupe_match.source_first_matched_byte_start,
                base_end_turn_index: dedupe_match.source_last_matched_turn_index,
                base_end_line_end: dedupe_match.source_last_matched_line_end,
                base_end_byte_end: dedupe_match.source_last_matched_byte_end,
                candidate_trim_turn_start: trim_event.turn_index,
                candidate_trim_line_start: trim_event.location.line_start,
                candidate_trim_byte_start: trim_event.location.byte_start,
                updated_at: trim_event
                    .timestamp
                    .or_else(|| events.iter().filter_map(|event| event.timestamp).max())
                    .unwrap_or(0),
            };
            Ok(DedupePlan::Trim {
                offset: trim_at,
                continuation,
            })
        }
    }
}

fn suppress_reason(source: &SessionSource, dedupe_match: &DedupeMatch) -> String {
    format!(
        "suppress {} {} by {} {} (shared_hashes={}, prefix_coverage={:.2}, total_coverage={:.2})",
        source.agent.as_str(),
        source.session_id,
        dedupe_match.source_agent,
        dedupe_match.source_session_id,
        dedupe_match.shared_hashes,
        dedupe_match.prefix_coverage,
        dedupe_match.total_coverage,
    )
}

fn remove_record_artifact(
    artifact_sink: &dyn MemoryArtifactSink,
    backend: &str,
    project_key: &str,
    record_id: &str,
) -> Result<()> {
    artifact_sink.remove_record_artifact(backend, project_key, record_id)
}

fn remove_legacy_qmd_memory_root(home: &Path) -> Result<()> {
    let path = home.join(".sessio").join("qmd-memory");
    match fs::remove_dir_all(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => {
            Err(err).with_context(|| format!("remove legacy qmd memory root {}", path.display()))
        }
    }
}

fn source_key(source: &SessionSource) -> (String, String, String) {
    source_key_parts(source.agent.as_str(), &source.session_id, &source.file_path)
}

fn source_key_parts(agent: &str, session_id: &str, file_path: &str) -> (String, String, String) {
    (
        agent.to_string(),
        session_id.to_string(),
        file_path.to_string(),
    )
}

fn normalize_path(path: impl AsRef<Path>) -> String {
    fs::canonicalize(path.as_ref())
        .unwrap_or_else(|_| path.as_ref().to_path_buf())
        .to_string_lossy()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::build_source_memory;
    use crate::memory::MemoryStore;
    use crate::models::{Agent, SessionInfo};
    use crate::providers::registry::{AgentProvider, ProviderRegistry};
    use crate::providers::types::{
        AgentKind, MessageContent, MessageEvent, MessageRole, Metadata, PathEvent, ProjectRef,
        SessionRecord, SessionSource, SourceKind, SourceLocation, WatchRoot,
    };
    use crate::store::sqlite::SqliteStore;
    use crate::store::SessionStore;
    use anyhow::Result;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct FakeProvider {
        source: SessionSource,
        events: Mutex<Vec<MessageEvent>>,
    }

    impl FakeProvider {
        fn new(source: SessionSource, events: Vec<MessageEvent>) -> Self {
            Self {
                source,
                events: Mutex::new(events),
            }
        }
    }

    impl AgentProvider for FakeProvider {
        fn agent(&self) -> AgentKind {
            self.source.agent.clone()
        }

        fn display_name(&self) -> &'static str {
            "Fake"
        }

        fn roots(&self) -> Result<Vec<WatchRoot>> {
            Ok(Vec::new())
        }

        fn discover(&self) -> Result<Vec<SessionSource>> {
            Ok(vec![self.source.clone()])
        }

        fn parse_source(&self, source: &SessionSource) -> Result<SessionRecord> {
            Ok(SessionRecord {
                source: source.clone(),
                started_at: None,
                updated_at: None,
                message_count: self.events.lock().unwrap().len(),
                first_user_message: None,
                file_size: 0,
                file_mtime: None,
                partial: false,
                available: true,
                archived: false,
                children: Vec::new(),
                metadata: Metadata::default(),
            })
        }

        fn read_messages(&self, _source: &SessionSource) -> Result<Vec<MessageEvent>> {
            Ok(self.events.lock().unwrap().clone())
        }

        fn classify_path_event(
            &self,
            _event: &PathEvent,
        ) -> Option<crate::providers::types::ProviderTask> {
            None
        }
    }

    #[test]
    fn build_source_memory_marks_card_unavailable_and_removes_markdown_when_source_goes_empty() {
        let root = unique_temp_dir("sessio-memory-build");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "session-1".to_string(),
            scope: "scope-1".to_string(),
            file_path: root.join("session.jsonl").to_string_lossy().to_string(),
            project: Some(ProjectRef {
                project_key: "test-project".to_string(),
                project_path: Some(root.join("project").to_string_lossy().to_string()),
                project_name: Some("project".to_string()),
            }),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };

        let event = MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index: 0,
            role: MessageRole::User,
            content: MessageContent::Text {
                text: "Discuss qmd memory sync".to_string(),
            },
            timestamp: Some(1),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let mut registry = ProviderRegistry::new();
        registry.register(FakeProvider::new(source.clone(), vec![event]));

        let first = build_source_memory(&registry, &store, &artifacts_root, &source).unwrap();
        assert_eq!(first.cards_written, 1);
        assert_eq!(first.cards_marked_unavailable, 0);

        let card_id = "sessio-fake-session-1";
        let card_path = artifacts_root
            .join("test-project")
            .join("sessions")
            .join(format!("{card_id}.md"));
        assert!(card_path.exists());
        assert!(store.card_by_id(card_id).unwrap().unwrap().available);
        let fingerprints_before = store
            .list_turn_fingerprints("test-project", "fake", "session-1")
            .unwrap();
        assert_eq!(fingerprints_before.len(), 1);
        assert_eq!(fingerprints_before[0].turn_index, 0);
        assert_eq!(fingerprints_before[0].role, "user");
        assert!(!fingerprints_before[0].canonical_hash.is_empty());

        let mut empty_registry = ProviderRegistry::new();
        empty_registry.register(FakeProvider::new(source.clone(), Vec::new()));

        let second =
            build_source_memory(&empty_registry, &store, &artifacts_root, &source).unwrap();
        assert_eq!(second.cards_written, 0);
        assert_eq!(second.cards_marked_unavailable, 1);
        assert!(!card_path.exists());
        assert!(!store.card_by_id(card_id).unwrap().unwrap().available);
        let fingerprints_after = store
            .list_turn_fingerprints("test-project", "fake", "session-1")
            .unwrap();
        assert!(fingerprints_after.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_source_memory_replaces_fingerprints_when_turns_change() {
        let root = unique_temp_dir("sessio-memory-fingerprints");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "session-fp".to_string(),
            scope: "scope-fp".to_string(),
            file_path: root.join("session.jsonl").to_string_lossy().to_string(),
            project: Some(ProjectRef {
                project_key: "fp-project".to_string(),
                project_path: Some(root.join("project").to_string_lossy().to_string()),
                project_name: Some("project".to_string()),
            }),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };

        let make_event = |turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let mut registry = ProviderRegistry::new();
        registry.register(FakeProvider::new(
            source.clone(),
            vec![
                make_event(0, MessageRole::User, "first question"),
                make_event(1, MessageRole::Assistant, "first answer"),
                make_event(2, MessageRole::User, "follow-up"),
            ],
        ));

        build_source_memory(&registry, &store, &artifacts_root, &source).unwrap();
        let first = store
            .list_turn_fingerprints("fp-project", "fake", "session-fp")
            .unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].turn_index, 0);
        assert_eq!(first[1].turn_index, 1);
        assert_eq!(first[2].turn_index, 2);
        let initial_hash_for_turn_2 = first[2].canonical_hash.clone();

        let mut shrunk_registry = ProviderRegistry::new();
        shrunk_registry.register(FakeProvider::new(
            source.clone(),
            vec![
                make_event(0, MessageRole::User, "first question"),
                make_event(1, MessageRole::Assistant, "revised answer"),
            ],
        ));
        build_source_memory(&shrunk_registry, &store, &artifacts_root, &source).unwrap();
        let second = store
            .list_turn_fingerprints("fp-project", "fake", "session-fp")
            .unwrap();
        assert_eq!(second.len(), 2, "trailing turn should be removed");
        assert_eq!(second[0].canonical_hash, first[0].canonical_hash);
        assert_ne!(
            second[1].canonical_hash, first[1].canonical_hash,
            "assistant turn text changed, hash must differ"
        );
        assert!(second
            .iter()
            .all(|fp| fp.canonical_hash != initial_hash_for_turn_2));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_source_memory_trims_continuation_replay_prefix() {
        let root = unique_temp_dir("sessio-memory-continuation-trim");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "continuation-project".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let existing_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "001-existing".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("existing.jsonl").to_string_lossy().to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let continuation_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "002-continuation".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("continuation.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };
        let replay = vec![
            (
                MessageRole::User,
                "Explain turn fingerprints in this project",
            ),
            (
                MessageRole::Assistant,
                "They are generated from role and canonical event text",
            ),
            (
                MessageRole::User,
                "So session id should not be part of the hash",
            ),
            (
                MessageRole::Assistant,
                "Correct, otherwise cross-session replay cannot match",
            ),
            (MessageRole::User, "How should continuation dedupe work"),
            (
                MessageRole::Assistant,
                "It should compare ordered event sequences",
            ),
        ];
        let existing_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&existing_source, idx, *role, text))
            .collect::<Vec<_>>();
        let mut existing_registry = ProviderRegistry::new();
        existing_registry.register(FakeProvider::new(existing_source.clone(), existing_events));
        build_source_memory(
            &existing_registry,
            &store,
            &artifacts_root,
            &existing_source,
        )
        .unwrap();

        let mut continuation_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&continuation_source, idx, *role, text))
            .collect::<Vec<_>>();
        continuation_events.push(make_event(
            &continuation_source,
            6,
            MessageRole::User,
            "Please implement prefix trim now",
        ));
        continuation_events.push(make_event(
            &continuation_source,
            7,
            MessageRole::Assistant,
            "I will generate the continuation card from suffix events only",
        ));
        let mut continuation_registry = ProviderRegistry::new();
        continuation_registry.register(FakeProvider::new(
            continuation_source.clone(),
            continuation_events,
        ));
        let result = build_source_memory(
            &continuation_registry,
            &store,
            &artifacts_root,
            &continuation_source,
        )
        .unwrap();
        assert_eq!(result.cards_written, 1);

        let card_path = artifacts_root
            .join("continuation-project")
            .join("sessions")
            .join("sessio-fake-002-continuation.md");
        let body = fs::read_to_string(card_path).unwrap();
        assert!(!body.contains("Explain turn fingerprints in this project"));
        assert!(!body.contains("They are generated from role and canonical event text"));
        assert!(body.contains("Please implement prefix trim now"));
        assert!(body.contains("I will generate the continuation card from suffix events only"));
        assert!(!body.contains("shared prefix covered by:"));

        let continuation = store
            .continuation_for_card("sessio-fake-002-continuation")
            .unwrap()
            .unwrap();
        assert_eq!(continuation.base_session_id, existing_source.session_id);
        assert_eq!(continuation.base_start_turn_index, 0);
        assert_eq!(continuation.base_end_turn_index, 5);
        assert_eq!(continuation.candidate_trim_turn_start, 6);

        let fingerprints = store
            .list_turn_fingerprints("continuation-project", "fake", "002-continuation")
            .unwrap();
        assert_eq!(
            fingerprints.len(),
            8,
            "fingerprints remain full-source for future overlap detection"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_source_memory_trims_full_confirmed_prefix_beyond_twelve_events() {
        let root = unique_temp_dir("sessio-memory-long-prefix-trim");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "continuation-project-long".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let existing_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "001-existing-long".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("existing-long.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let continuation_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "002-continuation-long".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("continuation-long.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: String| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text { text },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let replay = (0..14)
            .map(|idx| {
                if idx % 2 == 0 {
                    (
                        MessageRole::User,
                        format!("shared question {}", idx / 2 + 1),
                    )
                } else {
                    (
                        MessageRole::Assistant,
                        format!("shared answer {}", idx / 2 + 1),
                    )
                }
            })
            .collect::<Vec<_>>();
        let existing_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&existing_source, idx, *role, text.clone()))
            .collect::<Vec<_>>();
        let mut existing_registry = ProviderRegistry::new();
        existing_registry.register(FakeProvider::new(existing_source.clone(), existing_events));
        build_source_memory(
            &existing_registry,
            &store,
            &artifacts_root,
            &existing_source,
        )
        .unwrap();

        let mut continuation_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&continuation_source, idx, *role, text.clone()))
            .collect::<Vec<_>>();
        continuation_events.push(make_event(
            &continuation_source,
            14,
            MessageRole::User,
            "new branch request".to_string(),
        ));
        continuation_events.push(make_event(
            &continuation_source,
            15,
            MessageRole::Assistant,
            "new branch answer".to_string(),
        ));

        let mut continuation_registry = ProviderRegistry::new();
        continuation_registry.register(FakeProvider::new(
            continuation_source.clone(),
            continuation_events,
        ));
        let result = build_source_memory(
            &continuation_registry,
            &store,
            &artifacts_root,
            &continuation_source,
        )
        .unwrap();
        assert_eq!(result.cards_written, 1);

        let card_path = artifacts_root
            .join("continuation-project-long")
            .join("sessions")
            .join("sessio-fake-002-continuation-long.md");
        let body = fs::read_to_string(card_path).unwrap();
        assert!(!body.contains("shared question 1"));
        assert!(!body.contains("shared answer 7"));
        assert!(body.contains("new branch request"));
        assert!(body.contains("new branch answer"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_source_memory_does_not_trim_earlier_sibling_back_from_later_one() {
        let root = unique_temp_dir("sessio-memory-sibling-direction");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "continuation-project-sibling".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let earlier_source = SessionSource {
            agent: AgentKind::new("claude"),
            session_id: "07-earlier".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("earlier.jsonl").to_string_lossy().to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let later_source = SessionSource {
            agent: AgentKind::new("claude"),
            session_id: "21-later".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("later.jsonl").to_string_lossy().to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: String| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text { text },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let replay = vec![
            (MessageRole::User, "shared opening request".to_string()),
            (MessageRole::Assistant, "shared opening answer".to_string()),
            (MessageRole::User, "shared follow-up".to_string()),
            (
                MessageRole::Assistant,
                "shared follow-up answer".to_string(),
            ),
        ];

        let mut later_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&later_source, idx, *role, text.clone()))
            .collect::<Vec<_>>();
        later_events.push(make_event(
            &later_source,
            4,
            MessageRole::User,
            "later unique request".to_string(),
        ));
        later_events.push(make_event(
            &later_source,
            5,
            MessageRole::Assistant,
            "later unique answer".to_string(),
        ));
        let mut later_registry = ProviderRegistry::new();
        later_registry.register(FakeProvider::new(later_source.clone(), later_events));
        build_source_memory(&later_registry, &store, &artifacts_root, &later_source).unwrap();

        let mut earlier_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&earlier_source, idx, *role, text.clone()))
            .collect::<Vec<_>>();
        earlier_events.push(make_event(
            &earlier_source,
            4,
            MessageRole::User,
            "earlier unique request".to_string(),
        ));
        earlier_events.push(make_event(
            &earlier_source,
            5,
            MessageRole::Assistant,
            "earlier unique answer".to_string(),
        ));
        let mut earlier_registry = ProviderRegistry::new();
        earlier_registry.register(FakeProvider::new(earlier_source.clone(), earlier_events));
        build_source_memory(&earlier_registry, &store, &artifacts_root, &earlier_source).unwrap();

        let earlier_card_path = artifacts_root
            .join("continuation-project-sibling")
            .join("sessions")
            .join("sessio-claude-07-earlier.md");
        let earlier_body = fs::read_to_string(earlier_card_path).unwrap();
        assert!(earlier_body.contains("shared opening request"));
        assert!(earlier_body.contains("earlier unique request"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_source_memory_trim_prefix_starts_at_next_user_block() {
        let root = unique_temp_dir("sessio-memory-user-block-trim");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "continuation-project-user-block".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let existing_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "001-existing-user-block".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("existing-user-block.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let continuation_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "002-continuation-user-block".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("continuation-user-block.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let existing_events = vec![
            make_event(
                &existing_source,
                0,
                MessageRole::User,
                "shared opening request",
            ),
            make_event(&existing_source, 1, MessageRole::Assistant, "shared answer"),
            make_event(
                &existing_source,
                2,
                MessageRole::User,
                "shared next request",
            ),
            make_event(
                &existing_source,
                3,
                MessageRole::Assistant,
                "shared next answer",
            ),
        ];
        let mut existing_registry = ProviderRegistry::new();
        existing_registry.register(FakeProvider::new(existing_source.clone(), existing_events));
        build_source_memory(
            &existing_registry,
            &store,
            &artifacts_root,
            &existing_source,
        )
        .unwrap();

        let continuation_events = vec![
            make_event(
                &continuation_source,
                0,
                MessageRole::User,
                "shared opening request",
            ),
            make_event(
                &continuation_source,
                1,
                MessageRole::Assistant,
                "shared answer",
            ),
            make_event(
                &continuation_source,
                2,
                MessageRole::User,
                "shared next request",
            ),
            make_event(
                &continuation_source,
                3,
                MessageRole::Assistant,
                "shared next answer",
            ),
            make_event(
                &continuation_source,
                4,
                MessageRole::ToolUse,
                "continuation tool use",
            ),
            make_event(
                &continuation_source,
                5,
                MessageRole::ToolResult,
                "continuation tool result",
            ),
            make_event(
                &continuation_source,
                6,
                MessageRole::User,
                "new user request after tool work",
            ),
            make_event(
                &continuation_source,
                7,
                MessageRole::Assistant,
                "new answer after user request",
            ),
        ];
        let mut continuation_registry = ProviderRegistry::new();
        continuation_registry.register(FakeProvider::new(
            continuation_source.clone(),
            continuation_events,
        ));
        let result = build_source_memory(
            &continuation_registry,
            &store,
            &artifacts_root,
            &continuation_source,
        )
        .unwrap();
        assert_eq!(result.cards_written, 1);

        let card_path = artifacts_root
            .join("continuation-project-user-block")
            .join("sessions")
            .join("sessio-fake-002-continuation-user-block.md");
        let body = fs::read_to_string(card_path).unwrap();
        assert!(!body.contains("continuation tool use"));
        assert!(!body.contains("continuation tool result"));
        assert!(body.contains("new user request after tool work"));
        assert!(body.contains("new answer after user request"));

        let _ = fs::remove_dir_all(&root);
    }

    // Regression: when dedupe matches a continuation prefix and the
    // candidate has no user-block boundary after that prefix (i.e. no
    // fresh conversation of its own), the source should be suppressed —
    // a forked session that ends with replayed content plus dangling
    // assistant/tool noise carries no independent value.
    #[test]
    fn build_source_memory_trim_without_user_anchor_suppresses_source() {
        let root = unique_temp_dir("sessio-memory-trim-no-anchor");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "no-anchor-project".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let existing_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "001-existing-no-anchor".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("existing-no-anchor.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let continuation_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "002-continuation-no-anchor".to_string(),
            scope: "scope".to_string(),
            file_path: root
                .join("continuation-no-anchor.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        // Existing session: 4 turns. Continuation session: same 4 turns
        // + a trailing assistant turn (no further user block). All "new"
        // content is just an assistant tail with no follow-up question.
        let replay = vec![
            (MessageRole::User, "shared opening request"),
            (MessageRole::Assistant, "shared opening answer"),
            (MessageRole::User, "shared follow-up"),
            (MessageRole::Assistant, "shared follow-up answer"),
        ];
        let existing_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&existing_source, idx, *role, text))
            .collect::<Vec<_>>();
        let mut existing_registry = ProviderRegistry::new();
        existing_registry.register(FakeProvider::new(existing_source.clone(), existing_events));
        build_source_memory(
            &existing_registry,
            &store,
            &artifacts_root,
            &existing_source,
        )
        .unwrap();

        let mut continuation_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&continuation_source, idx, *role, text))
            .collect::<Vec<_>>();
        continuation_events.push(make_event(
            &continuation_source,
            4,
            MessageRole::Assistant,
            "tail-only assistant message with no following user turn",
        ));

        let mut continuation_registry = ProviderRegistry::new();
        continuation_registry.register(FakeProvider::new(
            continuation_source.clone(),
            continuation_events,
        ));
        let result = build_source_memory(
            &continuation_registry,
            &store,
            &artifacts_root,
            &continuation_source,
        )
        .unwrap();
        assert_eq!(
            result.cards_written, 0,
            "continuation without a fresh user block should not produce a card"
        );

        let candidate_card_id = "sessio-fake-002-continuation-no-anchor";
        let card_path = artifacts_root
            .join("no-anchor-project")
            .join("sessions")
            .join(format!("{candidate_card_id}.md"));
        assert!(
            !card_path.exists(),
            "no card markdown should remain for a fully-covered continuation"
        );
        let continuation = store.continuation_for_card(candidate_card_id).unwrap();
        assert!(
            continuation.is_none(),
            "suppressed source should not record continuation provenance"
        );
        let card = store.card_by_id(candidate_card_id).unwrap();
        assert!(
            card.is_none_or(|c| !c.available),
            "any pre-existing candidate card must be marked unavailable"
        );

        let _ = fs::remove_dir_all(&root);
    }

    // Regression: codex sessions without a forked_from_id should not
    // be treated as totally un-deduplicatable. They fall back to the
    // same started_at/updated_at-driven direction the other agents use,
    // so an older codex session can still serve as a base for a newer
    // one when timestamps make the ordering unambiguous.
    #[test]
    fn build_source_memory_dedupes_codex_without_forked_from_id_by_started_at() {
        let root = unique_temp_dir("sessio-memory-codex-no-fork");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "codex-no-fork-project".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        // UUIDs deliberately chosen so the lex fallback would prefer the
        // wrong direction (earlier < later in real time but later < earlier
        // by string ordering). The test passes only if dedupe consults
        // started_at rather than the UUID ordering.
        let earlier_session_id = "f0000000-0000-0000-0000-000000000001".to_string();
        let later_session_id = "00000000-0000-0000-0000-000000000002".to_string();

        let mut earlier_metadata = Metadata::default();
        earlier_metadata.insert("started_at".to_string(), serde_json::json!(1_000_i64));
        earlier_metadata.insert("updated_at".to_string(), serde_json::json!(1_500_i64));
        let earlier_source = SessionSource {
            agent: AgentKind::new("codex"),
            session_id: earlier_session_id.clone(),
            scope: "scope".to_string(),
            file_path: root
                .join("codex-earlier.jsonl")
                .to_string_lossy()
                .to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: earlier_metadata,
        };
        let mut later_metadata = Metadata::default();
        later_metadata.insert("started_at".to_string(), serde_json::json!(2_000_i64));
        later_metadata.insert("updated_at".to_string(), serde_json::json!(2_500_i64));
        let later_source = SessionSource {
            agent: AgentKind::new("codex"),
            session_id: later_session_id.clone(),
            scope: "scope".to_string(),
            file_path: root.join("codex-later.jsonl").to_string_lossy().to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: later_metadata,
        };

        // session_time_info reads from the sessions table, so register
        // the candidate (earlier) session there with its started_at.
        store
            .upsert_session(
                &earlier_source.scope,
                &SessionInfo {
                    id: earlier_session_id.clone(),
                    agent: Agent::Codex,
                    forked_from_id: None,
                    project_path: earlier_source
                        .project
                        .as_ref()
                        .and_then(|p| p.project_path.clone()),
                    project_name: Some("project".to_string()),
                    started_at: Some(1_000),
                    updated_at: Some(1_500),
                    message_count: 0,
                    first_user_message: None,
                    file_path: earlier_source.file_path.clone(),
                    file_size: 0,
                    partial: false,
                    available: true,
                    archived: false,
                    subagents: Vec::new(),
                },
            )
            .unwrap();

        let make_event = |source: &SessionSource, turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let replay = vec![
            (MessageRole::User, "shared codex opening request"),
            (MessageRole::Assistant, "shared codex opening answer"),
            (MessageRole::User, "shared codex follow-up"),
            (MessageRole::Assistant, "shared codex follow-up answer"),
        ];
        let earlier_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&earlier_source, idx, *role, text))
            .collect::<Vec<_>>();
        let mut earlier_registry = ProviderRegistry::new();
        earlier_registry.register(FakeProvider::new(earlier_source.clone(), earlier_events));
        build_source_memory(&earlier_registry, &store, &artifacts_root, &earlier_source).unwrap();

        let mut later_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&later_source, idx, *role, text))
            .collect::<Vec<_>>();
        later_events.push(make_event(
            &later_source,
            4,
            MessageRole::User,
            "later unique request after the shared prefix",
        ));
        later_events.push(make_event(
            &later_source,
            5,
            MessageRole::Assistant,
            "later unique answer that extends the conversation further",
        ));
        let mut later_registry = ProviderRegistry::new();
        later_registry.register(FakeProvider::new(later_source.clone(), later_events));
        build_source_memory(&later_registry, &store, &artifacts_root, &later_source).unwrap();

        let later_card_id = format!("sessio-codex-{}", later_session_id);
        let later_card_path = artifacts_root
            .join("codex-no-fork-project")
            .join("sessions")
            .join(format!("{later_card_id}.md"));
        let later_body = fs::read_to_string(later_card_path).unwrap();
        assert!(
            !later_body.contains("shared codex opening request"),
            "later codex card should have the shared prefix trimmed"
        );
        assert!(later_body.contains("later unique request after the shared prefix"));
        let continuation = store.continuation_for_card(&later_card_id).unwrap();
        let continuation = continuation.expect(
            "later card must record continuation provenance pointing at the earlier session",
        );
        assert_eq!(continuation.base_session_id, earlier_session_id);

        let _ = fs::remove_dir_all(&root);
    }

    // Regression: when a base session is reindexed and its turn
    // fingerprints change, dependent card_continuations rows must be
    // dropped and dependent cards marked unavailable so they get rebuilt.
    #[test]
    fn build_source_memory_invalidates_continuations_when_base_changes() {
        let root = unique_temp_dir("sessio-memory-base-reindex");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "base-reindex-project".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let base_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "001-base".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("base.jsonl").to_string_lossy().to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let candidate_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "002-candidate".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("candidate.jsonl").to_string_lossy().to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let replay = vec![
            (MessageRole::User, "shared opening request for base reindex"),
            (
                MessageRole::Assistant,
                "shared opening answer for base reindex",
            ),
            (MessageRole::User, "shared follow-up for base reindex"),
            (
                MessageRole::Assistant,
                "shared follow-up answer for base reindex",
            ),
            (MessageRole::User, "shared third request for base reindex"),
            (
                MessageRole::Assistant,
                "shared third answer for base reindex",
            ),
        ];
        let base_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&base_source, idx, *role, text))
            .collect::<Vec<_>>();
        let mut base_registry = ProviderRegistry::new();
        base_registry.register(FakeProvider::new(base_source.clone(), base_events.clone()));
        build_source_memory(&base_registry, &store, &artifacts_root, &base_source).unwrap();

        let mut candidate_events = replay
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&candidate_source, idx, *role, text))
            .collect::<Vec<_>>();
        candidate_events.push(make_event(
            &candidate_source,
            6,
            MessageRole::User,
            "candidate unique request",
        ));
        candidate_events.push(make_event(
            &candidate_source,
            7,
            MessageRole::Assistant,
            "candidate unique answer is long enough",
        ));
        let mut candidate_registry = ProviderRegistry::new();
        candidate_registry.register(FakeProvider::new(
            candidate_source.clone(),
            candidate_events,
        ));
        build_source_memory(
            &candidate_registry,
            &store,
            &artifacts_root,
            &candidate_source,
        )
        .unwrap();

        let candidate_card_id = "sessio-fake-002-candidate";
        let continuation_before = store.continuation_for_card(candidate_card_id).unwrap();
        assert!(
            continuation_before.is_some(),
            "candidate must record continuation initially"
        );

        // Reindex the base with extended content; this rewrites
        // base fingerprints and must invalidate the candidate's
        // continuation row + mark its card unavailable.
        let mut extended_base_events = base_events;
        extended_base_events.push(make_event(
            &base_source,
            6,
            MessageRole::User,
            "base extension request",
        ));
        extended_base_events.push(make_event(
            &base_source,
            7,
            MessageRole::Assistant,
            "base extension answer",
        ));
        let mut extended_base_registry = ProviderRegistry::new();
        extended_base_registry
            .register(FakeProvider::new(base_source.clone(), extended_base_events));
        build_source_memory(
            &extended_base_registry,
            &store,
            &artifacts_root,
            &base_source,
        )
        .unwrap();

        let continuation_after = store.continuation_for_card(candidate_card_id).unwrap();
        assert!(
            continuation_after.is_none(),
            "candidate continuation row must be invalidated when base fingerprints change"
        );
        let card = store.card_by_id(candidate_card_id).unwrap().unwrap();
        assert!(
            !card.available,
            "candidate card must be marked unavailable after its base was reindexed"
        );

        let _ = fs::remove_dir_all(&root);
    }

    // Regression: when continuation chains exist (A is base of B, B is base
    // of C), reindexing A must surface B as a dependent so the indexer can
    // requeue it; building B in turn must surface C. The test exercises one
    // link at a time — together they prove the chain converges across
    // successive build passes.
    #[test]
    fn build_source_memory_propagates_dependents_along_chain() {
        let root = unique_temp_dir("sessio-memory-chain");
        let db_path = root.join("memory.db");
        let artifacts_root = root.join("artifacts-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "chain-project".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let a_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "001-a".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("a.jsonl").to_string_lossy().to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let b_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "002-b".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("b.jsonl").to_string_lossy().to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let c_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "003-c".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("c.jsonl").to_string_lossy().to_string(),
            project: Some(project),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let make_event = |source: &SessionSource, turn_index, role, text: &str| MessageEvent {
            source: source.clone(),
            event_id: None,
            turn_index,
            role,
            content: MessageContent::Text {
                text: text.to_string(),
            },
            timestamp: Some(turn_index as i64),
            location: SourceLocation::file(source.file_path.clone()),
            metadata: Metadata::default(),
        };

        let a_turns = vec![
            (MessageRole::User, "chain shared opening request"),
            (MessageRole::Assistant, "chain shared opening answer"),
            (MessageRole::User, "chain shared follow-up"),
            (MessageRole::Assistant, "chain shared follow-up answer"),
            (MessageRole::User, "chain third shared request"),
            (MessageRole::Assistant, "chain third shared answer"),
        ];
        let a_events = a_turns
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&a_source, idx, *role, text))
            .collect::<Vec<_>>();
        let mut a_registry = ProviderRegistry::new();
        a_registry.register(FakeProvider::new(a_source.clone(), a_events.clone()));
        build_source_memory(&a_registry, &store, &artifacts_root, &a_source).unwrap();

        let mut b_events = a_turns
            .iter()
            .enumerate()
            .map(|(idx, (role, text))| make_event(&b_source, idx, *role, text))
            .collect::<Vec<_>>();
        b_events.push(make_event(
            &b_source,
            6,
            MessageRole::User,
            "chain b unique request after shared prefix",
        ));
        b_events.push(make_event(
            &b_source,
            7,
            MessageRole::Assistant,
            "chain b unique answer that is long enough",
        ));
        b_events.push(make_event(
            &b_source,
            8,
            MessageRole::User,
            "chain b second unique request after shared prefix",
        ));
        b_events.push(make_event(
            &b_source,
            9,
            MessageRole::Assistant,
            "chain b second unique answer that is long enough",
        ));
        let mut b_registry = ProviderRegistry::new();
        b_registry.register(FakeProvider::new(b_source.clone(), b_events.clone()));
        build_source_memory(&b_registry, &store, &artifacts_root, &b_source).unwrap();

        let mut c_events = b_events.clone();
        for event in c_events.iter_mut() {
            event.source = c_source.clone();
            event.location = SourceLocation::file(c_source.file_path.clone());
        }
        c_events.push(make_event(
            &c_source,
            10,
            MessageRole::User,
            "chain c unique tail request",
        ));
        c_events.push(make_event(
            &c_source,
            11,
            MessageRole::Assistant,
            "chain c unique tail answer that is long enough",
        ));
        let mut c_registry = ProviderRegistry::new();
        c_registry.register(FakeProvider::new(c_source.clone(), c_events));
        build_source_memory(&c_registry, &store, &artifacts_root, &c_source).unwrap();

        let b_card_id = "sessio-fake-002-b";
        let c_card_id = "sessio-fake-003-c";
        let b_continuation_before = store
            .continuation_for_card(b_card_id)
            .unwrap()
            .expect("B must record continuation pointing at A");
        assert_eq!(b_continuation_before.base_session_id, a_source.session_id);
        let c_continuation_before = store
            .continuation_for_card(c_card_id)
            .unwrap()
            .expect("C must record continuation pointing at B");
        assert_eq!(c_continuation_before.base_session_id, b_source.session_id);

        // Reindex A with extended content. B is the only direct dependent,
        // so we should get B's path back. C is the dependent of B and
        // remains untouched until B itself rebuilds.
        let mut extended_a_events = a_events;
        extended_a_events.push(make_event(
            &a_source,
            6,
            MessageRole::User,
            "chain a extension request",
        ));
        extended_a_events.push(make_event(
            &a_source,
            7,
            MessageRole::Assistant,
            "chain a extension answer",
        ));
        let mut extended_a_registry = ProviderRegistry::new();
        extended_a_registry.register(FakeProvider::new(a_source.clone(), extended_a_events));
        let a_result =
            build_source_memory(&extended_a_registry, &store, &artifacts_root, &a_source).unwrap();
        let b_path = PathBuf::from(&b_source.file_path);
        assert!(
            a_result.dependent_source_paths.contains(&b_path),
            "rebuilding A must surface B as a dependent (got {:?})",
            a_result.dependent_source_paths
        );
        assert!(
            !a_result
                .dependent_source_paths
                .contains(&PathBuf::from(&c_source.file_path)),
            "C is not a direct dependent of A; it should not appear in A's first-hop result"
        );

        let b_continuation_after_a = store.continuation_for_card(b_card_id).unwrap();
        assert!(
            b_continuation_after_a.is_none(),
            "B's continuation row must be invalidated when its base A changes"
        );

        // Simulate the indexer requeueing B. Building B must now surface C.
        let mut b_registry_again = ProviderRegistry::new();
        b_registry_again.register(FakeProvider::new(b_source.clone(), b_events));
        let b_result =
            build_source_memory(&b_registry_again, &store, &artifacts_root, &b_source).unwrap();
        let c_path = PathBuf::from(&c_source.file_path);
        assert!(
            b_result.dependent_source_paths.contains(&c_path),
            "rebuilding B must surface C as a dependent (got {:?})",
            b_result.dependent_source_paths
        );
        let c_continuation_after_b = store.continuation_for_card(c_card_id).unwrap();
        assert!(
            c_continuation_after_b.is_none(),
            "C's continuation row must be invalidated once B has been rebuilt"
        );

        let _ = fs::remove_dir_all(&root);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
