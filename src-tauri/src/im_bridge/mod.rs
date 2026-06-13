//! IM bridge: expose sessio agent runtime over external chat platforms.
//!
//! The bridge runs as a headless "frontend": it drives the same
//! [`RuntimeManager`] the Tauri UI uses, mapping each IM chat to a sessio
//! runtime session. Inbound chat messages become agent prompts; outbound
//! runtime events are aggregated per turn and pushed back to the chat.
//!
//! The first supported platform is Telegram (long polling, no public ingress
//! required). The module is structured so additional platforms (Discord, Lark,
//! WeWork) can be added under `platforms/` without touching the router or the
//! outbound aggregator.

mod config;
mod idle;
mod outbound;
mod platforms;
mod router;
mod state;

pub use config::{ImBridgeConfig, TelegramConfig, WorkspaceBindingConfig};

use std::sync::Arc;

use anyhow::Result;
use tauri::AppHandle;

use crate::agents::runtime::RuntimeManager;
use crate::store::SessionStore;

use self::state::ImBridgeState;

/// Owns the IM bridge background workers. Held in Tauri managed state so it
/// lives for the duration of the app.
#[derive(Clone)]
pub struct ImBridgeService {
    state: Arc<ImBridgeState>,
}

impl ImBridgeService {
    /// Construct the service. Does not start any workers yet.
    pub fn new(
        _app: AppHandle,
        store: Arc<dyn SessionStore>,
        runtime: RuntimeManager,
        config: ImBridgeConfig,
    ) -> Self {
        let state = Arc::new(ImBridgeState::new(store, runtime, config));
        Self { state }
    }

    /// Load config from `~/.sessio/im-bridge.yaml` and construct the service.
    /// Returns `None` (after logging) when the bridge is disabled or unconfigured,
    /// so the caller can skip startup without treating it as an error.
    pub fn from_config_file(
        app: AppHandle,
        store: Arc<dyn SessionStore>,
        runtime: RuntimeManager,
    ) -> Option<Self> {
        match config::load_config() {
            Ok(Some(config)) if config.enabled => Some(Self::new(app, store, runtime, config)),
            Ok(Some(_)) => {
                log::info!("[im-bridge] disabled via config (enabled = false)");
                None
            }
            Ok(None) => {
                log::info!(
                    "[im-bridge] no config file at {}; bridge inactive",
                    config::config_path_display()
                );
                None
            }
            Err(error) => {
                log::warn!("[im-bridge] failed to load config: {error:#}");
                None
            }
        }
    }

    /// Start the outbound event pump and every configured platform listener.
    /// Each runs on its own thread; this returns immediately.
    pub fn start(&self) -> Result<()> {
        platforms::spawn_all(self.state.clone());
        outbound::spawn(self.state.clone())?;
        idle::spawn(self.state.clone())?;
        log::info!("[im-bridge] started");
        Ok(())
    }

    /// Apply a freshly saved config to the running bridge. Platform workers
    /// pick this up on their next loop, so Settings changes no longer require
    /// an app restart.
    pub fn update_config(&self, config: ImBridgeConfig) {
        self.state.update_config(config);
        log::info!("[im-bridge] config updated");
    }
}

pub fn load_config_or_default() -> Result<ImBridgeConfig> {
    config::load_config_or_default()
}

pub fn save_config(config: &ImBridgeConfig) -> Result<()> {
    config::save_config(config)
}

pub fn detect_telegram_user_ids(bot_token: &str, api_base: Option<&str>) -> Result<Vec<i64>> {
    platforms::detect_telegram_user_ids(bot_token, api_base)
}

pub fn test_telegram_bot_connection(bot_token: &str, api_base: Option<&str>) -> Result<()> {
    platforms::test_telegram_bot_connection(bot_token, api_base)
}
