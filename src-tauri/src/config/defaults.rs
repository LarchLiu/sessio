use anyhow::Result;
use std::collections::BTreeMap;

use super::parser::parse_raw_config;
use super::raw::{RawConfig, RawMcpServerConfig};
use super::{
    serialize_app_config, AppConfig, AppshotConfig, DebugConfig, IndexConfig, NetworkConfig,
};
use crate::computer_use::settings::ComputerUseSettings;
use crate::mcp::McpSettings;

pub(super) fn raw_config_with_defaults(mut raw: RawConfig) -> Result<(RawConfig, bool)> {
    let defaults = parse_raw_config(&serialize_app_config(&default_app_config()?))?;
    let mut changed = false;
    merge_option(
        &mut raw.index.poll_interval_seconds,
        defaults.index.poll_interval_seconds,
        &mut changed,
    );
    merge_option(
        &mut raw.network.proxy.enabled,
        defaults.network.proxy.enabled,
        &mut changed,
    );
    merge_option(
        &mut raw.debug.acp_config,
        defaults.debug.acp_config,
        &mut changed,
    );
    merge_option(
        &mut raw.appshot.shortcut,
        defaults.appshot.shortcut,
        &mut changed,
    );
    merge_mcp_server_defaults(
        &mut raw.mcp.servers,
        &defaults.mcp.servers,
        crate::mcp::BUILTIN_COMPUTER_USE_CONFIG_KEY,
        &mut changed,
    );
    merge_option(
        &mut raw.debug.update_preview,
        defaults.debug.update_preview,
        &mut changed,
    );

    Ok((raw, changed))
}

fn merge_option<T>(target: &mut Option<T>, default: Option<T>, changed: &mut bool) {
    if target.is_none() && default.is_some() {
        *target = default;
        *changed = true;
    }
}

fn merge_mcp_server_defaults(
    target: &mut BTreeMap<String, RawMcpServerConfig>,
    defaults: &BTreeMap<String, RawMcpServerConfig>,
    server_id: &str,
    changed: &mut bool,
) {
    let Some(default_server) = defaults.get(server_id) else {
        return;
    };
    match target.get_mut(server_id) {
        Some(server) => {
            merge_option(&mut server.name, default_server.name.clone(), changed);
            merge_option(&mut server.builtin, default_server.builtin.clone(), changed);
            merge_option(
                &mut server.transport,
                default_server.transport.clone(),
                changed,
            );
            merge_option(&mut server.enabled, default_server.enabled, changed);
            merge_option(
                &mut server.description,
                default_server.description.clone(),
                changed,
            );
            merge_option(&mut server.url, default_server.url.clone(), changed);
            merge_option(&mut server.headers, default_server.headers.clone(), changed);
            merge_option(&mut server.command, default_server.command.clone(), changed);
            merge_option(&mut server.args, default_server.args.clone(), changed);
            merge_option(&mut server.env, default_server.env.clone(), changed);
        }
        None => {
            target.insert(server_id.to_string(), default_server.clone());
            *changed = true;
        }
    }
}

pub(super) fn default_app_config() -> Result<AppConfig> {
    Ok(AppConfig {
        memory: None,
        index: IndexConfig {
            poll_interval_seconds: 60,
        },
        network: NetworkConfig::default(),
        mcp: McpSettings::default(),
        appshot: AppshotConfig::default(),
        computer_use: ComputerUseSettings::recommended(),
        debug: DebugConfig {
            acp_config: false,
            update_preview: false,
        },
    })
}
