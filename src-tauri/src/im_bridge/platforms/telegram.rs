//! Telegram Bot API platform implementation.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::models::{Agent, AgentInfo, RuntimeAgentOptionMetadata};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::{Client, ClientBuilder};
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::config::TelegramConfig;
use super::super::router;
use super::super::state::{
    ChannelContext, ChatKey, ChatPermissionRequest, ChatSink, ImBridgeState,
};

const PLATFORM: &str = "telegram";
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const CALLBACK_PREFIX: &str = "sessio_perm:";
const ACTION_CALLBACK_PREFIX: &str = "sessio:";
const TELEGRAM_TEXT_LIMIT: usize = 3900;

pub fn spawn(state: Arc<ImBridgeState>) {
    if let Err(error) = thread::Builder::new()
        .name("im-bridge-telegram".to_string())
        .spawn(move || poll_loop(state))
    {
        log::warn!("[im-bridge:telegram] failed to spawn worker: {error:#}");
    }
}

pub fn detect_user_ids(bot_token: &str, api_base: Option<&str>) -> Result<Vec<i64>> {
    let sink = TelegramSink::for_token(bot_token, api_base, 30)?;
    let updates = sink.get_updates(None, 0)?;
    let mut ids = Vec::new();
    for update in updates {
        if let Some(user_id) = update.user_id() {
            if !ids.contains(&user_id) {
                ids.push(user_id);
            }
        }
    }
    Ok(ids)
}

pub fn test_connection(bot_token: &str, api_base: Option<&str>) -> Result<()> {
    let sink = TelegramSink::for_token(bot_token, api_base, 30)?;
    sink.get_me().map(|_| ())
}

fn poll_loop(state: Arc<ImBridgeState>) {
    let mut offset: Option<i64> = None;
    let mut active_key: Option<TelegramConnectionKey> = None;
    let mut sink: Option<Arc<TelegramSink>> = None;

    loop {
        let bridge_config = state.config_snapshot();
        let Some(config) = bridge_config.telegram.clone() else {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                offset = None;
                log::info!("[im-bridge:telegram] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        if !bridge_config.enabled || !config.enabled || config.bot_token.trim().is_empty() {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                offset = None;
                log::info!("[im-bridge:telegram] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let next_key = TelegramConnectionKey::from_config(&config);
        if active_key.as_ref() != Some(&next_key) {
            match TelegramSink::new(config.clone()) {
                Ok(next_sink) => {
                    let next_sink = Arc::new(next_sink);
                    if let Err(error) = next_sink.set_commands() {
                        log::warn!("[im-bridge:telegram] failed to set bot commands: {error:#}");
                    }
                    state.register_sink(next_sink.clone());
                    sink = Some(next_sink);
                    active_key = Some(next_key);
                    offset = None;
                    log::info!("[im-bridge:telegram] connected");
                }
                Err(error) => {
                    state.unregister_sink(PLATFORM);
                    sink = None;
                    active_key = None;
                    offset = None;
                    log::warn!("[im-bridge:telegram] failed to initialize: {error:#}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }

        let Some(sink) = sink.as_ref().cloned() else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        match sink.get_updates(offset, config.poll_timeout_secs) {
            Ok(updates) => {
                for update in updates {
                    offset = Some(update.update_id + 1);
                    handle_update(&state, &sink, &config, update);
                }
            }
            Err(error) => {
                log::warn!("[im-bridge:telegram] polling failed: {error:#}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramConnectionKey {
    bot_token: String,
    api_base: Option<String>,
    poll_timeout_secs: u64,
}

impl TelegramConnectionKey {
    fn from_config(config: &TelegramConfig) -> Self {
        Self {
            bot_token: config.bot_token.trim().to_string(),
            api_base: config
                .api_base
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            poll_timeout_secs: config.poll_timeout_secs,
        }
    }
}

fn handle_update(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    config: &TelegramConfig,
    update: TelegramUpdate,
) {
    if let Some(callback) = update.callback_query {
        handle_callback(state, sink, config, callback);
        return;
    }

    let Some(message) = update.message else {
        return;
    };
    let Some(text) = message.text.as_deref() else {
        return;
    };
    let Some(from) = message.from.as_ref() else {
        return;
    };
    if !is_allowed(config, from.id) {
        log::warn!(
            "[im-bridge:telegram] rejected message from unauthorized user {}",
            from.id
        );
        return;
    }

    let chat_id = message.chat.id.to_string();
    if is_duplicate_update(state, &chat_id, update.update_id) {
        log::info!(
            "[im-bridge:telegram] skipped duplicate update {} for chat {}",
            update.update_id,
            chat_id
        );
        return;
    }

    let key = ChatKey::new(PLATFORM, chat_id.clone());
    state.remember_channel_context(
        key.clone(),
        telegram_channel_context(&message, from, update.update_id),
    );
    if let Some(command_reply) = handle_interactive_command(state, sink, &key, text) {
        if let Err(error) = command_reply {
            if let Err(send_error) = sink.send_text(&chat_id, &format!("⚠️ {error:#}")) {
                log::warn!(
                    "[im-bridge:telegram] failed to send interactive command error: {send_error:#}"
                );
            }
        }
        record_update_activity(state, &chat_id, update.update_id);
        return;
    }
    let outcome = router::handle_message(state, &key, text);
    if let Some(reply) = outcome.reply {
        if let Err(error) = sink.send_text(&chat_id, &reply) {
            log::warn!("[im-bridge:telegram] failed to send command reply: {error:#}");
        }
    }
    record_update_activity(state, &chat_id, update.update_id);
}

fn record_update_activity(state: &Arc<ImBridgeState>, chat_id: &str, update_id: i64) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, chat_id, Some(update_id), now_ms())
    {
        log::warn!("[im-bridge:telegram] failed to record update id: {error:#}");
    }
}

fn handle_interactive_command(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
    text: &str,
) -> Option<Result<()>> {
    let trimmed = text.trim();
    let command_text = trimmed.strip_prefix('/')?;
    let mut parts = command_text.splitn(2, char::is_whitespace);
    let command = parts
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let arg = parts.next().map(str::trim).unwrap_or("");
    let result = match command.as_str() {
        "agent" if arg.is_empty() => send_agent_menu(state, sink, key),
        "model" => send_model_menu(state, sink, key),
        "effort" => send_effort_menu(state, sink, key),
        "workspace" => send_workspace_menu(state, sink, key),
        _ => return None,
    };
    Some(result)
}

fn send_agent_menu(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
) -> Result<()> {
    let agents = available_agents(state)?;
    if agents.is_empty() {
        bail!("no agents configured");
    }
    let rows = agents
        .into_iter()
        .filter_map(|agent_info| {
            let agent = Agent::from_db_str(&agent_info.id)?;
            Some(vec![json!({
                "text": agent_info.display_name,
                "callback_data": format!("{ACTION_CALLBACK_PREFIX}agent:{}", agent.as_str()),
            })])
        })
        .collect::<Vec<_>>();
    sink.send_message(
        &key.chat_id,
        "选择 agent。切换到不同 agent 会开启新会话；选择当前 agent 不会新建。",
        Some(json!({ "inline_keyboard": rows })),
    )
}

fn send_model_menu(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
) -> Result<()> {
    let session = router::ensure_chat_session(state, key)?;
    let agent_info = agent_info(state, session.agent)?;
    let choices = option_choices(&agent_info.models, agent_info.model.as_deref());
    if choices.is_empty() {
        bail!("{} has no model options", agent_info.display_name);
    }
    let rows = choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            vec![json!({
                "text": choice.label,
                "callback_data": format!(
                    "{ACTION_CALLBACK_PREFIX}model:{}:{index}",
                    session.agent.as_str()
                ),
            })]
        })
        .collect::<Vec<_>>();
    sink.send_message(
        &key.chat_id,
        &format!(
            "选择 {} 的 model。不会新建 session。",
            agent_info.display_name
        ),
        Some(json!({ "inline_keyboard": rows })),
    )
}

fn send_effort_menu(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
) -> Result<()> {
    let session = router::ensure_chat_session(state, key)?;
    let agent_info = agent_info(state, session.agent)?;
    let choices = option_choices(&agent_info.efforts, agent_info.effort.as_deref());
    if choices.is_empty() {
        bail!("{} has no effort options", agent_info.display_name);
    }
    let rows = choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            vec![json!({
                "text": choice.label,
                "callback_data": format!(
                    "{ACTION_CALLBACK_PREFIX}effort:{}:{index}",
                    session.agent.as_str()
                ),
            })]
        })
        .collect::<Vec<_>>();
    sink.send_message(
        &key.chat_id,
        &format!(
            "选择 {} 的 effort。不会新建 session。",
            agent_info.display_name
        ),
        Some(json!({ "inline_keyboard": rows })),
    )
}

fn send_workspace_menu(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
) -> Result<()> {
    let choices = workspace_choices(state, key);
    if choices.is_empty() {
        bail!("no allowed workspaces configured");
    }
    let rows = choices
        .iter()
        .enumerate()
        .map(|(index, choice)| {
            vec![json!({
                "text": choice.label,
                "callback_data": format!("{ACTION_CALLBACK_PREFIX}workspace:{index}"),
            })]
        })
        .collect::<Vec<_>>();
    sink.send_message(
        &key.chat_id,
        "选择当前会话的 workspace。会开启同 agent 的新 runtime session，不会修改默认 workspace。",
        Some(json!({ "inline_keyboard": rows })),
    )
}

fn available_agents(state: &Arc<ImBridgeState>) -> Result<Vec<AgentInfo>> {
    Ok(state
        .store
        .list_agents()?
        .into_iter()
        .filter(|agent| agent.enabled && Agent::from_db_str(&agent.id).is_some())
        .collect())
}

fn agent_info(state: &Arc<ImBridgeState>, agent: Agent) -> Result<AgentInfo> {
    state
        .store
        .list_agents()?
        .into_iter()
        .find(|agent_info| agent_info.id == agent.as_str())
        .with_context(|| format!("agent not configured: {}", agent.as_str()))
}

#[derive(Debug, Clone)]
struct OptionChoice {
    value: String,
    label: String,
}

fn option_choices(
    options: &[RuntimeAgentOptionMetadata],
    fallback: Option<&str>,
) -> Vec<OptionChoice> {
    let mut choices = options
        .iter()
        .filter(|option| option.enabled)
        .map(|option| OptionChoice {
            value: option.value.clone(),
            label: if option.display_name.trim().is_empty() {
                option.label.clone()
            } else {
                option.display_name.clone()
            },
        })
        .collect::<Vec<_>>();
    if choices.is_empty() {
        if let Some(value) = fallback.map(str::trim).filter(|value| !value.is_empty()) {
            choices.push(OptionChoice {
                value: value.to_string(),
                label: value.to_string(),
            });
        }
    }
    choices
}

#[derive(Debug, Clone)]
struct WorkspaceChoice {
    path: String,
    label: String,
}

fn workspace_choices(state: &Arc<ImBridgeState>, key: &ChatKey) -> Vec<WorkspaceChoice> {
    let config = state.config_snapshot();
    let projects = state.store.list_projects().unwrap_or_default();
    let mut paths = Vec::<String>::new();
    if let Some(workspace) = config.default_workspace() {
        push_unique(&mut paths, workspace);
    }
    for workspace in &config.allowed_workspaces {
        push_unique(&mut paths, workspace);
    }
    for binding in &config.workspace_bindings {
        if binding.platform == key.platform && binding.chat_id.trim() == key.chat_id {
            push_unique(&mut paths, &binding.workspace_path);
        }
    }
    if let Some(session) = state.chat_session(key) {
        push_unique(&mut paths, &session.workspace_path);
    }
    paths
        .into_iter()
        .filter(|path| config.is_workspace_allowed(path))
        .map(|path| {
            let label = projects
                .iter()
                .find(|project| project.path == path)
                .map(|project| project.name.clone())
                .unwrap_or_else(|| workspace_label(&path));
            WorkspaceChoice { path, label }
        })
        .collect()
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn workspace_label(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn telegram_channel_context(
    message: &TelegramMessage,
    from: &TelegramUser,
    update_id: i64,
) -> ChannelContext {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "telegramChat".to_string(),
        json!({
            "id": message.chat.id,
            "type": message.chat.kind.as_deref(),
            "title": message.chat.title.as_deref(),
            "username": message.chat.username.as_deref(),
            "firstName": message.chat.first_name.as_deref(),
            "lastName": message.chat.last_name.as_deref(),
        }),
    );
    metadata.insert(
        "telegramUser".to_string(),
        json!({
            "id": from.id,
            "username": from.username.as_deref(),
            "firstName": from.first_name.as_deref(),
            "lastName": from.last_name.as_deref(),
        }),
    );
    ChannelContext {
        channel_type: message.chat.kind.clone(),
        user_id: Some(from.id.to_string()),
        team_id: None,
        thread_id: None,
        display_name: telegram_chat_display_name(&message.chat)
            .or_else(|| telegram_user_display_name(from)),
        metadata,
        last_update_id: Some(update_id),
    }
}

fn telegram_chat_display_name(chat: &TelegramChat) -> Option<String> {
    trimmed(&chat.title)
        .map(str::to_string)
        .or_else(|| person_display_name(&chat.first_name, &chat.last_name, &chat.username))
}

fn telegram_user_display_name(user: &TelegramUser) -> Option<String> {
    person_display_name(&user.first_name, &user.last_name, &user.username)
}

fn person_display_name(
    first_name: &Option<String>,
    last_name: &Option<String>,
    username: &Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(first_name) = trimmed(first_name) {
        parts.push(first_name);
    }
    if let Some(last_name) = trimmed(last_name) {
        parts.push(last_name);
    }
    if !parts.is_empty() {
        return Some(parts.join(" "));
    }
    trimmed(username).map(|value| format!("@{}", value.trim_start_matches('@')))
}

fn trimmed(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_duplicate_update(state: &Arc<ImBridgeState>, chat_id: &str, update_id: i64) -> bool {
    match state.store.get_active_channel_session(PLATFORM, chat_id) {
        Ok(Some(record)) => record
            .last_update_id
            .map(|last_update_id| update_id <= last_update_id)
            .unwrap_or(false),
        Ok(None) => false,
        Err(error) => {
            log::warn!("[im-bridge:telegram] failed to read last update id: {error:#}");
            false
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn handle_callback(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    config: &TelegramConfig,
    callback: TelegramCallbackQuery,
) {
    if !is_allowed(config, callback.from.id) {
        let _ = sink.answer_callback_query(&callback.id, "Not authorized");
        return;
    }
    let Some(data) = callback.data.as_deref() else {
        let _ = sink.answer_callback_query(&callback.id, "Missing callback data");
        return;
    };
    if let Some(action) = data.strip_prefix(ACTION_CALLBACK_PREFIX) {
        let chat_id = callback
            .message
            .as_ref()
            .map(|message| message.chat.id.to_string());
        match chat_id {
            Some(chat_id) => {
                let key = ChatKey::new(PLATFORM, chat_id.clone());
                state.touch_chat(&key);
                record_chat_activity(state, &chat_id);
                match handle_action_callback(state, &chat_id, action) {
                    Ok(message) => {
                        let _ = sink.answer_callback_query(&callback.id, &message);
                    }
                    Err(error) => {
                        let _ = sink.answer_callback_query(&callback.id, "Failed");
                        if let Err(send_error) = sink.send_text(&chat_id, &format!("⚠️ {error:#}"))
                        {
                            log::warn!(
                                "[im-bridge:telegram] failed to send action callback error: {send_error:#}"
                            );
                        }
                    }
                }
            }
            None => {
                let _ = sink.answer_callback_query(&callback.id, "Missing chat");
            }
        }
        return;
    }
    let Some(token) = data.strip_prefix(CALLBACK_PREFIX) else {
        return;
    };
    let Some(decision) = state.take_permission_token(token) else {
        let _ = sink.answer_callback_query(&callback.id, "Permission request expired");
        return;
    };
    if let Some(chat_id) = callback
        .message
        .as_ref()
        .map(|message| message.chat.id.to_string())
    {
        record_chat_activity(state, &chat_id);
        state.touch_chat(&ChatKey::new(PLATFORM, chat_id));
    }
    match state.runtime.respond_permission(
        &decision.sessio_runtime_session_id,
        &decision.request_id,
        decision.option_id,
    ) {
        Ok(()) => {
            let _ = sink.answer_callback_query(&callback.id, "Recorded");
        }
        Err(error) => {
            let _ = sink.answer_callback_query(&callback.id, "Failed");
            log::warn!("[im-bridge:telegram] permission response failed: {error:#}");
        }
    }
}

fn record_chat_activity(state: &Arc<ImBridgeState>, chat_id: &str) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, chat_id, None, now_ms())
    {
        log::warn!("[im-bridge:telegram] failed to record chat activity: {error:#}");
    }
}

fn handle_action_callback(
    state: &Arc<ImBridgeState>,
    chat_id: &str,
    action: &str,
) -> Result<String> {
    let key = ChatKey::new(PLATFORM, chat_id.to_string());
    let mut parts = action.split(':');
    match parts.next().unwrap_or("") {
        "agent" => {
            let agent = parts.next().context("missing agent")?;
            router::switch_agent(state, &key, agent)?;
            Ok(format!("Agent: {agent}"))
        }
        "model" => {
            let agent = parse_action_agent(parts.next())?;
            let index = parse_action_index(parts.next())?;
            ensure_current_agent(state, &key, agent)?;
            let agent_info = agent_info(state, agent)?;
            let choices = option_choices(&agent_info.models, agent_info.model.as_deref());
            let choice = choices
                .get(index)
                .with_context(|| "model menu expired; open /model again")?;
            router::set_session_config_option(
                state,
                &key,
                "model",
                serde_json::Value::String(choice.value.clone()),
            )?;
            Ok(format!("Model: {}", choice.label))
        }
        "effort" => {
            let agent = parse_action_agent(parts.next())?;
            let index = parse_action_index(parts.next())?;
            ensure_current_agent(state, &key, agent)?;
            let agent_info = agent_info(state, agent)?;
            let choices = option_choices(&agent_info.efforts, agent_info.effort.as_deref());
            let choice = choices
                .get(index)
                .with_context(|| "effort menu expired; open /effort again")?;
            router::set_session_config_option(
                state,
                &key,
                effort_config_id(agent),
                serde_json::Value::String(choice.value.clone()),
            )?;
            Ok(format!("Effort: {}", choice.label))
        }
        "workspace" => {
            let index = parse_action_index(parts.next())?;
            let choices = workspace_choices(state, &key);
            let choice = choices
                .get(index)
                .with_context(|| "workspace menu expired; open /workspace again")?;
            router::start_new_session(state, &key, Some(&choice.path))?;
            Ok(format!("Workspace: {}", choice.label))
        }
        other => bail!("unknown action: {other}"),
    }
}

fn parse_action_agent(value: Option<&str>) -> Result<Agent> {
    let value = value.context("missing agent")?;
    Agent::from_db_str(value).with_context(|| format!("unknown agent: {value}"))
}

fn parse_action_index(value: Option<&str>) -> Result<usize> {
    value
        .context("missing index")?
        .parse::<usize>()
        .context("invalid index")
}

fn ensure_current_agent(state: &Arc<ImBridgeState>, key: &ChatKey, agent: Agent) -> Result<()> {
    let session = router::ensure_chat_session(state, key)?;
    if session.agent != agent {
        bail!("agent changed; open the menu again");
    }
    Ok(())
}

fn effort_config_id(agent: Agent) -> &'static str {
    match agent {
        Agent::Codex => "reasoning_effort",
        Agent::AstraPi | Agent::Claude | Agent::Gemini => "effort",
    }
}

fn is_allowed(config: &TelegramConfig, user_id: i64) -> bool {
    config
        .allowed_user_ids
        .iter()
        .any(|allowed| *allowed == user_id)
}

pub struct TelegramSink {
    client: Client,
    api_base: String,
    bot_token: String,
}

impl TelegramSink {
    fn new(config: TelegramConfig) -> Result<Self> {
        Self::for_token(
            &config.bot_token,
            config.api_base.as_deref(),
            config.poll_timeout_secs,
        )
    }

    fn for_token(bot_token: &str, api_base: Option<&str>, poll_timeout_secs: u64) -> Result<Self> {
        let bot_token = bot_token.trim();
        if bot_token.is_empty() {
            bail!("Telegram bot token is required");
        }
        let timeout = Duration::from_secs(poll_timeout_secs.max(5) + 20);
        let client = ClientBuilder::new()
            .timeout(timeout)
            .build()
            .context("build Telegram HTTP client")?;
        Ok(Self {
            client,
            api_base: api_base
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_API_BASE)
                .trim_end_matches('/')
                .to_string(),
            bot_token: bot_token.to_string(),
        })
    }

    fn endpoint(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api_base, self.bot_token, method)
    }

    fn get_updates(&self, offset: Option<i64>, timeout_secs: u64) -> Result<Vec<TelegramUpdate>> {
        let body = json!({
            "offset": offset,
            "timeout": timeout_secs,
            "allowed_updates": ["message", "callback_query"],
        });
        let response: TelegramApiResponse<Vec<TelegramUpdate>> =
            self.post_json("getUpdates", &body)?;
        response.into_result()
    }

    fn get_me(&self) -> Result<Value> {
        let response: TelegramApiResponse<Value> = self.post_json("getMe", &json!({}))?;
        response.into_result()
    }

    fn set_commands(&self) -> Result<()> {
        let body = json!({
            "commands": [
                { "command": "new", "description": "Start a new Sessio session" },
                { "command": "agent", "description": "Choose agent" },
                { "command": "model", "description": "Choose model for current session" },
                { "command": "effort", "description": "Choose effort for current session" },
                { "command": "workspace", "description": "Choose workspace for current session" },
                { "command": "status", "description": "Show current Sessio session" },
                { "command": "cancel", "description": "Cancel current turn" },
                { "command": "end", "description": "End current IM session" },
                { "command": "help", "description": "Show help" }
            ]
        });
        let response: TelegramApiResponse<Value> = self.post_json("setMyCommands", &body)?;
        response.into_result().map(|_| ())
    }

    fn send_message(&self, chat_id: &str, text: &str, reply_markup: Option<Value>) -> Result<()> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        if let Some(reply_markup) = reply_markup {
            body["reply_markup"] = reply_markup;
        }
        let response: TelegramApiResponse<Value> = self.post_json("sendMessage", &body)?;
        response.into_result().map(|_| ())
    }

    fn answer_callback_query(&self, callback_query_id: &str, text: &str) -> Result<()> {
        let body = json!({
            "callback_query_id": callback_query_id,
            "text": text,
            "show_alert": false,
        });
        let response: TelegramApiResponse<Value> = self.post_json("answerCallbackQuery", &body)?;
        response.into_result().map(|_| ())
    }

    fn post_json<T: for<'de> Deserialize<'de>>(&self, method: &str, body: &Value) -> Result<T> {
        self.client
            .post(self.endpoint(method))
            .json(body)
            .send()
            .with_context(|| format!("Telegram {method} request failed"))?
            .error_for_status()
            .with_context(|| format!("Telegram {method} returned HTTP error"))?
            .json::<T>()
            .with_context(|| format!("parse Telegram {method} response"))
    }
}

impl ChatSink for TelegramSink {
    fn platform(&self) -> &'static str {
        PLATFORM
    }

    fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        for chunk in split_telegram_text(text) {
            self.send_message(chat_id, &chunk, None)?;
        }
        Ok(())
    }

    fn send_permission_request(
        &self,
        chat_id: &str,
        request: &ChatPermissionRequest,
    ) -> Result<()> {
        let mut text = format!("Permission requested\nTool: {}", request.tool_name);
        if let Some(input) = &request.input_summary {
            if !input.trim().is_empty() {
                text.push_str("\n\nInput:\n");
                text.push_str(input);
            }
        }
        let buttons: Vec<Value> = request
            .options
            .iter()
            .map(|option| {
                json!({
                    "text": option.label,
                    "callback_data": format!("{}{}", CALLBACK_PREFIX, option.token),
                })
            })
            .collect();
        let reply_markup = if buttons.is_empty() {
            None
        } else {
            Some(json!({ "inline_keyboard": [buttons] }))
        };
        self.send_message(chat_id, &text, reply_markup)
    }
}

fn split_telegram_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= TELEGRAM_TEXT_LIMIT {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

impl<T> TelegramApiResponse<T> {
    fn into_result(self) -> Result<T> {
        if self.ok {
            self.result
                .ok_or_else(|| anyhow!("Telegram response missing result"))
        } else {
            bail!(
                "{}",
                self.description
                    .unwrap_or_else(|| "Telegram API returned ok=false".to_string())
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    callback_query: Option<TelegramCallbackQuery>,
}

impl TelegramUpdate {
    fn user_id(&self) -> Option<i64> {
        self.message
            .as_ref()
            .and_then(|message| message.from.as_ref())
            .map(|user| user.id)
            .or_else(|| self.callback_query.as_ref().map(|query| query.from.id))
    }
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    chat: TelegramChat,
    from: Option<TelegramUser>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    kind: Option<String>,
    title: Option<String>,
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackQuery {
    id: String,
    from: TelegramUser,
    message: Option<TelegramCallbackMessage>,
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackMessage {
    chat: TelegramChat,
}
