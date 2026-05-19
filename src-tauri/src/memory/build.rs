use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::memory::cards::{cards_for_source, fingerprints_for_source};
use crate::memory::dedupe::{should_suppress_source, DedupeAction};
use crate::memory::normalize::normalize_events;
use crate::memory::{MemoryCard, MemoryStore};
use crate::providers::registry::ProviderRegistry;
use crate::providers::types::SessionSource;

#[derive(Debug, Clone)]
pub struct MemoryBuildOptions {
    pub project_path: PathBuf,
    pub output_root: PathBuf,
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
    pub output_root: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryBuildSourceResult {
    pub project_key: Option<String>,
    pub cards_written: usize,
    pub cards_marked_unavailable: usize,
}

pub fn build_project_memory(
    registry: &ProviderRegistry,
    store: &dyn MemoryStore,
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
        output_root: options.output_root.to_string_lossy().to_string(),
        errors: Vec::new(),
    };
    let mut seen_sources = HashSet::new();

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
                        remove_existing_source_card_files(store, &options.output_root, &source)
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
                    remove_existing_source_card_files(store, &options.output_root, &source)
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
            let mut card_events = events.as_slice();
            if let Some(dedupe_match) = should_suppress_source(store, &source, &fingerprints)? {
                match dedupe_match.action {
                    DedupeAction::SuppressWholeSource => {
                        summary.sources_skipped += 1;
                        summary.errors.push(format!(
                            "suppress {} {} by {} {} (shared_hashes={}, prefix_coverage={:.2}, total_coverage={:.2})",
                            source.agent.as_str(),
                            source.session_id,
                            dedupe_match.source_agent,
                            dedupe_match.source_session_id,
                            dedupe_match.shared_hashes,
                            dedupe_match.prefix_coverage,
                            dedupe_match.total_coverage,
                        ));
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
                            remove_existing_source_card_files(store, &options.output_root, &source)
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
                    DedupeAction::TrimPrefix => {
                        let trim_at = events
                            .iter()
                            .position(|event| {
                                event.turn_index >= dedupe_match.suffix_start_turn_index
                            })
                            .unwrap_or(events.len());
                        card_events = &events[trim_at..];
                        if card_events.is_empty() {
                            summary.sources_skipped += 1;
                            if let Err(mark_error) = store.mark_source_cards_unavailable(
                                source.agent.as_str(),
                                &source.session_id,
                                &source.file_path,
                            ) {
                                summary.errors.push(format!(
                                    "mark empty continuation tail unavailable {} {} failed: {mark_error}",
                                    source.agent.as_str(),
                                    source.file_path
                                ));
                            }
                            if let Err(remove_error) = remove_existing_source_card_files(
                                store,
                                &options.output_root,
                                &source,
                            ) {
                                summary.errors.push(format!(
                                    "remove empty continuation tail files {} {} failed: {remove_error}",
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
                    }
                }
            }

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
                    remove_existing_source_card_files(store, &options.output_root, &source)
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
            for (card, sources) in generated {
                write_card_markdown(
                    &options.output_root,
                    &card.project_key,
                    &card.card_id,
                    &card.body,
                )?;
                store.upsert_card(&card)?;
                store.replace_card_sources(&card.card_id, &sources)?;
                summary.cards_written += 1;
            }
        }
    }

    if let Some(project_key) = summary.project_key.as_deref() {
        for card in store.list_project_cards(project_key)? {
            if !card.available {
                continue;
            }
            let sources = store.sources_for_card(&card.card_id)?;
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
                store.mark_card_unavailable(&card.card_id)?;
                remove_card_markdown(&options.output_root, &card)?;
            }
        }
    }

    Ok(summary)
}

pub fn build_source_memory(
    registry: &ProviderRegistry,
    store: &dyn MemoryStore,
    output_root: &Path,
    source: &SessionSource,
) -> Result<MemoryBuildSourceResult> {
    let Some(project) = &source.project else {
        return Ok(MemoryBuildSourceResult {
            project_key: None,
            cards_written: 0,
            cards_marked_unavailable: 0,
        });
    };
    let Some(provider) = registry.provider_for_agent(&source.agent) else {
        anyhow::bail!("no provider for agent {}", source.agent.as_str());
    };

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
                store.mark_card_unavailable(&card.card_id)?;
                remove_card_markdown(output_root, &card)?;
                marked_unavailable += 1;
            }
        }
        clear_source_fingerprints(store, source)?;
        return Ok(MemoryBuildSourceResult {
            project_key: Some(project.project_key.clone()),
            cards_written: 0,
            cards_marked_unavailable: marked_unavailable,
        });
    }

    let fingerprints = fingerprints_for_source(source, &events);
    let mut card_events = events.as_slice();
    if let Some(dedupe_match) = should_suppress_source(store, source, &fingerprints)? {
        match dedupe_match.action {
            DedupeAction::SuppressWholeSource => {
                for card in existing {
                    if card.available {
                        store.mark_card_unavailable(&card.card_id)?;
                        remove_card_markdown(output_root, &card)?;
                        marked_unavailable += 1;
                    }
                }
                clear_source_fingerprints(store, source)?;
                return Ok(MemoryBuildSourceResult {
                    project_key: Some(project.project_key.clone()),
                    cards_written: 0,
                    cards_marked_unavailable: marked_unavailable,
                });
            }
            DedupeAction::TrimPrefix => {
                let trim_at = events
                    .iter()
                    .position(|event| event.turn_index >= dedupe_match.suffix_start_turn_index)
                    .unwrap_or(events.len());
                card_events = &events[trim_at..];
                if card_events.is_empty() {
                    for card in existing {
                        if card.available {
                            store.mark_card_unavailable(&card.card_id)?;
                            remove_card_markdown(output_root, &card)?;
                            marked_unavailable += 1;
                        }
                    }
                    clear_source_fingerprints(store, source)?;
                    return Ok(MemoryBuildSourceResult {
                        project_key: Some(project.project_key.clone()),
                        cards_written: 0,
                        cards_marked_unavailable: marked_unavailable,
                    });
                }
            }
        }
    }

    let generated = cards_for_source(source, card_events);
    if generated.is_empty() {
        for card in existing {
            if card.available {
                store.mark_card_unavailable(&card.card_id)?;
                remove_card_markdown(output_root, &card)?;
                marked_unavailable += 1;
            }
        }
        clear_source_fingerprints(store, source)?;
        return Ok(MemoryBuildSourceResult {
            project_key: Some(project.project_key.clone()),
            cards_written: 0,
            cards_marked_unavailable: marked_unavailable,
        });
    }

    let generated_ids = generated
        .iter()
        .map(|(card, _)| card.card_id.clone())
        .collect::<std::collections::HashSet<_>>();
    for card in existing {
        if card.available && !generated_ids.contains(&card.card_id) {
            store.mark_card_unavailable(&card.card_id)?;
            remove_card_markdown(output_root, &card)?;
            marked_unavailable += 1;
        }
    }

    store.replace_turn_fingerprints(
        &project.project_key,
        source.agent.as_str(),
        &source.session_id,
        &fingerprints,
    )?;

    let mut cards_written = 0;
    for (card, sources) in generated {
        write_card_markdown(output_root, &card.project_key, &card.card_id, &card.body)?;
        store.upsert_card(&card)?;
        store.replace_card_sources(&card.card_id, &sources)?;
        cards_written += 1;
    }

    Ok(MemoryBuildSourceResult {
        project_key: Some(project.project_key.clone()),
        cards_written,
        cards_marked_unavailable: marked_unavailable,
    })
}

pub fn default_output_root() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".sessio").join("qmd-memory").join("projects"))
}

fn write_card_markdown(
    output_root: &Path,
    project_key: &str,
    card_id: &str,
    body: &str,
) -> Result<PathBuf> {
    let dir = output_root.join(project_key).join("cards");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{card_id}.md"));
    fs::write(&path, body)?;
    Ok(path)
}

fn remove_existing_source_card_files(
    store: &dyn MemoryStore,
    output_root: &Path,
    source: &SessionSource,
) -> Result<()> {
    for card in
        store.list_cards_for_source(source.agent.as_str(), &source.session_id, &source.file_path)?
    {
        remove_card_markdown(output_root, &card)?;
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

fn remove_card_markdown(output_root: &Path, card: &MemoryCard) -> Result<()> {
    let path = output_root.join(&card.qmd_path);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("remove card markdown {}", path.display())),
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
        let cards_root = root.join("cards-root");
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

        let first = build_source_memory(&registry, &store, &cards_root, &source).unwrap();
        assert_eq!(first.cards_written, 1);
        assert_eq!(first.cards_marked_unavailable, 0);

        let card_id = "sessio-fake-session-1";
        let card_path = cards_root
            .join("test-project")
            .join("cards")
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

        let second = build_source_memory(&empty_registry, &store, &cards_root, &source).unwrap();
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
        let cards_root = root.join("cards-root");
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

        build_source_memory(&registry, &store, &cards_root, &source).unwrap();
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
        build_source_memory(&shrunk_registry, &store, &cards_root, &source).unwrap();
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
        let cards_root = root.join("cards-root");
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();

        let project = ProjectRef {
            project_key: "continuation-project".to_string(),
            project_path: Some(root.join("project").to_string_lossy().to_string()),
            project_name: Some("project".to_string()),
        };
        let existing_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "existing".to_string(),
            scope: "scope".to_string(),
            file_path: root.join("existing.jsonl").to_string_lossy().to_string(),
            project: Some(project.clone()),
            source_kind: SourceKind::MainSession,
            metadata: Metadata::default(),
        };
        let continuation_source = SessionSource {
            agent: AgentKind::new("fake"),
            session_id: "continuation".to_string(),
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
        build_source_memory(&existing_registry, &store, &cards_root, &existing_source).unwrap();

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
            &cards_root,
            &continuation_source,
        )
        .unwrap();
        assert_eq!(result.cards_written, 1);

        let card_path = cards_root
            .join("continuation-project")
            .join("cards")
            .join("sessio-fake-continuation.md");
        let body = fs::read_to_string(card_path).unwrap();
        assert!(!body.contains("Explain turn fingerprints in this project"));
        assert!(!body.contains("They are generated from role and canonical event text"));
        assert!(body.contains("Please implement prefix trim now"));
        assert!(body.contains("I will generate the continuation card from suffix events only"));

        let fingerprints = store
            .list_turn_fingerprints("continuation-project", "fake", "continuation")
            .unwrap();
        assert_eq!(
            fingerprints.len(),
            8,
            "fingerprints remain full-source for future overlap detection"
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
