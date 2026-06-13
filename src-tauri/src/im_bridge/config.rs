//! IM bridge configuration, loaded from `~/.sessio/im-bridge.yaml`.
//!
//! Kept separate from the app's hand-written `config.toml` parser: the bridge
//! config is nested and platform-shaped, so it leans on `serde_yaml` (already a
//! dependency) for a maintainable schema.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::models::Agent;

/// Top-level IM bridge configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImBridgeConfig {
    /// Master switch. When false the bridge never starts.
    #[serde(default)]
    pub enabled: bool,

    /// Default agent for new chat-initiated sessions when the user does not
    /// pick one with `/use`.
    #[serde(default = "default_agent", alias = "default_agent")]
    pub default_agent: Agent,

    /// Default workspace for chat-initiated sessions when a chat-specific
    /// binding is absent.
    #[serde(default, alias = "default_workspace")]
    pub default_workspace: Option<String>,

    /// Workspaces a chat is allowed to open sessions in. The first entry is the
    /// default. Sessions outside this allowlist are rejected — this is the main
    /// guard against turning the bridge into arbitrary remote command execution.
    #[serde(default, alias = "allowed_workspaces")]
    pub allowed_workspaces: Vec<String>,

    /// Optional per-channel workspace overrides.
    #[serde(default, alias = "workspace_bindings")]
    pub workspace_bindings: Vec<WorkspaceBindingConfig>,

    /// Telegram platform config. Absent = Telegram disabled.
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
}

/// Bind one external chat/channel to a local workspace.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBindingConfig {
    pub platform: String,
    #[serde(alias = "chat_id")]
    pub chat_id: String,
    #[serde(alias = "workspace_path")]
    pub workspace_path: String,
}

/// Telegram bot configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramConfig {
    /// Whether the Telegram worker should start.
    #[serde(default)]
    pub enabled: bool,

    /// Bot token from @BotFather.
    #[serde(default, alias = "bot_token")]
    pub bot_token: String,

    /// Allowlist of Telegram user IDs permitted to drive the bridge. Empty =
    /// nobody (fail closed). A message from any other user is ignored.
    #[serde(default, alias = "allowed_user_ids")]
    pub allowed_user_ids: Vec<i64>,

    /// Optional model override for Telegram-created sessions.
    #[serde(default, alias = "default_model")]
    pub default_model: Option<String>,

    /// Optional effort override for Telegram-created sessions.
    #[serde(default, alias = "default_effort")]
    pub default_effort: Option<String>,

    /// Long-poll timeout in seconds passed to `getUpdates`. Telegram holds the
    /// request open this long when idle.
    #[serde(default = "default_poll_timeout", alias = "poll_timeout_secs")]
    pub poll_timeout_secs: u64,

    /// Optional override for the Telegram API base (for self-hosted bot API
    /// servers). Defaults to the public endpoint.
    #[serde(default, alias = "api_base")]
    pub api_base: Option<String>,
}

fn default_agent() -> Agent {
    Agent::Claude
}

fn default_poll_timeout() -> u64 {
    30
}

impl Default for ImBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            default_agent: default_agent(),
            default_workspace: None,
            allowed_workspaces: Vec::new(),
            workspace_bindings: Vec::new(),
            telegram: Some(TelegramConfig::default()),
        }
    }
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            allowed_user_ids: Vec::new(),
            default_model: None,
            default_effort: None,
            poll_timeout_secs: default_poll_timeout(),
            api_base: None,
        }
    }
}

impl ImBridgeConfig {
    /// The default workspace for chat-initiated sessions, if any.
    pub fn default_workspace(&self) -> Option<&str> {
        self.default_workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.allowed_workspaces.first().map(String::as_str))
    }

    /// Whether `path` is permitted for a new session. An empty allowlist denies
    /// everything (fail closed).
    pub fn is_workspace_allowed(&self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        self.allowed_workspaces.iter().any(|w| w == path)
            || self.default_workspace() == Some(path)
            || self
                .workspace_bindings
                .iter()
                .any(|binding| binding.workspace_path.trim() == path)
    }

    /// Workspace selected for a specific chat. Falls back to the global default.
    pub fn workspace_for_chat(&self, platform: &str, chat_id: &str) -> Option<&str> {
        self.workspace_bindings
            .iter()
            .find(|binding| {
                binding.platform == platform && binding.chat_id.trim() == chat_id.trim()
            })
            .map(|binding| binding.workspace_path.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.default_workspace())
    }

    /// Optional model override for sessions opened by a given platform.
    pub fn default_model_for_platform(&self, platform: &str) -> Option<&str> {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .and_then(|config| config.default_model.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            _ => None,
        }
    }

    /// Optional effort override for sessions opened by a given platform.
    pub fn default_effort_for_platform(&self, platform: &str) -> Option<&str> {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .and_then(|config| config.default_effort.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            _ => None,
        }
    }
}

/// Path to the bridge config file: `~/.sessio/im-bridge.yaml`.
fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".sessio").join("im-bridge.yaml"))
}

/// Best-effort display string for the config path, for log messages.
pub fn config_path_display() -> String {
    config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.sessio/im-bridge.yaml".to_string())
}

/// Load the config file. Returns `Ok(None)` when the file does not exist so the
/// caller can treat "no bridge configured" as a normal, silent state.
pub fn load_config() -> Result<Option<ImBridgeConfig>> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("read im-bridge config {}", path.display()))?;
    if contents.trim().is_empty() {
        return Ok(None);
    }
    let config: ImBridgeConfig = serde_yaml::from_str(&contents)
        .with_context(|| format!("parse im-bridge config {}", path.display()))?;
    Ok(Some(config))
}

/// Load config for UI editing, returning defaults when the config file is
/// absent.
pub fn load_config_or_default() -> Result<ImBridgeConfig> {
    Ok(load_config()?.unwrap_or_default())
}

/// Persist the bridge config. The running service also receives the saved
/// config through the Tauri command path so Settings changes take effect live.
pub fn save_config(config: &ImBridgeConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create im-bridge config dir {}", parent.display()))?;
    }
    let contents = serde_yaml::to_string(config)
        .with_context(|| format!("serialize im-bridge config {}", path.display()))?;
    std::fs::write(&path, contents)
        .with_context(|| format!("write im-bridge config {}", path.display()))
}
