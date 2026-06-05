use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::config::{self, NetworkConfig, NetworkProxyConfig};

const PROXY_ENV_KEYS: [&str; 6] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];
const NO_PROXY_ENV_KEYS: [&str; 2] = ["NO_PROXY", "no_proxy"];
static PREVIOUS_PROXY_ENV: OnceLock<Mutex<Option<HashMap<String, Option<String>>>>> = OnceLock::new();

pub fn load_network_config() -> Result<NetworkConfig> {
    Ok(config::load_config()?.network)
}

pub fn save_network_config(network: NetworkConfig) -> Result<NetworkConfig> {
    let mut app_config = config::load_config()?;
    app_config.network = normalize_network_config(network);
    config::save_config(&app_config)?;
    apply_network_proxy_env(&app_config.network.proxy);
    Ok(app_config.network)
}

pub fn apply_network_proxy_env(proxy: &NetworkProxyConfig) {
    let url = proxy.url.as_deref().map(str::trim).filter(|value| !value.is_empty());
    if proxy.enabled {
        if let Some(url) = url {
            capture_previous_proxy_env();
            for key in PROXY_ENV_KEYS {
                std::env::set_var(key, url);
            }
        }
        if let Some(no_proxy) = proxy
            .no_proxy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            for key in NO_PROXY_ENV_KEYS {
                std::env::set_var(key, no_proxy);
            }
        }
    } else {
        restore_previous_proxy_env();
    }
}

fn capture_previous_proxy_env() {
    let lock = PREVIOUS_PROXY_ENV.get_or_init(|| Mutex::new(None));
    let Ok(mut previous) = lock.lock() else {
        return;
    };
    if previous.is_some() {
        return;
    }
    let mut values = HashMap::new();
    for key in PROXY_ENV_KEYS.into_iter().chain(NO_PROXY_ENV_KEYS) {
        values.insert(key.to_string(), std::env::var(key).ok());
    }
    *previous = Some(values);
}

fn restore_previous_proxy_env() {
    let lock = PREVIOUS_PROXY_ENV.get_or_init(|| Mutex::new(None));
    let Ok(mut previous) = lock.lock() else {
        return;
    };
    let Some(values) = previous.take() else {
        return;
    };
    for key in PROXY_ENV_KEYS.into_iter().chain(NO_PROXY_ENV_KEYS) {
        match values.get(key).and_then(|value| value.as_deref()) {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

fn normalize_network_config(mut network: NetworkConfig) -> NetworkConfig {
    network.proxy.url = trimmed(network.proxy.url.as_deref());
    network.proxy.no_proxy = trimmed(network.proxy.no_proxy.as_deref());
    network
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
