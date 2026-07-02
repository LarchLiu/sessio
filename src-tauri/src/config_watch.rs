use anyhow::{Context, Result};
use notify::{recommended_watcher, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

pub const APP_CONFIG_UPDATED_EVENT: &str = "app_config_updated";

#[derive(Clone)]
pub struct ConfigWatcher {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
    _watched_dir: PathBuf,
}

impl ConfigWatcher {
    pub fn new(app: AppHandle) -> Result<Self> {
        let config_path = crate::app_paths::config_path()?;
        let watched_dir = config_path
            .parent()
            .context("config path has no parent directory")?
            .to_path_buf();
        let callback_app = app.clone();
        let callback_path = config_path.clone();
        let mut watcher = recommended_watcher(move |result: notify::Result<Event>| match result {
            Ok(event) => handle_config_event(&callback_app, &callback_path, event),
            Err(error) => log::warn!("[config-watch] watcher error: {error}"),
        })
        .context("create config watcher")?;
        watcher
            .watch(&watched_dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("watch config dir {}", watched_dir.display()))?;

        Ok(Self {
            _watcher: Arc::new(Mutex::new(watcher)),
            _watched_dir: watched_dir,
        })
    }
}

fn handle_config_event(app: &AppHandle, config_path: &Path, event: Event) {
    if !matches!(
        event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) {
        return;
    }
    if !event.paths.iter().any(|path| path == config_path) {
        return;
    }
    apply_updated_config(app);
}

fn apply_updated_config(app: &AppHandle) {
    match crate::config::load_config_strict() {
        Ok(config) => {
            if let Some(cache) = app.try_state::<crate::mcp::McpSettingsCache>() {
                cache.set(crate::mcp::merged_settings(
                    &config.mcp,
                    &config.computer_use,
                ));
            }
            if let Some(runtime) = app.try_state::<crate::agents::runtime::RuntimeManager>() {
                runtime.update_computer_use_settings(config.computer_use.clone());
            }
            crate::network::apply_network_proxy_env(&config.network.proxy);
            let _ = app.emit(APP_CONFIG_UPDATED_EVENT, ());
        }
        Err(error) => {
            log::warn!("[config-watch] ignoring invalid config update: {error:#}");
        }
    }
}
