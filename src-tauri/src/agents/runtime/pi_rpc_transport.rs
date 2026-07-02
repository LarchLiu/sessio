use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child as TokioChild, Command as TokioCommand};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use super::manager::RuntimeManager;
use super::types::{
    AgentAttachment, AgentAttachmentKind, AgentInput, AgentRuntimeEventPayload,
    AgentRuntimeSessionConfig, ComputerUseInjection, RuntimeCapabilitySet, RuntimeError,
    RuntimeMetadata,
};
use crate::app_paths;
use crate::models::Agent;
use crate::turns::history_session_update_message;

const PI_RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const PI_RPC_COMMAND_CHANNEL_SIZE: usize = 32;

#[derive(Debug, Clone)]
pub struct PiRpcSessionController {
    command_tx: tauri::async_runtime::Sender<PiRpcWorkerCommand>,
}

impl PiRpcSessionController {
    pub fn send_prompt(&self, turn_id: String, input: AgentInput) -> Result<()> {
        self.command_tx
            .try_send(PiRpcWorkerCommand::Prompt { turn_id, input })
            .map_err(|error| anyhow::anyhow!("failed to queue Pi RPC prompt: {error}"))
    }

    pub fn cancel_turn(&self, turn_id: String) -> Result<()> {
        self.command_tx
            .try_send(PiRpcWorkerCommand::Cancel { turn_id })
            .map_err(|error| anyhow::anyhow!("failed to queue Pi RPC cancellation: {error}"))
    }

    pub fn set_config_option(&self, config_id: String, value: Value) -> Result<()> {
        self.command_tx
            .try_send(PiRpcWorkerCommand::SetConfigOption { config_id, value })
            .map_err(|error| anyhow::anyhow!("failed to queue Pi RPC config update: {error}"))
    }
}

#[derive(Debug, Clone)]
pub enum PiRpcSessionStart {
    New,
    Load { agent_session_id: String },
    Resume { agent_session_id: String },
    Fork { source_session_id: String },
}

#[derive(Debug)]
enum PiRpcWorkerCommand {
    Prompt { turn_id: String, input: AgentInput },
    Cancel { turn_id: String },
    SetConfigOption { config_id: String, value: Value },
}

pub struct PiRpcSessionSpec {
    pub agent: Agent,
    pub workspace_path: String,
    pub command: String,
    pub runtime_config: Option<AgentRuntimeSessionConfig>,
    pub computer_use: Option<PiRpcComputerUseExtension>,
    pub start: PiRpcSessionStart,
}

#[derive(Debug, Clone)]
pub struct PiRpcComputerUseExtension {
    pub injection: ComputerUseInjection,
    pub extension_path: PathBuf,
    pub sessio_runtime_session_id: String,
}

pub struct PiRpcSessionWorker {
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    spec: PiRpcSessionSpec,
    command_rx: tauri::async_runtime::Receiver<PiRpcWorkerCommand>,
}

impl PiRpcSessionWorker {
    pub fn start(self) {
        tauri::async_runtime::spawn(async move {
            let PiRpcSessionWorker {
                manager,
                sessio_runtime_session_id,
                spec,
                command_rx,
            } = self;
            if let Err(error) = run_session(
                manager.clone(),
                sessio_runtime_session_id.clone(),
                spec,
                command_rx,
            )
            .await
            {
                let _ = manager.fail_session_start(&sessio_runtime_session_id, error.to_string());
            }
        });
    }
}

#[derive(Debug, Clone)]
pub struct PiRpcState {
    pub session_id: Option<String>,
    pub session_file: Option<String>,
    pub session_name: Option<String>,
    pub model: Option<String>,
    pub model_raw: Option<Value>,
    pub thinking_level: Option<String>,
    pub raw: Value,
}

pub fn default_pi_rpc_command() -> String {
    "pi --mode rpc".to_string()
}

pub fn command_from_options(options: &RuntimeMetadata) -> String {
    options
        .get("piRpcCommand")
        .or_else(|| options.get("command"))
        .and_then(Value::as_str)
        .map(ensure_rpc_mode)
        .unwrap_or_else(default_pi_rpc_command)
}

pub fn runtime_capabilities() -> RuntimeCapabilitySet {
    RuntimeCapabilitySet {
        supports_cancel: true,
        supports_permissions: false,
        supports_tool_deltas: true,
        supports_load_session: true,
        supports_resume: true,
        supports_fork: false,
        supports_image_attachments: true,
        supports_audio_attachments: false,
        supports_embedded_context: true,
        supports_attachments: true,
        supports_modes: false,
        // Pi is not an ACP agent and exposes no MCP server channel, but it does
        // have a native extension system (`pi.registerTool`) that Sessio can
        // target — so it is injectable via the `native_extension` path.
        mcp_injection: crate::agents::runtime::types::McpInjectionCapabilities {
            http: false,
            sse: false,
            acp: false,
            native_extension: true,
        },
    }
}

pub fn spawn_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    spec: PiRpcSessionSpec,
) -> PiRpcSessionController {
    let (controller, worker) = prepare_session(manager, sessio_runtime_session_id, spec);
    worker.start();
    controller
}

pub fn prepare_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    spec: PiRpcSessionSpec,
) -> (PiRpcSessionController, PiRpcSessionWorker) {
    let (command_tx, command_rx) = tauri::async_runtime::channel(PI_RPC_COMMAND_CHANNEL_SIZE);
    (
        PiRpcSessionController { command_tx },
        PiRpcSessionWorker {
            manager,
            sessio_runtime_session_id,
            spec,
            command_rx,
        },
    )
}

async fn run_session(
    manager: RuntimeManager,
    sessio_runtime_session_id: String,
    spec: PiRpcSessionSpec,
    mut command_rx: tauri::async_runtime::Receiver<PiRpcWorkerCommand>,
) -> Result<()> {
    let PiRpcSessionSpec {
        agent,
        workspace_path,
        command,
        runtime_config,
        computer_use,
        start,
    } = spec;
    let spawned = spawn_pi_rpc_process(&command, &workspace_path, computer_use.as_ref())
        .with_context(|| format!("failed to spawn Pi RPC command in workspace {workspace_path}"))?;
    let SpawnedPiRpcProcess {
        stdin,
        stdout,
        stderr,
        child,
        group,
    } = spawned;
    let stderr_log = Arc::new(Mutex::new(String::new()));
    let (event_tx, mut event_rx) = tauri::async_runtime::channel(128);
    let client = PiRpcClient::new(stdin, event_tx.clone());
    spawn_stdout_reader(stdout, client.pending.clone(), event_tx);
    spawn_stderr_reader(stderr, stderr_log.clone());

    let mut child = TokioChildGuard { child, group };
    let current_turn_id = Arc::new(Mutex::new(None::<String>));
    let current_turn_had_text = Arc::new(Mutex::new(false));
    let mut state = initialize_pi_session(
        &manager,
        &sessio_runtime_session_id,
        &client,
        agent,
        runtime_config,
        start,
    )
    .await?;
    let agent_runtime_session_id = state
        .session_id
        .clone()
        .or_else(|| state.session_file.clone())
        .unwrap_or_else(|| sessio_runtime_session_id.clone());
    manager.mark_session_ready(
        &sessio_runtime_session_id,
        agent_runtime_session_id,
        runtime_capabilities(),
    )?;
    emit_session_state_updates(&manager, &sessio_runtime_session_id, &client, &state).await;

    loop {
        tokio::select! {
            status = child.wait() => {
                let status = status.context("wait for Pi RPC child")?;
                if !status.success() {
                    let detail = collected_stderr(&stderr_log);
                    let message = if detail.is_empty() {
                        format!("Pi RPC process exited with {status}")
                    } else {
                        format!("Pi RPC process exited with {status}: {detail}")
                    };
                    if let Some(turn_id) = current_turn(&current_turn_id) {
                        let _ = manager.fail_turn(
                            &sessio_runtime_session_id,
                            &turn_id,
                            RuntimeError::new("pi_rpc_process_exited", message),
                        );
                    } else {
                        return Err(anyhow::anyhow!(message));
                    }
                }
                break;
            }
            command = command_rx.recv() => {
                let Some(command) = command else {
                    break;
                };
                handle_worker_command(
                    &manager,
                    &sessio_runtime_session_id,
                    &client,
                    &current_turn_id,
                    &current_turn_had_text,
                    command,
                )
                .await;
            }
            incoming = event_rx.recv() => {
                let Some(incoming) = incoming else {
                    continue;
                };
                if let Some(next_state) = handle_incoming_event(
                    &manager,
                    &sessio_runtime_session_id,
                    &current_turn_id,
                    &current_turn_had_text,
                    incoming,
                )? {
                    state = next_state;
                    emit_session_update(&manager, &sessio_runtime_session_id, "session_info", session_info_update(&state));
                }
            }
        }
    }

    let _ = manager.emit(AgentRuntimeEventPayload::SessionEnded {
        sessio_runtime_session_id,
    });
    Ok(())
}

async fn initialize_pi_session(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    client: &PiRpcClient,
    agent: Agent,
    runtime_config: Option<AgentRuntimeSessionConfig>,
    start: PiRpcSessionStart,
) -> Result<PiRpcState> {
    let _initial_state = request_state(client).await.ok();
    match start {
        PiRpcSessionStart::New => {
            let _ = client.request("new_session", json!({})).await;
        }
        PiRpcSessionStart::Load { agent_session_id }
        | PiRpcSessionStart::Resume { agent_session_id } => {
            switch_to_session(client, &agent_session_id).await?;
        }
        PiRpcSessionStart::Fork { source_session_id } => {
            log::info!(
                "[sessio-runtime:pi-rpc] fork requested for {}, starting a new Pi session",
                source_session_id
            );
            let _ = client.request("new_session", json!({})).await;
        }
    }

    if let Some(config) = runtime_config {
        apply_runtime_config(client, agent, config).await;
    }

    let state = request_state(client).await?;
    emit_session_update(
        manager,
        sessio_runtime_session_id,
        "session_info",
        session_info_update(&state),
    );
    Ok(state)
}

async fn switch_to_session(client: &PiRpcClient, agent_session_id: &str) -> Result<()> {
    let session_file = find_pi_session_file(agent_session_id).ok().flatten();
    let mut params = serde_json::Map::new();
    if let Some(path) = session_file {
        params.insert(
            "sessionPath".to_string(),
            Value::String(path.to_string_lossy().to_string()),
        );
    } else {
        params.insert(
            "sessionPath".to_string(),
            Value::String(agent_session_id.to_string()),
        );
    }
    let _ = client
        .request("switch_session", Value::Object(params))
        .await?;
    Ok(())
}

async fn request_state(client: &PiRpcClient) -> Result<PiRpcState> {
    let value = client.request("get_state", json!({})).await?;
    Ok(PiRpcState {
        session_id: string_field(&value, "sessionId")
            .or_else(|| string_field(&value, "session_id")),
        session_file: string_field(&value, "sessionFile")
            .or_else(|| string_field(&value, "session_file")),
        session_name: string_field(&value, "sessionName")
            .or_else(|| string_field(&value, "session_name")),
        model: model_id_from_state(&value),
        model_raw: value.get("model").cloned(),
        thinking_level: string_field(&value, "thinkingLevel")
            .or_else(|| string_field(&value, "thinking_level")),
        raw: value,
    })
}

async fn apply_runtime_config(
    client: &PiRpcClient,
    _agent: Agent,
    config: AgentRuntimeSessionConfig,
) {
    if let Some(model) = config
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let _ = client
            .request("set_model", model_request_params(model))
            .await;
    }
    if let Some(level) = config
        .effort
        .as_deref()
        .map(normalize_thinking_level)
        .filter(|value| !value.is_empty())
    {
        let _ = client
            .request(
                "set_thinking_level",
                json!({ "thinkingLevel": level, "level": level }),
            )
            .await;
    }
}

async fn emit_session_state_updates(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    client: &PiRpcClient,
    state: &PiRpcState,
) {
    if let Ok(value) = client.request("get_commands", json!({})).await {
        let commands = value
            .get("commands")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        emit_session_update(
            manager,
            sessio_runtime_session_id,
            "available_commands",
            json!({
                "sessionUpdate": "available_commands_update",
                "availableCommands": commands.into_iter().map(normalize_command_update).collect::<Vec<_>>(),
            }),
        );
    }

    let models = client
        .request("get_available_models", json!({}))
        .await
        .ok()
        .and_then(|value| {
            value
                .get("models")
                .or_else(|| value.get("availableModels"))
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default();
    let config_options = pi_config_options(state, models);
    emit_session_update(
        manager,
        sessio_runtime_session_id,
        "config_options",
        json!({
            "sessionUpdate": "config_options_update",
            "configOptions": config_options,
        }),
    );
}

async fn handle_worker_command(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    client: &PiRpcClient,
    current_turn_id: &Arc<Mutex<Option<String>>>,
    current_turn_had_text: &Arc<Mutex<bool>>,
    command: PiRpcWorkerCommand,
) {
    match command {
        PiRpcWorkerCommand::Prompt { turn_id, input } => {
            set_current_turn(current_turn_id, Some(turn_id.clone()));
            set_turn_had_text(current_turn_had_text, false);
            match prompt_params_from_input(input) {
                Ok(params) => {
                    let client = client.clone();
                    let manager = manager.clone();
                    let sessio_runtime_session_id = sessio_runtime_session_id.to_string();
                    let current_turn_id = current_turn_id.clone();
                    let current_turn_had_text = current_turn_had_text.clone();
                    tauri::async_runtime::spawn(async move {
                        match client.request_no_timeout("prompt", params).await {
                            Ok(response) => {
                                if let Err(error) = handle_prompt_response(
                                    &manager,
                                    &sessio_runtime_session_id,
                                    &turn_id,
                                    &current_turn_id,
                                    &current_turn_had_text,
                                    &response,
                                ) {
                                    let _ = manager.fail_turn(
                                        &sessio_runtime_session_id,
                                        &turn_id,
                                        RuntimeError::new(
                                            "pi_rpc_prompt_response_error",
                                            error.to_string(),
                                        ),
                                    );
                                }
                            }
                            Err(error) => {
                                clear_current_turn(&current_turn_id, &turn_id);
                                let _ = manager.fail_turn(
                                    &sessio_runtime_session_id,
                                    &turn_id,
                                    RuntimeError::new("pi_rpc_prompt_error", error.to_string()),
                                );
                            }
                        }
                    });
                }
                Err(error) => {
                    clear_current_turn(current_turn_id, &turn_id);
                    let _ = manager.fail_turn(
                        sessio_runtime_session_id,
                        &turn_id,
                        RuntimeError::new("pi_rpc_prompt_error", error.to_string()),
                    );
                }
            }
        }
        PiRpcWorkerCommand::Cancel { turn_id } => {
            let _ = client.request("abort", json!({})).await;
            clear_current_turn(current_turn_id, &turn_id);
            let _ = manager.cancel_turn_if_active(sessio_runtime_session_id, &turn_id);
        }
        PiRpcWorkerCommand::SetConfigOption { config_id, value } => {
            apply_config_option(client, &config_id, value).await;
        }
    }
}

fn handle_prompt_response(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    turn_id: &str,
    current_turn_id: &Arc<Mutex<Option<String>>>,
    current_turn_had_text: &Arc<Mutex<bool>>,
    response: &Value,
) -> Result<()> {
    let has_final_text = final_assistant_text_from_event(response).is_some();
    emit_final_text_if_missing(
        manager,
        sessio_runtime_session_id,
        turn_id,
        current_turn_had_text,
        response,
    )?;
    if has_final_text {
        clear_current_turn(current_turn_id, turn_id);
        manager.complete_turn(sessio_runtime_session_id, turn_id)?;
    }
    Ok(())
}

async fn apply_config_option(client: &PiRpcClient, config_id: &str, value: Value) {
    let text = value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string());
    match config_id {
        "model" => {
            let _ = client
                .request("set_model", model_request_params(&text))
                .await;
        }
        "effort" | "reasoningEffort" | "reasoning_effort" | "thinkingLevel" => {
            let level = normalize_thinking_level(&text);
            let _ = client
                .request(
                    "set_thinking_level",
                    json!({ "thinkingLevel": level, "level": level }),
                )
                .await;
        }
        _ => {}
    }
}

fn handle_incoming_event(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    current_turn_id: &Arc<Mutex<Option<String>>>,
    current_turn_had_text: &Arc<Mutex<bool>>,
    incoming: PiRpcIncoming,
) -> Result<Option<PiRpcState>> {
    let PiRpcIncoming::Event(value) = incoming;
    let event_type = rpc_event_type(&value);
    match event_type.as_deref() {
        Some("turn_start") => {
            set_turn_had_text(current_turn_had_text, false);
        }
        Some("turn_end") => {
            if let Some(turn_id) = current_turn(current_turn_id) {
                emit_final_text_if_missing(
                    manager,
                    sessio_runtime_session_id,
                    &turn_id,
                    current_turn_had_text,
                    &value,
                )?;
            }
        }
        Some("message_end") => {
            if let Some(turn_id) = current_turn(current_turn_id) {
                emit_final_text_if_missing(
                    manager,
                    sessio_runtime_session_id,
                    &turn_id,
                    current_turn_had_text,
                    &value,
                )?;
            }
        }
        Some("agent_end") => {
            if let Some(turn_id) = current_turn(current_turn_id) {
                emit_final_text_if_missing(
                    manager,
                    sessio_runtime_session_id,
                    &turn_id,
                    current_turn_had_text,
                    &value,
                )?;
                clear_current_turn(current_turn_id, &turn_id);
                manager.complete_turn(sessio_runtime_session_id, &turn_id)?;
            }
        }
        Some("message_update") => {
            handle_message_update(
                manager,
                sessio_runtime_session_id,
                current_turn_id,
                current_turn_had_text,
                &value,
            )?;
        }
        Some("tool_execution_start") => {
            if let Some(turn_id) = current_turn(current_turn_id) {
                manager.emit(tool_started_event(
                    sessio_runtime_session_id,
                    &turn_id,
                    &value,
                ))?;
            }
        }
        Some("tool_execution_update") => {
            if let Some(turn_id) = current_turn(current_turn_id) {
                manager.emit(tool_output_event(
                    sessio_runtime_session_id,
                    &turn_id,
                    &value,
                ))?;
            }
        }
        Some("tool_execution_end") => {
            if let Some(turn_id) = current_turn(current_turn_id) {
                if let Some(message) =
                    pi_tool_execution_end_acp_message(sessio_runtime_session_id, &turn_id, &value)
                {
                    manager.emit(AgentRuntimeEventPayload::AcpProtocolMessage {
                        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                        turn_id: Some(turn_id.clone()),
                        message,
                    })?;
                }
                manager.emit(tool_status_event(
                    sessio_runtime_session_id,
                    &turn_id,
                    &value,
                ))?;
            }
        }
        Some("state_update") | Some("state") => {
            return Ok(Some(PiRpcState {
                session_id: string_field(&value, "sessionId")
                    .or_else(|| string_field(&value, "session_id")),
                session_file: string_field(&value, "sessionFile")
                    .or_else(|| string_field(&value, "session_file")),
                session_name: string_field(&value, "sessionName")
                    .or_else(|| string_field(&value, "session_name")),
                model: model_id_from_state(&value),
                model_raw: value.get("model").cloned(),
                thinking_level: string_field(&value, "thinkingLevel")
                    .or_else(|| string_field(&value, "thinking_level")),
                raw: value,
            }));
        }
        Some(other) => {
            log::debug!("[sessio-runtime:pi-rpc:event] ignored event={other} raw={value}");
        }
        None => {
            log::debug!("[sessio-runtime:pi-rpc:event] ignored raw={value}");
        }
    }
    Ok(None)
}

fn handle_message_update(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    current_turn_id: &Arc<Mutex<Option<String>>>,
    current_turn_had_text: &Arc<Mutex<bool>>,
    value: &Value,
) -> Result<()> {
    let Some(turn_id) = current_turn(current_turn_id) else {
        return Ok(());
    };
    let message = value
        .get("assistantMessageEvent")
        .or_else(|| value.get("messageEvent"))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("assistantMessageEvent"))
        })
        .unwrap_or(value);
    let message_type = string_field(message, "type").unwrap_or_default();
    match message_type.as_str() {
        "text_delta" => {
            if let Some(text) = delta_text(message) {
                if !text.is_empty() {
                    set_turn_had_text(current_turn_had_text, true);
                }
                manager.emit(AgentRuntimeEventPayload::TextDelta {
                    sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                    turn_id,
                    text,
                })?;
            }
        }
        "thinking_delta" => {
            if let Some(text) = delta_text(message) {
                manager.emit(AgentRuntimeEventPayload::ReasoningDelta {
                    sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                    turn_id,
                    text,
                })?;
            }
        }
        "toolcall_delta" => {
            let tool_id = tool_id_from_value(message);
            manager.emit(AgentRuntimeEventPayload::ToolInputDelta {
                sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                turn_id,
                tool_id,
                delta: delta_text(message).unwrap_or_default(),
                data: Some(message.clone()),
            })?;
        }
        "error" => {
            let message = string_field(message, "message")
                .or_else(|| string_field(message, "error"))
                .unwrap_or_else(|| "Pi RPC message stream failed".to_string());
            manager.fail_turn(
                sessio_runtime_session_id,
                &turn_id,
                RuntimeError::new("pi_rpc_message_error", message),
            )?;
        }
        "done" => {}
        _ => {}
    }
    Ok(())
}

fn emit_final_text_if_missing(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    turn_id: &str,
    current_turn_had_text: &Arc<Mutex<bool>>,
    value: &Value,
) -> Result<()> {
    if turn_had_text(current_turn_had_text) {
        return Ok(());
    }
    let Some(text) = final_assistant_text_from_event(value) else {
        return Ok(());
    };
    if text.trim().is_empty() {
        return Ok(());
    }
    set_turn_had_text(current_turn_had_text, true);
    manager.emit(AgentRuntimeEventPayload::TextDelta {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        text,
    })?;
    Ok(())
}

fn tool_started_event(
    sessio_runtime_session_id: &str,
    turn_id: &str,
    value: &Value,
) -> AgentRuntimeEventPayload {
    let body = event_data(value);
    let tool_id = tool_id_from_value(body);
    AgentRuntimeEventPayload::ToolStarted {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        tool_id,
        name: string_field(body, "name")
            .or_else(|| string_field(body, "toolName"))
            .unwrap_or_else(|| "tool".to_string()),
        input: value_field(body, "input").or_else(|| value_field(body, "args")),
        data: value.clone(),
    }
}

fn tool_output_event(
    sessio_runtime_session_id: &str,
    turn_id: &str,
    value: &Value,
) -> AgentRuntimeEventPayload {
    let body = event_data(value);
    AgentRuntimeEventPayload::ToolOutputDelta {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        tool_id: tool_id_from_value(body),
        delta: value_field(body, "partialResult")
            .as_ref()
            .and_then(value_to_display_text)
            .or_else(|| string_field(body, "output"))
            .or_else(|| {
                value_field(body, "result")
                    .as_ref()
                    .and_then(value_to_display_text)
            })
            .unwrap_or_default(),
        data: Some(value.clone()),
    }
}

fn tool_status_event(
    sessio_runtime_session_id: &str,
    turn_id: &str,
    value: &Value,
) -> AgentRuntimeEventPayload {
    let body = event_data(value);
    let status = string_field(body, "status").unwrap_or_else(|| {
        if body.get("error").is_some() || body.get("isError").and_then(Value::as_bool) == Some(true)
        {
            "failed".to_string()
        } else {
            "completed".to_string()
        }
    });
    AgentRuntimeEventPayload::ToolStatusChanged {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: turn_id.to_string(),
        tool_id: tool_id_from_value(body),
        status,
        data: Some(value.clone()),
    }
}

fn pi_tool_execution_end_acp_message(
    _sessio_runtime_session_id: &str,
    _turn_id: &str,
    value: &Value,
) -> Option<crate::agents::runtime::types::AcpProtocolMessage> {
    let body = event_data(value);
    let tool_name = string_field(body, "toolName")
        .or_else(|| string_field(body, "name"))
        .unwrap_or_else(|| "tool".to_string());
    let content = pi_tool_result_content_from_rpc_result(
        &tool_name,
        value_field(body, "args").as_ref(),
        value_field(body, "result").as_ref(),
    );
    if content.is_empty() {
        return None;
    }
    let tool_id = tool_id_from_value(body);
    Some(history_session_update_message(
        "tool_call_update",
        json!({
            "sessionUpdate": "tool_call_update",
            "toolCallId": tool_id,
            "title": tool_name,
            "kind": "edit",
            "status": "completed",
            "content": content,
            "rawOutput": value_field(body, "result").unwrap_or(Value::Null),
        }),
        None,
    ))
}

fn pi_tool_result_content_from_rpc_result(
    tool_name: &str,
    args: Option<&Value>,
    result: Option<&Value>,
) -> Vec<Value> {
    let normalized = tool_name.trim().to_ascii_lowercase();
    if normalized != "edit" && normalized != "patch" && normalized != "apply_patch" {
        return Vec::new();
    }
    let Some(result) = result else {
        return Vec::new();
    };
    let Some(details) = result.get("details") else {
        return Vec::new();
    };
    let patch = details
        .get("patch")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let diff = details
        .get("diff")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    if patch.is_none() && diff.is_none() {
        return Vec::new();
    }
    let path = args
        .and_then(|args| {
            string_field(args, "path")
                .or_else(|| string_field(args, "filePath"))
                .or_else(|| string_field(args, "file_path"))
        })
        .or_else(|| {
            details
                .get("path")
                .or_else(|| details.get("filePath"))
                .or_else(|| details.get("file_path"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });
    let first_changed_line = details
        .get("firstChangedLine")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    vec![json!({
        "type": "diff",
        "path": path,
        "patch": patch,
        "diff": diff,
        "detail": diff,
        "meta": first_changed_line.map(|line| json!({
            "firstChangedLine": line,
            "startLine": line,
            "line": line,
        })).unwrap_or(Value::Null),
    })]
}

#[derive(Debug)]
enum PiRpcIncoming {
    Event(Value),
}

#[derive(Clone)]
struct PiRpcClient {
    stdin: Arc<AsyncMutex<tokio::process::ChildStdin>>,
    pending: Arc<AsyncMutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>,
    request_id: Arc<AtomicU64>,
}

impl PiRpcClient {
    fn new(
        stdin: tokio::process::ChildStdin,
        _event_tx: tauri::async_runtime::Sender<PiRpcIncoming>,
    ) -> Self {
        Self {
            stdin: Arc::new(AsyncMutex::new(stdin)),
            pending: Arc::new(AsyncMutex::new(HashMap::new())),
            request_id: Arc::new(AtomicU64::new(1)),
        }
    }

    async fn request(&self, command: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(command, params, Some(PI_RPC_REQUEST_TIMEOUT))
            .await
    }

    async fn request_no_timeout(&self, command: &str, params: Value) -> Result<Value> {
        self.request_with_timeout(command, params, None).await
    }

    async fn request_with_timeout(
        &self,
        command: &str,
        params: Value,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed).to_string();
        let request = rpc_request_value(&id, command, params)?;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        let write_result = async {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(request.to_string().as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        }
        .await;
        if let Err(error) = write_result {
            self.pending.lock().await.remove(&id);
            return Err(anyhow::anyhow!("write Pi RPC request `{command}`: {error}"));
        }

        let response = if let Some(timeout) = timeout {
            match tokio::time::timeout(timeout, rx).await {
                Ok(response) => response,
                Err(_) => {
                    self.pending.lock().await.remove(&id);
                    return Err(anyhow::anyhow!("Pi RPC request `{command}` timed out"));
                }
            }
        } else {
            rx.await
        };

        match response {
            Ok(Ok(value)) => Ok(response_payload(value)),
            Ok(Err(message)) => Err(anyhow::anyhow!(message)),
            Err(_) => Err(anyhow::anyhow!(
                "Pi RPC response channel closed for `{command}`"
            )),
        }
    }
}

fn spawn_stdout_reader(
    stdout: tokio::process::ChildStdout,
    pending: Arc<AsyncMutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>,
    event_tx: tauri::async_runtime::Sender<PiRpcIncoming>,
) {
    tauri::async_runtime::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    log::warn!("[sessio-runtime:pi-rpc:stdout] read failed: {error}");
                    break;
                }
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(trimmed) {
                Ok(value) => value,
                Err(error) => {
                    log::debug!(
                        "[sessio-runtime:pi-rpc:stdout] non-json line `{trimmed}`: {error}"
                    );
                    continue;
                }
            };
            if let Some(id) = response_id(&value) {
                let sender = pending.lock().await.remove(&id);
                if let Some(sender) = sender {
                    let _ = sender.send(response_result(value));
                    continue;
                }
            }
            let _ = event_tx.try_send(PiRpcIncoming::Event(value));
        }
    });
}

fn spawn_stderr_reader(stderr: tokio::process::ChildStderr, stderr_log: Arc<Mutex<String>>) {
    tauri::async_runtime::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log::debug!("[sessio-runtime:pi-rpc:stderr] {line}");
            if let Ok(mut collected) = stderr_log.lock() {
                if !collected.is_empty() {
                    collected.push('\n');
                }
                collected.push_str(&line);
                if collected.len() > 16 * 1024 {
                    let keep_from = collected.len().saturating_sub(12 * 1024);
                    let tail = collected[keep_from..].to_string();
                    *collected = tail;
                }
            }
        }
    });
}

struct SpawnedPiRpcProcess {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    child: TokioChild,
    group: ChildProcessGroup,
}

struct TokioChildGuard {
    child: TokioChild,
    group: ChildProcessGroup,
}

impl TokioChildGuard {
    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait().await
    }
}

impl Drop for TokioChildGuard {
    fn drop(&mut self) {
        kill_child_process_group(&mut self.child, &self.group);
    }
}

#[derive(Clone, Copy, Debug)]
enum ChildProcessGroup {
    #[cfg(unix)]
    Unix(libc::pid_t),
    None,
}

#[cfg(unix)]
fn configure_child_process_group(child: &mut TokioCommand) {
    unsafe {
        child.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn configure_child_process_group(_child: &mut TokioCommand) {}

#[cfg(unix)]
fn child_process_group(child: &TokioChild) -> ChildProcessGroup {
    child
        .id()
        .map(|pid| ChildProcessGroup::Unix(pid as libc::pid_t))
        .unwrap_or(ChildProcessGroup::None)
}

#[cfg(not(unix))]
fn child_process_group(_child: &TokioChild) -> ChildProcessGroup {
    ChildProcessGroup::None
}

fn kill_child_process_group(child: &mut TokioChild, group: &ChildProcessGroup) {
    #[cfg(unix)]
    if let ChildProcessGroup::Unix(pgid) = group {
        unsafe {
            libc::kill(-(*pgid), libc::SIGKILL);
        }
        return;
    }
    #[cfg(not(unix))]
    let _ = group;
    let _ = child.start_kill();
}

fn spawn_pi_rpc_process(
    command: &str,
    workspace_path: &str,
    computer_use: Option<&PiRpcComputerUseExtension>,
) -> Result<SpawnedPiRpcProcess> {
    let command = ensure_rpc_mode(command);
    let command = if let Some(extension) = computer_use {
        command_with_extension(&command, &extension.extension_path)
    } else {
        command
    };
    let args = shell_words::split(&command)
        .with_context(|| format!("failed to parse Pi RPC command: {command}"))?;
    let (program, rest) = args
        .split_first()
        .context("Pi RPC command cannot be empty")?;

    let mut child = TokioCommand::new(program);
    configure_child_process_group(&mut child);
    child
        .args(rest)
        .current_dir(workspace_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(extension) = computer_use {
        child
            .env("SESSIO_COMPUTER_USE_MCP_URL", &extension.injection.url)
            .env(
                "SESSIO_COMPUTER_USE_TOKEN",
                &extension.injection.bearer_token,
            )
            .env(
                "SESSIO_COMPUTER_USE_SESSION_ID",
                &extension.sessio_runtime_session_id,
            );
    }

    let mut child = child
        .spawn()
        .with_context(|| format!("spawn Pi RPC command `{command}` with cwd `{workspace_path}`"))?;
    let group = child_process_group(&child);
    let stdin = child.stdin.take().context("failed to open Pi RPC stdin")?;
    let stdout = child
        .stdout
        .take()
        .context("failed to open Pi RPC stdout")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to open Pi RPC stderr")?;

    Ok(SpawnedPiRpcProcess {
        stdin,
        stdout,
        stderr,
        child,
        group,
    })
}

fn prompt_params_from_input(input: AgentInput) -> Result<Value> {
    let AgentInput {
        text,
        attachments,
        options,
    } = input;
    let mut message = normalize_runtime_prompt_text(&text, &options);
    let mut images = Vec::new();
    for attachment in attachments {
        match attachment.kind {
            AgentAttachmentKind::Image => images.push(image_attachment_json(&attachment)?),
            AgentAttachmentKind::File => {
                let block = text_attachment_block(&attachment)?;
                if !message.is_empty() {
                    message.push_str("\n\n");
                }
                message.push_str(&block);
            }
        }
    }
    Ok(json!({
        "message": message,
        "prompt": message,
        "images": images,
    }))
}

fn normalize_runtime_prompt_text(text: &str, options: &RuntimeMetadata) -> String {
    let text = normalize_canvas_prompt_text(text, options);
    let text = crate::work_state_skill_resource::inject_work_state_skill_prompt_block(&text);
    let text = crate::skills::inject_selected_skills_prompt_block(&text, options);
    if !runtime_option_bool(options, "computerUse") && !runtime_option_bool(options, "computer_use")
    {
        return text;
    }
    let skill_block = computer_use_prompt_block();
    if skill_block.trim().is_empty() {
        return text;
    }
    format!("{skill_block}\n\n\n\n{text}")
}

fn runtime_option_bool(options: &RuntimeMetadata, key: &str) -> bool {
    options.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn computer_use_prompt_block() -> String {
    crate::computer_use::skill_resource::computer_use_prompt_block()
}

fn normalize_canvas_prompt_text(text: &str, options: &RuntimeMetadata) -> String {
    let Some(canvas_context) = options.get("canvasContext") else {
        return text.to_string();
    };
    let Some(block) = build_canvas_prompt_block(canvas_context) else {
        return text.to_string();
    };
    format!("{block}\n\n---\n\n{text}")
}

fn build_canvas_prompt_block(value: &Value) -> Option<String> {
    let context = value.as_object()?;
    let scope = context
        .get("scope")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("canvas");
    let refs = context
        .get("refs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut lines = vec![
        "[Canvas context]".to_string(),
        format!("Canvas scope: {scope}"),
    ];
    let block_ids = context
        .get("blockIds")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    if block_ids > 0 {
        lines.push(format!("Selected items: {block_ids}"));
    }
    if refs.is_empty() {
        lines.push("Use the current canvas selection when answering.".to_string());
    } else {
        lines.push("Selected refs:".to_string());
        for (index, item) in refs.iter().take(8).enumerate() {
            let Some(record) = item.as_object() else {
                continue;
            };
            let kind = record
                .get("blockKind")
                .and_then(Value::as_str)
                .unwrap_or("item");
            let source = record
                .get("sourcePath")
                .and_then(Value::as_str)
                .or_else(|| record.get("sourceKey").and_then(Value::as_str))
                .unwrap_or(kind);
            let summary = record
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| truncate_canvas_summary(value, 180));
            let mut line = format!("{}. {} - {}", index + 1, kind, source);
            if let Some(summary) = summary {
                line.push_str(": ");
                line.push_str(&summary);
            }
            lines.push(line);
        }
        if refs.len() > 8 {
            lines.push(format!("... and {} more items.", refs.len() - 8));
        }
    }
    if context
        .get("snapshotAttachmentPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .is_some()
    {
        lines.push("Use the attached canvas snapshot when helpful.".to_string());
    }
    Some(lines.join("\n"))
}

fn truncate_canvas_summary(value: &str, limit: usize) -> String {
    let trimmed = value.trim().replace('\n', " ");
    let mut chars = trimmed.chars();
    let head: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{head}...")
    } else {
        head
    }
}

fn image_attachment_json(attachment: &AgentAttachment) -> Result<Value> {
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
    Ok(json!({
        "type": "image",
        "data": base64::engine::general_purpose::STANDARD.encode(data),
        "mimeType": mime_type,
        "uri": file_uri(path),
        "name": attachment_display_name(attachment, path),
    }))
}

fn text_attachment_block(attachment: &AgentAttachment) -> Result<String> {
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
    Ok(format!(
        "<sessio-upload-file uri=\"{}\" name=\"{}\" mimeType=\"{}\">\n{}\n</sessio-upload-file>",
        escape_xml_attr(&file_uri(path)),
        escape_xml_attr(attachment_display_name(attachment, path)),
        escape_xml_attr(&mime_type),
        text
    ))
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

fn ensure_file_size(path: &Path, max_bytes: u64, message: &str) -> Result<()> {
    let size = std::fs::metadata(path)
        .with_context(|| format!("read attachment metadata {}", path.display()))?
        .len();
    if size > max_bytes {
        anyhow::bail!("{message}: {} bytes", size);
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

fn escape_xml_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

fn find_pi_session_file(agent_session_id: &str) -> Result<Option<PathBuf>> {
    let root = app_paths::pi_agent_sessions_dir()?;
    if !root.exists() {
        return Ok(None);
    }
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_name.contains(agent_session_id) {
            return Ok(Some(path.to_path_buf()));
        }
    }
    Ok(None)
}

fn ensure_rpc_mode(command: &str) -> String {
    let Ok(args) = shell_words::split(command) else {
        return command.to_string();
    };
    if args
        .iter()
        .any(|arg| arg == "--mode" || arg.starts_with("--mode="))
    {
        command.to_string()
    } else {
        format!("{command} --mode rpc")
    }
}

fn command_with_extension(command: &str, extension_path: &Path) -> String {
    let path = extension_path.to_string_lossy();
    format!("{command} -e {}", shell_words::quote(&path))
}

fn response_id(value: &Value) -> Option<String> {
    value.get("id").and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn response_result(value: Value) -> Result<Value, String> {
    if value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "response")
    {
        return Ok(value);
    }
    if value
        .get("success")
        .and_then(Value::as_bool)
        .is_some_and(|success| !success)
    {
        return Err(response_error_message(&value));
    }
    if let Some(error) = value.get("error") {
        return Err(error
            .as_str()
            .map(ToString::to_string)
            .unwrap_or_else(|| error.to_string()));
    }
    Ok(value)
}

fn response_payload(value: Value) -> Value {
    value
        .get("data")
        .cloned()
        .or_else(|| value.get("result").cloned())
        .or_else(|| value.get("response").cloned())
        .unwrap_or(value)
}

fn rpc_request_value(id: &str, command: &str, params: Value) -> Result<Value> {
    let mut object = match params {
        Value::Object(object) => object,
        Value::Null => serde_json::Map::new(),
        other => anyhow::bail!("Pi RPC params for `{command}` must be an object, got {other}"),
    };
    object.insert("id".to_string(), Value::String(id.to_string()));
    object.insert("type".to_string(), Value::String(command.to_string()));
    Ok(Value::Object(object))
}

fn response_error_message(value: &Value) -> String {
    value
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn rpc_event_type(value: &Value) -> Option<String> {
    string_field(value, "event")
        .or_else(|| string_field(value, "eventType"))
        .or_else(|| {
            let kind = string_field(value, "type")?;
            if kind == "event" {
                string_field(value, "name").or_else(|| {
                    value.get("data").and_then(|data| {
                        string_field(data, "type").or_else(|| string_field(data, "event"))
                    })
                })
            } else {
                Some(kind)
            }
        })
}

fn event_data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

fn delta_text(value: &Value) -> Option<String> {
    raw_string_field(value, "delta")
        .or_else(|| raw_string_field(value, "text"))
        .or_else(|| raw_string_field(value, "content"))
}

fn final_assistant_text_from_event(value: &Value) -> Option<String> {
    for root in [value, event_data(value)] {
        if let Some(messages) = root.as_array() {
            if let Some(text) = messages
                .iter()
                .rev()
                .find_map(|candidate| assistant_text_from_value(candidate, false))
            {
                return Some(text);
            }
        }
        if let Some(text) = assistant_text_from_value(root, false) {
            return Some(text);
        }
        for key in [
            "message",
            "assistantMessage",
            "assistant_message",
            "finalMessage",
            "final_message",
            "response",
        ] {
            if let Some(text) = root
                .get(key)
                .and_then(|candidate| assistant_text_from_value(candidate, false))
            {
                return Some(text);
            }
        }
        for key in ["messages", "assistantMessages", "assistant_messages"] {
            let Some(messages) = root.get(key).and_then(Value::as_array) else {
                continue;
            };
            if let Some(text) = messages
                .iter()
                .rev()
                .find_map(|candidate| assistant_text_from_value(candidate, false))
            {
                return Some(text);
            }
        }
    }
    None
}

fn assistant_text_from_value(value: &Value, allow_roleless: bool) -> Option<String> {
    let role = string_field(value, "role");
    if role
        .as_deref()
        .is_some_and(|role| role != "assistant" && role != "system")
    {
        return None;
    }
    let roleless = role.is_none();
    if roleless && !allow_roleless {
        let has_message_shape =
            value
                .get("content")
                .and_then(Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| string_field(item, "type").is_some())
                });
        if !has_message_shape {
            return None;
        }
    }
    if let Some(content) = value.get("content") {
        if let Some(text) = text_from_content_value(content) {
            return Some(text);
        }
    }
    if allow_roleless || !roleless {
        raw_non_empty_string_field(value, "text")
            .or_else(|| raw_non_empty_string_field(value, "output"))
    } else {
        None
    }
}

fn text_from_content_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return (!text.trim().is_empty()).then(|| text.to_string());
    }
    let text = value
        .as_array()?
        .iter()
        .filter(|part| {
            string_field(part, "type")
                .map(|kind| kind == "text")
                .unwrap_or_else(|| part.get("text").is_some())
        })
        .filter_map(|part| raw_string_field(part, "text"))
        .collect::<Vec<_>>()
        .join("");
    (!text.trim().is_empty()).then_some(text)
}

fn tool_id_from_value(value: &Value) -> String {
    string_field(value, "toolCallId")
        .or_else(|| string_field(value, "tool_call_id"))
        .or_else(|| string_field(value, "id"))
        .or_else(|| {
            value.get("toolCall").and_then(|tool| {
                string_field(tool, "id").or_else(|| string_field(tool, "toolCallId"))
            })
        })
        .unwrap_or_else(|| "tool".to_string())
}

fn model_id_from_state(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(|model| {
            model
                .as_str()
                .map(ToString::to_string)
                .or_else(|| {
                    string_field(model, "provider").and_then(|provider| {
                        string_field(model, "id")
                            .or_else(|| string_field(model, "modelId"))
                            .or_else(|| string_field(model, "name"))
                            .map(|id| format!("{provider}/{id}"))
                    })
                })
                .or_else(|| string_field(model, "id"))
                .or_else(|| string_field(model, "modelId"))
                .or_else(|| string_field(model, "name"))
        })
        .or_else(|| string_field(value, "modelId"))
}

fn model_request_params(value: &str) -> Value {
    let base = value.split_once(':').map(|(base, _)| base).unwrap_or(value);
    let (provider, model_id) = base
        .split_once('/')
        .map(|(provider, model_id)| (Some(provider.trim()), model_id.trim()))
        .unwrap_or((None, base.trim()));
    let mut object = serde_json::Map::new();
    let model_id = model_id.to_string();
    if let Some(provider) = provider.filter(|value| !value.is_empty()) {
        object.insert("provider".to_string(), Value::String(provider.to_string()));
    }
    if !model_id.is_empty() {
        object.insert("modelId".to_string(), Value::String(model_id.clone()));
        object.insert("model".to_string(), Value::String(model_id));
    }
    Value::Object(object)
}

fn value_to_display_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(content_text) = value
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| string_field(item, "text"))
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|text| !text.is_empty())
    {
        return Some(content_text);
    }
    Some(value.to_string())
}

fn normalize_command_update(value: Value) -> Value {
    let name = string_field(&value, "name")
        .or_else(|| string_field(&value, "command"))
        .unwrap_or_else(|| "command".to_string());
    json!({
        "name": name,
        "description": string_field(&value, "description").unwrap_or_default(),
        "input": value.get("input").cloned().unwrap_or(Value::Null),
        "meta": {
            "source": "piRpc",
            "raw": value,
        },
    })
}

fn pi_config_options(state: &PiRpcState, models: Vec<Value>) -> Vec<Value> {
    let model_options = models
        .into_iter()
        .map(|model| {
            let id = model_option_value(&model)
                .or_else(|| string_field(&model, "id"))
                .or_else(|| string_field(&model, "name"))
                .unwrap_or_default();
            json!({
                "value": id,
                "name": string_field(&model, "name").unwrap_or_else(|| id.clone()),
                "description": string_field(&model, "description"),
                "meta": {
                    "source": "piRpc",
                    "raw": model,
                },
            })
        })
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    if !model_options.is_empty() {
        out.push(json!({
            "id": "model",
            "name": "Model",
            "category": "model",
            "type": "select",
            "currentValue": state.model.clone().map(Value::String).unwrap_or(Value::Null),
            "options": model_options,
            "meta": { "source": "piRpc" },
        }));
    }
    let thinking_options = ["off", "minimal", "low", "medium", "high", "xhigh"]
        .iter()
        .map(|value| json!({ "value": value, "name": value }))
        .collect::<Vec<_>>();
    out.push(json!({
        "id": "effort",
        "name": "Thinking",
        "category": "reasoning",
        "type": "select",
        "currentValue": state.thinking_level.clone().map(Value::String).unwrap_or_else(|| Value::String("medium".to_string())),
        "options": thinking_options,
        "meta": { "source": "piRpc" },
    }));
    out
}

fn model_option_value(model: &Value) -> Option<String> {
    let id = string_field(model, "modelId")
        .or_else(|| string_field(model, "id"))
        .or_else(|| string_field(model, "name"))?;
    string_field(model, "provider")
        .or_else(|| string_field(model, "providerId"))
        .map(|provider| format!("{provider}/{id}"))
        .or(Some(id))
}

fn session_info_update(state: &PiRpcState) -> Value {
    json!({
        "sessionUpdate": "session_info_update",
        "title": state.session_name,
        "meta": {
            "source": "piRpc",
            "sessionId": state.session_id,
            "sessionFile": state.session_file,
            "raw": state.raw,
        },
    })
}

fn emit_session_update(
    manager: &RuntimeManager,
    sessio_runtime_session_id: &str,
    update_type: &str,
    data: Value,
) {
    let _ = manager.emit(AgentRuntimeEventPayload::SessionUpdate {
        sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
        turn_id: "session".to_string(),
        update_type: update_type.to_string(),
        data,
    });
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn raw_string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn raw_non_empty_string_field(value: &Value, key: &str) -> Option<String> {
    raw_string_field(value, key).filter(|value| !value.trim().is_empty())
}

fn value_field(value: &Value, key: &str) -> Option<Value> {
    value.get(key).cloned()
}

fn normalize_thinking_level(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => "off".to_string(),
        "minimal" | "min" => "minimal".to_string(),
        "low" => "low".to_string(),
        "medium" | "default" => "medium".to_string(),
        "high" => "high".to_string(),
        "xhigh" | "extra-high" | "extra_high" | "max" => "xhigh".to_string(),
        _ => "medium".to_string(),
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

fn turn_had_text(current_turn_had_text: &Arc<Mutex<bool>>) -> bool {
    current_turn_had_text
        .lock()
        .map(|guard| *guard)
        .unwrap_or(true)
}

fn set_turn_had_text(current_turn_had_text: &Arc<Mutex<bool>>, value: bool) {
    if let Ok(mut guard) = current_turn_had_text.lock() {
        *guard = value;
    }
}

fn collected_stderr(stderr_log: &Arc<Mutex<String>>) -> String {
    stderr_log
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_assistant_text_from_event_extracts_pi_message_end_text_only() {
        let event = json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "internal notes" },
                    { "type": "text", "text": "当前目录里我只看到 1 个文档：" }
                ]
            }
        });

        assert_eq!(
            final_assistant_text_from_event(&event).as_deref(),
            Some("当前目录里我只看到 1 个文档：")
        );
    }

    #[test]
    fn final_assistant_text_from_event_uses_latest_assistant_from_turn_end() {
        let event = json!({
            "type": "turn_end",
            "data": {
                "messages": [
                    { "role": "user", "content": [{ "type": "text", "text": "question" }] },
                    { "role": "assistant", "content": [{ "type": "text", "text": "answer" }] }
                ]
            }
        });

        assert_eq!(
            final_assistant_text_from_event(&event).as_deref(),
            Some("answer")
        );
    }

    #[test]
    fn final_assistant_text_from_event_extracts_prompt_response_payload() {
        let response = json!({
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "question" }] },
                {
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": "not visible final text" },
                        { "type": "text", "text": "已写入文档。" }
                    ]
                }
            ]
        });

        assert_eq!(
            final_assistant_text_from_event(&response).as_deref(),
            Some("已写入文档。")
        );
    }

    #[test]
    fn final_assistant_text_from_event_ignores_prompt_ack_payload() {
        let response = json!({
            "ok": true,
            "accepted": true,
            "sessionId": "session-1"
        });

        assert_eq!(final_assistant_text_from_event(&response), None);
    }

    #[test]
    fn delta_text_preserves_streaming_whitespace() {
        assert_eq!(
            delta_text(&json!({ "type": "thinking_delta", "delta": " need to" })).as_deref(),
            Some(" need to")
        );
    }

    #[test]
    fn computer_use_prompt_layer_mentions_raise_recovery() {
        let markers = crate::prompt_markers::sessio_prompt_markers();
        let mut options = RuntimeMetadata::new();
        options.insert("computerUse".into(), json!(true));

        let text = normalize_runtime_prompt_text("send the message", &options);

        assert!(text.contains(markers.skills_prompt_start));
        assert!(text.contains(&format!("kind=\"{}\"", markers.builtin_skill_prompt_kind)));
        assert!(text.contains("id: `builtin:computer-use`"));
        assert!(text.contains("computer_get_app_state"));
        assert!(text.contains("computer_raise_app"));
        assert!(text.contains("open -a"));
        assert!(text.contains("AppleScript"));
        assert!(text.ends_with("send the message"));
    }

    #[test]
    fn work_state_skill_pointer_is_injected_for_work_context() {
        let markers = crate::prompt_markers::sessio_prompt_markers();
        let prompt = format!(
            "{} nonce=\"abc\" kind=\"{}\" -->\nstage context\n{} nonce=\"abc\" -->",
            markers.thread_prompt_start,
            markers.thread_prompt_kind_work_context,
            markers.thread_prompt_end
        );

        let text = normalize_runtime_prompt_text(&prompt, &RuntimeMetadata::new());

        assert!(text.contains(&format!("kind=\"{}\"", markers.builtin_skill_prompt_kind)));
        assert!(text.contains("id: `builtin:sessio-work-state`"));
        assert!(text.contains("~/.sessio/bin/sessio"));
        assert!(text.ends_with(&prompt));
    }

    #[test]
    fn command_with_extension_quotes_extension_path() {
        let command = command_with_extension(
            "pi --mode rpc",
            Path::new("/tmp/Sessio Computer Use/sessio-computer-use.ts"),
        );
        let args = shell_words::split(&command).unwrap();

        assert_eq!(
            args,
            vec![
                "pi",
                "--mode",
                "rpc",
                "-e",
                "/tmp/Sessio Computer Use/sessio-computer-use.ts"
            ]
        );
    }

    #[test]
    fn ensure_rpc_mode_then_extension_preserves_both_flags() {
        let command = ensure_rpc_mode("pi");
        let command = command_with_extension(&command, Path::new("/tmp/cu.ts"));
        let args = shell_words::split(&command).unwrap();

        assert_eq!(args, vec!["pi", "--mode", "rpc", "-e", "/tmp/cu.ts"]);
    }

    #[test]
    fn final_assistant_text_preserves_content_part_whitespace() {
        let event = json!({
            "type": "agent_end",
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "I need" },
                    { "type": "text", "text": " to keep spaces." }
                ]
            }
        });

        assert_eq!(
            final_assistant_text_from_event(&event).as_deref(),
            Some("I need to keep spaces.")
        );
    }

    #[test]
    fn pi_tool_execution_end_emits_edit_diff_tool_update() {
        let event = json!({
            "type": "tool_execution_end",
            "data": {
                "toolCallId": "call-edit-1",
                "toolName": "edit",
                "args": {
                    "path": "world-cup.md",
                    "edits": [{ "oldText": "old", "newText": "new" }]
                },
                "result": {
                    "content": [
                        { "type": "text", "text": "Successfully replaced 1 block(s) in world-cup.md." }
                    ],
                    "details": {
                        "diff": "-1 old\n+1 new",
                        "patch": "--- world-cup.md\n+++ world-cup.md\n@@ -1 +1 @@\n-old\n+new\n",
                        "firstChangedLine": 1
                    }
                },
                "isError": false
            }
        });

        let message = pi_tool_execution_end_acp_message("runtime-1", "turn-1", &event)
            .expect("tool update message");
        let update = message
            .data
            .get("update")
            .and_then(Value::as_object)
            .expect("tool update payload");
        assert_eq!(
            update.get("toolCallId").and_then(Value::as_str),
            Some("call-edit-1")
        );
        let content = update
            .get("content")
            .and_then(Value::as_array)
            .expect("content");
        assert_eq!(content[0]["path"].as_str(), Some("world-cup.md"));
        assert_eq!(
            content[0]["patch"].as_str(),
            Some("--- world-cup.md\n+++ world-cup.md\n@@ -1 +1 @@\n-old\n+new\n")
        );
        assert_eq!(content[0]["detail"].as_str(), Some("-1 old\n+1 new"));
    }
}
