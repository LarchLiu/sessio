use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub(super) struct RawConfig {
    pub(super) memory: Option<RawMemoryConfig>,
    pub(super) index: RawIndexConfig,
    pub(super) network: RawNetworkConfig,
    pub(super) mcp: RawMcpConfig,
    pub(super) appshot: RawAppshotConfig,
    pub(super) computer_use: RawComputerUseConfig,
    pub(super) debug: RawDebugConfig,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawIndexConfig {
    pub(super) poll_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawNetworkConfig {
    pub(super) proxy: RawNetworkProxyConfig,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawNetworkProxyConfig {
    pub(super) enabled: Option<bool>,
    pub(super) url: Option<String>,
    pub(super) no_proxy: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawMcpConfig {
    pub(super) legacy_custom_servers: Option<Vec<crate::mcp::McpServerConfig>>,
    pub(super) servers: BTreeMap<String, RawMcpServerConfig>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawMcpServerConfig {
    pub(super) name: Option<String>,
    pub(super) builtin: Option<String>,
    pub(super) transport: Option<String>,
    pub(super) enabled: Option<bool>,
    pub(super) description: Option<String>,
    pub(super) url: Option<String>,
    pub(super) headers: Option<Vec<String>>,
    pub(super) command: Option<String>,
    pub(super) args: Option<Vec<String>>,
    pub(super) env: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawAppshotConfig {
    pub(super) shortcut: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawComputerUseConfig {
    pub(super) enabled: Option<bool>,
    pub(super) mcp_description: Option<String>,
    pub(super) approved_apps: Option<Vec<String>>,
    pub(super) app_route_preferences:
        Option<BTreeMap<String, crate::computer_use::settings::AppRoutePreferences>>,
    pub(super) allow_input_injection: Option<bool>,
    pub(super) allow_foreground_takeover: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawDebugConfig {
    pub(super) acp_config: Option<bool>,
    pub(super) update_preview: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawMemoryConfig {
    pub(super) backend: Option<String>,
    pub(super) backends: RawMemoryBackends,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawMemoryBackends {
    pub(super) qmd: RawQmdBackendConfig,
}

#[derive(Debug, Clone, Default)]
pub(super) struct RawQmdBackendConfig {
    pub(super) binary: Option<String>,
    pub(super) index: Option<String>,
    pub(super) artifacts_root: Option<String>,
    pub(super) auto_embed: Option<bool>,
    pub(super) install_command: Option<String>,
}
