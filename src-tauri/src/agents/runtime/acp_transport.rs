use std::str::FromStr;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{
    CancelNotification, ContentBlock, ForkSessionRequest, InitializeRequest, LoadSessionRequest,
    NewSessionRequest, PromptRequest, ProtocolVersion, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest, SessionId,
    SessionNotification, SessionUpdate, StopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent as AcpAgentRole, ConnectionTo, UntypedMessage};
use anyhow::{Context, Result};

use super::acp::{
    acp_protocol_event, convert_session_notification, permission_response_from_decision,
};
use super::manager::{RuntimeManager, RuntimePermissionDecision};
use super::types::{
    AgentInput, RuntimeCapabilitySet, RuntimeError, RuntimeMetadata, RuntimeTransportKind,
};
use crate::config::AgentRuntimeConfig;
use crate::models::Agent;

#[derive(Debug, Clone)]
pub struct AcpSessionController {
    command_tx: tauri::async_runtime::Sender<AcpWorkerCommand>,
}

impl AcpSessionController {
    pub fn send_prompt(&self, turn_id: String, input: AgentInput) -> Result<()> {
        self.command_tx
            .try_send(AcpWorkerCommand::Prompt { turn_id, input })
            .map_err(|e| anyhow::anyhow!("failed to queue ACP prompt: {e}"))
    }

    pub fn cancel_turn(&self, turn_id: String) -> Result<()> {
        self.command_tx
            .try_send(AcpWorkerCommand::Cancel { turn_id })
            .map_err(|e| anyhow::anyhow!("failed to queue ACP cancellation: {e}"))
    }

    pub fn set_config_option(&self, config_id: String, value: serde_json::Value) -> Result<()> {
        self.command_tx
            .try_send(AcpWorkerCommand::SetConfigOption { config_id, value })
            .map_err(|e| anyhow::anyhow!("failed to queue ACP config update: {e}"))
    }
}

#[derive(Debug, Clone)]
pub enum AcpSessionStart {
    New,
    Load { agent_session_id: String },
    Resume { agent_session_id: String },
    Fork { source_session_id: String },
}

#[derive(Debug)]
enum AcpWorkerCommand {
    Prompt { turn_id: String, input: AgentInput },
    Cancel { turn_id: String },
    SetConfigOption { config_id: String, value: serde_json::Value },
}

pub fn command_from_options(agent: Agent, options: &RuntimeMetadata) -> String {
    options
        .get("acpCommand")
        .or_else(|| options.get("command"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| default_acp_command(agent).to_string())
}

pub fn command_from_config(agent: Agent, config: &AgentRuntimeConfig) -> String {
    config
        .command
        .clone()
        .unwrap_or_else(|| default_acp_command(agent).to_string())
}

pub fn spawn_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    _agent: Agent,
    workspace_path: String,
    command: String,
    start: AcpSessionStart,
) -> AcpSessionController {
    let (command_tx, command_rx) = tauri::async_runtime::channel(32);

    tauri::async_runtime::spawn({
        let manager = manager.clone();
        async move {
            if let Err(error) = run_session(
                manager.clone(),
                sessio_runtime_session_id.clone(),
                workspace_path,
                command,
                start,
                command_rx,
            )
            .await
            {
                let _ = manager.fail_session_start(&sessio_runtime_session_id, error.to_string());
            }
        }
    });

    AcpSessionController { command_tx }
}

async fn run_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    workspace_path: String,
    command: String,
    start: AcpSessionStart,
    command_rx: tauri::async_runtime::Receiver<AcpWorkerCommand>,
) -> Result<()> {
    let agent = AcpAgent::from_str(&command)
        .with_context(|| format!("failed to parse ACP command: {command}"))?;
    let current_turn_id = Arc::new(Mutex::new(None::<String>));
    let notification_manager = manager.clone();
    let notification_session_id = sessio_runtime_session_id.clone();
    let notification_turn_id = current_turn_id.clone();
    let permission_manager = manager.clone();
    let permission_session_id = sessio_runtime_session_id.clone();
    let permission_turn_id = current_turn_id.clone();

    agent_client_protocol::Client
        .builder()
        .name("sessio")
        .on_receive_notification(
            async move |notification: SessionNotification, _connection| {
                let turn_id = current_turn(&notification_turn_id).or_else(|| {
                    notification_manager.active_turn_id(&notification_session_id)
                });
                let Some(turn_id) = turn_id else {
                    log::warn!(
                        "[sessio-runtime:acp:notification:drop] session={} update={:?}",
                        notification_session_id,
                        notification.update
                    );
                    return Ok(());
                };
                log::info!(
                    "[sessio-runtime:acp:notification] session={} turn={} update={:?}",
                    notification_session_id,
                    turn_id,
                    notification.update
                );
                notification_manager
                    .emit(
                        acp_protocol_event(
                            &notification_session_id,
                            "agent_to_client",
                            "notification",
                            "session/update",
                            Some(notification.session_id.to_string()),
                            Some(turn_id.clone()),
                            None,
                            Some(session_update_type(&notification.update).to_string()),
                            &notification,
                        )
                        .map_err(acp_internal_error)?,
                    )
                    .map_err(acp_internal_error)?;
                if let Some(event) =
                    convert_session_notification(&notification, &notification_session_id, &turn_id)
                        .map_err(acp_internal_error)?
                {
                    notification_manager
                        .emit(event)
                        .map_err(acp_internal_error)?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                let turn_id = current_turn(&permission_turn_id)
                    .or_else(|| permission_manager.active_turn_id(&permission_session_id));
                let Some(turn_id) = turn_id else {
                    log::warn!(
                        "[sessio-runtime:permission-request:drop] session={} request={:?}",
                        permission_session_id,
                        request
                    );
                    return responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    ));
                };
                let request_id = json_id_to_string(responder.id());
                permission_manager
                    .emit(
                        acp_protocol_event(
                            &permission_session_id,
                            "agent_to_client",
                            "request",
                            "session/request_permission",
                            Some(request.session_id.to_string()),
                            Some(turn_id.clone()),
                            Some(request_id.clone()),
                            None,
                            &request,
                        )
                        .map_err(acp_internal_error)?,
                    )
                    .map_err(acp_internal_error)?;
                let manager = permission_manager.clone();
                let sessio_runtime_session_id = permission_session_id.clone();
                let request_for_ui = request.clone();
                let request_id_for_ui = request_id.clone();
                let turn_id_for_ui = turn_id.clone();
                let decision = tauri::async_runtime::spawn_blocking(move || {
                    manager.request_permission_from_acp(
                        &request_for_ui,
                        &sessio_runtime_session_id,
                        &turn_id_for_ui,
                        &request_id_for_ui,
                    )
                })
                .await
                .map_err(acp_internal_error)?
                .map_err(acp_internal_error)?;

                let response = match decision {
                    RuntimePermissionDecision::Selected { option_id } => {
                        permission_response_from_decision(&request, &option_id)
                    }
                    RuntimePermissionDecision::Cancelled => {
                        RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
                    }
                };
                log::info!(
                    "[sessio-runtime:permission-response:acp] request={} response={:?}",
                    request_id,
                    response
                );
                permission_manager
                    .emit(
                        acp_protocol_event(
                            &permission_session_id,
                            "client_to_agent",
                            "response",
                            "session/request_permission",
                            Some(request.session_id.to_string()),
                            Some(turn_id.clone()),
                            Some(request_id),
                            None,
                            &response,
                        )
                        .map_err(acp_internal_error)?,
                    )
                    .map_err(acp_internal_error)?;
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<AcpAgentRole>| {
            let manager = manager.clone();
            let sessio_runtime_session_id = sessio_runtime_session_id.clone();
            let workspace_path = workspace_path.clone();
            let start = start.clone();
            let current_turn_id = current_turn_id.clone();
            async move {
                let init = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                manager
                    .emit(
                        acp_protocol_event(
                            &sessio_runtime_session_id,
                            "agent_to_client",
                            "response",
                            "initialize",
                            None,
                            None,
                            None,
                            None,
                            &init,
                        )
                        .map_err(acp_internal_error)?,
                    )
                    .map_err(acp_internal_error)?;
                let capabilities = runtime_capabilities_from_acp(&init.agent_capabilities);
                let acp_session_id = match start {
                    AcpSessionStart::New => {
                        let session = connection
                            .send_request(NewSessionRequest::new(workspace_path))
                            .block_task()
                            .await?;
                        manager
                            .emit(
                                acp_protocol_event(
                                    &sessio_runtime_session_id,
                                    "agent_to_client",
                                    "response",
                                    "session/new",
                                    Some(session.session_id.to_string()),
                                    None,
                                    None,
                                    None,
                                    &session,
                                )
                                .map_err(acp_internal_error)?,
                            )
                            .map_err(acp_internal_error)?;
                        session.session_id
                    }
                    AcpSessionStart::Load { agent_session_id } => {
                        if !init.agent_capabilities.load_session {
                            return Err(acp_internal_error(format!(
                                "ACP agent does not support session/load for session {agent_session_id}"
                            )));
                        }
                        let acp_session_id = SessionId::new(agent_session_id);
                        let session = connection
                            .send_request(LoadSessionRequest::new(
                                acp_session_id.clone(),
                                workspace_path,
                            ))
                            .block_task()
                            .await?;
                        manager
                            .emit(
                                acp_protocol_event(
                                    &sessio_runtime_session_id,
                                    "agent_to_client",
                                    "response",
                                    "session/load",
                                    Some(acp_session_id.to_string()),
                                    None,
                                    None,
                                    None,
                                    &session,
                                )
                                .map_err(acp_internal_error)?,
                            )
                            .map_err(acp_internal_error)?;
                        acp_session_id
                    }
                    AcpSessionStart::Resume { agent_session_id } => {
                        let acp_session_id = SessionId::new(agent_session_id);
                        if init.agent_capabilities.session_capabilities.resume.is_none()
                            && init.agent_capabilities.load_session
                        {
                            log::info!(
                                "[sessio-runtime:acp:resume-fallback-load] session={}",
                                acp_session_id
                            );
                            let session = connection
                                .send_request(LoadSessionRequest::new(
                                    acp_session_id.clone(),
                                    workspace_path,
                                ))
                                .block_task()
                                .await?;
                            manager
                                .emit(
                                    acp_protocol_event(
                                        &sessio_runtime_session_id,
                                        "agent_to_client",
                                        "response",
                                        "session/load",
                                        Some(acp_session_id.to_string()),
                                        None,
                                        None,
                                        None,
                                        &session,
                                    )
                                    .map_err(acp_internal_error)?,
                                )
                                .map_err(acp_internal_error)?;
                            acp_session_id
                        } else {
                            if init.agent_capabilities.session_capabilities.resume.is_none() {
                                return Err(acp_internal_error(format!(
                                    "ACP agent does not support session/resume or session/load for session {acp_session_id}"
                                )));
                            }
                            let session = connection
                                .send_request(ResumeSessionRequest::new(
                                    acp_session_id.clone(),
                                    workspace_path,
                                ))
                                .block_task()
                                .await?;
                            manager
                                .emit(
                                    acp_protocol_event(
                                        &sessio_runtime_session_id,
                                        "agent_to_client",
                                        "response",
                                        "session/resume",
                                        Some(acp_session_id.to_string()),
                                        None,
                                        None,
                                        None,
                                        &session,
                                    )
                                    .map_err(acp_internal_error)?,
                                )
                                .map_err(acp_internal_error)?;
                            acp_session_id
                        }
                    }
                    AcpSessionStart::Fork { source_session_id } => {
                        if init.agent_capabilities.session_capabilities.fork.is_none() {
                            return Err(acp_internal_error(format!(
                                "ACP agent does not support session/fork for session {source_session_id}"
                            )));
                        }
                        let source_acp_session_id = SessionId::new(source_session_id);
                        let session = connection
                            .send_request(ForkSessionRequest::new(
                                source_acp_session_id,
                                workspace_path,
                            ))
                            .block_task()
                            .await?;
                        manager
                            .emit(
                                acp_protocol_event(
                                    &sessio_runtime_session_id,
                                    "agent_to_client",
                                    "response",
                                    "session/fork",
                                    Some(session.session_id.to_string()),
                                    None,
                                    None,
                                    None,
                                    &session,
                                )
                                .map_err(acp_internal_error)?,
                            )
                            .map_err(acp_internal_error)?;
                        session.session_id
                    }
                };
                let agent_runtime_session_id = acp_session_id.to_string();
                manager.mark_session_ready(
                    &sessio_runtime_session_id,
                    agent_runtime_session_id,
                    capabilities,
                )
                .map_err(acp_internal_error)?;
                run_command_loop(
                    manager,
                    sessio_runtime_session_id,
                    acp_session_id,
                    connection,
                    command_rx,
                    current_turn_id,
                )
                .await
                .map_err(acp_internal_error)
            }
        })
        .await
        .map_err(anyhow::Error::from)
}

async fn run_command_loop(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    acp_session_id: SessionId,
    connection: ConnectionTo<AcpAgentRole>,
    mut command_rx: tauri::async_runtime::Receiver<AcpWorkerCommand>,
    current_turn_id: Arc<Mutex<Option<String>>>,
) -> Result<()> {
    while let Some(command) = command_rx.recv().await {
        match command {
            AcpWorkerCommand::Prompt { turn_id, input } => {
                set_current_turn(&current_turn_id, Some(turn_id.clone()));
                spawn_prompt_task(
                    manager.clone(),
                    sessio_runtime_session_id.clone(),
                    acp_session_id.clone(),
                    connection.clone(),
                    current_turn_id.clone(),
                    turn_id,
                    input,
                );
            }
            AcpWorkerCommand::Cancel { turn_id } => {
                log::info!(
                    "[sessio-runtime:acp:cancel] session={} turn={}",
                    sessio_runtime_session_id,
                    turn_id
                );
                let notification = CancelNotification::new(acp_session_id.clone());
                manager.emit(acp_protocol_event(
                    &sessio_runtime_session_id,
                    "client_to_agent",
                    "notification",
                    "session/cancel",
                    Some(acp_session_id.to_string()),
                    Some(turn_id.clone()),
                    None,
                    None,
                    &notification,
                )?)?;
                connection
                    .send_notification(notification)
                    .map_err(anyhow::Error::from)?;
            }
            AcpWorkerCommand::SetConfigOption { config_id, value } => {
                log::info!(
                    "[sessio-runtime:acp:set-config] session={} config={} value={:?}",
                    sessio_runtime_session_id,
                    config_id,
                    value
                );
                let request = serde_json::json!({
                    "sessionId": acp_session_id.to_string(),
                    "configId": config_id,
                    "value": value,
                });
                manager.emit(acp_protocol_event(
                    &sessio_runtime_session_id,
                    "client_to_agent",
                    "request",
                    "session/set_config_option",
                    Some(acp_session_id.to_string()),
                    current_turn(&current_turn_id),
                    None,
                    None,
                    &request,
                )?)?;
                let response = connection
                    .send_request(UntypedMessage::new("session/set_config_option", request)?)
                    .block_task()
                    .await?;
                manager.emit(acp_protocol_event(
                    &sessio_runtime_session_id,
                    "agent_to_client",
                    "response",
                    "session/set_config_option",
                    Some(acp_session_id.to_string()),
                    current_turn(&current_turn_id),
                    None,
                    None,
                    &response,
                )?)?;
            }
        }
    }
    Ok(())
}

fn spawn_prompt_task(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    acp_session_id: SessionId,
    connection: ConnectionTo<AcpAgentRole>,
    current_turn_id: Arc<Mutex<Option<String>>>,
    turn_id: String,
    input: AgentInput,
) {
    tauri::async_runtime::spawn(async move {
        log::info!(
            "[sessio-runtime:acp:prompt] session={} turn={} text={:?}",
            sessio_runtime_session_id,
            turn_id,
            input.text
        );
        let request = PromptRequest::new(
            acp_session_id.clone(),
            vec![ContentBlock::Text(TextContent::new(input.text))],
        );
        match acp_protocol_event(
            &sessio_runtime_session_id,
            "client_to_agent",
            "request",
            "session/prompt",
            Some(acp_session_id.to_string()),
            Some(turn_id.clone()),
            None,
            None,
            &request,
        ) {
            Ok(event) => {
                let _ = manager.emit(event);
            }
            Err(error) => {
                log::warn!("[sessio-runtime:acp:prompt-request-event] {error}");
            }
        }
        let result = connection.send_request(request).block_task().await;

        match result {
            Ok(response) if response.stop_reason == StopReason::Cancelled => {
                match acp_protocol_event(
                    &sessio_runtime_session_id,
                    "agent_to_client",
                    "response",
                    "session/prompt",
                    Some(acp_session_id.to_string()),
                    Some(turn_id.clone()),
                    None,
                    None,
                    &response,
                ) {
                    Ok(event) => {
                        let _ = manager.emit(event);
                    }
                    Err(error) => {
                        log::warn!("[sessio-runtime:acp:prompt-response-event] {error}");
                    }
                }
                log::info!(
                    "[sessio-runtime:acp:prompt-response] session={} turn={} stop_reason={:?}",
                    sessio_runtime_session_id,
                    turn_id,
                    response.stop_reason
                );
                clear_current_turn(&current_turn_id, &turn_id);
                let _ = manager.cancel_turn_if_active(&sessio_runtime_session_id, &turn_id);
            }
            Ok(response) => {
                match acp_protocol_event(
                    &sessio_runtime_session_id,
                    "agent_to_client",
                    "response",
                    "session/prompt",
                    Some(acp_session_id.to_string()),
                    Some(turn_id.clone()),
                    None,
                    None,
                    &response,
                ) {
                    Ok(event) => {
                        let _ = manager.emit(event);
                    }
                    Err(error) => {
                        log::warn!("[sessio-runtime:acp:prompt-response-event] {error}");
                    }
                }
                log::info!(
                    "[sessio-runtime:acp:prompt-response] session={} turn={} stop_reason={:?}",
                    sessio_runtime_session_id,
                    turn_id,
                    response.stop_reason
                );
                tauri::async_runtime::spawn_blocking({
                    let manager = manager.clone();
                    let sessio_runtime_session_id = sessio_runtime_session_id.clone();
                    let current_turn_id = current_turn_id.clone();
                    let turn_id = turn_id.clone();
                    move || {
                        std::thread::sleep(std::time::Duration::from_millis(250));
                        clear_current_turn(&current_turn_id, &turn_id);
                        let _ = manager.complete_turn(&sessio_runtime_session_id, &turn_id);
                    }
                });
            }
            Err(error) => {
                log::warn!(
                    "[sessio-runtime:acp:prompt-error] session={} turn={} error={}",
                    sessio_runtime_session_id,
                    turn_id,
                    error
                );
                clear_current_turn(&current_turn_id, &turn_id);
                let _ = manager.fail_turn(
                    &sessio_runtime_session_id,
                    &turn_id,
                    RuntimeError::new("acp_prompt_error", error.to_string()),
                );
            }
        }
    });
}

fn runtime_capabilities_from_acp(
    capabilities: &agent_client_protocol::schema::AgentCapabilities,
) -> RuntimeCapabilitySet {
    RuntimeCapabilitySet {
        supports_cancel: true,
        supports_permissions: true,
        supports_tool_deltas: true,
        supports_resume: capabilities.session_capabilities.resume.is_some(),
        supports_fork: capabilities.session_capabilities.fork.is_some(),
        supports_attachments: capabilities.prompt_capabilities.image
            || capabilities.prompt_capabilities.audio
            || capabilities.prompt_capabilities.embedded_context,
        supports_modes: false,
    }
}

fn session_update_type(update: &SessionUpdate) -> &'static str {
    match update {
        SessionUpdate::UserMessageChunk(_) => "user_message_chunk",
        SessionUpdate::AgentMessageChunk(_) => "agent_message_chunk",
        SessionUpdate::AgentThoughtChunk(_) => "agent_thought_chunk",
        SessionUpdate::ToolCall(_) => "tool_call",
        SessionUpdate::ToolCallUpdate(_) => "tool_call_update",
        SessionUpdate::Plan(_) => "plan",
        SessionUpdate::AvailableCommandsUpdate(_) => "available_commands",
        SessionUpdate::CurrentModeUpdate(_) => "current_mode",
        SessionUpdate::ConfigOptionUpdate(_) => "config_options",
        SessionUpdate::SessionInfoUpdate(_) => "session_info",
        _ => "unknown",
    }
}

fn current_turn(current_turn_id: &Arc<Mutex<Option<String>>>) -> Option<String> {
    current_turn_id.lock().ok().and_then(|guard| guard.clone())
}

fn set_current_turn(current_turn_id: &Arc<Mutex<Option<String>>>, turn_id: Option<String>) {
    if let Ok(mut guard) = current_turn_id.lock() {
        *guard = turn_id;
    }
}

fn clear_current_turn(current_turn_id: &Arc<Mutex<Option<String>>>, turn_id: &str) {
    if let Ok(mut guard) = current_turn_id.lock() {
        if guard.as_deref() == Some(turn_id) {
            *guard = None;
        }
    }
}

fn json_id_to_string(value: serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value,
        other => other.to_string(),
    }
}

fn default_acp_command(agent: Agent) -> &'static str {
    match agent {
        Agent::Codex => "npx -y @zed-industries/codex-acp@latest",
        Agent::Claude => "npx -y @zed-industries/claude-code-acp@latest",
        Agent::Gemini => "npx -y -- @google/gemini-cli@latest --experimental-acp",
    }
}

fn acp_internal_error(error: impl ToString) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(error.to_string())
}

pub fn transport_requested(options: &RuntimeMetadata) -> RuntimeTransportKind {
    options
        .get("transport")
        .and_then(|value| value.as_str())
        .map(transport_from_str)
        .unwrap_or(RuntimeTransportKind::Fake)
}

pub fn transport_from_config(config: &AgentRuntimeConfig) -> RuntimeTransportKind {
    config
        .transport
        .as_deref()
        .map(transport_from_str)
        .unwrap_or(RuntimeTransportKind::Fake)
}

fn transport_from_str(transport: &str) -> RuntimeTransportKind {
    match transport {
        "acp" => RuntimeTransportKind::Acp,
        "cliStreamJson" => RuntimeTransportKind::CliStreamJson,
        "plainCli" => RuntimeTransportKind::PlainCli,
        "fake" => RuntimeTransportKind::Fake,
        _ => RuntimeTransportKind::Fake,
    }
}
