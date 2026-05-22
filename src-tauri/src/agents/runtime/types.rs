use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::Agent;

pub type RuntimeMetadata = BTreeMap<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeTransportKind {
    Acp,
    CliStreamJson,
    PlainCli,
    Fake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeSessionStatus {
    Starting,
    Active,
    Idle,
    Cancelling,
    Completed,
    Errored,
    Disconnected,
    Ended,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilitySet {
    pub supports_cancel: bool,
    pub supports_permissions: bool,
    pub supports_tool_deltas: bool,
    pub supports_resume: bool,
    pub supports_attachments: bool,
    pub supports_modes: bool,
}

impl RuntimeCapabilitySet {
    pub fn fake() -> Self {
        Self {
            supports_cancel: true,
            supports_permissions: true,
            supports_tool_deltas: true,
            supports_resume: true,
            supports_attachments: false,
            supports_modes: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub agent: Agent,
    pub transport: RuntimeTransportKind,
    pub available: bool,
    pub status: RuntimeSessionStatus,
    pub capabilities: RuntimeCapabilitySet,
    pub error: Option<String>,
    #[serde(default)]
    pub metadata: RuntimeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAgentSession {
    pub agent: Agent,
    pub workspace_path: String,
    pub initial_prompt: Option<String>,
    pub source_session_id: Option<String>,
    #[serde(default)]
    pub options: RuntimeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionHandle {
    pub sessio_runtime_session_id: String,
    pub agent: Agent,
    pub transport: RuntimeTransportKind,
    pub agent_runtime_session_id: String,
    pub workspace_path: String,
    pub status: RuntimeSessionStatus,
    pub capabilities: RuntimeCapabilitySet,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAttachment {
    pub path: String,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInput {
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<AgentAttachment>,
    #[serde(default)]
    pub options: RuntimeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurnHandle {
    pub sessio_runtime_session_id: String,
    pub turn_id: String,
    pub status: RuntimeTurnStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeTurnStatus {
    Pending,
    Streaming,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeError {
    pub code: String,
    pub message: String,
    pub data: Option<Value>,
}

impl RuntimeError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum AgentRuntimeEventPayload {
    SessionStarted {
        agent: Agent,
        sessio_runtime_session_id: String,
        agent_runtime_session_id: String,
        transport: RuntimeTransportKind,
        workspace_path: String,
        capabilities: RuntimeCapabilitySet,
    },
    TurnStarted {
        sessio_runtime_session_id: String,
        turn_id: String,
    },
    TextDelta {
        sessio_runtime_session_id: String,
        turn_id: String,
        text: String,
    },
    ReasoningDelta {
        sessio_runtime_session_id: String,
        turn_id: String,
        text: String,
    },
    ToolStarted {
        sessio_runtime_session_id: String,
        turn_id: String,
        tool_id: String,
        name: String,
        input: Option<Value>,
    },
    ToolInputDelta {
        sessio_runtime_session_id: String,
        turn_id: String,
        tool_id: String,
        delta: String,
    },
    ToolOutputDelta {
        sessio_runtime_session_id: String,
        turn_id: String,
        tool_id: String,
        delta: String,
    },
    PermissionRequested {
        sessio_runtime_session_id: String,
        turn_id: String,
        request_id: String,
        tool_name: String,
        input: Option<Value>,
    },
    PermissionResolved {
        sessio_runtime_session_id: String,
        turn_id: String,
        request_id: String,
        approved: bool,
    },
    TurnCompleted {
        sessio_runtime_session_id: String,
        turn_id: String,
        result: Option<Value>,
    },
    TurnError {
        sessio_runtime_session_id: String,
        turn_id: String,
        error: RuntimeError,
    },
    TurnCancelled {
        sessio_runtime_session_id: String,
        turn_id: String,
    },
    SessionEnded {
        sessio_runtime_session_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntimeEvent {
    pub sequence: u64,
    pub timestamp: i64,
    #[serde(flatten)]
    pub payload: AgentRuntimeEventPayload,
}
