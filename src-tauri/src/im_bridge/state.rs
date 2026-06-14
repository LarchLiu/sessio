//! Shared, platform-agnostic state for the IM bridge.
//!
//! Holds the chat-to-session mapping, the per-turn outbound text buffers, and
//! the handles ([`RuntimeManager`], [`SessionStore`], config) every worker
//! needs. One instance lives behind an `Arc` for the lifetime of the app.

use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};

use crate::agents::runtime::RuntimeManager;
use crate::models::Agent;
use crate::store::{ChannelSessionRecord, SessionStore};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::config::ImBridgeConfig;
use super::router::QueuedPrompt;

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

/// Identifies a chat across platforms. `platform` is a short tag ("telegram",
/// "discord", ...) so the same chat id on two platforms never collides.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChatKey {
    pub platform: &'static str,
    pub chat_id: String,
}

impl ChatKey {
    pub fn new(platform: &'static str, chat_id: impl Into<String>) -> Self {
        Self {
            platform,
            chat_id: chat_id.into(),
        }
    }
}

/// Per-chat binding: which sessio runtime session this chat currently drives,
/// plus the agent/workspace it was opened with so `/new` can be reissued.
#[derive(Debug, Clone)]
pub struct ChatSession {
    pub sessio_runtime_session_id: String,
    pub agent_runtime_session_id: Option<String>,
    pub agent: Agent,
    pub workspace_path: String,
    pub last_activity_at: i64,
}

/// Platform-neutral metadata for the external chat/channel that owns a session.
#[derive(Debug, Clone, Default)]
pub struct ChannelContext {
    pub channel_type: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub thread_id: Option<String>,
    pub display_name: Option<String>,
    pub metadata: JsonMap<String, JsonValue>,
    pub last_update_id: Option<i64>,
}

/// Buffers an in-flight turn's streamed text so the bridge can post one
/// consolidated reply on completion rather than a message per delta.
#[derive(Debug, Default)]
pub struct TurnBuffer {
    pub text: String,
    /// Streamed reasoning/thought content shown as a prefix block on flush.
    pub thought: String,
    /// Tool-call titles seen this turn, surfaced as a short activity footer.
    pub tools: Vec<String>,
    /// Human-readable tool summaries, such as TodoWrite checklist updates.
    pub tool_summaries: Vec<String>,
}

/// Button-style permission prompt sent to a chat platform.
#[derive(Debug, Clone)]
pub struct ChatPermissionRequest {
    pub tool_name: String,
    pub input_summary: Option<String>,
    pub options: Vec<ChatPermissionOption>,
}

impl ChatPermissionRequest {
    pub fn fallback_text(&self) -> String {
        let mut text = format!("Permission requested for tool: {}", self.tool_name);
        if let Some(input) = &self.input_summary {
            if !input.trim().is_empty() {
                text.push_str("\n\n");
                text.push_str(input);
            }
        }
        if !self.options.is_empty() {
            text.push_str("\n\nOpen Sessio or use a supported platform button to respond.");
        }
        text
    }
}

#[derive(Debug, Clone)]
pub struct ChatPermissionOption {
    pub option_id: String,
    pub label: String,
    pub token: String,
}

#[derive(Debug, Clone)]
pub struct PendingPermissionDecision {
    pub sessio_runtime_session_id: String,
    pub request_id: String,
    pub option_id: String,
}

/// Tracks a permission message we already sent to a chat so we can edit/clear
/// the buttons once the upstream runtime emits `PermissionResolved` (from any
/// channel — IM button click, Tauri UI, etc.).
#[derive(Debug, Clone)]
pub struct PendingPermissionMessage {
    pub chat: ChatKey,
    /// Opaque per-platform handle (message id, channel+id, etc.) the sink will
    /// use to locate the original message.
    pub message_ref: JsonValue,
    pub request: ChatPermissionRequest,
}

/// A platform's outbound sink. Implemented per platform so the outbound pump
/// stays platform-agnostic. Implementations must be cheap to clone or wrap an
/// `Arc` internally; they are called from the outbound thread.
pub trait ChatSink: Send + Sync {
    /// Platform tag, e.g. "telegram".
    fn platform(&self) -> &'static str;

    /// Deliver a plain-text message to `chat_id`. Errors are logged by the
    /// caller; a failed send must not poison the pump.
    fn send_text(&self, chat_id: &str, text: &str) -> anyhow::Result<()>;

    /// Upload an image file from local disk. Default falls back to a text note
    /// so the user at least sees the path.
    fn send_image(
        &self,
        chat_id: &str,
        path: &std::path::Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image");
        let mut text = format!("[image: {}]", label);
        if let Some(caption) = caption {
            if !caption.trim().is_empty() {
                text.push('\n');
                text.push_str(caption);
            }
        }
        self.send_text(chat_id, &text)
    }

    /// Upload a generic file from local disk. Default falls back to a text note.
    fn send_file(
        &self,
        chat_id: &str,
        path: &std::path::Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        let mut text = format!("[file: {}]", label);
        if let Some(caption) = caption {
            if !caption.trim().is_empty() {
                text.push('\n');
                text.push_str(caption);
            }
        }
        self.send_text(chat_id, &text)
    }

    /// Whether this platform can upload images via [`send_image`]. Outbound
    /// scanning skips files when both this and [`supports_files`] return false.
    fn supports_images(&self) -> bool {
        false
    }

    /// Whether this platform can upload arbitrary files via [`send_file`].
    fn supports_files(&self) -> bool {
        false
    }

    /// Deliver a permission request. Platforms with buttons override this;
    /// others get a safe text-only fallback.
    ///
    /// Returns an opaque platform-specific message handle when the sink can
    /// later edit the message (to strip buttons after the user responds). A
    /// `None` return means the sink delivered the request but cannot edit it
    /// (e.g. plain-text fallback).
    fn send_permission_request(
        &self,
        chat_id: &str,
        request: &ChatPermissionRequest,
    ) -> anyhow::Result<Option<JsonValue>> {
        self.send_text(chat_id, &request.fallback_text())?;
        Ok(None)
    }

    /// Strip interactive controls from a previously sent permission message and
    /// annotate it with the user's choice so the prompt cannot be triggered
    /// twice. Default no-op for platforms that cannot edit messages.
    fn resolve_permission_message(
        &self,
        _chat_id: &str,
        _message_ref: &JsonValue,
        _request: &ChatPermissionRequest,
        _outcome: PermissionResolutionOutcome<'_>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Send a "typing" / activity indicator to the chat. Implemented for
    /// platforms that support it (Telegram, Discord). Default no-op.
    fn send_typing(&self, _chat_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Conveys the resolution of a permission prompt to the sink that originally
/// rendered it. `label` is the human-readable name of the chosen option when
/// known, used to annotate the original message.
#[derive(Debug, Clone, Copy)]
pub struct PermissionResolutionOutcome<'a> {
    pub approved: bool,
    pub label: Option<&'a str>,
}

struct Inner {
    /// chat -> active session binding
    chats: Mutex<HashMap<ChatKey, ChatSession>>,
    /// chat -> latest platform metadata for persistence/display
    channel_contexts: Mutex<HashMap<ChatKey, ChannelContext>>,
    /// sessio runtime session id -> owning chat (reverse lookup for events)
    session_to_chat: Mutex<HashMap<String, ChatKey>>,
    /// sessio runtime session id -> in-flight turn buffer
    turns: Mutex<HashMap<String, TurnBuffer>>,
    /// chat -> plain prompts waiting for the active turn to finish
    inbound_queues: Mutex<HashMap<ChatKey, VecDeque<QueuedPrompt>>>,
    /// short callback token -> permission response data
    pending_permissions: Mutex<HashMap<String, PendingPermissionDecision>>,
    /// sessio runtime session id -> request id -> rendered permission message
    /// so we can edit/clear it after the user (or another channel) resolves it.
    pending_permission_messages:
        Mutex<HashMap<String, HashMap<String, PendingPermissionMessage>>>,
    permission_counter: AtomicU64,
    /// registered outbound sinks, keyed by platform tag
    sinks: Mutex<HashMap<&'static str, Arc<dyn ChatSink>>>,
}

/// The bridge's shared state. Cloneable handle semantics via `Arc` at the
/// service level; this struct itself is held behind one `Arc`.
pub struct ImBridgeState {
    pub store: Arc<dyn SessionStore>,
    pub runtime: RuntimeManager,
    config: RwLock<ImBridgeConfig>,
    inner: Inner,
}

impl ImBridgeState {
    pub fn new(
        store: Arc<dyn SessionStore>,
        runtime: RuntimeManager,
        config: ImBridgeConfig,
    ) -> Self {
        Self {
            store,
            runtime,
            config: RwLock::new(config),
            inner: Inner {
                chats: Mutex::new(HashMap::new()),
                channel_contexts: Mutex::new(HashMap::new()),
                session_to_chat: Mutex::new(HashMap::new()),
                turns: Mutex::new(HashMap::new()),
                inbound_queues: Mutex::new(HashMap::new()),
                pending_permissions: Mutex::new(HashMap::new()),
                pending_permission_messages: Mutex::new(HashMap::new()),
                permission_counter: AtomicU64::new(1),
                sinks: Mutex::new(HashMap::new()),
            },
        }
    }

    /// Snapshot the current bridge config. Runtime workers read this frequently
    /// so settings saved from the UI can take effect without restarting Sessio.
    pub fn config_snapshot(&self) -> ImBridgeConfig {
        self.config
            .read()
            .map(|config| config.clone())
            .unwrap_or_default()
    }

    /// Replace the active bridge config after the UI saves it to disk.
    pub fn update_config(&self, config: ImBridgeConfig) {
        if let Ok(mut current) = self.config.write() {
            *current = config;
        }
    }

    /// Register a platform's outbound sink so the event pump can route replies.
    pub fn register_sink(&self, sink: Arc<dyn ChatSink>) {
        if let Ok(mut sinks) = self.inner.sinks.lock() {
            sinks.insert(sink.platform(), sink);
        }
    }

    /// Remove a platform sink, typically when a platform is disabled or its
    /// token becomes invalid during a live config reload.
    pub fn unregister_sink(&self, platform: &'static str) {
        if let Ok(mut sinks) = self.inner.sinks.lock() {
            sinks.remove(platform);
        }
    }

    /// Look up the current session binding for a chat.
    pub fn chat_session(&self, key: &ChatKey) -> Option<ChatSession> {
        self.inner.chats.lock().ok()?.get(key).cloned()
    }

    /// Mark a chat as recently used. Inbound IM messages and platform callback
    /// interactions both count as activity for idle suspension.
    pub fn touch_chat(&self, key: &ChatKey) {
        if let Ok(mut chats) = self.inner.chats.lock() {
            if let Some(session) = chats.get_mut(key) {
                session.last_activity_at = now_ms();
            }
        }
    }

    /// Remember the latest platform-side identifiers for this chat and refresh
    /// the persisted row if the chat is already bound to a runtime session.
    pub fn remember_channel_context(&self, key: ChatKey, context: ChannelContext) {
        if let Ok(mut contexts) = self.inner.channel_contexts.lock() {
            contexts.insert(key.clone(), context);
        }
        if let Some(session) = self.chat_session(&key) {
            self.persist_channel_session(&key, &session);
        }
    }

    fn channel_context(&self, key: &ChatKey) -> Option<ChannelContext> {
        self.inner.channel_contexts.lock().ok()?.get(key).cloned()
    }

    /// Bind a chat to a runtime session, replacing any prior binding and
    /// maintaining the reverse index.
    pub fn bind_chat(&self, key: ChatKey, mut session: ChatSession) {
        if session.last_activity_at <= 0 {
            session.last_activity_at = now_ms();
        }
        self.persist_channel_session(&key, &session);
        let old_session_id = self
            .inner
            .chats
            .lock()
            .ok()
            .and_then(|mut chats| chats.insert(key.clone(), session.clone()))
            .map(|old| old.sessio_runtime_session_id);
        if let Ok(mut sessions) = self.inner.session_to_chat.lock() {
            if let Some(old_session_id) = &old_session_id {
                sessions.remove(old_session_id);
            }
            sessions.insert(session.sessio_runtime_session_id.clone(), key);
        }
        if let Some(old_session_id) = old_session_id {
            if let Ok(mut turns) = self.inner.turns.lock() {
                turns.remove(&old_session_id);
            }
        }
    }

    /// Drop a chat's binding (e.g. after the session ends). Also clears the
    /// reverse index and any pending turn buffer.
    pub fn unbind_chat(&self, key: &ChatKey) {
        let removed = self
            .inner
            .chats
            .lock()
            .ok()
            .and_then(|mut chats| chats.remove(key));
        if let Some(session) = removed {
            self.mark_channel_session_ended(key, &session);
            let session_id = session.sessio_runtime_session_id;
            if let Ok(mut rev) = self.inner.session_to_chat.lock() {
                rev.remove(&session_id);
            }
            if let Ok(mut turns) = self.inner.turns.lock() {
                turns.remove(&session_id);
            }
        }
        self.clear_queued_prompts(key);
    }

    /// Drop a chat's in-memory runtime binding without marking the persisted
    /// channel session as ended. Used for idle suspend; the next inbound
    /// message resumes from `channel_sessions`.
    pub fn suspend_chat(&self, key: &ChatKey) -> Option<ChatSession> {
        let removed = self
            .inner
            .chats
            .lock()
            .ok()
            .and_then(|mut chats| chats.remove(key));
        if let Some(session) = removed.as_ref() {
            let session_id = &session.sessio_runtime_session_id;
            if let Ok(mut rev) = self.inner.session_to_chat.lock() {
                rev.remove(session_id);
            }
            if let Ok(mut turns) = self.inner.turns.lock() {
                turns.remove(session_id);
            }
        }
        self.clear_queued_prompts(key);
        removed
    }

    pub fn idle_suspend_candidates(&self, idle_before_ms: i64) -> Vec<(ChatKey, ChatSession)> {
        self.inner
            .chats
            .lock()
            .map(|chats| {
                chats
                    .iter()
                    .filter(|(_, session)| session.last_activity_at <= idle_before_ms)
                    .map(|(key, session)| (key.clone(), session.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn persist_channel_session(&self, key: &ChatKey, session: &ChatSession) {
        let Some(agent_session_id) = session
            .agent_runtime_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        let now = now_ms();
        let context = self.channel_context(key);
        let mut metadata = JsonMap::new();
        metadata.insert("chatId".to_string(), JsonValue::String(key.chat_id.clone()));
        if let Some(context) = context.as_ref() {
            for (metadata_key, metadata_value) in &context.metadata {
                metadata.insert(metadata_key.clone(), metadata_value.clone());
            }
        }
        let metadata_json = JsonValue::Object(metadata).to_string();
        let record = ChannelSessionRecord {
            platform: key.platform.to_string(),
            channel_id: key.chat_id.clone(),
            channel_type: context
                .as_ref()
                .and_then(|value| value.channel_type.clone()),
            user_id: context.as_ref().and_then(|value| value.user_id.clone()),
            team_id: context.as_ref().and_then(|value| value.team_id.clone()),
            thread_id: context.as_ref().and_then(|value| value.thread_id.clone()),
            display_name: context
                .as_ref()
                .and_then(|value| value.display_name.clone()),
            agent: session.agent,
            agent_session_id: agent_session_id.to_string(),
            sessio_runtime_session_id: session.sessio_runtime_session_id.clone(),
            workspace_path: session.workspace_path.clone(),
            metadata_json,
            last_update_id: context.as_ref().and_then(|value| value.last_update_id),
            created_at: now,
            updated_at: now,
            last_activity_at: now,
            ended_at: None,
        };
        if let Err(error) = self.store.upsert_channel_session(&record) {
            log::warn!("[im-bridge] failed to persist channel session: {error:#}");
        }
    }

    fn mark_channel_session_ended(&self, key: &ChatKey, session: &ChatSession) {
        let Some(agent_session_id) = session
            .agent_runtime_session_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return;
        };
        if let Err(error) = self.store.mark_channel_session_ended(
            key.platform,
            &key.chat_id,
            session.agent,
            agent_session_id,
            now_ms(),
        ) {
            log::warn!("[im-bridge] failed to end channel session: {error:#}");
        }
    }

    /// Reverse lookup: which chat owns this runtime session, if any.
    pub fn chat_for_session(&self, session_id: &str) -> Option<ChatKey> {
        self.inner
            .session_to_chat
            .lock()
            .ok()?
            .get(session_id)
            .cloned()
    }

    /// Append streamed assistant text to a session's turn buffer.
    pub fn buffer_text(&self, session_id: &str, text: &str) {
        if let Ok(mut turns) = self.inner.turns.lock() {
            turns
                .entry(session_id.to_string())
                .or_default()
                .text
                .push_str(text);
        }
    }

    /// Append streamed reasoning/thought content for the active turn.
    pub fn buffer_thought(&self, session_id: &str, text: &str) {
        if let Ok(mut turns) = self.inner.turns.lock() {
            turns
                .entry(session_id.to_string())
                .or_default()
                .thought
                .push_str(text);
        }
    }

    /// Record a tool-call title for the current turn.
    pub fn buffer_tool(&self, session_id: &str, tool: &str) {
        if let Ok(mut turns) = self.inner.turns.lock() {
            let buf = turns.entry(session_id.to_string()).or_default();
            if !buf.tools.iter().any(|t| t == tool) {
                buf.tools.push(tool.to_string());
            }
        }
    }

    /// Record a formatted tool summary for the current turn.
    pub fn buffer_tool_summary(&self, session_id: &str, summary: String) {
        if summary.trim().is_empty() {
            return;
        }
        if let Ok(mut turns) = self.inner.turns.lock() {
            turns
                .entry(session_id.to_string())
                .or_default()
                .tool_summaries
                .push(summary);
        }
    }

    /// Take and clear the turn buffer for a session (called on turn completion).
    pub fn take_turn_buffer(&self, session_id: &str) -> Option<TurnBuffer> {
        self.inner.turns.lock().ok()?.remove(session_id)
    }

    /// Queue a prompt behind the chat's active turn. Returns the new 1-based
    /// queue length for user-facing feedback.
    pub fn enqueue_prompt(&self, key: &ChatKey, prompt: QueuedPrompt) -> usize {
        if let Ok(mut queues) = self.inner.inbound_queues.lock() {
            let queue = queues.entry(key.clone()).or_default();
            queue.push_back(prompt);
            queue.len()
        } else {
            0
        }
    }

    /// Put a prompt back at the front of a chat queue. Used when a queued prompt
    /// races with another active turn.
    pub fn prepend_prompt(&self, key: &ChatKey, prompt: QueuedPrompt) {
        if let Ok(mut queues) = self.inner.inbound_queues.lock() {
            queues.entry(key.clone()).or_default().push_front(prompt);
        }
    }

    /// Pop the next queued prompt for a chat.
    pub fn pop_queued_prompt(&self, key: &ChatKey) -> Option<QueuedPrompt> {
        let mut queues = self.inner.inbound_queues.lock().ok()?;
        let queue = queues.get_mut(key)?;
        let prompt = queue.pop_front();
        if queue.is_empty() {
            queues.remove(key);
        }
        prompt
    }

    /// Current queued prompt count for a chat.
    pub fn queued_prompt_count(&self, key: &ChatKey) -> usize {
        self.inner
            .inbound_queues
            .lock()
            .ok()
            .and_then(|queues| queues.get(key).map(VecDeque::len))
            .unwrap_or(0)
    }

    /// Clear queued prompts for a chat, usually after switching sessions.
    pub fn clear_queued_prompts(&self, key: &ChatKey) {
        if let Ok(mut queues) = self.inner.inbound_queues.lock() {
            queues.remove(key);
        }
    }

    /// Send text to a chat via its platform sink. Returns an error if no sink is
    /// registered for that platform or the send fails.
    pub fn send_to_chat(&self, key: &ChatKey, text: &str) -> anyhow::Result<()> {
        let sink = self
            .inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .ok_or_else(|| anyhow::anyhow!("no sink registered for platform {}", key.platform))?;
        sink.send_text(&key.chat_id, text)
    }

    /// Send an image file to a chat via its platform sink.
    pub fn send_image_to_chat(
        &self,
        key: &ChatKey,
        path: &std::path::Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let sink = self
            .inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .ok_or_else(|| anyhow::anyhow!("no sink registered for platform {}", key.platform))?;
        sink.send_image(&key.chat_id, path, caption)
    }

    /// Send a generic file to a chat via its platform sink.
    pub fn send_file_to_chat(
        &self,
        key: &ChatKey,
        path: &std::path::Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let sink = self
            .inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .ok_or_else(|| anyhow::anyhow!("no sink registered for platform {}", key.platform))?;
        sink.send_file(&key.chat_id, path, caption)
    }

    /// Whether the platform sink for `key` advertises image upload support.
    pub fn sink_supports_images(&self, key: &ChatKey) -> bool {
        self.inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .map(|sink| sink.supports_images())
            .unwrap_or(false)
    }

    /// Whether the platform sink for `key` advertises file upload support.
    pub fn sink_supports_files(&self, key: &ChatKey) -> bool {
        self.inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .map(|sink| sink.supports_files())
            .unwrap_or(false)
    }

    /// Send a permission request to a chat via its platform sink. If the sink
    /// returns a message handle, we remember it so a later `PermissionResolved`
    /// event can strip the buttons and prevent double-triggering.
    pub fn send_permission_to_chat(
        &self,
        key: &ChatKey,
        sessio_runtime_session_id: &str,
        request_id: &str,
        request: &ChatPermissionRequest,
    ) -> anyhow::Result<()> {
        let sink = self
            .inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .ok_or_else(|| anyhow::anyhow!("no sink registered for platform {}", key.platform))?;
        let message_ref = sink.send_permission_request(&key.chat_id, request)?;
        if let Some(message_ref) = message_ref {
            if let Ok(mut messages) = self.inner.pending_permission_messages.lock() {
                messages
                    .entry(sessio_runtime_session_id.to_string())
                    .or_default()
                    .insert(
                        request_id.to_string(),
                        PendingPermissionMessage {
                            chat: key.clone(),
                            message_ref,
                            request: request.clone(),
                        },
                    );
            }
        }
        Ok(())
    }

    /// Send a "typing" indicator to a chat. Best-effort: missing sinks or
    /// platform errors return an error but the caller usually just logs it.
    pub fn send_typing_to_chat(&self, key: &ChatKey) -> anyhow::Result<()> {
        let sink = self
            .inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(key.platform).cloned())
            .ok_or_else(|| anyhow::anyhow!("no sink registered for platform {}", key.platform))?;
        sink.send_typing(&key.chat_id)
    }

    /// Pop the stored permission-message handle for a (session, request) pair.
    pub fn take_permission_message(
        &self,
        sessio_runtime_session_id: &str,
        request_id: &str,
    ) -> Option<PendingPermissionMessage> {
        let mut messages = self.inner.pending_permission_messages.lock().ok()?;
        let bucket = messages.get_mut(sessio_runtime_session_id)?;
        let removed = bucket.remove(request_id);
        if bucket.is_empty() {
            messages.remove(sessio_runtime_session_id);
        }
        removed
    }

    /// Strip controls from a previously-rendered permission message. Used when
    /// the runtime emits `PermissionResolved` (from any source: an IM button,
    /// the Tauri UI, or another platform).
    pub fn resolve_permission_message(
        &self,
        sessio_runtime_session_id: &str,
        request_id: &str,
        approved: bool,
        option_id: Option<&str>,
    ) {
        let Some(pending) = self.take_permission_message(sessio_runtime_session_id, request_id)
        else {
            return;
        };
        let sink = match self
            .inner
            .sinks
            .lock()
            .ok()
            .and_then(|sinks| sinks.get(pending.chat.platform).cloned())
        {
            Some(sink) => sink,
            None => return,
        };
        let label = option_id.and_then(|wanted| {
            pending
                .request
                .options
                .iter()
                .find(|option| option.option_id == wanted)
                .map(|option| option.label.as_str())
        });
        let outcome = PermissionResolutionOutcome { approved, label };
        if let Err(error) = sink.resolve_permission_message(
            &pending.chat.chat_id,
            &pending.message_ref,
            &pending.request,
            outcome,
        ) {
            log::warn!(
                "[im-bridge:{}] failed to resolve permission message: {error:#}",
                pending.chat.platform
            );
        }
    }

    /// Create a compact callback token for a permission option.
    pub fn register_permission_option(
        &self,
        sessio_runtime_session_id: &str,
        request_id: &str,
        option_id: &str,
    ) -> String {
        let token = format!(
            "p{}",
            self.inner
                .permission_counter
                .fetch_add(1, Ordering::Relaxed)
        );
        if let Ok(mut pending) = self.inner.pending_permissions.lock() {
            pending.insert(
                token.clone(),
                PendingPermissionDecision {
                    sessio_runtime_session_id: sessio_runtime_session_id.to_string(),
                    request_id: request_id.to_string(),
                    option_id: option_id.to_string(),
                },
            );
        }
        token
    }

    /// Resolve and remove a permission callback token.
    pub fn take_permission_token(&self, token: &str) -> Option<PendingPermissionDecision> {
        self.inner.pending_permissions.lock().ok()?.remove(token)
    }
}
