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

use super::{
    pick_stage_agent, rolling_stage_task_batch, short_hash, stage_label, summarize_task_output,
    task_blocked_by_thread_exception, AstraDecision, AstraOrchestration, AstraRun,
    AstraTaskCompletion, AstraTaskDecision, AstraTaskProposal, AstraTaskResult, AstraTaskRisk,
};
use crate::astra::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use crate::astra::prompt::build_astra_orchestration_prompt;
use crate::models::{Agent, IssueSeverity, IssueStatus, StageStatus, ThreadInfo};

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
                let backend_key = if purpose == "decision" {
                    "decisionBackend"
                } else {
                    "plannerBackend"
                };
                let mut sessio_meta = serde_json::Map::new();
                sessio_meta.insert("astraRunId".to_string(), Value::String(run_id.clone()));
                sessio_meta.insert(
                    "astraInternalPurpose".to_string(),
                    Value::String(purpose.clone()),
                );
                sessio_meta.insert(backend_key.to_string(), Value::String("astra_pi_acp".to_string()));
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
    std::fs::create_dir_all(&config.session_dir)
        .map_err(|error| AstraPiAcpFailure::new("config_write_failed", error.to_string()))?;

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

    let settings = json!({
        "defaultProvider": provider_id,
        "defaultModel": model_id,
        "defaultThinkingLevel": provider.thinking_level.as_deref().unwrap_or("off"),
        "sessionStore": "jsonl",
        "sessionDurability": "strict",
        "quietStartup": true,
        "packages": [],
    });
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
    meta.insert(
        "sessionDir".to_string(),
        Value::String(config.session_dir.clone()),
    );
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
#[serde(rename_all = "camelCase")]
struct RawAstraPiAcpOrchestration {
    summary: Option<String>,
    #[serde(default)]
    decisions: Vec<RawAstraPiAcpTaskDecision>,
    #[serde(default)]
    tasks: Vec<RawAstraPiAcpTask>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAstraPiAcpTaskDecision {
    task_id: String,
    #[serde(default)]
    decision: Option<RawAstraPiAcpDecision>,
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    stage: Option<Value>,
    #[serde(default)]
    issue: Option<Value>,
    #[serde(default)]
    retry: Option<Value>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, alias = "threadStageId")]
    stage_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

impl RawAstraPiAcpTaskDecision {
    fn into_decision(self) -> Result<RawAstraPiAcpDecision, AstraPiAcpFailure> {
        if let Some(decision) = self.decision {
            return Ok(decision);
        }
        let action = self.action.ok_or_else(|| {
            AstraPiAcpFailure::new(
                "validation_failed",
                format!(
                    "decision missing action for completed task: {}",
                    self.task_id
                ),
            )
        })?;
        Ok(RawAstraPiAcpDecision {
            action,
            stage: self.stage,
            issue: self.issue,
            retry: self.retry,
            summary: self.summary,
            stage_id: self.stage_id,
            status: self.status,
            outcome: self.outcome,
            reason: self.reason,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAstraPiAcpTask {
    #[serde(rename = "id")]
    id: Option<String>,
    title: Option<String>,
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
    let value = parse_json_object(response)?;
    let raw: RawAstraPiAcpOrchestration = serde_json::from_value(value)
        .map_err(|error| AstraPiAcpFailure::new("invalid_json", error.to_string()))?;
    let RawAstraPiAcpOrchestration {
        summary,
        decisions: raw_decisions,
        tasks: raw_tasks,
    } = raw;

    let mut decisions = Vec::new();
    for raw_decision in raw_decisions {
        let task_id = raw_decision.task_id.clone();
        let completion = completions
            .iter()
            .find(|completion| completion.task.id == task_id)
            .ok_or_else(|| {
                AstraPiAcpFailure::new(
                    "validation_failed",
                    format!("decision references unknown completed task: {}", task_id),
                )
            })?;
        let decision = sanitize_astra_pi_acp_decision(
            raw_decision.into_decision()?,
            thread,
            &completion.result,
            &completion.task,
        )?;
        decisions.push(AstraTaskDecision {
            task_id: completion.task.id.clone(),
            decision,
        });
    }
    for completion in completions {
        if !decisions
            .iter()
            .any(|decision| decision.task_id == completion.task.id)
        {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                format!(
                    "missing decision for completed task: {}",
                    completion.task.id
                ),
            ));
        }
    }

    let planning_thread = thread_after_decisions(thread, &decisions);
    let mut tasks = Vec::new();
    let mut invalid_messages = Vec::new();
    for (idx, task) in raw_tasks.into_iter().enumerate() {
        match sanitize_astra_pi_acp_task(task, run, &planning_thread, round_index, idx) {
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
    tasks = rolling_stage_task_batch(tasks);

    Ok(AstraOrchestration {
        summary: summary
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                format!(
                "Astra Pi Orchestrator handled {} completion(s) and selected {} rolling task(s).",
                completions.len(),
                tasks.len()
            )
            }),
        decisions,
        tasks,
    })
}

fn thread_after_decisions(thread: &ThreadInfo, decisions: &[AstraTaskDecision]) -> ThreadInfo {
    let mut projected = thread.clone();
    for task_decision in decisions {
        apply_decision_to_projected_thread(&mut projected, &task_decision.decision);
    }
    projected
}

fn apply_decision_to_projected_thread(thread: &mut ThreadInfo, decision: &AstraDecision) {
    match decision {
        AstraDecision::UpdateStage { args } => {
            let Some(stage_id) = args
                .get("threadStageId")
                .or_else(|| args.get("stageId"))
                .or_else(|| args.get("id"))
                .and_then(Value::as_str)
            else {
                return;
            };
            let Some(stage) = thread.stages.iter_mut().find(|stage| stage.id == stage_id) else {
                return;
            };
            if let Some(status) = args
                .get("status")
                .and_then(Value::as_str)
                .and_then(StageStatus::from_db_str)
            {
                stage.status = status;
            }
            if let Some(summary) = args
                .get("summary")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                stage.summary = Some(summary.to_string());
            }
            if let Some(outcome) = args
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                stage.outcome = Some(outcome.to_string());
            }
        }
        AstraDecision::Composite { decisions } => {
            for decision in decisions {
                apply_decision_to_projected_thread(thread, decision);
            }
        }
        AstraDecision::AddOrUpdateIssue { args } => {
            let Some(stage_id) = args
                .get("threadStageId")
                .or_else(|| args.get("stageId"))
                .and_then(Value::as_str)
            else {
                return;
            };
            let Some(stage) = thread.stages.iter_mut().find(|stage| stage.id == stage_id) else {
                return;
            };
            let issue_id = args
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let status = args
                .get("status")
                .and_then(Value::as_str)
                .and_then(IssueStatus::from_db_str)
                .unwrap_or(IssueStatus::Open);
            let Some(issue) = stage.issues.iter_mut().find(|issue| {
                issue_id.is_some_and(|id| issue.id == id)
                    || title.is_some_and(|title| issue.title.eq_ignore_ascii_case(title))
            }) else {
                return;
            };
            issue.status = status;
            if let Some(severity) = args
                .get("severity")
                .and_then(Value::as_str)
                .and_then(IssueSeverity::from_db_str)
            {
                issue.severity = severity;
            }
            if let Some(title) = title {
                issue.title = title.to_string();
            }
            if let Some(description) = args.get("description").and_then(Value::as_str) {
                issue.description = Some(description.to_string());
            }
        }
        _ => {}
    }
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
    let stage_id = raw.target_stage_id.filter(|value| !value.trim().is_empty());
    let stage = stage_id
        .as_deref()
        .map(|stage_id| {
            thread
                .stages
                .iter()
                .find(|stage| stage.id == stage_id)
                .ok_or_else(|| AstraPiAcpFailure::new("validation_failed", "unknown targetStageId"))
        })
        .transpose()?;
    let (target_stage_id, target_agent, fallback_title, id_scope) = if let Some(stage) = stage {
        if matches!(stage.status, StageStatus::Completed | StageStatus::Skipped) {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                "task targets terminal stage",
            ));
        }
        if let Some(reason) = task_blocked_by_thread_exception(run, thread, Some(&stage.id)) {
            return Err(AstraPiAcpFailure::new("validation_failed", reason));
        }
        let assignable_agent = pick_stage_agent(stage).ok_or_else(|| {
            AstraPiAcpFailure::new("validation_failed", "stage has no assignable agent")
        })?;
        let target_agent = raw
            .target_agent
            .as_deref()
            .and_then(Agent::from_db_str)
            .unwrap_or(assignable_agent);
        if target_agent != assignable_agent {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                "task targetAgent is not assignable for targetStageId",
            ));
        }
        (
            Some(stage.id.clone()),
            target_agent,
            format!(
                "{} {}",
                if super::stage_needs_agent_review(stage) {
                    "Review"
                } else {
                    "Advance"
                },
                stage_label(stage)
            ),
            stage.id.clone(),
        )
    } else {
        if let Some(reason) = task_blocked_by_thread_exception(run, thread, None) {
            return Err(AstraPiAcpFailure::new("validation_failed", reason));
        }
        let target_agent = raw
            .target_agent
            .as_deref()
            .and_then(Agent::from_db_str)
            .ok_or_else(|| {
                AstraPiAcpFailure::new(
                    "validation_failed",
                    "thread-level task missing valid targetAgent",
                )
            })?;
        (
            None,
            target_agent,
            "Advance thread".to_string(),
            "thread".to_string(),
        )
    };
    let title = raw
        .title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_title);
    let id = format!(
        "task-{}",
        short_hash(&format!(
            "{}:{}:{}:{}:{}",
            run.run_id, run.thread_id, id_scope, round_index, idx
        ))
    );
    Ok(AstraTaskProposal {
        id,
        title,
        target_stage_id,
        target_agent,
        prompt,
        expected_output: raw
            .expected_output
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Stage progress summary and verification notes.".to_string()),
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAstraPiAcpDecision {
    action: String,
    #[serde(default)]
    stage: Option<Value>,
    #[serde(default)]
    issue: Option<Value>,
    #[serde(default)]
    retry: Option<Value>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default, alias = "threadStageId")]
    stage_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
    reason: Option<String>,
}

fn stage_payload_from_decision(
    raw: &RawAstraPiAcpDecision,
) -> Result<Option<Value>, AstraPiAcpFailure> {
    let has_flat_stage_payload = raw
        .stage_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || raw
            .status
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || raw
            .summary
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || raw
            .outcome
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
    if !has_flat_stage_payload {
        return Ok(raw.stage.clone());
    }

    let mut value = raw.stage.clone().unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| AstraPiAcpFailure::new("validation_failed", "stage must be an object"))?;
    if !object.contains_key("threadStageId")
        && !object.contains_key("stageId")
        && !object.contains_key("id")
    {
        if let Some(stage_id) = raw
            .stage_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            object.insert(
                "threadStageId".to_string(),
                Value::String(stage_id.to_string()),
            );
        }
    }
    if !object.contains_key("status") {
        if let Some(status) = raw
            .status
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            object.insert("status".to_string(), Value::String(status.to_string()));
        }
    }
    if !object.contains_key("summary") {
        if let Some(summary) = raw
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            object.insert("summary".to_string(), Value::String(summary.to_string()));
        }
    }
    if !object.contains_key("outcome") {
        if let Some(outcome) = raw
            .outcome
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            object.insert("outcome".to_string(), Value::String(outcome.to_string()));
        }
    }
    Ok(Some(value))
}

fn sanitize_astra_pi_acp_decision(
    raw: RawAstraPiAcpDecision,
    thread: &ThreadInfo,
    result: &AstraTaskResult,
    task: &AstraTaskProposal,
) -> Result<AstraDecision, AstraPiAcpFailure> {
    let reason = raw
        .reason
        .as_deref()
        .or(raw.summary.as_deref())
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| summarize_task_output(&result.output));
    match raw.action.as_str() {
        "update_stage" => Ok(AstraDecision::UpdateStage {
            args: stage_decision_args(
                stage_payload_from_decision(&raw)?,
                thread,
                result,
                task,
                &reason,
            )?,
        }),
        "add_or_update_issue" => Ok(AstraDecision::AddOrUpdateIssue {
            args: issue_decision_args(raw.issue, thread, result, task, &reason)?,
        }),
        "retry_stage" => {
            if result.retry_limit_reached {
                Ok(AstraDecision::ErrorRun {
                    reason: "retry limit reached".to_string(),
                })
            } else {
                let retry_reason = raw
                    .retry
                    .as_ref()
                    .and_then(|value| value.get("reason"))
                    .and_then(Value::as_str)
                    .unwrap_or(&reason)
                    .to_string();
                Ok(AstraDecision::RetryStage {
                    reason: retry_reason,
                })
            }
        }
        "plan_next_round" => Ok(AstraDecision::PlanNextRound { reason }),
        "complete_run" => Ok(AstraDecision::CompleteRun { reason }),
        "error_run" => Ok(AstraDecision::ErrorRun { reason }),
        other => Err(AstraPiAcpFailure::new(
            "validation_failed",
            format!("unknown decision action: {other}"),
        )),
    }
}

fn stage_decision_args(
    stage: Option<Value>,
    thread: &ThreadInfo,
    result: &AstraTaskResult,
    task: &AstraTaskProposal,
    fallback_summary: &str,
) -> Result<Value, AstraPiAcpFailure> {
    let mut value = stage.unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| AstraPiAcpFailure::new("validation_failed", "stage must be an object"))?;
    let stage_id = object
        .get("threadStageId")
        .or_else(|| object.get("stageId"))
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| result.thread_stage_id.clone())
        .or_else(|| task.target_stage_id.clone())
        .ok_or_else(|| {
            AstraPiAcpFailure::new("validation_failed", "stage decision missing stage id")
        })?;
    if !thread.stages.iter().any(|stage| stage.id == stage_id) {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            "stage decision references unknown stage",
        ));
    }
    object.insert("taskId".to_string(), Value::String(task.id.clone()));
    object.insert("threadStageId".to_string(), Value::String(stage_id));
    if !object.contains_key("status") {
        object.insert(
            "status".to_string(),
            Value::String("needs_review".to_string()),
        );
    } else {
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AstraPiAcpFailure::new("validation_failed", "stage status must be a string")
            })?;
        if StageStatus::from_db_str(status).is_none() {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                format!("unknown stage status: {status}"),
            ));
        }
    }
    if !object.contains_key("summary") {
        object.insert(
            "summary".to_string(),
            Value::String(fallback_summary.to_string()),
        );
    }
    if !object.contains_key("outcome") {
        object.insert(
            "outcome".to_string(),
            Value::String(fallback_summary.to_string()),
        );
    }
    Ok(value)
}

fn issue_decision_args(
    issue: Option<Value>,
    thread: &ThreadInfo,
    result: &AstraTaskResult,
    task: &AstraTaskProposal,
    fallback_summary: &str,
) -> Result<Value, AstraPiAcpFailure> {
    let mut value = issue.unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| AstraPiAcpFailure::new("validation_failed", "issue must be an object"))?;
    let stage_id = object
        .get("threadStageId")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| result.thread_stage_id.clone())
        .or_else(|| task.target_stage_id.clone())
        .ok_or_else(|| {
            AstraPiAcpFailure::new("validation_failed", "issue decision missing stage id")
        })?;
    if !thread.stages.iter().any(|stage| stage.id == stage_id) {
        return Err(AstraPiAcpFailure::new(
            "validation_failed",
            "issue decision references unknown stage",
        ));
    }
    object.insert("taskId".to_string(), Value::String(task.id.clone()));
    object.insert("threadStageId".to_string(), Value::String(stage_id));
    let has_issue_id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    if !object.contains_key("title") && !has_issue_id {
        object.insert(
            "title".to_string(),
            Value::String(format!("Astra follow-up: {}", task.title)),
        );
    }
    if !object.contains_key("description") && !has_issue_id {
        object.insert(
            "description".to_string(),
            Value::String(fallback_summary.to_string()),
        );
    }
    if !object.contains_key("severity") && !has_issue_id {
        object.insert(
            "severity".to_string(),
            Value::String(IssueSeverity::High.as_str().to_string()),
        );
    } else if object.contains_key("severity") {
        let severity = object
            .get("severity")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AstraPiAcpFailure::new("validation_failed", "issue severity must be a string")
            })?;
        if IssueSeverity::from_db_str(severity).is_none() {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                format!("unknown issue severity: {severity}"),
            ));
        }
    }
    if !object.contains_key("status") {
        object.insert(
            "status".to_string(),
            Value::String(IssueStatus::Open.as_str().to_string()),
        );
    } else {
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AstraPiAcpFailure::new("validation_failed", "issue status must be a string")
            })?;
        if IssueStatus::from_db_str(status).is_none() {
            return Err(AstraPiAcpFailure::new(
                "validation_failed",
                format!("unknown issue status: {status}"),
            ));
        }
    }
    Ok(value)
}

fn parse_json_object(response: &str) -> Result<Value, AstraPiAcpFailure> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(AstraPiAcpFailure::new(
            "empty_response",
            "Astra Pi ACP returned an empty response",
        ));
    }
    let candidate = extract_json_candidate(trimmed).ok_or_else(|| {
        AstraPiAcpFailure::new(
            "invalid_json",
            "Astra Pi ACP response did not contain a JSON object",
        )
    })?;
    let value: Value = serde_json::from_str(candidate)
        .map_err(|error| AstraPiAcpFailure::new("invalid_json", error.to_string()))?;
    if !value.is_object() {
        return Err(AstraPiAcpFailure::new(
            "invalid_json",
            "Astra Pi ACP response JSON must be an object",
        ));
    }
    Ok(value)
}

fn extract_json_candidate(value: &str) -> Option<&str> {
    if value.starts_with('{') && value.ends_with('}') {
        return Some(value);
    }
    if let Some(fenced) = value.strip_prefix("```") {
        let fenced = fenced
            .strip_prefix("json")
            .or_else(|| fenced.strip_prefix("JSON"))
            .unwrap_or(fenced)
            .trim_start();
        if let Some(end) = fenced.rfind("```") {
            return extract_json_candidate(fenced[..end].trim());
        }
    }
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(&value[start..=end])
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn run() -> AstraRun {
        AstraRun {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp/project".to_string(),
            status: super::super::AstraRunStatus::Planning,
            proposed_tasks: Vec::new(),
            approved_task_ids: Vec::new(),
            delegated_session_ids: Vec::new(),
            task_results: Vec::new(),
            mode: "rust_native".to_string(),
            current_stage_id: None,
            completed_task_ids: Vec::new(),
            stage_attempt_counts: HashMap::new(),
            retry_limit: 3,
            planner_backend: None,
            decision_backend: None,
            round_index: None,
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_session_ids: Vec::new(),
            internal_decision_session_ids: Vec::new(),
            run_diagnostics: Vec::new(),
            error: None,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn thread() -> ThreadInfo {
        let assistant = crate::models::StageAssistantInfo {
            assistant_id: "assistant-1".to_string(),
            name: "Codex".to_string(),
            color: None,
            agent: crate::models::AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: String::new(),
                mode: String::new(),
                effort: String::new(),
            },
            system_prompt: None,
            order: 0,
        };
        let mut stage = super::super::tests::test_stage("stage-1", StageStatus::InProgress);
        stage.assistants.push(assistant.clone());
        let mut next_stage = super::super::tests::test_stage("stage-2", StageStatus::InProgress);
        next_stage.assistants.push(assistant);
        super::super::tests::test_thread(vec![stage, next_stage])
    }

    #[test]
    fn parses_fenced_orchestration_and_sanitizes_task() {
        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"```json
            {"summary":"ok","tasks":[{"id":"bad id!","title":"Do it","targetStageId":"stage-1","targetAgent":"codex","prompt":"Work","expectedOutput":"Notes","risk":"medium"}]}
            ```"#,
            &run(),
            &thread(),
            0,
            &[],
        )
        .unwrap();

        assert_eq!(orchestration.summary, "ok");
        assert_eq!(orchestration.tasks.len(), 1);
        assert!(orchestration.tasks[0].id.starts_with("task-"));
        assert_eq!(orchestration.tasks[0].risk, AstraTaskRisk::Medium);
    }

    #[test]
    fn invalid_orchestration_task_fails_whole_response() {
        let error = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"bad","tasks":[{"targetStageId":"missing","targetAgent":"codex","prompt":"Work"}]}"#,
            &run(),
            &thread(),
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("unknown targetStageId"));
    }

    #[test]
    fn rejects_ordinary_task_when_any_stage_needs_review() {
        let mut thread = thread();
        thread.stages[0].status = StageStatus::NeedsReview;

        let error = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"bad","tasks":[{"targetStageId":"stage-2","targetAgent":"codex","prompt":"Work"}]}"#,
            &run(),
            &thread,
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("needing review"));
    }

    #[test]
    fn accepts_review_task_for_agent_stage_needing_review() {
        let mut thread = thread();
        thread.stages[0].status = StageStatus::NeedsReview;

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"review","tasks":[{"targetStageId":"stage-1","targetAgent":"codex","prompt":"Review the stage result"}]}"#,
            &run(),
            &thread,
            0,
            &[],
        )
        .unwrap();

        assert_eq!(orchestration.tasks.len(), 1);
        assert_eq!(
            orchestration.tasks[0].target_stage_id.as_deref(),
            Some("stage-1")
        );
    }

    #[test]
    fn rejects_agent_task_for_human_stage_needing_review() {
        let mut thread = thread();
        thread.stages[0].status = StageStatus::NeedsReview;
        thread.stages[0].kind = Some(crate::models::StageType::Human);

        let error = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"bad","tasks":[{"targetStageId":"stage-1","targetAgent":"codex","prompt":"Review"}]}"#,
            &run(),
            &thread,
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("human review"));
    }

    #[test]
    fn rejects_ordinary_task_when_blocked_stage_exists() {
        let mut thread = thread();
        thread.stages[0].status = StageStatus::Blocked;

        let error = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"bad","tasks":[{"targetStageId":"stage-2","targetAgent":"codex","prompt":"Work"}]}"#,
            &run(),
            &thread,
            0,
            &[],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("blocked stage"));
    }

    #[test]
    fn parses_thread_level_orchestration_task() {
        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"thread","tasks":[{"title":"Thread task","targetAgent":"codex","prompt":"Work thread"}]}"#,
            &run(),
            &thread(),
            0,
            &[],
        )
        .unwrap();

        assert_eq!(orchestration.tasks.len(), 1);
        assert_eq!(orchestration.tasks[0].target_stage_id, None);
        assert_eq!(orchestration.tasks[0].target_agent, Agent::Codex);
    }

    #[test]
    fn orchestration_keeps_first_stage_parallel_batch_only() {
        let mut tasks = (0..5)
            .map(|idx| {
                json!({
                    "id": format!("task-{idx}"),
                    "title": format!("Task {idx}"),
                    "targetStageId": "stage-1",
                    "targetAgent": "codex",
                    "prompt": "Work"
                })
            })
            .collect::<Vec<_>>();
        tasks.push(json!({
            "id": "task-next-stage",
            "title": "Next stage task",
            "targetStageId": "stage-2",
            "targetAgent": "codex",
            "prompt": "Work later"
        }));
        let response = json!({ "summary": "many", "tasks": tasks }).to_string();
        let orchestration =
            parse_astra_pi_acp_orchestration_response(&response, &run(), &thread(), 0, &[])
                .unwrap();

        assert_eq!(orchestration.tasks.len(), 4);
        assert_eq!(orchestration.tasks[0].title, "Task 0");
        assert!(orchestration
            .tasks
            .iter()
            .all(|task| task.target_stage_id.as_deref() == Some("stage-1")));
        assert!(!orchestration
            .tasks
            .iter()
            .any(|task| task.title == "Task 4"));
        assert!(!orchestration
            .tasks
            .iter()
            .any(|task| task.title == "Next stage task"));
    }

    #[test]
    fn rejects_orchestration_without_json_object() {
        assert!(parse_astra_pi_acp_orchestration_response(
            "no json here",
            &run(),
            &thread(),
            0,
            &[]
        )
        .is_err());
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
        assert_eq!(settings["sessionStore"], "jsonl");
        assert_eq!(settings["sessionDurability"], "strict");
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

    #[test]
    fn maps_nested_update_stage_decision() {
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Task".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "done".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"done","decisions":[{"taskId":"task-1","decision":{"action":"update_stage","stage":{"status":"completed"},"reason":"done"}}],"tasks":[{"title":"Review","targetStageId":"stage-2","targetAgent":"codex","prompt":"Review work"}]}"#,
            &run(),
            &thread(),
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap();

        assert_eq!(orchestration.decisions.len(), 1);
        assert_eq!(orchestration.tasks.len(), 1);
        assert_eq!(
            orchestration.tasks[0].target_stage_id.as_deref(),
            Some("stage-2")
        );
        match &orchestration.decisions[0].decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], "stage-1");
                assert_eq!(args["status"], "completed");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn maps_issue_status_decision_for_existing_issue() {
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Task".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "resolved".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"resolved","decisions":[{"taskId":"task-1","decision":{"action":"add_or_update_issue","issue":{"id":"issue-1","threadStageId":"stage-1","status":"resolved"},"reason":"fixed"}},{"taskId":"task-1","decision":{"action":"update_stage","stage":{"status":"completed"},"reason":"done"}}],"tasks":[]}"#,
            &run(),
            &thread(),
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap();

        assert_eq!(orchestration.decisions.len(), 2);
        match &orchestration.decisions[0].decision {
            AstraDecision::AddOrUpdateIssue { args } => {
                assert_eq!(args["id"], "issue-1");
                assert_eq!(args["threadStageId"], "stage-1");
                assert_eq!(args["status"], "resolved");
                assert!(args.get("title").is_none());
                assert!(args.get("severity").is_none());
            }
            other => panic!("unexpected decision: {other:?}"),
        }
        match &orchestration.decisions[1].decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], "stage-1");
                assert_eq!(args["status"], "completed");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn maps_legacy_flat_update_stage_decision() {
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Task".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "done".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"done","decisions":[{"taskId":"task-1","action":"update_stage","stage":{"status":"completed"},"reason":"done"}],"tasks":[]}"#,
            &run(),
            &thread(),
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap();

        assert_eq!(orchestration.decisions.len(), 1);
        match &orchestration.decisions[0].decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], "stage-1");
                assert_eq!(args["status"], "completed");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn maps_runtime_agent_flat_update_stage_decision() {
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Task".to_string(),
            target_stage_id: None,
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: None,
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "done".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"ok","decisions":[{"taskId":"task-1","decision":{"action":"update_stage","stageId":"stage-1","status":"completed","summary":"done"}}],"tasks":[{"title":"Plan next","targetStageId":"stage-2","targetAgent":"codex","prompt":"Plan"}]}"#,
            &run(),
            &thread(),
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap();

        assert_eq!(orchestration.decisions.len(), 1);
        assert_eq!(orchestration.tasks.len(), 1);
        assert_eq!(
            orchestration.tasks[0].target_stage_id.as_deref(),
            Some("stage-2")
        );
        match &orchestration.decisions[0].decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], "stage-1");
                assert_eq!(args["status"], "completed");
                assert_eq!(args["summary"], "done");
                assert_eq!(args["outcome"], "done");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }

    #[test]
    fn validates_tasks_after_projecting_stage_decisions() {
        let mut thread = thread();
        thread.stages[0].status = StageStatus::NeedsReview;

        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Review research".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Review".to_string(),
            expected_output: "Review notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "approved-with-corrections".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"reviewed","decisions":[{"taskId":"task-1","decision":{"action":"update_stage","stageId":"stage-1","status":"completed","summary":"approved"}}],"tasks":[{"title":"Plan answer","targetStageId":"stage-2","targetAgent":"codex","prompt":"Plan the answer"}]}"#,
            &run(),
            &thread,
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap();

        assert_eq!(orchestration.decisions.len(), 1);
        assert_eq!(orchestration.tasks.len(), 1);
        assert_eq!(
            orchestration.tasks[0].target_stage_id.as_deref(),
            Some("stage-2")
        );
    }

    #[test]
    fn validates_tasks_after_projecting_non_review_stage_decisions() {
        let mut thread = thread();
        thread.stages[0].status = StageStatus::Blocked;

        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Recover research".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Recover".to_string(),
            expected_output: "Recovery notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "recovered".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let orchestration = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"recovered","decisions":[{"taskId":"task-1","decision":{"action":"update_stage","stageId":"stage-1","status":"completed","summary":"recovered"}}],"tasks":[{"title":"Plan answer","targetStageId":"stage-2","targetAgent":"codex","prompt":"Plan the answer"}]}"#,
            &run(),
            &thread,
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap();

        assert_eq!(orchestration.decisions.len(), 1);
        assert_eq!(orchestration.tasks.len(), 1);
        assert_eq!(
            orchestration.tasks[0].target_stage_id.as_deref(),
            Some("stage-2")
        );
    }

    #[test]
    fn orchestration_requires_decision_for_each_completion() {
        let task = AstraTaskProposal {
            id: "task-1".to_string(),
            title: "Task".to_string(),
            target_stage_id: Some("stage-1".to_string()),
            target_agent: Agent::Codex,
            prompt: "Work".to_string(),
            expected_output: "Notes".to_string(),
            risk: AstraTaskRisk::Low,
        };
        let result = AstraTaskResult {
            task_id: "task-1".to_string(),
            thread_stage_id: Some("stage-1".to_string()),
            sessio_runtime_session_id: "runtime-1".to_string(),
            turn_id: None,
            status: super::super::AstraTaskResultStatus::Completed,
            output: "done".to_string(),
            error: None,
            attempt_count: 1,
            retry_limit_reached: false,
            decision_action: None,
            decision_reason: None,
            completed_at: 1,
        };

        let error = parse_astra_pi_acp_orchestration_response(
            r#"{"summary":"missing decision","tasks":[]}"#,
            &run(),
            &thread(),
            1,
            &[AstraTaskCompletion { task, result }],
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("missing decision"));
    }
}
