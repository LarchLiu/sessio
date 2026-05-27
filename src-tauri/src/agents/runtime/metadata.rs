use std::process::Command;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use super::acp_transport;
use super::types::{RuntimeCapabilitySet, RuntimeTransportKind};
use crate::config::{self, AgentRuntimeConfig};
use crate::models::{Agent, RuntimeAgentMetadata, RuntimeAgentOptionMetadata};
use crate::store::{RuntimeAgentCapabilityRecord, SessionStore};

#[derive(Default)]
pub struct RuntimeAgentsCache {
    inner: RwLock<Vec<RuntimeAgentMetadata>>,
}

impl RuntimeAgentsCache {
    pub fn get(&self) -> Vec<RuntimeAgentMetadata> {
        self.inner
            .read()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    pub fn set(&self, items: Vec<RuntimeAgentMetadata>) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = items;
        }
    }
}

pub fn configured_runtime_agents() -> Result<Vec<RuntimeAgentMetadata>> {
    let config = config::load_config()?;
    let mut agents = Vec::new();
    for agent in [Agent::Codex, Agent::Claude, Agent::Gemini] {
        let runtime = config.agents.runtime.get(agent);
        if !runtime.enabled {
            continue;
        }
        agents.push(RuntimeAgentMetadata {
            agent,
            enabled: runtime.enabled,
            configured: runtime.enabled,
            transport: transport_from_runtime_config(runtime),
            model: runtime.model.clone(),
            models: runtime_options_metadata(&runtime.models, runtime.model.as_deref()),
            permission_mode: runtime_permission_mode(agent, runtime.permission_mode.as_deref()),
            permission_modes: runtime_permission_options_metadata(
                agent,
                &runtime.permission_modes,
                runtime.permission_mode.as_deref(),
            ),
            session_command: Some(acp_transport::command_from_config(agent, runtime)),
            version_command: runtime.command.version.clone(),
            detected_version: None,
            capabilities: None,
            updated_at: None,
        });
    }
    Ok(agents)
}

pub fn runtime_agents_with_detected_capabilities(
    store: Arc<dyn SessionStore>,
) -> Result<Vec<RuntimeAgentMetadata>> {
    let mut agents = configured_runtime_agents()?;
    for metadata in agents.iter_mut() {
        if let Some(record) = store.get_runtime_agent_capability(metadata.agent)? {
            metadata.detected_version = record.version;
            metadata.capabilities = derive_runtime_capabilities(&record.raw_capabilities_json).ok();
            metadata.updated_at = Some(record.updated_at);
        }
    }
    Ok(agents)
}

pub fn startup_probe_runtime_agents(
    store: Arc<dyn SessionStore>,
) -> Result<Vec<RuntimeAgentMetadata>> {
    let config = config::load_config()?;
    let mut out = Vec::new();

    for agent in [Agent::Codex, Agent::Claude, Agent::Gemini] {
        let runtime_config = config.agents.runtime.get(agent).clone();
        if !runtime_config.enabled {
            continue;
        }
        let transport = transport_from_runtime_config(&runtime_config);
        let session_command = acp_transport::command_from_config(agent, &runtime_config);
        let detected_version = runtime_config
            .command
            .version
            .as_deref()
            .and_then(run_version_command);
        let cached = store.get_runtime_agent_capability(agent)?;

        let should_probe = cached
            .as_ref()
            .map(|record| record.version != detected_version)
            .unwrap_or(true);

        let capability_record = if should_probe {
            let workspace_path = ensure_probe_workspace(agent)?;
            match detect_capabilities_with_initialize_only(
                agent,
                &workspace_path,
                transport,
                session_command.clone(),
            ) {
                Ok(probe) => {
                    let record = RuntimeAgentCapabilityRecord {
                        agent,
                        transport,
                        version: detected_version.clone(),
                        protocol_version: Some(probe.protocol_version),
                        raw_initialize_response_json: probe.raw_initialize_response_json,
                        raw_capabilities_json: probe.raw_capabilities_json,
                        updated_at: now_ms(),
                    };
                    store.upsert_runtime_agent_capability(&record)?;
                    Some(record)
                }
                Err(error) => {
                    log::warn!(
                        "[sessio-runtime:metadata:probe-failed] agent={} error={error}",
                        agent.as_str()
                    );
                    cached
                }
            }
        } else {
            cached
        };

        out.push(RuntimeAgentMetadata {
            agent,
            enabled: runtime_config.enabled,
            configured: runtime_config.enabled,
            transport,
            model: runtime_config.model.clone(),
            models: runtime_options_metadata(
                &runtime_config.models,
                runtime_config.model.as_deref(),
            ),
            permission_mode: runtime_permission_mode(
                agent,
                runtime_config.permission_mode.as_deref(),
            ),
            permission_modes: runtime_permission_options_metadata(
                agent,
                &runtime_config.permission_modes,
                runtime_config.permission_mode.as_deref(),
            ),
            session_command: Some(session_command),
            version_command: runtime_config.command.version.clone(),
            detected_version: capability_record
                .as_ref()
                .and_then(|record| record.version.clone())
                .or(detected_version),
            capabilities: capability_record
                .as_ref()
                .and_then(|record| derive_runtime_capabilities(&record.raw_capabilities_json).ok()),
            updated_at: capability_record.as_ref().map(|record| record.updated_at),
        });
    }

    Ok(out)
}

fn runtime_options_metadata(
    options: &[config::AgentRuntimeOptionConfig],
    selected: Option<&str>,
) -> Vec<RuntimeAgentOptionMetadata> {
    let mut out: Vec<RuntimeAgentOptionMetadata> = options
        .iter()
        .filter(|option| !option.value.trim().is_empty())
        .map(|option| RuntimeAgentOptionMetadata {
            value: option.value.clone(),
            label: if option.label.trim().is_empty() {
                option.value.clone()
            } else {
                option.label.clone()
            },
        })
        .collect();
    if let Some(selected) = selected.filter(|value| !value.trim().is_empty()) {
        if !out.iter().any(|option| option.value == selected) {
            out.insert(
                0,
                RuntimeAgentOptionMetadata {
                    value: selected.to_string(),
                    label: selected.to_string(),
                },
            );
        }
    }
    out
}

fn runtime_permission_options_metadata(
    agent: Agent,
    options: &[config::AgentRuntimeOptionConfig],
    selected: Option<&str>,
) -> Vec<RuntimeAgentOptionMetadata> {
    let fallback;
    let source = if options.is_empty() {
        fallback = default_permission_options(agent);
        &fallback
    } else {
        options
    };
    if agent == Agent::Claude {
        return claude_permission_options_metadata(source, selected);
    }
    runtime_options_metadata(source, selected)
}

fn default_permission_options(agent: Agent) -> Vec<config::AgentRuntimeOptionConfig> {
    match agent {
        Agent::Codex => vec![
            runtime_option("read-only", "Default permissions"),
            runtime_option("auto", "Auto-review"),
            runtime_option("full-access", "Full access"),
        ],
        Agent::Claude => vec![
            runtime_option("default", "Ask before edits"),
            runtime_option("acceptEdits", "Edit automatically"),
            runtime_option("plan", "Plan mode"),
            runtime_option("dontAsk", "Don't Ask"),
        ],
        Agent::Gemini => Vec::new(),
    }
}

fn runtime_permission_mode(agent: Agent, selected: Option<&str>) -> Option<String> {
    match agent {
        Agent::Claude => selected
            .and_then(normalize_claude_permission_mode)
            .or(Some("default"))
            .map(ToString::to_string),
        _ => selected
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    }
}

fn claude_permission_options_metadata(
    options: &[config::AgentRuntimeOptionConfig],
    selected: Option<&str>,
) -> Vec<RuntimeAgentOptionMetadata> {
    let mut out = default_permission_options(Agent::Claude)
        .into_iter()
        .map(|option| {
            let label = options
                .iter()
                .find(|candidate| {
                    normalize_claude_permission_mode(&candidate.value)
                        == Some(option.value.as_str())
                })
                .and_then(|candidate| {
                    let label = candidate.label.trim();
                    (!label.is_empty()).then(|| candidate.label.clone())
                })
                .unwrap_or_else(|| option.label.clone());
            RuntimeAgentOptionMetadata {
                value: option.value,
                label,
            }
        })
        .collect::<Vec<_>>();

    let selected = selected
        .and_then(normalize_claude_permission_mode)
        .unwrap_or("default");
    if !out.iter().any(|option| option.value == selected) {
        out.insert(
            0,
            RuntimeAgentOptionMetadata {
                value: selected.to_string(),
                label: selected.to_string(),
            },
        );
    }
    out
}

fn normalize_claude_permission_mode(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "default" => Some("default"),
        "acceptedits" => Some("acceptEdits"),
        "plan" => Some("plan"),
        "dontask" => Some("dontAsk"),
        _ => None,
    }
}

fn runtime_option(value: &str, label: &str) -> config::AgentRuntimeOptionConfig {
    config::AgentRuntimeOptionConfig {
        value: value.to_string(),
        label: label.to_string(),
    }
}

fn detect_capabilities_with_initialize_only(
    agent: Agent,
    workspace_path: &str,
    transport: RuntimeTransportKind,
    command: String,
) -> Result<acp_transport::AcpInitializeProbe> {
    if transport != RuntimeTransportKind::Acp {
        return Err(anyhow::anyhow!(
            "initialize-only probe currently only supports ACP transport for {} at {}",
            agent.as_str(),
            workspace_path
        ));
    }
    acp_transport::probe_initialize_response(command)
}

fn run_version_command(command: &str) -> Option<String> {
    run_shell_command(command).ok().and_then(|output| {
        let stdout = output.trim().to_string();
        (!stdout.is_empty()).then_some(stdout)
    })
}

fn run_shell_command(command: &str) -> Result<String> {
    let output = Command::new("/bin/sh")
        .arg("-lc")
        .arg(command)
        .output()
        .with_context(|| format!("run command: {command}"))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "command failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn transport_from_runtime_config(config: &AgentRuntimeConfig) -> RuntimeTransportKind {
    match config.transport.as_deref() {
        Some("cliStreamJson") => RuntimeTransportKind::CliStreamJson,
        Some("plainCli") => RuntimeTransportKind::PlainCli,
        Some("fake") => RuntimeTransportKind::Fake,
        _ => RuntimeTransportKind::Acp,
    }
}

fn ensure_probe_workspace(agent: Agent) -> Result<String> {
    let home = dirs::home_dir().context("no home dir")?;
    let dir = home
        .join(".sessio")
        .join("projects")
        .join(format!(".{}", agent.as_str()))
        .join("tmp-agent-capabilities");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create probe workspace {}", dir.display()))?;
    Ok(dir.to_string_lossy().to_string())
}

fn derive_runtime_capabilities(raw_capabilities_json: &str) -> Result<RuntimeCapabilitySet> {
    let capabilities: agent_client_protocol::schema::AgentCapabilities =
        serde_json::from_str(raw_capabilities_json).context("parse ACP capabilities json")?;
    Ok(acp_transport::runtime_capabilities_from_acp(&capabilities))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::runtime::types::RuntimeTransportKind;
    use crate::models::Agent;
    use crate::store::sqlite::SqliteStore;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_runtime_metadata_db() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sessio-runtime-metadata-{nanos}.db"))
    }

    #[test]
    fn manual_runtime_agent_capability_db_roundtrip_on_local_db() {
        let db_path = unique_runtime_metadata_db();
        let store = Arc::new(SqliteStore::open(&db_path).expect("open sqlite"));
        store.init().expect("init sqlite");

        let record = RuntimeAgentCapabilityRecord {
            agent: Agent::Codex,
            transport: RuntimeTransportKind::Acp,
            version: Some("test-version".to_string()),
            protocol_version: Some("1".to_string()),
            raw_initialize_response_json: "{\"protocolVersion\":1,\"agentCapabilities\":{}}"
                .to_string(),
            raw_capabilities_json: "{\"loadSession\":true,\"promptCapabilities\":{}}".to_string(),
            updated_at: now_ms(),
        };

        store
            .upsert_runtime_agent_capability(&record)
            .expect("upsert runtime capability");

        let loaded = store
            .get_runtime_agent_capability(Agent::Codex)
            .expect("read runtime capability")
            .expect("runtime capability row");

        assert_eq!(loaded.agent, Agent::Codex);
        assert_eq!(loaded.transport, RuntimeTransportKind::Acp);
        assert_eq!(loaded.version.as_deref(), Some("test-version"));
        assert_eq!(loaded.protocol_version.as_deref(), Some("1"));
        assert_eq!(
            loaded.raw_initialize_response_json,
            "{\"protocolVersion\":1,\"agentCapabilities\":{}}"
        );
        assert_eq!(
            loaded.raw_capabilities_json,
            "{\"loadSession\":true,\"promptCapabilities\":{}}"
        );

        let _ = std::fs::remove_file(db_path);
    }

    #[test]
    fn claude_permission_options_keep_common_modes_only() {
        let options = vec![
            runtime_option("auto", "Auto"),
            runtime_option("bypassPermissions", "Bypass"),
            runtime_option("acceptEdits", "Edit automatically"),
        ];

        let modes = runtime_permission_options_metadata(Agent::Claude, &options, Some("auto"));
        let values: Vec<_> = modes.iter().map(|mode| mode.value.as_str()).collect();

        assert_eq!(values, vec!["default", "acceptEdits", "plan", "dontAsk"]);
        assert_eq!(modes[1].label, "Edit automatically");
        assert_eq!(
            runtime_permission_mode(Agent::Claude, Some("bypassPermissions")).as_deref(),
            Some("default")
        );
    }
}
