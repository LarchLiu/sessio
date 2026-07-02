use std::collections::HashSet;
use std::sync::RwLock;

use agent_client_protocol::schema::v1::{
    EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerSse, McpServerStdio,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::agents::runtime::types::{ComputerUseInjection, RuntimeCapabilitySet};
use crate::computer_use::settings::ComputerUseSettings;
use crate::config;

const BUILTIN_COMPUTER_USE_ID: &str = "builtin:computer-use";
const BUILTIN_COMPUTER_USE_NAME: &str = "Sessio Computer Use";
const BUILTIN_COMPUTER_USE_SERVER_NAME: &str = "sessio-computer-use";
pub const SELECTED_MCP_IDS_OPTION: &str = "selectedMcpIds";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpServerSource {
    Builtin,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpServerTransport {
    Http,
    Sse,
    Stdio,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum McpServerInjectionMode {
    Always,
    SessionOptIn,
}

impl Default for McpServerInjectionMode {
    fn default() -> Self {
        Self::SessionOptIn
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinMcpKind {
    ComputerUse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpKeyValue {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source: McpServerSource,
    pub transport: McpServerTransport,
    #[serde(default)]
    pub injection_mode: McpServerInjectionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_kind: Option<BuiltinMcpKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: Vec<McpKeyValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<McpKeyValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

#[derive(Default)]
pub struct McpSettingsCache {
    inner: RwLock<McpSettings>,
}

impl McpSettingsCache {
    pub fn get(&self) -> McpSettings {
        self.inner
            .read()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn set(&self, settings: McpSettings) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = settings;
        }
    }

    pub fn refresh_from_disk(&self) -> Result<McpSettings> {
        let settings = load_settings()?;
        self.set(settings.clone());
        Ok(settings)
    }
}

pub fn load_settings() -> Result<McpSettings> {
    let app_config = config::load_config()?;
    Ok(merged_settings(&app_config.mcp, &app_config.computer_use))
}

pub fn save_settings(settings: McpSettings) -> Result<McpSettings> {
    let mut app_config = config::load_config()?;
    app_config.mcp = normalize_custom_settings(settings)?;
    config::save_config(&app_config)?;
    Ok(merged_settings(&app_config.mcp, &app_config.computer_use))
}

pub fn normalize_custom_settings(settings: McpSettings) -> Result<McpSettings> {
    let mut seen_ids = HashSet::new();
    let mut servers = Vec::new();
    for server in settings.servers {
        if server.source != McpServerSource::Custom {
            continue;
        }
        let server = normalize_custom_server(server)?;
        if server.id == BUILTIN_COMPUTER_USE_ID {
            bail!("`{BUILTIN_COMPUTER_USE_ID}` is reserved for built-in MCP servers");
        }
        if !seen_ids.insert(server.id.clone()) {
            bail!("duplicate MCP server id: {}", server.id);
        }
        servers.push(server);
    }
    Ok(McpSettings { servers })
}

pub fn selected_session_servers(
    settings: &McpSettings,
    capabilities: Option<&RuntimeCapabilitySet>,
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> Result<Vec<McpServer>> {
    let selected_ids = selected_mcp_ids_from_options(options);
    if selected_ids.is_empty() {
        return Ok(Vec::new());
    }
    let selectable_servers = selectable_custom_servers(settings, capabilities);
    let mut out = Vec::new();
    for id in selected_ids {
        let Some(server) = selectable_servers.iter().find(|server| server.id == id) else {
            continue;
        };
        out.push(configured_server_to_mcp_server(server)?);
    }
    Ok(out)
}

pub fn computer_use_server_entry(computer_use: &ComputerUseSettings) -> McpServerConfig {
    McpServerConfig {
        id: BUILTIN_COMPUTER_USE_ID.to_string(),
        name: BUILTIN_COMPUTER_USE_NAME.to_string(),
        enabled: computer_use.enabled,
        source: McpServerSource::Builtin,
        transport: McpServerTransport::Http,
        injection_mode: McpServerInjectionMode::SessionOptIn,
        builtin_kind: Some(BuiltinMcpKind::ComputerUse),
        url: None,
        headers: Vec::new(),
        command: None,
        args: Vec::new(),
        env: Vec::new(),
    }
}

pub fn computer_use_runtime_server(injection: &ComputerUseInjection) -> McpServer {
    McpServer::Http(
        McpServerHttp::new(BUILTIN_COMPUTER_USE_SERVER_NAME, injection.url.clone()).headers(vec![
            HttpHeader::new(
                "Authorization",
                format!("Bearer {}", injection.bearer_token),
            ),
        ]),
    )
}

pub fn merged_settings(custom: &McpSettings, computer_use: &ComputerUseSettings) -> McpSettings {
    let mut servers = vec![computer_use_server_entry(computer_use)];
    servers.extend(custom.servers.clone());
    McpSettings { servers }
}

fn normalize_custom_server(server: McpServerConfig) -> Result<McpServerConfig> {
    let id = server.id.trim().to_string();
    if id.is_empty() {
        bail!("MCP server id is required");
    }
    let name = server.name.trim().to_string();
    if name.is_empty() {
        bail!("MCP server name is required");
    }

    let headers = normalize_entries(server.headers);
    let env = normalize_entries(server.env);
    let args = server
        .args
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    Ok(match server.transport {
        McpServerTransport::Http | McpServerTransport::Sse => {
            let Some(url) = trimmed_option(server.url.as_deref()) else {
                bail!("MCP server `{name}` requires a URL");
            };
            McpServerConfig {
                id,
                name,
                enabled: server.enabled,
                source: McpServerSource::Custom,
                transport: server.transport,
                injection_mode: McpServerInjectionMode::SessionOptIn,
                builtin_kind: None,
                url: Some(url),
                headers,
                command: None,
                args: Vec::new(),
                env: Vec::new(),
            }
        }
        McpServerTransport::Stdio => {
            let Some(command) = trimmed_option(server.command.as_deref()) else {
                bail!("MCP server `{name}` requires a command");
            };
            McpServerConfig {
                id,
                name,
                enabled: server.enabled,
                source: McpServerSource::Custom,
                transport: McpServerTransport::Stdio,
                injection_mode: McpServerInjectionMode::SessionOptIn,
                builtin_kind: None,
                url: None,
                headers: Vec::new(),
                command: Some(command),
                args,
                env,
            }
        }
    })
}

fn normalize_entries(entries: Vec<McpKeyValue>) -> Vec<McpKeyValue> {
    entries
        .into_iter()
        .map(|entry| McpKeyValue {
            name: entry.name.trim().to_string(),
            value: entry.value.trim().to_string(),
        })
        .filter(|entry| !entry.name.is_empty())
        .collect()
}

fn selected_mcp_ids_from_options(
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> Vec<String> {
    let Some(values) = options
        .get(SELECTED_MCP_IDS_OPTION)
        .or_else(|| options.get("selected_mcp_ids"))
        .and_then(|value| value.as_array())
    else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    values
        .iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(|value| {
            let owned = value.to_string();
            seen.insert(owned.clone()).then_some(owned)
        })
        .collect()
}

fn selectable_custom_servers<'a>(
    settings: &'a McpSettings,
    capabilities: Option<&RuntimeCapabilitySet>,
) -> Vec<&'a McpServerConfig> {
    settings
        .servers
        .iter()
        .filter(|server| {
            server.source == McpServerSource::Custom
                && server.enabled
                && transport_supported(server.transport, capabilities)
        })
        .collect()
}

fn configured_server_to_mcp_server(server: &McpServerConfig) -> Result<McpServer> {
    Ok(match server.transport {
        McpServerTransport::Http => McpServer::Http(
            McpServerHttp::new(server.name.clone(), required_string(server.url.as_deref())?)
                .headers(http_headers(&server.headers)),
        ),
        McpServerTransport::Sse => McpServer::Sse(
            McpServerSse::new(server.name.clone(), required_string(server.url.as_deref())?)
                .headers(http_headers(&server.headers)),
        ),
        McpServerTransport::Stdio => McpServer::Stdio(
            McpServerStdio::new(
                server.name.clone(),
                config::expand_path(required_string(server.command.as_deref())?)
                    .with_context(|| format!("expand MCP command for {}", server.name))?,
            )
            .args(server.args.clone())
            .env(env_vars(&server.env)),
        ),
    })
}

fn http_headers(entries: &[McpKeyValue]) -> Vec<HttpHeader> {
    entries
        .iter()
        .map(|entry| HttpHeader::new(entry.name.clone(), entry.value.clone()))
        .collect()
}

fn env_vars(entries: &[McpKeyValue]) -> Vec<EnvVariable> {
    entries
        .iter()
        .map(|entry| EnvVariable::new(entry.name.clone(), entry.value.clone()))
        .collect()
}

fn required_string(value: Option<&str>) -> Result<&str> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .context("missing required MCP value")
}

fn trimmed_option(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn transport_supported(
    transport: McpServerTransport,
    capabilities: Option<&RuntimeCapabilitySet>,
) -> bool {
    match transport {
        McpServerTransport::Http => capabilities
            .map(|caps| caps.mcp_injection.http)
            .unwrap_or(false),
        McpServerTransport::Sse => capabilities
            .map(|caps| caps.mcp_injection.sse)
            .unwrap_or(false),
        McpServerTransport::Stdio => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::runtime::types::McpInjectionCapabilities;
    use serde_json::json;

    fn caps(http: bool, sse: bool) -> RuntimeCapabilitySet {
        let mut capabilities = RuntimeCapabilitySet::fake();
        capabilities.mcp_injection = McpInjectionCapabilities {
            http,
            sse,
            acp: false,
            native_extension: false,
        };
        capabilities
    }

    #[test]
    fn normalize_settings_keeps_only_custom_servers() {
        let settings = normalize_custom_settings(McpSettings {
            servers: vec![
                computer_use_server_entry(&ComputerUseSettings::recommended()),
                McpServerConfig {
                    id: "custom-1".into(),
                    name: "Docs".into(),
                    enabled: true,
                    source: McpServerSource::Custom,
                    transport: McpServerTransport::Http,
                    injection_mode: McpServerInjectionMode::SessionOptIn,
                    builtin_kind: Some(BuiltinMcpKind::ComputerUse),
                    url: Some("http://127.0.0.1:8123/mcp".into()),
                    headers: vec![McpKeyValue {
                        name: "X-Test".into(),
                        value: "1".into(),
                    }],
                    command: Some("ignored".into()),
                    args: vec!["ignored".into()],
                    env: vec![McpKeyValue {
                        name: "IGNORED".into(),
                        value: "1".into(),
                    }],
                },
            ],
        })
        .unwrap();

        assert_eq!(settings.servers.len(), 1);
        let server = &settings.servers[0];
        assert_eq!(server.id, "custom-1");
        assert_eq!(server.injection_mode, McpServerInjectionMode::SessionOptIn);
        assert_eq!(server.url.as_deref(), Some("http://127.0.0.1:8123/mcp"));
        assert!(server.command.is_none());
        assert!(server.args.is_empty());
        assert!(server.env.is_empty());
        assert!(server.builtin_kind.is_none());
    }

    #[test]
    fn selected_session_servers_inject_only_selected_ids() {
        let settings = McpSettings {
            servers: vec![
                McpServerConfig {
                    id: "http".into(),
                    name: "HTTP".into(),
                    enabled: true,
                    source: McpServerSource::Custom,
                    transport: McpServerTransport::Http,
                    injection_mode: McpServerInjectionMode::SessionOptIn,
                    builtin_kind: None,
                    url: Some("http://127.0.0.1:8123/mcp".into()),
                    headers: Vec::new(),
                    command: None,
                    args: Vec::new(),
                    env: Vec::new(),
                },
                McpServerConfig {
                    id: "sse".into(),
                    name: "SSE".into(),
                    enabled: true,
                    source: McpServerSource::Custom,
                    transport: McpServerTransport::Sse,
                    injection_mode: McpServerInjectionMode::SessionOptIn,
                    builtin_kind: None,
                    url: Some("http://127.0.0.1:9000/sse".into()),
                    headers: Vec::new(),
                    command: None,
                    args: Vec::new(),
                    env: Vec::new(),
                },
                McpServerConfig {
                    id: "stdio".into(),
                    name: "Stdio".into(),
                    enabled: true,
                    source: McpServerSource::Custom,
                    transport: McpServerTransport::Stdio,
                    injection_mode: McpServerInjectionMode::SessionOptIn,
                    builtin_kind: None,
                    url: None,
                    headers: Vec::new(),
                    command: Some("/bin/echo".into()),
                    args: vec!["ok".into()],
                    env: Vec::new(),
                },
            ],
        };

        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCP_IDS_OPTION.to_string(),
            json!(["stdio", "http", "missing"]),
        );

        let selected =
            selected_session_servers(&settings, Some(&caps(true, false)), &options).unwrap();
        assert_eq!(selected.len(), 2);
        assert!(matches!(selected[0], McpServer::Stdio(_)));
        assert!(matches!(selected[1], McpServer::Http(_)));
    }

    #[test]
    fn selected_session_servers_ignore_unsupported_transports() {
        let settings = McpSettings {
            servers: vec![
                McpServerConfig {
                    id: "http".into(),
                    name: "HTTP".into(),
                    enabled: true,
                    source: McpServerSource::Custom,
                    transport: McpServerTransport::Http,
                    injection_mode: McpServerInjectionMode::SessionOptIn,
                    builtin_kind: None,
                    url: Some("http://127.0.0.1:8123/mcp".into()),
                    headers: Vec::new(),
                    command: None,
                    args: Vec::new(),
                    env: Vec::new(),
                },
                McpServerConfig {
                    id: "stdio".into(),
                    name: "Stdio".into(),
                    enabled: true,
                    source: McpServerSource::Custom,
                    transport: McpServerTransport::Stdio,
                    injection_mode: McpServerInjectionMode::SessionOptIn,
                    builtin_kind: None,
                    url: None,
                    headers: Vec::new(),
                    command: Some("/bin/echo".into()),
                    args: vec!["ok".into()],
                    env: Vec::new(),
                },
            ],
        };
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCP_IDS_OPTION.to_string(),
            json!(["http", "stdio"]),
        );

        let selected =
            selected_session_servers(&settings, Some(&caps(false, false)), &options).unwrap();
        assert_eq!(selected.len(), 1);
        assert!(matches!(selected[0], McpServer::Stdio(_)));
    }
}
