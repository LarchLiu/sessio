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
    pub supports_load_session: bool,
    pub supports_resume: bool,
    pub supports_fork: bool,
    pub supports_image_attachments: bool,
    pub supports_audio_attachments: bool,
    pub supports_embedded_context: bool,
    pub supports_attachments: bool,
    pub supports_modes: bool,
}

impl RuntimeCapabilitySet {
    pub fn fake() -> Self {
        Self {
            supports_cancel: true,
            supports_permissions: true,
            supports_tool_deltas: true,
            supports_load_session: true,
            supports_resume: true,
            supports_fork: false,
            supports_image_attachments: false,
            supports_audio_attachments: false,
            supports_embedded_context: false,
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
    pub source_agent: Option<Agent>,
    #[serde(default)]
    pub options: RuntimeMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnsureAgentRuntimeSession {
    pub agent: Agent,
    pub sessio_runtime_session_id: String,
    pub workspace_path: String,
    #[serde(default)]
    pub agent_runtime_session_id: Option<String>,
    #[serde(default)]
    pub source_agent: Option<Agent>,
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
pub enum AgentAttachmentKind {
    Image,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentAttachment {
    pub path: String,
    pub mime_type: Option<String>,
    pub kind: AgentAttachmentKind,
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
pub struct AgentSessionConfigChange {
    pub config_id: String,
    pub value: Value,
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
#[serde(rename_all = "camelCase")]
pub struct AcpProtocolMessage {
    pub direction: String,
    pub message_kind: String,
    pub method: String,
    pub protocol_version: Option<String>,
    pub acp_session_id: Option<String>,
    pub turn_id: Option<String>,
    pub request_id: Option<String>,
    pub update_type: Option<String>,
    pub data: Value,
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
        data: Value,
    },
    ToolInputDelta {
        sessio_runtime_session_id: String,
        turn_id: String,
        tool_id: String,
        delta: String,
        data: Option<Value>,
    },
    ToolOutputDelta {
        sessio_runtime_session_id: String,
        turn_id: String,
        tool_id: String,
        delta: String,
        data: Option<Value>,
    },
    ToolStatusChanged {
        sessio_runtime_session_id: String,
        turn_id: String,
        tool_id: String,
        status: String,
        data: Option<Value>,
    },
    SessionUpdate {
        sessio_runtime_session_id: String,
        turn_id: String,
        update_type: String,
        data: Value,
    },
    AcpProtocolMessage {
        sessio_runtime_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<String>,
        message: AcpProtocolMessage,
    },
    PermissionRequested {
        sessio_runtime_session_id: String,
        turn_id: String,
        request_id: String,
        tool_name: String,
        input: Option<Value>,
        data: Value,
    },
    PermissionResolved {
        sessio_runtime_session_id: String,
        turn_id: String,
        request_id: String,
        approved: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        option_id: Option<String>,
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
