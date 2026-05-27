use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::memory::build::default_artifacts_root;
use crate::models::Agent;

#[derive(Debug, Clone, Serialize)]
pub struct AppConfig {
    pub memory: MemoryConfig,
    pub agents: AgentsConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentsConfig {
    pub runtime: RuntimeAgentsConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RuntimeAgentsConfig {
    pub codex: AgentRuntimeConfig,
    pub claude: AgentRuntimeConfig,
    pub gemini: AgentRuntimeConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentRuntimeConfig {
    pub enabled: bool,
    pub transport: Option<String>,
    pub model: Option<String>,
    pub models: Vec<AgentRuntimeOptionConfig>,
    pub permission_mode: Option<String>,
    pub permission_modes: Vec<AgentRuntimeOptionConfig>,
    pub sandbox: Option<String>,
    pub command: AgentRuntimeCommandConfig,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentRuntimeOptionConfig {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentRuntimeCommandConfig {
    pub session: Option<String>,
    pub version: Option<String>,
}

impl RuntimeAgentsConfig {
    pub fn get(&self, agent: Agent) -> &AgentRuntimeConfig {
        match agent {
            Agent::Codex => &self.codex,
            Agent::Claude => &self.claude,
            Agent::Gemini => &self.gemini,
        }
    }
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
    agents: RawAgentsConfig,
}

#[derive(Debug, Clone, Default)]
struct RawAgentsConfig {
    runtime: RawRuntimeAgentsConfig,
}

#[derive(Debug, Clone, Default)]
struct RawRuntimeAgentsConfig {
    codex: RawAgentRuntimeConfig,
    claude: RawAgentRuntimeConfig,
    gemini: RawAgentRuntimeConfig,
}

#[derive(Debug, Clone, Default)]
struct RawAgentRuntimeConfig {
    enabled: Option<bool>,
    transport: Option<String>,
    model: Option<String>,
    models: Option<String>,
    permission_mode: Option<String>,
    permission_modes: Option<String>,
    sandbox: Option<String>,
    command: RawAgentRuntimeCommandConfig,
}

#[derive(Debug, Clone, Default)]
struct RawAgentRuntimeCommandConfig {
    session: Option<String>,
    version: Option<String>,
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

pub fn update_agent_runtime_preferences(
    agent: Agent,
    update: AgentRuntimePreferencesUpdate,
) -> Result<AppConfig> {
    let mut config = load_config()?;
    let runtime = match agent {
        Agent::Codex => &mut config.agents.runtime.codex,
        Agent::Claude => &mut config.agents.runtime.claude,
        Agent::Gemini => &mut config.agents.runtime.gemini,
    };
    if let Some(model) = normalize_optional_string(update.model) {
        runtime.model = Some(model);
    }
    if let Some(permission_mode) = normalize_optional_string(update.permission_mode) {
        runtime.permission_mode = Some(permission_mode);
    }
    for option in update.models {
        upsert_runtime_option(&mut runtime.models, option);
    }
    for option in update.permission_modes {
        upsert_runtime_option(&mut runtime.permission_modes, option);
    }
    save_config(&config)?;
    Ok(config)
}

#[derive(Debug, Clone, Default)]
pub struct AgentRuntimePreferencesUpdate {
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    pub models: Vec<AgentRuntimeOptionConfig>,
    pub permission_modes: Vec<AgentRuntimeOptionConfig>,
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
            Section::AgentRuntime(agent) => {
                let target = raw_runtime_agent_mut(&mut raw, agent);
                match key {
                    "enabled" => target.enabled = value.map(parse_bool).transpose()?,
                    "transport" => target.transport = value,
                    "model" => target.model = value,
                    "models" => target.models = value,
                    "permission_mode" => target.permission_mode = value,
                    "permission_modes" => target.permission_modes = value,
                    "sandbox" => target.sandbox = value,
                    "command" => target.command.session = value,
                    other => bail!(
                        "unknown key in [agents.runtime.{}]: {other}",
                        agent.as_str()
                    ),
                }
            }
            Section::AgentRuntimeCommand(agent) => {
                let target = raw_runtime_agent_mut(&mut raw, agent);
                match key {
                    "session" => target.command.session = value,
                    "version" => target.command.version = value,
                    other => bail!(
                        "unknown key in [agents.runtime.{}.command]: {other}",
                        agent.as_str()
                    ),
                }
            }
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
    AgentRuntime(Agent),
    AgentRuntimeCommand(Agent),
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
        [a, b, c] if a == "memory" && b == "backends" && c == "qmd" => Section::MemoryBackendsQmd,
        [a, b, c] if a == "agents" && b == "runtime" => match c.as_str() {
            "codex" => Section::AgentRuntime(Agent::Codex),
            "claude" => Section::AgentRuntime(Agent::Claude),
            "gemini" => Section::AgentRuntime(Agent::Gemini),
            _ => Section::Ignored,
        },
        [a, b, c, d] if a == "agents" && b == "runtime" && d == "command" => match c.as_str() {
            "codex" => Section::AgentRuntimeCommand(Agent::Codex),
            "claude" => Section::AgentRuntimeCommand(Agent::Claude),
            "gemini" => Section::AgentRuntimeCommand(Agent::Gemini),
            _ => Section::Ignored,
        },
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
        agents: resolve_agents_config(raw)?,
    })
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

    merge_raw_agent_runtime_defaults(
        &mut raw.agents.runtime.codex,
        defaults.agents.runtime.codex,
        &mut changed,
    )?;
    merge_raw_agent_runtime_defaults(
        &mut raw.agents.runtime.claude,
        defaults.agents.runtime.claude,
        &mut changed,
    )?;
    merge_raw_agent_runtime_defaults(
        &mut raw.agents.runtime.gemini,
        defaults.agents.runtime.gemini,
        &mut changed,
    )?;

    Ok((raw, changed))
}

fn merge_raw_agent_runtime_defaults(
    target: &mut RawAgentRuntimeConfig,
    defaults: RawAgentRuntimeConfig,
    changed: &mut bool,
) -> Result<()> {
    merge_option(&mut target.enabled, defaults.enabled, changed);
    merge_option(&mut target.transport, defaults.transport, changed);
    merge_option(&mut target.model, defaults.model, changed);
    merge_runtime_options_string(&mut target.models, defaults.models, changed)?;
    merge_option(
        &mut target.permission_mode,
        defaults.permission_mode,
        changed,
    );
    merge_runtime_options_string(
        &mut target.permission_modes,
        defaults.permission_modes,
        changed,
    )?;
    merge_option(&mut target.sandbox, defaults.sandbox, changed);
    merge_option(
        &mut target.command.session,
        defaults.command.session,
        changed,
    );
    merge_option(
        &mut target.command.version,
        defaults.command.version,
        changed,
    );
    Ok(())
}

fn merge_option<T>(target: &mut Option<T>, default: Option<T>, changed: &mut bool) {
    if target.is_none() && default.is_some() {
        *target = default;
        *changed = true;
    }
}

fn merge_runtime_options_string(
    target: &mut Option<String>,
    default: Option<String>,
    changed: &mut bool,
) -> Result<()> {
    let Some(default) = default.filter(|value| !value.trim().is_empty()) else {
        return Ok(());
    };
    let Some(current) = target.as_mut() else {
        *target = Some(default);
        *changed = true;
        return Ok(());
    };
    if current.trim().is_empty() {
        *current = default;
        *changed = true;
        return Ok(());
    }

    let mut options = parse_runtime_options(Some(current))?;
    let defaults = parse_runtime_options(Some(&default))?;
    let mut options_changed = false;
    for default_option in defaults {
        if options
            .iter()
            .any(|option| option.value == default_option.value)
        {
            continue;
        }
        options.push(default_option);
        options_changed = true;
    }
    if options_changed {
        *current = serialize_runtime_options(&options);
        *changed = true;
    }
    Ok(())
}

fn resolve_agents_config(raw: RawConfig) -> Result<AgentsConfig> {
    Ok(AgentsConfig {
        runtime: RuntimeAgentsConfig {
            codex: resolve_agent_runtime_config(raw.agents.runtime.codex)?,
            claude: resolve_agent_runtime_config(raw.agents.runtime.claude)?,
            gemini: resolve_agent_runtime_config(raw.agents.runtime.gemini)?,
        },
    })
}

fn resolve_agent_runtime_config(raw: RawAgentRuntimeConfig) -> Result<AgentRuntimeConfig> {
    let transport = raw.transport.filter(|value| !value.trim().is_empty());
    if let Some(transport) = &transport {
        match transport.as_str() {
            "fake" | "acp" | "cliStreamJson" | "plainCli" => {}
            other => bail!("unsupported runtime transport in config: {other}"),
        }
    }
    Ok(AgentRuntimeConfig {
        enabled: raw.enabled.unwrap_or(false),
        transport,
        model: raw.model.filter(|value| !value.trim().is_empty()),
        models: parse_runtime_options(raw.models.as_deref())?,
        permission_mode: raw.permission_mode.filter(|value| !value.trim().is_empty()),
        permission_modes: parse_runtime_options(raw.permission_modes.as_deref())?,
        sandbox: raw.sandbox.filter(|value| !value.trim().is_empty()),
        command: AgentRuntimeCommandConfig {
            session: raw.command.session.filter(|value| !value.trim().is_empty()),
            version: raw.command.version.filter(|value| !value.trim().is_empty()),
        },
    })
}

fn parse_runtime_options(value: Option<&str>) -> Result<Vec<AgentRuntimeOptionConfig>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in split_escaped(value, ',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut parts = split_escaped(entry, '|').into_iter();
        let value = unescape_runtime_option_part(parts.next().unwrap_or_default().trim())?;
        if value.is_empty() {
            continue;
        }
        let label = parts
            .next()
            .map(|part| unescape_runtime_option_part(part.trim()))
            .transpose()?
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| value.clone());
        out.push(AgentRuntimeOptionConfig { value, label });
    }
    Ok(out)
}

fn serialize_runtime_options(options: &[AgentRuntimeOptionConfig]) -> String {
    options
        .iter()
        .filter(|option| !option.value.trim().is_empty())
        .map(|option| {
            let label = if option.label.trim().is_empty() {
                option.value.as_str()
            } else {
                option.label.as_str()
            };
            format!(
                "{}|{}",
                escape_runtime_option_part(option.value.trim()),
                escape_runtime_option_part(label.trim())
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn split_escaped(value: &str, delimiter: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (idx, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == delimiter {
            out.push(&value[start..idx]);
            start = idx + ch.len_utf8();
        }
    }
    out.push(&value[start..]);
    out
}

fn escape_runtime_option_part(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace(',', "\\,")
}

fn unescape_runtime_option_part(value: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            bail!("unfinished runtime option escape sequence");
        };
        out.push(next);
    }
    Ok(out)
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn upsert_runtime_option(
    options: &mut Vec<AgentRuntimeOptionConfig>,
    option: AgentRuntimeOptionConfig,
) {
    let value = option.value.trim();
    if value.is_empty() {
        return;
    }
    let label = if option.label.trim().is_empty() {
        value.to_string()
    } else {
        option.label.trim().to_string()
    };
    if let Some(existing) = options.iter_mut().find(|existing| existing.value == value) {
        existing.label = label;
        return;
    }
    options.push(AgentRuntimeOptionConfig {
        value: value.to_string(),
        label,
    });
}

fn raw_runtime_agent_mut(raw: &mut RawConfig, agent: Agent) -> &mut RawAgentRuntimeConfig {
    match agent {
        Agent::Codex => &mut raw.agents.runtime.codex,
        Agent::Claude => &mut raw.agents.runtime.claude,
        Agent::Gemini => &mut raw.agents.runtime.gemini,
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
        agents: default_agents_config(),
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

fn default_agents_config() -> AgentsConfig {
    AgentsConfig {
        runtime: RuntimeAgentsConfig {
            codex: AgentRuntimeConfig {
                enabled: true,
                transport: Some("acp".to_string()),
                model: Some("gpt-5.3-codex".to_string()),
                models: vec![
                    runtime_option_config("gpt-5.5", "5.5"),
                    runtime_option_config("gpt-5.4", "5.4"),
                    runtime_option_config("gpt-5.3-codex", "5.3 Codex"),
                ],
                permission_mode: Some("read-only".to_string()),
                permission_modes: vec![
                    runtime_option_config("read-only", "Default permissions"),
                    runtime_option_config("auto", "Auto-review"),
                    runtime_option_config("full-access", "Full access"),
                ],
                sandbox: None,
                command: AgentRuntimeCommandConfig {
                    session: Some("npx -y @zed-industries/codex-acp@latest".to_string()),
                    version: Some("codex --version".to_string()),
                },
            },
            claude: AgentRuntimeConfig {
                enabled: true,
                transport: Some("acp".to_string()),
                model: Some("claude-opus-4-7".to_string()),
                models: vec![
                    runtime_option_config("claude-opus-4-7", "Opus 4.7"),
                    runtime_option_config("claude-opus-4-6", "Opus 4.6"),
                ],
                permission_mode: Some("default".to_string()),
                permission_modes: vec![
                    runtime_option_config("default", "Ask before edits"),
                    runtime_option_config("acceptEdits", "Edit automatically"),
                    runtime_option_config("plan", "Plan mode"),
                    runtime_option_config("dontAsk", "Don't Ask"),
                ],
                sandbox: None,
                command: AgentRuntimeCommandConfig {
                    session: Some(
                        "npx -y @agentclientprotocol/claude-agent-acp@latest".to_string(),
                    ),
                    version: Some("claude --version".to_string()),
                },
            },
            gemini: AgentRuntimeConfig {
                enabled: false,
                transport: Some("acp".to_string()),
                model: None,
                models: Vec::new(),
                permission_mode: None,
                permission_modes: Vec::new(),
                sandbox: None,
                command: AgentRuntimeCommandConfig {
                    session: Some(
                        "npx -y -- @google/gemini-cli@latest --experimental-acp".to_string(),
                    ),
                    version: Some("gemini --version".to_string()),
                },
            },
        },
    }
}

fn runtime_option_config(value: &str, label: &str) -> AgentRuntimeOptionConfig {
    AgentRuntimeOptionConfig {
        value: value.to_string(),
        label: label.to_string(),
    }
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
    out.push_str(&serialize_agents_config(&config.agents));
    out
}

fn serialize_agents_config(config: &AgentsConfig) -> String {
    let mut out = String::new();
    for (name, runtime) in [
        ("codex", &config.runtime.codex),
        ("claude", &config.runtime.claude),
        ("gemini", &config.runtime.gemini),
    ] {
        if !runtime.enabled
            && runtime.transport.is_none()
            && runtime.model.is_none()
            && runtime.models.is_empty()
            && runtime.permission_mode.is_none()
            && runtime.permission_modes.is_empty()
            && runtime.sandbox.is_none()
            && runtime.command.session.is_none()
            && runtime.command.version.is_none()
        {
            continue;
        }
        out.push_str("[agents.runtime.");
        out.push_str(name);
        out.push_str("]\n");
        out.push_str("enabled = ");
        out.push_str(if runtime.enabled { "true" } else { "false" });
        out.push('\n');
        if let Some(transport) = &runtime.transport {
            out.push_str("transport = ");
            out.push_str(&toml_string(transport));
            out.push('\n');
        }
        if let Some(model) = &runtime.model {
            out.push_str("model = ");
            out.push_str(&toml_string(model));
            out.push('\n');
        }
        if !runtime.models.is_empty() {
            out.push_str("models = ");
            out.push_str(&toml_string(&serialize_runtime_options(&runtime.models)));
            out.push('\n');
        }
        if let Some(permission_mode) = &runtime.permission_mode {
            out.push_str("permission_mode = ");
            out.push_str(&toml_string(permission_mode));
            out.push('\n');
        }
        if !runtime.permission_modes.is_empty() {
            out.push_str("permission_modes = ");
            out.push_str(&toml_string(&serialize_runtime_options(
                &runtime.permission_modes,
            )));
            out.push('\n');
        }
        if let Some(sandbox) = &runtime.sandbox {
            out.push_str("sandbox = ");
            out.push_str(&toml_string(sandbox));
            out.push('\n');
        }
        if runtime.command.session.is_some() || runtime.command.version.is_some() {
            out.push_str("[agents.runtime.");
            out.push_str(name);
            out.push_str(".command]\n");
            if let Some(command) = &runtime.command.session {
                out.push_str("session = ");
                out.push_str(&toml_string(command));
                out.push('\n');
            }
            if let Some(command) = &runtime.command.version {
                out.push_str("version = ");
                out.push_str(&toml_string(command));
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        default_app_config, parse_raw_config, raw_config_with_defaults, resolve_agents_config,
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
    fn parses_agent_runtime_config() {
        let raw = parse_raw_config(
            r#"
            [agents.runtime.codex]
            enabled = true
            transport = "acp"
            model = "gpt-5"
            models = "gpt-5|GPT 5,gpt-5-codex|GPT 5 Codex"
            permission_mode = "read-only"
            permission_modes = "read-only|Default permissions,auto|Auto-review,full-access|Full access"
            sandbox = "workspace-write"
            [agents.runtime.codex.command]
            session = "npx -y @zed-industries/codex-acp@latest"
            version = "codex --version"
            "#,
        )
        .unwrap();
        let config = resolve_agents_config(raw).unwrap();

        assert!(config.runtime.codex.enabled);
        assert_eq!(config.runtime.codex.transport.as_deref(), Some("acp"));
        assert_eq!(config.runtime.codex.model.as_deref(), Some("gpt-5"));
        assert_eq!(config.runtime.codex.models.len(), 2);
        assert_eq!(config.runtime.codex.models[0].value, "gpt-5");
        assert_eq!(config.runtime.codex.models[0].label, "GPT 5");
        assert_eq!(
            config.runtime.codex.permission_mode.as_deref(),
            Some("read-only")
        );
        assert_eq!(config.runtime.codex.permission_modes.len(), 3);
        assert_eq!(config.runtime.codex.permission_modes[0].value, "read-only");
        assert_eq!(
            config.runtime.codex.permission_modes[0].label,
            "Default permissions"
        );
        assert_eq!(
            config.runtime.codex.sandbox.as_deref(),
            Some("workspace-write")
        );
        assert_eq!(
            config.runtime.codex.command.session.as_deref(),
            Some("npx -y @zed-industries/codex-acp@latest")
        );
        assert_eq!(
            config.runtime.codex.command.version.as_deref(),
            Some("codex --version")
        );
    }

    #[test]
    fn default_app_config_serializes_runtime_agents() {
        let config = default_app_config().unwrap();
        let serialized = serialize_app_config(&config);
        let raw = parse_raw_config(&serialized).unwrap();
        let agents = resolve_agents_config(raw).unwrap();

        assert!(serialized.contains("[agents.runtime.codex]"));
        assert!(serialized.contains("[agents.runtime.claude]"));
        assert!(serialized.contains("[agents.runtime.gemini]"));
        assert!(agents.runtime.codex.enabled);
        assert!(agents.runtime.claude.enabled);
        assert!(!agents.runtime.gemini.enabled);
        assert_eq!(agents.runtime.codex.transport.as_deref(), Some("acp"));
        assert_eq!(
            agents.runtime.claude.permission_mode.as_deref(),
            Some("default")
        );
        assert_eq!(
            agents.runtime.gemini.command.session.as_deref(),
            Some("npx -y -- @google/gemini-cli@latest --experimental-acp")
        );
    }

    #[test]
    fn empty_config_is_completed_with_default_runtime_agents() {
        let raw = parse_raw_config("").unwrap();
        let (raw, changed) = raw_config_with_defaults(raw).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(changed);
        assert!(config.agents.runtime.codex.enabled);
        assert!(config.agents.runtime.claude.enabled);
        assert!(!config.agents.runtime.gemini.enabled);
        assert_eq!(
            config.agents.runtime.codex.transport.as_deref(),
            Some("acp")
        );
        assert_eq!(
            config.agents.runtime.codex.command.session.as_deref(),
            Some("npx -y @zed-industries/codex-acp@latest")
        );
        assert_eq!(
            config.agents.runtime.claude.command.session.as_deref(),
            Some("npx -y @agentclientprotocol/claude-agent-acp@latest")
        );
        assert_eq!(config.agents.runtime.claude.permission_modes.len(), 4);
    }

    #[test]
    fn existing_config_is_completed_without_overwriting_user_values() {
        let raw = parse_raw_config(
            r#"
            [agents.runtime.codex]
            enabled = false
            model = "custom-codex"
            models = "custom-codex|Custom Codex"

            [agents.runtime.claude]
            enabled = true
            permission_modes = "default|Ask before edits"
            [agents.runtime.claude.command]
            session = "custom-claude-acp"
            "#,
        )
        .unwrap();
        let (raw, changed) = raw_config_with_defaults(raw).unwrap();
        let config = super::resolve_app_config(raw, false).unwrap();

        assert!(changed);
        assert!(!config.agents.runtime.codex.enabled);
        assert_eq!(
            config.agents.runtime.codex.model.as_deref(),
            Some("custom-codex")
        );
        assert_eq!(config.agents.runtime.codex.models[0].value, "custom-codex");
        assert!(config
            .agents
            .runtime
            .codex
            .models
            .iter()
            .any(|option| option.value == "gpt-5.3-codex"));
        assert_eq!(
            config.agents.runtime.claude.command.session.as_deref(),
            Some("custom-claude-acp")
        );
        assert!(config
            .agents
            .runtime
            .claude
            .permission_modes
            .iter()
            .any(|option| option.value == "dontAsk"));
        assert_eq!(
            config.agents.runtime.gemini.command.version.as_deref(),
            Some("gemini --version")
        );
    }
}
