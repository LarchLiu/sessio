use anyhow::{Context, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::indexer::{IndexTask, IndexerHandle};
use crate::providers;
use crate::providers::types::{PathEvent, PathEventKind, ProviderTask, SourceKind, WatchPurpose};

pub struct WatcherHandle {
    _debouncer: Box<dyn std::any::Any + Send>,
}

pub fn spawn(indexer: IndexerHandle) -> Result<WatcherHandle> {
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer =
        new_debouncer(Duration::from_millis(500), None, tx).context("create file debouncer")?;

    let registry = providers::builtin_providers();
    let watch_roots = registry.watch_roots().context("collect watch roots")?;

    for root in &watch_roots {
        // ProjectMappings roots are typically a single file (e.g. Gemini's
        // projects.json). We watch the parent dir non-recursively so we can
        // still catch create/modify/remove events even when the file does
        // not exist yet.
        let (watch_path, mode) = match root.purpose {
            WatchPurpose::ProjectMappings => {
                let Some(parent) = root.path.parent() else {
                    continue;
                };
                (parent.to_path_buf(), RecursiveMode::NonRecursive)
            }
            _ => {
                let mode = if root.recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                (root.path.clone(), mode)
            }
        };

        if !watch_path.exists() {
            log::warn!(
                "watcher: skipping missing root {} (agent={}, purpose={:?})",
                watch_path.display(),
                root.agent.as_str(),
                root.purpose
            );
            continue;
        }

        match debouncer
            .watcher()
            .watch(&watch_path, mode)
            .with_context(|| format!("watch {}", watch_path.display()))
        {
            Ok(()) => log::info!(
                "watcher: watching {} ({:?}, agent={}, purpose={:?})",
                watch_path.display(),
                mode,
                root.agent.as_str(),
                root.purpose
            ),
            Err(e) => log::warn!("watcher: failed to register {}: {e}", watch_path.display()),
        }
    }

    thread::spawn(move || {
        log::info!("watcher: event loop started");
        while let Ok(result) = rx.recv() {
            match result {
                Ok(events) => {
                    log::info!("watcher: received {} events", events.len());
                    for ev in events {
                        let kind = path_event_kind(ev.event.kind);
                        for path in &ev.event.paths {
                            log::info!("watcher: event {:?} on {}", ev.event.kind, path.display());
                            if is_platform_junk(path) {
                                continue;
                            }
                            let path_event = PathEvent {
                                path: path.clone(),
                                kind,
                            };
                            for task in registry.classify_path_event(&path_event) {
                                let Some(index_task) = provider_task_to_index_task(task) else {
                                    continue;
                                };
                                log::info!("watcher: dispatching task {:?}", index_task);
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

    Ok(WatcherHandle {
        _debouncer: Box::new(debouncer),
    })
}

fn path_event_kind(kind: EventKind) -> PathEventKind {
    match kind {
        EventKind::Create(_) => PathEventKind::Create,
        EventKind::Modify(_) => PathEventKind::Modify,
        EventKind::Remove(_) => PathEventKind::Remove,
        _ => PathEventKind::Unknown,
    }
}

// Translates a provider-emitted ProviderTask into the concrete IndexTask
// consumed by the indexer. ProviderTask keeps the provider abstraction
// agent-agnostic; this function is the only place the watcher layer needs to
// know which IndexTask variant corresponds to which agent's source kind.
// Adding a new agent today is "add the agent to this match plus add an
// IndexTask variant" — no path-prefix branching elsewhere in watch/.
fn provider_task_to_index_task(task: ProviderTask) -> Option<IndexTask> {
    match task {
        ProviderTask::ReindexSource(source) => {
            let path = PathBuf::from(&source.file_path);
            match source.agent.as_str() {
                "codex" => Some(IndexTask::ReindexCodexFile(path)),
                "claude" => match source.source_kind {
                    SourceKind::Subagent => Some(IndexTask::ReindexClaudeSubagentFile(path)),
                    _ => Some(IndexTask::ReindexClaudeFile(path)),
                },
                "gemini" => Some(IndexTask::ReindexGeminiLogs(path)),
                _ => None,
            }
        }
        ProviderTask::ReindexScope { agent, scope } => {
            let path = PathBuf::from(&scope);
            match agent.as_str() {
                "claude" => Some(IndexTask::ReindexClaudeProject(path)),
                "gemini" => Some(IndexTask::ReindexGeminiLogs(path)),
                _ => None,
            }
        }
        ProviderTask::MarkSourceUnavailable(source) => {
            let path = PathBuf::from(&source.file_path);
            match source.source_kind {
                SourceKind::Subagent => Some(IndexTask::DeleteSubagentFile(path)),
                _ => Some(IndexTask::DeleteFile(path)),
            }
        }
        ProviderTask::RefreshProjectMappings { agent } => match agent.as_str() {
            "gemini" => Some(IndexTask::RefreshGeminiProjectMappings),
            _ => None,
        },
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
    use crate::providers::types::{AgentKind, SessionSource};

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
        let task = provider_task_to_index_task(ProviderTask::ReindexSource(src(
            "codex",
            "/tmp/codex/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(task, Some(IndexTask::ReindexCodexFile(_))));
    }

    #[test]
    fn claude_main_vs_subagent_translate_to_different_tasks() {
        let main = provider_task_to_index_task(ProviderTask::ReindexSource(src(
            "claude",
            "/tmp/claude/proj/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(main, Some(IndexTask::ReindexClaudeFile(_))));

        let sub = provider_task_to_index_task(ProviderTask::ReindexSource(src(
            "claude",
            "/tmp/claude/proj/a/subagents/b.jsonl",
            SourceKind::Subagent,
        )));
        assert!(matches!(sub, Some(IndexTask::ReindexClaudeSubagentFile(_))));
    }

    #[test]
    fn gemini_reindex_source_translates_to_reindex_gemini_logs() {
        let task = provider_task_to_index_task(ProviderTask::ReindexSource(src(
            "gemini",
            "/tmp/gemini/abc/logs.json",
            SourceKind::Logs,
        )));
        assert!(matches!(task, Some(IndexTask::ReindexGeminiLogs(_))));
    }

    #[test]
    fn claude_scope_maps_to_reindex_claude_project() {
        let task = provider_task_to_index_task(ProviderTask::ReindexScope {
            agent: AgentKind::new("claude"),
            scope: "/tmp/claude/proj".to_string(),
        });
        assert!(matches!(task, Some(IndexTask::ReindexClaudeProject(_))));
    }

    #[test]
    fn gemini_scope_maps_to_reindex_gemini_logs() {
        let task = provider_task_to_index_task(ProviderTask::ReindexScope {
            agent: AgentKind::new("gemini"),
            scope: "/tmp/gemini/abc/logs.json".to_string(),
        });
        assert!(matches!(task, Some(IndexTask::ReindexGeminiLogs(_))));
    }

    #[test]
    fn mark_source_unavailable_main_vs_subagent_translate_to_distinct_deletes() {
        let main = provider_task_to_index_task(ProviderTask::MarkSourceUnavailable(src(
            "claude",
            "/tmp/claude/proj/a.jsonl",
            SourceKind::MainSession,
        )));
        assert!(matches!(main, Some(IndexTask::DeleteFile(_))));

        let sub = provider_task_to_index_task(ProviderTask::MarkSourceUnavailable(src(
            "claude",
            "/tmp/claude/proj/a/subagents/b.jsonl",
            SourceKind::Subagent,
        )));
        assert!(matches!(sub, Some(IndexTask::DeleteSubagentFile(_))));
    }

    #[test]
    fn refresh_gemini_project_mappings_translates() {
        let task = provider_task_to_index_task(ProviderTask::RefreshProjectMappings {
            agent: AgentKind::new("gemini"),
        });
        assert!(matches!(task, Some(IndexTask::RefreshGeminiProjectMappings)));
    }

    #[test]
    fn unknown_agent_returns_none() {
        let task = provider_task_to_index_task(ProviderTask::ReindexSource(src(
            "future-agent",
            "/tmp/x.jsonl",
            SourceKind::MainSession,
        )));
        assert!(task.is_none());
    }
}
