use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;

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
static PREVIOUS_PROXY_ENV: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

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
    // Serialize with every other env writer in the process (e.g. login-shell import):
    // std::env::set_var races with concurrent getenv, so only one writer mutates at a time.
    let _env_guard = crate::shell_env::env_write_guard();
    let Ok(mut previous) = PREVIOUS_PROXY_ENV.lock() else {
        log::warn!("[network:proxy] previous-env lock poisoned");
        return;
    };
    if proxy.enabled {
        // Snapshot the original values once, before mutating anything, so disabling
        // restores both proxy and no_proxy keys even when only one of them was set.
        capture_previous_proxy_env(&mut previous);
        if let Some(url) = proxy
            .url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
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
        restore_previous_proxy_env(&mut previous);
    }
}

fn capture_previous_proxy_env(previous: &mut Option<HashMap<String, Option<String>>>) {
    if previous.is_some() {
        return;
    }
    let mut values = HashMap::new();
    for key in PROXY_ENV_KEYS.into_iter().chain(NO_PROXY_ENV_KEYS) {
        values.insert(key.to_string(), std::env::var(key).ok());
    }
    *previous = Some(values);
}

fn restore_previous_proxy_env(previous: &mut Option<HashMap<String, Option<String>>>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_proxy_env() {
        for key in PROXY_ENV_KEYS.into_iter().chain(NO_PROXY_ENV_KEYS) {
            std::env::remove_var(key);
        }
        if let Ok(mut previous) = PREVIOUS_PROXY_ENV.lock() {
            *previous = None;
        }
    }

    #[test]
    fn disabling_restores_no_proxy_set_without_url() {
        reset_proxy_env();

        apply_network_proxy_env(&NetworkProxyConfig {
            enabled: true,
            url: None,
            no_proxy: Some("localhost,127.0.0.1".to_string()),
        });
        assert_eq!(
            std::env::var("NO_PROXY").ok().as_deref(),
            Some("localhost,127.0.0.1")
        );

        apply_network_proxy_env(&NetworkProxyConfig {
            enabled: false,
            url: None,
            no_proxy: None,
        });
        assert!(std::env::var("NO_PROXY").is_err());
        assert!(std::env::var("no_proxy").is_err());

        reset_proxy_env();
    }
}
