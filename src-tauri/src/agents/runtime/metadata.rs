use std::process::Command;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use super::acp_transport;
use super::types::{RuntimeCapabilitySet, RuntimeTransportKind};
use crate::models::{Agent, AgentInfo, AgentType, RuntimeAgentMetadata};
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

pub fn runtime_metadata_from_agent_info(
    agent: AgentInfo,
    cached: Option<&RuntimeAgentMetadata>,
) -> Option<RuntimeAgentMetadata> {
    let runtime_agent = Agent::from_db_str(&agent.id)?;
    Some(RuntimeAgentMetadata {
        agent: runtime_agent,
        enabled: agent.enabled,
        configured: agent.enabled,
        order: agent.order,
        transport: agent.transport,
        model: agent.model,
        models: agent.models,
        effort: agent.effort,
        efforts: agent.efforts,
        permission_mode: agent.permission_mode,
        permission_modes: agent.permission_modes,
        session_command: agent.commands.session.first().cloned(),
        version_command: agent.commands.version.first().cloned(),
        detected_version: cached.and_then(|metadata| metadata.detected_version.clone()),
        capabilities: cached.and_then(|metadata| metadata.capabilities.clone()),
        updated_at: Some(agent.updated_at),
    })
}

pub fn runtime_agents_from_db(
    store: Arc<dyn SessionStore>,
    cache: &[RuntimeAgentMetadata],
) -> Result<Vec<RuntimeAgentMetadata>> {
    let agents: Vec<RuntimeAgentMetadata> = store
        .list_agents()?
        .into_iter()
        .filter(|agent| agent.agent_type == AgentType::Builtin)
        .filter_map(|agent| {
            let runtime_agent = Agent::from_db_str(&agent.id)?;
            let cached = cache
                .iter()
                .find(|metadata| metadata.agent == runtime_agent);
            runtime_metadata_from_agent_info(agent, cached)
        })
        .collect();
    Ok(agents)
}

pub fn startup_probe_runtime_agents(
    store: Arc<dyn SessionStore>,
) -> Result<Vec<RuntimeAgentMetadata>> {
    let mut out = Vec::new();
    for agent in store
        .list_agents()?
        .into_iter()
        .filter(|agent| agent.agent_type == AgentType::Builtin && agent.enabled)
    {
        let Some(runtime_agent) = Agent::from_db_str(&agent.id) else {
            continue;
        };
        let session_command = agent
            .commands
            .session
            .first()
            .cloned()
            .unwrap_or_else(|| acp_transport::default_acp_command(runtime_agent));
        let version_command = agent.commands.version.first().cloned();
        let cached = store.get_runtime_agent_capability(runtime_agent)?;

        let should_probe = cached.is_none();
        let capability_record = if should_probe {
            let workspace_path = ensure_probe_workspace(runtime_agent)?;
            match detect_capabilities_with_initialize_only(
                runtime_agent,
                &workspace_path,
                agent.transport,
                session_command.clone(),
            ) {
                Ok(probe) => {
                    let record = RuntimeAgentCapabilityRecord {
                        agent: runtime_agent,
                        transport: agent.transport,
                        version: None,
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
                        runtime_agent.as_str()
                    );
                    cached
                }
            }
        } else {
            cached
        };

        out.push(RuntimeAgentMetadata {
            agent: runtime_agent,
            enabled: agent.enabled,
            configured: agent.enabled,
            order: agent.order,
            transport: agent.transport,
            model: agent.model,
            models: agent.models,
            effort: agent.effort,
            efforts: agent.efforts,
            permission_mode: agent.permission_mode,
            permission_modes: agent.permission_modes,
            session_command: Some(session_command),
            version_command,
            detected_version: capability_record
                .as_ref()
                .and_then(|record| record.version.clone()),
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

#[allow(dead_code)]
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
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
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

        let _ = std::fs::remove_file(db_path);
    }
}
