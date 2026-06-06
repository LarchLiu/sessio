use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use serde_json::{json, Value};

use super::backend::{BackendFailure, BackendResponse, DecisionBackend, PlannerBackend};
use super::pi_acp_adapter::{parse_pi_decision_response, parse_pi_plan_response};
use super::{AstraDecision, AstraPlan, AstraRun, AstraTaskProposal, AstraTaskResult};
use crate::agents::runtime::types::{
    AgentInput, AgentRuntimeEventPayload, RuntimeMetadata, StartAgentSession,
};
use crate::agents::runtime::RuntimeManager;
use crate::models::{Agent, ThreadInfo};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct RuntimeAgentBackendConfig {
    pub agent: Agent,
    pub timeout_ms: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Default for RuntimeAgentBackendConfig {
    fn default() -> Self {
        Self {
            agent: Agent::Claude,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            model: None,
            effort: None,
        }
    }
}

pub struct RuntimeAgentPlanner {
    runtime: RuntimeManager,
    config: RuntimeAgentBackendConfig,
}

impl RuntimeAgentPlanner {
    pub fn new(runtime: RuntimeManager, config: RuntimeAgentBackendConfig) -> Self {
        Self { runtime, config }
    }
}

impl PlannerBackend for RuntimeAgentPlanner {
    fn plan(
        &self,
        run: &AstraRun,
        thread: &ThreadInfo,
        user_prompt: Option<&str>,
        round_index: u32,
        _backend_config: &Value,
    ) -> Result<BackendResponse<AstraPlan>, BackendFailure> {
        let prompt = build_planning_prompt(run, thread, user_prompt, round_index);

        match execute_agent_session(
            &self.runtime,
            &self.config,
            &run.project_path,
            &prompt,
            "planning",
        ) {
            Ok((text, session_id)) => {
                match parse_pi_plan_response(&text, run, thread, round_index) {
                    Ok(plan) => Ok(BackendResponse {
                        data: plan,
                        session_id,
                        backend_type: format!("runtime_agent_{}", self.config.agent.as_str()),
                    }),
                    Err(pi_error) => Err(BackendFailure::new(
                        format!("runtime_agent_{}", self.config.agent.as_str()),
                        pi_error.code,
                        pi_error.message,
                    )
                    .with_session_id(Some(session_id))),
                }
            }
            Err(failure) => Err(failure),
        }
    }

    fn backend_type(&self) -> &'static str {
        "runtime_agent"
    }

    fn supports_fallback(&self) -> bool {
        true
    }
}

pub struct RuntimeAgentDecisionEngine {
    runtime: RuntimeManager,
    config: RuntimeAgentBackendConfig,
}

impl RuntimeAgentDecisionEngine {
    pub fn new(runtime: RuntimeManager, config: RuntimeAgentBackendConfig) -> Self {
        Self { runtime, config }
    }
}

impl DecisionBackend for RuntimeAgentDecisionEngine {
    fn decide(
        &self,
        _run: &AstraRun,
        thread: &ThreadInfo,
        result: &AstraTaskResult,
        task: &AstraTaskProposal,
        _backend_config: &Value,
    ) -> Result<BackendResponse<AstraDecision>, BackendFailure> {
        let prompt = build_decision_prompt(thread, result, task);

        match execute_agent_session(&self.runtime, &self.config, "", &prompt, "decision") {
            Ok((text, session_id)) => {
                match parse_pi_decision_response(&text, thread, result, task) {
                    Ok(decision) => Ok(BackendResponse {
                        data: decision,
                        session_id,
                        backend_type: format!("runtime_agent_{}", self.config.agent.as_str()),
                    }),
                    Err(pi_error) => Err(BackendFailure::new(
                        format!("runtime_agent_{}", self.config.agent.as_str()),
                        pi_error.code,
                        pi_error.message,
                    )
                    .with_session_id(Some(session_id))),
                }
            }
            Err(failure) => Err(failure),
        }
    }

    fn backend_type(&self) -> &'static str {
        "runtime_agent"
    }

    fn supports_fallback(&self) -> bool {
        true
    }
}

fn execute_agent_session(
    runtime: &RuntimeManager,
    config: &RuntimeAgentBackendConfig,
    workspace_path: &str,
    prompt: &str,
    purpose: &str,
) -> Result<(String, String), BackendFailure> {
    let mut options = RuntimeMetadata::default();
    options.insert("astraInternal".to_string(), Value::Bool(true));
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
    let text = Arc::new(Mutex::new(String::new()));
    let text_for_events = text.clone();

    // Subscribe to events to collect text
    let receiver = runtime.subscribe_events().map_err(|error| {
        BackendFailure::new(
            format!("runtime_agent_{}", config.agent.as_str()),
            "transport_failure",
            error.to_string(),
        )
    })?;

    let session_id_for_filter = session_id.clone();
    std::thread::spawn(move || {
        for event in receiver {
            if let AgentRuntimeEventPayload::TextDelta {
                sessio_runtime_session_id,
                text: delta,
                ..
            } = event.payload
            {
                if sessio_runtime_session_id == session_id_for_filter {
                    if let Ok(mut buffer) = text_for_events.lock() {
                        buffer.push_str(&delta);
                    }
                }
            }
        }
    });

    // Send prompt
    runtime
        .send_input(
            &session_id,
            AgentInput {
                text: prompt.to_string(),
                attachments: Vec::new(),
                options: RuntimeMetadata::default(),
            },
        )
        .map_err(|error| {
            BackendFailure::new(
                format!("runtime_agent_{}", config.agent.as_str()),
                "transport_failure",
                error.to_string(),
            )
        })?;

    // Wait for completion with timeout
    std::thread::sleep(Duration::from_millis(config.timeout_ms));

    let output = text.lock().map(|buffer| buffer.clone()).unwrap_or_default();

    // Clean up session
    let _ = runtime.dispose_session_silent(&session_id);

    if output.is_empty() {
        return Err(BackendFailure::new(
            format!("runtime_agent_{}", config.agent.as_str()),
            "empty_response",
            "Agent returned empty response",
        )
        .with_session_id(Some(session_id)));
    }

    Ok((output, session_id))
}

fn build_planning_prompt(
    run: &AstraRun,
    thread: &ThreadInfo,
    user_prompt: Option<&str>,
    round_index: u32,
) -> String {
    let stages = thread
        .stages
        .iter()
        .map(|stage| {
            let agent = super::pick_stage_agent(stage).map(|agent| agent.as_str().to_string());
            json!({
                "id": stage.id,
                "title": super::stage_label(stage),
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

fn build_decision_prompt(
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
