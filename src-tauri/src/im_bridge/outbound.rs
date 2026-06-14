//! Outbound routing: aggregate runtime events and deliver chat replies.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use serde_json::Value;

use crate::agents::runtime::types::{
    AgentAttachmentKind, AgentRuntimeEvent, AgentRuntimeEventPayload,
};

use super::attachments::{extract_outbound_attachments, OutboundAttachment};
use super::router;
use super::state::{
    ChatKey, ChatPermissionOption, ChatPermissionRequest, ChatStreamCapability, ChatStreamMode,
    ImBridgeState, TurnBuffer,
};

/// How often we re-send the platform's "typing" indicator while a turn is in
/// flight. Telegram's `sendChatAction` expires at ~5s and Discord's typing at
/// ~10s; 4s keeps both alive without spamming.
const TYPING_REFRESH_INTERVAL: Duration = Duration::from_secs(4);
/// Telegram `sendMessageDraft` previews are temporary 30-second drafts, so keep
/// resending the latest snapshot while a turn is blocked on external input.
const STREAM_DRAFT_REFRESH_INTERVAL: Duration = Duration::from_secs(20);

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
                | AgentRuntimeEventPayload::ToolInputDelta { .. }
                | AgentRuntimeEventPayload::ToolOutputDelta { .. }
                | AgentRuntimeEventPayload::ToolStatusChanged { .. }
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
    let streaming = Arc::new(StreamReplyTracker::default());
    spawn_typing_refresher(state.clone(), typing.clone());
    spawn_stream_refresher(state.clone(), streaming.clone());

    thread::Builder::new()
        .name("im-bridge-outbound".to_string())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                handle_event(&state, &typing, &streaming, event);
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

#[derive(Debug, Clone)]
struct StreamReplyState {
    chat: ChatKey,
    capability: ChatStreamCapability,
    message_ref: Option<Value>,
    last_sent_at: Option<Instant>,
    last_text: String,
    failed: bool,
}

#[derive(Default)]
struct StreamReplyTracker {
    streams: Mutex<HashMap<String, StreamReplyState>>,
    permission_blocked: Mutex<HashSet<String>>,
}

impl StreamReplyTracker {
    fn take(&self, session_id: &str) -> Option<StreamReplyState> {
        self.streams.lock().ok()?.remove(session_id)
    }

    fn clear(&self, session_id: &str) {
        if let Ok(mut streams) = self.streams.lock() {
            streams.remove(session_id);
        }
        self.set_permission_blocked(session_id, false);
    }

    fn set_permission_blocked(&self, session_id: &str, blocked: bool) {
        if let Ok(mut sessions) = self.permission_blocked.lock() {
            if blocked {
                sessions.insert(session_id.to_string());
            } else {
                sessions.remove(session_id);
            }
        }
    }

    fn is_permission_blocked(&self, session_id: &str) -> bool {
        self.permission_blocked
            .lock()
            .ok()
            .map(|sessions| sessions.contains(session_id))
            .unwrap_or(false)
    }

    fn active_draft_sessions(&self) -> Vec<String> {
        self.streams
            .lock()
            .ok()
            .map(|streams| {
                streams
                    .iter()
                    .filter(|(session_id, stream)| {
                        matches!(stream.capability.mode, ChatStreamMode::Draft)
                            && stream.message_ref.is_some()
                            && !stream.failed
                            && !stream.last_text.trim().is_empty()
                            && !self.is_permission_blocked(session_id)
                    })
                    .map(|(session_id, _)| session_id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn spawn_stream_refresher(state: Arc<ImBridgeState>, streaming: Arc<StreamReplyTracker>) {
    let _ = thread::Builder::new()
        .name("im-bridge-stream".to_string())
        .spawn(move || loop {
            thread::sleep(STREAM_DRAFT_REFRESH_INTERVAL);
            for session_id in streaming.active_draft_sessions() {
                maybe_update_stream_reply(&state, &streaming, &session_id, true);
            }
        });
}

fn handle_event(
    state: &Arc<ImBridgeState>,
    typing: &Arc<TypingTracker>,
    streaming: &Arc<StreamReplyTracker>,
    event: AgentRuntimeEvent,
) {
    match event.payload {
        AgentRuntimeEventPayload::TurnStarted {
            sessio_runtime_session_id,
            ..
        } => {
            if let Some(chat) = state.chat_for_session(&sessio_runtime_session_id) {
                typing.mark_active(&sessio_runtime_session_id);
                streaming.clear(&sessio_runtime_session_id);
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
        } if state.chat_for_session(&sessio_runtime_session_id).is_some() => {
            state.buffer_text(&sessio_runtime_session_id, &text);
            maybe_update_stream_reply(state, streaming, &sessio_runtime_session_id, false);
        }
        AgentRuntimeEventPayload::ReasoningDelta {
            sessio_runtime_session_id,
            text,
            ..
        } if state.chat_for_session(&sessio_runtime_session_id).is_some() => {
            state.buffer_thought(&sessio_runtime_session_id, &text);
            maybe_update_stream_reply(state, streaming, &sessio_runtime_session_id, false);
        }
        AgentRuntimeEventPayload::ToolStarted {
            sessio_runtime_session_id,
            name,
            input,
            data,
            ..
        } if state.chat_for_session(&sessio_runtime_session_id).is_some() => {
            if is_todo_tool(&name, &data) {
                if let Some(summary) = todo_summary(input.as_ref(), &data) {
                    state.buffer_tool_summary(&sessio_runtime_session_id, summary);
                } else {
                    state.buffer_tool(
                        &sessio_runtime_session_id,
                        &tool_display_name(&name, Some(&data)),
                    );
                }
            } else {
                state.buffer_tool(
                    &sessio_runtime_session_id,
                    &tool_display_name(&name, Some(&data)),
                );
            }
            maybe_update_stream_reply(state, streaming, &sessio_runtime_session_id, false);
        }
        AgentRuntimeEventPayload::ToolInputDelta {
            sessio_runtime_session_id,
            tool_id,
            data,
            ..
        }
        | AgentRuntimeEventPayload::ToolOutputDelta {
            sessio_runtime_session_id,
            tool_id,
            data,
            ..
        } if state.chat_for_session(&sessio_runtime_session_id).is_some() => {
            state.buffer_tool(
                &sessio_runtime_session_id,
                &tool_display_name(&tool_id, data.as_ref()),
            );
            maybe_update_stream_reply(state, streaming, &sessio_runtime_session_id, false);
        }
        AgentRuntimeEventPayload::ToolStatusChanged {
            sessio_runtime_session_id,
            tool_id,
            status,
            data,
            ..
        } if state.chat_for_session(&sessio_runtime_session_id).is_some() => {
            state.buffer_tool(
                &sessio_runtime_session_id,
                &tool_status_display_name(&tool_id, &status, data.as_ref()),
            );
            maybe_update_stream_reply(state, streaming, &sessio_runtime_session_id, false);
        }
        AgentRuntimeEventPayload::PermissionRequested {
            sessio_runtime_session_id,
            request_id,
            tool_name,
            input: _,
            data,
            ..
        } => {
            let Some(chat) = state.chat_for_session(&sessio_runtime_session_id) else {
                return;
            };
            let display_tool_name = tool_display_name(&tool_name, Some(&data));
            state.buffer_tool(&sessio_runtime_session_id, &display_tool_name);
            streaming.set_permission_blocked(&sessio_runtime_session_id, true);
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
                tool_name: if display_tool_name.is_empty() {
                    "requested action".to_string()
                } else {
                    display_tool_name
                },
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
            streaming.set_permission_blocked(&sessio_runtime_session_id, false);
            maybe_update_stream_reply(state, streaming, &sessio_runtime_session_id, true);
        }
        AgentRuntimeEventPayload::TurnCompleted {
            sessio_runtime_session_id,
            ..
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            streaming.set_permission_blocked(&sessio_runtime_session_id, false);
            flush_turn(state, streaming, &sessio_runtime_session_id, "Done.");
            dispatch_next_queued_prompt(state, &sessio_runtime_session_id);
        }
        AgentRuntimeEventPayload::TurnError {
            sessio_runtime_session_id,
            error,
            ..
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            streaming.set_permission_blocked(&sessio_runtime_session_id, false);
            let fallback = format!("Turn failed: {}", error.message);
            flush_turn(state, streaming, &sessio_runtime_session_id, &fallback);
            dispatch_next_queued_prompt(state, &sessio_runtime_session_id);
        }
        AgentRuntimeEventPayload::TurnCancelled {
            sessio_runtime_session_id,
            ..
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            streaming.set_permission_blocked(&sessio_runtime_session_id, false);
            flush_turn(
                state,
                streaming,
                &sessio_runtime_session_id,
                "Turn cancelled.",
            );
            dispatch_next_queued_prompt(state, &sessio_runtime_session_id);
        }
        AgentRuntimeEventPayload::SessionEnded {
            sessio_runtime_session_id,
        } => {
            typing.mark_inactive(&sessio_runtime_session_id);
            streaming.set_permission_blocked(&sessio_runtime_session_id, false);
            streaming.clear(&sessio_runtime_session_id);
            if let Some(chat) = state.chat_for_session(&sessio_runtime_session_id) {
                state.unbind_chat(&chat);
            }
        }
        _ => {}
    }
}

fn maybe_update_stream_reply(
    state: &Arc<ImBridgeState>,
    streaming: &Arc<StreamReplyTracker>,
    session_id: &str,
    force: bool,
) {
    if streaming.is_permission_blocked(session_id) {
        return;
    }
    let Some(chat) = state.chat_for_session(session_id) else {
        return;
    };
    let Some(capability) = state.stream_capability_for_chat(&chat) else {
        streaming.clear(session_id);
        return;
    };
    let Some(buffer) = state.turn_buffer_snapshot(session_id) else {
        return;
    };
    let text = truncate_chars(format_turn_text(&buffer, "").trim(), capability.max_chars);
    if text.is_empty() {
        return;
    }
    let now = Instant::now();

    enum StreamAction {
        Start {
            chat: ChatKey,
            text: String,
        },
        Update {
            chat: ChatKey,
            message_ref: Value,
            text: String,
        },
    }

    let action = {
        let mut streams = match streaming.streams.lock() {
            Ok(streams) => streams,
            Err(_) => return,
        };
        let entry = streams
            .entry(session_id.to_string())
            .or_insert_with(|| StreamReplyState {
                chat: chat.clone(),
                capability,
                message_ref: None,
                last_sent_at: None,
                last_text: String::new(),
                failed: false,
            });
        entry.chat = chat.clone();
        entry.capability = capability;
        if entry.failed {
            return;
        }
        if !force {
            if let Some(last_sent_at) = entry.last_sent_at {
                if now.duration_since(last_sent_at) < capability.min_interval {
                    return;
                }
            }
        }
        if !force && entry.last_text == text {
            return;
        }
        entry.last_sent_at = Some(now);
        entry.last_text = text.clone();
        match entry.message_ref.clone() {
            Some(message_ref) => StreamAction::Update {
                chat: chat.clone(),
                message_ref,
                text,
            },
            None => StreamAction::Start {
                chat: chat.clone(),
                text,
            },
        }
    };

    let result = match action {
        StreamAction::Start { chat, text } => {
            state.start_stream_reply_to_chat(&chat, &text).map(Some)
        }
        StreamAction::Update {
            chat,
            message_ref,
            text,
        } => state
            .update_stream_reply_to_chat(&chat, &message_ref, &text)
            .map(|_| None),
    };

    if let Ok(message_ref) = result {
        if let Some(message_ref) = message_ref {
            if let Ok(mut streams) = streaming.streams.lock() {
                if let Some(entry) = streams.get_mut(session_id) {
                    entry.message_ref = Some(message_ref);
                }
            }
        }
    } else if let Err(error) = result {
        if let Ok(mut streams) = streaming.streams.lock() {
            if let Some(entry) = streams.get_mut(session_id) {
                entry.failed = true;
            }
        }
        log::debug!("[im-bridge:outbound] stream reply update failed: {error:#}");
    }
}

fn flush_turn(
    state: &Arc<ImBridgeState>,
    streaming: &Arc<StreamReplyTracker>,
    session_id: &str,
    empty_fallback: &str,
) {
    let Some(chat) = state.chat_for_session(session_id) else {
        return;
    };
    let buffer = state.take_turn_buffer(session_id).unwrap_or_default();
    let stream = streaming.take(session_id);
    let body = buffer.text.trim().to_string();
    let text = format_turn_text(&buffer, empty_fallback);
    // Extract attachments referenced from the body text and dispatch them
    // through the platform sink before the text reply (caption-style). Send
    // text first so platforms without media support still see the message.
    let attachments = collect_outbound_attachments(state, &chat, &body);

    let mut text_finalized_by_stream = false;
    if let Some(stream) = stream.as_ref() {
        if !stream.failed {
            if let Some(message_ref) = stream.message_ref.as_ref() {
                let final_text = truncate_chars(text.trim(), stream.capability.max_chars);
                if !final_text.is_empty() {
                    if let Err(error) =
                        state.finish_stream_reply_to_chat(&stream.chat, message_ref, &final_text)
                    {
                        log::debug!("[im-bridge:outbound] stream reply finish failed: {error:#}");
                    } else if matches!(stream.capability.mode, ChatStreamMode::Editable) {
                        text_finalized_by_stream = true;
                    }
                }
            }
        }
    }

    if !text_finalized_by_stream {
        if let Err(error) = state.send_to_chat(&chat, &text) {
            log::warn!("[im-bridge:outbound] failed to send chat reply: {error:#}");
        }
    }
    for attachment in attachments {
        if let Err(error) = send_outbound_attachment(state, &chat, &attachment) {
            log::warn!(
                "[im-bridge:outbound] failed to send attachment {}: {error:#}",
                attachment.path.display()
            );
        }
    }
}

fn format_turn_text(buffer: &TurnBuffer, empty_fallback: &str) -> String {
    let body = buffer.text.trim();
    let mut text = String::new();
    let thought = buffer.thought.trim();
    if !thought.is_empty() {
        append_turn_section(&mut text, &format!("💭 Thought\n{thought}"));
    }
    if !body.is_empty() {
        append_turn_section(&mut text, body);
    }
    if !buffer.tool_summaries.is_empty() {
        append_turn_section(&mut text, &buffer.tool_summaries.join("\n\n"));
    }
    let tools = buffer
        .tools
        .iter()
        .map(|tool| tool.trim())
        .filter(|tool| !tool.is_empty())
        .collect::<Vec<_>>();
    if !tools.is_empty() {
        append_turn_section(&mut text, &format!("Tools: {}", tools.join(", ")));
    }
    if text.trim().is_empty() {
        text.push_str(empty_fallback);
    }
    text
}

fn append_turn_section(text: &mut String, section: &str) {
    let section = section.trim();
    if section.is_empty() {
        return;
    }
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    text.push_str(section);
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 || text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect()
}

/// Resolve assistant-referenced attachment paths against the chat's workspace,
/// keep only those the platform sink can actually upload, and cap the count so
/// a runaway response cannot flood the chat.
fn collect_outbound_attachments(
    state: &Arc<ImBridgeState>,
    chat: &ChatKey,
    body: &str,
) -> Vec<OutboundAttachment> {
    const MAX_OUTBOUND_ATTACHMENTS: usize = 5;
    let Some(session) = state.chat_session(chat) else {
        return Vec::new();
    };
    let supports_images = state.sink_supports_images(chat);
    let supports_files = state.sink_supports_files(chat);
    if !supports_images && !supports_files {
        return Vec::new();
    }
    extract_outbound_attachments(body, &session.workspace_path)
        .into_iter()
        .filter(|attachment| match attachment.kind {
            AgentAttachmentKind::Image => supports_images,
            AgentAttachmentKind::File => supports_files,
        })
        .take(MAX_OUTBOUND_ATTACHMENTS)
        .collect()
}

fn send_outbound_attachment(
    state: &Arc<ImBridgeState>,
    chat: &ChatKey,
    attachment: &OutboundAttachment,
) -> Result<()> {
    let caption = attachment.display_name.as_deref();
    match attachment.kind {
        AgentAttachmentKind::Image => state.send_image_to_chat(chat, &attachment.path, caption),
        AgentAttachmentKind::File => state.send_file_to_chat(chat, &attachment.path, caption),
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

fn tool_display_name(tool_id: &str, data: Option<&Value>) -> String {
    let name = data.and_then(|value| {
        string_field(value, &["title", "name"])
            .or_else(|| string_field(value.get("fields")?, &["title", "name"]))
            .or_else(|| string_field(value.get("toolCall")?, &["title", "name"]))
            .or_else(|| string_field(value.get("tool_call")?, &["title", "name"]))
    });
    match name {
        Some(name) if !looks_like_tool_use_id(&name) => name,
        _ if !looks_like_tool_use_id(tool_id) => tool_id.to_string(),
        _ => String::new(),
    }
}

fn tool_status_display_name(tool_id: &str, status: &str, data: Option<&Value>) -> String {
    let name = tool_display_name(tool_id, data);
    let status = status.trim();
    if name.is_empty() {
        return String::new();
    }
    if status.is_empty() {
        name
    } else {
        format!("{name} ({status})")
    }
}

fn looks_like_tool_use_id(value: &str) -> bool {
    let value = value.trim();
    let normalized = value.to_ascii_lowercase();
    normalized.starts_with("tooluse_")
        || normalized.starts_with("toolu_")
        || normalized.starts_with("tool_call_")
        || normalized.starts_with("call_")
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_display_name_hides_internal_tool_use_ids() {
        assert_eq!(tool_display_name("tooluse_abc123", None), "");
        assert_eq!(tool_display_name("toolu_abc123", None), "");
        assert_eq!(tool_display_name("tool_call_abc123", None), "");
        assert_eq!(tool_display_name("call_abc123", None), "");
    }

    #[test]
    fn tool_display_name_prefers_human_title_over_internal_id() {
        let data = json!({ "title": "Read File" });
        assert_eq!(
            tool_display_name("tooluse_abc123", Some(&data)),
            "Read File"
        );

        let nested = json!({ "tool_call": { "name": "Terminal" } });
        assert_eq!(
            tool_display_name("tooluse_def456", Some(&nested)),
            "Terminal"
        );
    }

    #[test]
    fn tool_status_display_name_hides_internal_id_without_title() {
        assert_eq!(
            tool_status_display_name("tooluse_abc123", "completed", None),
            ""
        );
    }

    #[test]
    fn format_turn_text_omits_empty_tool_footer() {
        let buffer = TurnBuffer {
            tools: vec!["".to_string(), "  ".to_string()],
            ..TurnBuffer::default()
        };

        assert_eq!(format_turn_text(&buffer, "Done."), "Done.");
    }

    #[test]
    fn format_turn_text_lists_tools_together() {
        let buffer = TurnBuffer {
            text: "Result".to_string(),
            tools: vec!["Terminal".to_string(), "Read File".to_string()],
            ..TurnBuffer::default()
        };

        assert_eq!(
            format_turn_text(&buffer, "Done."),
            "Result\n\nTools: Terminal, Read File"
        );
    }
}
