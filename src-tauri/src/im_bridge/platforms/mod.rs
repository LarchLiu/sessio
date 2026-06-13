//! Platform workers for the IM bridge.

use std::sync::Arc;

use super::state::ImBridgeState;

mod discord;
mod feishu;
mod telegram;
mod wechat;

pub fn spawn_all(state: Arc<ImBridgeState>) {
    telegram::spawn(state.clone());
    discord::spawn(state.clone());
    feishu::spawn(state.clone());
    wechat::spawn(state);
}

pub fn detect_telegram_user_ids(
    bot_token: &str,
    api_base: Option<&str>,
) -> anyhow::Result<Vec<i64>> {
    telegram::detect_user_ids(bot_token, api_base)
}

pub fn test_telegram_bot_connection(bot_token: &str, api_base: Option<&str>) -> anyhow::Result<()> {
    telegram::test_connection(bot_token, api_base)
}

pub fn test_discord_bot_connection(bot_token: &str, api_base: Option<&str>) -> anyhow::Result<()> {
    discord::test_connection(bot_token, api_base)
}

pub fn test_feishu_bot_connection(
    app_id: &str,
    app_secret: &str,
    domain: Option<&str>,
) -> anyhow::Result<()> {
    feishu::test_connection(app_id, app_secret, domain)
}

pub fn test_wechat_bot_connection(bot_token: &str, base_url: Option<&str>) -> anyhow::Result<()> {
    wechat::test_connection(bot_token, base_url)
}

pub fn get_wechat_qrcode(base_url: Option<&str>) -> anyhow::Result<wechat::WechatQrCode> {
    wechat::get_qrcode(base_url)
}

pub fn poll_wechat_qrcode_status(
    qrcode: &str,
    base_url: Option<&str>,
) -> anyhow::Result<wechat::WechatQrStatus> {
    wechat::poll_qrcode_status(qrcode, base_url)
}
