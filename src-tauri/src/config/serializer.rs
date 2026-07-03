use crate::computer_use::settings::ComputerUseSettings;
use crate::mcp::McpSettings;

use super::{AppConfig, AppshotConfig, DebugConfig, IndexConfig, MemoryConfig, NetworkConfig};

pub fn serialize_app_config(config: &AppConfig) -> String {
    let mut out = String::new();
    if let Some(memory) = &config.memory {
        out.push_str(&serialize_memory_config(memory));
        out.push('\n');
    }
    out.push_str(&serialize_index_config(&config.index));
    out.push('\n');
    out.push_str(&serialize_network_config(&config.network));
    out.push('\n');
    out.push_str(&serialize_mcp_config(&config.mcp, &config.computer_use));
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&serialize_appshot_config(&config.appshot));
    out.push('\n');
    let computer_use = serialize_computer_use_config(&config.computer_use);
    if !computer_use.is_empty() {
        out.push_str(&computer_use);
        out.push('\n');
    }
    out.push_str(&serialize_debug_config(&config.debug));
    out
}

fn serialize_memory_config(config: &MemoryConfig) -> String {
    let mut out = String::new();
    out.push_str("[memory]\n");
    out.push_str("backend = ");
    out.push_str(&toml_string(&config.backend));
    out.push_str("\n\n[memory.backends.qmd]\n");
    if let Some(binary) = &config.qmd.binary {
        out.push_str("binary = ");
        out.push_str(&toml_string(binary));
        out.push('\n');
    }
    out.push_str("index = ");
    out.push_str(&toml_string(&config.qmd.index));
    out.push('\n');
    out.push_str("artifacts_root = ");
    out.push_str(&toml_string(&config.qmd.artifacts_root.to_string_lossy()));
    out.push('\n');
    out.push_str("auto_embed = ");
    out.push_str(if config.qmd.auto_embed {
        "true"
    } else {
        "false"
    });
    out.push('\n');
    out.push_str("install_command = ");
    out.push_str(&toml_string(&config.qmd.install_command));
    out.push('\n');
    out
}

fn serialize_index_config(config: &IndexConfig) -> String {
    let mut out = String::new();
    out.push_str("[index]\n");
    out.push_str("poll_interval_seconds = ");
    out.push_str(&config.poll_interval_seconds.to_string());
    out.push('\n');
    out
}

fn serialize_network_config(config: &NetworkConfig) -> String {
    let mut out = String::new();
    out.push_str("[network.proxy]\n");
    out.push_str("enabled = ");
    out.push_str(if config.proxy.enabled {
        "true"
    } else {
        "false"
    });
    out.push('\n');
    if let Some(url) = &config.proxy.url {
        out.push_str("url = ");
        out.push_str(&toml_string(url));
        out.push('\n');
    }
    if let Some(no_proxy) = &config.proxy.no_proxy {
        out.push_str("no_proxy = ");
        out.push_str(&toml_string(no_proxy));
        out.push('\n');
    }
    out
}

fn serialize_mcp_config(config: &McpSettings, computer_use: &ComputerUseSettings) -> String {
    let mut out = String::new();
    let builtin = crate::mcp::computer_use_server_entry(computer_use);
    out.push_str("[mcp_servers.");
    out.push_str(crate::mcp::BUILTIN_COMPUTER_USE_CONFIG_KEY);
    out.push_str("]\n");
    out.push_str("builtin = ");
    out.push_str(&toml_string("computer_use"));
    out.push('\n');
    out.push_str("transport = ");
    out.push_str(&toml_string("http"));
    out.push('\n');
    out.push_str("enabled = ");
    out.push_str(if builtin.enabled { "true" } else { "false" });
    out.push('\n');
    out.push_str("description = ");
    out.push_str(&toml_string(
        builtin
            .description
            .as_deref()
            .unwrap_or("Use for desktop observation and GUI control."),
    ));
    out.push('\n');
    let mut custom_servers = config
        .servers
        .iter()
        .filter(|server| server.source == crate::mcp::McpServerSource::Custom)
        .collect::<Vec<_>>();
    custom_servers.sort_by(|left, right| left.id.cmp(&right.id));
    for server in custom_servers {
        out.push('\n');
        out.push_str("[mcp_servers.");
        out.push_str(&server.id);
        out.push_str("]\n");
        if !server.name.trim().is_empty() && server.name != server.id {
            out.push_str("name = ");
            out.push_str(&toml_string(&server.name));
            out.push('\n');
        }
        out.push_str("transport = ");
        out.push_str(&toml_string(match server.transport {
            crate::mcp::McpServerTransport::Http => "http",
            crate::mcp::McpServerTransport::Sse => "sse",
            crate::mcp::McpServerTransport::Stdio => "stdio",
        }));
        out.push('\n');
        out.push_str("enabled = ");
        out.push_str(if server.enabled { "true" } else { "false" });
        out.push('\n');
        if let Some(description) = &server.description {
            out.push_str("description = ");
            out.push_str(&toml_string(description));
            out.push('\n');
        }
        match server.transport {
            crate::mcp::McpServerTransport::Http | crate::mcp::McpServerTransport::Sse => {
                if let Some(url) = &server.url {
                    out.push_str("url = ");
                    out.push_str(&toml_string(url));
                    out.push('\n');
                }
                if !server.headers.is_empty() {
                    out.push_str("headers = ");
                    out.push_str(&toml_string_array(
                        &server
                            .headers
                            .iter()
                            .map(|entry| format!("{}={}", entry.name, entry.value))
                            .collect::<Vec<_>>(),
                    ));
                    out.push('\n');
                }
            }
            crate::mcp::McpServerTransport::Stdio => {
                if let Some(command) = &server.command {
                    out.push_str("command = ");
                    out.push_str(&toml_string(command));
                    out.push('\n');
                }
                if !server.args.is_empty() {
                    out.push_str("args = ");
                    out.push_str(&toml_string_array_ordered(&server.args));
                    out.push('\n');
                }
                if !server.env.is_empty() {
                    out.push_str("env = ");
                    out.push_str(&toml_string_array(
                        &server
                            .env
                            .iter()
                            .map(|entry| format!("{}={}", entry.name, entry.value))
                            .collect::<Vec<_>>(),
                    ));
                    out.push('\n');
                }
            }
        }
    }
    out
}

fn serialize_appshot_config(config: &AppshotConfig) -> String {
    let mut out = String::new();
    out.push_str("[appshot]\n");
    out.push_str("shortcut = ");
    out.push_str(&toml_string(&config.shortcut));
    out.push('\n');
    out
}

fn serialize_computer_use_config(config: &ComputerUseSettings) -> String {
    if config.approved_apps.is_empty() && config.app_route_preferences.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("[computer_use]\n");
    if !config.approved_apps.is_empty() {
        out.push_str("approved_apps = ");
        out.push_str(&toml_string_array(&config.approved_apps));
        out.push('\n');
    }
    if !config.app_route_preferences.is_empty() {
        out.push_str("app_route_preferences = ");
        out.push_str(&toml_string(
            &serde_json::to_string(&config.app_route_preferences)
                .expect("serialize computer use route preferences"),
        ));
        out.push('\n');
    }
    out
}

fn serialize_debug_config(config: &DebugConfig) -> String {
    let mut out = String::new();
    out.push_str("[debug]\n");
    out.push_str("acp_config = ");
    out.push_str(if config.acp_config { "true" } else { "false" });
    out.push('\n');
    out.push_str("update_preview = ");
    out.push_str(if config.update_preview {
        "true"
    } else {
        "false"
    });
    out.push('\n');
    out
}

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serialize config string")
}

fn toml_string_array(values: &[String]) -> String {
    let mut values = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    let items = values
        .into_iter()
        .map(toml_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}

fn toml_string_array_ordered(values: &[String]) -> String {
    let items = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(toml_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{items}]")
}
