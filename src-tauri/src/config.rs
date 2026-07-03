use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::computer_use::settings::ComputerUseSettings;
use crate::mcp::McpSettings;

mod defaults;
mod loader;
mod parser;
mod raw;
mod resolver;
mod serializer;

#[cfg(test)]
use defaults::{default_app_config, raw_config_with_defaults};
#[cfg(test)]
use loader::load_config_from_path;
pub(crate) use loader::load_config_strict;
pub use loader::{
    load_config, load_memory_config, save_config, save_memory_config, take_config_recovery_notice,
};
#[cfg(test)]
use parser::parse_raw_config;
#[cfg(test)]
use raw::RawConfig;
pub(in crate::config) use resolver::resolve_app_config;
#[cfg(test)]
use resolver::resolve_memory_config_inner;
pub use serializer::serialize_app_config;

#[derive(Debug, Clone, Serialize)]
pub struct AppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
    pub index: IndexConfig,
    pub network: NetworkConfig,
    pub mcp: McpSettings,
    pub appshot: AppshotConfig,
    pub computer_use: ComputerUseSettings,
    pub debug: DebugConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRecoveryNotice {
    pub path: String,
    pub backup_path: Option<String>,
    pub error: String,
    pub line_number: Option<usize>,
    pub line_text: Option<String>,
    pub used_defaults: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexConfig {
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConfig {
    pub proxy: NetworkProxyConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProxyConfig {
    pub enabled: bool,
    pub url: Option<String>,
    pub no_proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppshotConfig {
    pub shortcut: String,
}

impl Default for AppshotConfig {
    fn default() -> Self {
        Self {
            shortcut: "Shift+Alt+Super+KeyK".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugConfig {
    pub acp_config: bool,
    pub update_preview: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryConfig {
    pub backend: String,
    pub qmd: QmdBackendConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct QmdBackendConfig {
    pub binary: Option<String>,
    pub index: String,
    pub artifacts_root: PathBuf,
    pub auto_embed: bool,
    pub install_command: String,
}

pub fn expand_path(value: &str) -> Result<PathBuf> {
    if let Some(rest) = value.strip_prefix("~/") {
        let home = dirs::home_dir().context("no home dir")?;
        return Ok(home.join(rest));
    }
    if value == "~" {
        return dirs::home_dir().context("no home dir");
    }
    Ok(Path::new(value).to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::Context;

    use super::{
        default_app_config, parse_raw_config, raw_config_with_defaults,
        resolve_memory_config_inner, serialize_app_config, take_config_recovery_notice,
    };

    static CONFIG_TEST_GUARD: Mutex<()> = Mutex::new(());

    fn resolve_memory_config(raw: super::RawConfig) -> super::Result<super::MemoryConfig> {
        resolve_memory_config_inner(raw.memory.context("memory is not configured")?, true)
    }

    fn complete_memory_config(extra: &str) -> String {
        format!(
            r#"
            [memory]
            backend = "qmd"

            [memory.backends.qmd]
            index = "sessio-test"
            artifacts_root = "/tmp/sessio-artifacts"
            auto_embed = false
            install_command = "npm install -g @tobilu/qmd"
            {extra}
            "#
        )
    }

    #[test]
    fn parses_memory_qmd_config() {
        let raw = parse_raw_config(
            r#"
            [memory]
            backend = "qmd"

            [memory.backends.qmd]
            binary = "/usr/local/bin/qmd"
            index = "sessio-test"
            artifacts_root = "/tmp/sessio-artifacts"
            auto_embed = false
            install_command = "npm install -g @tobilu/qmd"
            "#,
        )
        .unwrap();
        let config = resolve_memory_config(raw).unwrap();

        assert_eq!(config.backend, "qmd");
        assert_eq!(config.qmd.binary.as_deref(), Some("/usr/local/bin/qmd"));
        assert_eq!(config.qmd.index, "sessio-test");
        assert_eq!(
            config.qmd.artifacts_root.to_string_lossy(),
            "/tmp/sessio-artifacts"
        );
    }

    #[test]
    fn rejects_non_qmd_backend() {
        let raw = parse_raw_config(
            r#"
            [memory]
            backend = "sqlite"
            "#,
        )
        .unwrap();

        assert!(resolve_memory_config(raw).is_err());
    }

    #[test]
    fn parses_auto_embed_boolean_and_strips_comments() {
        let raw = parse_raw_config(&complete_memory_config(
            r#"auto_embed = true  # inline comment after value"#,
        ))
        .unwrap();
        let config = resolve_memory_config(raw).unwrap();
        assert!(config.qmd.auto_embed);
    }

    #[test]
    fn rejects_invalid_boolean_value() {
        let raw = parse_raw_config(
            r#"
            [memory.backends.qmd]
            auto_embed = sometimes
            "#,
        );
        assert!(raw.is_err());
    }

    #[test]
    fn parse_errors_include_line_number_for_invalid_line() {
        let err = parse_raw_config(
            r#"
            [debug]
            acp_config = false
            oops
            "#,
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("line 4: invalid config line: oops"));
    }

    #[test]
    fn parses_index_poll_interval_seconds() {
        let raw = parse_raw_config(
            r#"
            [index]
            poll_interval_seconds = 120
            "#,
        )
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert_eq!(config.index.poll_interval_seconds, 120);
    }

    #[test]
    fn parses_network_proxy_config() {
        let raw = parse_raw_config(
            r#"
            [network.proxy]
            enabled = true
            url = "http://127.0.0.1:7890"
            no_proxy = "localhost,127.0.0.1"
            "#,
        )
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(config.network.proxy.enabled);
        assert_eq!(
            config.network.proxy.url.as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            config.network.proxy.no_proxy.as_deref(),
            Some("localhost,127.0.0.1")
        );
    }

    #[test]
    fn ignores_legacy_astra_config_sections() {
        let raw = parse_raw_config(
            r#"
            [astra]
            round_limit = 5
            retry_limit = 2

            [astra.pi]
            command = "pi-agent --acp"
            model = "pi-model"
            thinking_level = "medium"
            session_dir = "/tmp/pi-sessions"

            [astra.pi.env]
            COMMON = "base"
            SHARED = "common"

            [astra.pi.planner]
            timeout_ms = 1000

            [astra.pi.planner.env]
            SHARED = "planner"

            [astra.pi.decision]
            command = "pi-agent --acp --decision"
            model = "decision-model"
            timeout_ms = 2000
            "#,
        )
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();
        let serialized = serialize_app_config(&config);

        assert_eq!(config.index.poll_interval_seconds, 60);
        assert!(!serialized.contains("[astra]"));
        assert!(!serialized.contains("[astra.pi]"));
    }

    #[test]
    fn ignores_unknown_sections() {
        let raw = parse_raw_config(
            r#"
            [unrelated.section]
            key = "value"

            [agents.runtime.codex]
            enabled = false
            model = "ignored"

            [agents.runtime.codex.command]
            session = "ignored"

            [memory]
            backend = "qmd"

            [memory.backends.qmd]
            index = "sessio-test"
            artifacts_root = "/tmp/sessio-artifacts"
            auto_embed = false
            install_command = "npm install -g @tobilu/qmd"
            "#,
        )
        .unwrap();
        let config = resolve_memory_config(raw).unwrap();
        assert_eq!(config.backend, "qmd");
    }

    #[test]
    fn comment_inside_quoted_string_is_preserved() {
        let raw =
            parse_raw_config(&complete_memory_config(r#"binary = "/path/with#hash/qmd""#)).unwrap();
        let config = resolve_memory_config(raw).unwrap();
        assert_eq!(config.qmd.binary.as_deref(), Some("/path/with#hash/qmd"));
    }

    #[test]
    fn default_app_config_serializes_debug_without_memory() {
        let config = default_app_config().unwrap();
        let serialized = serialize_app_config(&config);
        let raw = parse_raw_config(&serialized).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(!serialized.contains("[memory]"));
        assert!(serialized.contains("[index]"));
        assert!(serialized.contains("poll_interval_seconds = 60"));
        assert!(!serialized.contains("[astra]"));
        assert!(serialized.contains("[debug]"));
        assert!(serialized.contains("[network.proxy]"));
        assert!(serialized.contains("enabled = false"));
        assert!(serialized.contains("[mcp_servers.computer_use]"));
        assert!(serialized.contains(r#"builtin = "computer_use""#));
        assert!(!serialized.contains("[agents.runtime"));
        assert!(config.memory.is_none());
        assert_eq!(config.index.poll_interval_seconds, 60);
        assert!(!config.network.proxy.enabled);
    }

    #[test]
    fn empty_config_is_completed_with_debug_defaults_only() {
        let raw = parse_raw_config("").unwrap();
        let (raw, changed) = raw_config_with_defaults(raw).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(changed);
        assert!(config.memory.is_none());
        assert_eq!(config.index.poll_interval_seconds, 60);
        assert!(config.computer_use.enabled);
        assert!(!config.debug.acp_config);
        assert!(!config.debug.update_preview);
    }

    #[test]
    fn parses_computer_use_config() {
        let prefs_json = serde_json::to_string(&std::collections::BTreeMap::from([(
            "com.example.one".to_string(),
            crate::computer_use::settings::AppRoutePreferences {
                click_at: Some(crate::computer_use::settings::OperationRoutePreference::Hid),
                ..crate::computer_use::settings::AppRoutePreferences::default()
            },
        )]))
        .unwrap();
        let raw = parse_raw_config(&format!(
            r#"
            [mcp_servers.computer_use]
            builtin = "computer_use"
            transport = "http"
            enabled = false
            description = "Use for desktop observation and GUI control."

            [computer_use]
            approved_apps = ["com.example.two", "com.example.one", "com.example.two", ""]
            app_route_preferences = {prefs_json:?}
            allow_input_injection = true
            allow_foreground_takeover = false
            "#
        ))
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(!config.computer_use.enabled);
        assert_eq!(
            config.computer_use.mcp_description.as_deref(),
            Some("Use for desktop observation and GUI control.")
        );
        assert_eq!(
            config.computer_use.approved_apps,
            vec!["com.example.one".to_string(), "com.example.two".to_string()]
        );
        assert_eq!(
            config
                .computer_use
                .app_route_preferences
                .get("com.example.one")
                .and_then(|prefs| prefs.click_at.as_ref())
                .map(|pref| pref.to_dispatch_route()),
            Some(crate::computer_use::provider::ClickDispatchRoute::Hid)
        );
        let serialized = serialize_app_config(&config);
        assert!(serialized.contains("[mcp_servers.computer_use]"));
        assert!(serialized.contains("enabled = false"));
        assert!(serialized.contains(r#"approved_apps = ["com.example.one", "com.example.two"]"#));
        assert!(serialized.contains("app_route_preferences = "));
        assert!(!serialized.contains("[computer_use]\nenabled ="));
        assert!(!serialized.contains("allow_input_injection"));
        assert!(!serialized.contains("allow_foreground_takeover"));
    }

    #[test]
    fn parses_custom_mcp_config() {
        let raw = parse_raw_config(
            r#"
            [mcp_servers.docs]
            name = "Docs"
            transport = "stdio"
            command = "~/bin/docs-mcp"
            args = ["serve"]
            env = ["DOCS_ROOT=/tmp/docs"]
            enabled = true
            description = "Project docs"
            "#
        )
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert_eq!(config.mcp.servers.len(), 1);
        assert_eq!(config.mcp.servers[0].id, "docs");
        assert_eq!(config.mcp.servers[0].name, "Docs");
        assert_eq!(
            config.mcp.servers[0].command.as_deref(),
            Some("~/bin/docs-mcp")
        );
        assert_eq!(config.mcp.servers[0].args, vec!["serve".to_string()]);
        assert_eq!(
            config.mcp.servers[0].env,
            vec![crate::mcp::McpKeyValue {
                name: "DOCS_ROOT".into(),
                value: "/tmp/docs".into(),
            }]
        );

        let serialized = serialize_app_config(&config);
        assert!(serialized.contains("[mcp_servers.docs]"));
        assert!(serialized.contains(r#"transport = "stdio""#));
        assert!(serialized.contains(r#"env = ["DOCS_ROOT=/tmp/docs"]"#));
    }

    #[test]
    fn parses_legacy_custom_mcp_config() {
        let servers_json = serde_json::to_string(&vec![crate::mcp::McpServerConfig {
            id: "docs".to_string(),
            name: "Docs".to_string(),
            description: Some("Project docs".to_string()),
            enabled: true,
            source: crate::mcp::McpServerSource::Custom,
            transport: crate::mcp::McpServerTransport::Http,
            injection_mode: crate::mcp::McpServerInjectionMode::SessionOptIn,
            builtin_kind: None,
            url: Some("http://127.0.0.1:3001/mcp".to_string()),
            headers: vec![crate::mcp::McpKeyValue {
                name: "Authorization".to_string(),
                value: "Bearer token".to_string(),
            }],
            command: None,
            args: Vec::new(),
            env: Vec::new(),
        }])
        .unwrap();
        let raw = parse_raw_config(&format!(
            r#"
            [mcp]
            custom_servers = {servers_json:?}
            "#
        ))
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert_eq!(config.mcp.servers.len(), 1);
        assert_eq!(config.mcp.servers[0].id, "docs");
        assert_eq!(
            config.mcp.servers[0].headers,
            vec![crate::mcp::McpKeyValue {
                name: "Authorization".into(),
                value: "Bearer token".into(),
            }]
        );
    }

    #[test]
    fn rejects_incomplete_memory_config() {
        let raw = parse_raw_config(
            r#"
            [memory]
            backend = "qmd"
            "#,
        )
        .unwrap();

        assert!(resolve_memory_config(raw).is_err());
    }

    #[test]
    fn invalid_config_recovery_reports_notice_and_keeps_original_file() {
        let _guard = CONFIG_TEST_GUARD.lock().unwrap();
        let _ = take_config_recovery_notice();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sessio-config-test-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            r#"[debug]
acp_config = false
e
"#,
        )
        .unwrap();

        let config = super::load_config_from_path(&path).unwrap();
        let notice = take_config_recovery_notice().expect("config recovery notice");
        let preserved = fs::read_to_string(&path).unwrap();

        assert!(!config.debug.acp_config);
        assert_eq!(notice.path, path.display().to_string());
        assert_eq!(notice.line_number, Some(3));
        assert_eq!(notice.line_text.as_deref(), Some("e"));
        assert!(notice.backup_path.is_none());
        assert!(notice.error.contains("invalid config line: e"));
        assert!(notice.used_defaults);
        assert!(preserved.contains("\ne\n"));
        assert!(preserved.contains("[debug]"));

        let _ = fs::remove_dir_all(&dir);
    }
}
