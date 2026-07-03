use anyhow::{bail, Context, Result};

use super::raw::{RawConfig, RawMcpConfig, RawMcpServerConfig, RawMemoryConfig};
use super::{
    expand_path, AppConfig, AppshotConfig, DebugConfig, IndexConfig, MemoryConfig, NetworkConfig,
    NetworkProxyConfig, QmdBackendConfig,
};
use crate::computer_use::settings::ComputerUseSettings;
use crate::mcp::McpSettings;

pub(super) fn resolve_app_config(raw: RawConfig, apply_env: bool) -> Result<AppConfig> {
    let memory = raw
        .memory
        .clone()
        .map(|memory| resolve_memory_config_inner(memory, apply_env))
        .transpose()?;
    let mcp = resolve_mcp_config(raw.clone())?;
    let computer_use = resolve_computer_use_config(raw.clone())?;
    Ok(AppConfig {
        memory,
        index: resolve_index_config(raw.clone()),
        network: resolve_network_config(raw.clone()),
        mcp,
        appshot: resolve_appshot_config(raw.clone()),
        computer_use,
        debug: resolve_debug_config(raw),
    })
}

fn resolve_index_config(raw: RawConfig) -> IndexConfig {
    IndexConfig {
        poll_interval_seconds: raw.index.poll_interval_seconds.unwrap_or(60),
    }
}

fn resolve_network_config(raw: RawConfig) -> NetworkConfig {
    let proxy = raw.network.proxy;
    NetworkConfig {
        proxy: NetworkProxyConfig {
            enabled: proxy.enabled.unwrap_or(false),
            url: trimmed_string(proxy.url.as_deref()),
            no_proxy: trimmed_string(proxy.no_proxy.as_deref()),
        },
    }
}

fn resolve_mcp_config(raw: RawConfig) -> Result<McpSettings> {
    let mut servers = raw.mcp.legacy_custom_servers.unwrap_or_default();
    for (server_id, raw_server) in raw.mcp.servers {
        if let Some(builtin) = raw_server.builtin.as_deref() {
            if builtin != "computer_use" {
                bail!("unknown builtin MCP server in [mcp_servers.{server_id}]: {builtin}");
            }
            continue;
        }
        servers.push(resolve_custom_mcp_server(server_id, raw_server)?);
    }
    crate::mcp::normalize_custom_settings(McpSettings { servers })
}

fn resolve_appshot_config(raw: RawConfig) -> AppshotConfig {
    AppshotConfig {
        shortcut: trimmed_string(raw.appshot.shortcut.as_deref())
            .unwrap_or_else(|| AppshotConfig::default().shortcut),
    }
}

fn resolve_computer_use_config(raw: RawConfig) -> Result<ComputerUseSettings> {
    let defaults = ComputerUseSettings::recommended();
    let _legacy_control_settings = (
        raw.computer_use.allow_input_injection,
        raw.computer_use.allow_foreground_takeover,
    );
    let builtin = resolve_builtin_computer_use_mcp_server(&raw.mcp)?;
    Ok(ComputerUseSettings {
        enabled: builtin
            .as_ref()
            .and_then(|server| server.enabled)
            .or(raw.computer_use.enabled)
            .unwrap_or(defaults.enabled),
        mcp_description: builtin
            .and_then(|server| server.description)
            .or(raw.computer_use.mcp_description)
            .or(defaults.mcp_description),
        approved_apps: normalized_string_list(raw.computer_use.approved_apps.unwrap_or_default()),
        app_route_preferences: raw.computer_use.app_route_preferences.unwrap_or_default(),
    })
}

#[derive(Debug, Clone)]
struct BuiltinComputerUseMcpServer {
    enabled: Option<bool>,
    description: Option<String>,
}

fn resolve_builtin_computer_use_mcp_server(
    raw: &RawMcpConfig,
) -> Result<Option<BuiltinComputerUseMcpServer>> {
    let mut builtin = None;
    for (server_id, server) in &raw.servers {
        let Some(kind) = server.builtin.as_deref() else {
            continue;
        };
        if kind != "computer_use" {
            bail!("unknown builtin MCP server in [mcp_servers.{server_id}]: {kind}");
        }
        if builtin.is_some() {
            bail!("duplicate builtin MCP server configuration for computer_use");
        }
        if let Some(transport) = server.transport.as_deref() {
            let transport = parse_mcp_transport(transport)?;
            if transport != crate::mcp::McpServerTransport::Http {
                bail!("builtin computer_use MCP server must use http transport");
            }
        }
        if server.url.is_some()
            || server.command.is_some()
            || server.args.as_ref().is_some_and(|args| !args.is_empty())
            || server
                .headers
                .as_ref()
                .is_some_and(|headers| !headers.is_empty())
            || server.env.as_ref().is_some_and(|env| !env.is_empty())
        {
            bail!("builtin computer_use MCP server does not accept transport-specific connection fields");
        }
        builtin = Some(BuiltinComputerUseMcpServer {
            enabled: server.enabled,
            description: trimmed_string(server.description.as_deref()),
        });
    }
    Ok(builtin)
}

fn resolve_custom_mcp_server(
    server_id: String,
    raw: RawMcpServerConfig,
) -> Result<crate::mcp::McpServerConfig> {
    let transport = parse_mcp_transport(raw.transport.as_deref().context(format!(
        "missing MCP transport for [mcp_servers.{server_id}]"
    ))?)?;
    let name = trimmed_string(raw.name.as_deref()).unwrap_or_else(|| server_id.clone());
    let description = trimmed_string(raw.description.as_deref());
    let args = normalize_ordered_strings(raw.args.unwrap_or_default());
    let headers = parse_key_value_entries(raw.headers.unwrap_or_default(), "headers")?;
    let env = parse_key_value_entries(raw.env.unwrap_or_default(), "env")?;

    Ok(match transport {
        crate::mcp::McpServerTransport::Http | crate::mcp::McpServerTransport::Sse => {
            let url = trimmed_string(raw.url.as_deref())
                .context(format!("missing MCP url for [mcp_servers.{server_id}]"))?;
            crate::mcp::McpServerConfig {
                id: server_id,
                name,
                description,
                enabled: raw.enabled.unwrap_or(true),
                source: crate::mcp::McpServerSource::Custom,
                transport,
                injection_mode: crate::mcp::McpServerInjectionMode::SessionOptIn,
                builtin_kind: None,
                url: Some(url),
                headers,
                command: None,
                args: Vec::new(),
                env: Vec::new(),
            }
        }
        crate::mcp::McpServerTransport::Stdio => {
            let command = trimmed_string(raw.command.as_deref())
                .context(format!("missing MCP command for [mcp_servers.{server_id}]"))?;
            crate::mcp::McpServerConfig {
                id: server_id,
                name,
                description,
                enabled: raw.enabled.unwrap_or(true),
                source: crate::mcp::McpServerSource::Custom,
                transport,
                injection_mode: crate::mcp::McpServerInjectionMode::SessionOptIn,
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

fn parse_mcp_transport(value: &str) -> Result<crate::mcp::McpServerTransport> {
    match value.trim().to_ascii_lowercase().as_str() {
        "http" => Ok(crate::mcp::McpServerTransport::Http),
        "sse" => Ok(crate::mcp::McpServerTransport::Sse),
        "stdio" => Ok(crate::mcp::McpServerTransport::Stdio),
        other => bail!("invalid MCP transport: {other}"),
    }
}

fn parse_key_value_entries(
    values: Vec<String>,
    field_name: &str,
) -> Result<Vec<crate::mcp::McpKeyValue>> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((name, entry_value)) = trimmed.split_once('=') else {
            bail!("{field_name} entries must use NAME=VALUE syntax: {trimmed}");
        };
        let name = name.trim();
        if name.is_empty() {
            bail!("{field_name} entries must include a key name");
        }
        out.push(crate::mcp::McpKeyValue {
            name: name.to_string(),
            value: entry_value.trim().to_string(),
        });
    }
    Ok(out)
}

fn normalize_ordered_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalized_string_list(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn resolve_debug_config(raw: RawConfig) -> DebugConfig {
    DebugConfig {
        acp_config: raw.debug.acp_config.unwrap_or(false),
        update_preview: raw.debug.update_preview.unwrap_or(false),
    }
}

pub(super) fn resolve_memory_config_inner(
    raw: RawMemoryConfig,
    apply_env: bool,
) -> Result<MemoryConfig> {
    let backend = raw.backend.context("missing [memory].backend")?;
    if backend != "qmd" {
        bail!("unsupported memory backend in config: {backend}");
    }

    let qmd = raw.backends.qmd;
    let mut config = QmdBackendConfig {
        binary: qmd.binary,
        index: qmd.index.context("missing [memory.backends.qmd].index")?,
        artifacts_root: expand_path(
            &qmd.artifacts_root
                .context("missing [memory.backends.qmd].artifacts_root")?,
        )?,
        auto_embed: qmd
            .auto_embed
            .context("missing [memory.backends.qmd].auto_embed")?,
        install_command: qmd
            .install_command
            .context("missing [memory.backends.qmd].install_command")?,
    };
    if apply_env {
        if let Ok(binary) = std::env::var("SESSIO_QMD_BINARY") {
            config.binary = Some(binary);
        }
        if let Ok(index) = std::env::var("SESSIO_QMD_INDEX") {
            if !index.is_empty() {
                config.index = index;
            }
        }
        if let Ok(root) = std::env::var("SESSIO_QMD_ARTIFACTS_ROOT") {
            config.artifacts_root = expand_path(&root)?;
        }
        if let Ok(value) = std::env::var("SESSIO_QMD_AUTO_EMBED") {
            config.auto_embed = matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if let Ok(command) = std::env::var("SESSIO_QMD_INSTALL_COMMAND") {
            if !command.is_empty() {
                config.install_command = command;
            }
        }
    }

    Ok(MemoryConfig {
        backend,
        qmd: config,
    })
}

fn trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
