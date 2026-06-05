use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,
    pub index: IndexConfig,
    pub astra: AstraConfig,
    pub debug: DebugConfig,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexConfig {
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraConfig {
    pub round_limit: u32,
    pub retry_limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pi: Option<AstraPiConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraPiConfig {
    pub command: String,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub session_dir: Option<String>,
    pub env: BTreeMap<String, String>,
    pub planner: AstraPiPurposeConfig,
    pub decision: AstraPiPurposeConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AstraPiPurposeConfig {
    pub command: Option<String>,
    pub model: Option<String>,
    pub thinking_level: Option<String>,
    pub timeout_ms: u64,
    pub session_dir: Option<String>,
    pub env: BTreeMap<String, String>,
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
    astra: RawAstraConfig,
    debug: RawDebugConfig,
}

#[derive(Debug, Clone, Default)]
struct RawIndexConfig {
    poll_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct RawAstraConfig {
    round_limit: Option<u32>,
    retry_limit: Option<u32>,
    pi: RawAstraPiConfig,
}

#[derive(Debug, Clone, Default)]
struct RawAstraPiConfig {
    command: Option<String>,
    model: Option<String>,
    thinking_level: Option<String>,
    session_dir: Option<String>,
    env: BTreeMap<String, String>,
    planner: RawAstraPiPurposeConfig,
    decision: RawAstraPiPurposeConfig,
}

#[derive(Debug, Clone, Default)]
struct RawAstraPiPurposeConfig {
    command: Option<String>,
    model: Option<String>,
    thinking_level: Option<String>,
    timeout_ms: Option<u64>,
    session_dir: Option<String>,
    env: BTreeMap<String, String>,
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

pub fn load_config() -> Result<AppConfig> {
    let raw = load_raw_config()?;
    let (raw, added_defaults) = raw_config_with_defaults(raw)?;
    if added_defaults {
        save_config(&resolve_app_config(raw.clone(), false)?)?;
    }
    resolve_app_config(raw, true)
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
                "backend" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backend = value
                }
                other => bail!("unknown key in [memory]: {other}"),
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
                        .auto_embed = value.map(parse_bool).transpose()?
                }
                "install_command" => {
                    raw.memory
                        .get_or_insert_with(RawMemoryConfig::default)
                        .backends
                        .qmd
                        .install_command = value
                }
                other => bail!("unknown key in [memory.backends.qmd]: {other}"),
            },
            Section::Index => match key {
                "poll_interval_seconds" => {
                    raw.index.poll_interval_seconds = value.map(parse_u64).transpose()?
                }
                other => bail!("unknown key in [index]: {other}"),
            },
            Section::Astra => match key {
                "round_limit" => raw.astra.round_limit = value.map(parse_u32).transpose()?,
                "retry_limit" => raw.astra.retry_limit = value.map(parse_u32).transpose()?,
                other => bail!("unknown key in [astra]: {other}"),
            },
            Section::AstraPi => match key {
                "command" => raw.astra.pi.command = value,
                "model" => raw.astra.pi.model = value,
                "thinking_level" => raw.astra.pi.thinking_level = value,
                "session_dir" => raw.astra.pi.session_dir = value,
                other => bail!("unknown key in [astra.pi]: {other}"),
            },
            Section::AstraPiEnv => {
                raw.astra
                    .pi
                    .env
                    .insert(key.to_string(), value.unwrap_or_default());
            }
            Section::AstraPiPlanner => match key {
                "command" => raw.astra.pi.planner.command = value,
                "model" => raw.astra.pi.planner.model = value,
                "thinking_level" => raw.astra.pi.planner.thinking_level = value,
                "timeout_ms" => {
                    raw.astra.pi.planner.timeout_ms = value.map(parse_u64).transpose()?
                }
                "session_dir" => raw.astra.pi.planner.session_dir = value,
                other => bail!("unknown key in [astra.pi.planner]: {other}"),
            },
            Section::AstraPiPlannerEnv => {
                raw.astra
                    .pi
                    .planner
                    .env
                    .insert(key.to_string(), value.unwrap_or_default());
            }
            Section::AstraPiDecision => match key {
                "command" => raw.astra.pi.decision.command = value,
                "model" => raw.astra.pi.decision.model = value,
                "thinking_level" => raw.astra.pi.decision.thinking_level = value,
                "timeout_ms" => {
                    raw.astra.pi.decision.timeout_ms = value.map(parse_u64).transpose()?
                }
                "session_dir" => raw.astra.pi.decision.session_dir = value,
                other => bail!("unknown key in [astra.pi.decision]: {other}"),
            },
            Section::AstraPiDecisionEnv => {
                raw.astra
                    .pi
                    .decision
                    .env
                    .insert(key.to_string(), value.unwrap_or_default());
            }
            Section::Debug => match key {
                "acp_config" => raw.debug.acp_config = value.map(parse_bool).transpose()?,
                "update_preview" => raw.debug.update_preview = value.map(parse_bool).transpose()?,
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
    Index,
    Astra,
    AstraPi,
    AstraPiEnv,
    AstraPiPlanner,
    AstraPiPlannerEnv,
    AstraPiDecision,
    AstraPiDecisionEnv,
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
        [a] if a == "astra" => Section::Astra,
        [a, b] if a == "astra" && b == "pi" => Section::AstraPi,
        [a, b, c] if a == "astra" && b == "pi" && c == "env" => Section::AstraPiEnv,
        [a, b, c] if a == "astra" && b == "pi" && c == "planner" => Section::AstraPiPlanner,
        [a, b, c, d] if a == "astra" && b == "pi" && c == "planner" && d == "env" => {
            Section::AstraPiPlannerEnv
        }
        [a, b, c] if a == "astra" && b == "pi" && c == "decision" => Section::AstraPiDecision,
        [a, b, c, d] if a == "astra" && b == "pi" && c == "decision" && d == "env" => {
            Section::AstraPiDecisionEnv
        }
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

fn parse_u32(value: String) -> Result<u32> {
    value
        .parse::<u32>()
        .with_context(|| format!("invalid unsigned integer value: {value}"))
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
        astra: resolve_astra_config(raw.clone(), apply_env)?,
        debug: resolve_debug_config(raw),
    })
}

fn resolve_index_config(raw: RawConfig) -> IndexConfig {
    IndexConfig {
        poll_interval_seconds: raw.index.poll_interval_seconds.unwrap_or(60),
    }
}

fn resolve_astra_config(raw: RawConfig, apply_env: bool) -> Result<AstraConfig> {
    let mut config = AstraConfig {
        round_limit: raw.astra.round_limit.unwrap_or(3),
        retry_limit: raw.astra.retry_limit.unwrap_or(3),
        pi: resolve_astra_pi_config(raw.astra.pi, apply_env)?,
    };
    if apply_env {
        if let Ok(value) = std::env::var("SESSIO_ASTRA_ROUND_LIMIT") {
            if !value.trim().is_empty() {
                config.round_limit = parse_u32(value)?;
            }
        }
        if let Ok(value) = std::env::var("SESSIO_ASTRA_RETRY_LIMIT") {
            if !value.trim().is_empty() {
                config.retry_limit = parse_u32(value)?;
            }
        }
    }
    if config.round_limit == 0 {
        bail!("astra.round_limit must be greater than 0");
    }
    if config.retry_limit == 0 {
        bail!("astra.retry_limit must be greater than 0");
    }
    Ok(config)
}

fn resolve_astra_pi_config(
    mut raw: RawAstraPiConfig,
    apply_env: bool,
) -> Result<Option<AstraPiConfig>> {
    if apply_env {
        apply_string_env(&mut raw.command, "SESSIO_ASTRA_PI_COMMAND");
        apply_string_env(&mut raw.model, "SESSIO_ASTRA_PI_MODEL");
        apply_string_env(&mut raw.thinking_level, "SESSIO_ASTRA_PI_THINKING_LEVEL");
        apply_string_env(&mut raw.session_dir, "SESSIO_ASTRA_PI_SESSION_DIR");
        apply_env_map(&mut raw.env, "SESSIO_ASTRA_PI_ENV")?;
        apply_string_env(&mut raw.planner.command, "SESSIO_ASTRA_PI_PLANNER_COMMAND");
        apply_string_env(&mut raw.planner.model, "SESSIO_ASTRA_PI_PLANNER_MODEL");
        apply_string_env(
            &mut raw.planner.thinking_level,
            "SESSIO_ASTRA_PI_PLANNER_THINKING_LEVEL",
        );
        apply_u64_env(
            &mut raw.planner.timeout_ms,
            "SESSIO_ASTRA_PI_PLANNER_TIMEOUT_MS",
        )?;
        apply_string_env(
            &mut raw.planner.session_dir,
            "SESSIO_ASTRA_PI_PLANNER_SESSION_DIR",
        );
        apply_env_map(&mut raw.planner.env, "SESSIO_ASTRA_PI_PLANNER_ENV")?;
        apply_string_env(
            &mut raw.decision.command,
            "SESSIO_ASTRA_PI_DECISION_COMMAND",
        );
        apply_string_env(&mut raw.decision.model, "SESSIO_ASTRA_PI_DECISION_MODEL");
        apply_string_env(
            &mut raw.decision.thinking_level,
            "SESSIO_ASTRA_PI_DECISION_THINKING_LEVEL",
        );
        apply_u64_env(
            &mut raw.decision.timeout_ms,
            "SESSIO_ASTRA_PI_DECISION_TIMEOUT_MS",
        )?;
        apply_string_env(
            &mut raw.decision.session_dir,
            "SESSIO_ASTRA_PI_DECISION_SESSION_DIR",
        );
        apply_env_map(&mut raw.decision.env, "SESSIO_ASTRA_PI_DECISION_ENV")?;
    }

    let Some(command) = raw.command.clone().filter(|value| !value.trim().is_empty()) else {
        return Ok(None);
    };
    validate_env_keys("astra.pi.env", &raw.env)?;
    validate_env_keys("astra.pi.planner.env", &raw.planner.env)?;
    validate_env_keys("astra.pi.decision.env", &raw.decision.env)?;
    let planner = resolve_astra_pi_purpose_config(&raw, raw.planner.clone(), 30_000, "planner")?;
    let decision = resolve_astra_pi_purpose_config(&raw, raw.decision.clone(), 30_000, "decision")?;
    Ok(Some(AstraPiConfig {
        command,
        model: raw.model.filter(|value| !value.trim().is_empty()),
        thinking_level: raw.thinking_level.filter(|value| !value.trim().is_empty()),
        session_dir: raw.session_dir.filter(|value| !value.trim().is_empty()),
        env: raw.env,
        planner,
        decision,
    }))
}

fn resolve_astra_pi_purpose_config(
    common: &RawAstraPiConfig,
    raw: RawAstraPiPurposeConfig,
    default_timeout_ms: u64,
    purpose: &str,
) -> Result<AstraPiPurposeConfig> {
    let timeout_ms = raw.timeout_ms.unwrap_or(default_timeout_ms);
    if timeout_ms == 0 {
        bail!("astra.pi.{purpose}.timeout_ms must be greater than 0");
    }
    let mut env = common.env.clone();
    env.extend(raw.env);
    Ok(AstraPiPurposeConfig {
        command: raw.command.filter(|value| !value.trim().is_empty()),
        model: raw
            .model
            .or_else(|| common.model.clone())
            .filter(|value| !value.trim().is_empty()),
        thinking_level: raw
            .thinking_level
            .or_else(|| common.thinking_level.clone())
            .filter(|value| !value.trim().is_empty()),
        timeout_ms,
        session_dir: raw
            .session_dir
            .or_else(|| common.session_dir.clone())
            .filter(|value| !value.trim().is_empty()),
        env,
    })
}

fn apply_string_env(target: &mut Option<String>, key: &str) {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            *target = Some(value);
        }
    }
}

fn apply_u64_env(target: &mut Option<u64>, key: &str) -> Result<()> {
    if let Ok(value) = std::env::var(key) {
        if !value.trim().is_empty() {
            *target = Some(parse_u64(value)?);
        }
    }
    Ok(())
}

fn apply_env_map(target: &mut BTreeMap<String, String>, key: &str) -> Result<()> {
    let Ok(value) = std::env::var(key) else {
        return Ok(());
    };
    if value.trim().is_empty() {
        return Ok(());
    }
    let env: BTreeMap<String, String> =
        serde_json::from_str(&value).with_context(|| format!("parse {key} JSON object"))?;
    target.extend(env);
    Ok(())
}

fn validate_env_keys(section: &str, env: &BTreeMap<String, String>) -> Result<()> {
    for key in env.keys() {
        if !valid_env_key(key) {
            bail!("{section} contains invalid env key: {key}");
        }
    }
    Ok(())
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
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
        &mut raw.astra.round_limit,
        defaults.astra.round_limit,
        &mut changed,
    );
    merge_option(
        &mut raw.astra.retry_limit,
        defaults.astra.retry_limit,
        &mut changed,
    );
    merge_option(
        &mut raw.debug.acp_config,
        defaults.debug.acp_config,
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
        memory: None,
        index: IndexConfig {
            poll_interval_seconds: 60,
        },
        astra: AstraConfig {
            round_limit: 3,
            retry_limit: 3,
            pi: None,
        },
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
    out.push_str(&serialize_astra_config(&config.astra));
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

fn serialize_astra_config(config: &AstraConfig) -> String {
    let mut out = String::new();
    out.push_str("[astra]\n");
    out.push_str("round_limit = ");
    out.push_str(&config.round_limit.to_string());
    out.push('\n');
    out.push_str("retry_limit = ");
    out.push_str(&config.retry_limit.to_string());
    out.push('\n');
    if let Some(pi) = &config.pi {
        out.push('\n');
        out.push_str("[astra.pi]\n");
        out.push_str("command = ");
        out.push_str(&toml_string(&pi.command));
        out.push('\n');
        if let Some(model) = &pi.model {
            out.push_str("model = ");
            out.push_str(&toml_string(model));
            out.push('\n');
        }
        if let Some(thinking_level) = &pi.thinking_level {
            out.push_str("thinking_level = ");
            out.push_str(&toml_string(thinking_level));
            out.push('\n');
        }
        if let Some(session_dir) = &pi.session_dir {
            out.push_str("session_dir = ");
            out.push_str(&toml_string(session_dir));
            out.push('\n');
        }
        serialize_env_section("astra.pi.env", &pi.env, &mut out);
        serialize_astra_pi_purpose_config("planner", &pi.planner, &pi.env, &mut out);
        serialize_astra_pi_purpose_config("decision", &pi.decision, &pi.env, &mut out);
    }
    out
}

fn serialize_astra_pi_purpose_config(
    purpose: &str,
    config: &AstraPiPurposeConfig,
    common_env: &BTreeMap<String, String>,
    out: &mut String,
) {
    out.push('\n');
    out.push_str("[astra.pi.");
    out.push_str(purpose);
    out.push_str("]\n");
    if let Some(command) = &config.command {
        out.push_str("command = ");
        out.push_str(&toml_string(command));
        out.push('\n');
    }
    if let Some(model) = &config.model {
        out.push_str("model = ");
        out.push_str(&toml_string(model));
        out.push('\n');
    }
    if let Some(thinking_level) = &config.thinking_level {
        out.push_str("thinking_level = ");
        out.push_str(&toml_string(thinking_level));
        out.push('\n');
    }
    out.push_str("timeout_ms = ");
    out.push_str(&config.timeout_ms.to_string());
    out.push('\n');
    if let Some(session_dir) = &config.session_dir {
        out.push_str("session_dir = ");
        out.push_str(&toml_string(session_dir));
        out.push('\n');
    }
    let purpose_env = config
        .env
        .iter()
        .filter(|(key, value)| common_env.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    serialize_env_section(&format!("astra.pi.{purpose}.env"), &purpose_env, out);
}

fn serialize_env_section(section: &str, env: &BTreeMap<String, String>, out: &mut String) {
    if env.is_empty() {
        return;
    }
    out.push('\n');
    out.push('[');
    out.push_str(section);
    out.push_str("]\n");
    for (key, value) in env {
        out.push_str(key);
        out.push_str(" = ");
        out.push_str(&toml_string(value));
        out.push('\n');
    }
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
    use anyhow::Context;

    use super::{
        default_app_config, parse_raw_config, raw_config_with_defaults,
        resolve_memory_config_inner, serialize_app_config,
    };

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
    fn parses_astra_limits() {
        let raw = parse_raw_config(
            r#"
            [astra]
            round_limit = 7
            retry_limit = 4
            "#,
        )
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert_eq!(config.astra.round_limit, 7);
        assert_eq!(config.astra.retry_limit, 4);
    }

    #[test]
    fn parses_astra_pi_config() {
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
        let pi = config.astra.pi.unwrap();

        assert_eq!(pi.command, "pi-agent --acp");
        assert_eq!(pi.model.as_deref(), Some("pi-model"));
        assert_eq!(pi.env.get("COMMON").map(String::as_str), Some("base"));
        assert_eq!(pi.planner.model.as_deref(), Some("pi-model"));
        assert_eq!(pi.planner.thinking_level.as_deref(), Some("medium"));
        assert_eq!(pi.planner.timeout_ms, 1000);
        assert_eq!(pi.planner.session_dir.as_deref(), Some("/tmp/pi-sessions"));
        assert_eq!(
            pi.planner.env.get("COMMON").map(String::as_str),
            Some("base")
        );
        assert_eq!(
            pi.planner.env.get("SHARED").map(String::as_str),
            Some("planner")
        );
        assert_eq!(
            pi.decision.command.as_deref(),
            Some("pi-agent --acp --decision")
        );
        assert_eq!(pi.decision.model.as_deref(), Some("decision-model"));
        assert_eq!(pi.decision.timeout_ms, 2000);
        assert_eq!(
            pi.decision.env.get("SHARED").map(String::as_str),
            Some("common")
        );
    }

    #[test]
    fn rejects_zero_astra_pi_timeout() {
        let raw = parse_raw_config(
            r#"
            [astra.pi]
            command = "pi-agent --acp"

            [astra.pi.planner]
            timeout_ms = 0
            "#,
        )
        .unwrap();

        assert!(super::resolve_app_config(raw, false).is_err());
    }

    #[test]
    fn serializes_astra_pi_env_sections() {
        let raw = parse_raw_config(
            r#"
            [astra.pi]
            command = "pi-agent --acp"

            [astra.pi.env]
            COMMON = "common"
            TOKEN = "common"

            [astra.pi.planner]
            timeout_ms = 1000

            [astra.pi.decision.env]
            TOKEN = "decision"
            "#,
        )
        .unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();
        let serialized = serialize_app_config(&config);

        assert!(serialized.contains("[astra.pi.env]"));
        assert!(serialized.contains("COMMON = \"common\""));
        assert!(serialized.contains("TOKEN = \"common\""));
        assert!(!serialized.contains("[astra.pi.planner.env]"));
        assert!(serialized.contains("[astra.pi.decision.env]"));
        assert!(serialized.contains("TOKEN = \"decision\""));
        assert_eq!(serialized.matches("COMMON = \"common\"").count(), 1);
    }

    #[test]
    fn rejects_invalid_astra_pi_env_key() {
        let raw = parse_raw_config(
            r#"
            [astra.pi]
            command = "pi-agent --acp"

            [astra.pi.env]
            API-KEY = "secret"
            "#,
        )
        .unwrap();

        let error = super::resolve_app_config(raw, false).unwrap_err();
        assert!(error.to_string().contains("invalid env key: API-KEY"));
    }

    #[test]
    fn rejects_zero_astra_limits() {
        let raw = parse_raw_config(
            r#"
            [astra]
            round_limit = 0
            retry_limit = 3
            "#,
        )
        .unwrap();

        assert!(super::resolve_app_config(raw, false).is_err());
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
        assert!(serialized.contains("[astra]"));
        assert!(serialized.contains("round_limit = 3"));
        assert!(serialized.contains("retry_limit = 3"));
        assert!(serialized.contains("[debug]"));
        assert!(!serialized.contains("[agents.runtime"));
        assert!(config.memory.is_none());
        assert_eq!(config.index.poll_interval_seconds, 60);
        assert_eq!(config.astra.round_limit, 3);
        assert_eq!(config.astra.retry_limit, 3);
    }

    #[test]
    fn empty_config_is_completed_with_debug_defaults_only() {
        let raw = parse_raw_config("").unwrap();
        let (raw, changed) = raw_config_with_defaults(raw).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(changed);
        assert!(config.memory.is_none());
        assert_eq!(config.index.poll_interval_seconds, 60);
        assert_eq!(config.astra.round_limit, 3);
        assert_eq!(config.astra.retry_limit, 3);
        assert!(!config.debug.acp_config);
        assert!(!config.debug.update_preview);
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
}
