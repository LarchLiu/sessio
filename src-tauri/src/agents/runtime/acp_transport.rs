use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{
    CancelNotification, ContentBlock, EmbeddedResource, EmbeddedResourceResource,
    ForkSessionRequest, ImageContent, InitializeRequest, LoadSessionRequest, NewSessionRequest,
    PromptRequest, ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, ResumeSessionRequest, SessionId, SessionNotification, SessionUpdate,
    StopReason, TextContent, TextResourceContents,
};
use agent_client_protocol::{AcpAgent, Agent as AcpAgentRole, ConnectionTo, UntypedMessage};
use anyhow::{Context, Result};
use base64::Engine;

use super::acp::{
    acp_protocol_event, convert_session_notification, permission_response_from_decision,
};
use super::manager::{RuntimeManager, RuntimePermissionDecision};
use super::types::{
    AgentAttachment, AgentAttachmentKind, AgentInput, AgentRuntimeSessionConfig,
    RuntimeCapabilitySet, RuntimeError, RuntimeMetadata, RuntimeTransportKind,
};
use crate::models::Agent;

#[derive(Debug, Clone)]
pub struct AcpInitializeProbe {
    pub protocol_version: String,
    pub raw_initialize_response_json: String,
    pub raw_capabilities_json: String,
}

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
    Prompt {
        turn_id: String,
        input: AgentInput,
    },
    Cancel {
        turn_id: String,
    },
    SetConfigOption {
        config_id: String,
        value: serde_json::Value,
    },
}

pub fn command_from_options(agent: Agent, options: &RuntimeMetadata) -> String {
    options
        .get("acpCommand")
        .or_else(|| options.get("command"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| default_acp_command(agent).to_string())
}

pub fn spawn_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    agent: Agent,
    workspace_path: String,
    command: String,
    runtime_config: Option<AgentRuntimeSessionConfig>,
    start: AcpSessionStart,
) -> AcpSessionController {
    let (command_tx, command_rx) = tauri::async_runtime::channel(32);

    tauri::async_runtime::spawn({
        let manager = manager.clone();
        async move {
            if let Err(error) = run_session(
                manager.clone(),
                sessio_runtime_session_id.clone(),
                agent,
                workspace_path,
                command,
                runtime_config,
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

pub fn probe_capabilities(command: String) -> Result<RuntimeCapabilitySet> {
    tauri::async_runtime::block_on(async move {
        let agent = AcpAgent::from_str(&command)
            .with_context(|| format!("failed to parse ACP command: {command}"))?;
        agent_client_protocol::Client
            .builder()
            .name("sessio")
            .connect_with(agent, |connection: ConnectionTo<AcpAgentRole>| async move {
                let init = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                Ok::<RuntimeCapabilitySet, agent_client_protocol::Error>(
                    runtime_capabilities_from_acp(&init.agent_capabilities),
                )
            })
            .await
            .map_err(anyhow::Error::from)
    })
}

pub fn probe_initialize_response(command: String) -> Result<AcpInitializeProbe> {
    tauri::async_runtime::block_on(async move {
        let agent = AcpAgent::from_str(&command)
            .with_context(|| format!("failed to parse ACP command: {command}"))?;
        agent_client_protocol::Client
            .builder()
            .name("sessio")
            .connect_with(agent, |connection: ConnectionTo<AcpAgentRole>| async move {
                let init = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                let protocol_version = init.protocol_version.to_string();
                let raw_initialize_response_json =
                    serde_json::to_string(&init).map_err(|error| {
                        agent_client_protocol::Error::internal_error().data(error.to_string())
                    })?;
                let raw_capabilities_json = serde_json::to_string(&init.agent_capabilities)
                    .map_err(|error| {
                        agent_client_protocol::Error::internal_error().data(error.to_string())
                    })?;
                Ok::<AcpInitializeProbe, agent_client_protocol::Error>(AcpInitializeProbe {
                    protocol_version,
                    raw_initialize_response_json,
                    raw_capabilities_json,
                })
            })
            .await
            .map_err(anyhow::Error::from)
    })
}

pub fn runtime_capabilities_from_acp(
    capabilities: &agent_client_protocol::schema::AgentCapabilities,
) -> RuntimeCapabilitySet {
    let supports_image_attachments = capabilities.prompt_capabilities.image;
    let supports_audio_attachments = capabilities.prompt_capabilities.audio;
    let supports_embedded_context = capabilities.prompt_capabilities.embedded_context;
    RuntimeCapabilitySet {
        supports_cancel: true,
        supports_permissions: true,
        supports_tool_deltas: true,
        supports_load_session: capabilities.load_session,
        supports_resume: capabilities.session_capabilities.resume.is_some(),
        supports_fork: capabilities.session_capabilities.fork.is_some(),
        supports_image_attachments,
        supports_audio_attachments,
        supports_embedded_context,
        supports_attachments: supports_image_attachments
            || supports_audio_attachments
            || supports_embedded_context,
        supports_modes: false,
    }
}

async fn run_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    agent: Agent,
    workspace_path: String,
    command: String,
    runtime_config: Option<AgentRuntimeSessionConfig>,
    start: AcpSessionStart,
    command_rx: tauri::async_runtime::Receiver<AcpWorkerCommand>,
) -> Result<()> {
    let acp_agent = AcpAgent::from_str(&command)
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
                let update_type = session_update_type(&notification.update).to_string();
                let Some(turn_id) = current_turn(&notification_turn_id) else {
                    log::warn!(
                        "[sessio-runtime:acp:notification:session-level] session={} update={:?}",
                        notification_session_id,
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
                                None,
                                None,
                                Some(update_type),
                                &notification,
                            )
                            .map_err(acp_internal_error)?,
                        )
                        .map_err(acp_internal_error)?;
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
                            Some(update_type),
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
                let turn_id = current_turn(&permission_turn_id);
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
        .connect_with(acp_agent, move |connection: ConnectionTo<AcpAgentRole>| {
            let manager = manager.clone();
            let sessio_runtime_session_id = sessio_runtime_session_id.clone();
            let agent = agent;
            let workspace_path = workspace_path.clone();
            let start = start.clone();
            let current_turn_id = current_turn_id.clone();
            let runtime_config = runtime_config.clone();
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
                        let request =
                            new_session_request(agent, workspace_path, runtime_config.as_ref());
                        let session = connection
                            .send_request(request)
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
                apply_initial_session_config(
                    agent,
                    &manager,
                    &sessio_runtime_session_id,
                    &connection,
                    &acp_session_id,
                    runtime_config.as_ref(),
                )
                .await?;
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

fn new_session_request(
    agent: Agent,
    workspace_path: String,
    config: Option<&AgentRuntimeSessionConfig>,
) -> NewSessionRequest {
    let mut request = NewSessionRequest::new(workspace_path);
    if agent != Agent::Claude {
        return request;
    }
    let Some(config) = config else {
        return request;
    };
    let mut options = serde_json::Map::new();
    if let Some(model) = config.model.as_deref() {
        options.insert(
            "model".to_string(),
            serde_json::Value::String(model.to_string()),
        );
    }
    if let Some(permission_mode) = config.permission_mode.as_deref() {
        options.insert(
            "permissionMode".to_string(),
            serde_json::Value::String(permission_mode.to_string()),
        );
    }
    if let Some(effort) = config.effort.as_deref() {
        options.insert(
            "effort".to_string(),
            serde_json::Value::String(effort.to_string()),
        );
    }
    if options.is_empty() {
        return request;
    }
    let mut meta = serde_json::Map::new();
    meta.insert(
        "claudeCode".to_string(),
        serde_json::json!({
            "options": options,
        }),
    );
    request.meta = Some(meta);
    request
}

async fn apply_initial_session_config(
    agent: Agent,
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    connection: &ConnectionTo<AcpAgentRole>,
    acp_session_id: &SessionId,
    config: Option<&AgentRuntimeSessionConfig>,
) -> Result<(), agent_client_protocol::Error> {
    let Some(config) = config else {
        return Ok(());
    };
    if let Some(model) = config.model.as_deref() {
        if let Err(error) = send_session_config_request(
            manager,
            sessio_runtime_session_id,
            connection,
            acp_session_id,
            None,
            "model",
            serde_json::Value::String(model.to_string()),
        )
        .await
        {
            log::warn!(
                "[sessio-runtime:acp:initial-config-failed] session={} config=model value={} error={error}",
                sessio_runtime_session_id,
                model
            );
        }
    }
    if let Some(permission_mode) = config.permission_mode.as_deref() {
        if let Err(error) = send_session_config_request(
            manager,
            sessio_runtime_session_id,
            connection,
            acp_session_id,
            None,
            "mode",
            serde_json::Value::String(permission_mode.to_string()),
        )
        .await
        {
            log::warn!(
                "[sessio-runtime:acp:initial-config-failed] session={} config=mode value={} error={error}",
                sessio_runtime_session_id,
                permission_mode
            );
        }
    }
    if let Some(effort) = config.effort.as_deref() {
        let config_id = effort_config_id(agent);
        if let Err(error) = send_session_config_request(
            manager,
            sessio_runtime_session_id,
            connection,
            acp_session_id,
            None,
            config_id,
            serde_json::Value::String(effort.to_string()),
        )
        .await
        {
            log::warn!(
                "[sessio-runtime:acp:initial-config-failed] session={} config={} value={} error={error}",
                sessio_runtime_session_id,
                config_id,
                effort
            );
        }
    }
    Ok(())
}

fn effort_config_id(agent: Agent) -> &'static str {
    match agent {
        Agent::Codex => "reasoning_effort",
        Agent::Claude | Agent::Gemini => "effort",
    }
}

async fn send_session_config_request(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    connection: &ConnectionTo<AcpAgentRole>,
    acp_session_id: &SessionId,
    turn_id: Option<String>,
    config_id: &str,
    value: serde_json::Value,
) -> Result<(), agent_client_protocol::Error> {
    let (method, request) = session_config_message(acp_session_id, config_id, value)?;
    manager
        .emit(
            acp_protocol_event(
                sessio_runtime_session_id,
                "client_to_agent",
                "request",
                method,
                Some(acp_session_id.to_string()),
                turn_id.clone(),
                None,
                None,
                &request,
            )
            .map_err(acp_internal_error)?,
        )
        .map_err(acp_internal_error)?;
    let response = connection
        .send_request(UntypedMessage::new(method, request)?)
        .block_task()
        .await?;
    manager
        .emit(
            acp_protocol_event(
                sessio_runtime_session_id,
                "agent_to_client",
                "response",
                method,
                Some(acp_session_id.to_string()),
                turn_id,
                None,
                None,
                &response,
            )
            .map_err(acp_internal_error)?,
        )
        .map_err(acp_internal_error)?;
    Ok(())
}

fn session_config_message(
    acp_session_id: &SessionId,
    config_id: &str,
    value: serde_json::Value,
) -> Result<(&'static str, serde_json::Value), agent_client_protocol::Error> {
    match config_id {
        "model" => Ok((
            "session/set_model",
            serde_json::json!({
                "sessionId": acp_session_id.to_string(),
                "modelId": json_id_to_string(value),
            }),
        )),
        "mode" | "permission_mode" | "permissionMode" => Ok((
            "session/set_mode",
            serde_json::json!({
                "sessionId": acp_session_id.to_string(),
                "modeId": json_id_to_string(value),
            }),
        )),
        _ => Ok((
            "session/set_config_option",
            serde_json::json!({
                "sessionId": acp_session_id.to_string(),
                "configId": config_id,
                "value": json_id_to_string(value),
            }),
        )),
    }
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
                if let Err(error) = send_session_config_request(
                    &manager,
                    &sessio_runtime_session_id,
                    &connection,
                    &acp_session_id,
                    current_turn(&current_turn_id),
                    &config_id,
                    value,
                )
                .await
                {
                    log::warn!(
                        "[sessio-runtime:acp:set-config-failed] session={} config={} error={error}",
                        sessio_runtime_session_id,
                        config_id
                    );
                }
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
        let request = match prompt_request_from_input(acp_session_id.clone(), input) {
            Ok(request) => request,
            Err(error) => {
                log::warn!(
                    "[sessio-runtime:acp:prompt-attachment-error] session={} turn={} error={}",
                    sessio_runtime_session_id,
                    turn_id,
                    error
                );
                clear_current_turn(&current_turn_id, &turn_id);
                let _ = manager.fail_turn(
                    &sessio_runtime_session_id,
                    &turn_id,
                    RuntimeError::new("acp_attachment_error", error.to_string()),
                );
                return;
            }
        };
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
                        let _ = manager.complete_turn(&sessio_runtime_session_id, &turn_id);
                        clear_current_turn(&current_turn_id, &turn_id);
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

fn prompt_request_from_input(session_id: SessionId, input: AgentInput) -> Result<PromptRequest> {
    let mut prompt = vec![ContentBlock::Text(TextContent::new(input.text))];
    for attachment in input.attachments {
        prompt.push(content_block_from_attachment(&attachment)?);
    }
    Ok(PromptRequest::new(session_id, prompt))
}

fn content_block_from_attachment(attachment: &AgentAttachment) -> Result<ContentBlock> {
    match attachment.kind {
        AgentAttachmentKind::Image => image_content_block(attachment),
        AgentAttachmentKind::File => text_resource_content_block(attachment),
    }
}

fn image_content_block(attachment: &AgentAttachment) -> Result<ContentBlock> {
    let path = Path::new(&attachment.path);
    ensure_absolute_existing_file(path)?;
    let mime_type = image_mime_type(path)
        .or_else(|| {
            attachment
                .mime_type
                .as_deref()
                .filter(|mime| mime.starts_with("image/"))
        })
        .context("unsupported image attachment type")?
        .to_string();
    const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;
    ensure_file_size(path, MAX_IMAGE_BYTES, "image attachment is too large")?;
    let data =
        std::fs::read(path).with_context(|| format!("read image attachment {}", path.display()))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data);
    Ok(ContentBlock::Image(
        ImageContent::new(encoded, mime_type).uri(file_uri(path)),
    ))
}

fn text_resource_content_block(attachment: &AgentAttachment) -> Result<ContentBlock> {
    let path = Path::new(&attachment.path);
    ensure_absolute_existing_file(path)?;
    let mime_type = text_mime_type(path)
        .or_else(|| {
            attachment
                .mime_type
                .as_deref()
                .filter(|mime| mime.starts_with("text/"))
        })
        .context("unsupported file attachment type")?
        .to_string();
    const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
    ensure_file_size(path, MAX_TEXT_BYTES, "file attachment is too large")?;
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read text attachment {}", path.display()))?;
    let uri = file_uri(path);
    let marked_text = format!(
        "<sessio-upload-file uri=\"{}\" name=\"{}\" mimeType=\"{}\">\n{}\n</sessio-upload-file>",
        escape_xml_attr(&uri),
        escape_xml_attr(attachment_display_name(attachment, path)),
        escape_xml_attr(&mime_type),
        text
    );
    let resource = TextResourceContents::new(marked_text, uri).mime_type(mime_type);
    Ok(ContentBlock::Resource(EmbeddedResource::new(
        EmbeddedResourceResource::TextResourceContents(resource),
    )))
}

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn ensure_absolute_existing_file(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        anyhow::bail!("attachment path must be absolute: {}", path.display());
    }
    let metadata =
        std::fs::metadata(path).with_context(|| format!("read attachment {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("attachment is not a file: {}", path.display());
    }
    Ok(())
}

fn attachment_display_name<'a>(attachment: &'a AgentAttachment, path: &'a Path) -> &'a str {
    attachment
        .display_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .unwrap_or("attachment")
}

fn ensure_file_size(path: &Path, max_bytes: u64, message: &str) -> Result<()> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("read attachment metadata {}", path.display()))?
        .len();
    if size > max_bytes {
        anyhow::bail!("{message}: {} bytes", size);
    }
    Ok(())
}

fn file_uri(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy())
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match extension_lower(path).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("svg") => Some("image/svg+xml"),
        Some("bmp") => Some("image/bmp"),
        Some("heic") => Some("image/heic"),
        Some("heif") => Some("image/heif"),
        _ => None,
    }
}

fn text_mime_type(path: &Path) -> Option<&'static str> {
    match extension_lower(path).as_deref() {
        Some("txt") => Some("text/plain"),
        Some("md") | Some("markdown") => Some("text/markdown"),
        Some("rst") => Some("text/x-rst"),
        Some("json") | Some("jsonl") => Some("application/json"),
        Some("yaml") | Some("yml") => Some("application/yaml"),
        Some("toml") => Some("application/toml"),
        Some("xml") => Some("application/xml"),
        Some("csv") => Some("text/csv"),
        Some("html") | Some("htm") => Some("text/html"),
        Some("css") | Some("scss") | Some("sass") | Some("less") => Some("text/css"),
        Some("sh") | Some("zsh") | Some("bash") => Some("application/x-sh"),
        Some("sql") => Some("application/sql"),
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("mjs") | Some("cjs")
        | Some("py") | Some("rs") | Some("go") | Some("java") | Some("kt") | Some("swift")
        | Some("rb") | Some("php") | Some("c") | Some("h") | Some("cpp") | Some("hpp")
        | Some("cs") | Some("lua") | Some("pl") | Some("r") | Some("ex") | Some("exs")
        | Some("erl") | Some("clj") | Some("scala") | Some("dart") | Some("vue")
        | Some("svelte") => Some("text/plain"),
        _ => match path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase())
            .as_deref()
        {
            Some("dockerfile") | Some(".gitignore") | Some(".env") => Some("text/plain"),
            _ => None,
        },
    }
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_attachment_becomes_embedded_resource() {
        let path =
            std::env::temp_dir().join(format!("sessio-acp-attachment-{}.md", std::process::id()));
        std::fs::write(&path, "# Context\nhello").expect("write temp text attachment");

        let attachment = AgentAttachment {
            path: path.to_string_lossy().to_string(),
            mime_type: None,
            kind: AgentAttachmentKind::File,
            display_name: None,
        };
        let block = content_block_from_attachment(&attachment).expect("content block");

        match block {
            ContentBlock::Resource(resource) => match resource.resource {
                EmbeddedResourceResource::TextResourceContents(contents) => {
                    assert!(contents.text.contains("<sessio-upload-file "));
                    assert!(contents.text.contains("name=\"sessio-acp-attachment-"));
                    assert!(contents.text.contains("# Context\nhello"));
                    assert!(contents.text.contains("</sessio-upload-file>"));
                    assert_eq!(contents.mime_type.as_deref(), Some("text/markdown"));
                    assert!(contents.uri.starts_with("file://"));
                }
                _ => panic!("expected text resource contents"),
            },
            _ => panic!("expected resource block"),
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn text_attachment_uses_display_name_when_available() {
        let path =
            std::env::temp_dir().join(format!("sessio-acp-display-name-{}.md", std::process::id()));
        std::fs::write(&path, "# Cross context").expect("write temp text attachment");

        let attachment = AgentAttachment {
            path: path.to_string_lossy().to_string(),
            mime_type: None,
            kind: AgentAttachmentKind::File,
            display_name: Some("sessio-cross-context.md".to_string()),
        };
        let block = content_block_from_attachment(&attachment).expect("content block");

        match block {
            ContentBlock::Resource(resource) => match resource.resource {
                EmbeddedResourceResource::TextResourceContents(contents) => {
                    assert!(contents.text.contains("name=\"sessio-cross-context.md\""));
                }
                _ => panic!("expected text resource contents"),
            },
            _ => panic!("expected resource block"),
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn image_attachment_becomes_image_block() {
        let path =
            std::env::temp_dir().join(format!("sessio-acp-attachment-{}.png", std::process::id()));
        std::fs::write(&path, [1_u8, 2, 3, 4]).expect("write temp image attachment");

        let attachment = AgentAttachment {
            path: path.to_string_lossy().to_string(),
            mime_type: None,
            kind: AgentAttachmentKind::Image,
            display_name: None,
        };
        let block = content_block_from_attachment(&attachment).expect("content block");

        match block {
            ContentBlock::Image(image) => {
                assert_eq!(image.mime_type, "image/png");
                assert_eq!(image.data, "AQIDBA==");
                assert!(image
                    .uri
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with("file://"));
            }
            _ => panic!("expected image block"),
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn codex_config_keeps_acp_adapter_command_clean() {
        let mut options = RuntimeMetadata::default();
        options.insert(
            "command".to_string(),
            serde_json::Value::String("npx -y @zed-industries/codex-acp@latest".to_string()),
        );
        let command = command_from_options(Agent::Codex, &options);

        assert_eq!(command, "npx -y @zed-industries/codex-acp@latest");
    }

    #[test]
    fn model_config_uses_session_set_model_params() {
        let session_id = SessionId::new("session-123");
        let (method, request) = session_config_message(
            &session_id,
            "model",
            serde_json::Value::String("gpt-5-codex".to_string()),
        )
        .expect("session config message");

        assert_eq!(method, "session/set_model");
        assert_eq!(
            request,
            serde_json::json!({
                "sessionId": "session-123",
                "modelId": "gpt-5-codex",
            })
        );
    }

    #[test]
    fn permission_config_uses_session_set_mode_params() {
        let session_id = SessionId::new("session-123");
        let (method, request) = session_config_message(
            &session_id,
            "mode",
            serde_json::Value::String("acceptEdits".to_string()),
        )
        .expect("session config message");

        assert_eq!(method, "session/set_mode");
        assert_eq!(
            request,
            serde_json::json!({
                "sessionId": "session-123",
                "modeId": "acceptEdits",
            })
        );
    }

    #[test]
    fn other_config_uses_session_set_config_option_params() {
        let session_id = SessionId::new("session-123");
        let (method, request) = session_config_message(
            &session_id,
            "effort",
            serde_json::Value::String("high".to_string()),
        )
        .expect("session config message");

        assert_eq!(method, "session/set_config_option");
        assert_eq!(
            request,
            serde_json::json!({
                "sessionId": "session-123",
                "configId": "effort",
                "value": "high",
            })
        );
    }

    #[test]
    fn codex_effort_uses_reasoning_effort_config_id() {
        assert_eq!(effort_config_id(Agent::Codex), "reasoning_effort");
    }

    #[test]
    fn claude_effort_uses_effort_config_id() {
        assert_eq!(effort_config_id(Agent::Claude), "effort");
    }

    #[test]
    fn claude_config_keeps_acp_adapter_command_clean() {
        let mut options = RuntimeMetadata::default();
        options.insert(
            "command".to_string(),
            serde_json::Value::String("npx -y @zed-industries/claude-code-acp@latest".to_string()),
        );
        let command = command_from_options(Agent::Claude, &options);

        assert_eq!(command, "npx -y @zed-industries/claude-code-acp@latest");
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

pub(crate) fn default_acp_command(agent: Agent) -> &'static str {
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
        .unwrap_or(RuntimeTransportKind::Acp)
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
