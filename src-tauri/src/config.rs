use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::memory::build::default_artifacts_root;

#[derive(Debug, Clone, Serialize)]
pub struct AppConfig {
    pub memory: MemoryConfig,
    pub debug: DebugConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugConfig {
    pub acp_config: bool,
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
    memory: RawMemoryConfig,
    debug: RawDebugConfig,
}

#[derive(Debug, Clone, Default)]
struct RawDebugConfig {
    acp_config: Option<bool>,
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

pub fn load_config() -> Result<AppConfig> {
    let raw = load_raw_config()?;
    let (raw, added_defaults) = raw_config_with_defaults(raw)?;
    if added_defaults {
        save_config(&resolve_app_config(raw.clone(), false)?)?;
    }
    resolve_app_config(raw, true)
}

pub fn load_memory_config() -> Result<MemoryConfig> {
    Ok(load_config()?.memory)
}

pub fn save_memory_config(config: &MemoryConfig) -> Result<()> {
    let mut app_config = load_config().or_else(|_| default_app_config())?;
    app_config.memory = config.clone();
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

fn load_raw_config() -> Result<RawConfig> {
    let path = config_path()?;
    if !path.exists() {
        write_default_config_file(&path)?;
    }
    let contents =
        fs::read_to_string(&path).with_context(|| format!("read config {}", path.display()))?;
    if contents.trim().is_empty() {
        return Ok(RawConfig::default());
    }
    parse_raw_config(&contents).with_context(|| format!("parse config {}", path.display()))
}

fn parse_raw_config(contents: &str) -> Result<RawConfig> {
    let mut raw = RawConfig::default();
    let mut section = Section::Root;

    for line in contents.lines() {
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(section_name) = parse_section(line)? {
            section = section_name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            bail!("invalid config line: {line}");
        };
        let key = key.trim();
        let value = parse_value(value.trim())?;
        match section {
            Section::Memory => match key {
                "backend" => raw.memory.backend = value,
                other => bail!("unknown key in [memory]: {other}"),
            },
            Section::MemoryBackendsQmd => match key {
                "binary" => raw.memory.backends.qmd.binary = value,
                "index" => raw.memory.backends.qmd.index = value,
                "artifacts_root" => raw.memory.backends.qmd.artifacts_root = value,
                "auto_embed" => {
                    raw.memory.backends.qmd.auto_embed = value.map(parse_bool).transpose()?
                }
                "install_command" => raw.memory.backends.qmd.install_command = value,
                other => bail!("unknown key in [memory.backends.qmd]: {other}"),
            },
            Section::Debug => match key {
                "acp_config" => raw.debug.acp_config = value.map(parse_bool).transpose()?,
                other => bail!("unknown key in [debug]: {other}"),
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
    Ok(AppConfig {
        memory: resolve_memory_config_inner(raw.clone(), apply_env)?,
        debug: resolve_debug_config(raw),
    })
}

fn resolve_debug_config(raw: RawConfig) -> DebugConfig {
    DebugConfig {
        acp_config: raw.debug.acp_config.unwrap_or(false),
    }
}

fn resolve_memory_config_inner(raw: RawConfig, apply_env: bool) -> Result<MemoryConfig> {
    let backend = raw.memory.backend.unwrap_or_else(|| "qmd".to_string());
    if backend != "qmd" {
        bail!("unsupported memory backend in config: {backend}");
    }

    let qmd = raw.memory.backends.qmd;
    let mut config = QmdBackendConfig {
        binary: qmd.binary,
        index: qmd.index.unwrap_or_else(|| "sessio".to_string()),
        artifacts_root: qmd
            .artifacts_root
            .as_deref()
            .map(expand_path)
            .transpose()?
            .unwrap_or(default_artifacts_root()?),
        auto_embed: qmd.auto_embed.unwrap_or(false),
        install_command: qmd
            .install_command
            .unwrap_or_else(|| "npm install -g @tobilu/qmd".to_string()),
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
        &mut raw.memory.backend,
        defaults.memory.backend,
        &mut changed,
    );
    merge_option(
        &mut raw.memory.backends.qmd.binary,
        defaults.memory.backends.qmd.binary,
        &mut changed,
    );
    merge_option(
        &mut raw.memory.backends.qmd.index,
        defaults.memory.backends.qmd.index,
        &mut changed,
    );
    merge_option(
        &mut raw.memory.backends.qmd.artifacts_root,
        defaults.memory.backends.qmd.artifacts_root,
        &mut changed,
    );
    merge_option(
        &mut raw.memory.backends.qmd.auto_embed,
        defaults.memory.backends.qmd.auto_embed,
        &mut changed,
    );
    merge_option(
        &mut raw.memory.backends.qmd.install_command,
        defaults.memory.backends.qmd.install_command,
        &mut changed,
    );
    merge_option(
        &mut raw.debug.acp_config,
        defaults.debug.acp_config,
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
    let home = dirs::home_dir().context("no home dir")?;
    Ok(home.join(".sessio").join("config.toml"))
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
        memory: default_memory_config()?,
        debug: DebugConfig { acp_config: false },
    })
}

fn default_memory_config() -> Result<MemoryConfig> {
    Ok(MemoryConfig {
        backend: "qmd".to_string(),
        qmd: QmdBackendConfig {
            binary: None,
            index: "sessio".to_string(),
            artifacts_root: default_artifacts_root()?,
            auto_embed: false,
            install_command: "npm install -g @tobilu/qmd".to_string(),
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
    out.push_str(&serialize_memory_config(&config.memory));
    out.push('\n');
    out.push_str(&serialize_debug_config(&config.debug));
    out
}

fn serialize_debug_config(config: &DebugConfig) -> String {
    let mut out = String::new();
    out.push_str("[debug]\n");
    out.push_str("acp_config = ");
    out.push_str(if config.acp_config { "true" } else { "false" });
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::{
        default_app_config, parse_raw_config, raw_config_with_defaults,
        resolve_memory_config_inner, serialize_app_config,
    };

    fn resolve_memory_config(raw: super::RawConfig) -> super::Result<super::MemoryConfig> {
        resolve_memory_config_inner(raw, true)
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
        let raw = parse_raw_config(
            r#"
            # global comment
            [memory.backends.qmd]
            auto_embed = true  # inline comment after value
            "#,
        )
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
            "#,
        )
        .unwrap();
        let config = resolve_memory_config(raw).unwrap();
        assert_eq!(config.backend, "qmd");
    }

    #[test]
    fn comment_inside_quoted_string_is_preserved() {
        let raw = parse_raw_config(
            r#"
            [memory.backends.qmd]
            binary = "/path/with#hash/qmd"
            "#,
        )
        .unwrap();
        let config = resolve_memory_config(raw).unwrap();
        assert_eq!(config.qmd.binary.as_deref(), Some("/path/with#hash/qmd"));
    }

    #[test]
    fn default_app_config_serializes_memory_and_debug_only() {
        let config = default_app_config().unwrap();
        let serialized = serialize_app_config(&config);
        let raw = parse_raw_config(&serialized).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(serialized.contains("[memory]"));
        assert!(serialized.contains("[debug]"));
        assert!(!serialized.contains("[agents.runtime"));
        assert_eq!(config.memory.backend, "qmd");
    }

    #[test]
    fn empty_config_is_completed_with_memory_and_debug_defaults() {
        let raw = parse_raw_config("").unwrap();
        let (raw, changed) = raw_config_with_defaults(raw).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(changed);
        assert_eq!(config.memory.backend, "qmd");
        assert!(!config.debug.acp_config);
    }
}
