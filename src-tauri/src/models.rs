use serde::{Deserialize, Serialize};

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
pub struct SessionMessage {
    pub role: String,
    pub text: String,
    pub timestamp: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
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
    use super::normalize_preview;

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
}
