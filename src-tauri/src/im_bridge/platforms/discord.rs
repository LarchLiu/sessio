//! Discord Bot Gateway platform implementation.

use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::blocking::{multipart, Client, ClientBuilder};
use serde::Deserialize;
use serde_json::{json, Value};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};

use crate::agents::runtime::types::AgentAttachmentKind;

use super::super::attachments::{
    allocate_attachment_path, attachment_dir, download_to_file, guess_file_mime, guess_image_mime,
    InboundAttachment,
};
use super::super::config::DiscordConfig;
use super::super::router;
use super::super::state::{
    ChannelContext, ChatKey, ChatPermissionRequest, ChatSink, ImBridgeState,
    PermissionResolutionOutcome,
};

const PLATFORM: &str = "discord";
const DEFAULT_API_BASE: &str = "https://discord.com/api/v10";
const DEFAULT_GATEWAY_URL: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const DISCORD_TEXT_LIMIT: usize = 1900;
const CALLBACK_PREFIX: &str = "sessio_perm:";

const OP_DISPATCH: i64 = 0;
const OP_HEARTBEAT: i64 = 1;
const OP_IDENTIFY: i64 = 2;
const OP_RECONNECT: i64 = 7;
const OP_INVALID_SESSION: i64 = 9;
const OP_HELLO: i64 = 10;
const OP_HEARTBEAT_ACK: i64 = 11;

const INTENT_GUILDS: i64 = 1 << 0;
const INTENT_GUILD_MESSAGES: i64 = 1 << 9;
const INTENT_DIRECT_MESSAGES: i64 = 1 << 12;
const INTENT_MESSAGE_CONTENT: i64 = 1 << 15;

pub fn spawn(state: Arc<ImBridgeState>) {
    if let Err(error) = thread::Builder::new()
        .name("im-bridge-discord".to_string())
        .spawn(move || gateway_loop(state))
    {
        log::warn!("[im-bridge:discord] failed to spawn worker: {error:#}");
    }
}

pub fn test_connection(bot_token: &str, api_base: Option<&str>) -> Result<()> {
    let sink = DiscordSink::for_token(bot_token, api_base)?;
    sink.get_current_user().map(|_| ())
}

fn gateway_loop(state: Arc<ImBridgeState>) {
    let mut active_key: Option<DiscordConnectionKey> = None;
    let mut sink: Option<Arc<DiscordSink>> = None;

    loop {
        let bridge_config = state.config_snapshot();
        let Some(config) = bridge_config.discord.clone() else {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                log::info!("[im-bridge:discord] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        if !bridge_config.enabled || !config.enabled || config.bot_token.trim().is_empty() {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                log::info!("[im-bridge:discord] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let next_key = DiscordConnectionKey::from_config(&config);
        if active_key.as_ref() != Some(&next_key) {
            match DiscordSink::new(config.clone()) {
                Ok(next_sink) => {
                    let next_sink = Arc::new(next_sink);
                    state.register_sink(next_sink.clone());
                    sink = Some(next_sink);
                    active_key = Some(next_key);
                    log::info!("[im-bridge:discord] configured");
                }
                Err(error) => {
                    state.unregister_sink(PLATFORM);
                    sink = None;
                    active_key = None;
                    log::warn!("[im-bridge:discord] failed to initialize: {error:#}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }

        let Some(sink) = sink.as_ref().cloned() else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        let run_config = state.config_snapshot().discord.unwrap_or_default();
        match run_gateway_once(&state, &sink, &run_config) {
            Ok(()) => thread::sleep(Duration::from_millis(500)),
            Err(error) => {
                log::warn!("[im-bridge:discord] gateway failed: {error:#}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscordConnectionKey {
    bot_token: String,
    api_base: Option<String>,
    gateway_url: Option<String>,
}

impl DiscordConnectionKey {
    fn from_config(config: &DiscordConfig) -> Self {
        Self {
            bot_token: config.bot_token.trim().to_string(),
            api_base: normalized_optional(&config.api_base),
            gateway_url: normalized_optional(&config.gateway_url),
        }
    }
}

fn run_gateway_once(
    state: &Arc<ImBridgeState>,
    sink: &Arc<DiscordSink>,
    config: &DiscordConfig,
) -> Result<()> {
    let gateway_url = config
        .gateway_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_GATEWAY_URL);
    let mut request = gateway_url.into_client_request()?;
    request
        .headers_mut()
        .insert("User-Agent", "sessio-im-bridge/1.0".parse()?);
    let (mut socket, _) = connect(request).context("connect Discord Gateway")?;
    let hello = read_gateway_event(&mut socket).context("read Discord hello")?;
    if hello.op != OP_HELLO {
        bail!("expected Discord hello, got op {}", hello.op);
    }
    let heartbeat_interval = hello
        .d
        .as_ref()
        .and_then(|value| value.get("heartbeat_interval"))
        .and_then(Value::as_u64)
        .context("Discord hello missing heartbeat_interval")?;
    set_socket_read_timeout(&mut socket, Some(Duration::from_secs(5)));
    send_identify(&mut socket, &sink.bot_token)?;

    let mut last_sequence: Option<i64> = None;
    let mut last_heartbeat = Instant::now();
    let heartbeat_every = Duration::from_millis(heartbeat_interval);

    loop {
        let bridge_config = state.config_snapshot();
        let Some(current_config) = bridge_config.discord.as_ref() else {
            return Ok(());
        };
        if !bridge_config.enabled || !current_config.enabled {
            return Ok(());
        }

        if last_heartbeat.elapsed() >= heartbeat_every {
            send_heartbeat(&mut socket, last_sequence)?;
            last_heartbeat = Instant::now();
        }

        match socket.read() {
            Ok(Message::Text(text)) => {
                let event: DiscordGatewayEvent =
                    serde_json::from_str(&text).context("parse Discord Gateway event")?;
                if let Some(seq) = event.s {
                    last_sequence = Some(seq);
                }
                handle_gateway_event(state, sink, current_config, event)?;
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

fn send_identify(
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    bot_token: &str,
) -> Result<()> {
    let payload = json!({
        "op": OP_IDENTIFY,
        "d": {
            "token": bot_token,
            "intents": INTENT_GUILDS | INTENT_GUILD_MESSAGES | INTENT_DIRECT_MESSAGES | INTENT_MESSAGE_CONTENT,
            "properties": {
                "os": std::env::consts::OS,
                "browser": "sessio",
                "device": "sessio"
            }
        }
    });
    socket.write(Message::Text(payload.to_string()))?;
    Ok(())
}

fn send_heartbeat(
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    sequence: Option<i64>,
) -> Result<()> {
    socket.write(Message::Text(
        json!({ "op": OP_HEARTBEAT, "d": sequence }).to_string(),
    ))?;
    Ok(())
}

fn read_gateway_event(
    socket: &mut WebSocket<MaybeTlsStream<std::net::TcpStream>>,
) -> Result<DiscordGatewayEvent> {
    loop {
        match socket.read()? {
            Message::Text(text) => {
                return serde_json::from_str(&text).context("parse Gateway JSON")
            }
            Message::Ping(payload) => socket.write(Message::Pong(payload))?,
            Message::Close(_) => bail!("Discord Gateway closed before hello"),
            _ => {}
        }
    }
}

fn handle_gateway_event(
    state: &Arc<ImBridgeState>,
    sink: &Arc<DiscordSink>,
    config: &DiscordConfig,
    event: DiscordGatewayEvent,
) -> Result<()> {
    match event.op {
        OP_DISPATCH => match event.t.as_deref() {
            Some("READY") => {
                if let Some(data) = event.d {
                    sink.set_current_user_id(
                        data.get("user")
                            .and_then(|user| user.get("id"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    );
                }
            }
            Some("MESSAGE_CREATE") => {
                if let Some(data) = event.d {
                    let message: DiscordMessage =
                        serde_json::from_value(data).context("parse Discord message")?;
                    handle_message_create(state, sink, config, message);
                }
            }
            Some("INTERACTION_CREATE") => {
                if let Some(data) = event.d {
                    let interaction: DiscordInteraction =
                        serde_json::from_value(data).context("parse Discord interaction")?;
                    handle_interaction_create(state, sink, interaction);
                }
            }
            _ => {}
        },
        OP_HEARTBEAT => {
            // Discord may request an immediate heartbeat; the main loop keeps
            // the regular cadence.
        }
        OP_HEARTBEAT_ACK => {}
        OP_RECONNECT | OP_INVALID_SESSION => return Ok(()),
        _ => {}
    }
    Ok(())
}

fn handle_message_create(
    state: &Arc<ImBridgeState>,
    sink: &Arc<DiscordSink>,
    config: &DiscordConfig,
    message: DiscordMessage,
) {
    if message.author.bot.unwrap_or(false) {
        return;
    }
    let channel_id = message.channel_id.trim();
    if channel_id.is_empty() {
        return;
    }
    if !is_allowed(config, &message) {
        log::debug!(
            "[im-bridge:discord] ignored message {} from disallowed guild/channel",
            message.id
        );
        return;
    }
    if config.mention_only && !should_respond_mention_only(sink, &message) {
        return;
    }
    let content = message.content.trim();
    if content.is_empty() && message.attachments.is_empty() {
        return;
    }

    let key = ChatKey::new(PLATFORM, channel_id.to_string());
    state.remember_channel_context(key.clone(), discord_channel_context(&message));
    let attachments = download_discord_attachments(state, sink, &key, &message);
    let outcome = router::handle_message_with_attachments(
        state,
        &key,
        &strip_bot_mention(sink, content),
        attachments,
    );
    if let Some(reply) = outcome.reply {
        if let Err(error) = sink.send_text(channel_id, &reply) {
            log::warn!("[im-bridge:discord] failed to send command reply: {error:#}");
        }
    }
    record_message_activity(state, channel_id, snowflake_i64(&message.id));
}

fn download_discord_attachments(
    state: &Arc<ImBridgeState>,
    sink: &Arc<DiscordSink>,
    key: &ChatKey,
    message: &DiscordMessage,
) -> Vec<InboundAttachment> {
    if message.attachments.is_empty() {
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
                "[im-bridge:discord] dropping attachments for {}: no workspace bound",
                key.chat_id
            );
            return Vec::new();
        }
    };
    let dir = match attachment_dir(&workspace, key.platform, &key.chat_id) {
        Ok(dir) => dir,
        Err(error) => {
            log::warn!("[im-bridge:discord] cannot prepare attachment dir: {error:#}");
            return Vec::new();
        }
    };
    let mut downloaded = Vec::new();
    for attachment in &message.attachments {
        let kind = attachment
            .content_type
            .as_deref()
            .filter(|mime| mime.starts_with("image/"))
            .map(|_| AgentAttachmentKind::Image)
            .unwrap_or(AgentAttachmentKind::File);
        let suggested = attachment.filename.clone();
        let destination = allocate_attachment_path(&dir, suggested.as_deref());
        if let Err(error) = download_to_file(&sink.client, &attachment.url, None, &destination) {
            log::warn!(
                "[im-bridge:discord] download attachment {} failed: {error:#}",
                attachment.url
            );
            continue;
        }
        downloaded.push(InboundAttachment {
            path: destination,
            kind,
            mime_type: attachment.content_type.clone(),
            display_name: suggested,
        });
    }
    downloaded
}

fn handle_interaction_create(
    state: &Arc<ImBridgeState>,
    sink: &Arc<DiscordSink>,
    interaction: DiscordInteraction,
) {
    let Some(data) = interaction.data.as_ref() else {
        return;
    };
    let Some(token) = data
        .custom_id
        .as_deref()
        .and_then(|value| value.strip_prefix(CALLBACK_PREFIX))
    else {
        return;
    };
    let Some(decision) = state.take_permission_token(token) else {
        if let Err(error) = sink.respond_interaction(
            &interaction.id,
            &interaction.token,
            "Permission request expired",
        ) {
            log::warn!("[im-bridge:discord] failed to respond expired permission: {error:#}");
        }
        return;
    };
    match state.runtime.respond_permission(
        &decision.sessio_runtime_session_id,
        &decision.request_id,
        decision.option_id,
    ) {
        Ok(()) => {
            if let Err(error) = sink.respond_interaction(
                &interaction.id,
                &interaction.token,
                "Permission response recorded",
            ) {
                log::warn!("[im-bridge:discord] failed to acknowledge permission: {error:#}");
            }
        }
        Err(error) => {
            let _ = sink.respond_interaction(
                &interaction.id,
                &interaction.token,
                "Permission response failed",
            );
            log::warn!("[im-bridge:discord] permission response failed: {error:#}");
        }
    }
}

fn is_allowed(config: &DiscordConfig, message: &DiscordMessage) -> bool {
    let guild_allowed = config.allowed_server_ids.is_empty()
        || message
            .guild_id
            .as_deref()
            .map(|guild| contains_trimmed(&config.allowed_server_ids, guild))
            .unwrap_or(true);
    let channel_allowed = config.allowed_channel_ids.is_empty()
        || contains_trimmed(&config.allowed_channel_ids, &message.channel_id);
    guild_allowed && channel_allowed
}

fn should_respond_mention_only(sink: &Arc<DiscordSink>, message: &DiscordMessage) -> bool {
    if message.guild_id.is_none() {
        return true;
    }
    if message.referenced_message.is_some() {
        return true;
    }
    let Some(bot_id) = sink.current_user_id() else {
        return false;
    };
    message.mentions.iter().any(|user| user.id == bot_id)
        || message.content.contains(&format!("<@{bot_id}>"))
        || message.content.contains(&format!("<@!{bot_id}>"))
}

fn strip_bot_mention(sink: &Arc<DiscordSink>, content: &str) -> String {
    let Some(bot_id) = sink.current_user_id() else {
        return content.trim().to_string();
    };
    content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
        .trim()
        .to_string()
}

fn discord_channel_context(message: &DiscordMessage) -> ChannelContext {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "discordMessage".to_string(),
        json!({
            "id": message.id,
            "channelId": message.channel_id,
            "guildId": message.guild_id.as_deref(),
            "authorId": message.author.id,
            "authorUsername": message.author.username.as_deref(),
        }),
    );
    ChannelContext {
        channel_type: message
            .guild_id
            .as_ref()
            .map(|_| "guild".to_string())
            .or_else(|| {
                if message.guild_id.is_none() {
                    Some("dm".to_string())
                } else {
                    None
                }
            }),
        user_id: Some(message.author.id.clone()),
        team_id: message.guild_id.clone(),
        thread_id: None,
        display_name: discord_display_name(message),
        metadata,
        last_update_id: snowflake_i64(&message.id),
    }
}

fn discord_display_name(message: &DiscordMessage) -> Option<String> {
    message
        .author
        .global_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            message
                .author
                .username
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("@{value}"))
        })
}

fn record_message_activity(state: &Arc<ImBridgeState>, channel_id: &str, message_id: Option<i64>) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, channel_id, message_id, now_ms())
    {
        log::warn!("[im-bridge:discord] failed to record message activity: {error:#}");
    }
}

fn snowflake_i64(value: &str) -> Option<i64> {
    value.parse::<i64>().ok()
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn contains_trimmed(values: &[String], wanted: &str) -> bool {
    let wanted = wanted.trim();
    values.iter().any(|value| value.trim() == wanted)
}

fn normalized_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub struct DiscordSink {
    client: Client,
    api_base: String,
    bot_token: String,
    current_user_id: std::sync::Mutex<Option<String>>,
}

impl DiscordSink {
    fn new(config: DiscordConfig) -> Result<Self> {
        Self::for_token(&config.bot_token, config.api_base.as_deref())
    }

    fn for_token(bot_token: &str, api_base: Option<&str>) -> Result<Self> {
        let bot_token = bot_token.trim();
        if bot_token.is_empty() {
            bail!("Discord bot token is required");
        }
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build Discord HTTP client")?;
        Ok(Self {
            client,
            api_base: api_base
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(DEFAULT_API_BASE)
                .trim_end_matches('/')
                .to_string(),
            bot_token: bot_token.to_string(),
            current_user_id: std::sync::Mutex::new(None),
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.api_base, path)
    }

    fn set_current_user_id(&self, user_id: Option<String>) {
        if let Ok(mut current) = self.current_user_id.lock() {
            *current = user_id;
        }
    }

    fn current_user_id(&self) -> Option<String> {
        self.current_user_id.lock().ok()?.clone()
    }

    fn get_current_user(&self) -> Result<Value> {
        self.get_json("/users/@me")
    }

    fn send_message(&self, channel_id: &str, text: &str, components: Option<Value>) -> Result<()> {
        let mut body = json!({
            "content": text,
            "allowed_mentions": { "parse": [] },
        });
        if let Some(components) = components {
            body["components"] = components;
        }
        self.post_json::<Value>(&format!("/channels/{channel_id}/messages"), &body)
            .map(|_| ())
    }

    /// Send a message and return the resulting message id so we can later edit
    /// the message (used for permission prompts).
    fn send_message_with_id(
        &self,
        channel_id: &str,
        text: &str,
        components: Option<Value>,
    ) -> Result<String> {
        let mut body = json!({
            "content": text,
            "allowed_mentions": { "parse": [] },
        });
        if let Some(components) = components {
            body["components"] = components;
        }
        let value: Value = self.post_json(&format!("/channels/{channel_id}/messages"), &body)?;
        value
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("Discord createMessage response missing id"))
    }

    fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        text: &str,
        components: Option<Value>,
    ) -> Result<()> {
        let mut body = json!({
            "content": text,
            "allowed_mentions": { "parse": [] },
        });
        if let Some(components) = components {
            body["components"] = components;
        }
        self.client
            .patch(self.endpoint(&format!("/channels/{channel_id}/messages/{message_id}")))
            .bearer_auth(&self.bot_token)
            .json(&body)
            .send()
            .with_context(|| "Discord editMessage failed")?
            .error_for_status()
            .with_context(|| "Discord editMessage returned HTTP error")?;
        Ok(())
    }

    fn trigger_typing(&self, channel_id: &str) -> Result<()> {
        self.client
            .post(self.endpoint(&format!("/channels/{channel_id}/typing")))
            .bearer_auth(&self.bot_token)
            .send()
            .with_context(|| "Discord triggerTyping failed")?
            .error_for_status()
            .with_context(|| "Discord triggerTyping returned HTTP error")?;
        Ok(())
    }

    fn respond_interaction(&self, interaction_id: &str, token: &str, content: &str) -> Result<()> {
        let body = json!({
            "type": 4,
            "data": {
                "content": content,
                "flags": 64
            }
        });
        self.client
            .post(self.endpoint(&format!("/interactions/{interaction_id}/{token}/callback")))
            .json(&body)
            .send()
            .with_context(|| "Discord interaction callback failed")?
            .error_for_status()
            .with_context(|| "Discord interaction callback returned HTTP error")?;
        Ok(())
    }

    fn send_attachment(
        &self,
        channel_id: &str,
        path: &Path,
        caption: Option<&str>,
        mime: &str,
    ) -> Result<()> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file")
            .to_string();
        let bytes =
            std::fs::read(path).with_context(|| format!("read attachment {}", path.display()))?;
        let part = multipart::Part::bytes(bytes)
            .file_name(file_name)
            .mime_str(mime)
            .context("set attachment MIME type")?;
        let payload = json!({
            "content": caption.unwrap_or(""),
            "allowed_mentions": { "parse": [] },
        });
        let form = multipart::Form::new()
            .text("payload_json", payload.to_string())
            .part("files[0]", part);
        self.client
            .post(self.endpoint(&format!("/channels/{channel_id}/messages")))
            .bearer_auth(&self.bot_token)
            .multipart(form)
            .send()
            .with_context(|| "Discord createMessage (attachment) failed")?
            .error_for_status()
            .with_context(|| "Discord createMessage (attachment) returned HTTP error")?;
        Ok(())
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        self.client
            .get(self.endpoint(path))
            .bearer_auth(&self.bot_token)
            .send()
            .with_context(|| format!("Discord GET {path} failed"))?
            .error_for_status()
            .with_context(|| format!("Discord GET {path} returned HTTP error"))?
            .json::<T>()
            .with_context(|| format!("parse Discord GET {path} response"))
    }

    fn post_json<T: for<'de> Deserialize<'de>>(&self, path: &str, body: &Value) -> Result<T> {
        self.client
            .post(self.endpoint(path))
            .bearer_auth(&self.bot_token)
            .json(body)
            .send()
            .with_context(|| format!("Discord POST {path} failed"))?
            .error_for_status()
            .with_context(|| format!("Discord POST {path} returned HTTP error"))?
            .json::<T>()
            .with_context(|| format!("parse Discord POST {path} response"))
    }
}

impl ChatSink for DiscordSink {
    fn platform(&self) -> &'static str {
        PLATFORM
    }

    fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        for chunk in split_discord_text(text) {
            self.send_message(chat_id, &chunk, None)?;
        }
        Ok(())
    }

    fn send_image(&self, chat_id: &str, path: &Path, caption: Option<&str>) -> Result<()> {
        let mime = guess_image_mime(path);
        self.send_attachment(chat_id, path, caption, mime)
    }

    fn send_file(&self, chat_id: &str, path: &Path, caption: Option<&str>) -> Result<()> {
        let mime = guess_file_mime(path);
        self.send_attachment(chat_id, path, caption, mime)
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
        let text = format_permission_text(request);
        let components = permission_components(request);
        let message_id = self.send_message_with_id(chat_id, &text, components)?;
        Ok(Some(json!({
            "channel_id": chat_id,
            "message_id": message_id,
        })))
    }

    fn resolve_permission_message(
        &self,
        chat_id: &str,
        message_ref: &Value,
        request: &ChatPermissionRequest,
        outcome: PermissionResolutionOutcome<'_>,
    ) -> Result<()> {
        let Some(message_id) = message_ref.get("message_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let mut text = format_permission_text(request);
        text.push_str("\n\n");
        text.push_str(&format_permission_outcome(outcome));
        // Editing with an empty components array clears the buttons.
        self.edit_message(chat_id, message_id, &text, Some(json!([])))
    }

    fn send_typing(&self, chat_id: &str) -> Result<()> {
        self.trigger_typing(chat_id)
    }
}

fn format_permission_text(request: &ChatPermissionRequest) -> String {
    let mut text = format!("Permission requested\nTool: {}", request.tool_name);
    if let Some(input) = &request.input_summary {
        if !input.trim().is_empty() {
            text.push_str("\n\nInput:\n");
            text.push_str(input);
        }
    }
    text
}

fn format_permission_outcome(outcome: PermissionResolutionOutcome<'_>) -> String {
    let marker = if outcome.approved { "✅" } else { "❌" };
    match outcome.label {
        Some(label) => format!("{marker} {label}"),
        None if outcome.approved => format!("{marker} Allowed"),
        None => format!("{marker} Rejected"),
    }
}

fn permission_components(request: &ChatPermissionRequest) -> Option<Value> {
    let buttons = request
        .options
        .iter()
        .map(|option| {
            json!({
                "type": 2,
                "style": 2,
                "label": option.label,
                "custom_id": format!("{}{}", CALLBACK_PREFIX, option.token),
            })
        })
        .collect::<Vec<_>>();
    if buttons.is_empty() {
        None
    } else {
        Some(json!([{ "type": 1, "components": buttons }]))
    }
}

fn split_discord_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= DISCORD_TEXT_LIMIT {
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

#[derive(Debug, Deserialize)]
struct DiscordGatewayEvent {
    op: i64,
    #[serde(default)]
    d: Option<Value>,
    #[serde(default)]
    s: Option<i64>,
    #[serde(default)]
    t: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordMessage {
    id: String,
    #[serde(rename = "channel_id")]
    channel_id: String,
    #[serde(default, rename = "guild_id")]
    guild_id: Option<String>,
    #[serde(default)]
    content: String,
    author: DiscordUser,
    #[serde(default)]
    mentions: Vec<DiscordUser>,
    #[serde(default, rename = "referenced_message")]
    referenced_message: Option<Value>,
    #[serde(default)]
    attachments: Vec<DiscordAttachment>,
}

#[derive(Debug, Deserialize)]
struct DiscordAttachment {
    url: String,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default, rename = "content_type")]
    content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiscordUser {
    id: String,
    #[serde(default)]
    username: Option<String>,
    #[serde(default, rename = "global_name")]
    global_name: Option<String>,
    #[serde(default)]
    bot: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DiscordInteraction {
    id: String,
    token: String,
    #[serde(default)]
    data: Option<DiscordInteractionData>,
}

#[derive(Debug, Deserialize)]
struct DiscordInteractionData {
    #[serde(default, rename = "custom_id")]
    custom_id: Option<String>,
}
