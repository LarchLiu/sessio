//! Telegram Bot API platform implementation.

use std::path::Path;
use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::agents::runtime::types::AgentAttachmentKind;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::{multipart, Client, ClientBuilder};
use serde::Deserialize;
use serde_json::{json, Value};

use super::super::attachments::{
    allocate_attachment_path, attachment_dir, download_to_file, guess_file_mime, guess_image_mime,
    InboundAttachment,
};
use super::super::config::TelegramConfig;
use super::super::router;
use super::super::state::{
    ChannelContext, ChatKey, ChatPermissionRequest, ChatSink, ChatStreamCapability, ChatStreamMode,
    ImBridgeState, PermissionResolutionOutcome,
};

const PLATFORM: &str = "telegram";
const DEFAULT_API_BASE: &str = "https://api.telegram.org";
const CALLBACK_PREFIX: &str = "sessio_perm:";
const ACTION_CALLBACK_PREFIX: &str = "sessio:";
const TELEGRAM_TEXT_LIMIT: usize = 3900;
const TELEGRAM_STREAM_LIMIT: usize = 3900;
const TELEGRAM_STREAM_INTERVAL: Duration = Duration::from_millis(650);
const TELEGRAM_PARSE_MODE: &str = "HTML";
static TELEGRAM_DRAFT_COUNTER: AtomicI64 = AtomicI64::new(1);

pub fn spawn(state: Arc<ImBridgeState>) {
    if let Err(error) = thread::Builder::new()
        .name("im-bridge-telegram".to_string())
        .spawn(move || poll_loop(state))
    {
        log::warn!("[im-bridge:telegram] failed to spawn worker: {error:#}");
    }
}

pub fn detect_user_ids(bot_token: &str, api_base: Option<&str>) -> Result<Vec<i64>> {
    let sink = TelegramSink::for_token(bot_token, api_base, 30)?;
    let updates = sink.get_updates(None, 0)?;
    let mut ids = Vec::new();
    for update in updates {
        if let Some(user_id) = update.user_id() {
            if !ids.contains(&user_id) {
                ids.push(user_id);
            }
        }
    }
    Ok(ids)
}

pub fn test_connection(bot_token: &str, api_base: Option<&str>) -> Result<()> {
    let sink = TelegramSink::for_token(bot_token, api_base, 30)?;
    sink.get_me().map(|_| ())
}

fn poll_loop(state: Arc<ImBridgeState>) {
    let mut offset: Option<i64> = None;
    let mut active_key: Option<TelegramConnectionKey> = None;
    let mut sink: Option<Arc<TelegramSink>> = None;

    loop {
        let bridge_config = state.config_snapshot();
        let Some(config) = bridge_config.telegram.clone() else {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                offset = None;
                log::info!("[im-bridge:telegram] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        if !bridge_config.enabled || !config.enabled || config.bot_token.trim().is_empty() {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                offset = None;
                log::info!("[im-bridge:telegram] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let next_key = TelegramConnectionKey::from_config(&config);
        if active_key.as_ref() != Some(&next_key) {
            match TelegramSink::new(config.clone()) {
                Ok(next_sink) => {
                    let next_sink = Arc::new(next_sink);
                    if let Err(error) = next_sink.set_commands() {
                        log::warn!("[im-bridge:telegram] failed to set bot commands: {error:#}");
                    }
                    state.register_sink(next_sink.clone());
                    sink = Some(next_sink);
                    active_key = Some(next_key);
                    offset = None;
                    log::info!("[im-bridge:telegram] connected");
                }
                Err(error) => {
                    state.unregister_sink(PLATFORM);
                    sink = None;
                    active_key = None;
                    offset = None;
                    log::warn!("[im-bridge:telegram] failed to initialize: {error:#}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }

        let Some(sink) = sink.as_ref().cloned() else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        match sink.get_updates(offset, config.poll_timeout_secs) {
            Ok(updates) => {
                for update in updates {
                    offset = Some(update.update_id + 1);
                    handle_update(&state, &sink, &config, update);
                }
            }
            Err(error) => {
                log::warn!("[im-bridge:telegram] polling failed: {error:#}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramConnectionKey {
    bot_token: String,
    api_base: Option<String>,
    poll_timeout_secs: u64,
}

impl TelegramConnectionKey {
    fn from_config(config: &TelegramConfig) -> Self {
        Self {
            bot_token: config.bot_token.trim().to_string(),
            api_base: config
                .api_base
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            poll_timeout_secs: config.poll_timeout_secs,
        }
    }
}

fn handle_update(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    config: &TelegramConfig,
    update: TelegramUpdate,
) {
    if let Some(callback) = update.callback_query {
        handle_callback(state, sink, config, callback);
        return;
    }

    let Some(message) = update.message else {
        return;
    };
    let text = message
        .text
        .as_deref()
        .or(message.caption.as_deref())
        .unwrap_or("");
    let Some(from) = message.from.as_ref() else {
        return;
    };
    if !is_allowed(config, from.id) {
        log::warn!(
            "[im-bridge:telegram] rejected message from unauthorized user {}",
            from.id
        );
        return;
    }

    let chat_id = message.chat.id.to_string();
    if is_duplicate_update(state, &chat_id, update.update_id) {
        log::info!(
            "[im-bridge:telegram] skipped duplicate update {} for chat {}",
            update.update_id,
            chat_id
        );
        return;
    }

    let key = ChatKey::new(PLATFORM, chat_id.clone());
    state.remember_channel_context(
        key.clone(),
        telegram_channel_context(&message, from, update.update_id),
    );
    if let Some(command_reply) = handle_interactive_command(state, sink, &key, text) {
        if let Err(error) = command_reply {
            if let Err(send_error) = sink.send_text(&chat_id, &format!("⚠️ {error:#}")) {
                log::warn!(
                    "[im-bridge:telegram] failed to send interactive command error: {send_error:#}"
                );
            }
        }
        record_update_activity(state, &chat_id, update.update_id);
        return;
    }
    let attachments = download_message_attachments(state, sink, &key, &message);
    let outcome = router::handle_message_with_attachments(state, &key, text, attachments);
    if let Some(reply) = outcome.reply {
        if let Err(error) = sink.send_text(&chat_id, &reply) {
            log::warn!("[im-bridge:telegram] failed to send command reply: {error:#}");
        }
    }
    record_update_activity(state, &chat_id, update.update_id);
}

/// Download photo/document attachments to the chat's workspace attachment
/// directory. Logs and skips entries that fail; the message still flows.
fn download_message_attachments(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
    message: &TelegramMessage,
) -> Vec<InboundAttachment> {
    let mut entries: Vec<(String, AgentAttachmentKind, Option<String>)> = Vec::new();
    if let Some(photo) = best_photo(&message.photo) {
        entries.push((photo.file_id.clone(), AgentAttachmentKind::Image, None));
    }
    if let Some(document) = message.document.as_ref() {
        let kind = document
            .mime_type
            .as_deref()
            .filter(|mime| mime.starts_with("image/"))
            .map(|_| AgentAttachmentKind::Image)
            .unwrap_or(AgentAttachmentKind::File);
        entries.push((document.file_id.clone(), kind, document.file_name.clone()));
    }
    if entries.is_empty() {
        return Vec::new();
    }
    let workspace = match state
        .chat_session(key)
        .map(|s| s.workspace_path)
        .or_else(|| {
            state
                .config_snapshot()
                .workspace_for_chat(key.platform, &key.chat_id)
                .map(str::to_string)
        }) {
        Some(workspace) => workspace,
        None => {
            log::warn!(
                "[im-bridge:telegram] dropping attachments for {}: no workspace bound",
                key.chat_id
            );
            return Vec::new();
        }
    };
    let dir = match attachment_dir(&workspace, key.platform, &key.chat_id) {
        Ok(dir) => dir,
        Err(error) => {
            log::warn!("[im-bridge:telegram] cannot prepare attachment dir: {error:#}");
            return Vec::new();
        }
    };
    let mut downloaded = Vec::new();
    for (file_id, kind, suggested_name) in entries {
        let file = match sink.get_file(&file_id) {
            Ok(file) => file,
            Err(error) => {
                log::warn!("[im-bridge:telegram] getFile failed for {file_id}: {error:#}");
                continue;
            }
        };
        let Some(path) = file.file_path else {
            log::warn!("[im-bridge:telegram] getFile response missing file_path");
            continue;
        };
        let display = suggested_name.clone().or_else(|| {
            Path::new(&path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        });
        let destination = allocate_attachment_path(&dir, display.as_deref());
        let url = sink.file_download_url(&path);
        if let Err(error) = download_to_file(&sink.client, &url, None, &destination) {
            log::warn!("[im-bridge:telegram] download {file_id} failed: {error:#}");
            continue;
        }
        downloaded.push(InboundAttachment {
            path: destination,
            kind,
            mime_type: None,
            display_name: display,
        });
    }
    downloaded
}

/// Pick the largest available photo size. Telegram delivers an array of
/// progressively higher-resolution variants.
fn best_photo(photos: &[TelegramPhotoSize]) -> Option<&TelegramPhotoSize> {
    photos
        .iter()
        .max_by_key(|photo| photo.file_size.unwrap_or(0))
}

fn record_update_activity(state: &Arc<ImBridgeState>, chat_id: &str, update_id: i64) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, chat_id, Some(update_id), now_ms())
    {
        log::warn!("[im-bridge:telegram] failed to record update id: {error:#}");
    }
}

fn handle_interactive_command(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
    text: &str,
) -> Option<Result<()>> {
    router::interactive_action_menu(state, key, text).map(|menu| {
        let menu = menu?;
        send_action_menu(sink, key, &menu)
    })
}

fn send_action_menu(
    sink: &Arc<TelegramSink>,
    key: &ChatKey,
    menu: &router::ActionMenu,
) -> Result<()> {
    let rows = menu
        .choices
        .iter()
        .map(|choice| {
            vec![json!({
                "text": choice.label,
                "callback_data": format!("{ACTION_CALLBACK_PREFIX}{}", choice.action),
            })]
        })
        .collect::<Vec<_>>();
    let rendered = telegram_markdown_to_html(&menu.text);
    sink.send_message(
        &key.chat_id,
        &rendered,
        Some(json!({ "inline_keyboard": rows })),
        Some(TELEGRAM_PARSE_MODE),
        None,
    )
}

fn telegram_channel_context(
    message: &TelegramMessage,
    from: &TelegramUser,
    update_id: i64,
) -> ChannelContext {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "telegramChat".to_string(),
        json!({
            "id": message.chat.id,
            "type": message.chat.kind.as_deref(),
            "title": message.chat.title.as_deref(),
            "username": message.chat.username.as_deref(),
            "firstName": message.chat.first_name.as_deref(),
            "lastName": message.chat.last_name.as_deref(),
        }),
    );
    if let Some(message_thread_id) = message.message_thread_id {
        metadata.insert("messageThreadId".to_string(), json!(message_thread_id));
    }
    metadata.insert(
        "telegramUser".to_string(),
        json!({
            "id": from.id,
            "username": from.username.as_deref(),
            "firstName": from.first_name.as_deref(),
            "lastName": from.last_name.as_deref(),
        }),
    );
    ChannelContext {
        channel_type: message.chat.kind.clone(),
        user_id: Some(from.id.to_string()),
        team_id: None,
        thread_id: message.message_thread_id.map(|value| value.to_string()),
        display_name: telegram_chat_display_name(&message.chat)
            .or_else(|| telegram_user_display_name(from)),
        metadata,
        last_update_id: Some(update_id),
    }
}

fn telegram_chat_display_name(chat: &TelegramChat) -> Option<String> {
    trimmed(&chat.title)
        .map(str::to_string)
        .or_else(|| person_display_name(&chat.first_name, &chat.last_name, &chat.username))
}

fn telegram_user_display_name(user: &TelegramUser) -> Option<String> {
    person_display_name(&user.first_name, &user.last_name, &user.username)
}

fn person_display_name(
    first_name: &Option<String>,
    last_name: &Option<String>,
    username: &Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(first_name) = trimmed(first_name) {
        parts.push(first_name);
    }
    if let Some(last_name) = trimmed(last_name) {
        parts.push(last_name);
    }
    if !parts.is_empty() {
        return Some(parts.join(" "));
    }
    trimmed(username).map(|value| format!("@{}", value.trim_start_matches('@')))
}

fn trimmed(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn is_duplicate_update(state: &Arc<ImBridgeState>, chat_id: &str, update_id: i64) -> bool {
    match state.store.get_active_channel_session(PLATFORM, chat_id) {
        Ok(Some(record)) => record
            .last_update_id
            .map(|last_update_id| update_id <= last_update_id)
            .unwrap_or(false),
        Ok(None) => false,
        Err(error) => {
            log::warn!("[im-bridge:telegram] failed to read last update id: {error:#}");
            false
        }
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn handle_callback(
    state: &Arc<ImBridgeState>,
    sink: &Arc<TelegramSink>,
    config: &TelegramConfig,
    callback: TelegramCallbackQuery,
) {
    if !is_allowed(config, callback.from.id) {
        let _ = sink.answer_callback_query(&callback.id, "Not authorized");
        return;
    }
    let Some(data) = callback.data.as_deref() else {
        let _ = sink.answer_callback_query(&callback.id, "Missing callback data");
        return;
    };
    if let Some(action) = data.strip_prefix(ACTION_CALLBACK_PREFIX) {
        match callback.message.as_ref() {
            Some(message) => {
                let chat_id = message.chat.id.to_string();
                let key = ChatKey::new(PLATFORM, chat_id.clone());
                state.touch_chat(&key);
                record_chat_activity(state, &chat_id);
                match router::handle_action_callback(state, &key, action) {
                    Ok(reply) => {
                        let _ = sink.answer_callback_query(&callback.id, &reply);
                        if let Err(error) = sink.delete_message(&chat_id, message.message_id) {
                            log::debug!(
                                "[im-bridge:telegram] failed to delete action menu; falling back to edit: {error:#}"
                            );
                            let rendered = telegram_markdown_to_html(&reply);
                            if let Err(edit_error) = sink.edit_message_text(
                                &chat_id,
                                message.message_id,
                                &rendered,
                                Some(TELEGRAM_PARSE_MODE),
                                None,
                            ) {
                                log::debug!(
                                    "[im-bridge:telegram] failed to clear action menu: {edit_error:#}"
                                );
                            }
                        }
                    }
                    Err(error) => {
                        let _ = sink.answer_callback_query(&callback.id, "Failed");
                        if let Err(send_error) = sink.send_text(&chat_id, &format!("⚠️ {error:#}"))
                        {
                            log::warn!(
                                "[im-bridge:telegram] failed to send action callback error: {send_error:#}"
                            );
                        }
                    }
                }
            }
            None => {
                let _ = sink.answer_callback_query(&callback.id, "Missing chat");
            }
        }
        return;
    }
    let Some(token) = data.strip_prefix(CALLBACK_PREFIX) else {
        return;
    };
    let Some(decision) = state.take_permission_token(token) else {
        let _ = sink.answer_callback_query(&callback.id, "Permission request expired");
        return;
    };
    if let Some(chat_id) = callback
        .message
        .as_ref()
        .map(|message| message.chat.id.to_string())
    {
        record_chat_activity(state, &chat_id);
        state.touch_chat(&ChatKey::new(PLATFORM, chat_id));
    }
    match state.runtime.respond_permission(
        &decision.sessio_runtime_session_id,
        &decision.request_id,
        decision.option_id,
    ) {
        Ok(()) => {
            let _ = sink.answer_callback_query(&callback.id, "Recorded");
        }
        Err(error) => {
            let _ = sink.answer_callback_query(&callback.id, "Failed");
            log::warn!("[im-bridge:telegram] permission response failed: {error:#}");
        }
    }
}

fn record_chat_activity(state: &Arc<ImBridgeState>, chat_id: &str) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, chat_id, None, now_ms())
    {
        log::warn!("[im-bridge:telegram] failed to record chat activity: {error:#}");
    }
}

fn is_allowed(config: &TelegramConfig, user_id: i64) -> bool {
    config.allowed_user_ids.contains(&user_id)
}

pub struct TelegramSink {
    client: Client,
    api_base: String,
    bot_token: String,
}

impl TelegramSink {
    fn new(config: TelegramConfig) -> Result<Self> {
        Self::for_token(
            &config.bot_token,
            config.api_base.as_deref(),
            config.poll_timeout_secs,
        )
    }

    fn for_token(bot_token: &str, api_base: Option<&str>, poll_timeout_secs: u64) -> Result<Self> {
        let bot_token = bot_token.trim();
        if bot_token.is_empty() {
            bail!("Telegram bot token is required");
        }
        let timeout = Duration::from_secs(poll_timeout_secs.max(5) + 20);
        let client = ClientBuilder::new()
            .timeout(timeout)
            .build()
            .context("build Telegram HTTP client")?;
        Ok(Self {
            client,
            api_base: api_base
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_API_BASE)
                .trim_end_matches('/')
                .to_string(),
            bot_token: bot_token.to_string(),
        })
    }

    fn endpoint(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api_base, self.bot_token, method)
    }

    fn get_updates(&self, offset: Option<i64>, timeout_secs: u64) -> Result<Vec<TelegramUpdate>> {
        let body = json!({
            "offset": offset,
            "timeout": timeout_secs,
            "allowed_updates": ["message", "callback_query"],
        });
        let response: TelegramApiResponse<Vec<TelegramUpdate>> =
            self.post_json("getUpdates", &body)?;
        response.into_result()
    }

    fn get_me(&self) -> Result<Value> {
        let response: TelegramApiResponse<Value> = self.post_json("getMe", &json!({}))?;
        response.into_result()
    }

    fn set_commands(&self) -> Result<()> {
        let body = json!({
            "commands": [
                { "command": "new", "description": "Start a new Sessio session" },
                { "command": "agent", "description": "Choose agent" },
                { "command": "model", "description": "Choose model for current session" },
                { "command": "effort", "description": "Choose effort for current session" },
                { "command": "workspace", "description": "Choose workspace for current session" },
                { "command": "status", "description": "Show current Sessio session" },
                { "command": "cancel", "description": "Cancel current turn" },
                { "command": "end", "description": "End current IM session" },
                { "command": "help", "description": "Show help" }
            ]
        });
        let response: TelegramApiResponse<Value> = self.post_json("setMyCommands", &body)?;
        response.into_result().map(|_| ())
    }

    fn send_message(
        &self,
        chat_id: &str,
        text: &str,
        reply_markup: Option<Value>,
        parse_mode: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        apply_telegram_context(&mut body, context);
        if let Some(parse_mode) = parse_mode {
            body["parse_mode"] = Value::String(parse_mode.to_string());
        }
        if let Some(reply_markup) = reply_markup {
            body["reply_markup"] = reply_markup;
        }
        let response: TelegramApiResponse<Value> = self.post_json("sendMessage", &body)?;
        response.into_result().map(|_| ())
    }

    /// Send a message and return the resulting `message_id`. Used when we may
    /// need to edit the message later (e.g. permission prompts).
    fn send_message_with_id(
        &self,
        chat_id: &str,
        text: &str,
        reply_markup: Option<Value>,
        parse_mode: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<i64> {
        let mut body = json!({
            "chat_id": chat_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        apply_telegram_context(&mut body, context);
        if let Some(parse_mode) = parse_mode {
            body["parse_mode"] = Value::String(parse_mode.to_string());
        }
        if let Some(reply_markup) = reply_markup {
            body["reply_markup"] = reply_markup;
        }
        let response: TelegramApiResponse<Value> = self.post_json("sendMessage", &body)?;
        let value = response.into_result()?;
        value
            .get("message_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("Telegram sendMessage response missing message_id"))
    }

    fn edit_message_text(
        &self,
        chat_id: &str,
        message_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let mut body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "disable_web_page_preview": true,
        });
        apply_telegram_context(&mut body, context);
        if let Some(parse_mode) = parse_mode {
            body["parse_mode"] = Value::String(parse_mode.to_string());
        }
        let response: TelegramApiResponse<Value> = self.post_json("editMessageText", &body)?;
        response.into_result().map(|_| ())
    }

    fn delete_message(&self, chat_id: &str, message_id: i64) -> Result<()> {
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });
        let response: TelegramApiResponse<bool> = self.post_json("deleteMessage", &body)?;
        response.into_result().map(|_| ())
    }

    fn send_message_draft(
        &self,
        chat_id: &str,
        draft_id: i64,
        text: &str,
        parse_mode: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let mut body = json!({
            "chat_id": chat_id,
            "draft_id": draft_id,
            "text": text,
        });
        apply_telegram_context(&mut body, context);
        if let Some(parse_mode) = parse_mode {
            body["parse_mode"] = Value::String(parse_mode.to_string());
        }
        let response: TelegramApiResponse<Value> = self.post_json("sendMessageDraft", &body)?;
        response.into_result().map(|_| ())
    }

    fn send_chat_action(
        &self,
        chat_id: &str,
        action: &str,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let mut body = json!({
            "chat_id": chat_id,
            "action": action,
        });
        apply_telegram_context(&mut body, context);
        let response: TelegramApiResponse<Value> = self.post_json("sendChatAction", &body)?;
        response.into_result().map(|_| ())
    }

    fn answer_callback_query(&self, callback_query_id: &str, text: &str) -> Result<()> {
        let body = json!({
            "callback_query_id": callback_query_id,
            "text": text,
            "show_alert": false,
        });
        let response: TelegramApiResponse<Value> = self.post_json("answerCallbackQuery", &body)?;
        response.into_result().map(|_| ())
    }

    fn get_file(&self, file_id: &str) -> Result<TelegramFile> {
        let body = json!({ "file_id": file_id });
        let response: TelegramApiResponse<TelegramFile> = self.post_json("getFile", &body)?;
        response.into_result()
    }

    fn file_download_url(&self, file_path: &str) -> String {
        format!("{}/file/bot{}/{}", self.api_base, self.bot_token, file_path)
    }

    fn send_photo(
        &self,
        chat_id: &str,
        path: &Path,
        caption: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("photo")
            .to_string();
        let mime = guess_image_mime(path);
        let bytes =
            std::fs::read(path).with_context(|| format!("read image {}", path.display()))?;
        let part = multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str(mime)
            .context("set image MIME type")?;
        let mut form = multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("photo", part);
        if let Some(message_thread_id) = telegram_message_thread_id(context) {
            form = form.text("message_thread_id", message_thread_id.to_string());
        }
        if let Some(caption) = caption {
            if !caption.trim().is_empty() {
                form = form.text("caption", caption.to_string());
            }
        }
        let response: TelegramApiResponse<Value> = self
            .client
            .post(self.endpoint("sendPhoto"))
            .multipart(form)
            .send()
            .with_context(|| "Telegram sendPhoto request failed")?
            .error_for_status()
            .with_context(|| "Telegram sendPhoto returned HTTP error")?
            .json()
            .with_context(|| "parse Telegram sendPhoto response")?;
        response.into_result().map(|_| ())
    }

    fn send_document(
        &self,
        chat_id: &str,
        path: &Path,
        caption: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        let mime = guess_file_mime(path);
        let bytes = std::fs::read(path).with_context(|| format!("read file {}", path.display()))?;
        let part = multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str(mime)
            .context("set document MIME type")?;
        let mut form = multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);
        if let Some(message_thread_id) = telegram_message_thread_id(context) {
            form = form.text("message_thread_id", message_thread_id.to_string());
        }
        if let Some(caption) = caption {
            if !caption.trim().is_empty() {
                form = form.text("caption", caption.to_string());
            }
        }
        let response: TelegramApiResponse<Value> = self
            .client
            .post(self.endpoint("sendDocument"))
            .multipart(form)
            .send()
            .with_context(|| "Telegram sendDocument request failed")?
            .error_for_status()
            .with_context(|| "Telegram sendDocument returned HTTP error")?
            .json()
            .with_context(|| "parse Telegram sendDocument response")?;
        response.into_result().map(|_| ())
    }

    fn post_json<T: for<'de> Deserialize<'de>>(&self, method: &str, body: &Value) -> Result<T> {
        self.client
            .post(self.endpoint(method))
            .json(body)
            .send()
            .with_context(|| format!("Telegram {method} request failed"))?
            .error_for_status()
            .with_context(|| format!("Telegram {method} returned HTTP error"))?
            .json::<T>()
            .with_context(|| format!("parse Telegram {method} response"))
    }
}

impl ChatSink for TelegramSink {
    fn platform(&self) -> &'static str {
        PLATFORM
    }

    fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        self.send_text_with_context(chat_id, text, None)
    }

    fn send_text_with_context(
        &self,
        chat_id: &str,
        text: &str,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        for chunk in split_telegram_text(text) {
            let rendered = telegram_markdown_to_html(&chunk);
            self.send_message(chat_id, &rendered, None, Some(TELEGRAM_PARSE_MODE), context)?;
        }
        Ok(())
    }

    fn stream_capability(&self) -> Option<ChatStreamCapability> {
        Some(ChatStreamCapability {
            mode: ChatStreamMode::Draft,
            min_interval: TELEGRAM_STREAM_INTERVAL,
            max_chars: TELEGRAM_STREAM_LIMIT,
        })
    }

    fn stream_capability_with_context(
        &self,
        context: Option<&ChannelContext>,
    ) -> Option<ChatStreamCapability> {
        if context
            .and_then(telegram_chat_type)
            .is_some_and(|kind| kind != "private")
        {
            return None;
        }
        self.stream_capability()
    }

    fn start_stream_reply_with_context(
        &self,
        chat_id: &str,
        text: &str,
        context: Option<&ChannelContext>,
    ) -> Result<Value> {
        let draft_id = telegram_draft_id();
        let rendered = telegram_markdown_to_html(text);
        self.send_message_draft(
            chat_id,
            draft_id,
            &rendered,
            Some(TELEGRAM_PARSE_MODE),
            context,
        )?;
        Ok(json!({ "draft_id": draft_id }))
    }

    fn update_stream_reply_with_context(
        &self,
        chat_id: &str,
        message_ref: &Value,
        text: &str,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        let Some(draft_id) = message_ref.get("draft_id").and_then(Value::as_i64) else {
            return Ok(());
        };
        let rendered = telegram_markdown_to_html(text);
        self.send_message_draft(
            chat_id,
            draft_id,
            &rendered,
            Some(TELEGRAM_PARSE_MODE),
            context,
        )
    }

    fn finish_stream_reply_with_context(
        &self,
        chat_id: &str,
        message_ref: &Value,
        text: &str,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        self.update_stream_reply_with_context(chat_id, message_ref, text, context)
    }

    fn send_image(&self, chat_id: &str, path: &Path, caption: Option<&str>) -> Result<()> {
        self.send_image_with_context(chat_id, path, caption, None)
    }

    fn send_image_with_context(
        &self,
        chat_id: &str,
        path: &Path,
        caption: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        self.send_photo(chat_id, path, caption, context)
    }

    fn send_file(&self, chat_id: &str, path: &Path, caption: Option<&str>) -> Result<()> {
        self.send_file_with_context(chat_id, path, caption, None)
    }

    fn send_file_with_context(
        &self,
        chat_id: &str,
        path: &Path,
        caption: Option<&str>,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        self.send_document(chat_id, path, caption, context)
    }

    fn supports_images(&self) -> bool {
        true
    }

    fn supports_files(&self) -> bool {
        true
    }

    fn send_permission_request(
        &self,
        chat_id: &str,
        request: &ChatPermissionRequest,
    ) -> Result<Option<Value>> {
        self.send_permission_request_with_context(chat_id, request, None)
    }

    fn send_permission_request_with_context(
        &self,
        chat_id: &str,
        request: &ChatPermissionRequest,
        context: Option<&ChannelContext>,
    ) -> Result<Option<Value>> {
        let text = format_permission_text(request);
        let buttons: Vec<Value> = request
            .options
            .iter()
            .map(|option| {
                json!({
                    "text": option.label,
                    "callback_data": format!("{}{}", CALLBACK_PREFIX, option.token),
                })
            })
            .collect();
        let reply_markup = if buttons.is_empty() {
            None
        } else {
            Some(json!({ "inline_keyboard": [buttons] }))
        };
        let message_id = self.send_message_with_id(chat_id, &text, reply_markup, None, context)?;
        Ok(Some(json!({ "message_id": message_id })))
    }

    fn resolve_permission_message(
        &self,
        chat_id: &str,
        message_ref: &Value,
        request: &ChatPermissionRequest,
        outcome: PermissionResolutionOutcome<'_>,
    ) -> Result<()> {
        let Some(message_id) = message_ref.get("message_id").and_then(Value::as_i64) else {
            return Ok(());
        };
        if let Err(error) = self.delete_message(chat_id, message_id) {
            log::debug!(
                "[im-bridge:telegram] failed to delete permission message; falling back to edit: {error:#}"
            );
        } else {
            return Ok(());
        }
        let mut text = format_permission_text(request);
        text.push_str("\n\n");
        text.push_str(&format_permission_outcome(outcome));
        // If deleteMessage is unavailable for this message, editMessageText
        // with no reply_markup still drops the inline keyboard.
        self.edit_message_text(chat_id, message_id, &text, None, None)
    }

    fn send_typing(&self, chat_id: &str) -> Result<()> {
        self.send_typing_with_context(chat_id, None)
    }

    fn send_typing_with_context(
        &self,
        chat_id: &str,
        context: Option<&ChannelContext>,
    ) -> Result<()> {
        self.send_chat_action(chat_id, "typing", context)
    }
}

fn format_permission_text(request: &ChatPermissionRequest) -> String {
    format!("Permission requested\nTool: {}", request.tool_name)
}

fn format_permission_outcome(outcome: PermissionResolutionOutcome<'_>) -> String {
    let marker = if outcome.approved { "✅" } else { "❌" };
    match outcome.label {
        Some(label) => format!("{marker} {label}"),
        None if outcome.approved => format!("{marker} Allowed"),
        None => format!("{marker} Rejected"),
    }
}

fn apply_telegram_context(body: &mut Value, context: Option<&ChannelContext>) {
    if let Some(message_thread_id) = telegram_message_thread_id(context) {
        body["message_thread_id"] = json!(message_thread_id);
    }
}

fn telegram_message_thread_id(context: Option<&ChannelContext>) -> Option<i64> {
    context.and_then(|context| {
        context
            .metadata
            .get("messageThreadId")
            .and_then(Value::as_i64)
            .or_else(|| context.thread_id.as_deref()?.parse::<i64>().ok())
    })
}

fn telegram_chat_type(context: &ChannelContext) -> Option<&str> {
    context
        .metadata
        .get("telegramChat")
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .or(context.channel_type.as_deref())
}

fn telegram_draft_id() -> i64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    let counter = TELEGRAM_DRAFT_COUNTER.fetch_add(1, Ordering::Relaxed) % 1000;
    (millis % 1_000_000_000) * 1000 + counter
}

fn split_telegram_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= TELEGRAM_TEXT_LIMIT {
            chunks.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn telegram_markdown_to_html(text: &str) -> String {
    let mut out = String::new();
    let mut lines = text.split('\n').peekable();
    let mut in_code = false;
    let mut code = String::new();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if in_code {
                push_code_block(&mut out, &code);
                code.clear();
                in_code = false;
            } else {
                if !out.is_empty() {
                    out.push('\n');
                }
                in_code = true;
            }
            continue;
        }

        if in_code {
            code.push_str(line);
            if lines.peek().is_some() {
                code.push('\n');
            }
            continue;
        }

        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&render_markdown_line(line));
    }

    if in_code {
        push_code_block(&mut out, &code);
    }
    out
}

fn push_code_block(out: &mut String, code: &str) {
    out.push_str("<pre><code>");
    out.push_str(&escape_telegram_html(code.trim_matches('\n')));
    out.push_str("</code></pre>");
}

fn render_markdown_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(heading) = markdown_heading_text(trimmed) {
        return format!("<b>{}</b>", render_inline_markdown(heading.trim()));
    }
    if let Some(quote) = trimmed.strip_prefix("> ") {
        return format!("<blockquote>{}</blockquote>", render_inline_markdown(quote));
    }
    render_inline_markdown(line)
}

fn markdown_heading_text(trimmed: &str) -> Option<&str> {
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && trimmed.chars().nth(hashes) == Some(' ') {
        Some(&trimmed[hashes + 1..])
    } else {
        None
    }
}

fn render_inline_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let rest = &text[cursor..];
        if let Some(stripped) = rest.strip_prefix('`') {
            if let Some(end) = stripped.find('`') {
                let inner = &stripped[..end];
                out.push_str("<code>");
                out.push_str(&escape_telegram_html(inner));
                out.push_str("</code>");
                cursor += end + 2;
                continue;
            }
        }

        if rest.starts_with('[') {
            if let Some((label, url, consumed)) = parse_markdown_link(rest) {
                if is_safe_telegram_link(url) {
                    out.push_str("<a href=\"");
                    out.push_str(&escape_telegram_html_attr(url));
                    out.push_str("\">");
                    out.push_str(&escape_telegram_html(label));
                    out.push_str("</a>");
                    cursor += consumed;
                    continue;
                }
            }
        }

        if let Some((tag, inner, consumed)) = parse_inline_span(rest) {
            out.push('<');
            out.push_str(tag);
            out.push('>');
            out.push_str(&render_inline_markdown(inner));
            out.push_str("</");
            out.push_str(tag);
            out.push('>');
            cursor += consumed;
            continue;
        }

        let ch = rest.chars().next().expect("cursor is in bounds");
        escape_telegram_html_char(ch, &mut out);
        cursor += ch.len_utf8();
    }
    out
}

fn parse_markdown_link(rest: &str) -> Option<(&str, &str, usize)> {
    let close_label = rest.find("](")?;
    if close_label == 1 {
        return None;
    }
    let after_open = close_label + 2;
    let close_url = rest[after_open..].find(')')? + after_open;
    let label = &rest[1..close_label];
    let url = rest[after_open..close_url].trim();
    if label.trim().is_empty() || url.is_empty() {
        return None;
    }
    Some((label, url, close_url + 1))
}

fn parse_inline_span(rest: &str) -> Option<(&'static str, &str, usize)> {
    for (marker, tag) in [("**", "b"), ("~~", "s"), ("*", "i")] {
        if !rest.starts_with(marker) {
            continue;
        }
        let inner_start = marker.len();
        if rest[inner_start..]
            .chars()
            .next()
            .map(char::is_whitespace)
            .unwrap_or(true)
        {
            continue;
        }
        let Some(close_offset) = rest[inner_start..].find(marker) else {
            continue;
        };
        let close = inner_start + close_offset;
        let inner = &rest[inner_start..close];
        if inner
            .chars()
            .last()
            .map(char::is_whitespace)
            .unwrap_or(true)
        {
            continue;
        }
        return Some((tag, inner, close + marker.len()));
    }
    None
}

fn is_safe_telegram_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("tg://")
        || lower.starts_with("mailto:")
}

fn escape_telegram_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        escape_telegram_html_char(ch, &mut out);
    }
    out
}

fn escape_telegram_html_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            _ => escape_telegram_html_char(ch, &mut out),
        }
    }
    out
}

fn escape_telegram_html_char(ch: char, out: &mut String) {
    match ch {
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '&' => out.push_str("&amp;"),
        _ => out.push(ch),
    }
}

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

impl<T> TelegramApiResponse<T> {
    fn into_result(self) -> Result<T> {
        if self.ok {
            self.result
                .ok_or_else(|| anyhow!("Telegram response missing result"))
        } else {
            bail!(
                "{}",
                self.description
                    .unwrap_or_else(|| "Telegram API returned ok=false".to_string())
            )
        }
    }
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    callback_query: Option<TelegramCallbackQuery>,
}

impl TelegramUpdate {
    fn user_id(&self) -> Option<i64> {
        self.message
            .as_ref()
            .and_then(|message| message.from.as_ref())
            .map(|user| user.id)
            .or_else(|| self.callback_query.as_ref().map(|query| query.from.id))
    }
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    chat: TelegramChat,
    from: Option<TelegramUser>,
    #[serde(default)]
    message_thread_id: Option<i64>,
    text: Option<String>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    photo: Vec<TelegramPhotoSize>,
    #[serde(default)]
    document: Option<TelegramDocument>,
}

#[derive(Debug, Deserialize)]
struct TelegramPhotoSize {
    file_id: String,
    #[serde(default)]
    file_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TelegramDocument {
    file_id: String,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramFile {
    #[serde(default)]
    file_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
    #[serde(rename = "type")]
    kind: Option<String>,
    title: Option<String>,
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    username: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackQuery {
    id: String,
    from: TelegramUser,
    message: Option<TelegramCallbackMessage>,
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackMessage {
    message_id: i64,
    chat: TelegramChat,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_to_telegram_html_renders_safe_subset() {
        let input = "# Title\n\n**bold** *em* `x < y` [link](https://example.com?a=1&b=2)\n\n```rust\nlet x = 1 < 2;\n```";
        let rendered = telegram_markdown_to_html(input);

        assert!(rendered.contains("<b>Title</b>"));
        assert!(rendered.contains("<b>bold</b> <i>em</i> <code>x &lt; y</code>"));
        assert!(rendered.contains("<a href=\"https://example.com?a=1&amp;b=2\">link</a>"));
        assert!(rendered.contains("<pre><code>let x = 1 &lt; 2;</code></pre>"));
    }

    #[test]
    fn markdown_to_telegram_html_escapes_unsafe_links() {
        let rendered = telegram_markdown_to_html("[bad](javascript:alert(1)) <tag>");

        assert!(!rendered.contains("<a "));
        assert!(rendered.contains("[bad](javascript:alert(1)) &lt;tag&gt;"));
    }

    #[test]
    fn telegram_context_preserves_forum_topic_id() {
        let message = TelegramMessage {
            chat: TelegramChat {
                id: -100,
                kind: Some("supergroup".to_string()),
                title: Some("Group".to_string()),
                username: None,
                first_name: None,
                last_name: None,
            },
            from: Some(TelegramUser {
                id: 42,
                username: Some("alex".to_string()),
                first_name: Some("Alex".to_string()),
                last_name: None,
            }),
            message_thread_id: Some(67890),
            text: Some("hello".to_string()),
            caption: None,
            photo: Vec::new(),
            document: None,
        };
        let context = telegram_channel_context(&message, message.from.as_ref().unwrap(), 123);

        assert_eq!(context.thread_id.as_deref(), Some("67890"));
        assert_eq!(telegram_message_thread_id(Some(&context)), Some(67890));
        assert_eq!(telegram_chat_type(&context), Some("supergroup"));
    }
}
