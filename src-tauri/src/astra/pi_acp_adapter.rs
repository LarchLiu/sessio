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
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    pick_stage_agent, short_hash, stage_label, summarize_task_output, AstraDecision, AstraPlan,
    AstraRun, AstraTaskProposal, AstraTaskResult, AstraTaskRisk,
};
use crate::models::{Agent, IssueSeverity, StageStatus, ThreadInfo};

#[derive(Debug, Clone)]
pub(super) struct AstraPiConfig {
    pub command: String,
    pub session_dir: String,
    pub agent_dir: String,
    pub planner: AstraPiPurposeConfig,
    pub decision: AstraPiPurposeConfig,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AstraPiProviderConfig {
    pub provider: Option<String>,
    pub api: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct AstraPiPurposeConfig {
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PiAcpPurpose {
    Planning,
    Decision,
}

impl PiAcpPurpose {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Decision => "decision",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct PiAcpFailure {
    pub code: &'static str,
    pub message: String,
    pub session_id: Option<String>,
}

impl PiAcpFailure {
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
pub(super) struct PiAcpPlanResponse {
    pub plan: AstraPlan,
    pub session_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct PiAcpDecisionResponse {
    pub decision: AstraDecision,
    pub session_id: String,
}

#[derive(Debug, Clone)]
struct PiAcpTextResponse {
    text: String,
    session_id: String,
}

#[derive(Debug, Clone)]
pub(super) struct PiAcpPlanner {
    config: AstraPiConfig,
}

impl PiAcpPlanner {
    pub(super) fn new(config: AstraPiConfig) -> Self {
        Self { config }
    }

    pub(super) fn plan(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        provider_config: &AstraPiProviderConfig,
    ) -> Result<PiAcpPlanResponse, PiAcpFailure> {
        let prompt = planning_prompt(run, thread, user_prompt, round_index);
        let response = run_internal_pi_acp(
            &self.config,
            PiAcpPurpose::Planning,
            &run.run_id,
            &run.project_path,
            &prompt,
            provider_config,
        )?;
        Ok(PiAcpPlanResponse {
            plan: parse_pi_plan_response(&response.text, run, thread, round_index)?,
            session_id: response.session_id,
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct PiAcpDecisionEngine {
    config: AstraPiConfig,
}

impl PiAcpDecisionEngine {
    pub(super) fn new(config: AstraPiConfig) -> Self {
        Self { config }
    }

    pub(super) fn decide(
        &self,
        run_id: &str,
        workspace_path: &str,
        thread: &ThreadInfo,
        result: &AstraTaskResult,
        task: &AstraTaskProposal,
        provider_config: &AstraPiProviderConfig,
    ) -> Result<PiAcpDecisionResponse, PiAcpFailure> {
        let prompt = decision_prompt(thread, result, task);
        let response = run_internal_pi_acp(
            &self.config,
            PiAcpPurpose::Decision,
            run_id,
            workspace_path,
            &prompt,
            provider_config,
        )?;
        Ok(PiAcpDecisionResponse {
            decision: parse_pi_decision_response(&response.text, thread, result, task)?,
            session_id: response.session_id,
        })
    }
}

fn run_internal_pi_acp(
    config: &AstraPiConfig,
    purpose: PiAcpPurpose,
    run_id: &str,
    workspace_path: &str,
    prompt: &str,
    provider_config: &AstraPiProviderConfig,
) -> Result<PiAcpTextResponse, PiAcpFailure> {
    let purpose_config = purpose_config(config, purpose);
    let command = config.command.clone();
    let meta = internal_pi_meta(config, provider_config, purpose);
    let timeout = Duration::from_millis(purpose_config.timeout_ms);
    let workspace = if workspace_path.trim().is_empty() {
        std::env::current_dir()
            .map_err(|error| PiAcpFailure::new("transport_failure", error.to_string()))?
    } else {
        PathBuf::from(workspace_path)
    };
    log::info!(
        "[astra:pi-acp:call] purpose={} runId={} command={} workspace={} timeoutMs={} sessionDir={} model={:?} thinkingLevel={:?} meta={} promptChars={}",
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
        let result =
            run_internal_pi_acp_async(
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
                "[astra:pi-acp:response] purpose={} sessionId={} textChars={}",
                purpose.as_str(),
                response.session_id,
                response.text.chars().count()
            );
            Ok(response)
        }
        Ok(Err(failure)) => {
            log::warn!(
                "[astra:pi-acp:error] purpose={} code={} sessionId={:?} message={}",
                purpose.as_str(),
                failure.code,
                failure.session_id,
                failure.message
            );
            Err(failure)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            handle.abort();
            let failure = PiAcpFailure::new(
                "timeout",
                format!(
                    "Pi ACP {} timed out after {}ms",
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
                "[astra:pi-acp:error] purpose={} code={} sessionId={:?} message={}",
                purpose.as_str(),
                failure.code,
                failure.session_id,
                failure.message
            );
            Err(failure)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let failure = PiAcpFailure::new(
                "transport_failure",
                format!("Pi ACP {} worker disconnected", purpose.as_str()),
            );
            log::warn!(
                "[astra:pi-acp:error] purpose={} code={} sessionId={:?} message={}",
                purpose.as_str(),
                failure.code,
                failure.session_id,
                failure.message
            );
            Err(failure)
        }
    }
}

async fn run_internal_pi_acp_async(
    command: String,
    purpose: String,
    run_id: String,
    meta: serde_json::Map<String, Value>,
    workspace: PathBuf,
    prompt: String,
    internal_session_id: Arc<Mutex<Option<String>>>,
) -> Result<PiAcpTextResponse, PiAcpFailure> {
    let agent = AcpAgent::from_str(&command)
        .map_err(|error| PiAcpFailure::new("transport_failure", error.to_string()))?;
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
                        "[astra:pi-acp:notification] purpose={} error={}",
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
                    "[astra:pi-acp:stage] purpose={} runId={} stage=initialize:start",
                    purpose,
                    run_id
                );
                let init = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
                log::info!(
                    "[astra:pi-acp:stage] purpose={} runId={} stage=initialize:ok protocolVersion={:?} capabilities={:?}",
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
                sessio_meta.insert(backend_key.to_string(), Value::String("pi_acp".to_string()));
                sessio_meta.insert("pi".to_string(), Value::Object(meta));
                request.meta = Some(
                    json!({ "sessio": Value::Object(sessio_meta) })
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                );
                log::info!(
                    "[astra:pi-acp:stage] purpose={} runId={} stage=new_session:start meta={}",
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
                    "[astra:pi-acp:stage] purpose={} runId={} stage=new_session:ok sessionId={}",
                    purpose,
                    run_id,
                    session_id
                );
                log::info!(
                    "[astra:pi-acp:stage] purpose={} runId={} stage=prompt:start sessionId={} promptChars={}",
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
                        .data("Pi ACP internal session requested a denied permission"));
                }
                if response.stop_reason == StopReason::Cancelled {
                    return Err(agent_client_protocol::Error::internal_error()
                        .data("Pi ACP internal session was cancelled"));
                }
                let output = text.lock().map(|value| value.clone()).unwrap_or_default();
                log::info!(
                    "[astra:pi-acp:stage] purpose={} runId={} stage=prompt:ok sessionId={} stopReason={:?} outputChars={}",
                    purpose,
                    run_id,
                    session_id,
                    response.stop_reason,
                    output.chars().count()
                );
                Ok::<PiAcpTextResponse, agent_client_protocol::Error>(PiAcpTextResponse {
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
            classify_pi_acp_error(
                message,
                failure_policy_denied.load(Ordering::SeqCst),
                session_id,
            )
        })
}

fn classify_pi_acp_error(
    message: String,
    policy_denied: bool,
    session_id: Option<String>,
) -> PiAcpFailure {
    if policy_denied || message.contains("denied permission") {
        PiAcpFailure::new("policy_denied", message).with_session_id(session_id)
    } else {
        PiAcpFailure::new("transport_failure", message).with_session_id(session_id)
    }
}

pub(super) fn prepare_pi_agent_config(
    config: &AstraPiConfig,
    provider: &AstraPiProviderConfig,
) -> Result<(), PiAcpFailure> {
    let agent_dir = PathBuf::from(&config.agent_dir);
    std::fs::create_dir_all(&agent_dir)
        .map_err(|error| PiAcpFailure::new("config_write_failed", error.to_string()))?;
    std::fs::create_dir_all(&config.session_dir)
        .map_err(|error| PiAcpFailure::new("config_write_failed", error.to_string()))?;

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
        "[astra:pi-acp:config] agentDir={} provider={} api={} baseUrl={} model={} thinkingLevel={:?} apiKeySet={}",
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

fn write_json_file(path: &std::path::Path, value: &Value) -> Result<(), PiAcpFailure> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| PiAcpFailure::new("config_write_failed", error.to_string()))?;
    std::fs::write(path, text)
        .map_err(|error| PiAcpFailure::new("config_write_failed", error.to_string()))
}

fn internal_pi_meta(
    config: &AstraPiConfig,
    provider_config: &AstraPiProviderConfig,
    purpose: PiAcpPurpose,
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

fn purpose_config(config: &AstraPiConfig, purpose: PiAcpPurpose) -> &AstraPiPurposeConfig {
    match purpose {
        PiAcpPurpose::Planning => &config.planner,
        PiAcpPurpose::Decision => &config.decision,
    }
}

fn collect_notification_text(
    notification: &SessionNotification,
    text: &Arc<Mutex<String>>,
) -> Result<()> {
    if let SessionUpdate::AgentMessageChunk(chunk) = &notification.update {
        text.lock()
            .map_err(|_| anyhow::anyhow!("Pi ACP text buffer lock poisoned"))?
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

fn planning_prompt(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> String {
    let stages = thread
        .stages
        .iter()
        .map(|stage| {
            let agent = pick_stage_agent(stage).map(|agent| agent.as_str().to_string());
            json!({
                "id": stage.id,
                "title": stage_label(stage),
                "order": stage.order,
                "status": stage.status,
                "assignableAgent": agent,
                "summary": stage.summary,
                "issues": stage.issues,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "instruction": "Return only a JSON object with shape {\"summary\": string, \"tasks\": array}. Each task must include title, targetStageId, targetAgent, prompt, expectedOutput, risk.",
        "thread": {
            "id": thread.id,
            "goal": thread.goal,
            "stages": stages,
        },
        "run": {
            "id": run.run_id,
            "roundIndex": round_index,
            "retryLimit": run.retry_limit,
            "completedTaskIds": run.completed_task_ids,
            "stageAttemptCounts": run.stage_attempt_counts,
        },
        "userPrompt": user_prompt.unwrap_or(""),
    })
    .to_string()
}

fn decision_prompt(
    thread: &ThreadInfo,
    result: &AstraTaskResult,
    task: &AstraTaskProposal,
) -> String {
    json!({
        "instruction": "Return only a JSON object with shape {\"action\": string, \"stage\": object?, \"issue\": object?, \"retry\": object?, \"summary\": string?, \"reason\": string}. action must be one of update_stage, add_or_update_issue, retry_stage, plan_next_round, complete_run, error_run.",
        "thread": thread,
        "task": task,
        "result": {
            "taskId": result.task_id,
            "threadStageId": result.thread_stage_id,
            "status": result.status,
            "output": result.output,
            "error": result.error,
            "attemptCount": result.attempt_count,
            "retryLimitReached": result.retry_limit_reached,
        },
    })
    .to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPiPlan {
    summary: Option<String>,
    #[serde(default)]
    tasks: Vec<RawPiTask>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPiTask {
    #[serde(rename = "id")]
    id: Option<String>,
    title: Option<String>,
    target_stage_id: Option<String>,
    target_agent: Option<String>,
    prompt: Option<String>,
    expected_output: Option<String>,
    risk: Option<String>,
}

pub(super) fn parse_pi_plan_response(
    response: &str,
    run: &AstraRun,
    thread: &ThreadInfo,
    round_index: u32,
) -> Result<AstraPlan, PiAcpFailure> {
    let value = parse_json_object(response)?;
    let raw: RawPiPlan = serde_json::from_value(value)
        .map_err(|error| PiAcpFailure::new("invalid_json", error.to_string()))?;
    let mut tasks = Vec::new();
    let mut invalid_messages = Vec::new();
    for (idx, task) in raw.tasks.into_iter().enumerate() {
        match sanitize_pi_task(task, run, thread, round_index, idx) {
            Ok(task) => tasks.push(task),
            Err(error) => invalid_messages.push(error.message),
        }
    }
    if !invalid_messages.is_empty() {
        return Err(PiAcpFailure::new(
            "validation_failed",
            format!(
                "invalid Pi planner task(s): {}",
                invalid_messages.join("; ")
            ),
        ));
    }
    tasks.truncate(20);
    Ok(AstraPlan {
        summary: raw
            .summary
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("Pi ACP planned {} task(s).", tasks.len())),
        tasks,
    })
}

fn sanitize_pi_task(
    raw: RawPiTask,
    run: &AstraRun,
    thread: &ThreadInfo,
    round_index: u32,
    idx: usize,
) -> Result<AstraTaskProposal, PiAcpFailure> {
    let _raw_id = raw.id;
    let prompt = raw
        .prompt
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| PiAcpFailure::new("validation_failed", "task missing prompt"))?;
    let stage_id = raw.target_stage_id.filter(|value| !value.trim().is_empty());
    let stage = stage_id
        .as_deref()
        .map(|stage_id| {
            thread
                .stages
                .iter()
                .find(|stage| stage.id == stage_id)
                .ok_or_else(|| PiAcpFailure::new("validation_failed", "unknown targetStageId"))
        })
        .transpose()?;
    let (target_stage_id, target_agent, fallback_title, id_scope) = if let Some(stage) = stage {
        if matches!(
            stage.status,
            StageStatus::Completed | StageStatus::Skipped | StageStatus::NeedsReview
        ) {
            return Err(PiAcpFailure::new(
                "validation_failed",
                "task targets terminal stage",
            ));
        }
        let assignable_agent = pick_stage_agent(stage).ok_or_else(|| {
            PiAcpFailure::new("validation_failed", "stage has no assignable agent")
        })?;
        let target_agent = raw
            .target_agent
            .as_deref()
            .and_then(Agent::from_db_str)
            .unwrap_or(assignable_agent);
        if target_agent != assignable_agent {
            return Err(PiAcpFailure::new(
                "validation_failed",
                "task targetAgent is not assignable for targetStageId",
            ));
        }
        (
            Some(stage.id.clone()),
            target_agent,
            format!("Advance {}", stage_label(stage)),
            stage.id.clone(),
        )
    } else {
        let target_agent = raw
            .target_agent
            .as_deref()
            .and_then(Agent::from_db_str)
            .ok_or_else(|| {
                PiAcpFailure::new(
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
struct RawPiDecision {
    action: String,
    #[serde(default)]
    stage: Option<Value>,
    #[serde(default)]
    issue: Option<Value>,
    #[serde(default)]
    retry: Option<Value>,
    #[serde(default)]
    summary: Option<String>,
    reason: Option<String>,
}

pub(super) fn parse_pi_decision_response(
    response: &str,
    thread: &ThreadInfo,
    result: &AstraTaskResult,
    task: &AstraTaskProposal,
) -> Result<AstraDecision, PiAcpFailure> {
    let value = parse_json_object(response)?;
    let raw: RawPiDecision = serde_json::from_value(value)
        .map_err(|error| PiAcpFailure::new("invalid_json", error.to_string()))?;
    let reason = raw
        .reason
        .or(raw.summary)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| summarize_task_output(&result.output));
    match raw.action.as_str() {
        "update_stage" => Ok(AstraDecision::UpdateStage {
            args: stage_decision_args(raw.stage, thread, result, task, &reason)?,
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
        other => Err(PiAcpFailure::new(
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
) -> Result<Value, PiAcpFailure> {
    let mut value = stage.unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| PiAcpFailure::new("validation_failed", "stage must be an object"))?;
    let stage_id = object
        .get("threadStageId")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| result.thread_stage_id.clone())
        .or_else(|| task.target_stage_id.clone())
        .ok_or_else(|| PiAcpFailure::new("validation_failed", "stage decision missing stage id"))?;
    if !thread.stages.iter().any(|stage| stage.id == stage_id) {
        return Err(PiAcpFailure::new(
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
                PiAcpFailure::new("validation_failed", "stage status must be a string")
            })?;
        if StageStatus::from_db_str(status).is_none() {
            return Err(PiAcpFailure::new(
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
) -> Result<Value, PiAcpFailure> {
    let mut value = issue.unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .ok_or_else(|| PiAcpFailure::new("validation_failed", "issue must be an object"))?;
    let stage_id = object
        .get("threadStageId")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| result.thread_stage_id.clone())
        .or_else(|| task.target_stage_id.clone())
        .ok_or_else(|| PiAcpFailure::new("validation_failed", "issue decision missing stage id"))?;
    if !thread.stages.iter().any(|stage| stage.id == stage_id) {
        return Err(PiAcpFailure::new(
            "validation_failed",
            "issue decision references unknown stage",
        ));
    }
    object.insert("taskId".to_string(), Value::String(task.id.clone()));
    object.insert("threadStageId".to_string(), Value::String(stage_id));
    if !object.contains_key("title") {
        object.insert(
            "title".to_string(),
            Value::String(format!("Astra follow-up: {}", task.title)),
        );
    }
    if !object.contains_key("description") {
        object.insert(
            "description".to_string(),
            Value::String(fallback_summary.to_string()),
        );
    }
    if !object.contains_key("severity") {
        object.insert(
            "severity".to_string(),
            Value::String(IssueSeverity::High.as_str().to_string()),
        );
    } else {
        let severity = object
            .get("severity")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                PiAcpFailure::new("validation_failed", "issue severity must be a string")
            })?;
        if IssueSeverity::from_db_str(severity).is_none() {
            return Err(PiAcpFailure::new(
                "validation_failed",
                format!("unknown issue severity: {severity}"),
            ));
        }
    }
    Ok(value)
}

fn parse_json_object(response: &str) -> Result<Value, PiAcpFailure> {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return Err(PiAcpFailure::new(
            "empty_response",
            "Pi ACP returned an empty response",
        ));
    }
    let candidate = extract_json_candidate(trimmed).ok_or_else(|| {
        PiAcpFailure::new(
            "invalid_json",
            "Pi ACP response did not contain a JSON object",
        )
    })?;
    let value: Value = serde_json::from_str(candidate)
        .map_err(|error| PiAcpFailure::new("invalid_json", error.to_string()))?;
    if !value.is_object() {
        return Err(PiAcpFailure::new(
            "invalid_json",
            "Pi ACP response JSON must be an object",
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
        let mut stage = super::super::tests::test_stage("stage-1", StageStatus::InProgress);
        stage.assistants.push(crate::models::StageAssistantInfo {
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
        });
        super::super::tests::test_thread(vec![stage])
    }

    #[test]
    fn parses_fenced_pi_plan_and_sanitizes_task() {
        let plan = parse_pi_plan_response(
            r#"```json
            {"summary":"ok","tasks":[{"id":"bad id!","title":"Do it","targetStageId":"stage-1","targetAgent":"codex","prompt":"Work","expectedOutput":"Notes","risk":"medium"}]}
            ```"#,
            &run(),
            &thread(),
            0,
        )
        .unwrap();

        assert_eq!(plan.summary, "ok");
        assert_eq!(plan.tasks.len(), 1);
        assert!(plan.tasks[0].id.starts_with("task-"));
        assert_eq!(plan.tasks[0].risk, AstraTaskRisk::Medium);
    }

    #[test]
    fn invalid_pi_plan_task_fails_whole_plan() {
        let error = parse_pi_plan_response(
            r#"{"summary":"bad","tasks":[{"targetStageId":"missing","targetAgent":"codex","prompt":"Work"}]}"#,
            &run(),
            &thread(),
            0,
        )
        .unwrap_err();

        assert_eq!(error.code, "validation_failed");
        assert!(error.message.contains("unknown targetStageId"));
    }

    #[test]
    fn parses_thread_level_pi_plan_task() {
        let plan = parse_pi_plan_response(
            r#"{"summary":"thread","tasks":[{"title":"Thread task","targetAgent":"codex","prompt":"Work thread"}]}"#,
            &run(),
            &thread(),
            0,
        )
        .unwrap();

        assert_eq!(plan.tasks.len(), 1);
        assert_eq!(plan.tasks[0].target_stage_id, None);
        assert_eq!(plan.tasks[0].target_agent, Agent::Codex);
    }

    #[test]
    fn pi_plan_truncates_to_twenty_tasks() {
        let tasks = (0..25)
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
        let response = json!({ "summary": "many", "tasks": tasks }).to_string();
        let plan = parse_pi_plan_response(&response, &run(), &thread(), 0).unwrap();

        assert_eq!(plan.tasks.len(), 20);
    }

    #[test]
    fn rejects_plan_without_json_object() {
        assert!(parse_pi_plan_response("no json here", &run(), &thread(), 0).is_err());
    }

    #[test]
    fn policy_denied_flag_wins_over_transport_message() {
        let failure = classify_pi_acp_error(
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
        let config = AstraPiConfig {
            command: "astra --acp".to_string(),
            session_dir: session_dir.to_string_lossy().to_string(),
            agent_dir: agent_dir.to_string_lossy().to_string(),
            planner: AstraPiPurposeConfig { timeout_ms: 30_000 },
            decision: AstraPiPurposeConfig { timeout_ms: 30_000 },
        };
        let provider = AstraPiProviderConfig {
            provider: Some("custom-endpoint".to_string()),
            api: Some("openai-responses".to_string()),
            base_url: Some("https://example.test/v1".to_string()),
            api_key: Some("secret-key".to_string()),
            model: Some("gpt-test".to_string()),
            thinking_level: Some("high".to_string()),
        };

        prepare_pi_agent_config(&config, &provider).unwrap();

        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(agent_dir.join("settings.json")).unwrap())
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
        assert_eq!(models["providers"]["custom-endpoint"]["apiKey"], "secret-key");
        assert_eq!(
            models["providers"]["custom-endpoint"]["models"][0]["id"],
            "gpt-test"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn maps_update_stage_decision() {
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

        let decision = parse_pi_decision_response(
            r#"{"action":"update_stage","stage":{"status":"completed"},"reason":"done"}"#,
            &thread(),
            &result,
            &task,
        )
        .unwrap();

        match decision {
            AstraDecision::UpdateStage { args } => {
                assert_eq!(args["threadStageId"], "stage-1");
                assert_eq!(args["status"], "completed");
            }
            other => panic!("unexpected decision: {other:?}"),
        }
    }
}
