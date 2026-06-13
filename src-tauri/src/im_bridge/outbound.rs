//! Outbound routing: aggregate runtime events and deliver chat replies.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::agents::runtime::types::{AgentRuntimeEvent, AgentRuntimeEventPayload};

use super::router;
use super::state::{ChatPermissionOption, ChatPermissionRequest, ImBridgeState};

/// How often we re-send the platform's "typing" indicator while a turn is in
/// flight. Telegram's `sendChatAction` expires at ~5s and Discord's typing at
/// ~10s; 4s keeps both alive without spamming.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);

/// Start the runtime event pump. It subscribes once, filters to events that can
/// produce chat output, and keeps all network sending outside the runtime emit
/// path.
pub fn spawn(state: Arc<ImBridgeState>) -> Result<()> {
    let receiver = state.runtime.subscribe_events_filtered(|payload| {
        matches!(
            payload,
            AgentRuntimeEventPayload::TextDelta { .. }
                | AgentRuntimeEventPayload::ReasoningDelta { .. }
                | AgentRuntimeEventPayload::ToolStarted { .. }
                | AgentRuntimeEventPayload::TurnStarted { .. }
                | AgentRuntimeEventPayload::PermissionRequested { .. }
                | AgentRuntimeEventPayload::PermissionResolved { .. }
                | AgentRuntimeEventPayload::TurnCompleted { .. }
                | AgentRuntimeEventPayload::TurnError { .. }
                | AgentRuntimeEventPayload::TurnCancelled { .. }
                | AgentRuntimeEventPayload::SessionEnded { .. }
        )
    })?;

    let typing = Arc::new(TypingTracker::default());
    spawn_typing_refresher(state.clone(), typing.clone());

    thread::Builder::new()
        .name("im-bridge-outbound".to_string())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                handle_event(&state, &typing, event);
            }
            log::warn!("[im-bridge:outbound] runtime event subscription closed");
        })?;

    Ok(())
}

/// Tracks which sessions currently have an in-flight turn so the typing
/// refresher knows where to send heartbeats. A session enters on
/// `TurnStarted` and leaves on any terminal turn event.
#[derive(Default)]
struct TypingTracker {
    active: Mutex<HashSet<String>>,
    last_sent: Mutex<HashMap<String, Instant>>,
}

impl TypingTracker {
    fn mark_active(&self, session_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.insert(session_id.to_string());
        }
    }

    fn mark_inactive(&self, session_id: &str) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(session_id);
        }
        if let Ok(mut last) = self.last_sent.lock() {
            last.remove(session_id);
        }
    }

    fn active_sessions(&self) -> Vec<String> {
        self.active
            .lock()
            .ok()
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Returns true if it's time to send a typing heartbeat for this session.
    fn should_send(&self, session_id: &str, now: Instant) -> bool {
        let mut last = match self.last_sent.lock() {
            Ok(last) => last,
            Err(_) => return false,
        };
        let due = match last.get(session_id) {
            Some(previous) => now.duration_since(*previous) >= TYPING_REFRESH_INTERVAL,
            None => true,
        };
        if due {
            last.insert(session_id.to_string(), now);
        }
        due
    }
}

fn spawn_typing_refresher(state: Arc<ImBridgeState>, typing: Arc<TypingTracker>) {
    let _ = thread::Builder::new()
        .name("im-bridge-typing".to_string())
        .spawn(move || loop {
            thread::sleep(TYPING_REFRESH_INTERVAL);
            let now = Instant::now();
            for session_id in typing.active_sessions() {
                if !typing.should_send(&session_id, now) {
                    continue;
                }
                let Some(chat) = state.chat_for_session(&session_id) else {
                    continue;
                };
                if let Err(error) = state.send_typing_to_chat(&chat) {
                    log::debug!("[im-bridge:outbound] typing send failed: {error:#}");
                }
            }
        });
}

fn handle_event(state: &Arc<ImBridgeState>, typing: &Arc<TypingTracker>, event: AgentRuntimeEvent) {
    match event.payload {
        AgentRuntimeEventPayload::TurnStarted {
            sessio_runtime_session_id,
            ..
        } => {
            if let Some(chat) = state.chat_for_session(&sessio_runtime_session_id) {
                typing.mark_active(&sessio_runtime_session_id);
                // Fire-and-forget initial typing burst so the user sees activity
                // immediately, rather than waiting up to one refresh interval.
                if let Err(error) = state.send_typing_to_chat(&chat) {
                    log::debug!("[im-bridge:outbound] initial typing send failed: {error:#}");
                }
            }
        }
        AgentRuntimeEventPayload::TextDelta {
            sessio_runtime_session_id,
            text,
            ..
        } => {
            if state.chat_for_session(&sessio_runtime_session_id).is_some() {
                state.buffer_text(&sessio_runtime_session_id, &text);
            }
        }
        AgentRuntimeEventPayload::ReasoningDelta {
            sessio_runtime_session_id,
            text,
            ..
        } => {
            if state.chat_for_session(&sessio_runtime_session_id).is_some() {
                state.buffer_thought(&sessio_runtime_session_id, &text);
            }
        }
        AgentRuntimeEventPayload::ToolStarted {
            sessio_runtime_session_id,
            name,
            input,
            data,
            ..
        } => {
            if state.chat_for_session(&sessio_runtime_session_id).is_some() {
                if is_todo_tool(&name, &data) {
                    if let Some(summary) = todo_summary(input.as_ref(), &data) {
                        state.buffer_tool_summary(&sessio_runtime_session_id, summary);
                    } else {
                        state.buffer_tool(&sessio_runtime_session_id, &name);
                    }
                } else {
                    state.buffer_tool(&sessio_runtime_session_id, &name);
                }
            }
        }
        AgentRuntimeEventPayload::PermissionRequested {
            sessio_runtime_session_id,
            request_id,
            tool_name,
            input,
            data,
            ..
        } => {
            let Some(chat) = state.chat_for_session(&sessio_runtime_session_id) else {
                return;
            };
            let options = permission_options_from_data(&data);
            let options = options
                .into_iter()
                .map(|(option_id, label)| ChatPermissionOption {
                    label,
                    token: state.register_permission_option(
                        &sessio_runtime_session_id,
                        &request_id,
                        &option_id,
                    ),
                    option_id,
                })
                .collect();
            let request = ChatPermissionRequest {
                tool_name,
                input_summary: input.as_ref().map(summarize_json),
                options,
            };
            if let Err(error) = state.send_permission_to_chat(
                &chat,
                &sessio_runtime_session_id,
                &request_id,
                &request,
            ) {
                log::warn!("[im-bridge:outbound] failed to send permission request: {error:#}");
            }
        }
        AgentRuntimeEventPayload::PermissionResolved {
            sessio_runtime_session_id,
            request_id,
            approved,
            option_id,
            ..
        } => {
            state.resolve_permission_message(
                &sessio_runtime_session_id,
                &request_id,
                approved,
                option_id.as_deref(),
            );
        }
        AgentRuntimeEventPayload::TurnCompleted {
            sessio_runtime_session_id,
            ..
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            flush_turn(state, &sessio_runtime_session_id, "Done.");
            dispatch_next_queued_prompt(state, &sessio_runtime_session_id);
        }
        AgentRuntimeEventPayload::TurnError {
            sessio_runtime_session_id,
            error,
            ..
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            let fallback = format!("Turn failed: {}", error.message);
            flush_turn(state, &sessio_runtime_session_id, &fallback);
            dispatch_next_queued_prompt(state, &sessio_runtime_session_id);
        }
        AgentRuntimeEventPayload::TurnCancelled {
            sessio_runtime_session_id,
            ..
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            flush_turn(state, &sessio_runtime_session_id, "Turn cancelled.");
            dispatch_next_queued_prompt(state, &sessio_runtime_session_id);
        }
        AgentRuntimeEventPayload::SessionEnded {
            sessio_runtime_session_id,
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            if let Some(chat) = state.chat_for_session(&sessio_runtime_session_id) {
                state.unbind_chat(&chat);
            }
        }
        _ => {}
    }
}

fn flush_turn(state: &Arc<ImBridgeState>, session_id: &str, empty_fallback: &str) {
    let Some(chat) = state.chat_for_session(session_id) else {
        return;
    };
    let buffer = state.take_turn_buffer(session_id).unwrap_or_default();
    let body = buffer.text.trim().to_string();
    let mut text = String::new();
    let thought = buffer.thought.trim();
    if !thought.is_empty() {
        text.push_str("💭 Thought\n");
        text.push_str(thought);
        text.push_str("\n\n");
    }
    if body.is_empty() {
        text.push_str(empty_fallback);
    } else {
        text.push_str(&body);
    }
    if !buffer.tool_summaries.is_empty() {
        text.push_str("\n\n");
        text.push_str(&buffer.tool_summaries.join("\n\n"));
    }
    if !buffer.tools.is_empty() {
        text.push_str("\n\nTools: ");
        text.push_str(&buffer.tools.join(", "));
    }
    if let Err(error) = state.send_to_chat(&chat, &text) {
        log::warn!("[im-bridge:outbound] failed to send chat reply: {error:#}");
    }
}

fn dispatch_next_queued_prompt(state: &Arc<ImBridgeState>, session_id: &str) {
    let Some(chat) = state.chat_for_session(session_id) else {
        return;
    };
    router::dispatch_next_queued_prompt(state, &chat);
}

fn permission_options_from_data(data: &Value) -> Vec<(String, String)> {
    let mut options = Vec::new();
    if let Some(items) = data.get("options").and_then(Value::as_array) {
        for item in items {
            let Some(option_id) = string_field(item, &["optionId", "option_id", "id"]) else {
                continue;
            };
            let label = string_field(item, &["name", "label", "title", "kind"])
                .unwrap_or_else(|| option_id.clone());
            options.push((option_id, label));
        }
    }
    if options.is_empty() {
        options.push(("allow_once".to_string(), "Allow once".to_string()));
        options.push(("reject_once".to_string(), "Reject once".to_string()));
    }
    options
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn summarize_json(value: &Value) -> String {
    let summary = match value {
        Value::String(text) => text.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    };
    const LIMIT: usize = 1200;
    if summary.chars().count() <= LIMIT {
        summary
    } else {
        let mut truncated = summary.chars().take(LIMIT).collect::<String>();
        truncated.push_str("\n...");
        truncated
    }
}

#[derive(Debug, Clone)]
struct TodoEntry {
    content: String,
    status: Option<String>,
}

fn is_todo_tool(name: &str, data: &Value) -> bool {
    let normalized = name.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "todowrite" | "todo_write" | "todos") {
        return true;
    }
    string_field(data, &["kind"])
        .map(|kind| kind == "todo" || kind == "task_list")
        .unwrap_or(false)
}

fn todo_summary(input: Option<&Value>, data: &Value) -> Option<String> {
    let todos = input
        .and_then(todo_entries_from_value)
        .or_else(|| todo_entries_from_value(data))?;
    if todos.is_empty() {
        return None;
    }

    let mut lines = vec!["Todos".to_string()];
    let total = todos.len();
    for todo in todos.into_iter().take(20) {
        lines.push(format!(
            "{} {}",
            todo_status_marker(todo.status.as_deref()),
            todo.content
        ));
    }
    if total > 20 {
        lines.push(format!("... {} more", total - 20));
    }
    Some(lines.join("\n"))
}

fn todo_entries_from_value(value: &Value) -> Option<Vec<TodoEntry>> {
    if let Some(entries) = value.as_array().map(|items| parse_todo_array(items)) {
        return Some(entries);
    }
    let object = value.as_object()?;
    for key in ["todos", "entries", "plan", "tasks"] {
        if let Some(entries) = object.get(key).and_then(todo_entries_from_value) {
            if !entries.is_empty() {
                return Some(entries);
            }
        }
    }
    for path in [
        ["rawInput"].as_slice(),
        ["raw_input"].as_slice(),
        ["fields", "rawInput"].as_slice(),
        ["fields", "raw_input"].as_slice(),
        ["toolCall", "fields", "rawInput"].as_slice(),
        ["toolCall", "fields", "raw_input"].as_slice(),
    ] {
        if let Some(candidate) = value_at_path(value, path) {
            if let Some(entries) = todo_entries_from_value(candidate) {
                if !entries.is_empty() {
                    return Some(entries);
                }
            }
        }
    }
    None
}

fn parse_todo_array(items: &[Value]) -> Vec<TodoEntry> {
    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let content = [
                "content",
                "activeForm",
                "active_form",
                "step",
                "text",
                "title",
            ]
            .iter()
            .find_map(|key| {
                object
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })?;
            let status = object
                .get("status")
                .and_then(Value::as_str)
                .map(str::to_string);
            Some(TodoEntry { content, status })
        })
        .collect()
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn todo_status_marker(status: Option<&str>) -> &'static str {
    match status.unwrap_or("").trim() {
        "completed" | "complete" | "done" => "[x]",
        "in_progress" | "in-progress" | "active" | "running" => "[~]",
        _ => "[ ]",
    }
}
