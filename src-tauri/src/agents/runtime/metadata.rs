use std::process::Command;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use super::acp_transport;
use super::types::{RuntimeCapabilitySet, RuntimeTransportKind};
use crate::config::{self, AgentRuntimeConfig};
use crate::models::{Agent, RuntimeAgentMetadata};
use crate::store::{RuntimeAgentCapabilityRecord, SessionStore};

#[derive(Default)]
pub struct RuntimeAgentsCache {
    inner: RwLock<Vec<RuntimeAgentMetadata>>,
}

impl RuntimeAgentsCache {
    pub fn get(&self) -> Vec<RuntimeAgentMetadata> {
        self.inner.read().map(|items| items.clone()).unwrap_or_default()
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
            session_command: runtime.command.session.clone(),
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
                runtime_config
                    .command
                    .session
                    .clone()
                    .context("missing runtime session command")?,
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
            session_command: runtime_config.command.session.clone(),
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

    #[test]
    fn manual_runtime_agent_capability_db_roundtrip_on_local_db() {
        let home = dirs::home_dir().expect("home dir");
        let db_path = home.join(".sessio").join("db-data").join("sessio-index.db");
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
    }
}
