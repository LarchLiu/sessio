use std::str::FromStr;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::{
    CancelNotification, ContentBlock, InitializeRequest, LoadSessionRequest, NewSessionRequest,
    PromptRequest, ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SessionId, SessionNotification, StopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent as AcpAgentRole, ConnectionTo};
use anyhow::{Context, Result};

use super::acp::{convert_session_notification, permission_response_from_decision};
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
}

#[derive(Debug, Clone)]
pub enum AcpSessionStart {
    New,
    Load { agent_session_id: String },
}

#[derive(Debug)]
enum AcpWorkerCommand {
    Prompt { turn_id: String, input: AgentInput },
    Cancel { turn_id: String },
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
                let turn_id = current_turn(&notification_turn_id);
                let Some(turn_id) = turn_id else {
                    return Ok(());
                };
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
                    return responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    ));
                };
                let request_id = json_id_to_string(responder.id());
                let manager = permission_manager.clone();
                let sessio_runtime_session_id = permission_session_id.clone();
                let request_for_ui = request.clone();
                let request_id_for_ui = request_id.clone();
                let decision = tauri::async_runtime::spawn_blocking(move || {
                    manager.request_permission_from_acp(
                        &request_for_ui,
                        &sessio_runtime_session_id,
                        &turn_id,
                        &request_id_for_ui,
                    )
                })
                .await
                .map_err(acp_internal_error)?
                .map_err(acp_internal_error)?;

                let response = match decision {
                    RuntimePermissionDecision::Selected { approved } => {
                        permission_response_from_decision(&request, approved)
                    }
                    RuntimePermissionDecision::Cancelled => {
                        RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
                    }
                };
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
                let capabilities = runtime_capabilities_from_acp(&init.agent_capabilities);
                let acp_session_id = match start {
                    AcpSessionStart::New => {
                        let session = connection
                            .send_request(NewSessionRequest::new(workspace_path))
                            .block_task()
                            .await?;
                        session.session_id
                    }
                    AcpSessionStart::Load { agent_session_id } => {
                        if !init.agent_capabilities.load_session {
                            return Err(acp_internal_error(format!(
                                "ACP agent does not support session/load for session {agent_session_id}"
                            )));
                        }
                        let acp_session_id = SessionId::new(agent_session_id);
                        connection
                            .send_request(LoadSessionRequest::new(
                                acp_session_id.clone(),
                                workspace_path,
                            ))
                            .block_task()
                            .await?;
                        acp_session_id
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
                connection
                    .send_notification(CancelNotification::new(acp_session_id.clone()))
                    .map_err(anyhow::Error::from)?;
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
        let result = connection
            .send_request(PromptRequest::new(
                acp_session_id,
                vec![ContentBlock::Text(TextContent::new(input.text))],
            ))
            .block_task()
            .await;

        clear_current_turn(&current_turn_id, &turn_id);

        match result {
            Ok(response) if response.stop_reason == StopReason::Cancelled => {
                let _ = manager.cancel_turn_if_active(&sessio_runtime_session_id, &turn_id);
            }
            Ok(_) => {
                let _ = manager.complete_turn(&sessio_runtime_session_id, &turn_id);
            }
            Err(error) => {
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
        supports_attachments: capabilities.prompt_capabilities.image
            || capabilities.prompt_capabilities.audio
            || capabilities.prompt_capabilities.embedded_context,
        supports_modes: false,
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
