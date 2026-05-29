use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agents::runtime::types::{RuntimeCapabilitySet, RuntimeTransportKind};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Codex,
    Claude,
    Gemini,
}

impl Agent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Agent::Codex => "codex",
            Agent::Claude => "claude",
            Agent::Gemini => "gemini",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "codex" => Some(Agent::Codex),
            "claude" => Some(Agent::Claude),
            "gemini" => Some(Agent::Gemini),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub id: String,
    pub agent: Agent,
    pub forked_from_agent: Option<Agent>,
    pub forked_from_id: Option<String>,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub message_count: usize,
    pub title: Option<String>,
    pub first_user_message: Option<String>,
    pub file_path: String,
    pub file_size: u64,
    pub partial: bool,
    pub available: bool,
    pub archived: bool,
    #[serde(default)]
    pub subagents: Vec<SubagentInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfo {
    pub id: String,
    pub agent_type: Option<String>,
    pub description: Option<String>,
    pub started_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub message_count: usize,
    pub first_user_message: Option<String>,
    pub file_path: String,
    pub file_size: u64,
    pub partial: bool,
    #[serde(default = "default_available")]
    pub available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProjectType {
    Code,
    Writing,
    Research,
    General,
    VideoProduction,
}

impl ProjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectType::Code => "code",
            ProjectType::Writing => "writing",
            ProjectType::Research => "research",
            ProjectType::General => "general",
            ProjectType::VideoProduction => "video_production",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "code" => Some(ProjectType::Code),
            "writing" => Some(ProjectType::Writing),
            "research" => Some(ProjectType::Research),
            "general" => Some(ProjectType::General),
            "video_production" => Some(ProjectType::VideoProduction),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub project_type: ProjectType,
    pub created_at: i64,
    pub updated_at: i64,
    pub session_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum KanbanStatus {
    Todo,
    InProgress,
    Canceled,
    AgentReview,
    HumanReview,
    Done,
}

impl KanbanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KanbanStatus::Todo => "todo",
            KanbanStatus::InProgress => "in_progress",
            KanbanStatus::Canceled => "canceled",
            KanbanStatus::AgentReview => "agent_review",
            KanbanStatus::HumanReview => "human_review",
            KanbanStatus::Done => "done",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(KanbanStatus::Todo),
            "in_progress" => Some(KanbanStatus::InProgress),
            "canceled" => Some(KanbanStatus::Canceled),
            "agent_review" => Some(KanbanStatus::AgentReview),
            "human_review" => Some(KanbanStatus::HumanReview),
            "done" => Some(KanbanStatus::Done),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KanbanItem {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: KanbanStatus,
    pub sort_order: i64,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

fn default_available() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

impl SessionContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".to_string(),
            text: Some(text.into()),
            uri: None,
            data: None,
            mime_type: None,
            name: None,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: None,
            meta: None,
        }
    }

    pub fn image(uri: impl Into<String>, mime_type: Option<String>) -> Self {
        Self {
            kind: "image".to_string(),
            text: None,
            uri: Some(uri.into()),
            data: None,
            mime_type,
            name: None,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: None,
            meta: None,
        }
    }

    pub fn resource(uri: Option<String>, name: Option<String>, mime_type: Option<String>) -> Self {
        Self {
            kind: "resource".to_string(),
            text: None,
            uri,
            data: None,
            mime_type,
            name,
            title: None,
            description: None,
            size: None,
            blob: None,
            resource: None,
            annotations: None,
            meta: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub role: String,
    pub text: String,
    pub timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<SessionContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryTurn {
    pub turn_id: String,
    pub status: String,
    pub blocks: Vec<SessionHistoryBlock>,
    pub tools: Vec<SessionHistoryToolCall>,
    pub permissions: Vec<SessionHistoryPermissionRequest>,
    pub protocol_messages: Vec<Value>,
    pub stop_reason: Option<String>,
    pub error: Option<Value>,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryBlock {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<SessionContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryToolCall {
    pub tool_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default)]
    pub locations: Vec<Value>,
    pub raw_input: Value,
    pub raw_output: Value,
    pub meta: Value,
    pub raw: Value,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryPermissionRequest {
    pub request_id: String,
    pub tool_call: Value,
    pub tool_name: String,
    pub input: Value,
    pub options: Vec<SessionHistoryPermissionOption>,
    pub selected_option_id: Option<String>,
    pub cancelled: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryPermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: String,
    pub meta: Value,
}

impl SessionMessage {
    pub fn new(role: impl Into<String>, text: impl Into<String>, timestamp: Option<i64>) -> Self {
        let text = text.into();
        Self {
            role: role.into(),
            content_blocks: text_content_blocks(&text),
            text,
            timestamp,
            tool_call_id: None,
        }
    }

    pub fn with_tool_call_id(mut self, tool_call_id: Option<String>) -> Self {
        self.tool_call_id = tool_call_id;
        self
    }

    pub fn with_content_blocks(mut self, content_blocks: Vec<SessionContentBlock>) -> Self {
        self.content_blocks = content_blocks;
        self
    }
}

pub fn text_content_blocks(text: &str) -> Vec<SessionContentBlock> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    while cursor < text.len() {
        let next_image = text[cursor..].find("![").map(|idx| cursor + idx);
        let next_file = find_file_marker(text, cursor);
        let next = match (next_image, next_file) {
            (Some(image), Some(file)) => Some(image.min(file)),
            (Some(image), None) => Some(image),
            (None, Some(file)) => Some(file),
            (None, None) => None,
        };
        let Some(start) = next else {
            push_text_block(&mut blocks, &text[cursor..]);
            break;
        };
        push_text_block(&mut blocks, &text[cursor..start]);
        if text[start..].starts_with("![") {
            if let Some((block, end)) = parse_markdown_image(text, start) {
                blocks.push(block);
                cursor = end;
                continue;
            }
        } else if let Some((block, end)) = parse_file_marker(text, start) {
            blocks.push(block);
            cursor = end;
            continue;
        }
        push_text_block(&mut blocks, &text[start..start + 1]);
        cursor = start + 1;
    }
    if blocks.is_empty() {
        vec![SessionContentBlock::text(text.to_string())]
    } else {
        blocks
    }
}

fn push_text_block(blocks: &mut Vec<SessionContentBlock>, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    blocks.push(SessionContentBlock::text(text.trim().to_string()));
}

fn find_file_marker(text: &str, start: usize) -> Option<usize> {
    let haystack = text.get(start..)?.to_ascii_lowercase();
    haystack.find("[file:").map(|idx| start + idx)
}

fn parse_markdown_image(text: &str, start: usize) -> Option<(SessionContentBlock, usize)> {
    let after_open = start + 2;
    let label_end = text[after_open..].find("](").map(|idx| after_open + idx)?;
    let target_start = label_end + 2;
    let target_end = text[target_start..]
        .find(')')
        .map(|idx| target_start + idx)?;
    let alt = text[after_open..label_end].trim();
    let uri = text[target_start..target_end]
        .trim()
        .trim_matches(['<', '>']);
    if uri.is_empty() {
        return None;
    }
    let mime_type = if alt.contains('/') {
        Some(alt.to_string())
    } else {
        None
    };
    Some((
        SessionContentBlock::image(uri.to_string(), mime_type),
        target_end + 1,
    ))
}

fn parse_file_marker(text: &str, start: usize) -> Option<(SessionContentBlock, usize)> {
    let marker = text.get(start..)?;
    if !marker
        .get(..6)
        .map(|value| value.eq_ignore_ascii_case("[file:"))
        .unwrap_or(false)
    {
        return None;
    }
    let close = marker.find(']')?;
    let body = marker[6..close].trim();
    if body.is_empty() {
        return None;
    }
    let mut parts = body.splitn(2, '|');
    let name = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let uri = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Some((
        SessionContentBlock::resource(
            uri.map(ToOwned::to_owned),
            name.map(ToOwned::to_owned),
            None,
        ),
        start + close + 1,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAgentMetadata {
    pub agent: Agent,
    pub enabled: bool,
    pub configured: bool,
    pub transport: RuntimeTransportKind,
    pub model: Option<String>,
    pub models: Vec<RuntimeAgentOptionMetadata>,
    pub effort: Option<String>,
    pub efforts: Vec<RuntimeAgentOptionMetadata>,
    pub permission_mode: Option<String>,
    pub permission_modes: Vec<RuntimeAgentOptionMetadata>,
    pub session_command: Option<String>,
    pub version_command: Option<String>,
    pub detected_version: Option<String>,
    pub capabilities: Option<RuntimeCapabilitySet>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAgentOptionMetadata {
    pub value: String,
    pub label: String,
}

pub fn normalize_preview(s: &str) -> String {
    const MAX_PREVIEW_CHARS: usize = 50;
    let trimmed = s.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch == '\n' || ch == '\r' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    let mut chars = out.chars();
    let truncated: String = chars.by_ref().take(MAX_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub fn is_system_noise(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<INSTRUCTIONS>")
        || t.starts_with("# AGENTS.md")
        || t.starts_with("<system-reminder>")
        || t.starts_with("<command-name>")
        || t.starts_with("<command-message>")
        || t.starts_with("<command-args>")
        || t.starts_with("<local-command-stdout>")
        || t.starts_with("<local-command-caveat>")
        || t.starts_with("<bash-input>")
        || t.starts_with("<bash-stdout>")
        || t.starts_with("<bash-stderr>")
        || t.starts_with("<user-memory-input>")
        || t.starts_with("<turn_aborted>")
        || t.starts_with("Caveat:")
        || t.starts_with("Warning: apply_patch was requested via exec_command")
}

// Strip IDE-injected context blocks that some agents prepend to the real user
// message. Returns the underlying request text, or empty if the message is
// entirely context.
pub fn strip_injected_context(s: &str) -> String {
    let mut text: &str = s;

    // Claude-style: leading <ide_*>...</ide_*> wrapper blocks (e.g.
    // <ide_opened_file>, <ide_selection>).
    loop {
        let trimmed = text.trim_start();
        let after_lt = match trimmed.strip_prefix("<ide_") {
            Some(rest) => rest,
            None => break,
        };
        let close_idx = match after_lt.find('>') {
            Some(i) => i,
            None => break,
        };
        let tag = &after_lt[..close_idx];
        let close = format!("</ide_{}>", tag);
        let after_open = &after_lt[close_idx + 1..];
        match after_open.find(close.as_str()) {
            Some(i) => {
                text = &after_open[i + close.len()..];
            }
            None => break,
        }
    }

    // Codex-style: strip any preamble before the "## My request for Codex:"
    // header (e.g. "# Context from my IDE setup:", "# Files mentioned by the
    // user:"); the real user input follows the marker.
    const MARKER: &str = "## My request for Codex:";
    if let Some(i) = text.find(MARKER) {
        text = &text[i + MARKER.len()..];
    }

    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{normalize_preview, text_content_blocks};

    #[test]
    fn normalize_preview_limits_to_50_chars_plus_ellipsis() {
        let exact = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十";
        assert_eq!(exact.chars().count(), 50);
        assert_eq!(normalize_preview(exact), exact);

        let long = format!("{exact}超出");
        let preview = normalize_preview(&long);
        assert_eq!(preview, format!("{exact}..."));
        assert_eq!(preview.chars().count(), 53);
    }

    #[test]
    fn normalize_preview_flattens_newlines_before_truncating() {
        assert_eq!(
            normalize_preview(" hello\nworld\ragain "),
            "hello world again"
        );
    }

    #[test]
    fn text_content_blocks_parse_markdown_images_and_file_markers() {
        let blocks = text_content_blocks(
            "review\n[file: spec.md|file:///tmp/spec.md]\n![image/png](file:///tmp/screen.png)",
        );
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].kind, "text");
        assert_eq!(blocks[0].text.as_deref(), Some("review"));
        assert_eq!(blocks[1].kind, "resource");
        assert_eq!(blocks[1].name.as_deref(), Some("spec.md"));
        assert_eq!(blocks[1].uri.as_deref(), Some("file:///tmp/spec.md"));
        assert_eq!(blocks[2].kind, "image");
        assert_eq!(blocks[2].mime_type.as_deref(), Some("image/png"));
        assert_eq!(blocks[2].uri.as_deref(), Some("file:///tmp/screen.png"));
    }
}
