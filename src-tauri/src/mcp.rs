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

pub const BUILTIN_COMPUTER_USE_ID: &str = "builtin:computer-use";
pub const BUILTIN_COMPUTER_USE_CONFIG_KEY: &str = "computer_use";
const BUILTIN_COMPUTER_USE_NAME: &str = "Sessio Computer Use";
const BUILTIN_COMPUTER_USE_SERVER_NAME: &str = "sessio-computer-use";
pub const SELECTED_MCP_IDS_OPTION: &str = "selectedMcpIds";
pub const SELECTED_MCPS_OPTION: &str = "selectedMcps";
const BUILTIN_COMPUTER_USE_DESCRIPTION: &str = r#"Use for desktop observation and GUI control in native macOS apps. Prefer the exposed `computer_*` MCP tools over shell scripts.
Start with `computer_get_app_state`; use AX refs (`ref` / `elementId`) before screenshot coordinates.
If the target has no visible window or is Dock-minimized, call `computer_raise_app` for that bundle, then retry `computer_get_app_state`.
Avoid raw Swift/CoreGraphics/CGEvent, cliclick, `open -a`, or AppleScript mouse / activate fallbacks because they bypass Sessio approvals, snapshot coordinate mapping, post-action screenshots, and the pointer overlay."#;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum McpServerInjectionMode {
    Always,
    #[default]
    SessionOptIn,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SelectedMcpServerMetadata {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: McpServerSource,
    pub transport: McpServerTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_kind: Option<BuiltinMcpKind>,
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
    if let Some(server) = settings.servers.iter().find(|server| {
        server.source == McpServerSource::Builtin
            && server.builtin_kind == Some(BuiltinMcpKind::ComputerUse)
    }) {
        app_config.computer_use.enabled = server.enabled;
        app_config.computer_use.mcp_description = trimmed_option(server.description.as_deref());
    }
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
    mut builtin_runtime_server: impl FnMut(&McpServerConfig) -> Result<Option<McpServer>>,
) -> Result<Vec<McpServer>> {
    let selected_ids = selected_mcp_ids_from_options(options);
    if selected_ids.is_empty() {
        return Ok(Vec::new());
    }
    let selectable_servers = selectable_session_servers(settings, capabilities);
    let mut out = Vec::new();
    for id in selected_ids {
        let server = selectable_servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| (*server).clone());
        let Some(server) = server else {
            continue;
        };
        match server.source {
            McpServerSource::Builtin => {
                if let Some(server) = builtin_runtime_server(&server)? {
                    out.push(server);
                }
            }
            McpServerSource::Custom => {
                out.push(configured_server_to_mcp_server(&server)?);
            }
        }
    }
    Ok(out)
}

pub fn hydrate_selected_mcps_option(
    options: &mut crate::agents::runtime::types::RuntimeMetadata,
    settings: &McpSettings,
) {
    let selected_ids = selected_mcp_ids_from_options(options);
    if selected_ids.is_empty() {
        return;
    }
    let selected_mcps = selected_ids
        .iter()
        .filter_map(|id| {
            settings
                .servers
                .iter()
                .find(|server| server.id == *id && server.enabled)
                .map(selected_mcp_metadata)
        })
        .collect::<Vec<_>>();
    options.insert(
        SELECTED_MCP_IDS_OPTION.to_string(),
        serde_json::json!(selected_ids),
    );
    options.insert(
        SELECTED_MCPS_OPTION.to_string(),
        serde_json::to_value(selected_mcps).unwrap_or_else(|_| serde_json::json!([])),
    );
}

pub fn inject_selected_mcps_prompt_block(
    text: &str,
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> String {
    let Some(mcps) = selected_mcps_from_options(options) else {
        return text.to_string();
    };
    if mcps.is_empty() {
        return text.to_string();
    }
    prepend_mcps_prompt_block(text, &mcps)
}

pub fn selected_computer_use_server(
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> bool {
    selected_mcp_ids_from_options(options)
        .iter()
        .any(|id| id == BUILTIN_COMPUTER_USE_ID)
}

pub fn computer_use_server_entry(computer_use: &ComputerUseSettings) -> McpServerConfig {
    McpServerConfig {
        id: BUILTIN_COMPUTER_USE_ID.to_string(),
        name: BUILTIN_COMPUTER_USE_NAME.to_string(),
        description: Some(computer_use_description(computer_use).to_string()),
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

pub fn computer_use_description(computer_use: &ComputerUseSettings) -> &str {
    computer_use
        .mcp_description
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(BUILTIN_COMPUTER_USE_DESCRIPTION)
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
    if id == BUILTIN_COMPUTER_USE_CONFIG_KEY {
        bail!("`{BUILTIN_COMPUTER_USE_CONFIG_KEY}` is reserved for the built-in computer_use MCP server");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("MCP server id must use only ASCII letters, numbers, '-' or '_': {id}");
    }
    let name = server.name.trim().to_string();
    if name.is_empty() {
        bail!("MCP server name is required");
    }
    let description = trimmed_option(server.description.as_deref());

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
                description,
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
                description,
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

fn selected_mcps_from_options(
    options: &crate::agents::runtime::types::RuntimeMetadata,
) -> Option<Vec<SelectedMcpServerMetadata>> {
    let value = options
        .get(SELECTED_MCPS_OPTION)
        .or_else(|| options.get("selected_mcps"))?;
    serde_json::from_value::<Vec<SelectedMcpServerMetadata>>(value.clone()).ok()
}

fn selected_mcp_metadata(server: &McpServerConfig) -> SelectedMcpServerMetadata {
    SelectedMcpServerMetadata {
        id: server.id.clone(),
        name: server.name.clone(),
        description: server.description.clone(),
        source: server.source,
        transport: server.transport,
        builtin_kind: server.builtin_kind,
    }
}

fn prepend_mcps_prompt_block(text: &str, mcps: &[SelectedMcpServerMetadata]) -> String {
    if mcps.is_empty() {
        return text.to_string();
    }

    let nonce = uuid::Uuid::new_v4().to_string();
    let markers = crate::prompt_markers::sessio_prompt_markers();
    let mut block = String::new();
    block.push_str(&format!(
        "{} nonce=\"{nonce}\" kind=\"{}\" -->\n\n",
        markers.mcps_prompt_start, markers.selected_mcps_prompt_kind
    ));
    block.push_str(
        "Selected Sessio MCP servers are attached to this conversation.\nUse the metadata below to understand what each MCP is for before calling its tools. If an expected MCP tool is not visible in your available tools, say that the MCP is unavailable instead of assuming it exists.",
    );
    block.push_str("\n\n");
    for mcp in mcps {
        block.push_str(&render_mcp_metadata(mcp));
    }
    block.push_str(&format!(
        "\n{} nonce=\"{nonce}\" -->",
        markers.mcps_prompt_end
    ));
    if text.trim().is_empty() {
        block
    } else {
        format!("{block}\n\n{text}")
    }
}

fn render_mcp_metadata(mcp: &SelectedMcpServerMetadata) -> String {
    let mut lines = vec![format!("- `{}`", mcp.name)];
    lines.push(format!("  id: `{}`", mcp.id));
    lines.push(format!("  source: `{}`", mcp_source_label(mcp.source)));
    lines.push(format!(
        "  transport: `{}`",
        mcp_transport_label(mcp.transport)
    ));
    if let Some(kind) = mcp.builtin_kind {
        lines.push(format!("  builtinKind: `{}`", builtin_mcp_kind_label(kind)));
    }
    if let Some(description) = mcp
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push("  description:".to_string());
        lines.extend(
            description
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| format!("    {line}")),
        );
    }
    format!("{}\n", lines.join("\n"))
}

fn mcp_source_label(source: McpServerSource) -> &'static str {
    let markers = crate::prompt_markers::sessio_prompt_markers();
    match source {
        McpServerSource::Builtin => markers.mcp_source_builtin,
        McpServerSource::Custom => markers.mcp_source_custom,
    }
}

fn mcp_transport_label(transport: McpServerTransport) -> &'static str {
    match transport {
        McpServerTransport::Http => "http",
        McpServerTransport::Sse => "sse",
        McpServerTransport::Stdio => "stdio",
    }
}

fn builtin_mcp_kind_label(kind: BuiltinMcpKind) -> &'static str {
    let markers = crate::prompt_markers::sessio_prompt_markers();
    match kind {
        BuiltinMcpKind::ComputerUse => markers.builtin_mcp_kind_computer_use,
    }
}

fn selectable_session_servers<'a>(
    settings: &'a McpSettings,
    capabilities: Option<&RuntimeCapabilitySet>,
) -> Vec<&'a McpServerConfig> {
    settings
        .servers
        .iter()
        .filter(|server| server.enabled && transport_supported(server.transport, capabilities))
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
                    description: Some("Project docs".into()),
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
        assert_eq!(server.description.as_deref(), Some("Project docs"));
        assert_eq!(server.url.as_deref(), Some("http://127.0.0.1:8123/mcp"));
        assert!(server.command.is_none());
        assert!(server.args.is_empty());
        assert!(server.env.is_empty());
        assert!(server.builtin_kind.is_none());
    }

    #[test]
    fn hydrates_selected_mcps_from_ids_without_sensitive_config() {
        let settings = McpSettings {
            servers: vec![McpServerConfig {
                id: "docs".into(),
                name: "Docs".into(),
                description: Some("Look up project documentation.".into()),
                enabled: true,
                source: McpServerSource::Custom,
                transport: McpServerTransport::Http,
                injection_mode: McpServerInjectionMode::SessionOptIn,
                builtin_kind: None,
                url: Some("http://127.0.0.1:8123/mcp".into()),
                headers: vec![McpKeyValue {
                    name: "Authorization".into(),
                    value: "Bearer secret".into(),
                }],
                command: None,
                args: Vec::new(),
                env: Vec::new(),
            }],
        };
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(SELECTED_MCP_IDS_OPTION.to_string(), json!(["docs"]));

        hydrate_selected_mcps_option(&mut options, &settings);

        assert_eq!(
            options.get(SELECTED_MCPS_OPTION),
            Some(&json!([{
                "id": "docs",
                "name": "Docs",
                "description": "Look up project documentation.",
                "source": "custom",
                "transport": "http"
            }]))
        );
        let options_json = serde_json::to_string(&options).expect("options json");
        assert!(!options_json.contains("secret"));
        assert!(!options_json.contains("127.0.0.1"));
    }

    #[test]
    fn injects_selected_mcps_prompt_block() {
        let markers = crate::prompt_markers::sessio_prompt_markers();
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCPS_OPTION.to_string(),
            json!([{
                "id": BUILTIN_COMPUTER_USE_ID,
                "name": "Sessio Computer Use",
                "description": "Use for desktop observation and GUI control in native macOS apps.\nStart with `computer_get_app_state`.",
                "source": "builtin",
                "transport": "http",
                "builtinKind": "computerUse"
            }]),
        );

        let output = inject_selected_mcps_prompt_block("use the app", &options);

        assert!(output.contains(markers.mcps_prompt_start));
        assert!(output.contains(&format!("kind=\"{}\"", markers.selected_mcps_prompt_kind)));
        assert!(output.contains("Selected Sessio MCP servers are attached"));
        assert!(output.contains("id: `builtin:computer-use`"));
        assert!(output.contains("builtinKind: `computerUse`"));
        assert!(output.contains("computer_get_app_state"));
        assert!(output.ends_with("use the app"));
    }

    #[test]
    fn selected_session_servers_inject_only_selected_ids() {
        let settings = McpSettings {
            servers: vec![
                McpServerConfig {
                    id: "http".into(),
                    name: "HTTP".into(),
                    description: None,
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
                    description: None,
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
                    description: None,
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
            selected_session_servers(&settings, Some(&caps(true, false)), &options, |_| Ok(None))
                .unwrap();
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
                    description: None,
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
                    description: None,
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
            selected_session_servers(&settings, Some(&caps(false, false)), &options, |_| Ok(None))
                .unwrap();
        assert_eq!(selected.len(), 1);
        assert!(matches!(selected[0], McpServer::Stdio(_)));
    }

    #[test]
    fn selected_session_servers_resolves_builtin_servers() {
        let settings = McpSettings {
            servers: vec![computer_use_server_entry(&ComputerUseSettings {
                enabled: true,
                ..Default::default()
            })],
        };
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCP_IDS_OPTION.to_string(),
            json!([BUILTIN_COMPUTER_USE_ID]),
        );

        let selected =
            selected_session_servers(&settings, Some(&caps(true, false)), &options, |server| {
                assert_eq!(server.builtin_kind, Some(BuiltinMcpKind::ComputerUse));
                Ok(Some(computer_use_runtime_server(&ComputerUseInjection {
                    url: "http://127.0.0.1:1234/mcp".into(),
                    bearer_token: "token".into(),
                })))
            })
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert!(matches!(selected[0], McpServer::Http(_)));
    }

    #[test]
    fn selected_session_servers_skips_builtin_when_settings_are_missing() {
        let settings = McpSettings {
            servers: Vec::new(),
        };
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCP_IDS_OPTION.to_string(),
            json!([BUILTIN_COMPUTER_USE_ID]),
        );

        let selected =
            selected_session_servers(&settings, Some(&caps(true, false)), &options, |_| {
                panic!("unexpected builtin resolver call")
            })
            .unwrap();

        assert!(selected.is_empty());
    }

    #[test]
    fn selected_session_servers_skips_disabled_builtin() {
        let settings = McpSettings {
            servers: vec![computer_use_server_entry(&ComputerUseSettings::default())],
        };
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCP_IDS_OPTION.to_string(),
            json!([BUILTIN_COMPUTER_USE_ID]),
        );

        let selected =
            selected_session_servers(&settings, Some(&caps(true, false)), &options, |_| {
                panic!("unexpected builtin resolver call")
            })
            .unwrap();

        assert!(selected.is_empty());
    }

    #[test]
    fn selected_session_servers_skips_builtin_without_http_capability() {
        let settings = McpSettings {
            servers: vec![computer_use_server_entry(&ComputerUseSettings {
                enabled: true,
                ..Default::default()
            })],
        };
        let mut options = crate::agents::runtime::types::RuntimeMetadata::new();
        options.insert(
            SELECTED_MCP_IDS_OPTION.to_string(),
            json!([BUILTIN_COMPUTER_USE_ID]),
        );

        let selected =
            selected_session_servers(&settings, Some(&caps(false, false)), &options, |_| {
                panic!("unexpected builtin resolver call")
            })
            .unwrap();
        assert!(selected.is_empty());
    }

    #[test]
    fn selected_session_servers_without_selection_does_not_inject_builtin() {
        let settings = McpSettings {
            servers: vec![computer_use_server_entry(&ComputerUseSettings {
                enabled: true,
                ..Default::default()
            })],
        };
        let selected = selected_session_servers(
            &settings,
            Some(&caps(true, false)),
            &crate::agents::runtime::types::RuntimeMetadata::new(),
            |_| panic!("unexpected builtin resolver call"),
        )
        .unwrap();
        assert!(selected.is_empty());
    }
}
