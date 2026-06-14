//! WeChat iLink platform implementation.
//!
//! Supports text plus media (image/file) inbound and outbound. The CDN media
//! flow follows the openhanako reference adapter: pull
//! `getuploadurl` for an encrypted upload URL, AES-128-ECB-encrypt the file
//! body, POST the ciphertext to the iLink CDN, then reference the returned
//! `encrypt_query_param` in the chat message.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use aes::Aes128;
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyInit};
use md5::{Digest as Md5Digest, Md5};
use reqwest::blocking::{Client, ClientBuilder};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value};
use sha2::Sha256;
// `md5::Digest` (re-exported from the `digest` crate) is already in scope via
// the `Md5Digest` alias above, which makes `Sha256::update`/`finalize` work
// without an extra trait import.

use crate::agents::runtime::types::AgentAttachmentKind;

use super::super::attachments::{allocate_attachment_path, attachment_dir, InboundAttachment};
use super::super::config::WechatConfig;
use super::super::router;
use super::super::state::{
    ChannelContext, ChatKey, ChatPermissionRequest, ChatSink, ImBridgeState,
};

const PLATFORM: &str = "wechat";
const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const FIXED_QR_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const CDN_BASE_URL: &str = "https://novac2c.cdn.weixin.qq.com/c2c";
const WECHAT_TEXT_LIMIT: usize = 4000;
const MESSAGE_TYPE_USER: i64 = 1;
const MESSAGE_TYPE_BOT: i64 = 2;
const MESSAGE_STATE_FINISH: i64 = 2;
const MESSAGE_ITEM_TEXT: i64 = 1;
const MESSAGE_ITEM_IMAGE: i64 = 2;
const MESSAGE_ITEM_VOICE: i64 = 3;
const MESSAGE_ITEM_FILE: i64 = 4;
const UPLOAD_MEDIA_TYPE_IMAGE: i64 = 1;
const UPLOAD_MEDIA_TYPE_FILE: i64 = 3;

pub fn spawn(state: Arc<ImBridgeState>) {
    if let Err(error) = thread::Builder::new()
        .name("im-bridge-wechat".to_string())
        .spawn(move || poll_loop(state))
    {
        log::warn!("[im-bridge:wechat] failed to spawn worker: {error:#}");
    }
}

pub fn test_connection(bot_token: &str, base_url: Option<&str>) -> Result<()> {
    let client = WechatIlinkClient::for_token(bot_token, base_url, 10)?;
    client.get_config().map(|_| ())
}

pub fn get_qrcode(base_url: Option<&str>) -> Result<WechatQrCode> {
    let client = WechatIlinkClient::for_login(base_url)?;
    client.get_qrcode()
}

pub fn poll_qrcode_status(qrcode: &str, base_url: Option<&str>) -> Result<WechatQrStatus> {
    let client = WechatIlinkClient::for_login(base_url)?;
    client.poll_qrcode_status(qrcode)
}

fn poll_loop(state: Arc<ImBridgeState>) {
    let mut active_key: Option<WechatConnectionKey> = None;
    let mut sink: Option<Arc<WechatSink>> = None;
    let mut cursor = String::new();
    let mut cursor_path: Option<PathBuf> = None;

    loop {
        let bridge_config = state.config_snapshot();
        let Some(config) = bridge_config.wechat.clone() else {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                cursor.clear();
                cursor_path = None;
                log::info!("[im-bridge:wechat] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        if !bridge_config.enabled || !config.enabled || config.bot_token.trim().is_empty() {
            if active_key.take().is_some() {
                state.unregister_sink(PLATFORM);
                sink = None;
                cursor.clear();
                cursor_path = None;
                log::info!("[im-bridge:wechat] disabled; sink unregistered");
            }
            thread::sleep(Duration::from_secs(1));
            continue;
        }

        let next_key = WechatConnectionKey::from_config(&config);
        if active_key.as_ref() != Some(&next_key) {
            match WechatSink::new(config.clone()) {
                Ok(next_sink) => {
                    let next_sink = Arc::new(next_sink);
                    state.register_sink(next_sink.clone());
                    sink = Some(next_sink);
                    active_key = Some(next_key);
                    cursor_path = sync_cursor_path(&config.bot_token);
                    cursor = cursor_path
                        .as_ref()
                        .and_then(|path| load_cursor(path).ok())
                        .unwrap_or_default();
                    log::info!("[im-bridge:wechat] connected");
                }
                Err(error) => {
                    state.unregister_sink(PLATFORM);
                    sink = None;
                    active_key = None;
                    cursor.clear();
                    cursor_path = None;
                    log::warn!("[im-bridge:wechat] failed to initialize: {error:#}");
                    thread::sleep(Duration::from_secs(5));
                    continue;
                }
            }
        }

        let Some(current_sink) = sink.as_ref().cloned() else {
            thread::sleep(Duration::from_secs(1));
            continue;
        };

        match current_sink.get_updates(&cursor, config.poll_timeout_secs) {
            Ok(updates) => {
                if !updates.get_updates_buf.trim().is_empty() {
                    cursor = updates.get_updates_buf;
                    if let Some(path) = cursor_path.as_ref() {
                        if let Err(error) = save_cursor(path, &cursor) {
                            log::warn!("[im-bridge:wechat] failed to persist cursor: {error:#}");
                        }
                    }
                }
                for message in updates.msgs {
                    handle_message(&state, &current_sink, message);
                }
            }
            Err(error) if is_session_expired_error(&error) => {
                log::warn!("[im-bridge:wechat] auth session expired; QR login is required");
                state.unregister_sink(PLATFORM);
                sink = None;
                active_key = None;
                cursor.clear();
                cursor_path = None;
                thread::sleep(Duration::from_secs(5));
            }
            Err(error) => {
                log::warn!("[im-bridge:wechat] polling failed: {error:#}");
                thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WechatConnectionKey {
    bot_token: String,
    base_url: Option<String>,
    poll_timeout_secs: u64,
}

impl WechatConnectionKey {
    fn from_config(config: &WechatConfig) -> Self {
        Self {
            bot_token: config.bot_token.trim().to_string(),
            base_url: normalized_optional(&config.base_url),
            poll_timeout_secs: config.poll_timeout_secs,
        }
    }
}

fn handle_message(state: &Arc<ImBridgeState>, sink: &Arc<WechatSink>, message: WechatWireMessage) {
    if message.message_type != MESSAGE_TYPE_USER {
        return;
    }
    let chat_id = message.from_user_id.trim().to_string();
    if chat_id.is_empty() || chat_id.ends_with("@im.bot") {
        return;
    }
    if message
        .context_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        log::warn!("[im-bridge:wechat] message from {chat_id} did not include context token");
    }

    let text = extract_text(&message.item_list);
    let attachment_refs = extract_media_refs(&message.item_list);
    if text.trim().is_empty() && attachment_refs.is_empty() {
        return;
    }

    let key = ChatKey::new(PLATFORM, chat_id.clone());
    let context = wechat_channel_context(&message, &chat_id);
    if let Some(token) = message.context_token.as_deref() {
        sink.remember_context(&chat_id, token);
    }
    state.remember_channel_context(key.clone(), context);

    let attachments = if attachment_refs.is_empty() {
        Vec::new()
    } else {
        download_wechat_attachments(state, sink, &key, attachment_refs)
    };

    let outcome = router::handle_message_with_attachments(state, &key, &text, attachments);
    if let Some(reply) = outcome.reply {
        if let Err(error) = sink.send_text(&chat_id, &reply) {
            log::warn!("[im-bridge:wechat] failed to send command reply: {error:#}");
        }
    }
    record_chat_activity(state, &chat_id);
}

/// Reference to a media item discovered inside an inbound `item_list`. The CDN
/// download URL is encoded inside `encrypt_query_param`; AES key is needed to
/// decrypt the response body.
#[derive(Debug, Clone)]
struct WechatMediaRef {
    kind: AgentAttachmentKind,
    encrypt_query_param: String,
    aes_key: Option<String>,
    file_name: Option<String>,
}

fn extract_media_refs(items: &[WechatMessageItem]) -> Vec<WechatMediaRef> {
    let mut refs = Vec::new();
    for item in items {
        collect_media_refs(item, &mut refs);
    }
    refs
}

fn collect_media_refs(item: &WechatMessageItem, refs: &mut Vec<WechatMediaRef>) {
    match item.item_type {
        MESSAGE_ITEM_IMAGE => {
            if let Some(image) = item.image_item.as_ref() {
                if let Some(query) = image
                    .media
                    .as_ref()
                    .and_then(|media| media.encrypt_query_param.clone())
                {
                    let aes_key = image
                        .aeskey
                        .as_deref()
                        .map(encode_ilink_media_aes_key)
                        .transpose()
                        .ok()
                        .flatten()
                        .or_else(|| image.media.as_ref().and_then(|media| media.aes_key.clone()));
                    refs.push(WechatMediaRef {
                        kind: AgentAttachmentKind::Image,
                        encrypt_query_param: query,
                        aes_key,
                        file_name: None,
                    });
                }
            }
        }
        MESSAGE_ITEM_FILE => {
            if let Some(file) = item.file_item.as_ref() {
                if let Some(query) = file
                    .media
                    .as_ref()
                    .and_then(|media| media.encrypt_query_param.clone())
                {
                    let aes_key = file.media.as_ref().and_then(|media| media.aes_key.clone());
                    refs.push(WechatMediaRef {
                        kind: AgentAttachmentKind::File,
                        encrypt_query_param: query,
                        aes_key,
                        file_name: file.file_name.clone(),
                    });
                }
            }
        }
        _ => {}
    }
    if let Some(ref_item) = item
        .ref_msg
        .as_ref()
        .and_then(|ref_msg| ref_msg.message_item.as_deref())
    {
        collect_media_refs(ref_item, refs);
    }
}

fn download_wechat_attachments(
    state: &Arc<ImBridgeState>,
    sink: &Arc<WechatSink>,
    key: &ChatKey,
    refs: Vec<WechatMediaRef>,
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
                "[im-bridge:wechat] dropping attachments for {}: no workspace bound",
                key.chat_id
            );
            return Vec::new();
        }
    };
    let dir = match attachment_dir(&workspace, key.platform, &key.chat_id) {
        Ok(dir) => dir,
        Err(error) => {
            log::warn!("[im-bridge:wechat] cannot prepare attachment dir: {error:#}");
            return Vec::new();
        }
    };
    let mut downloaded = Vec::new();
    for entry in refs {
        let suggested = entry.file_name.clone().or_else(|| match entry.kind {
            AgentAttachmentKind::Image => Some("image.jpg".to_string()),
            AgentAttachmentKind::File => Some("file.bin".to_string()),
        });
        let destination = allocate_attachment_path(&dir, suggested.as_deref());
        if let Err(error) = sink.download_encrypted_media(
            &entry.encrypt_query_param,
            entry.aes_key.as_deref(),
            &destination,
        ) {
            log::warn!("[im-bridge:wechat] download attachment failed: {error:#}");
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

fn wechat_channel_context(message: &WechatWireMessage, chat_id: &str) -> ChannelContext {
    let mut metadata = JsonMap::new();
    metadata.insert(
        "wechat".to_string(),
        json!({
            "fromUserId": message.from_user_id,
            "toUserId": message.to_user_id,
            "clientId": message.client_id,
            "messageId": message.message_id,
        }),
    );
    if let Some(context_token) = message
        .context_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        metadata.insert(
            "wechatContext".to_string(),
            json!({
                "userId": chat_id,
                "contextToken": context_token,
                "observedAt": now_ms(),
            }),
        );
    }
    ChannelContext {
        channel_type: Some("direct".to_string()),
        user_id: Some(chat_id.to_string()),
        team_id: None,
        thread_id: None,
        display_name: Some(wechat_display_name(chat_id)),
        metadata,
        last_update_id: message.message_id,
    }
}

fn record_chat_activity(state: &Arc<ImBridgeState>, chat_id: &str) {
    if let Err(error) =
        state
            .store
            .update_channel_session_activity(PLATFORM, chat_id, None, now_ms())
    {
        log::warn!("[im-bridge:wechat] failed to record chat activity: {error:#}");
    }
}

fn extract_text(items: &[WechatMessageItem]) -> String {
    for item in items {
        if item.item_type == MESSAGE_ITEM_TEXT {
            if let Some(text) = item
                .text_item
                .as_ref()
                .and_then(|text| text.text.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let Some(ref_msg) = item.ref_msg.as_ref() else {
                    return text.to_string();
                };
                let mut parts = Vec::new();
                if let Some(title) = ref_msg
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                {
                    parts.push(title.to_string());
                }
                if let Some(ref_item) = ref_msg.message_item.as_deref() {
                    let ref_text = extract_text(std::slice::from_ref(ref_item));
                    if !ref_text.trim().is_empty() {
                        parts.push(ref_text);
                    }
                }
                if parts.is_empty() {
                    return text.to_string();
                }
                return format!("[引用: {}]\n{text}", parts.join(" | "));
            }
        }
        if item.item_type == MESSAGE_ITEM_VOICE {
            if let Some(text) = item
                .voice_item
                .as_ref()
                .and_then(|voice| voice.text.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return text.to_string();
            }
        }
    }
    String::new()
}

pub struct WechatSink {
    client: WechatIlinkClient,
    contexts: Mutex<HashMap<String, String>>,
}

impl WechatSink {
    fn new(config: WechatConfig) -> Result<Self> {
        let client = WechatIlinkClient::for_token(
            &config.bot_token,
            config.base_url.as_deref(),
            config.poll_timeout_secs,
        )?;
        Ok(Self {
            client,
            contexts: Mutex::new(HashMap::new()),
        })
    }

    fn remember_context(&self, chat_id: &str, token: &str) {
        let token = token.trim();
        if token.is_empty() {
            return;
        }
        if let Ok(mut contexts) = self.contexts.lock() {
            contexts.insert(chat_id.to_string(), token.to_string());
        }
    }

    fn context_token(&self, chat_id: &str) -> Option<String> {
        self.contexts.lock().ok()?.get(chat_id).cloned()
    }

    fn get_updates(&self, cursor: &str, timeout_secs: u64) -> Result<WechatUpdatesResponse> {
        self.client.get_updates(cursor, timeout_secs)
    }

    /// Download an encrypted CDN file referenced by `encrypt_query_param` to
    /// `destination`. The payload is AES-128-ECB encrypted with `aes_key`; when
    /// the key is missing we write the response body verbatim.
    fn download_encrypted_media(
        &self,
        encrypt_query_param: &str,
        aes_key: Option<&str>,
        destination: &Path,
    ) -> Result<()> {
        let url = format!(
            "{CDN_BASE_URL}/download?encrypted_query_param={}",
            percent_encode(encrypt_query_param)
        );
        let bytes = self
            .client
            .client
            .get(url)
            .send()
            .context("WeChat CDN download failed")?
            .error_for_status()
            .context("WeChat CDN download returned HTTP error")?
            .bytes()
            .context("read WeChat CDN download body")?;
        let decrypted = if let Some(aes_key) = aes_key {
            let key = decode_ilink_media_aes_key(aes_key)?;
            decrypt_aes128_ecb(&bytes, &key)?
        } else {
            bytes.to_vec()
        };
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create directory {}", parent.display()))?;
        }
        std::fs::write(destination, decrypted)
            .with_context(|| format!("write attachment file {}", destination.display()))?;
        Ok(())
    }

    /// Upload an arbitrary buffer through the iLink CDN. Returns the upload
    /// handle used to address the file inside a chat message.
    fn upload_media(
        &self,
        buffer: &[u8],
        to_user_id: &str,
        media_type: i64,
    ) -> Result<WechatUploadedMedia> {
        let rawsize = buffer.len();
        let mut md5 = Md5::new();
        md5.update(buffer);
        let rawfilemd5 = hex::encode(md5.finalize());
        let filesize = aes_ecb_padded_size(rawsize);
        let filekey = hex::encode({
            let mut hasher = Sha256::new();
            hasher.update(buffer);
            hasher.update(rawsize.to_be_bytes());
            hasher.update(rawfilemd5.as_bytes());
            hasher.update(to_user_id.as_bytes());
            hasher.finalize()
        });
        // iLink expects a 16-char filekey for image upload but tolerates 32+;
        // truncating to 32 hex chars stays inside CDN limits.
        let filekey = filekey[..32].to_string();
        let aes_key = create_ilink_media_aes_key();
        let aes_key_hex = hex::encode(aes_key);

        let upload_resp: WechatUploadUrlResponse = self.client.api_post_json(
            "/ilink/bot/getuploadurl",
            &json!({
                "filekey": filekey,
                "media_type": media_type,
                "to_user_id": to_user_id,
                "rawsize": rawsize,
                "rawfilemd5": rawfilemd5,
                "filesize": filesize,
                "no_need_thumb": true,
                "aeskey": aes_key_hex,
                "base_info": base_info(),
            }),
            20,
        )?;
        let upload_param = upload_resp
            .upload_param
            .filter(|value| !value.trim().is_empty())
            .context("WeChat getUploadUrl response missing upload_param")?;

        let ciphertext = encrypt_aes128_ecb(buffer, &aes_key);
        let cdn_url = format!(
            "{CDN_BASE_URL}/upload?encrypted_query_param={}&filekey={}",
            percent_encode(&upload_param),
            percent_encode(&filekey)
        );
        let resp = self
            .client
            .client
            .post(cdn_url)
            .header("Content-Type", "application/octet-stream")
            .body(ciphertext)
            .send()
            .context("WeChat CDN upload failed")?
            .error_for_status()
            .context("WeChat CDN upload returned HTTP error")?;
        let download_param = resp
            .headers()
            .get("x-encrypted-param")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .context("WeChat CDN upload missing x-encrypted-param header")?;
        Ok(WechatUploadedMedia {
            download_param,
            aes_key_hex,
            file_size: rawsize,
            file_size_ciphertext: filesize,
        })
    }

    fn send_image_message(
        &self,
        chat_id: &str,
        context_token: &str,
        uploaded: &WechatUploadedMedia,
    ) -> Result<()> {
        let msg = json!({
            "from_user_id": "",
            "to_user_id": chat_id,
            "client_id": client_id(),
            "message_type": MESSAGE_TYPE_BOT,
            "message_state": MESSAGE_STATE_FINISH,
            "item_list": [{
                "type": MESSAGE_ITEM_IMAGE,
                "image_item": {
                    "media": {
                        "encrypt_query_param": uploaded.download_param,
                        "aes_key": encode_ilink_media_aes_key(&uploaded.aes_key_hex)?,
                        "encrypt_type": 1,
                    },
                    "mid_size": uploaded.file_size_ciphertext,
                },
            }],
            "context_token": context_token,
        });
        self.client.api_post(
            "/ilink/bot/sendmessage",
            &json!({ "msg": msg, "base_info": base_info() }),
            15,
        )?;
        Ok(())
    }

    fn send_file_message(
        &self,
        chat_id: &str,
        context_token: &str,
        uploaded: &WechatUploadedMedia,
        file_name: &str,
    ) -> Result<()> {
        let msg = json!({
            "from_user_id": "",
            "to_user_id": chat_id,
            "client_id": client_id(),
            "message_type": MESSAGE_TYPE_BOT,
            "message_state": MESSAGE_STATE_FINISH,
            "item_list": [{
                "type": MESSAGE_ITEM_FILE,
                "file_item": {
                    "media": {
                        "encrypt_query_param": uploaded.download_param,
                        "aes_key": encode_ilink_media_aes_key(&uploaded.aes_key_hex)?,
                        "encrypt_type": 1,
                    },
                    "file_name": file_name,
                    "len": uploaded.file_size.to_string(),
                },
            }],
            "context_token": context_token,
        });
        self.client.api_post(
            "/ilink/bot/sendmessage",
            &json!({ "msg": msg, "base_info": base_info() }),
            15,
        )?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct WechatUploadedMedia {
    download_param: String,
    aes_key_hex: String,
    file_size: usize,
    file_size_ciphertext: usize,
}

#[derive(Debug, Deserialize)]
struct WechatUploadUrlResponse {
    #[serde(default)]
    upload_param: Option<String>,
}

impl ChatSink for WechatSink {
    fn platform(&self) -> &'static str {
        PLATFORM
    }

    fn send_text(&self, chat_id: &str, text: &str) -> Result<()> {
        let context_token = self
            .context_token(chat_id)
            .ok_or_else(|| anyhow!("WeChat requires a recent inbound message before replying"))?;
        for chunk in split_wechat_text(text) {
            self.client.send_text(chat_id, &context_token, &chunk)?;
        }
        Ok(())
    }

    fn send_image(&self, chat_id: &str, path: &Path, _caption: Option<&str>) -> Result<()> {
        let context_token = self
            .context_token(chat_id)
            .ok_or_else(|| anyhow!("WeChat requires a recent inbound message before replying"))?;
        let bytes =
            std::fs::read(path).with_context(|| format!("read image {}", path.display()))?;
        let uploaded = self.upload_media(&bytes, chat_id, UPLOAD_MEDIA_TYPE_IMAGE)?;
        self.send_image_message(chat_id, &context_token, &uploaded)
    }

    fn send_file(&self, chat_id: &str, path: &Path, _caption: Option<&str>) -> Result<()> {
        let context_token = self
            .context_token(chat_id)
            .ok_or_else(|| anyhow!("WeChat requires a recent inbound message before replying"))?;
        let bytes = std::fs::read(path).with_context(|| format!("read file {}", path.display()))?;
        let uploaded = self.upload_media(&bytes, chat_id, UPLOAD_MEDIA_TYPE_FILE)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file");
        self.send_file_message(chat_id, &context_token, &uploaded, file_name)
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
        self.send_text(chat_id, &request.fallback_text())?;
        Ok(None)
    }
}

pub struct WechatIlinkClient {
    client: Client,
    base_url: String,
    bot_token: Option<String>,
}

impl WechatIlinkClient {
    fn for_login(base_url: Option<&str>) -> Result<Self> {
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(45))
            .build()
            .context("build WeChat iLink HTTP client")?;
        Ok(Self {
            client,
            base_url: normalized_base_url(base_url),
            bot_token: None,
        })
    }

    fn for_token(bot_token: &str, base_url: Option<&str>, poll_timeout_secs: u64) -> Result<Self> {
        let bot_token = bot_token.trim();
        if bot_token.is_empty() {
            bail!("WeChat iLink bot token is required");
        }
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(poll_timeout_secs.max(5) + 15))
            .build()
            .context("build WeChat iLink HTTP client")?;
        Ok(Self {
            client,
            base_url: normalized_base_url(base_url),
            bot_token: Some(bot_token.to_string()),
        })
    }

    fn get_qrcode(&self) -> Result<WechatQrCode> {
        let response = self
            .client
            .post(format!(
                "{}/ilink/bot/get_bot_qrcode?bot_type=3",
                FIXED_QR_BASE_URL
            ))
            .headers(login_headers())
            .json(&json!({ "local_token_list": [] }))
            .send()
            .context("request WeChat QR code")?
            .error_for_status()
            .context("WeChat QR code returned HTTP error")?
            .json::<WechatQrCodeRaw>()
            .context("parse WeChat QR code response")?;
        if response.qrcode.trim().is_empty() {
            bail!("WeChat QR response missing qrcode");
        }
        let qrcode_content = response
            .qrcode_img_content
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .context("WeChat QR response missing qrcode_img_content")?;
        Ok(WechatQrCode {
            qrcode_id: response.qrcode.trim().to_string(),
            qrcode_content,
            qrcode_image_content: None,
        })
    }

    fn poll_qrcode_status(&self, qrcode: &str) -> Result<WechatQrStatus> {
        self.poll_qrcode_status_with_verify_code(qrcode, None)
    }

    fn poll_qrcode_status_with_verify_code(
        &self,
        qrcode: &str,
        verify_code: Option<&str>,
    ) -> Result<WechatQrStatus> {
        let qrcode = qrcode.trim();
        if qrcode.is_empty() {
            bail!("qrcode is required");
        }
        let mut url = format!(
            "{}/ilink/bot/get_qrcode_status?qrcode={}",
            self.base_url,
            percent_encode(qrcode)
        );
        if let Some(code) = verify_code.map(str::trim).filter(|value| !value.is_empty()) {
            url.push_str("&verify_code=");
            url.push_str(&percent_encode(code));
        }
        let response = self
            .client
            .get(url)
            .headers(login_headers())
            .timeout(Duration::from_secs(40))
            .send()
            .context("poll WeChat QR status")?
            .error_for_status()
            .context("WeChat QR status returned HTTP error")?
            .json::<WechatQrStatusRaw>()
            .context("parse WeChat QR status response")?;
        Ok(WechatQrStatus::from_raw(response, &self.base_url))
    }

    fn get_config(&self) -> Result<Value> {
        self.api_post(
            "/ilink/bot/getconfig",
            &json!({ "base_info": base_info() }),
            10,
        )
    }

    fn get_updates(&self, cursor: &str, timeout_secs: u64) -> Result<WechatUpdatesResponse> {
        let value = self.api_post(
            "/ilink/bot/getupdates",
            &json!({
                "get_updates_buf": cursor,
                "base_info": base_info(),
            }),
            timeout_secs.max(5) + 5,
        )?;
        serde_json::from_value::<WechatUpdatesResponse>(value)
            .context("parse WeChat getupdates response")
    }

    fn send_text(&self, chat_id: &str, context_token: &str, text: &str) -> Result<()> {
        let msg = json!({
            "from_user_id": "",
            "to_user_id": chat_id,
            "client_id": client_id(),
            "message_type": MESSAGE_TYPE_BOT,
            "message_state": MESSAGE_STATE_FINISH,
            "item_list": [{
                "type": MESSAGE_ITEM_TEXT,
                "text_item": { "text": text },
            }],
            "context_token": context_token,
        });
        self.api_post(
            "/ilink/bot/sendmessage",
            &json!({
                "msg": msg,
                "base_info": base_info(),
            }),
            15,
        )
        .map(|_| ())
    }

    fn api_post(&self, endpoint: &str, body: &Value, timeout_secs: u64) -> Result<Value> {
        let token = self
            .bot_token
            .as_deref()
            .context("WeChat iLink bot token is not configured")?;
        let response = self
            .client
            .post(format!("{}{}", self.base_url, endpoint))
            .timeout(Duration::from_secs(timeout_secs))
            .header("Content-Type", "application/json")
            .header("AuthorizationType", "ilink_bot_token")
            .header("Authorization", format!("Bearer {token}"))
            .header("X-WECHAT-UIN", random_wechat_uin())
            .json(body)
            .send()
            .with_context(|| format!("WeChat {endpoint} request failed"))?;
        let status = response.status();
        let text = response.text().unwrap_or_default();
        if !status.is_success() {
            bail!(
                "WeChat {endpoint} returned HTTP {status}: {}",
                redact_body(&text)
            );
        }
        let value = serde_json::from_str::<Value>(&text)
            .with_context(|| format!("parse WeChat {endpoint} response"))?;
        let code = value
            .get("errcode")
            .and_then(Value::as_i64)
            .filter(|code| *code != 0)
            .or_else(|| {
                value
                    .get("ret")
                    .and_then(Value::as_i64)
                    .filter(|code| *code != 0)
            });
        if let Some(code) = code {
            let message = value
                .get("errmsg")
                .or_else(|| value.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("WeChat API error");
            bail!("WeChat {endpoint} ret={code}: {message}");
        }
        Ok(value)
    }

    /// Typed convenience over [`api_post`] for callers that want a specific
    /// response shape instead of a free-form [`Value`].
    fn api_post_json<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        body: &Value,
        timeout_secs: u64,
    ) -> Result<T> {
        let value = self.api_post(endpoint, body, timeout_secs)?;
        serde_json::from_value(value)
            .with_context(|| format!("parse WeChat {endpoint} typed response"))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatQrCode {
    pub qrcode_id: String,
    pub qrcode_content: String,
    pub qrcode_image_content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatQrStatus {
    pub status: String,
    pub bot_token: Option<String>,
    pub bot_id: Option<String>,
    pub user_id: Option<String>,
    pub base_url: Option<String>,
    pub redirect_host: Option<String>,
    pub error: Option<String>,
}

impl WechatQrStatus {
    fn from_raw(raw: WechatQrStatusRaw, default_base_url: &str) -> Self {
        let status = match raw.status.as_str() {
            "wait" => "waiting",
            "scaned" => "scanned",
            "confirmed" => "confirmed",
            "expired" => "expired",
            "scaned_but_redirect" => "scannedRedirect",
            "need_verifycode" => "needVerifyCode",
            "verify_code_blocked" => "verifyCodeBlocked",
            "binded_redirect" => "alreadyConnected",
            other => other,
        }
        .to_string();
        let error = if status == "confirmed" && raw.bot_token.as_deref().unwrap_or("").is_empty() {
            Some("login confirmed but bot_token is missing".to_string())
        } else if status == "needVerifyCode" {
            Some("WeChat requires a phone verification code; Sessio does not support this QR login step yet.".to_string())
        } else if status == "verifyCodeBlocked" {
            Some("WeChat verification code attempts were blocked. Please request a new QR code later.".to_string())
        } else {
            None
        };
        Self {
            status,
            bot_token: raw.bot_token,
            bot_id: raw.ilink_bot_id,
            user_id: raw.ilink_user_id,
            base_url: raw
                .baseurl
                .filter(|value| !value.trim().is_empty())
                .or_else(|| Some(default_base_url.to_string())),
            redirect_host: raw.redirect_host.filter(|value| !value.trim().is_empty()),
            error,
        }
    }
}

#[derive(Debug, Deserialize)]
struct WechatQrCodeRaw {
    qrcode: String,
    qrcode_img_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WechatQrStatusRaw {
    status: String,
    bot_token: Option<String>,
    ilink_bot_id: Option<String>,
    ilink_user_id: Option<String>,
    baseurl: Option<String>,
    redirect_host: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WechatUpdatesResponse {
    #[serde(default)]
    msgs: Vec<WechatWireMessage>,
    #[serde(default)]
    get_updates_buf: String,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatWireMessage {
    #[serde(default)]
    from_user_id: String,
    #[serde(default)]
    to_user_id: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    message_id: Option<i64>,
    #[serde(default)]
    message_type: i64,
    #[serde(default)]
    context_token: Option<String>,
    #[serde(default)]
    item_list: Vec<WechatMessageItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatMessageItem {
    #[serde(default, rename = "type")]
    item_type: i64,
    text_item: Option<WechatTextItem>,
    voice_item: Option<WechatVoiceItem>,
    image_item: Option<WechatImageItem>,
    file_item: Option<WechatFileItem>,
    ref_msg: Option<WechatRefMessage>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatTextItem {
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatVoiceItem {
    text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatImageItem {
    media: Option<WechatMediaPayload>,
    /// Some inbound messages carry the AES key as a raw hex string under
    /// `aeskey` instead of embedding it inside `media.aes_key`.
    aeskey: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatFileItem {
    media: Option<WechatMediaPayload>,
    file_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatMediaPayload {
    #[serde(default)]
    encrypt_query_param: Option<String>,
    #[serde(default)]
    aes_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct WechatRefMessage {
    title: Option<String>,
    message_item: Option<Box<WechatMessageItem>>,
}

fn random_wechat_uin() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let value = ((nanos ^ (nanos >> 32)) & u32::MAX as u128) as u32;
    base64::engine::general_purpose::STANDARD.encode(value.to_string())
}

fn base_info() -> Value {
    json!({ "channel_version": env!("CARGO_PKG_VERSION") })
}

fn login_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("iLink-App-ClientVersion", HeaderValue::from_static("1"));
    headers
}

fn client_id() -> String {
    let now = now_ms();
    format!("sessio-wechat-{now}")
}

fn split_wechat_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if current.chars().count() >= WECHAT_TEXT_LIMIT {
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

fn normalized_base_url(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Mirror of openhanako's `createIlinkMediaAesKey` — a fresh 16-byte key used
/// to AES-128-ECB encrypt a single CDN upload.
fn create_ilink_media_aes_key() -> [u8; 16] {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut bytes = [0u8; 16];
    let mut hasher = Sha256::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    hasher.update(nanos.to_be_bytes());
    hasher.update(std::process::id().to_be_bytes());
    // Use the OS-level rng-ish source: address of a stack value mixed with
    // nanos already gives reasonable variability for a per-message key.
    let stack_marker = &nanos as *const _ as usize;
    hasher.update(stack_marker.to_be_bytes());
    let digest = hasher.finalize();
    bytes.copy_from_slice(&digest[..16]);
    bytes
}

/// iLink encodes its 16-byte raw key as the **base64 of the 32-char lowercase
/// hex string of the key**. Decoding accepts either form.
fn encode_ilink_media_aes_key(aes_key_hex: &str) -> Result<String> {
    let normalized = normalize_ilink_aes_key_hex(aes_key_hex)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(normalized.as_bytes()))
}

fn decode_ilink_media_aes_key(aes_key_base64: &str) -> Result<[u8; 16]> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(aes_key_base64.as_bytes())
        .context("invalid base64 in iLink media aes_key")?;
    match decoded.len() {
        16 => {
            let mut out = [0u8; 16];
            out.copy_from_slice(&decoded);
            Ok(out)
        }
        32 => {
            // hex-string form
            let hex_str = std::str::from_utf8(&decoded)
                .context("iLink media aes_key hex is not valid UTF-8")?;
            let normalized = normalize_ilink_aes_key_hex(hex_str)?;
            let mut out = [0u8; 16];
            hex::decode_to_slice(&normalized, &mut out)
                .context("invalid hex in iLink media aes_key")?;
            Ok(out)
        }
        other => Err(anyhow!(
            "invalid WeChat iLink media aes_key length: {other}"
        )),
    }
}

fn normalize_ilink_aes_key_hex(value: &str) -> Result<String> {
    let lowercase = value.trim().to_ascii_lowercase();
    if lowercase.len() != 32 || !lowercase.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("iLink media aes_key must be 32 hex characters");
    }
    Ok(lowercase)
}

fn aes_ecb_padded_size(plaintext_size: usize) -> usize {
    ((plaintext_size + 1).div_ceil(16)) * 16
}

type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

fn encrypt_aes128_ecb(plaintext: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let enc = Aes128EcbEnc::new(key.into());
    enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext)
}

fn decrypt_aes128_ecb(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>> {
    let dec = Aes128EcbDec::new(key.into());
    dec.decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|error| anyhow!("AES-128-ECB decrypt failed: {error}"))
}

fn normalized_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn sync_cursor_path(bot_token: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let mut hasher = Sha256::new();
    hasher.update(bot_token.trim().as_bytes());
    let digest = hex::encode(hasher.finalize());
    Some(
        home.join(".sessio")
            .join("im-bridge")
            .join(format!("wechat-sync-{}.json", &digest[..12])),
    )
}

fn load_cursor(path: &PathBuf) -> Result<String> {
    let text = std::fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&text)?;
    Ok(value
        .get("getUpdatesBuf")
        .or_else(|| value.get("get_updates_buf"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn save_cursor(path: &PathBuf, cursor: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string(&json!({ "getUpdatesBuf": cursor }))?;
    std::fs::write(path, data)?;
    Ok(())
}

fn redact_body(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() > 300 {
        let mut value = trimmed.chars().take(300).collect::<String>();
        value.push_str("...");
        value
    } else {
        trimmed.to_string()
    }
}

fn is_session_expired_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("ret=-14") || message.contains("errcode=-14")
}

fn wechat_display_name(chat_id: &str) -> String {
    let prefix = chat_id.split('@').next().unwrap_or(chat_id);
    if prefix.trim().is_empty() {
        "WeChat".to_string()
    } else {
        prefix.to_string()
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
