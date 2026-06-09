use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use super::astra_pi_acp_adapter::parse_astra_pi_acp_orchestration_response;
use super::backend::{BackendFailure, BackendResponse, OrchestratorBackend};
use super::prompt::build_astra_orchestration_prompt;
use super::{
    AstraOrchestration, AstraRun, AstraTaskCompletion, ASTRA_ORCHESTRATOR_TIMEOUT_MS,
    ASTRA_RUNTIME_CLEANUP_TIMEOUT_MS, ASTRA_RUNTIME_STARTUP_TIMEOUT_MS,
};
use crate::agents::runtime::types::{
    AgentInput, AgentRuntimeEventPayload, RuntimeMetadata, StartAgentSession,
};
use crate::agents::runtime::RuntimeManager;
use crate::models::{Agent, ThreadInfo};

#[derive(Debug, Clone)]
pub struct RuntimeAgentBackendConfig {
    pub agent: Agent,
    pub timeout_ms: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
}

impl Default for RuntimeAgentBackendConfig {
    fn default() -> Self {
        Self {
            agent: Agent::Claude,
            timeout_ms: ASTRA_ORCHESTRATOR_TIMEOUT_MS,
            model: None,
            effort: None,
            permission_mode: None,
        }
    }
}

pub struct RuntimeAgentOrchestrator {
    runtime: RuntimeManager,
    config: RuntimeAgentBackendConfig,
}

impl RuntimeAgentOrchestrator {
    pub fn new(runtime: RuntimeManager, config: RuntimeAgentBackendConfig) -> Self {
        Self { runtime, config }
    }
}

impl OrchestratorBackend for RuntimeAgentOrchestrator {
    fn orchestrate(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        completions: &[AstraTaskCompletion],
        _backend_config: &Value,
    ) -> Result<BackendResponse<AstraOrchestration>, BackendFailure> {
        let prompt =
            build_astra_orchestration_prompt(run, thread, user_prompt, round_index, completions);

        match execute_agent_session(
            &self.runtime,
            &self.config,
            run,
            thread,
            &run.project_path,
            &prompt,
            "orchestration",
        ) {
            Ok((text, session_id)) => {
                match parse_astra_pi_acp_orchestration_response(
                    &text,
                    run,
                    thread,
                    round_index,
                    completions,
                ) {
                    Ok(orchestration) => Ok(BackendResponse {
                        data: orchestration,
                        session_id,
                        backend_type: format!("runtime_agent_{}", self.config.agent.as_str()),
                    }),
                    Err(pi_error) => Err(BackendFailure::new(
                        format!("runtime_agent_{}", self.config.agent.as_str()),
                        pi_error.code,
                        pi_error.message,
                    )
                    .with_session_id(Some(session_id))
                    .with_raw_response(&text)),
                }
            }
            Err(failure) => Err(failure),
        }
    }
}

fn execute_agent_session(
    runtime: &RuntimeManager,
    config: &RuntimeAgentBackendConfig,
    run: &AstraRun,
    thread: &ThreadInfo,
    workspace_path: &str,
    prompt: &str,
    purpose: &str,
) -> Result<(String, String), BackendFailure> {
    let mut options = RuntimeMetadata::default();
    options.insert("astraInternal".to_string(), Value::Bool(true));
    options.insert("astraRunId".to_string(), Value::String(run.run_id.clone()));
    options.insert(
        "astraThreadId".to_string(),
        Value::String(thread.id.clone()),
    );
    options.insert(
        "astraThreadKind".to_string(),
        Value::String(thread.kind.as_str().to_string()),
    );
    options.insert(
        "astraPurpose".to_string(),
        Value::String(purpose.to_string()),
    );

    if let Some(model) = &config.model {
        options.insert("model".to_string(), Value::String(model.clone()));
    }
    if let Some(effort) = &config.effort {
        options.insert("effort".to_string(), Value::String(effort.clone()));
    }
    if let Some(permission_mode) = &config.permission_mode {
        options.insert(
            "permissionMode".to_string(),
            Value::String(permission_mode.clone()),
        );
    }

    let req = StartAgentSession {
        agent: config.agent,
        workspace_path: workspace_path.to_string(),
        initial_prompt: None,
        source_session_id: None,
        source_agent: None,
        options,
    };

    let handle = runtime.start_session(req).map_err(|error| {
        BackendFailure::new(
            format!("runtime_agent_{}", config.agent.as_str()),
            "transport_failure",
            error.to_string(),
        )
    })?;

    let session_id = handle.sessio_runtime_session_id.clone();

    let backend_type = format!("runtime_agent_{}", config.agent.as_str());
    let receiver = match runtime.subscribe_events() {
        Ok(receiver) => receiver,
        Err(error) => {
            cleanup_agent_session(runtime, &backend_type, &session_id);
            return Err(BackendFailure::new(
                backend_type,
                "transport_failure",
                error.to_string(),
            ));
        }
    };
    if let Err(error) = runtime.wait_for_session_startup(
        &session_id,
        Duration::from_millis(ASTRA_RUNTIME_STARTUP_TIMEOUT_MS),
    ) {
        cleanup_agent_session(runtime, &backend_type, &session_id);
        return Err(BackendFailure::new(
            backend_type,
            "startup_timeout",
            error.to_string(),
        ));
    }

    // Send prompt
    if let Err(error) = runtime.send_input(
        &session_id,
        AgentInput {
            text: prompt.to_string(),
            attachments: Vec::new(),
            options: RuntimeMetadata::default(),
        },
    ) {
        cleanup_agent_session(runtime, &backend_type, &session_id);
        return Err(BackendFailure::new(
            backend_type,
            "transport_failure",
            error.to_string(),
        ));
    }

    let output = wait_for_agent_output(receiver, &session_id, config.timeout_ms, &backend_type);
    let agent_session_id = runtime
        .agent_runtime_session_id_for_session(&session_id)
        .filter(|value| is_persistable_runtime_agent_session_id(value))
        .unwrap_or_else(|| session_id.clone());

    cleanup_agent_session(runtime, &backend_type, &session_id);

    let output = output?.trim().to_string();

    if output.is_empty() {
        return Err(BackendFailure::new(
            format!("runtime_agent_{}", config.agent.as_str()),
            "empty_response",
            "Agent returned empty response",
        )
        .with_session_id(Some(agent_session_id)));
    }

    Ok((output, agent_session_id))
}

fn cleanup_agent_session(runtime: &RuntimeManager, backend_type: &str, session_id: &str) {
    let report = runtime.cleanup_session_bounded(
        session_id,
        Duration::from_millis(ASTRA_RUNTIME_CLEANUP_TIMEOUT_MS),
    );
    if report.cancel_error.is_some() || report.dispose_error.is_some() || report.timed_out {
        log::warn!(
            "[astra:runtime-agent:cleanup] backend={} sessionId={} cancelError={:?} disposeError={:?} timedOut={}",
            backend_type,
            session_id,
            report.cancel_error,
            report.dispose_error,
            report.timed_out
        );
    }
}

fn is_persistable_runtime_agent_session_id(session_id: &str) -> bool {
    let session_id = session_id.trim();
    !session_id.is_empty()
        && !session_id.starts_with("runtime-")
        && !session_id.starts_with("fake-agent-session")
}

fn wait_for_agent_output(
    receiver: std::sync::mpsc::Receiver<crate::agents::runtime::types::AgentRuntimeEvent>,
    session_id: &str,
    timeout_ms: u64,
    backend_type: &str,
) -> Result<String, BackendFailure> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut output = String::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(BackendFailure::new(
                backend_type.to_string(),
                "timeout",
                format!("Agent session timed out after {timeout_ms}ms"),
            )
            .with_session_id(Some(session_id.to_string())));
        }

        let remaining = deadline.saturating_duration_since(now);
        let wait = remaining.min(Duration::from_millis(250));
        let event = match receiver.recv_timeout(wait) {
            Ok(event) => event,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(BackendFailure::new(
                    backend_type.to_string(),
                    "transport_failure",
                    "runtime event stream disconnected",
                )
                .with_session_id(Some(session_id.to_string())));
            }
        };

        match event.payload {
            AgentRuntimeEventPayload::TextDelta {
                sessio_runtime_session_id,
                text,
                ..
            } if sessio_runtime_session_id == session_id => {
                output.push_str(&text);
            }
            AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id,
                ..
            } if sessio_runtime_session_id == session_id => return Ok(output),
            AgentRuntimeEventPayload::TurnError {
                sessio_runtime_session_id,
                error,
                ..
            } if sessio_runtime_session_id == session_id => {
                return Err(BackendFailure::new(
                    backend_type.to_string(),
                    "turn_error",
                    format!("{}: {}", error.code, error.message),
                )
                .with_session_id(Some(session_id.to_string())));
            }
            AgentRuntimeEventPayload::TurnCancelled {
                sessio_runtime_session_id,
                ..
            } if sessio_runtime_session_id == session_id => {
                return Err(BackendFailure::new(
                    backend_type.to_string(),
                    "cancelled",
                    "Agent session turn was cancelled",
                )
                .with_session_id(Some(session_id.to_string())));
            }
            AgentRuntimeEventPayload::SessionEnded {
                sessio_runtime_session_id,
            } if sessio_runtime_session_id == session_id => return Ok(output),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::agents::runtime::types::{AgentRuntimeEvent, AgentRuntimeEventPayload};

    fn event(payload: AgentRuntimeEventPayload) -> AgentRuntimeEvent {
        AgentRuntimeEvent {
            sequence: 1,
            timestamp: 1,
            payload,
        }
    }

    #[test]
    fn wait_for_agent_output_filters_interleaved_sessions() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(event(AgentRuntimeEventPayload::TextDelta {
                sessio_runtime_session_id: "other-session".to_string(),
                turn_id: "turn-other".to_string(),
                text: "wrong".to_string(),
            }))
            .unwrap();
        sender
            .send(event(AgentRuntimeEventPayload::TextDelta {
                sessio_runtime_session_id: "target-session".to_string(),
                turn_id: "turn-target".to_string(),
                text: "{\"summary\":\"ok\"".to_string(),
            }))
            .unwrap();
        sender
            .send(event(AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: "other-session".to_string(),
                turn_id: "turn-other".to_string(),
                result: None,
            }))
            .unwrap();
        sender
            .send(event(AgentRuntimeEventPayload::TextDelta {
                sessio_runtime_session_id: "target-session".to_string(),
                turn_id: "turn-target".to_string(),
                text: "}".to_string(),
            }))
            .unwrap();
        sender
            .send(event(AgentRuntimeEventPayload::TurnCompleted {
                sessio_runtime_session_id: "target-session".to_string(),
                turn_id: "turn-target".to_string(),
                result: None,
            }))
            .unwrap();

        let output =
            wait_for_agent_output(receiver, "target-session", 1_000, "runtime_agent_codex")
                .unwrap();

        assert_eq!(output, r#"{"summary":"ok"}"#);
    }
}
