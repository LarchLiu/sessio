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
    pub transport: Option<String>,
    pub command: Option<String>,
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
    transport: Option<String>,
    command: Option<String>,
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
    Ok(AppConfig {
        memory: resolve_memory_config(raw.clone())?,
        agents: resolve_agents_config(raw)?,
    })
}

pub fn load_memory_config() -> Result<MemoryConfig> {
    let raw = load_raw_config()?;
    resolve_memory_config(raw)
}

pub fn save_memory_config(config: &MemoryConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create config dir {}", parent.display()))?;
    }
    fs::write(&path, serialize_memory_config(config))
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
        return Ok(RawConfig::default());
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
                    "transport" => target.transport = value,
                    "command" => target.command = value,
                    other => bail!(
                        "unknown key in [agents.runtime.{}]: {other}",
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

fn resolve_memory_config(raw: RawConfig) -> Result<MemoryConfig> {
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

    Ok(MemoryConfig {
        backend,
        qmd: config,
    })
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
        transport,
        command: raw.command.filter(|value| !value.trim().is_empty()),
    })
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
    let config = default_memory_config()?;
    fs::write(path, serialize_memory_config(&config))
        .with_context(|| format!("write config {}", path.display()))
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
        if runtime.transport.is_none() && runtime.command.is_none() {
            continue;
        }
        out.push_str("[agents.runtime.");
        out.push_str(name);
        out.push_str("]\n");
        if let Some(transport) = &runtime.transport {
            out.push_str("transport = ");
            out.push_str(&toml_string(transport));
            out.push('\n');
        }
        if let Some(command) = &runtime.command {
            out.push_str("command = ");
            out.push_str(&toml_string(command));
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_raw_config, resolve_agents_config, resolve_memory_config};

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
            transport = "acp"
            command = "npx -y @zed-industries/codex-acp@latest"
            "#,
        )
        .unwrap();
        let config = resolve_agents_config(raw).unwrap();

        assert_eq!(config.runtime.codex.transport.as_deref(), Some("acp"));
        assert_eq!(
            config.runtime.codex.command.as_deref(),
            Some("npx -y @zed-industries/codex-acp@latest")
        );
    }
}
