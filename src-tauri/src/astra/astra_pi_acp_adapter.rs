use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, InitializeRequest, NewSessionRequest, PromptRequest,
    ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SessionNotification, SessionUpdate, StopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent as AcpAgentRole, ConnectionTo};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_yaml::Value as YamlValue;

use super::{
    short_hash, AstraOrchestration, AstraRun, AstraRunIntent, AstraTaskCompletion,
    AstraTaskProposal, AstraTaskRisk,
};
use crate::astra::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use crate::astra::prompt::build_astra_orchestration_prompt;
use crate::models::{Agent, PlanRoundMode, ThreadInfo, ThreadKind};

#[derive(Debug, Clone)]
pub(super) struct AstraPiAcpConfig {
    pub command: String,
    pub session_dir: String,
    pub agent_dir: String,
    pub orchestrator: AstraPiAcpPurposeConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AstraPiAcpProviderConfig {
    pub provider: Option<String>,
    pub api: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
}

impl AstraPiAcpProviderConfig {
    pub(crate) fn with_runtime_overrides(
        mut self,
        model: Option<String>,
        thinking_level: Option<String>,
    ) -> Self {
        if model
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.model = model;
        }
        if thinking_level
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            self.thinking_level = thinking_level;
        }
        self
    }
}

#[derive(Debug, Clone)]
pub(super) struct AstraPiAcpPurposeConfig {
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AstraPiAcpPurpose {
    Orchestration,
}

impl AstraPiAcpPurpose {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Orchestration => "orchestration",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct AstraPiAcpFailure {
    pub code: &'static str,
    pub message: String,
    pub session_id: Option<String>,
}

impl AstraPiAcpFailure {
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            session_id: None,
        }
    }

    fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }
}

#[derive(Debug, Clone)]
struct AstraPiAcpTextResponse {
    text: String,
    session_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct AstraPiAcpOrchestrator {
    config: AstraPiAcpConfig,
}

impl OrchestratorBackend for AstraPiAcpOrchestrator {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure> {
        let provider_config: AstraPiAcpProviderConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();

        let prompt =
            build_astra_orchestration_prompt(run, thread, user_prompt, round_index, completions);
        let response = run_internal_astra_pi_acp(
            &self.config,
            AstraPiAcpPurpose::Orchestration,
            &run.run_id,
            &run.project_path,
            &prompt,
            &provider_config,
        )
        .map_err(|failure| {
            BackendFailure::new("astra_pi_acp", failure.code, failure.message)
                .with_session_id(failure.session_id)
        })?;

        let orchestration = parse_astra_pi_acp_orchestration_response(
            &response.text,
            run,
            thread,
            round_index,
            completions,
        )
        .map_err(|failure| {
            BackendFailure::new("astra_pi_acp", failure.code, failure.message)
                .with_session_id(Some(response.session_id.clone()))
                .with_raw_response(&response.text)
        })?;

        Ok(BackendResponse {
            data: orchestration,
            session_id: response.session_id,
            backend_type: "astra_pi_acp".to_string(),
        })
    }
}

impl AstraPiAcpOrchestrator {
    pub(super) fn new(config: AstraPiAcpConfig) -> Self {
        Self { config }
    }
}

fn run_internal_astra_pi_acp(
    config: &AstraPiAcpConfig,
    purpose: AstraPiAcpPurpose,
    run_id: &str,
    workspace_path: &str,
    prompt: &str,
    provider_config: &AstraPiAcpProviderConfig,
) -> Result<AstraPiAcpTextResponse, AstraPiAcpFailure> {
    let purpose_config = purpose_config(config, purpose);
    let command = config.command.clone();
    let meta = internal_astra_pi_acp_meta(config, provider_config, purpose);
    let timeout = Duration::from_millis(purpose_config.timeout_ms);
    let workspace = if workspace_path.trim().is_empty() {
        std::env::current_dir()
            .map_err(|error| AstraPiAcpFailure::new("transport_failure", error.to_string()))?
    } else {
        PathBuf::from(workspace_path)
    };
    log::info!(
        "[astra:astra-pi-acp:call] purpose={} runId={} command={} workspace={} timeoutMs={} sessionDir={} model={:?} thinkingLevel={:?} meta={} promptChars={}",
        purpose.as_str(),
        run_id,
        command,
        workspace.display(),
        timeout.as_millis(),
        config.session_dir,
        provider_config.model,
        provider_config.thinking_level,
        Value::Object(meta.clone()),
        prompt.chars().count()
    );
    let prompt = prompt.to_string();
    let purpose_name = purpose.as_str().to_string();
    let run_id = run_id.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let tracked_session_id = Arc::new(Mutex::new(None::<String>));
    let tracked_session_id_for_worker = tracked_session_id.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let result = run_internal_astra_pi_acp_async(
            command,
            purpose_name,
            run_id,
            meta,
            workspace,
            prompt,
            tracked_session_id_for_worker,
        )
        .await;
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(Ok(response)) => {
            log::info!(
                "[astra:astra-pi-acp:response] purpose={} sessionId={} textChars={}",
                purpose.as_str(),
                response.session_id,
                response.text.chars().count()
            );
            Ok(response)
        }
        Ok(Err(failure)) => {
            log::warn!(
                "[astra:astra-pi-acp:error] purpose={} code={} sessionId={:?} message={}",
                purpose.as_str(),
                failure.code,
                failure.session_id,
                failure.message
            );
            Err(failure)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            handle.abort();
            let failure = AstraPiAcpFailure::new(
                "timeout",
                format!(
                    "Astra Pi ACP {} timed out after {}ms",
                    purpose.as_str(),
                    timeout.as_millis()
                ),
            )
            .with_session_id(
                tracked_session_id
                    .lock()
                    .ok()
                    .and_then(|value| value.clone()),
            );
            log::warn!(
                "[astra:astra-pi-acp:error] purpose={} code={} sessionId={:?} message={}",
                purpose.as_str(),
                failure.code,
                failure.session_id,
                failure.message
            );
            Err(failure)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let failure = AstraPiAcpFailure::new(
                "transport_failure",
                format!("Astra Pi ACP {} worker disconnected", purpose.as_str()),
            );
            log::warn!(
                "[astra:astra-pi-acp:error] purpose={} code={} sessionId={:?} message={}",
                purpose.as_str(),
                failure.code,
                failure.session_id,
                failure.message
            );
            Err(failure)
        }
    }
}

async fn run_internal_astra_pi_acp_async(
    command: String,
    purpose: String,
    run_id: String,
    meta: serde_json::Map<String, Value>,
    workspace: PathBuf,
    prompt: String,
    internal_session_id: Arc<Mutex<Option<String>>>,
) -> Result<AstraPiAcpTextResponse, AstraPiAcpFailure> {
    let agent = AcpAgent::from_str(&command)
        .map_err(|error| AstraPiAcpFailure::new("transport_failure", error.to_string()))?;
    let text = Arc::new(Mutex::new(String::new()));
    let policy_denied = Arc::new(AtomicBool::new(false));
    let notification_text = text.clone();
    let notification_purpose = purpose.clone();
    let permission_denied = policy_denied.clone();
    let failure_policy_denied = policy_denied.clone();
    let failure_session_id = internal_session_id.clone();
    agent_client_protocol::Client
        .builder()
        .name("astra-internal")
        .on_receive_notification(
            async move |notification: SessionNotification, _connection| {
                if let Err(error) = collect_notification_text(&notification, &notification_text) {
                    log::warn!(
                        "[astra:astra-pi-acp:notification] purpose={} error={}",
                        notification_purpose,
                        error
                    );
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |_request: RequestPermissionRequest, responder, _connection| {
                permission_denied.store(true, Ordering::SeqCst);
                responder.respond(RequestPermissionResponse::new(
                    RequestPermissionOutcome::Cancelled,
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<AcpAgentRole>| {
            let text = text.clone();
            let policy_denied = policy_denied.clone();
            let internal_session_id = internal_session_id.clone();
            async move {
                log::info!(
                    "[astra:astra-pi-acp:stage] purpose={} runId={} stage=initialize:start",
                    purpose,
                    run_id
                );
                let init = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                log::info!(
                    "[astra:astra-pi-acp:stage] purpose={} runId={} stage=initialize:ok protocolVersion={:?} capabilities={:?}",
                    purpose,
                    run_id,
                    init.protocol_version,
                    init.agent_capabilities
                );
                let mut request = NewSessionRequest::new(workspace);
                let mut sessio_meta = serde_json::Map::new();
                sessio_meta.insert("astraRunId".to_string(), Value::String(run_id.clone()));
                sessio_meta.insert(
                    "astraInternalPurpose".to_string(),
                    Value::String(purpose.clone()),
                );
                sessio_meta.insert(
                    "orchestratorBackend".to_string(),
                    Value::String("astra_pi_acp".to_string()),
                );
                sessio_meta.insert("astraPiAcp".to_string(), Value::Object(meta));
                request.meta = Some(
                    json!({ "sessio": Value::Object(sessio_meta) })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                );
                log::info!(
                    "[astra:astra-pi-acp:stage] purpose={} runId={} stage=new_session:start meta={}",
                    purpose,
                    run_id,
                    serde_json::to_string(&request.meta).unwrap_or_default()
                );
                let session = connection.send_request(request).block_task().await?;
                let session_id = session.session_id.to_string();
                if let Ok(mut tracked) = internal_session_id.lock() {
                    *tracked = Some(session_id.clone());
                }
                log::info!(
                    "[astra:astra-pi-acp:stage] purpose={} runId={} stage=new_session:ok sessionId={}",
                    purpose,
                    run_id,
                    session_id
                );
                log::info!(
                    "[astra:astra-pi-acp:stage] purpose={} runId={} stage=prompt:start sessionId={} promptChars={}",
                    purpose,
                    run_id,
                    session_id,
                    prompt.chars().count()
                );
                let response = connection
                    .send_request(PromptRequest::new(
                        session.session_id,
                        vec![ContentBlock::Text(TextContent::new(prompt))],
                    ))
                    .block_task()
                    .await?;
                if policy_denied.load(Ordering::SeqCst) {
                    return Err(agent_client_protocol::Error::internal_error()
                        .data("Astra Pi ACP internal session requested a denied permission"));
                }
                if response.stop_reason == StopReason::Cancelled {
                    return Err(agent_client_protocol::Error::internal_error()
                        .data("Astra Pi ACP internal session was cancelled"));
                }
                let output = text.lock().map(|value| value.clone()).unwrap_or_default();
                log::info!(
                    "[astra:astra-pi-acp:stage] purpose={} runId={} stage=prompt:ok sessionId={} stopReason={:?} outputChars={}",
                    purpose,
                    run_id,
                    session_id,
                    response.stop_reason,
                    output.chars().count()
                );
                Ok::<AstraPiAcpTextResponse, agent_client_protocol::Error>(AstraPiAcpTextResponse {
                    text: output,
                    session_id,
                })
            }
        })
        .await
        .map_err(|error| {
            let message = error.to_string();
            let session_id = failure_session_id
                .lock()
                .ok()
                .and_then(|value| value.clone());
            classify_astra_pi_acp_error(
                message,
                failure_policy_denied.load(Ordering::SeqCst),
                session_id,
            )
        })
}

fn classify_astra_pi_acp_error(
    message: String,
    policy_denied: bool,
    session_id: Option<String>,
) -> AstraPiAcpFailure {
    if policy_denied || message.contains("denied permission") {
        AstraPiAcpFailure::new("policy_denied", message).with_session_id(session_id)
    } else {
        AstraPiAcpFailure::new("transport_failure", message).with_session_id(session_id)
    }
}

pub(super) fn prepare_astra_pi_agent_config(
    config: &AstraPiAcpConfig,
    provider: &AstraPiAcpProviderConfig,
) -> Result<(), AstraPiAcpFailure> {
    let agent_dir = PathBuf::from(&config.agent_dir);
    std::fs::create_dir_all(&agent_dir)
        .map_err(|error| AstraPiAcpFailure::new("config_write_failed", error.to_string()))?;
    if super::ENABLE_ASTRA_PI_SESSION_PERSISTENCE {
        std::fs::create_dir_all(&config.session_dir)
            .map_err(|error| AstraPiAcpFailure::new("config_write_failed", error.to_string()))?;
    }

    let provider_id = provider
        .provider
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openai");
    let model_id = provider
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("gpt-5.5");
    let base_url = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("http://127.0.0.1:15721/v1");
    let api = provider
        .api
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("openai-responses");

    let mut settings = json!({
        "defaultProvider": provider_id,
        "defaultModel": model_id,
        "defaultThinkingLevel": provider.thinking_level.as_deref().unwrap_or("off"),
        "quietStartup": true,
        "packages": [],
    });
    if super::ENABLE_ASTRA_PI_SESSION_PERSISTENCE {
        if let Some(settings) = settings.as_object_mut() {
            settings.insert("sessionStore".to_string(), Value::String("jsonl".to_string()));
            settings.insert(
                "sessionDurability".to_string(),
                Value::String("strict".to_string()),
            );
        }
    }
    let mut provider_json = serde_json::Map::new();
    provider_json.insert("baseUrl".to_string(), Value::String(base_url.to_string()));
    provider_json.insert("api".to_string(), Value::String(api.to_string()));
    if let Some(api_key) = provider
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        provider_json.insert("apiKey".to_string(), Value::String(api_key.to_string()));
    }
    provider_json.insert(
        "models".to_string(),
        Value::Array(vec![json!({ "id": model_id, "reasoning": true })]),
    );
    let models = json!({
        "providers": {
            provider_id: Value::Object(provider_json)
        }
    });

    write_json_file(&agent_dir.join("settings.json"), &settings)?;
    write_json_file(&agent_dir.join("models.json"), &models)?;
    log::info!(
        "[astra:astra-pi-acp:config] agentDir={} provider={} api={} baseUrl={} model={} thinkingLevel={:?} apiKeySet={}",
        agent_dir.display(),
        provider_id,
        api,
        base_url,
        model_id,
        provider.thinking_level,
        provider.api_key.as_deref().map(|value| !value.trim().is_empty()).unwrap_or(false)
    );
    Ok(())
}

fn write_json_file(path: &std::path::Path, value: &Value) -> Result<(), AstraPiAcpFailure> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| AstraPiAcpFailure::new("config_write_failed", error.to_string()))?;
    std::fs::write(path, text)
        .map_err(|error| AstraPiAcpFailure::new("config_write_failed", error.to_string()))
}

fn internal_astra_pi_acp_meta(
    config: &AstraPiAcpConfig,
    provider_config: &AstraPiAcpProviderConfig,
    purpose: AstraPiAcpPurpose,
) -> serde_json::Map<String, Value> {
    let mut meta = serde_json::Map::new();
    meta.insert(
        "purpose".to_string(),
        Value::String(purpose.as_str().to_string()),
    );
    if let Some(model) = provider_config
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        meta.insert("model".to_string(), Value::String(model.to_string()));
    }
    if let Some(thinking_level) = provider_config
        .thinking_level
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        meta.insert(
            "thinkingLevel".to_string(),
            Value::String(thinking_level.to_string()),
        );
    }
    if let Some(provider) = provider_config
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        meta.insert("provider".to_string(), Value::String(provider.to_string()));
    }
    if let Some(base_url) = provider_config
        .base_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        meta.insert("baseUrl".to_string(), Value::String(base_url.to_string()));
    }
    if super::ENABLE_ASTRA_PI_SESSION_PERSISTENCE {
        meta.insert(
            "sessionDir".to_string(),
            Value::String(config.session_dir.clone()),
        );
    }
    meta
}

fn purpose_config(
    config: &AstraPiAcpConfig,
    purpose: AstraPiAcpPurpose,
) -> &AstraPiAcpPurposeConfig {
    match purpose {
        AstraPiAcpPurpose::Orchestration => &config.orchestrator,
    }
}

fn collect_notification_text(
    notification: &SessionNotification,
    text: &Arc<Mutex<String>>,
) -> Result<()> {
    if let SessionUpdate::AgentMessageChunk(chunk) = &notification.update {
        text.lock()
            .map_err(|_| anyhow::anyhow!("Astra Pi ACP text buffer lock poisoned"))?
            .push_str(&content_chunk_text(chunk)?);
    }
    Ok(())
}

fn content_chunk_text(chunk: &ContentChunk) -> Result<String> {
    match &chunk.content {
        ContentBlock::Text(text) => Ok(text.text.clone()),
        other => serde_json::to_string(other).map_err(anyhow::Error::from),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct RawAstraPiAcpOrchestration {
    summary: Option<String>,
    run_intent: Option<AstraRunIntent>,
    reason: Option<String>,
    mode: Option<PlanRoundMode>,
    #[serde(default)]
    tasks: Vec<RawAstraPiAcpTask>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
struct RawAstraPiAcpTask {
    #[serde(rename = "id")]
    id: Option<String>,
    title: Option<String>,
    assistant_id: Option<String>,
    target_stage_id: Option<String>,
    target_agent: Option<String>,
    prompt: Option<String>,
    expected_output: Option<String>,
    risk: Option<String>,
}

pub(super) fn parse_astra_pi_acp_orchestration_response(
    response: &str,
    run: &AstraRun,
    thread: &ThreadInfo,
    round_index: u32,
    completions: &[AstraTaskCompletion],
) -> Result<AstraOrchestration, AstraPiAcpFailure> {
    let value = parse_yaml_mapping(response)?;
    reject_legacy_orchestration_fields(&value)?;
    let raw: RawAstraPiAcpOrchestration = serde_yaml::from_value(value)
        .map_err(|error| AstraPiAcpFailure::new("invalid_yaml", error.to_string()))?;
    let RawAstraPiAcpOrchestration {
        summary,
        run_intent,
        reason,
        mode,
        tasks: raw_tasks,
    } = raw;

    let run_intent = run_intent.ok_or_else(|| {
        AstraPiAcpFailure::new("validation_failed", "orchestration missing runIntent")
    })?;
    if run_intent == AstraRunIntent::Continue && thread.kind != ThreadKind::Teamwork {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            "Astra automatic orchestration is only supported for teamwork threads",
        ));
    }
    let reason = reason
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default_orchestration_reason(run_intent, completions.len()));
    let mut tasks = Vec::new();
    let mut invalid_messages = Vec::new();
    for (idx, task) in raw_tasks.into_iter().enumerate() {
        match sanitize_astra_pi_acp_task(task, run, thread, round_index, idx) {
            Ok(task) => tasks.push(task),
            Err(error) => invalid_messages.push(error.message),
        }
    }
    if !invalid_messages.is_empty() {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            format!(
                "invalid Astra Pi ACP orchestrator task(s): {}",
                invalid_messages.join("; ")
            ),
        ));
    }
    validate_orchestration_contract(thread, run_intent, mode, &tasks)?;

    Ok(AstraOrchestration {
        summary: summary
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                format!(
                "Astra Pi Orchestrator handled {} completion(s) with intent {} and selected {} task(s).",
                completions.len(),
                run_intent.as_str(),
                tasks.len()
            )
            }),
        run_intent,
        reason,
        mode,
        tasks,
        diagnostics: Vec::new(),
    })
}

fn reject_legacy_orchestration_fields(value: &YamlValue) -> Result<(), AstraPiAcpFailure> {
    let Some(object) = value.as_mapping() else {
        return Ok(());
    };
    for key in [
        "decisions",
        "decision",
        "action",
        "outcome",
        "issueStatus",
        "targetStageId",
        "stage",
        "issue",
        "retry",
    ] {
        let key_value = YamlValue::String(key.to_string());
        if object.contains_key(&key_value) {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                format!("legacy Astra orchestration field is not supported: {key}"),
            ));
        }
    }
    Ok(())
}

fn validate_orchestration_contract(
    thread: &ThreadInfo,
    run_intent: AstraRunIntent,
    mode: Option<PlanRoundMode>,
    tasks: &[AstraTaskProposal],
) -> Result<(), AstraPiAcpFailure> {
    match run_intent {
        AstraRunIntent::Continue => {
            if thread.kind != ThreadKind::Teamwork {
                return Err(AstraPiAcpFailure::new(
                    "validation_failed",
                    "Astra automatic orchestration is only supported for teamwork threads",
                ));
            }
            if mode.is_none() {
                return Err(AstraPiAcpFailure::new(
                    "validation_failed",
                    "continue runIntent requires mode",
                ));
            }
            if tasks.is_empty() {
                return Err(AstraPiAcpFailure::new(
                    "validation_failed",
                    "continue runIntent requires at least one task",
                ));
            }
        }
        AstraRunIntent::Complete | AstraRunIntent::WaitForHuman | AstraRunIntent::Error => {
            if mode.is_some() {
                return Err(AstraPiAcpFailure::new(
                    "validation_failed",
                    "terminal runIntent must not include mode",
                ));
            }
            if !tasks.is_empty() {
                return Err(AstraPiAcpFailure::new(
                    "validation_failed",
                    "terminal runIntent must not include tasks",
                ));
            }
        }
    }
    Ok(())
}

fn default_orchestration_reason(intent: AstraRunIntent, completion_count: usize) -> String {
    match intent {
        AstraRunIntent::Continue => "continue_with_next_plan_round",
        AstraRunIntent::Complete => "orchestration_complete",
        AstraRunIntent::WaitForHuman => "waiting_for_human",
        AstraRunIntent::Error => "orchestration_error",
    }
    .to_string()
        + &format!("_after_{}_completion(s)", completion_count)
}

fn sanitize_astra_pi_acp_task(
    raw: RawAstraPiAcpTask,
    run: &AstraRun,
    thread: &ThreadInfo,
    round_index: u32,
    idx: usize,
) -> Result<AstraTaskProposal, AstraPiAcpFailure> {
    let _raw_id = raw.id;
    let prompt = raw
        .prompt
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AstraPiAcpFailure::new("validation_failed", "task missing prompt"))?;
    let assistant_id = raw.assistant_id.filter(|value| !value.trim().is_empty());
    let stage_id = raw.target_stage_id.filter(|value| !value.trim().is_empty());
    if thread.kind != ThreadKind::Teamwork {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            "Astra automatic orchestration tasks are only supported for teamwork threads",
        ));
    }
    if stage_id.is_some() {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            "teamwork task must not include targetStageId",
        ));
    }
    let assistant_id = assistant_id.ok_or_else(|| {
        AstraPiAcpFailure::new("validation_failed", "teamwork task missing assistantId")
    })?;
    let assistant = thread
        .assistants
        .iter()
        .find(|assistant| assistant.assistant_id == assistant_id)
        .ok_or_else(|| AstraPiAcpFailure::new("validation_failed", "unknown assistantId"))?;
    let assistant_agent = Agent::from_db_str(&assistant.agent.id).ok_or_else(|| {
        AstraPiAcpFailure::new("validation_failed", "assistant has no valid runtime agent")
    })?;
    let target_agent = raw
        .target_agent
        .as_deref()
        .and_then(Agent::from_db_str)
        .unwrap_or(assistant_agent);
    if target_agent != assistant_agent {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            "task targetAgent does not match assistantId",
        ));
    }
    let title = raw
        .title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("{} teamwork task", assistant.name));
    let id = format!(
        "task-{}",
        short_hash(&format!(
            "{}:{}:{}:{}:{}",
            run.run_id, run.thread_id, assistant_id, round_index, idx
        ))
    );
    Ok(AstraTaskProposal {
        id,
        plan_task_id: None,
        assistant_id: Some(assistant_id),
        title,
        target_stage_id: None,
        target_agent,
        prompt,
        expected_output: raw
            .expected_output
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                "Teamwork task result, concrete progress, decisions, and verification notes."
                    .to_string()
            }),
        risk: parse_task_risk(raw.risk.as_deref()),
    })
}

fn parse_task_risk(value: Option<&str>) -> AstraTaskRisk {
    match value.unwrap_or_default().to_ascii_lowercase().as_str() {
        "high" => AstraTaskRisk::High,
        "medium" => AstraTaskRisk::Medium,
        _ => AstraTaskRisk::Low,
    }
}

fn parse_yaml_mapping(response: &str) -> Result<YamlValue, AstraPiAcpFailure> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(AstraPiAcpFailure::new(
            "empty_response",
            "Astra Pi ACP returned an empty response",
        ));
    }
    if trimmed.contains("```") {
        return Err(AstraPiAcpFailure::new(
            "invalid_yaml",
            "Astra Pi ACP response must be a plain YAML mapping, not a markdown code fence",
        ));
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Err(AstraPiAcpFailure::new(
            "invalid_yaml",
            "JSON Astra orchestration responses are not supported; return a YAML mapping",
        ));
    }
    let value: YamlValue = serde_yaml::from_str(trimmed)
        .map_err(|error| AstraPiAcpFailure::new("invalid_yaml", error.to_string()))?;
    if !value.is_mapping() {
        return Err(AstraPiAcpFailure::new(
            "invalid_yaml",
            "Astra Pi ACP response YAML must be a mapping",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> AstraRun {
        AstraRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp/project".to_string(),
            status: super::super::AstraRunStatus::Planning,
            mode: "rust_native".to_string(),
            planner_backend: None,
            round_index: None,
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn teamwork_thread() -> ThreadInfo {
        let mut thread = super::super::tests::test_thread(Vec::new());
        thread.kind = crate::models::ThreadKind::Teamwork;
        thread.assistants = vec![
            crate::models::ThreadAssistantInfo {
                assistant_id: "assistant-codex".to_string(),
                name: "Builder".to_string(),
                color: None,
                agent: crate::models::AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-write".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Build carefully.".to_string()),
                order: 0,
            },
            crate::models::ThreadAssistantInfo {
                assistant_id: "assistant-claude".to_string(),
                name: "Reviewer".to_string(),
                color: None,
                agent: crate::models::AssistantAgentInfo {
                    id: "claude".to_string(),
                    name: "Claude".to_string(),
                    model: "claude-sonnet-4-5".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Review carefully.".to_string()),
                order: 1,
            },
        ];
        thread
    }

    #[test]
    fn parses_yaml_teamwork_continue_and_sanitizes_assistant_task() {
        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"
summary: ok
runIntent: continue
reason: initial_plan
mode: parallel
tasks:
  - id: bad id!
    title: Build
    assistantId: assistant-codex
    targetAgent: codex
    prompt: Work
    expectedOutput: Notes
    risk: medium
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap();

        assert_eq!(orchestration.summary, "ok");
        assert_eq!(orchestration.run_intent, AstraRunIntent::Continue);
        assert_eq!(orchestration.reason, "initial_plan");
        assert_eq!(orchestration.mode, Some(PlanRoundMode::Parallel));
        assert_eq!(orchestration.tasks.len(), 1);
        assert!(orchestration.tasks[0].id.starts_with("task-"));
        assert_eq!(
            orchestration.tasks[0].assistant_id.as_deref(),
            Some("assistant-codex")
        );
        assert_eq!(orchestration.tasks[0].target_stage_id, None);
        assert_eq!(orchestration.tasks[0].target_agent, Agent::Codex);
        assert_eq!(orchestration.tasks[0].risk, AstraTaskRisk::Medium);
    }

    #[test]
    fn parses_teamwork_sequential_tasks_without_batch_trimming() {
        let response = r#"
summary: teamwork
runIntent: continue
reason: ordered_plan
mode: sequential
tasks:
  - title: Build
    assistantId: assistant-codex
    targetAgent: codex
    prompt: Build the feature
  - title: Review
    assistantId: assistant-claude
    targetAgent: claude
    prompt: Review the feature
"#;

        let orchestration =
            parse_astra_pi_acp_orchestration_response(response, &run(), &teamwork_thread(), 0, &[])
                .unwrap();

        assert_eq!(orchestration.run_intent, AstraRunIntent::Continue);
        assert_eq!(orchestration.mode, Some(PlanRoundMode::Sequential));
        assert_eq!(orchestration.tasks.len(), 2);
        assert!(orchestration
            .tasks
            .iter()
            .all(|task| task.target_stage_id.is_none()));
        assert_eq!(
            orchestration.tasks[0].assistant_id.as_deref(),
            Some("assistant-codex")
        );
        assert_eq!(
            orchestration.tasks[1].assistant_id.as_deref(),
            Some("assistant-claude")
        );
        assert_eq!(orchestration.tasks[0].target_agent, Agent::Codex);
        assert_eq!(orchestration.tasks[1].target_agent, Agent::Claude);
    }

    #[test]
    fn rejects_legacy_decisions_field() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: done
runIntent: continue
reason: legacy
mode: parallel
decisions: []
tasks: []
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("legacy Astra orchestration field"));
    }

    #[test]
    fn rejects_continue_without_mode() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: bad
runIntent: continue
reason: missing_mode
tasks:
  - assistantId: assistant-codex
    targetAgent: codex
    prompt: Work
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("requires mode"));
    }

    #[test]
    fn rejects_continue_with_empty_tasks() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: bad
runIntent: continue
reason: empty
mode: parallel
tasks: []
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("requires at least one task"));
    }

    #[test]
    fn rejects_continue_for_workflow_thread() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: bad
runIntent: continue
reason: workflow
mode: parallel
tasks:
  - title: Thread task
    targetAgent: codex
    prompt: Work thread
"#,
            &run(),
            &super::super::tests::test_thread(Vec::new()),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("only supported for teamwork"));
    }

    #[test]
    fn rejects_terminal_intent_with_mode_or_tasks() {
        let mode_error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: done
runIntent: complete
reason: done
mode: parallel
tasks: []
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();
        assert_eq!(mode_error.code, "validation_failed");
        assert!(mode_error.message.contains("must not include mode"));

        let tasks_error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: done
runIntent: complete
reason: done
mode: null
tasks:
  - assistantId: assistant-codex
    targetAgent: codex
    prompt: Work
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();
        assert_eq!(tasks_error.code, "validation_failed");
        assert!(tasks_error.message.contains("must not include tasks"));
    }

    #[test]
    fn accepts_terminal_intents_without_tasks() {
        for (intent, expected) in [
            ("complete", AstraRunIntent::Complete),
            ("wait_for_human", AstraRunIntent::WaitForHuman),
            ("error", AstraRunIntent::Error),
        ] {
            let response = format!(
                r#"
summary: terminal
runIntent: {intent}
reason: terminal_reason
mode: null
tasks: []
"#
            );
            let orchestration = parse_astra_pi_acp_orchestration_response(
                &response,
                &run(),
                &teamwork_thread(),
                0,
                &[],
            )
            .unwrap();

            assert_eq!(orchestration.run_intent, expected);
            assert_eq!(orchestration.reason, "terminal_reason");
            assert_eq!(orchestration.mode, None);
            assert!(orchestration.tasks.is_empty());
        }
    }

    #[test]
    fn rejects_teamwork_task_with_target_stage_id() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: bad
runIntent: continue
reason: bad_task
mode: parallel
tasks:
  - assistantId: assistant-codex
    targetStageId: stage-1
    targetAgent: codex
    prompt: Work
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("must not include targetStageId"));
    }

    #[test]
    fn rejects_teamwork_task_with_unknown_assistant_id() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: bad
runIntent: continue
reason: bad_task
mode: parallel
tasks:
  - assistantId: assistant-missing
    targetAgent: codex
    prompt: Work
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("unknown assistantId"));
    }

    #[test]
    fn rejects_teamwork_task_with_mismatched_target_agent() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"
summary: bad
runIntent: continue
reason: bad_task
mode: parallel
tasks:
  - assistantId: assistant-codex
    targetAgent: claude
    prompt: Work
"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("does not match assistantId"));
    }

    #[test]
    fn rejects_json_orchestration_response() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"bad","runIntent":"continue","reason":"json","mode":"parallel","tasks":[]}"#,
            &run(),
            &teamwork_thread(),
            0,
            &[]
        )
        .unwrap_err();

        assert_eq!(error.code, "invalid_yaml");
        assert!(error.message.contains("JSON"));
    }

    #[test]
    fn rejects_fenced_yaml_orchestration_response() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"```yaml
summary: bad
runIntent: complete
reason: fenced
mode: null
tasks: []
```"#,
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "invalid_yaml");
        assert!(error.message.contains("code fence"));
    }

    #[test]
    fn rejects_orchestration_without_yaml_mapping() {
        let error = parse_astra_pi_acp_orchestration_response(
            "no yaml mapping here",
            &run(),
            &teamwork_thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "invalid_yaml");
        assert!(error.message.contains("must be a mapping"));
    }

    #[test]
    fn policy_denied_flag_wins_over_transport_message() {
        let failure = classify_astra_pi_acp_error(
            "transport disconnected".to_string(),
            true,
            Some("session-1".to_string()),
        );

        assert_eq!(failure.code, "policy_denied");
        assert_eq!(failure.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn writes_isolated_pi_agent_provider_config() {
        let root = std::env::temp_dir().join(format!(
            "sessio-astra-pi-config-{}",
            super::super::short_hash(&format!(
                "{:?}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
            ))
        ));
        let agent_dir = root.join("agent");
        let session_dir = root.join("sessions");
        let config = AstraPiAcpConfig {
            command: "astra-pi --acp".to_string(),
            session_dir: session_dir.to_string_lossy().to_string(),
            agent_dir: agent_dir.to_string_lossy().to_string(),
            orchestrator: AstraPiAcpPurposeConfig {
                timeout_ms: super::super::ASTRA_ORCHESTRATOR_TIMEOUT_MS,
            },
        };
        let provider = AstraPiAcpProviderConfig {
            provider: Some("custom-endpoint".to_string()),
            api: Some("openai-responses".to_string()),
            base_url: Some("https://example.test/v1".to_string()),
            api_key: Some("secret-key".to_string()),
            model: Some("gpt-test".to_string()),
            thinking_level: Some("high".to_string()),
        };

        prepare_astra_pi_agent_config(&config, &provider).unwrap();

        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(agent_dir.join("settings.json")).unwrap(),
        )
        .unwrap();
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(agent_dir.join("models.json")).unwrap())
                .unwrap();
        assert_eq!(settings["defaultProvider"], "custom-endpoint");
        assert_eq!(settings["defaultModel"], "gpt-test");
        assert_eq!(settings["defaultThinkingLevel"], "high");
        assert!(settings.get("sessionStore").is_none());
        assert!(settings.get("sessionDurability").is_none());
        assert_eq!(
            models["providers"]["custom-endpoint"]["baseUrl"],
            "https://example.test/v1"
        );
        assert_eq!(
            models["providers"]["custom-endpoint"]["api"],
            "openai-responses"
        );
        assert_eq!(
            models["providers"]["custom-endpoint"]["apiKey"],
            "secret-key"
        );
        assert_eq!(
            models["providers"]["custom-endpoint"]["models"][0]["id"],
            "gpt-test"
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
