use anyhow::{Context, Result};
use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

pub const APPS_UPDATED_EVENT: &str = "sessio_apps_updated";

#[derive(Clone)]
pub struct AppsWatcher {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
    _apps_dir: PathBuf,
}

impl AppsWatcher {
    pub fn new(app: AppHandle) -> Result<Self> {
        let apps_dir = crate::app_paths::apps_dir()?;
        let watched_dir = if apps_dir.exists() {
            apps_dir.clone()
        } else {
            apps_dir
                .parent()
                .context("apps directory has no parent directory")?
                .to_path_buf()
        };
        let callback_app = app.clone();
        let callback_apps_dir = apps_dir.clone();
        let mut watcher = recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event) => emit_apps_updated(&callback_app, &callback_apps_dir, event),
            Err(error) => log::warn!("[apps-watch] watcher error: {error}"),
        })
        .context("create apps watcher")?;
        watcher
            .watch(&watched_dir, RecursiveMode::Recursive)
            .with_context(|| format!("watch apps directory {}", watched_dir.display()))?;

        Ok(Self {
            _watcher: Arc::new(Mutex::new(watcher)),
            _apps_dir: apps_dir,
        })
    }
}

fn emit_apps_updated(app: &AppHandle, apps_dir: &Path, event: Event) {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return;
    }
    let changed = event
        .paths
        .iter()
        .any(|path| path == apps_dir || path.starts_with(apps_dir));
    if changed {
        let _ = app.emit(APPS_UPDATED_EVENT, ());
    }
}
