//! IM bridge configuration, loaded from the current app home's `im-bridge.yaml`.
//!
//! Kept separate from the app's hand-written `config.toml` parser: the bridge
//! config is nested and platform-shaped, so it leans on `serde_yaml` (already a
//! dependency) for a maintainable schema.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app_paths;
use crate::models::Agent;

/// Top-level IM bridge configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImBridgeConfig {
    /// Master switch. When false the bridge never starts.
    #[serde(default)]
    pub enabled: bool,

    /// Seconds of no IM interaction before the in-memory runtime connection is
    /// suspended. The channel session row stays active and resumes on next use.
    #[serde(default = "default_idle_timeout_secs", alias = "idle_timeout_secs")]
    pub idle_timeout_secs: u64,

    /// Telegram platform config. Absent = Telegram disabled.
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,

    /// Discord platform config. Absent = Discord disabled.
    #[serde(default)]
    pub discord: Option<DiscordConfig>,

    /// Feishu/Lark platform config. Absent = Feishu disabled.
    #[serde(default)]
    pub feishu: Option<FeishuConfig>,

    /// WeChat iLink platform config. Absent = WeChat disabled.
    #[serde(default)]
    pub wechat: Option<WechatConfig>,
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

    /// Agent used for new Telegram-created sessions. When absent, the bridge
    /// resolves to the first enabled runtime agent from the local store.
    #[serde(default)]
    pub agent: Option<Agent>,

    /// Model used for new Telegram-created sessions.
    #[serde(default)]
    pub model: Option<String>,

    /// Effort used for new Telegram-created sessions.
    #[serde(default)]
    pub effort: Option<String>,

    /// Default workspace for Telegram-created sessions when a chat-specific
    /// binding is absent.
    #[serde(default, alias = "default_workspace")]
    pub default_workspace: Option<String>,

    /// Workspaces Telegram chats are allowed to open sessions in. Empty denies
    /// everything unless a default workspace or binding supplies the path.
    #[serde(default, alias = "allowed_workspaces")]
    pub allowed_workspaces: Vec<String>,

    /// Optional per-Telegram-chat workspace overrides.
    #[serde(default, alias = "workspace_bindings")]
    pub workspace_bindings: Vec<WorkspaceBindingConfig>,

    /// Bot token from @BotFather.
    #[serde(default, alias = "bot_token")]
    pub bot_token: String,

    /// Allowlist of Telegram user IDs permitted to drive the bridge. Empty =
    /// nobody (fail closed). A message from any other user is ignored.
    #[serde(default, alias = "allowed_user_ids")]
    pub allowed_user_ids: Vec<i64>,

    /// Long-poll timeout in seconds passed to `getUpdates`. Telegram holds the
    /// request open this long when idle.
    #[serde(default = "default_poll_timeout", alias = "poll_timeout_secs")]
    pub poll_timeout_secs: u64,

    /// Optional override for the Telegram API base (for self-hosted bot API
    /// servers). Defaults to the public endpoint.
    #[serde(default, alias = "api_base")]
    pub api_base: Option<String>,
}

/// Discord bot configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordConfig {
    /// Whether the Discord worker should start.
    #[serde(default)]
    pub enabled: bool,

    /// Agent used for new Discord-created sessions. When absent, the bridge
    /// resolves to the first enabled runtime agent from the local store.
    #[serde(default)]
    pub agent: Option<Agent>,

    /// Model used for new Discord-created sessions.
    #[serde(default)]
    pub model: Option<String>,

    /// Effort used for new Discord-created sessions.
    #[serde(default)]
    pub effort: Option<String>,

    /// Default workspace for Discord-created sessions when a channel-specific
    /// binding is absent.
    #[serde(default, alias = "default_workspace")]
    pub default_workspace: Option<String>,

    /// Workspaces Discord channels are allowed to open sessions in.
    #[serde(default, alias = "allowed_workspaces")]
    pub allowed_workspaces: Vec<String>,

    /// Optional per-Discord-channel workspace overrides.
    #[serde(default, alias = "workspace_bindings")]
    pub workspace_bindings: Vec<WorkspaceBindingConfig>,

    /// Bot token from the Discord Developer Portal.
    #[serde(default, alias = "bot_token")]
    pub bot_token: String,

    /// Guild/server IDs the bot may respond in. Empty = all guilds.
    #[serde(default, alias = "allowed_server_ids")]
    pub allowed_server_ids: Vec<String>,

    /// Channel IDs the bot may respond in. Empty = all channels.
    #[serde(default, alias = "allowed_channel_ids")]
    pub allowed_channel_ids: Vec<String>,

    /// When true, only respond to DMs, replies, or messages mentioning the bot.
    #[serde(default, alias = "mention_only")]
    pub mention_only: bool,

    /// Optional override for the Discord REST API base.
    #[serde(default, alias = "api_base")]
    pub api_base: Option<String>,

    /// Optional override for the Discord Gateway URL.
    #[serde(default, alias = "gateway_url")]
    pub gateway_url: Option<String>,
}

/// Feishu/Lark bot configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeishuConfig {
    /// Whether the Feishu worker should start.
    #[serde(default)]
    pub enabled: bool,

    /// Agent used for new Feishu-created sessions. When absent, the bridge
    /// resolves to the first enabled runtime agent from the local store.
    #[serde(default)]
    pub agent: Option<Agent>,

    /// Model used for new Feishu-created sessions.
    #[serde(default)]
    pub model: Option<String>,

    /// Effort used for new Feishu-created sessions.
    #[serde(default)]
    pub effort: Option<String>,

    /// Default workspace for Feishu-created sessions when a chat-specific
    /// binding is absent.
    #[serde(default, alias = "default_workspace")]
    pub default_workspace: Option<String>,

    /// Workspaces Feishu chats are allowed to open sessions in.
    #[serde(default, alias = "allowed_workspaces")]
    pub allowed_workspaces: Vec<String>,

    /// Optional per-Feishu-chat workspace overrides.
    #[serde(default, alias = "workspace_bindings")]
    pub workspace_bindings: Vec<WorkspaceBindingConfig>,

    /// Feishu/Lark application ID.
    #[serde(default, alias = "app_id")]
    pub app_id: String,

    /// Feishu/Lark application secret.
    #[serde(default, alias = "app_secret")]
    pub app_secret: String,

    /// Open platform domain. Defaults to Feishu China; Lark international can
    /// use `https://open.larksuite.com`.
    #[serde(default)]
    pub domain: Option<String>,
}

/// WeChat iLink bot configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatConfig {
    /// Whether the WeChat worker should start.
    #[serde(default)]
    pub enabled: bool,

    /// Agent used for new WeChat-created sessions. When absent, the bridge
    /// resolves to the first enabled runtime agent from the local store.
    #[serde(default)]
    pub agent: Option<Agent>,

    /// Model used for new WeChat-created sessions.
    #[serde(default)]
    pub model: Option<String>,

    /// Effort used for new WeChat-created sessions.
    #[serde(default)]
    pub effort: Option<String>,

    /// Default workspace for WeChat-created sessions when a user-specific
    /// binding is absent.
    #[serde(default, alias = "default_workspace")]
    pub default_workspace: Option<String>,

    /// Workspaces WeChat users are allowed to open sessions in.
    #[serde(default, alias = "allowed_workspaces")]
    pub allowed_workspaces: Vec<String>,

    /// Optional per-WeChat-user workspace overrides.
    #[serde(default, alias = "workspace_bindings")]
    pub workspace_bindings: Vec<WorkspaceBindingConfig>,

    /// iLink bot token acquired by QR login.
    #[serde(default, alias = "bot_token")]
    pub bot_token: String,

    /// Optional bot/account identifiers returned by QR login.
    #[serde(default, alias = "bot_id")]
    pub bot_id: Option<String>,
    #[serde(default, alias = "user_id")]
    pub user_id: Option<String>,

    /// iLink API base. Defaults to `https://ilinkai.weixin.qq.com`.
    #[serde(default, alias = "base_url")]
    pub base_url: Option<String>,

    /// Long-poll timeout in seconds for `getupdates`.
    #[serde(default = "default_wechat_poll_timeout", alias = "poll_timeout_secs")]
    pub poll_timeout_secs: u64,
}

fn default_poll_timeout() -> u64 {
    30
}

fn default_idle_timeout_secs() -> u64 {
    15 * 60
}

fn default_wechat_poll_timeout() -> u64 {
    40
}

impl Default for ImBridgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_secs: default_idle_timeout_secs(),
            telegram: Some(TelegramConfig::default()),
            discord: Some(DiscordConfig::default()),
            feishu: Some(FeishuConfig::default()),
            wechat: Some(WechatConfig::default()),
        }
    }
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            agent: None,
            model: None,
            effort: None,
            default_workspace: None,
            allowed_workspaces: Vec::new(),
            workspace_bindings: Vec::new(),
            bot_token: String::new(),
            allowed_user_ids: Vec::new(),
            poll_timeout_secs: default_poll_timeout(),
            api_base: None,
        }
    }
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            agent: None,
            model: None,
            effort: None,
            default_workspace: None,
            allowed_workspaces: Vec::new(),
            workspace_bindings: Vec::new(),
            bot_token: String::new(),
            allowed_server_ids: Vec::new(),
            allowed_channel_ids: Vec::new(),
            mention_only: true,
            api_base: None,
            gateway_url: None,
        }
    }
}

impl Default for WechatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            agent: None,
            model: None,
            effort: None,
            default_workspace: None,
            allowed_workspaces: Vec::new(),
            workspace_bindings: Vec::new(),
            bot_token: String::new(),
            bot_id: None,
            user_id: None,
            base_url: None,
            poll_timeout_secs: default_wechat_poll_timeout(),
        }
    }
}

impl ImBridgeConfig {
    /// Configured agent for a platform, if the platform has one.
    pub fn configured_agent_for_platform(&self, platform: &str) -> Option<Agent> {
        match platform {
            "telegram" => self.telegram.as_ref().and_then(|config| config.agent),
            "discord" => self.discord.as_ref().and_then(|config| config.agent),
            "feishu" => self.feishu.as_ref().and_then(|config| config.agent),
            "wechat" => self.wechat.as_ref().and_then(|config| config.agent),
            _ => None,
        }
    }

    /// Whether `path` is permitted for a new session on `platform`.
    pub fn is_workspace_allowed(&self, platform: &str, path: &str) -> bool {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .map(|config| config.is_workspace_allowed(path))
                .unwrap_or(false),
            "discord" => self
                .discord
                .as_ref()
                .map(|config| config.is_workspace_allowed(path))
                .unwrap_or(false),
            "feishu" => self
                .feishu
                .as_ref()
                .map(|config| config.is_workspace_allowed(path))
                .unwrap_or(false),
            "wechat" => self
                .wechat
                .as_ref()
                .map(|config| config.is_workspace_allowed(path))
                .unwrap_or(false),
            _ => false,
        }
    }

    /// Workspace selected for a specific chat. Falls back to the platform
    /// default.
    pub fn workspace_for_chat(&self, platform: &str, chat_id: &str) -> Option<&str> {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .and_then(|config| config.workspace_for_chat(platform, chat_id)),
            "discord" => self
                .discord
                .as_ref()
                .and_then(|config| config.workspace_for_chat(platform, chat_id)),
            "feishu" => self
                .feishu
                .as_ref()
                .and_then(|config| config.workspace_for_chat(platform, chat_id)),
            "wechat" => self
                .wechat
                .as_ref()
                .and_then(|config| config.workspace_for_chat(platform, chat_id)),
            _ => None,
        }
    }

    /// Workspaces that can be selected interactively for a platform/chat.
    pub fn workspace_choices_for_chat(&self, platform: &str, chat_id: &str) -> Vec<&str> {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .map(|config| config.workspace_choices_for_chat(platform, chat_id))
                .unwrap_or_default(),
            "discord" => self
                .discord
                .as_ref()
                .map(|config| config.workspace_choices_for_chat(platform, chat_id))
                .unwrap_or_default(),
            "feishu" => self
                .feishu
                .as_ref()
                .map(|config| config.workspace_choices_for_chat(platform, chat_id))
                .unwrap_or_default(),
            "wechat" => self
                .wechat
                .as_ref()
                .map(|config| config.workspace_choices_for_chat(platform, chat_id))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Optional model override for sessions opened by a given platform.
    pub fn model_for_platform(&self, platform: &str) -> Option<&str> {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .and_then(|config| config.model.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            "discord" => self
                .discord
                .as_ref()
                .and_then(|config| config.model.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            "feishu" => self
                .feishu
                .as_ref()
                .and_then(|config| config.model.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            "wechat" => self
                .wechat
                .as_ref()
                .and_then(|config| config.model.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            _ => None,
        }
    }

    /// Optional effort override for sessions opened by a given platform.
    pub fn effort_for_platform(&self, platform: &str) -> Option<&str> {
        match platform {
            "telegram" => self
                .telegram
                .as_ref()
                .and_then(|config| config.effort.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            "discord" => self
                .discord
                .as_ref()
                .and_then(|config| config.effort.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            "feishu" => self
                .feishu
                .as_ref()
                .and_then(|config| config.effort.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            "wechat" => self
                .wechat
                .as_ref()
                .and_then(|config| config.effort.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            _ => None,
        }
    }
}

impl TelegramConfig {
    /// The default workspace for Telegram chat-initiated sessions, if any.
    pub fn default_workspace(&self) -> Option<&str> {
        self.default_workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.allowed_workspaces.first().map(String::as_str))
    }

    /// Whether `path` is permitted for a new Telegram session. A workspace is
    /// allowed when it is the default, listed explicitly, or bound to a chat.
    pub fn is_workspace_allowed(&self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        self.allowed_workspaces.iter().any(|w| w.trim() == path)
            || self.default_workspace() == Some(path)
            || self
                .workspace_bindings
                .iter()
                .any(|binding| binding.workspace_path.trim() == path)
    }

    /// Workspace selected for a specific Telegram chat. Falls back to the
    /// Telegram default.
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

    pub fn workspace_choices_for_chat(&self, platform: &str, chat_id: &str) -> Vec<&str> {
        workspace_choices_from_config(
            platform,
            chat_id,
            self.default_workspace(),
            &self.allowed_workspaces,
            &self.workspace_bindings,
        )
    }
}

impl DiscordConfig {
    /// The default workspace for Discord chat-initiated sessions, if any.
    pub fn default_workspace(&self) -> Option<&str> {
        self.default_workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.allowed_workspaces.first().map(String::as_str))
    }

    /// Whether `path` is permitted for a new Discord session.
    pub fn is_workspace_allowed(&self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        self.allowed_workspaces.iter().any(|w| w.trim() == path)
            || self.default_workspace() == Some(path)
            || self
                .workspace_bindings
                .iter()
                .any(|binding| binding.workspace_path.trim() == path)
    }

    /// Workspace selected for a specific Discord channel. Falls back to the
    /// Discord default.
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

    pub fn workspace_choices_for_chat(&self, platform: &str, chat_id: &str) -> Vec<&str> {
        workspace_choices_from_config(
            platform,
            chat_id,
            self.default_workspace(),
            &self.allowed_workspaces,
            &self.workspace_bindings,
        )
    }
}

impl FeishuConfig {
    /// The default workspace for Feishu chat-initiated sessions, if any.
    pub fn default_workspace(&self) -> Option<&str> {
        self.default_workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.allowed_workspaces.first().map(String::as_str))
    }

    /// Whether `path` is permitted for a new Feishu session.
    pub fn is_workspace_allowed(&self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        self.allowed_workspaces.iter().any(|w| w.trim() == path)
            || self.default_workspace() == Some(path)
            || self
                .workspace_bindings
                .iter()
                .any(|binding| binding.workspace_path.trim() == path)
    }

    /// Workspace selected for a specific Feishu chat. Falls back to the Feishu
    /// default.
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

    pub fn workspace_choices_for_chat(&self, platform: &str, chat_id: &str) -> Vec<&str> {
        workspace_choices_from_config(
            platform,
            chat_id,
            self.default_workspace(),
            &self.allowed_workspaces,
            &self.workspace_bindings,
        )
    }
}

impl WechatConfig {
    /// The default workspace for WeChat chat-initiated sessions, if any.
    pub fn default_workspace(&self) -> Option<&str> {
        self.default_workspace
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| self.allowed_workspaces.first().map(String::as_str))
    }

    /// Whether `path` is permitted for a new WeChat session.
    pub fn is_workspace_allowed(&self, path: &str) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return false;
        }
        self.allowed_workspaces.iter().any(|w| w.trim() == path)
            || self.default_workspace() == Some(path)
            || self
                .workspace_bindings
                .iter()
                .any(|binding| binding.workspace_path.trim() == path)
    }

    /// Workspace selected for a specific WeChat user. Falls back to the WeChat
    /// default.
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

    pub fn workspace_choices_for_chat(&self, platform: &str, chat_id: &str) -> Vec<&str> {
        workspace_choices_from_config(
            platform,
            chat_id,
            self.default_workspace(),
            &self.allowed_workspaces,
            &self.workspace_bindings,
        )
    }
}

fn workspace_choices_from_config<'a>(
    platform: &str,
    chat_id: &str,
    default_workspace: Option<&'a str>,
    allowed_workspaces: &'a [String],
    workspace_bindings: &'a [WorkspaceBindingConfig],
) -> Vec<&'a str> {
    let mut values = Vec::new();
    if let Some(workspace) = default_workspace {
        push_unique_ref(&mut values, workspace);
    }
    for workspace in allowed_workspaces {
        push_unique_ref(&mut values, workspace);
    }
    for binding in workspace_bindings {
        if binding.platform == platform && binding.chat_id.trim() == chat_id.trim() {
            push_unique_ref(&mut values, &binding.workspace_path);
        }
    }
    values
}

fn push_unique_ref<'a>(values: &mut Vec<&'a str>, value: &'a str) {
    let value = value.trim();
    if !value.is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

/// Path to the bridge config file under the current app home.
fn config_path() -> Result<PathBuf> {
    app_paths::im_bridge_config_path()
}

/// Best-effort display string for the config path, for log messages.
pub fn config_path_display() -> String {
    config_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| format!("{}/im-bridge.yaml", app_paths::app_home_display()))
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
