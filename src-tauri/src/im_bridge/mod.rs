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

mod attachments;
mod config;
mod idle;
mod outbound;
mod platforms;
mod router;
mod state;

pub use config::{
    DiscordConfig, FeishuConfig, ImBridgeConfig, TelegramConfig, WechatConfig,
    WorkspaceBindingConfig,
};

use std::sync::Arc;

use anyhow::{bail, Result};
use tauri::AppHandle;

use crate::agents::runtime::types::AgentAttachmentKind;
use crate::agents::runtime::RuntimeManager;
use crate::store::SessionStore;

use self::attachments::extract_outbound_attachments;
use self::state::{ChatKey, ImBridgeState};

const MAX_NOTIFICATION_ATTACHMENTS: usize = 5;

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

    /// Load config from the current app home's `im-bridge.yaml` and construct the service.
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

    /// Submit `text` to a chat as if it had arrived from the platform, lazily
    /// opening a default session when the chat is unbound. The agent's output is
    /// delivered asynchronously through the outbound pump to the platform sink,
    /// so the platform listener for `platform` must be running.
    pub fn submit_prompt_to_chat(&self, platform: &str, chat_id: &str, text: &str) -> Result<()> {
        let platform_tag: &'static str = match platform {
            "telegram" => "telegram",
            "discord" => "discord",
            "feishu" => "feishu",
            "wechat" => "wechat",
            other => bail!("unknown IM platform: {other}"),
        };
        let key = ChatKey::new(platform_tag, chat_id.to_string());
        let outcome = router::handle_message_with_attachments(&self.state, &key, text, Vec::new());
        if let Some(reply) = outcome.reply {
            if let Err(error) = self.state.send_to_chat(&key, &reply) {
                log::warn!(
                    "[im-bridge] failed to deliver IM reply to {platform} chat {chat_id}: {error:#}"
                );
            }
        }
        Ok(())
    }

    /// Send outbound text to a chat without routing it back through the IM
    /// session handler. Used for notifications where IM is only a sink.
    pub fn send_text_to_chat(&self, platform: &str, chat_id: &str, text: &str) -> Result<()> {
        let platform_tag = platform_tag(platform)?;
        let key = ChatKey::new(platform_tag, chat_id.to_string());
        self.state.send_to_chat(&key, text)
    }

    /// Scan notification text for local image/file references and push them to
    /// the chat. This is outbound-only and never opens or drives an IM session.
    pub fn send_referenced_attachments_to_chat(
        &self,
        platform: &str,
        chat_id: &str,
        text: &str,
        workspace_path: &str,
    ) -> Result<usize> {
        let platform_tag = platform_tag(platform)?;
        let key = ChatKey::new(platform_tag, chat_id.to_string());
        let attachments = extract_outbound_attachments(text, workspace_path)
            .into_iter()
            .take(MAX_NOTIFICATION_ATTACHMENTS)
            .collect::<Vec<_>>();
        let count = attachments.len();
        for attachment in attachments {
            let caption = attachment.display_name.as_deref();
            match attachment.kind {
                AgentAttachmentKind::Image => {
                    self.state
                        .send_image_to_chat(&key, &attachment.path, caption)?;
                }
                AgentAttachmentKind::File => {
                    self.state
                        .send_file_to_chat(&key, &attachment.path, caption)?;
                }
            }
        }
        Ok(count)
    }
}

fn platform_tag(platform: &str) -> Result<&'static str> {
    Ok(match platform {
        "telegram" => "telegram",
        "discord" => "discord",
        "feishu" => "feishu",
        "wechat" => "wechat",
        other => bail!("unknown IM platform: {other}"),
    })
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

pub fn test_discord_bot_connection(bot_token: &str, api_base: Option<&str>) -> Result<()> {
    platforms::test_discord_bot_connection(bot_token, api_base)
}

pub fn test_feishu_bot_connection(
    app_id: &str,
    app_secret: &str,
    domain: Option<&str>,
) -> Result<()> {
    platforms::test_feishu_bot_connection(app_id, app_secret, domain)
}

pub fn test_wechat_bot_connection(bot_token: &str, base_url: Option<&str>) -> Result<()> {
    platforms::test_wechat_bot_connection(bot_token, base_url)
}

pub fn get_wechat_qrcode(base_url: Option<&str>) -> Result<serde_json::Value> {
    platforms::get_wechat_qrcode(base_url)
        .and_then(|value| serde_json::to_value(value).map_err(Into::into))
}

pub fn poll_wechat_qrcode_status(
    qrcode: &str,
    base_url: Option<&str>,
) -> Result<serde_json::Value> {
    platforms::poll_wechat_qrcode_status(qrcode, base_url)
        .and_then(|value| serde_json::to_value(value).map_err(Into::into))
}
