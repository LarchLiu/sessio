use anyhow::{Context, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::indexer::{IndexTask, IndexerHandle};
use crate::readers;

pub struct WatcherHandle {
    _debouncer: Box<dyn std::any::Any + Send>,
}

pub fn spawn(indexer: IndexerHandle) -> Result<WatcherHandle> {
    let (tx, rx) = mpsc::channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, tx)
        .context("create file debouncer")?;

    let (codex_live, codex_archived) = readers::codex::roots()?;
    let claude_root = readers::claude::root_dir()?;
    let (gemini_tmp, gemini_projects_json) = readers::gemini::paths()?;

    for root in [&codex_live, &codex_archived] {
        if root.exists() {
            log::info!("watcher: watching codex {}", root.display());
            debouncer
                .watcher()
                .watch(root, RecursiveMode::Recursive)
                .with_context(|| format!("watch {}", root.display()))?;
        } else {
            log::warn!("watcher: codex root does not exist: {}", root.display());
        }
    }
    if let Some(root) = claude_root.as_ref() {
        if root.exists() {
            debouncer
                .watcher()
                .watch(root, RecursiveMode::Recursive)
                .with_context(|| format!("watch {}", root.display()))?;
        }
    }
    if gemini_tmp.exists() {
        debouncer
            .watcher()
            .watch(&gemini_tmp, RecursiveMode::Recursive)
            .with_context(|| format!("watch {}", gemini_tmp.display()))?;
    }
    if let Some(parent) = gemini_projects_json.parent() {
        if parent.exists() {
            debouncer
                .watcher()
                .watch(parent, RecursiveMode::NonRecursive)
                .ok();
        }
    }

    thread::spawn(move || {
        let roots = Roots {
            codex_live,
            codex_archived,
            claude_root,
            gemini_tmp,
            gemini_projects_json,
        };
        log::info!("watcher: event loop started");
        while let Ok(result) = rx.recv() {
            match result {
                Ok(events) => {
                    log::info!("watcher: received {} events", events.len());
                    for ev in events {
                        for path in &ev.event.paths {
                            log::info!("watcher: event {:?} on {}", ev.event.kind, path.display());
            if let Some(task) = dispatch(path, ev.event.kind, &roots) {
                log::info!("watcher: dispatching task {:?}", task);
                if let Err(e) = indexer.submit(task) {
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

struct Roots {
    codex_live: PathBuf,
    codex_archived: PathBuf,
    claude_root: Option<PathBuf>,
    gemini_tmp: PathBuf,
    gemini_projects_json: PathBuf,
}

fn dispatch(path: &Path, kind: EventKind, roots: &Roots) -> Option<IndexTask> {
    if is_platform_junk(path) {
        return None;
    }
    let removed = matches!(kind, EventKind::Remove(_));

    if path == roots.gemini_projects_json {
        return Some(IndexTask::RefreshGeminiProjectMappings);
    }

    if path.starts_with(&roots.codex_live) || path.starts_with(&roots.codex_archived) {
        if is_jsonl(path) {
            if removed {
                return Some(IndexTask::DeleteFile(path.to_path_buf()));
            }
            return Some(IndexTask::ReindexCodexFile(path.to_path_buf()));
        }
        return None;
    }

    if let Some(claude_root) = roots.claude_root.as_ref() {
        if path.starts_with(claude_root) {
            return dispatch_claude(path, claude_root, removed);
        }
    }

    if path.starts_with(&roots.gemini_tmp) {
        return dispatch_gemini(path, &roots.gemini_tmp);
    }

    None
}

fn dispatch_claude(path: &Path, claude_root: &Path, removed: bool) -> Option<IndexTask> {
    let rel = path.strip_prefix(claude_root).ok()?;
    let mut comps = rel.components();
    let project_name = comps.next()?.as_os_str();
    let project_dir = claude_root.join(project_name);

    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name == "sessions-index.json" {
        return Some(IndexTask::ReindexClaudeProject(project_dir));
    }
    // Top-level <project>/<session>.jsonl edits only affect that one session's
    // main row. Subagents are handled below on their own track.
    if is_jsonl(path) && rel.components().count() == 2 {
        if removed {
            return Some(IndexTask::DeleteFile(path.to_path_buf()));
        }
        return Some(IndexTask::ReindexClaudeFile(path.to_path_buf()));
    }
    // Subagent jsonls live under <project>/<parent_session>/subagents/...
    // and have an independent lifecycle from the parent main row — touch
    // only the subagent row.
    let rest: Vec<_> = rel.components().skip(1).collect();
    if rest.iter().any(|c| c.as_os_str() == "subagents") && is_jsonl(path) {
        if removed {
            return Some(IndexTask::DeleteSubagentFile(path.to_path_buf()));
        }
        return Some(IndexTask::ReindexClaudeSubagentFile(path.to_path_buf()));
    }
    None
}

fn dispatch_gemini(path: &Path, gemini_tmp: &Path) -> Option<IndexTask> {
    let rel = path.strip_prefix(gemini_tmp).ok()?;
    let mut comps = rel.components();
    let dir = comps.next()?.as_os_str();
    let logs = gemini_tmp.join(dir).join("logs.json");
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if file_name == "logs.json" {
        return Some(IndexTask::ReindexGeminiLogs(logs));
    }
    None
}

fn is_jsonl(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("jsonl")
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
            | ".directory"      // KDE Dolphin
    ) || name.starts_with("._") // macOS AppleDouble sidecar
}
