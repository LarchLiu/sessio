use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Metadata = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentKind(pub String);

impl AgentKind {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRef {
    pub project_key: String,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    MainSession,
    Subagent,
    ProjectIndex,
    Logs,
    Archive,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSource {
    pub agent: AgentKind,
    pub session_id: String,
    pub scope: String,
    pub file_path: String,
    pub project: Option<ProjectRef>,
    pub source_kind: SourceKind,
    #[serde(default)]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    pub source: SessionSource,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub message_count: usize,
    pub title: Option<String>,
    pub first_user_message: Option<String>,
    pub file_size: u64,
    pub file_mtime: Option<i64>,
    pub partial: bool,
    pub available: bool,
    pub archived: bool,
    #[serde(default)]
    pub children: Vec<SessionRecord>,
    #[serde(default)]
    pub metadata: Metadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageRole {
    User,
    Assistant,
    Thinking,
    System,
    ToolUse,
    ToolResult,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseEvent {
    pub name: String,
    pub input: Option<Value>,
    pub raw: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultEvent {
    pub tool_name: Option<String>,
    pub exit_code: Option<i32>,
    pub success: Option<bool>,
    pub text: String,
    pub output_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageContent {
    Text { text: String },
    ToolUse { tool: ToolUseEvent },
    ToolResult { result: ToolResultEvent },
    Mixed { parts: Vec<MessageContent> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    pub file_path: String,
    pub line_start: Option<u64>,
    pub line_end: Option<u64>,
    pub byte_start: Option<u64>,
    pub byte_end: Option<u64>,
}

impl SourceLocation {
    pub fn file(file_path: impl Into<String>) -> Self {
        Self {
            file_path: file_path.into(),
            line_start: None,
            line_end: None,
            byte_start: None,
            byte_end: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEvent {
    pub source: SessionSource,
    pub event_id: Option<String>,
    pub turn_index: usize,
    pub role: MessageRole,
    pub content: MessageContent,
    pub timestamp: Option<i64>,
    pub location: SourceLocation,
    #[serde(default)]
    pub metadata: Metadata,
}

#[derive(Debug, Clone)]
pub struct WatchRoot {
    pub agent: AgentKind,
    pub path: PathBuf,
    pub recursive: bool,
    pub purpose: WatchPurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchPurpose {
    Sessions,
    Projects,
    Logs,
    ProjectMappings,
}

#[derive(Debug, Clone)]
pub struct PathEvent {
    pub path: PathBuf,
    pub kind: PathEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathEventKind {
    Create,
    Modify,
    Remove,
    Unknown,
}

#[derive(Debug, Clone)]
pub enum SourceIndexTask {
    ReindexSource(SessionSource),
    ReindexScope { agent: AgentKind, scope: String },
    MarkSourceUnavailable(SessionSource),
    RefreshProjectMappings { agent: AgentKind },
}
