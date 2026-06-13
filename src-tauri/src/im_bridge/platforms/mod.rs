//! Platform workers for the IM bridge.

use std::sync::Arc;

use super::state::ImBridgeState;

mod discord;
mod telegram;

pub fn spawn_all(state: Arc<ImBridgeState>) {
    telegram::spawn(state.clone());
    discord::spawn(state);
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
