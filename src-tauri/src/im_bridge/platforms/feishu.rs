//! Feishu/Lark WebSocket platform implementation.
//!
//! Feishu's long-connection mode uses a compact protobuf frame rather than a
//! JSON WebSocket. This module implements only the small pbbp2 Frame subset the
//! official SDK uses for receiving event callbacks and acknowledging them.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::{multipart, Client, ClientBuilder};
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

use crate::agents::runtime::types::AgentAttachmentKind;

use super::super::attachments::{
    allocate_attachment_path, attachment_dir, guess_file_mime, guess_image_mime, InboundAttachment,
};
use super::super::config::FeishuConfig;
use super::super::router;
use super::super::state::{
    ChannelContext, ChatKey, ChatPermissionRequest, ChatSink, ImBridgeState,
    PermissionResolutionOutcome,
};

const PLATFORM: &str = "feishu";
const DEFAULT_DOMAIN: &str = "https://open.feishu.cn";
const FEISHU_TEXT_LIMIT: usize = 8000;
const CALLBACK_PREFIX: &str = "sessio_perm:";
const ACTION_CALLBACK_PREFIX: &str = "sessio:";
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(120);

const FRAME_CONTROL: i32 = 0;
const FRAME_DATA: i32 = 1;
const HEADER_TYPE: &str = "type";
const HEADER_MESSAGE_ID: &str = "message_id";
const HEADER_SUM: &str = "sum";
const HEADER_SEQ: &str = "seq";
const HEADER_TRACE_ID: &str = "trace_id";
const HEADER_BIZ_RT: &str = "biz_rt";
const MESSAGE_EVENT: &str = "event";
const MESSAGE_CARD: &str = "card";
const MESSAGE_PING: &str = "ping";
const MESSAGE_PONG: &str = "pong";

pub fn spawn(state: Arc<ImBridgeState>) {
    if let Err(error) = thread::Builder::new()
        .name("im-bridge-feishu".to_string())
        .spawn(move || ws_loop(state))
    {
        log::warn!("[im-bridge:feishu] failed to spawn worker: {error:#}");
    }
}

pub fn test_connection(app_id: &str, app_secret: &str, domain: Option<&str>) -> Result<()> {
    let sink = FeishuSink::for_credentials(app_id, app_secret, domain)?;
    sink.tenant_access_token()?;
    sink.pull_ws_config().map(|_| ())
}

fn ws_loop(state: Arc<ImBridgeState>) {
    let mut active_key: Option<FeishuConnectionKey> = None;
    let mut sink: Option<Arc<FeishuSink>> = None;

    loop {
        let bridge_config = state.config_snapshot();
        let Some(config) = bridge_config.feishu.clone() else {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                log::info!("[im-bridge:feishu] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        if !bridge_config.enabled
            || !config.enabled
            || config.app_id.trim().is_empty()
            || config.app_secret.trim().is_empty()
        {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                log::info!("[im-bridge:feishu] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let next_key = FeishuConnectionKey::from_config(&config);
        if active_key.as_ref() != Some(&next_key) {
            match FeishuSink::new(config.clone()) {
                Ok(next_sink) => {
                    let next_sink = Arc::new(next_sink);
                    state.register_sink(next_sink.clone());
                    sink = Some(next_sink);
                    active_key = Some(next_key);
                    log::info!("[im-bridge:feishu] configured");
                }
                Err(error) => {
                    state.unregister_sink(PLATFORM);
                    sink = None;
                    active_key = None;
                    log::warn!("[im-bridge:feishu] failed to initialize: {error:#}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }

        let Some(sink) = sink.as_ref().cloned() else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        let run_config = state.config_snapshot().feishu.unwrap_or_default();
        match run_ws_once(&state, &sink, &run_config) {
            Ok(()) => thread::sleep(Duration::from_millis(500)),
            Err(error) => {
                log::warn!("[im-bridge:feishu] websocket failed: {error:#}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FeishuConnectionKey {
    app_id: String,
    app_secret: String,
    domain: Option<String>,
}

impl FeishuConnectionKey {
    fn from_config(config: &FeishuConfig) -> Self {
        Self {
            app_id: config.app_id.trim().to_string(),
            app_secret: config.app_secret.trim().to_string(),
            domain: normalized_optional(&config.domain),
        }
    }
}

fn run_ws_once(
    state: &Arc<ImBridgeState>,
    sink: &Arc<FeishuSink>,
    config: &FeishuConfig,
) -> Result<()> {
    let mut ws_config = sink
        .pull_ws_config()
        .context("pull Feishu WebSocket config")?;
    let mut request = ws_config.url.clone().into_client_request()?;
    request
        .headers_mut()
        .insert("User-Agent", "sessio-im-bridge/1.0".parse()?);
    let (mut socket, _) = connect(request).context("connect Feishu WebSocket")?;
    set_socket_read_timeout(&mut socket, Some(Duration::from_secs(5)));
    log::info!("[im-bridge:feishu] connected");

    send_ping(&mut socket, ws_config.service_id)?;
    let mut last_ping = Instant::now();
    let mut fragments = FragmentCache::default();

    loop {
        let bridge_config = state.config_snapshot();
        let Some(current_config) = bridge_config.feishu.as_ref() else {
            return Ok(());
        };
        if !bridge_config.enabled || !current_config.enabled {
            return Ok(());
        }
        if current_config.app_id.trim() != config.app_id.trim()
            || current_config.app_secret.trim() != config.app_secret.trim()
            || normalized_optional(&current_config.domain) != normalized_optional(&config.domain)
        {
            return Ok(());
        }

        if last_ping.elapsed() >= ws_config.ping_interval {
            send_ping(&mut socket, ws_config.service_id)?;
            last_ping = Instant::now();
        }

        match socket.read() {
            Ok(Message::Binary(bytes)) => {
                let frame = PbbpFrame::decode(&bytes).context("decode Feishu frame")?;
                match frame.method {
                    FRAME_CONTROL => handle_control_frame(&frame, &mut ws_config)?,
                    FRAME_DATA => {
                        handle_data_frame(state, sink, &mut socket, &mut fragments, frame)?
                    }
                    _ => {}
                }
            }
            Ok(Message::Ping(payload)) => {
                socket.write(Message::Pong(payload))?;
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

fn set_socket_read_timeout(
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    timeout: Option<Duration>,
) {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(timeout);
        }
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.get_mut().set_read_timeout(timeout);
        }
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

fn send_ping(
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    service_id: i32,
) -> Result<()> {
    let frame = PbbpFrame {
        seq_id: 0,
        log_id: 0,
        service: service_id,
        method: FRAME_CONTROL,
        headers: vec![PbbpHeader::new(HEADER_TYPE, MESSAGE_PING)],
        payload_encoding: String::new(),
        payload_type: String::new(),
        payload: Vec::new(),
        log_id_new: String::new(),
    };
    socket.write(Message::Binary(frame.encode()))?;
    Ok(())
}

fn handle_control_frame(frame: &PbbpFrame, ws_config: &mut FeishuWsConfig) -> Result<()> {
    if header_value(&frame.headers, HEADER_TYPE) != Some(MESSAGE_PONG) || frame.payload.is_empty() {
        return Ok(());
    }
    let value: Value =
        serde_json::from_slice(&frame.payload).context("parse Feishu pong payload")?;
    if let Some(seconds) = value.get("PingInterval").and_then(Value::as_u64) {
        ws_config.ping_interval = Duration::from_secs(seconds.max(1));
    }
    Ok(())
}

fn handle_data_frame(
    state: &Arc<ImBridgeState>,
    sink: &Arc<FeishuSink>,
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    fragments: &mut FragmentCache,
    frame: PbbpFrame,
) -> Result<()> {
    let headers = frame.headers.clone();
    let Some(message_type) = header_value(&headers, HEADER_TYPE) else {
        return Ok(());
    };
    if !matches!(message_type, MESSAGE_EVENT | MESSAGE_CARD) {
        return Ok(());
    }

    let start = Instant::now();
    let response_code = match fragments.merge(&headers, &frame.payload) {
        Ok(Some(value)) => match handle_feishu_payload(state, sink, message_type, value) {
            Ok(()) => 200,
            Err(error) => {
                log::warn!("[im-bridge:feishu] event handling failed: {error:#}");
                500
            }
        },
        Ok(None) => return Ok(()),
        Err(error) => {
            log::warn!("[im-bridge:feishu] failed to merge frame fragments: {error:#}");
            500
        }
    };

    let mut response = frame;
    response.headers.push(PbbpHeader::new(
        HEADER_BIZ_RT,
        start.elapsed().as_millis().to_string(),
    ));
    response.payload = serde_json::to_vec(&json!({ "code": response_code }))?;
    socket.write(Message::Binary(response.encode()))?;
    Ok(())
}

fn handle_feishu_payload(
    state: &Arc<ImBridgeState>,
    sink: &Arc<FeishuSink>,
    message_type: &str,
    value: Value,
) -> Result<()> {
    let event_type = value
        .get("header")
        .and_then(|header| header.get("event_type"))
        .and_then(Value::as_str)
        .or_else(|| value.get("type").and_then(Value::as_str))
        .unwrap_or("");
    if message_type == MESSAGE_CARD
        || event_type.contains("card")
        || value
            .get("event")
            .and_then(|event| event.get("action"))
            .is_some()
    {
        return handle_card_callback(state, sink, value);
    }
    if event_type != "im.message.receive_v1" {
        return Ok(());
    }
    let event = value.get("event").unwrap_or(&value);
    let message = event
        .get("message")
        .ok_or_else(|| anyhow!("Feishu event missing message"))?;
    let sender = event
        .get("sender")
        .ok_or_else(|| anyhow!("Feishu event missing sender"))?;
    if sender
        .get("sender_type")
        .and_then(Value::as_str)
        .map(|kind| kind == "bot")
        .unwrap_or(false)
    {
        return Ok(());
    }

    let sender_id = sender.get("sender_id").unwrap_or(sender);
    let open_id = sender_id
        .get("open_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let user_id = sender_id
        .get("user_id")
        .and_then(Value::as_str)
        .map(str::to_string);

    let chat_id = string_field(message, &["chat_id"]).context("Feishu message missing chat_id")?;
    let message_id = string_field(message, &["message_id"]);
    let (text, attachment_refs) = extract_message_payload(message)?;
    let key = ChatKey::new(PLATFORM, chat_id.clone());
    let trimmed_text = text
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_default();
    if trimmed_text.is_empty() && attachment_refs.is_empty() {
        return Ok(());
    }

    state.remember_channel_context(
        key.clone(),
        feishu_channel_context(message, sender, open_id.as_deref(), user_id.as_deref()),
    );
    if let Some(result) = handle_interactive_command(state, sink, &key, &trimmed_text) {
        match result {
            Ok(()) => {}
            Err(error) => {
                if let Err(send_error) = sink.send_text(&chat_id, &format!("⚠️ {error:#}")) {
                    log::warn!(
                        "[im-bridge:feishu] failed to send interactive command error: {send_error:#}"
                    );
                }
            }
        }
        record_message_activity(state, &chat_id);
        return Ok(());
    }
    let attachments = if attachment_refs.is_empty() {
        Vec::new()
    } else if let Some(message_id) = message_id.as_deref() {
        download_feishu_attachments(state, sink, &key, message_id, attachment_refs)
    } else {
        log::warn!("[im-bridge:feishu] cannot download attachments without message_id");
        Vec::new()
    };
    let outcome = router::handle_message_with_attachments(state, &key, &trimmed_text, attachments);
    if let Some(reply) = outcome.reply {
        if let Err(error) = sink.send_text(&chat_id, &reply) {
            log::warn!("[im-bridge:feishu] failed to send command reply: {error:#}");
        }
    }
    record_message_activity(state, &chat_id);
    Ok(())
}

/// What we know about an inbound Feishu attachment before downloading it.
struct FeishuAttachmentRef {
    key: String,
    kind: AgentAttachmentKind,
    resource_type: &'static str,
    file_name: Option<String>,
}

fn extract_message_payload(message: &Value) -> Result<(Option<String>, Vec<FeishuAttachmentRef>)> {
    let message_type = string_field(message, &["message_type"]).unwrap_or_default();
    let content = string_field(message, &["content"]).unwrap_or_default();
    match message_type.as_str() {
        "text" => {
            let value: Value =
                serde_json::from_str(&content).context("parse Feishu text content")?;
            let text = value
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok((text, Vec::new()))
        }
        "post" => {
            let value: Value =
                serde_json::from_str(&content).context("parse Feishu post content")?;
            Ok((extract_post_text(&value), Vec::new()))
        }
        "image" => {
            let value: Value =
                serde_json::from_str(&content).context("parse Feishu image content")?;
            let key = value
                .get("image_key")
                .and_then(Value::as_str)
                .context("Feishu image content missing image_key")?
                .to_string();
            Ok((
                None,
                vec![FeishuAttachmentRef {
                    key,
                    kind: AgentAttachmentKind::Image,
                    resource_type: "image",
                    file_name: None,
                }],
            ))
        }
        "file" | "media" | "audio" => {
            let value: Value =
                serde_json::from_str(&content).context("parse Feishu file content")?;
            let key = value
                .get("file_key")
                .and_then(Value::as_str)
                .context("Feishu file content missing file_key")?
                .to_string();
            let file_name = value
                .get("file_name")
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok((
                None,
                vec![FeishuAttachmentRef {
                    key,
                    kind: AgentAttachmentKind::File,
                    resource_type: "file",
                    file_name,
                }],
            ))
        }
        _ => Ok((None, Vec::new())),
    }
}

fn download_feishu_attachments(
    state: &Arc<ImBridgeState>,
    sink: &Arc<FeishuSink>,
    key: &ChatKey,
    message_id: &str,
    refs: Vec<FeishuAttachmentRef>,
) -> Vec<InboundAttachment> {
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
                "[im-bridge:feishu] dropping attachments for {}: no workspace bound",
                key.chat_id
            );
            return Vec::new();
        }
    };
    let dir = match attachment_dir(&workspace, key.platform, &key.chat_id) {
        Ok(dir) => dir,
        Err(error) => {
            log::warn!("[im-bridge:feishu] cannot prepare attachment dir: {error:#}");
            return Vec::new();
        }
    };
    let mut downloaded = Vec::new();
    for entry in refs {
        let suggested = entry.file_name.clone().or_else(|| Some(entry.key.clone()));
        let destination = allocate_attachment_path(&dir, suggested.as_deref());
        if let Err(error) = sink.download_message_resource(
            message_id,
            &entry.key,
            entry.resource_type,
            &destination,
        ) {
            log::warn!(
                "[im-bridge:feishu] download resource {} failed: {error:#}",
                entry.key
            );
            continue;
        }
        downloaded.push(InboundAttachment {
            path: destination,
            kind: entry.kind,
            mime_type: None,
            display_name: suggested,
        });
    }
    downloaded
}

fn extract_post_text(value: &Value) -> Option<String> {
    let body = value
        .get("zh_cn")
        .or_else(|| value.get("en_us"))
        .or_else(|| value.get("ja_jp"))?;
    let mut lines = Vec::new();
    if let Some(rows) = body.get("content").and_then(Value::as_array) {
        for row in rows {
            if let Some(items) = row.as_array() {
                let mut line = String::new();
                for item in items {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        line.push_str(text);
                    }
                }
                if !line.trim().is_empty() {
                    lines.push(line);
                }
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn handle_card_callback(
    state: &Arc<ImBridgeState>,
    sink: &Arc<FeishuSink>,
    value: Value,
) -> Result<()> {
    let action_value = value
        .get("event")
        .and_then(|event| event.get("action"))
        .and_then(|action| action.get("value"))
        .or_else(|| value.get("action").and_then(|action| action.get("value")))
        .or_else(|| value.get("value"));

    let chat_id = value
        .get("event")
        .and_then(|event| event.get("context"))
        .and_then(|context| context.get("open_chat_id"))
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("open_chat_id")
                .or_else(|| value.get("chat_id"))
                .and_then(Value::as_str)
        });

    if let Some(action) = action_value
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix(ACTION_CALLBACK_PREFIX))
    {
        let Some(chat_id) = chat_id else {
            return Ok(());
        };
        let key = ChatKey::new(PLATFORM, chat_id.to_string());
        state.touch_chat(&key);
        record_message_activity(state, chat_id);
        match router::handle_action_callback(state, &key, action) {
            Ok(reply) => {
                if let Some(message_id) = feishu_callback_message_id(&value) {
                    if let Err(error) = sink.delete_message(&message_id) {
                        log::debug!("[im-bridge:feishu] failed to delete action menu: {error:#}");
                    }
                }
                log::debug!("[im-bridge:feishu] action callback recorded: {reply}");
            }
            Err(error) => {
                if let Err(send_error) = sink.send_text(chat_id, &format!("⚠️ {error:#}")) {
                    log::warn!(
                        "[im-bridge:feishu] failed to send action callback error: {send_error:#}"
                    );
                }
            }
        }
        return Ok(());
    }

    let Some(token) = action_value
        .and_then(|value| {
            value
                .get("token")
                .or_else(|| value.get("permissionToken"))
                .or_else(|| value.get("permission_token"))
        })
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix(CALLBACK_PREFIX))
    else {
        return Ok(());
    };

    let Some(decision) = state.take_permission_token(token) else {
        return Ok(());
    };
    if let Some(chat_id) = chat_id {
        let key = ChatKey::new(PLATFORM, chat_id.to_string());
        state.touch_chat(&key);
        record_message_activity(state, chat_id);
    }
    match state.runtime.respond_permission(
        &decision.sessio_runtime_session_id,
        &decision.request_id,
        decision.option_id,
    ) {
        Ok(()) => {}
        Err(error) => {
            log::warn!("[im-bridge:feishu] permission response failed: {error:#}");
        }
    }
    Ok(())
}

fn feishu_callback_message_id(value: &Value) -> Option<String> {
    value
        .get("event")
        .and_then(|event| event.get("context"))
        .and_then(|context| string_field(context, &["message_id", "messageId"]))
        .or_else(|| {
            value
                .get("event")
                .and_then(|event| string_field(event, &["message_id", "messageId"]))
        })
        .or_else(|| string_field(value, &["message_id", "messageId"]))
}

fn handle_interactive_command(
    state: &Arc<ImBridgeState>,
    sink: &Arc<FeishuSink>,
    key: &ChatKey,
    text: &str,
) -> Option<Result<()>> {
    router::interactive_action_menu(state, key, text).map(|menu| {
        let menu = menu?;
        sink.send_action_menu(&key.chat_id, &menu).map(|_| ())
    })
}

fn feishu_channel_context(
    message: &Value,
    sender: &Value,
    open_id: Option<&str>,
    user_id: Option<&str>,
) -> ChannelContext {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "feishuMessage".to_string(),
        json!({
            "messageId": string_field(message, &["message_id"]),
            "chatId": string_field(message, &["chat_id"]),
            "chatType": string_field(message, &["chat_type"]),
            "messageType": string_field(message, &["message_type"]),
        }),
    );
    metadata.insert("feishuSender".to_string(), sender.clone());
    ChannelContext {
        channel_type: string_field(message, &["chat_type"]),
        user_id: open_id
            .map(str::to_string)
            .or_else(|| user_id.map(str::to_string)),
        team_id: string_field(sender, &["tenant_key"]),
        thread_id: string_field(message, &["thread_id"]),
        display_name: open_id
            .map(str::to_string)
            .or_else(|| user_id.map(str::to_string)),
        metadata,
        last_update_id: None,
    }
}

fn record_message_activity(state: &Arc<ImBridgeState>, chat_id: &str) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, chat_id, None, now_ms())
    {
        log::warn!("[im-bridge:feishu] failed to record message activity: {error:#}");
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
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

fn header_value<'a>(headers: &'a [PbbpHeader], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|header| header.key == key)
        .map(|header| header.value.as_str())
}

fn normalized_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Clone)]
struct FeishuWsConfig {
    url: String,
    service_id: i32,
    ping_interval: Duration,
}

#[derive(Default)]
struct FragmentCache {
    items: HashMap<String, FragmentEntry>,
}

struct FragmentEntry {
    parts: Vec<Option<Vec<u8>>>,
    created_at: Instant,
}

impl FragmentCache {
    fn merge(&mut self, headers: &[PbbpHeader], payload: &[u8]) -> Result<Option<Value>> {
        self.expire_old();
        let sum = header_value(headers, HEADER_SUM)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(1);
        let seq = header_value(headers, HEADER_SEQ)
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let message_id = header_value(headers, HEADER_MESSAGE_ID)
            .map(str::to_string)
            .unwrap_or_else(|| {
                header_value(headers, HEADER_TRACE_ID)
                    .unwrap_or("single")
                    .to_string()
            });
        if sum <= 1 {
            return parse_event_payload(payload).map(Some);
        }
        if seq >= sum {
            bail!("invalid Feishu fragment seq {seq} for sum {sum}");
        }
        let entry = self
            .items
            .entry(message_id.clone())
            .or_insert_with(|| FragmentEntry {
                parts: vec![None; sum],
                created_at: Instant::now(),
            });
        if entry.parts.len() != sum {
            entry.parts.resize(sum, None);
        }
        entry.parts[seq] = Some(payload.to_vec());
        if !entry.parts.iter().all(Option::is_some) {
            return Ok(None);
        }
        let entry = self.items.remove(&message_id).expect("entry exists");
        let mut merged = Vec::new();
        for part in entry.parts.into_iter().flatten() {
            merged.extend(part);
        }
        parse_event_payload(&merged).map(Some)
    }

    fn expire_old(&mut self) {
        self.items
            .retain(|_, entry| entry.created_at.elapsed() < Duration::from_secs(10));
    }
}

fn parse_event_payload(payload: &[u8]) -> Result<Value> {
    serde_json::from_slice(payload).context("parse Feishu event payload")
}

struct TokenCache {
    token: String,
    expires_at: Instant,
}

pub struct FeishuSink {
    client: Client,
    app_id: String,
    app_secret: String,
    domain: String,
    token_cache: Mutex<Option<TokenCache>>,
}

impl FeishuSink {
    fn new(config: FeishuConfig) -> Result<Self> {
        Self::for_credentials(&config.app_id, &config.app_secret, config.domain.as_deref())
    }

    fn for_credentials(app_id: &str, app_secret: &str, domain: Option<&str>) -> Result<Self> {
        let app_id = app_id.trim();
        let app_secret = app_secret.trim();
        if app_id.is_empty() {
            bail!("Feishu App ID is required");
        }
        if app_secret.is_empty() {
            bail!("Feishu App Secret is required");
        }
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build Feishu HTTP client")?;
        Ok(Self {
            client,
            app_id: app_id.to_string(),
            app_secret: app_secret.to_string(),
            domain: normalize_domain(domain),
            token_cache: Mutex::new(None),
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.domain, path)
    }

    fn tenant_access_token(&self) -> Result<String> {
        if let Some(token) = self
            .token_cache
            .lock()
            .ok()
            .and_then(|cache| {
                cache
                    .as_ref()
                    .map(|value| (value.token.clone(), value.expires_at))
            })
            .and_then(|(token, expires_at)| {
                if expires_at > Instant::now() + Duration::from_secs(30) {
                    Some(token)
                } else {
                    None
                }
            })
        {
            return Ok(token);
        }

        let response: TenantAccessTokenResponse = self
            .client
            .post(self.endpoint("/open-apis/auth/v3/tenant_access_token/internal"))
            .json(&json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .context("Feishu tenant_access_token request failed")?
            .error_for_status()
            .context("Feishu tenant_access_token returned HTTP error")?
            .json()
            .context("parse Feishu tenant_access_token response")?;
        if response.code != 0 {
            bail!(
                "Feishu tenant_access_token failed: code={}, msg={}",
                response.code,
                response.msg.unwrap_or_default()
            );
        }
        let token = response
            .tenant_access_token
            .filter(|value| !value.trim().is_empty())
            .context("Feishu tenant_access_token missing token")?;
        let expires =
            Duration::from_secs(response.expire.unwrap_or(7200).saturating_sub(60).max(60));
        if let Ok(mut cache) = self.token_cache.lock() {
            *cache = Some(TokenCache {
                token: token.clone(),
                expires_at: Instant::now() + expires,
            });
        }
        Ok(token)
    }

    fn pull_ws_config(&self) -> Result<FeishuWsConfig> {
        let response: WsEndpointResponse = self
            .client
            .post(self.endpoint("/callback/ws/endpoint"))
            .header("locale", "zh")
            .header("User-Agent", "sessio-im-bridge/1.0")
            .json(&json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .context("Feishu WebSocket endpoint request failed")?
            .error_for_status()
            .context("Feishu WebSocket endpoint returned HTTP error")?
            .json()
            .context("parse Feishu WebSocket endpoint response")?;
        if response.code != 0 {
            bail!(
                "Feishu WebSocket endpoint failed: code={}, msg={}",
                response.code,
                response.msg.unwrap_or_default()
            );
        }
        let data = response
            .data
            .context("Feishu WebSocket endpoint missing data")?;
        let url = data.url.context("Feishu WebSocket endpoint missing URL")?;
        let service_id = query_value(&url, "service_id")
            .and_then(|value| value.parse::<i32>().ok())
            .context("Feishu WebSocket URL missing service_id")?;
        let ping_interval = data
            .client_config
            .as_ref()
            .and_then(|config| config.ping_interval)
            .map(|seconds| Duration::from_secs(seconds.max(1)))
            .unwrap_or(DEFAULT_PING_INTERVAL);
        Ok(FeishuWsConfig {
            url,
            service_id,
            ping_interval,
        })
    }

    fn send_message(&self, chat_id: &str, body: &Value) -> Result<Option<String>> {
        let token = self.tenant_access_token()?;
        let response: FeishuApiResponse = self
            .client
            .post(self.endpoint("/open-apis/im/v1/messages"))
            .query(&[("receive_id_type", "chat_id")])
            .bearer_auth(token)
            .json(body)
            .send()
            .context("Feishu send message request failed")?
            .error_for_status()
            .context("Feishu send message returned HTTP error")?
            .json()
            .context("parse Feishu send message response")?;
        if response.code != 0 {
            bail!(
                "Feishu send message to {chat_id} failed: code={}, msg={}",
                response.code,
                response.msg.unwrap_or_default()
            );
        }
        Ok(response
            .data
            .as_ref()
            .and_then(|data| string_field(data, &["message_id", "messageId"])))
    }

    fn delete_message(&self, message_id: &str) -> Result<()> {
        let token = self.tenant_access_token()?;
        let response: FeishuApiResponse = self
            .client
            .delete(self.endpoint(&format!("/open-apis/im/v1/messages/{message_id}")))
            .bearer_auth(token)
            .send()
            .context("Feishu delete message request failed")?
            .error_for_status()
            .context("Feishu delete message returned HTTP error")?
            .json()
            .context("parse Feishu delete message response")?;
        if response.code != 0 {
            bail!(
                "Feishu delete message {message_id} failed: code={}, msg={}",
                response.code,
                response.msg.unwrap_or_default()
            );
        }
        Ok(())
    }

    fn send_action_menu(&self, chat_id: &str, menu: &router::ActionMenu) -> Result<Option<String>> {
        let actions = menu
            .choices
            .iter()
            .map(|choice| {
                json!({
                    "tag": "button",
                    "text": {
                        "tag": "plain_text",
                        "content": choice.label,
                    },
                    "type": "default",
                    "value": {
                        "action": format!("{}{}", ACTION_CALLBACK_PREFIX, choice.action),
                    },
                })
            })
            .collect::<Vec<_>>();
        let body = json!({
            "receive_id": chat_id,
            "msg_type": "interactive",
            "content": serde_json::to_string(&json!({
                "config": { "wide_screen_mode": true },
                "header": {
                    "template": "blue",
                    "title": {
                        "tag": "plain_text",
                        "content": "Sessio",
                    },
                },
                "elements": [
                    {
                        "tag": "markdown",
                        "content": menu.text,
                    },
                    {
                        "tag": "action",
                        "actions": actions,
                    }
                ],
            }))?,
        });
        self.send_message(chat_id, &body)
    }

    /// Download the file/image attached to an inbound message. `resource_type`
    /// is "image" or "file" per Feishu's resources API.
    fn download_message_resource(
        &self,
        message_id: &str,
        file_key: &str,
        resource_type: &str,
        destination: &Path,
    ) -> Result<()> {
        let token = self.tenant_access_token()?;
        let url = self.endpoint(&format!(
            "/open-apis/im/v1/messages/{message_id}/resources/{file_key}"
        ));
        let mut response = self
            .client
            .get(url)
            .query(&[("type", resource_type)])
            .bearer_auth(token)
            .send()
            .with_context(|| format!("Feishu resource {file_key} request failed"))?
            .error_for_status()
            .with_context(|| format!("Feishu resource {file_key} returned HTTP error"))?;
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create directory {}", parent.display()))?;
        }
        let mut file = std::fs::File::create(destination)
            .with_context(|| format!("create attachment file {}", destination.display()))?;
        response
            .copy_to(&mut file)
            .with_context(|| format!("write attachment file {}", destination.display()))?;
        Ok(())
    }

    /// Upload an image, returning the `image_key` used to reference it in
    /// subsequent `msg_type=image` messages.
    fn upload_image(&self, path: &Path) -> Result<String> {
        let token = self.tenant_access_token()?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
            .to_string();
        let mime = guess_image_mime(path);
        let bytes =
            std::fs::read(path).with_context(|| format!("read image {}", path.display()))?;
        let part = multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str(mime)
            .context("set image MIME type")?;
        let form = multipart::Form::new()
            .text("image_type", "message")
            .part("image", part);
        let response: FeishuUploadImageResponse = self
            .client
            .post(self.endpoint("/open-apis/im/v1/images"))
            .bearer_auth(token)
            .multipart(form)
            .send()
            .context("Feishu upload image failed")?
            .error_for_status()
            .context("Feishu upload image returned HTTP error")?
            .json()
            .context("parse Feishu upload image response")?;
        if response.code != 0 {
            bail!(
                "Feishu upload image failed: code={}, msg={}",
                response.code,
                response.msg.unwrap_or_default()
            );
        }
        response
            .data
            .and_then(|data| data.image_key)
            .context("Feishu upload image response missing image_key")
    }

    /// Upload a generic file, returning the `file_key` used in
    /// `msg_type=file` messages.
    fn upload_file(&self, path: &Path) -> Result<String> {
        let token = self.tenant_access_token()?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        let mime = guess_file_mime(path);
        let bytes = std::fs::read(path).with_context(|| format!("read file {}", path.display()))?;
        let part = multipart::Part::bytes(bytes)
            .file_name(file_name.clone())
            .mime_str(mime)
            .context("set file MIME type")?;
        let form = multipart::Form::new()
            .text("file_type", feishu_file_type(path))
            .text("file_name", file_name)
            .part("file", part);
        let response: FeishuUploadFileResponse = self
            .client
            .post(self.endpoint("/open-apis/im/v1/files"))
            .bearer_auth(token)
            .multipart(form)
            .send()
            .context("Feishu upload file failed")?
            .error_for_status()
            .context("Feishu upload file returned HTTP error")?
            .json()
            .context("parse Feishu upload file response")?;
        if response.code != 0 {
            bail!(
                "Feishu upload file failed: code={}, msg={}",
                response.code,
                response.msg.unwrap_or_default()
            );
        }
        response
            .data
            .and_then(|data| data.file_key)
            .context("Feishu upload file response missing file_key")
    }
}

impl ChatSink for FeishuSink {
    fn platform(&self) -> &'static str {
        PLATFORM
    }

    fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        for chunk in split_text(text, FEISHU_TEXT_LIMIT) {
            let body = json!({
                "receive_id": chat_id,
                "msg_type": "post",
                "content": serde_json::to_string(&json!({
                    "zh_cn": {
                        "content": [[{ "tag": "md", "text": chunk }]]
                    }
                }))?,
            });
            self.send_message(chat_id, &body).map(|_| ())?;
        }
        Ok(())
    }

    fn send_image(&self, chat_id: &str, path: &Path, _caption: Option<&str>) -> Result<()> {
        let image_key = self.upload_image(path)?;
        let body = json!({
            "receive_id": chat_id,
            "msg_type": "image",
            "content": serde_json::to_string(&json!({ "image_key": image_key }))?,
        });
        self.send_message(chat_id, &body).map(|_| ())
    }

    fn send_file(&self, chat_id: &str, path: &Path, _caption: Option<&str>) -> Result<()> {
        let file_key = self.upload_file(path)?;
        let body = json!({
            "receive_id": chat_id,
            "msg_type": "file",
            "content": serde_json::to_string(&json!({ "file_key": file_key }))?,
        });
        self.send_message(chat_id, &body).map(|_| ())
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
        let mut elements = vec![json!({
            "tag": "markdown",
            "content": permission_markdown(request),
        })];
        if !request.options.is_empty() {
            let actions = request
                .options
                .iter()
                .map(|option| {
                    json!({
                        "tag": "button",
                        "text": {
                            "tag": "plain_text",
                            "content": option.label,
                        },
                        "type": "default",
                        "value": {
                            "token": format!("{}{}", CALLBACK_PREFIX, option.token),
                        },
                    })
                })
                .collect::<Vec<_>>();
            elements.push(json!({
                "tag": "action",
                "actions": actions,
            }));
        }
        let body = json!({
            "receive_id": chat_id,
            "msg_type": "interactive",
            "content": serde_json::to_string(&json!({
                "config": { "wide_screen_mode": true },
                "header": {
                    "template": "yellow",
                    "title": {
                        "tag": "plain_text",
                        "content": "Permission requested",
                    },
                },
                "elements": elements,
            }))?,
        });
        let message_id = self.send_message(chat_id, &body)?;
        Ok(message_id.map(|message_id| json!({ "message_id": message_id })))
    }

    fn resolve_permission_message(
        &self,
        _chat_id: &str,
        message_ref: &Value,
        _request: &ChatPermissionRequest,
        _outcome: PermissionResolutionOutcome<'_>,
    ) -> Result<()> {
        let Some(message_id) = message_ref.get("message_id").and_then(Value::as_str) else {
            return Ok(());
        };
        self.delete_message(message_id)
    }
}

fn permission_markdown(request: &ChatPermissionRequest) -> String {
    let mut text = format!("**Tool:** `{}`", request.tool_name);
    if let Some(input) = &request.input_summary {
        if !input.trim().is_empty() {
            text.push_str("\n\n**Input:**\n```json\n");
            text.push_str(input);
            text.push_str("\n```");
        }
    }
    text
}

fn split_text(text: &str, limit: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= limit {
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

fn feishu_file_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "pdf",
        Some("doc" | "docx") => "doc",
        Some("xls" | "xlsx") => "xls",
        Some("ppt" | "pptx") => "ppt",
        Some("mp4") => "mp4",
        _ => "stream",
    }
}

fn normalize_domain(domain: Option<&str>) -> String {
    domain
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_DOMAIN)
        .trim_end_matches('/')
        .to_string()
}

fn query_value(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    query.split('&').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        if k == key {
            Some(v.to_string())
        } else {
            None
        }
    })
}

#[derive(Debug, Deserialize)]
struct TenantAccessTokenResponse {
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    tenant_access_token: Option<String>,
    #[serde(default)]
    expire: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WsEndpointResponse {
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<WsEndpointData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WsEndpointData {
    #[serde(default, rename = "URL")]
    url: Option<String>,
    #[serde(default)]
    client_config: Option<WsClientConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WsClientConfig {
    #[serde(default)]
    ping_interval: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FeishuApiResponse {
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct FeishuUploadImageResponse {
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<FeishuUploadImageData>,
}

#[derive(Debug, Deserialize)]
struct FeishuUploadImageData {
    #[serde(default)]
    image_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FeishuUploadFileResponse {
    code: i64,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<FeishuUploadFileData>,
}

#[derive(Debug, Deserialize)]
struct FeishuUploadFileData {
    #[serde(default)]
    file_key: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct PbbpHeader {
    key: String,
    value: String,
}

impl PbbpHeader {
    fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct PbbpFrame {
    seq_id: u64,
    log_id: u64,
    service: i32,
    method: i32,
    headers: Vec<PbbpHeader>,
    payload_encoding: String,
    payload_type: String,
    payload: Vec<u8>,
    log_id_new: String,
}

impl PbbpFrame {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_varint_field(&mut out, 1, self.seq_id);
        write_varint_field(&mut out, 2, self.log_id);
        write_varint_field(&mut out, 3, self.service as u64);
        write_varint_field(&mut out, 4, self.method as u64);
        for header in &self.headers {
            let mut nested = Vec::new();
            write_bytes_field(&mut nested, 1, header.key.as_bytes());
            write_bytes_field(&mut nested, 2, header.value.as_bytes());
            write_bytes_field(&mut out, 5, &nested);
        }
        if !self.payload_encoding.is_empty() {
            write_bytes_field(&mut out, 6, self.payload_encoding.as_bytes());
        }
        if !self.payload_type.is_empty() {
            write_bytes_field(&mut out, 7, self.payload_type.as_bytes());
        }
        if !self.payload.is_empty() {
            write_bytes_field(&mut out, 8, &self.payload);
        }
        if !self.log_id_new.is_empty() {
            write_bytes_field(&mut out, 9, self.log_id_new.as_bytes());
        }
        out
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut frame = Self::default();
        let mut cursor = ProtoCursor::new(bytes);
        while !cursor.is_eof() {
            let key = cursor.read_varint()?;
            let field = (key >> 3) as u32;
            let wire = (key & 0x07) as u8;
            match (field, wire) {
                (1, 0) => frame.seq_id = cursor.read_varint()?,
                (2, 0) => frame.log_id = cursor.read_varint()?,
                (3, 0) => frame.service = cursor.read_varint()? as i32,
                (4, 0) => frame.method = cursor.read_varint()? as i32,
                (5, 2) => frame.headers.push(decode_header(cursor.read_bytes()?)?),
                (6, 2) => frame.payload_encoding = decode_string(cursor.read_bytes()?)?,
                (7, 2) => frame.payload_type = decode_string(cursor.read_bytes()?)?,
                (8, 2) => frame.payload = cursor.read_bytes()?.to_vec(),
                (9, 2) => frame.log_id_new = decode_string(cursor.read_bytes()?)?,
                _ => cursor.skip(wire)?,
            }
        }
        Ok(frame)
    }
}

fn decode_header(bytes: &[u8]) -> Result<PbbpHeader> {
    let mut header = PbbpHeader::default();
    let mut cursor = ProtoCursor::new(bytes);
    while !cursor.is_eof() {
        let key = cursor.read_varint()?;
        let field = (key >> 3) as u32;
        let wire = (key & 0x07) as u8;
        match (field, wire) {
            (1, 2) => header.key = decode_string(cursor.read_bytes()?)?,
            (2, 2) => header.value = decode_string(cursor.read_bytes()?)?,
            _ => cursor.skip(wire)?,
        }
    }
    Ok(header)
}

fn decode_string(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).context("decode protobuf string")
}

fn write_varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(out, (field as u64) << 3);
    write_varint(out, value);
}

fn write_bytes_field(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    write_varint(out, ((field as u64) << 3) | 2);
    write_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

struct ProtoCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ProtoCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn read_varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *self
                .bytes
                .get(self.pos)
                .ok_or_else(|| anyhow!("unexpected EOF reading protobuf varint"))?;
            self.pos += 1;
            value |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        bail!("protobuf varint too long")
    }

    fn read_bytes(&mut self) -> Result<&'a [u8]> {
        let len = self.read_varint()? as usize;
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| anyhow!("protobuf length overflow"))?;
        if end > self.bytes.len() {
            bail!("unexpected EOF reading protobuf bytes");
        }
        let value = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(value)
    }

    fn skip(&mut self, wire: u8) -> Result<()> {
        match wire {
            0 => {
                self.read_varint()?;
            }
            1 => {
                self.pos = self
                    .pos
                    .checked_add(8)
                    .ok_or_else(|| anyhow!("protobuf skip overflow"))?;
            }
            2 => {
                let len = self.read_varint()? as usize;
                self.pos = self
                    .pos
                    .checked_add(len)
                    .ok_or_else(|| anyhow!("protobuf skip overflow"))?;
            }
            5 => {
                self.pos = self
                    .pos
                    .checked_add(4)
                    .ok_or_else(|| anyhow!("protobuf skip overflow"))?;
            }
            _ => bail!("unsupported protobuf wire type {wire}"),
        }
        if self.pos > self.bytes.len() {
            bail!("protobuf skip past end");
        }
        Ok(())
    }
}
