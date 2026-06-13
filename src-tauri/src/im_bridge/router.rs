//! Inbound routing: turn an incoming chat message into a runtime action.
//!
//! A message is either a slash command (`/new`, `/agent`, `/cancel`, ...) or a
//! plain prompt. Commands manage the chat-to-session binding; prompts are sent
//! to the bound session's agent. All platform listeners funnel through
//! [`handle_message`] so command semantics stay identical everywhere.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};

use crate::agents::runtime::types::{
    AgentInput, AgentSessionConfigChange, EnsureAgentRuntimeSession, StartAgentSession,
};
use crate::models::Agent;
use crate::store::ChannelSessionRecord;

use super::state::{ChatKey, ChatSession, ImBridgeState};

/// How long to wait for a freshly started session to leave `Starting` before
/// sending the first prompt. ACP agents spawn a child process, so allow time.
const SESSION_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

/// Result of handling one inbound message. The caller (platform listener)
/// decides how to surface `reply` — for Telegram it becomes a sendMessage.
pub struct HandleOutcome {
    /// Immediate text reply to post back to the chat, if any. Agent output does
    /// NOT come back here; it arrives asynchronously via the outbound pump.
    pub reply: Option<String>,
}

enum PromptDispatchOutcome {
    Sent,
    Queued(usize),
}

impl HandleOutcome {
    fn reply(text: impl Into<String>) -> Self {
        Self {
            reply: Some(text.into()),
        }
    }

    fn silent() -> Self {
        Self { reply: None }
    }
}

/// Entry point for an authenticated inbound message. `text` is the raw message
/// body; `key` identifies the originating chat.
pub fn handle_message(state: &Arc<ImBridgeState>, key: &ChatKey, text: &str) -> HandleOutcome {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return HandleOutcome::silent();
    }

    if let Some(rest) = trimmed.strip_prefix('/') {
        return handle_command(state, key, rest);
    }

    match dispatch_prompt(state, key, trimmed) {
        Ok(PromptDispatchOutcome::Sent) => HandleOutcome::silent(),
        Ok(PromptDispatchOutcome::Queued(position)) => HandleOutcome::reply(format!(
            "上一轮还在处理中，已将这条消息加入队列（第 {position} 条）。\n发送 /cancel 可取消当前回合。"
        )),
        Err(error) => HandleOutcome::reply(format!("⚠️ {error:#}")),
    }
}

/// Parse and execute a slash command. `rest` is everything after the leading
/// `/`. Unknown commands return usage help.
fn handle_command(state: &Arc<ImBridgeState>, key: &ChatKey, rest: &str) -> HandleOutcome {
    state.touch_chat(key);
    let mut parts = rest.splitn(2, char::is_whitespace);
    let cmd = parts.next().unwrap_or("").to_ascii_lowercase();
    let arg = parts.next().map(str::trim).unwrap_or("");

    match cmd.as_str() {
        "help" | "start" => HandleOutcome::reply(help_text()),
        "status" => HandleOutcome::reply(session_status_text(state, key)),
        "new" => match start_new_session(state, key, if arg.is_empty() { None } else { Some(arg) })
        {
            Ok(msg) => HandleOutcome::reply(msg),
            Err(error) => HandleOutcome::reply(format!("⚠️ {error:#}")),
        },
        "agent" if !arg.is_empty() => match switch_agent(state, key, arg) {
            Ok(msg) => HandleOutcome::reply(msg),
            Err(error) => HandleOutcome::reply(format!("⚠️ {error:#}")),
        },
        "cancel" => match cmd_cancel(state, key) {
            Ok(msg) => HandleOutcome::reply(msg),
            Err(error) => HandleOutcome::reply(format!("⚠️ {error:#}")),
        },
        "end" => match cmd_end(state, key) {
            Ok(msg) => HandleOutcome::reply(msg),
            Err(error) => HandleOutcome::reply(format!("⚠️ {error:#}")),
        },
        other => HandleOutcome::reply(format!("未知命令 /{other}\n\n{}", help_text())),
    }
}

/// Open a fresh session in the given (or default) workspace using the chat's
/// current agent (or the configured default).
pub(super) fn start_new_session(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    workspace_arg: Option<&str>,
) -> Result<String> {
    let config = state.config_snapshot();
    let workspace = workspace_arg
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            config
                .workspace_for_chat(key.platform, &key.chat_id)
                .map(str::to_string)
        })
        .context("no workspace given and no default configured")?;

    if !config.is_workspace_allowed(key.platform, &workspace) {
        bail!("workspace not allowed: {workspace}");
    }

    // Preserve the chat's current agent choice across /new if one exists.
    let current_session = state.chat_session(key);
    let agent = current_session
        .as_ref()
        .map(|session| session.agent)
        .unwrap_or(resolve_default_agent(state, key.platform)?);

    let handle = start_session(state, key.platform, agent, &workspace)?;
    if current_session.is_some() {
        state.unbind_chat(key);
    } else {
        state.clear_queued_prompts(key);
    }
    state.bind_chat(
        key.clone(),
        ChatSession {
            sessio_runtime_session_id: handle.clone(),
            agent_runtime_session_id: state.runtime.agent_runtime_session_id_for_session(&handle),
            agent,
            workspace_path: workspace.clone(),
            last_activity_at: 0,
        },
    );

    Ok(format!(
        "🆕 新会话已开启\nagent: {}\nworkspace: {}",
        agent.as_str(),
        workspace
    ))
}

/// Switch the agent for this chat. Changing agents opens a new session;
/// selecting the current agent keeps the existing session so model/effort
/// changes can be applied in place.
pub(super) fn switch_agent(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    agent_arg: &str,
) -> Result<String> {
    let arg = agent_arg.trim();
    let agent = Agent::from_db_str(arg.trim())
        .with_context(|| format!("unknown agent: {arg} (expected claude/codex/gemini/astra-pi)"))?;

    let config = state.config_snapshot();
    let current_session = state.chat_session(key);
    if let Some(session) = current_session.as_ref() {
        if session.agent == agent {
            state.clear_queued_prompts(key);
            return Ok(format!(
                "当前已在使用 agent: {}\nworkspace: {}",
                agent.as_str(),
                session.workspace_path
            ));
        }
    }

    let workspace = current_session
        .as_ref()
        .map(|session| session.workspace_path.clone())
        .or_else(|| {
            config
                .workspace_for_chat(key.platform, &key.chat_id)
                .map(str::to_string)
        })
        .context("no workspace bound and no default configured; use /new <workspace> first")?;

    let handle = start_session(state, key.platform, agent, &workspace)?;
    if current_session.is_some() {
        state.unbind_chat(key);
    } else {
        state.clear_queued_prompts(key);
    }
    state.bind_chat(
        key.clone(),
        ChatSession {
            agent_runtime_session_id: state.runtime.agent_runtime_session_id_for_session(&handle),
            sessio_runtime_session_id: handle,
            agent,
            workspace_path: workspace.clone(),
            last_activity_at: 0,
        },
    );

    Ok(format!(
        "🔀 已切换 agent: {}\nworkspace: {}",
        agent.as_str(),
        workspace
    ))
}

/// Update a config option on the current runtime session without changing the
/// channel session binding. Used for same-agent model/effort switches.
pub(super) fn set_session_config_option(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    config_id: &str,
    value: serde_json::Value,
) -> Result<()> {
    let session = state
        .chat_session(key)
        .context("no active session for this chat")?;
    state.runtime.set_config_option(
        &session.sessio_runtime_session_id,
        AgentSessionConfigChange {
            config_id: config_id.to_string(),
            value,
        },
    )
}

/// `/cancel` — cancel the active turn on the chat's session, if any.
fn cmd_cancel(state: &Arc<ImBridgeState>, key: &ChatKey) -> Result<String> {
    let session = state
        .chat_session(key)
        .context("no active session for this chat")?;
    let turn = state
        .runtime
        .active_turn_id(&session.sessio_runtime_session_id);
    match turn {
        Some(turn_id) => {
            state
                .runtime
                .cancel_turn(&session.sessio_runtime_session_id, &turn_id)?;
            Ok("🛑 已请求取消当前回合".to_string())
        }
        None => Ok("当前没有进行中的回合".to_string()),
    }
}

/// `/end` — dispose the chat's session and clear the binding.
fn cmd_end(state: &Arc<ImBridgeState>, key: &ChatKey) -> Result<String> {
    let session = state
        .chat_session(key)
        .context("no active session for this chat")?;
    let report = state
        .runtime
        .cleanup_session_bounded(&session.sessio_runtime_session_id, Duration::from_secs(5));
    state.unbind_chat(key);
    if let Some(error) = report.dispose_error {
        bail!("session disposed with error: {error}");
    }
    Ok("👋 会话已结束".to_string())
}

/// Send a plain prompt to the chat's bound session, lazily opening a default
/// session if none exists yet.
fn dispatch_prompt(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    text: &str,
) -> Result<PromptDispatchOutcome> {
    let session = match state.chat_session(key) {
        Some(session) => session,
        None => restore_or_start_session(state, key)?,
    };
    state.touch_chat(key);

    if state
        .runtime
        .active_turn_id(&session.sessio_runtime_session_id)
        .is_some()
    {
        let position = state.enqueue_prompt(key, text.to_string());
        return Ok(PromptDispatchOutcome::Queued(position));
    }

    state
        .runtime
        .send_input(
            &session.sessio_runtime_session_id,
            AgentInput {
                text: text.to_string(),
                attachments: Vec::new(),
                options: Default::default(),
            },
        )
        .map(|_| PromptDispatchOutcome::Sent)
}

/// Dispatch the next queued prompt for a chat after its active turn settles.
/// Commands are never queued, so this only needs to replay plain text prompts.
pub(super) fn dispatch_next_queued_prompt(state: &Arc<ImBridgeState>, key: &ChatKey) {
    let Some(text) = state.pop_queued_prompt(key) else {
        return;
    };
    let Some(session) = state.chat_session(key) else {
        log::warn!(
            "[im-bridge:router] dropping queued prompt for unbound {} chat {}",
            key.platform,
            key.chat_id
        );
        return;
    };

    if state
        .runtime
        .active_turn_id(&session.sessio_runtime_session_id)
        .is_some()
    {
        state.prepend_prompt(key, text);
        return;
    }

    if let Err(error) = state.runtime.send_input(
        &session.sessio_runtime_session_id,
        AgentInput {
            text,
            attachments: Vec::new(),
            options: Default::default(),
        },
    ) {
        log::warn!("[im-bridge:router] failed to send queued prompt: {error:#}");
        if let Err(send_error) =
            state.send_to_chat(key, &format!("⚠️ 发送队列中的下一条消息失败：{error:#}"))
        {
            log::warn!("[im-bridge:router] failed to report queued prompt error: {send_error:#}");
        }
    }
}

fn restore_or_start_session(state: &Arc<ImBridgeState>, key: &ChatKey) -> Result<ChatSession> {
    if let Some(record) = state
        .store
        .get_active_channel_session(key.platform, &key.chat_id)?
    {
        match resume_channel_session(state, key, record) {
            Ok(session) => return Ok(session),
            Err(error) => {
                log::warn!(
                    "[im-bridge:router] failed to resume channel session for {} chat {}: {error:#}",
                    key.platform,
                    key.chat_id
                );
            }
        }
    }

    let config = state.config_snapshot();
    let workspace = config
        .workspace_for_chat(key.platform, &key.chat_id)
        .map(str::to_string)
        .context("no session and no default workspace; use /new <workspace> first")?;
    let agent = resolve_default_agent(state, key.platform)?;
    let handle = start_session(state, key.platform, agent, &workspace)?;
    let session = ChatSession {
        agent_runtime_session_id: state.runtime.agent_runtime_session_id_for_session(&handle),
        sessio_runtime_session_id: handle,
        agent,
        workspace_path: workspace,
        last_activity_at: 0,
    };
    state.bind_chat(key.clone(), session.clone());
    Ok(session)
}

pub(super) fn ensure_chat_session(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
) -> Result<ChatSession> {
    match state.chat_session(key) {
        Some(session) => {
            state.touch_chat(key);
            Ok(session)
        }
        None => restore_or_start_session(state, key),
    }
}

fn resume_channel_session(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    record: ChannelSessionRecord,
) -> Result<ChatSession> {
    let config = state.config_snapshot();
    if !config.is_workspace_allowed(key.platform, &record.workspace_path) {
        bail!(
            "persisted workspace no longer allowed: {}",
            record.workspace_path
        );
    }

    let mut req = EnsureAgentRuntimeSession {
        agent: record.agent,
        sessio_runtime_session_id: record.sessio_runtime_session_id.clone(),
        workspace_path: record.workspace_path.clone(),
        agent_runtime_session_id: Some(record.agent_session_id.clone()),
        source_agent: Some(record.agent),
        options: Default::default(),
    };
    hydrate_options_from_store(req.agent, &mut req.options, &state.store)?;
    let handle = state.runtime.ensure_session(req)?;
    state
        .runtime
        .wait_for_session_startup(&handle.sessio_runtime_session_id, SESSION_STARTUP_TIMEOUT)?;
    let agent_runtime_session_id = state
        .runtime
        .agent_runtime_session_id_for_session(&handle.sessio_runtime_session_id)
        .or(Some(record.agent_session_id));
    let session = ChatSession {
        sessio_runtime_session_id: handle.sessio_runtime_session_id,
        agent_runtime_session_id,
        agent: handle.agent,
        workspace_path: handle.workspace_path,
        last_activity_at: 0,
    };
    state.bind_chat(key.clone(), session.clone());
    Ok(session)
}

/// Start a runtime session for `agent` in `workspace`, hydrating agent defaults
/// (model/effort/transport/command) from the store the same way the Tauri
/// command does, then wait for startup so the first prompt won't race.
fn start_session(
    state: &Arc<ImBridgeState>,
    platform: &str,
    agent: Agent,
    workspace: &str,
) -> Result<String> {
    let mut req = StartAgentSession {
        agent,
        workspace_path: workspace.to_string(),
        initial_prompt: None,
        source_session_id: None,
        source_agent: None,
        options: Default::default(),
    };
    hydrate_start_request(&mut req, &state.store)?;
    let config = state.config_snapshot();
    if let Some(model) = config.model_for_platform(platform) {
        req.options.insert(
            "model".to_string(),
            serde_json::Value::String(model.to_string()),
        );
    }
    if let Some(effort) = config.effort_for_platform(platform) {
        req.options.insert(
            "effort".to_string(),
            serde_json::Value::String(effort.to_string()),
        );
    }
    let handle = state.runtime.start_session(req)?;
    state
        .runtime
        .wait_for_session_startup(&handle.sessio_runtime_session_id, SESSION_STARTUP_TIMEOUT)?;
    Ok(handle.sessio_runtime_session_id)
}

fn resolve_default_agent(state: &Arc<ImBridgeState>, platform: &str) -> Result<Agent> {
    let config = state.config_snapshot();
    let agents = state.store.list_agents()?;
    if let Some(agent) = config.configured_agent_for_platform(platform) {
        if agents
            .iter()
            .any(|info| info.enabled && info.id == agent.as_str())
        {
            return Ok(agent);
        }
    }
    agents
        .into_iter()
        .find(|agent| agent.enabled)
        .and_then(|agent| Agent::from_db_str(&agent.id))
        .context("no enabled runtime agent configured")
}

/// Populate session options from the stored agent definition. Mirrors
/// `hydrate_start_request_from_db` in lib.rs (kept local to avoid widening that
/// function's visibility).
fn hydrate_start_request(
    req: &mut StartAgentSession,
    store: &Arc<dyn crate::store::SessionStore>,
) -> Result<()> {
    hydrate_options_from_store(req.agent, &mut req.options, store)
}

fn hydrate_options_from_store(
    agent_id: Agent,
    options: &mut crate::agents::runtime::types::RuntimeMetadata,
    store: &Arc<dyn crate::store::SessionStore>,
) -> Result<()> {
    let Some(agent) = store
        .list_agents()?
        .into_iter()
        .find(|a| a.id == agent_id.as_str())
    else {
        return Ok(());
    };
    insert_if_missing(options, "model", agent.model);
    insert_if_missing(options, "effort", agent.effort);
    insert_if_missing(options, "permissionMode", agent.permission_mode);
    insert_if_missing(
        options,
        "transport",
        Some(transport_option(agent.transport)),
    );
    if !options.contains_key("command") && !options.contains_key("acpCommand") {
        if let Some(command) = agent.commands.session.first().cloned() {
            insert_if_missing(options, "command", Some(command));
        }
    }
    Ok(())
}

fn insert_if_missing(
    options: &mut crate::agents::runtime::types::RuntimeMetadata,
    key: &str,
    value: Option<String>,
) {
    if options.contains_key(key) {
        return;
    }
    if let Some(value) = value.map(|v| v.trim().to_string()) {
        if !value.is_empty() {
            options.insert(key.to_string(), serde_json::Value::String(value));
        }
    }
}

fn transport_option(transport: crate::agents::runtime::types::RuntimeTransportKind) -> String {
    use crate::agents::runtime::types::RuntimeTransportKind as T;
    match transport {
        T::Acp => "acp",
        T::CliStreamJson => "cliStreamJson",
        T::PlainCli => "plainCli",
        T::Sidecar => "sidecar",
        T::Fake => "fake",
    }
    .to_string()
}

pub(super) fn session_status_text(state: &Arc<ImBridgeState>, key: &ChatKey) -> String {
    let queued = state.queued_prompt_count(key);
    match state.chat_session(key) {
        Some(session) => {
            let mut text = format!(
                "📍 当前会话\nagent: {}\nworkspace: {}\nsession: {}",
                session.agent.as_str(),
                session.workspace_path,
                session.sessio_runtime_session_id
            );
            if queued > 0 {
                text.push_str(&format!("\nqueued prompts: {queued}"));
            }
            text
        }
        None => "当前没有会话。发送任意消息或用 /new <workspace> 开启。".to_string(),
    }
}

fn help_text() -> String {
    "Sessio 命令:\n\
     /new [workspace] — 开启新会话\n\
     /agent [agent] — 选择或切换 agent (claude/codex/gemini/astra-pi)\n\
     /model — 切换当前会话的 model\n\
     /effort — 切换当前会话的 effort\n\
     /workspace — 切换当前会话的 workspace\n\
     /cancel — 取消当前回合\n\
     /end — 结束会话\n\
     /status — 查看当前会话\n\
     /help — 显示帮助\n\n\
     直接发送消息即可作为 prompt 提交给 agent。"
        .to_string()
}
