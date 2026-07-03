use crate::{config, mcp, network};
use tauri::State;

#[tauri::command]
pub(crate) fn get_debug_config() -> Result<config::DebugConfig, String> {
    config::load_config()
        .map(|config| config.debug)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn get_network_config() -> Result<config::NetworkConfig, String> {
    network::load_network_config().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn update_network_config(
    config: config::NetworkConfig,
) -> Result<config::NetworkConfig, String> {
    network::save_network_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn get_mcp_settings(
    cache: State<'_, mcp::McpSettingsCache>,
) -> Result<mcp::McpSettings, String> {
    Ok(cache.get())
}

#[tauri::command]
pub(crate) fn get_appshot_config() -> Result<config::AppshotConfig, String> {
    config::load_config()
        .map(|config| config.appshot)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn take_config_recovery_notice() -> Option<config::ConfigRecoveryNotice> {
    config::take_config_recovery_notice()
}
