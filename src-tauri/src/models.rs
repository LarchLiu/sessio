use serde::{Deserialize, Serialize};

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

pub fn normalize_preview(s: &str) -> String {
    let trimmed = s.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch == '\n' || ch == '\r' {
            out.push(' ');
        } else {
            out.push(ch);
        }
    }
    out
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
