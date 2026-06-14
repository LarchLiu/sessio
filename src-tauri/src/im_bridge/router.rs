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
use crate::models::{Agent, AgentInfo, RuntimeAgentOptionMetadata};
use crate::store::ChannelSessionRecord;

use super::attachments::{filter_by_capability, InboundAttachment};
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

#[derive(Debug, Clone)]
pub(super) struct ActionMenu {
    pub text: String,
    pub choices: Vec<ActionChoice>,
}

#[derive(Debug, Clone)]
pub(super) struct ActionChoice {
    pub label: String,
    pub action: String,
}

enum PromptDispatchOutcome {
    Sent,
    Queued(usize),
}

/// Plain prompts queued behind an active turn. Attachments piggy-back on the
/// prompt that introduced them so they're replayed together.
#[derive(Debug, Clone, Default)]
pub(super) struct QueuedPrompt {
    pub text: String,
    pub attachments: Vec<InboundAttachment>,
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

/// Variant accepting media/file attachments already downloaded to local disk.
/// Slash commands ignore attachments; plain prompts forward them subject to the
/// bound agent's runtime capabilities. Pass an empty `attachments` vec for
/// text-only flows.
pub fn handle_message_with_attachments(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    text: &str,
    attachments: Vec<InboundAttachment>,
) -> HandleOutcome {
    let trimmed = text.trim();
    let has_attachments = !attachments.is_empty();
    if trimmed.is_empty() && !has_attachments {
        return HandleOutcome::silent();
    }

    if let Some(rest) = trimmed.strip_prefix('/') {
        return handle_command(state, key, rest);
    }

    let effective_text = if trimmed.is_empty() {
        "(attached files)".to_string()
    } else {
        trimmed.to_string()
    };

    match dispatch_prompt(state, key, &effective_text, attachments) {
        Ok((PromptDispatchOutcome::Sent, notes)) => {
            if notes.is_empty() {
                HandleOutcome::silent()
            } else {
                HandleOutcome::reply(notes.join("\n"))
            }
        }
        Ok((PromptDispatchOutcome::Queued(position), notes)) => {
            let mut reply = format!(
                "上一轮还在处理中，已将这条消息加入队列（第 {position} 条）。\n发送 /cancel 可取消当前回合。"
            );
            if !notes.is_empty() {
                reply.push_str("\n\n");
                reply.push_str(&notes.join("\n"));
            }
            HandleOutcome::reply(reply)
        }
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

/// Build a platform-renderable menu for slash commands that need a click
/// target. Returns `None` for commands that should continue through the plain
/// text command router.
pub(super) fn interactive_action_menu(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    text: &str,
) -> Option<Result<ActionMenu>> {
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
        "agent" if arg.is_empty() => agent_menu(state),
        "model" => model_menu(state, key),
        "effort" => effort_menu(state, key),
        "workspace" => workspace_menu(state, key),
        _ => return None,
    };
    Some(result)
}

pub(super) fn handle_action_callback(
    state: &Arc<ImBridgeState>,
    key: &ChatKey,
    action: &str,
) -> Result<String> {
    let mut parts = action.split(':');
    match parts.next().unwrap_or("") {
        "agent" => {
            let agent = parts.next().context("missing agent")?;
            switch_agent(state, key, agent)?;
            Ok(format!("Agent: {agent}"))
        }
        "model" => {
            let agent = parse_action_agent(parts.next())?;
            let index = parse_action_index(parts.next())?;
            ensure_current_agent(state, key, agent)?;
            let agent_info = agent_info(state, agent)?;
            let choices = option_choices(&agent_info.models, agent_info.model.as_deref());
            let choice = choices
                .get(index)
                .with_context(|| "model menu expired; open /model again")?;
            set_session_config_option(
                state,
                key,
                "model",
                serde_json::Value::String(choice.value.clone()),
            )?;
            Ok(format!("Model: {}", choice.label))
        }
        "effort" => {
            let agent = parse_action_agent(parts.next())?;
            let index = parse_action_index(parts.next())?;
            ensure_current_agent(state, key, agent)?;
            let agent_info = agent_info(state, agent)?;
            let choices = option_choices(&agent_info.efforts, agent_info.effort.as_deref());
            let choice = choices
                .get(index)
                .with_context(|| "effort menu expired; open /effort again")?;
            set_session_config_option(
                state,
                key,
                effort_config_id(agent),
                serde_json::Value::String(choice.value.clone()),
            )?;
            Ok(format!("Effort: {}", choice.label))
        }
        "workspace" => {
            let index = parse_action_index(parts.next())?;
            let choices = workspace_choices(state, key);
            let choice = choices
                .get(index)
                .with_context(|| "workspace menu expired; open /workspace again")?;
            start_new_session(state, key, Some(&choice.path))?;
            Ok(format!("Workspace: {}", choice.label))
        }
        other => bail!("unknown action: {other}"),
    }
}

fn agent_menu(state: &Arc<ImBridgeState>) -> Result<ActionMenu> {
    let agents = available_agents(state)?;
    if agents.is_empty() {
        bail!("no agents configured");
    }
    let choices = agents
        .into_iter()
        .filter_map(|agent_info| {
            let agent = Agent::from_db_str(&agent_info.id)?;
            Some(ActionChoice {
                label: agent_info.display_name,
                action: format!("agent:{}", agent.as_str()),
            })
        })
        .collect();
    Ok(ActionMenu {
        text: "选择 agent。切换到不同 agent 会开启新会话；选择当前 agent 不会新建。".to_string(),
        choices,
    })
}

fn model_menu(state: &Arc<ImBridgeState>, key: &ChatKey) -> Result<ActionMenu> {
    let session = ensure_chat_session(state, key)?;
    let agent_info = agent_info(state, session.agent)?;
    let choices = option_choices(&agent_info.models, agent_info.model.as_deref());
    if choices.is_empty() {
        bail!("{} has no model options", agent_info.display_name);
    }
    Ok(ActionMenu {
        text: format!(
            "选择 {} 的 model。不会新建 session。",
            agent_info.display_name
        ),
        choices: choices
            .iter()
            .enumerate()
            .map(|(index, choice)| ActionChoice {
                label: choice.label.clone(),
                action: format!("model:{}:{index}", session.agent.as_str()),
            })
            .collect(),
    })
}

fn effort_menu(state: &Arc<ImBridgeState>, key: &ChatKey) -> Result<ActionMenu> {
    let session = ensure_chat_session(state, key)?;
    let agent_info = agent_info(state, session.agent)?;
    let choices = option_choices(&agent_info.efforts, agent_info.effort.as_deref());
    if choices.is_empty() {
        bail!("{} has no effort options", agent_info.display_name);
    }
    Ok(ActionMenu {
        text: format!(
            "选择 {} 的 effort。不会新建 session。",
            agent_info.display_name
        ),
        choices: choices
            .iter()
            .enumerate()
            .map(|(index, choice)| ActionChoice {
                label: choice.label.clone(),
                action: format!("effort:{}:{index}", session.agent.as_str()),
            })
            .collect(),
    })
}

fn workspace_menu(state: &Arc<ImBridgeState>, key: &ChatKey) -> Result<ActionMenu> {
    let choices = workspace_choices(state, key);
    if choices.is_empty() {
        bail!("no allowed workspaces configured");
    }
    Ok(ActionMenu {
        text: "选择当前会话的 workspace。会开启同 agent 的新 runtime session，不会修改默认 workspace。"
            .to_string(),
        choices: choices
            .iter()
            .enumerate()
            .map(|(index, choice)| ActionChoice {
                label: choice.label.clone(),
                action: format!("workspace:{index}"),
            })
            .collect(),
    })
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
    let session = ensure_chat_session(state, key)?;
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
    for workspace in config.workspace_choices_for_chat(key.platform, &key.chat_id) {
        push_unique(&mut paths, workspace);
    }
    if let Some(session) = state.chat_session(key) {
        push_unique(&mut paths, &session.workspace_path);
    }
    paths
        .into_iter()
        .filter(|path| config.is_workspace_allowed(key.platform, path))
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
    attachments: Vec<InboundAttachment>,
) -> Result<(PromptDispatchOutcome, Vec<String>)> {
    let session = match state.chat_session(key) {
        Some(session) => session,
        None => restore_or_start_session(state, key)?,
    };
    state.touch_chat(key);

    // Filter attachments to what the bound runtime/agent can accept and collect
    // a user-facing note for anything dropped.
    let (filtered, notes) = if attachments.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let caps = state
            .runtime
            .capabilities_for_session(&session.sessio_runtime_session_id);
        match caps {
            Some(caps) => {
                let result = filter_by_capability(attachments, &caps);
                let notes = result.notes(session.agent.as_str());
                (result.allowed, notes)
            }
            None => {
                // Capabilities unknown means the session is still warming up;
                // be conservative and drop attachments rather than racing the
                // ACP capability negotiation.
                (
                    Vec::new(),
                    vec![format!("⚠️ 会话尚未就绪，已忽略本条消息中的附件。")],
                )
            }
        }
    };

    if state
        .runtime
        .active_turn_id(&session.sessio_runtime_session_id)
        .is_some()
    {
        let position = state.enqueue_prompt(
            key,
            QueuedPrompt {
                text: text.to_string(),
                attachments: filtered,
            },
        );
        return Ok((PromptDispatchOutcome::Queued(position), notes));
    }

    let agent_attachments = filtered
        .into_iter()
        .map(InboundAttachment::into_agent)
        .collect::<Vec<_>>();

    state
        .runtime
        .send_input(
            &session.sessio_runtime_session_id,
            AgentInput {
                text: text.to_string(),
                attachments: agent_attachments,
                options: Default::default(),
            },
        )
        .map(|_| (PromptDispatchOutcome::Sent, notes))
}

/// Dispatch the next queued prompt for a chat after its active turn settles.
/// Commands are never queued, so this only needs to replay plain text prompts.
pub(super) fn dispatch_next_queued_prompt(state: &Arc<ImBridgeState>, key: &ChatKey) {
    let Some(prompt) = state.pop_queued_prompt(key) else {
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
        state.prepend_prompt(key, prompt);
        return;
    }

    let agent_attachments = prompt
        .attachments
        .into_iter()
        .map(InboundAttachment::into_agent)
        .collect::<Vec<_>>();

    if let Err(error) = state.runtime.send_input(
        &session.sessio_runtime_session_id,
        AgentInput {
            text: prompt.text,
            attachments: agent_attachments,
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
