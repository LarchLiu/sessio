use anyhow::{Context, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::agents::sources::registry::AgentSourceRegistry;
use crate::agents::sources::types::{PathEvent, PathEventKind, SourceIndexTask, SourceKind};
use crate::indexer::{IndexTask, IndexerHandle};
use crate::store::SessionStore;

type DebouncerHandle = Debouncer<notify::RecommendedWatcher, FileIdMap>;

pub struct WatcherHandle {
    state: Arc<Mutex<WatcherState>>,
    store: Arc<dyn SessionStore>,
}

struct WatcherState {
    debouncer: DebouncerHandle,
    registry: AgentSourceRegistry,
    watched_roots: HashMap<PathBuf, RecursiveMode>,
}

pub fn spawn(store: Arc<dyn SessionStore>, indexer: IndexerHandle) -> Result<WatcherHandle> {
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let debouncer =
        new_debouncer(Duration::from_millis(500), None, tx).context("create file debouncer")?;

    let registry = AgentSourceRegistry::new();
    let state = Arc::new(Mutex::new(WatcherState {
        debouncer,
        registry,
        watched_roots: HashMap::new(),
    }));
    let handle = WatcherHandle {
        state: state.clone(),
        store: store.clone(),
    };
    handle.refresh()?;

    thread::spawn(move || {
        log::info!("watcher: event loop started");
        while let Ok(result) = rx.recv() {
            match result {
                Ok(events) => {
                    for ev in events {
                        let kind = path_event_kind(ev.event.kind);
                        for path in &ev.event.paths {
                            if is_platform_junk(path) {
                                continue;
                            }
                            let path_event = PathEvent {
                                path: path.clone(),
                                kind,
                            };
                            let tasks = match state.lock() {
                                Ok(state) => state.registry.classify_path_event(&path_event),
                                Err(e) => {
                                    log::warn!("watcher: state lock poisoned: {e}");
                                    Vec::new()
                                }
                            };
                            for task in tasks {
                                let Some(index_task) = source_task_to_index_task(task) else {
                                    continue;
                                };
                                if let Err(e) = indexer.submit(index_task) {
                                    log::warn!("indexer submit failed: {e}");
                                }
                            }
                        }
                    }
                }
                Err(errors) => {
                    for e in errors {
                        log::warn!("watcher error: {e}");
                    }
                }
            }
        }
        log::warn!("watcher: event loop exited");
    });

    Ok(handle)
}

impl WatcherHandle {
    pub fn refresh(&self) -> Result<()> {
        let agents = self.store.list_agents()?;
        let enabled_agents = crate::agents::sources::enabled_builtin_agents(&agents);
        let registry = crate::agents::sources::builtin_agent_sources_for(enabled_agents);
        let watch_roots = registry.watch_roots().context("collect watch roots")?;
        let mut next_roots: HashMap<PathBuf, RecursiveMode> = HashMap::new();

        for root in &watch_roots {
            let mode = if root.recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            next_roots.insert(root.path.clone(), mode);
        }

        let mut state = self
            .state
            .lock()
            .map_err(|e| anyhow::anyhow!("watcher state lock poisoned: {e}"))?;

        let stale_roots: Vec<PathBuf> = state
            .watched_roots
            .keys()
            .filter(|path| !next_roots.contains_key(*path))
            .cloned()
            .collect();
        for path in stale_roots {
            if let Err(e) = state.debouncer.watcher().unwatch(&path) {
                log::warn!("watcher: failed to unwatch {}: {e}", path.display());
            }
            state.watched_roots.remove(&path);
        }

        for root in &watch_roots {
            let mode = if root.recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            let watch_path = root.path.clone();

            if !watch_path.exists() {
                log::warn!(
                    "watcher: skipping missing root {} (agent={}, purpose={:?})",
                    watch_path.display(),
                    root.agent.as_str(),
                    root.purpose
                );
                continue;
            }

            if state.watched_roots.get(&watch_path) == Some(&mode) {
                continue;
            }
            if state.watched_roots.contains_key(&watch_path) {
                if let Err(e) = state.debouncer.watcher().unwatch(&watch_path) {
                    log::warn!("watcher: failed to refresh {}: {e}", watch_path.display());
                }
            }

            match state.debouncer.watcher().watch(&watch_path, mode) {
                Ok(()) => {
                    state.watched_roots.insert(watch_path, mode);
                }
                Err(e) => log::warn!("watcher: failed to register {}: {e}", watch_path.display()),
            }
        }

        state.registry = registry;
        Ok(())
    }
}

fn path_event_kind(kind: EventKind) -> PathEventKind {
    match kind {
        EventKind::Create(_) => PathEventKind::Create,
        EventKind::Modify(_) => PathEventKind::Modify,
        EventKind::Remove(_) => PathEventKind::Remove,
        _ => PathEventKind::Unknown,
    }
}

// Translates a source-emitted SourceIndexTask into the concrete IndexTask
// consumed by the indexer. SourceIndexTask keeps the source abstraction
// agent-agnostic; this function is the only place the watcher layer needs to
// know which IndexTask variant corresponds to which agent's source kind.
// Adding a new agent today is "add the agent to this match plus add an
// IndexTask variant" — no path-prefix branching elsewhere in watch/.
fn source_task_to_index_task(task: SourceIndexTask) -> Option<IndexTask> {
    match task {
        SourceIndexTask::ReindexSource(source) => {
            let path = PathBuf::from(&source.file_path);
            match source.agent.as_str() {
                "codex" => Some(IndexTask::ReindexCodexFile(path)),
                "claude" => match source.source_kind {
                    SourceKind::Subagent => Some(IndexTask::ReindexClaudeSubagentFile(path)),
                    _ => Some(IndexTask::ReindexClaudeFile(path)),
                },
                "gemini" => Some(IndexTask::ReindexGeminiFile(path)),
                "pi" => Some(IndexTask::ReindexPiFile(path)),
                "opencode" => Some(IndexTask::ReindexOpencodeAll),
                _ => None,
            }
        }
        SourceIndexTask::ReindexScope { agent, scope } => {
            let path = PathBuf::from(&scope);
            match agent.as_str() {
                "claude" => Some(IndexTask::ReindexClaudeProject(path)),
                "opencode" => Some(IndexTask::ReindexOpencodeAll),
                _ => None,
            }
        }
        SourceIndexTask::MarkSourceUnavailable(source) => {
            let path = PathBuf::from(&source.file_path);
            match source.source_kind {
                SourceKind::Subagent => Some(IndexTask::DeleteSubagentFile(path)),
                _ => Some(IndexTask::DeleteFile(path)),
            }
        }
    }
}

// Ignore filesystem metadata noise that OS file managers sprinkle into watched
// directories so we don't keep firing reindex tasks on every Finder browse.
fn is_platform_junk(path: &Path) -> bool {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    matches!(
        name,
        ".DS_Store"             // macOS Finder
            | "Thumbs.db"       // Windows Explorer
            | "ehthumbs.db"     // Windows Explorer (legacy)
            | "desktop.ini"     // Windows
            | ".directory" // KDE Dolphin
    ) || name.starts_with("._") // macOS AppleDouble sidecar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::sources::types::{AgentKind, SessionSource};

    fn src(agent: &str, file_path: &str, kind: SourceKind) -> SessionSource {
        SessionSource {
            agent: AgentKind::new(agent),
            session_id: String::new(),
            scope: file_path.to_string(),
            file_path: file_path.to_string(),
            project: None,
            source_kind: kind,
            metadata: Default::default(),
        }
    }

    #[test]
    fn codex_reindex_source_translates_to_reindex_codex_file() {
        let task = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "codex",
            "/tmp/codex/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(task, Some(IndexTask::ReindexCodexFile(_))));
    }

    #[test]
    fn claude_main_vs_subagent_translate_to_different_tasks() {
        let main = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "claude",
            "/tmp/claude/proj/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(main, Some(IndexTask::ReindexClaudeFile(_))));

        let sub = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "claude",
            "/tmp/claude/proj/a/subagents/b.jsonl",
            SourceKind::Subagent,
        )));
        assert!(matches!(sub, Some(IndexTask::ReindexClaudeSubagentFile(_))));
    }

    #[test]
    fn gemini_reindex_source_translates_to_reindex_gemini_file() {
        let task = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "gemini",
            "/tmp/gemini/abc/chats/session-1.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(task, Some(IndexTask::ReindexGeminiFile(_))));
    }

    #[test]
    fn claude_scope_maps_to_reindex_claude_project() {
        let task = source_task_to_index_task(SourceIndexTask::ReindexScope {
            agent: AgentKind::new("claude"),
            scope: "/tmp/claude/proj".to_string(),
        });
        assert!(matches!(task, Some(IndexTask::ReindexClaudeProject(_))));
    }

    #[test]
    fn gemini_scope_is_not_reindexed() {
        let task = source_task_to_index_task(SourceIndexTask::ReindexScope {
            agent: AgentKind::new("gemini"),
            scope: "/tmp/gemini/abc".to_string(),
        });
        assert!(task.is_none());
    }

    #[test]
    fn mark_source_unavailable_main_vs_subagent_translate_to_distinct_deletes() {
        let main = source_task_to_index_task(SourceIndexTask::MarkSourceUnavailable(src(
            "claude",
            "/tmp/claude/proj/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(main, Some(IndexTask::DeleteFile(_))));

        let sub = source_task_to_index_task(SourceIndexTask::MarkSourceUnavailable(src(
            "claude",
            "/tmp/claude/proj/a/subagents/b.jsonl",
            SourceKind::Subagent,
        )));
        assert!(matches!(sub, Some(IndexTask::DeleteSubagentFile(_))));
    }

    #[test]
    fn unknown_agent_returns_none() {
        let task = source_task_to_index_task(SourceIndexTask::ReindexSource(src(
            "future-agent",
            "/tmp/x.jsonl",
            SourceKind::MainSession,
        )));
        assert!(task.is_none());
    }
}
