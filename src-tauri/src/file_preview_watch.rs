use anyhow::{Context, Result};
use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

const PREVIEW_FILE_CHANGED_EVENT: &str = "preview_file_changed";

#[derive(Clone)]
pub struct PreviewFileWatcher {
    state: Arc<Mutex<PreviewFileWatcherState>>,
    watched_files: Arc<Mutex<HashMap<PathBuf, usize>>>,
}

struct PreviewFileWatcherState {
    watcher: RecommendedWatcher,
    watched_dirs: HashMap<PathBuf, usize>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewFileChangedPayload {
    path: String,
}

impl PreviewFileWatcher {
    pub fn new(app: AppHandle) -> Result<Self> {
        let watched_files = Arc::new(Mutex::new(HashMap::new()));
        let callback_files = watched_files.clone();
        let callback_app = app.clone();
        let watcher = recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event) => emit_preview_events(&callback_app, &callback_files, event),
            Err(error) => log::warn!("preview watcher error: {error}"),
        })
        .context("create preview watcher")?;

        Ok(Self {
            state: Arc::new(Mutex::new(PreviewFileWatcherState {
                watcher,
                watched_dirs: HashMap::new(),
            })),
            watched_files,
        })
    }

    pub fn watch_path(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("File path has no parent directory"))?
            .to_path_buf();
        let target = path.to_path_buf();

        let mut state = self
            .state
            .lock()
            .map_err(|e| anyhow::anyhow!("preview watcher state lock poisoned: {e}"))?;
        let mut watched_files = self
            .watched_files
            .lock()
            .map_err(|e| anyhow::anyhow!("preview watcher file lock poisoned: {e}"))?;

        if let Some(count) = watched_files.get_mut(&target) {
            *count += 1;
        } else {
            watched_files.insert(target, 1);
        }

        let needs_watch = !matches!(state.watched_dirs.get(&parent), Some(count) if *count > 0);
        if needs_watch {
            state
                .watcher
                .watch(&parent, RecursiveMode::NonRecursive)
                .with_context(|| format!("watch preview parent {}", parent.display()))?;
        }
        *state.watched_dirs.entry(parent.clone()).or_insert(0) += 1;
        Ok(())
    }

    pub fn unwatch_path(&self, path: &Path) -> Result<()> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        let parent = parent.to_path_buf();
        let target = path.to_path_buf();

        let mut state = self
            .state
            .lock()
            .map_err(|e| anyhow::anyhow!("preview watcher state lock poisoned: {e}"))?;
        let mut watched_files = self
            .watched_files
            .lock()
            .map_err(|e| anyhow::anyhow!("preview watcher file lock poisoned: {e}"))?;

        let remove_file = match watched_files.get_mut(&target) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if remove_file {
            watched_files.remove(&target);
        }

        let remove_dir = match state.watched_dirs.get_mut(&parent) {
            Some(count) if *count > 1 => {
                *count -= 1;
                false
            }
            Some(_) => true,
            None => false,
        };
        if remove_dir {
            state
                .watcher
                .unwatch(&parent)
                .with_context(|| format!("unwatch preview parent {}", parent.display()))?;
            state.watched_dirs.remove(&parent);
        }

        Ok(())
    }
}

fn emit_preview_events(
    app: &AppHandle,
    watched_files: &Arc<Mutex<HashMap<PathBuf, usize>>>,
    event: Event,
) {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return;
    }

    let watched_paths = match watched_files.lock() {
        Ok(guard) => guard.keys().cloned().collect::<Vec<_>>(),
        Err(error) => {
            log::warn!("preview watcher file lock poisoned: {error}");
            return;
        }
    };

    for watched_path in watched_paths {
        if !event
            .paths
            .iter()
            .any(|event_path| event_path == &watched_path)
        {
            continue;
        }
        let _ = app.emit(
            PREVIEW_FILE_CHANGED_EVENT,
            PreviewFileChangedPayload {
                path: watched_path.to_string_lossy().to_string(),
            },
        );
    }
}
