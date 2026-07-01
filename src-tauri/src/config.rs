use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::app_paths;
use crate::computer_use::settings::ComputerUseSettings;

#[derive(Debug, Clone, Serialize)]
pub struct AppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
    pub index: IndexConfig,
    pub network: NetworkConfig,
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

#[derive(Debug, Clone, Default)]
struct RawConfig {
    memory: Option<RawMemoryConfig>,
    index: RawIndexConfig,
    network: RawNetworkConfig,
    appshot: RawAppshotConfig,
    computer_use: RawComputerUseConfig,
    debug: RawDebugConfig,
}

#[derive(Debug, Clone, Default)]
struct RawIndexConfig {
    poll_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct RawNetworkConfig {
    proxy: RawNetworkProxyConfig,
}

#[derive(Debug, Clone, Default)]
struct RawNetworkProxyConfig {
    enabled: Option<bool>,
    url: Option<String>,
    no_proxy: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RawAppshotConfig {
    shortcut: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RawComputerUseConfig {
    enabled: Option<bool>,
    approved_apps: Option<Vec<String>>,
    app_route_preferences: Option<
        std::collections::BTreeMap<String, crate::computer_use::settings::AppRoutePreferences>,
    >,
    allow_input_injection: Option<bool>,
    allow_foreground_takeover: Option<bool>,
}

#[derive(Debug, Clone, Default)]
struct RawDebugConfig {
    acp_config: Option<bool>,
    update_preview: Option<bool>,
}

#[derive(Debug, Clone, Default)]
struct RawMemoryConfig {
    backend: Option<String>,
    backends: RawMemoryBackends,
}

#[derive(Debug, Clone, Default)]
struct RawMemoryBackends {
    qmd: RawQmdBackendConfig,
}

#[derive(Debug, Clone, Default)]
struct RawQmdBackendConfig {
    binary: Option<String>,
    index: Option<String>,
    artifacts_root: Option<String>,
    auto_embed: Option<bool>,
    install_command: Option<String>,
}

static CONFIG_RECOVERY_NOTICE: OnceLock<Mutex<Option<ConfigRecoveryNotice>>> = OnceLock::new();

pub fn load_config() -> Result<AppConfig> {
    load_config_from_path(&config_path()?)
}

pub fn take_config_recovery_notice() -> Option<ConfigRecoveryNotice> {
    config_recovery_notice_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

pub fn load_memory_config() -> Result<MemoryConfig> {
    load_config()?.memory.context("memory is not configured")
}

pub fn save_memory_config(config: &MemoryConfig) -> Result<()> {
    let mut app_config = load_config().or_else(|_| default_app_config())?;
    app_config.memory = Some(config.clone());
    save_config(&app_config)
}

pub fn save_config(config: &AppConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    fs::write(&path, serialize_app_config(config))
        .with_context(|| format!("write config {}", path.display()))
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

fn load_config_from_path(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        write_default_config_file(path)?;
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    if contents.trim().is_empty() {
        return finalize_loaded_config(path, Some(contents.as_str()), RawConfig::default());
    }
    let raw = match parse_raw_config(&contents) {
        Ok(raw) => raw,
        Err(error) => {
            return recover_invalid_config(path, Some(contents.as_str()), &error)
                .with_context(|| format!("parse config {}", path.display()));
        }
    };
    finalize_loaded_config(path, Some(contents.as_str()), raw)
}

fn finalize_loaded_config(
    path: &Path,
    contents: Option<&str>,
    raw: RawConfig,
) -> Result<AppConfig> {
    let (raw, added_defaults) = raw_config_with_defaults(raw)?;
    let config = match resolve_app_config(raw.clone(), true) {
        Ok(config) => config,
        Err(error) => {
            return recover_invalid_config(path, contents, &error)
                .with_context(|| format!("resolve config {}", path.display()));
        }
    };
    if added_defaults {
        save_config(&resolve_app_config(raw, false)?)?;
    }
    Ok(config)
}

fn recover_invalid_config(
    path: &Path,
    contents: Option<&str>,
    error: &anyhow::Error,
) -> Result<AppConfig> {
    let default = default_app_config()?;
    let error_text = format!("{error:#}");
    let line_number = extract_line_number(&error_text);
    let line_text = line_number
        .and_then(|line| contents.and_then(|text| text.lines().nth(line.saturating_sub(1))))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string);
    log::warn!(
        "[config] invalid config at {}: {error:#}. Keeping the file unchanged and using defaults for this launch.",
        path.display(),
    );

    set_config_recovery_notice(ConfigRecoveryNotice {
        path: path.display().to_string(),
        backup_path: None,
        error: error_text,
        line_number,
        line_text,
        used_defaults: true,
    });

    Ok(default)
}

fn config_recovery_notice_slot() -> &'static Mutex<Option<ConfigRecoveryNotice>> {
    CONFIG_RECOVERY_NOTICE.get_or_init(|| Mutex::new(None))
}

fn set_config_recovery_notice(notice: ConfigRecoveryNotice) {
    *config_recovery_notice_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(notice);
}

fn extract_line_number(message: &str) -> Option<usize> {
    let needle = "line ";
    let start = message.find(needle)? + needle.len();
    let digits = message[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<usize>().ok()
    }
}

fn line_context(line_number: usize, raw_line: &str) -> String {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        format!("line {line_number}")
    } else {
        format!("line {line_number}: {trimmed}")
    }
}

fn parse_raw_config(contents: &str) -> Result<RawConfig> {
    let mut raw = RawConfig::default();
    let mut section = Section::Root;

    for (index, raw_line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section_name) =
            parse_section(line).with_context(|| line_context(line_number, raw_line))?
        {
            section = section_name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bail!("line {line_number}: invalid config line: {line}");
        };
        let key = key.trim();
        let value =
            parse_value(value.trim()).with_context(|| line_context(line_number, raw_line))?;
        match section {
            Section::Memory => match key {
                "backend" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backend = value
                }
                other => bail!("line {line_number}: unknown key in [memory]: {other}"),
            },
            Section::MemoryBackendsQmd => match key {
                "binary" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .binary = value
                }
                "index" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .index = value
                }
                "artifacts_root" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .artifacts_root = value
                }
                "auto_embed" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .auto_embed = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "install_command" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .install_command = value
                }
                other => bail!("line {line_number}: unknown key in [memory.backends.qmd]: {other}"),
            },
            Section::Index => match key {
                "poll_interval_seconds" => {
                    raw.index.poll_interval_seconds = value
                        .map(parse_u64)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [index]: {other}"),
            },
            Section::NetworkProxy => match key {
                "enabled" => {
                    raw.network.proxy.enabled = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "url" => raw.network.proxy.url = value,
                "no_proxy" => raw.network.proxy.no_proxy = value,
                other => bail!("line {line_number}: unknown key in [network.proxy]: {other}"),
            },
            Section::Appshot => match key {
                "shortcut" => raw.appshot.shortcut = value,
                other => bail!("line {line_number}: unknown key in [appshot]: {other}"),
            },
            Section::ComputerUse => match key {
                "enabled" => {
                    raw.computer_use.enabled = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "approved_apps" => {
                    raw.computer_use.approved_apps = value
                        .map(|value| parse_string_array(&value))
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "app_route_preferences" => {
                    raw.computer_use.app_route_preferences = value
                        .map(|value| {
                            serde_json::from_str::<
                                std::collections::BTreeMap<
                                    String,
                                    crate::computer_use::settings::AppRoutePreferences,
                                >,
                            >(&value)
                            .map_err(anyhow::Error::from)
                        })
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "allow_input_injection" => {
                    raw.computer_use.allow_input_injection = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "allow_foreground_takeover" => {
                    raw.computer_use.allow_foreground_takeover =
                        value
                            .map(parse_bool)
                            .transpose()
                            .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [computer_use]: {other}"),
            },
            Section::Debug => match key {
                "acp_config" => {
                    raw.debug.acp_config = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                "update_preview" => {
                    raw.debug.update_preview = value
                        .map(parse_bool)
                        .transpose()
                        .with_context(|| line_context(line_number, raw_line))?
                }
                other => bail!("line {line_number}: unknown key in [debug]: {other}"),
            },
            Section::Root | Section::Ignored => {}
        }
    }

    Ok(raw)
}

#[derive(Debug, Clone)]
enum Section {
    Root,
    Memory,
    MemoryBackendsQmd,
    Index,
    NetworkProxy,
    Appshot,
    ComputerUse,
    Debug,
    Ignored,
}

fn parse_section(line: &str) -> Result<Option<Section>> {
    if !(line.starts_with('[') && line.ends_with(']')) {
        return Ok(None);
    }
    let name = &line[1..line.len() - 1];
    let parts: Vec<String> = name
        .split('.')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect();
    if parts.is_empty() {
        bail!("empty section header");
    }
    Ok(Some(match parts.as_slice() {
        [a] if a == "memory" => Section::Memory,
        [a] if a == "index" => Section::Index,
        [a, b] if a == "network" && b == "proxy" => Section::NetworkProxy,
        [a] if a == "appshot" => Section::Appshot,
        [a] if a == "computer_use" => Section::ComputerUse,
        [a, ..] if a == "astra" => Section::Ignored,
        [a] if a == "debug" => Section::Debug,
        [a, b, c] if a == "memory" && b == "backends" && c == "qmd" => Section::MemoryBackendsQmd,
        _ => Section::Ignored,
    }))
}

fn parse_value(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("null") {
        return Ok(None);
    }
    if let Some(stripped) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        return Ok(Some(unescape_string(stripped)?));
    }
    if value.is_empty() {
        return Ok(Some(String::new()));
    }
    Ok(Some(value.to_string()))
}

fn parse_bool(value: String) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        other => bail!("invalid boolean value: {other}"),
    }
}

fn parse_u64(value: String) -> Result<u64> {
    value
        .parse::<u64>()
        .with_context(|| format!("invalid unsigned integer value: {value}"))
}

fn parse_string_array(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    let Some(inner) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) else {
        bail!("invalid string array value: {value}");
    };
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            current.push('\\');
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => {
                current.push(ch);
                in_string = !in_string;
            }
            ',' if !in_string => {
                push_string_array_item(&mut items, &current)?;
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if in_string {
        bail!("unterminated string in array");
    }
    if !current.trim().is_empty() {
        push_string_array_item(&mut items, &current)?;
    }
    Ok(items)
}

fn push_string_array_item(items: &mut Vec<String>, raw: &str) -> Result<()> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(());
    }
    let Some(stripped) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
        bail!("string array item must be quoted: {value}");
    };
    items.push(unescape_string(stripped)?);
    Ok(())
}

fn unescape_string(value: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            bail!("unfinished escape sequence");
        };
        match next {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            other => out.push(other),
        }
    }
    Ok(out)
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn resolve_app_config(raw: RawConfig, apply_env: bool) -> Result<AppConfig> {
    let memory = raw
        .memory
        .clone()
        .map(|memory| resolve_memory_config_inner(memory, apply_env))
        .transpose()?;
    Ok(AppConfig {
        memory,
        index: resolve_index_config(raw.clone()),
        network: resolve_network_config(raw.clone()),
        appshot: resolve_appshot_config(raw.clone()),
        computer_use: resolve_computer_use_config(raw.clone()),
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

fn resolve_appshot_config(raw: RawConfig) -> AppshotConfig {
    AppshotConfig {
        shortcut: trimmed_string(raw.appshot.shortcut.as_deref())
            .unwrap_or_else(|| AppshotConfig::default().shortcut),
    }
}

fn resolve_computer_use_config(raw: RawConfig) -> ComputerUseSettings {
    let defaults = ComputerUseSettings::recommended();
    let _legacy_control_settings = (
        raw.computer_use.allow_input_injection,
        raw.computer_use.allow_foreground_takeover,
    );
    ComputerUseSettings {
        enabled: raw.computer_use.enabled.unwrap_or(defaults.enabled),
        approved_apps: normalized_string_list(raw.computer_use.approved_apps.unwrap_or_default()),
        app_route_preferences: raw.computer_use.app_route_preferences.unwrap_or_default(),
    }
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

fn resolve_memory_config_inner(raw: RawMemoryConfig, apply_env: bool) -> Result<MemoryConfig> {
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

fn raw_config_with_defaults(mut raw: RawConfig) -> Result<(RawConfig, bool)> {
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
    merge_option(
        &mut raw.computer_use.enabled,
        defaults.computer_use.enabled,
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

fn config_path() -> Result<PathBuf> {
    app_paths::config_path()
}

fn write_default_config_file(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    let config = default_app_config()?;
    fs::write(path, serialize_app_config(&config))
        .with_context(|| format!("write config {}", path.display()))
}

fn default_app_config() -> Result<AppConfig> {
    Ok(AppConfig {
        memory: None,
        index: IndexConfig {
            poll_interval_seconds: 60,
        },
        network: NetworkConfig::default(),
        appshot: AppshotConfig::default(),
        computer_use: ComputerUseSettings::recommended(),
        debug: DebugConfig {
            acp_config: false,
            update_preview: false,
        },
    })
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

fn toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serialize config string")
}

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
    out.push_str(&serialize_appshot_config(&config.appshot));
    out.push('\n');
    out.push_str(&serialize_computer_use_config(&config.computer_use));
    out.push('\n');
    out.push_str(&serialize_debug_config(&config.debug));
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

fn serialize_appshot_config(config: &AppshotConfig) -> String {
    let mut out = String::new();
    out.push_str("[appshot]\n");
    out.push_str("shortcut = ");
    out.push_str(&toml_string(&config.shortcut));
    out.push('\n');
    out
}

fn serialize_computer_use_config(config: &ComputerUseSettings) -> String {
    let mut out = String::new();
    out.push_str("[computer_use]\n");
    out.push_str("enabled = ");
    out.push_str(if config.enabled { "true" } else { "false" });
    out.push('\n');
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

fn trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
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
            [computer_use]
            enabled = false
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
        assert!(serialized.contains(r#"approved_apps = ["com.example.one", "com.example.two"]"#));
        assert!(serialized.contains("app_route_preferences = "));
        assert!(!serialized.contains("allow_input_injection"));
        assert!(!serialized.contains("allow_foreground_takeover"));
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
