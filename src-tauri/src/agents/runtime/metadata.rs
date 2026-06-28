use std::process::Command;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use super::types::{RuntimeCapabilitySet, RuntimeTransportKind};
use super::{acp_transport, pi_rpc_transport};
use crate::app_paths;
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
    let capabilities = cached.and_then(|metadata| metadata.capabilities.clone());
    let computer_use_eligible = derive_computer_use_eligible(runtime_agent, capabilities.as_ref());
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
        capabilities,
        computer_use_eligible,
        updated_at: Some(agent.updated_at),
    })
}

/// Whether an agent is eligible for the session-scoped `computer use` feature.
///
/// Two layers, per the implementation plan:
/// 1. **Transport injectability** — the runtime can accept a Sessio-provided
///    tool server (`mcp_injection.is_injectable()`): an ACP MCP server
///    (`http`/`sse`/`acp`) or a native extension path (Pi).
/// 2. **Product support** — Sessio actually supports the computer-use contract
///    for this agent. Phase 0's per-agent spike narrows this set; until an agent
///    is confirmed it stays out of [`COMPUTER_USE_SUPPORTED_AGENTS`].
///
/// Both must hold. Capability data may be absent (agent not yet probed), in
/// which case the agent is not eligible until a probe populates it.
pub fn derive_computer_use_eligible(
    agent: Agent,
    capabilities: Option<&RuntimeCapabilitySet>,
) -> bool {
    let injectable = capabilities
        .map(|caps| caps.mcp_injection.is_injectable())
        .unwrap_or(false);
    injectable && computer_use_product_supported(agent)
}

/// Product-level allowlist of agents Sessio supports the computer-use contract
/// for. As of the v3 implementation plan, the supported MVP set is the ACP
/// agents verified to accept desktop-owned HTTP MCP injection: Codex and Claude.
/// Pi remains a separate extension path and OpenCode has not yet been verified
/// against the computer-use contract end-to-end.
fn computer_use_product_supported(agent: Agent) -> bool {
    matches!(agent, Agent::Codex | Agent::Claude)
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
        let configured_session_command = agent
            .commands
            .session
            .first()
            .cloned()
            .unwrap_or_else(|| acp_transport::default_acp_command(runtime_agent));
        let probe_command = startup_probe_command(runtime_agent, &agent);
        let version_command = agent.commands.version.first().cloned();
        let cached = store.get_runtime_agent_capability(runtime_agent)?;
        let detected_adapter_version = version_command
            .as_deref()
            .and_then(run_version_command)
            .or_else(|| cached.as_ref().and_then(|record| record.version.clone()));
        let should_probe = cached.is_none()
            || cached
                .as_ref()
                .map(|record| record.version != detected_adapter_version)
                .unwrap_or(false);
        let capability_record = if should_probe {
            let workspace_path = ensure_probe_workspace(runtime_agent)?;
            match detect_capabilities_with_initialize_only(
                runtime_agent,
                &workspace_path,
                agent.transport,
                probe_command,
            ) {
                Ok(probe) => {
                    let record = RuntimeAgentCapabilityRecord {
                        agent: runtime_agent,
                        transport: agent.transport,
                        version: detected_adapter_version.clone(),
                        protocol_version: Some(probe.protocol_version),
                        raw_initialize_response_json: probe.raw_initialize_response_json,
                        raw_capabilities_json: probe.raw_capabilities_json,
                        updated_at: now_ms(),
                    };
                    store.upsert_runtime_agent_capability(&record)?;
                    if let Some(adapter_version) = record.version.as_deref() {
                        store.mark_runtime_agent_session_config_needs_refresh(
                            runtime_agent,
                            adapter_version,
                        )?;
                    }
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

        let capabilities = capability_record
            .as_ref()
            .and_then(|record| derive_runtime_capabilities(&record.raw_capabilities_json).ok());
        let computer_use_eligible =
            derive_computer_use_eligible(runtime_agent, capabilities.as_ref());
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
            session_command: Some(configured_session_command),
            version_command,
            detected_version: capability_record
                .as_ref()
                .and_then(|record| record.version.clone()),
            capabilities,
            computer_use_eligible,
            updated_at: capability_record.as_ref().map(|record| record.updated_at),
        });
    }

    Ok(out)
}

fn startup_probe_command(runtime_agent: Agent, agent: &AgentInfo) -> String {
    match runtime_agent {
        _ => agent
            .commands
            .session
            .first()
            .cloned()
            .unwrap_or_else(|| acp_transport::default_acp_command(runtime_agent)),
    }
}

fn detect_capabilities_with_initialize_only(
    agent: Agent,
    workspace_path: &str,
    transport: RuntimeTransportKind,
    command: String,
) -> Result<acp_transport::AcpInitializeProbe> {
    if transport == RuntimeTransportKind::PiRpc {
        let capabilities = pi_rpc_transport::runtime_capabilities();
        return Ok(acp_transport::AcpInitializeProbe {
            protocol_version: "pi-rpc".to_string(),
            raw_initialize_response_json: serde_json::json!({
                "protocolVersion": "pi-rpc",
                "command": command,
                "workspacePath": workspace_path,
                "agent": agent.as_str(),
            })
            .to_string(),
            raw_capabilities_json: serde_json::to_string(&capabilities)?,
        });
    }
    if transport == RuntimeTransportKind::Acp {
        return acp_transport::probe_initialize_response(command, workspace_path.to_string());
    }
    Err(anyhow::anyhow!(
        "initialize-only probe does not support {:?} transport for {} at {}",
        transport,
        agent.as_str(),
        workspace_path
    ))
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
    let dir = app_paths::agent_probe_workspace_dir(agent.as_str())?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create probe workspace {}", dir.display()))?;
    Ok(dir.to_string_lossy().to_string())
}

fn derive_runtime_capabilities(raw_capabilities_json: &str) -> Result<RuntimeCapabilitySet> {
    if let Ok(capabilities) = serde_json::from_str::<RuntimeCapabilitySet>(raw_capabilities_json) {
        return Ok(capabilities);
    }
    let capabilities: agent_client_protocol::schema::v1::AgentCapabilities =
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

    #[test]
    fn initialize_only_probe_requires_workspace_and_uses_acp_transport() {
        let result = detect_capabilities_with_initialize_only(
            Agent::Codex,
            "/tmp/sessio-probe-workspace",
            RuntimeTransportKind::Fake,
            "npx -y @agentclientprotocol/codex-acp@latest".to_string(),
        );

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("initialize-only probe does not support Fake transport"));
    }

    #[test]
    fn initialize_only_probe_supports_pi_rpc_transport_locally() {
        let probe = detect_capabilities_with_initialize_only(
            Agent::Pi,
            "/tmp/sessio-probe-workspace",
            RuntimeTransportKind::PiRpc,
            "pi --mode rpc".to_string(),
        )
        .expect("pi rpc probe");

        assert_eq!(probe.protocol_version, "pi-rpc");
        let capabilities: RuntimeCapabilitySet =
            serde_json::from_str(&probe.raw_capabilities_json).expect("capabilities json");
        assert!(capabilities.supports_cancel);
        assert!(capabilities.supports_image_attachments);
    }

    #[test]
    fn pi_rpc_capabilities_advertise_native_extension_injection() {
        let capabilities = pi_rpc_transport::runtime_capabilities();
        assert!(capabilities.mcp_injection.native_extension);
        assert!(capabilities.mcp_injection.is_injectable());
    }

    #[test]
    fn computer_use_eligibility_requires_injectable_and_product_support() {
        // No capabilities probed yet → not eligible.
        assert!(!derive_computer_use_eligible(Agent::Pi, None));

        // Injectable transport but no product support yet → still not eligible.
        let injectable = pi_rpc_transport::runtime_capabilities();
        assert!(injectable.mcp_injection.is_injectable());
        assert!(!derive_computer_use_eligible(Agent::Pi, Some(&injectable)));

        // A non-injectable capability set is never eligible regardless of product
        // support.
        let mut not_injectable = RuntimeCapabilitySet::fake();
        not_injectable.mcp_injection = Default::default();
        assert!(!not_injectable.mcp_injection.is_injectable());
        assert!(!derive_computer_use_eligible(
            Agent::Pi,
            Some(&not_injectable)
        ));

        let mut acp_http = RuntimeCapabilitySet::fake();
        acp_http.mcp_injection.http = true;
        assert!(derive_computer_use_eligible(Agent::Codex, Some(&acp_http)));
        assert!(derive_computer_use_eligible(Agent::Claude, Some(&acp_http)));
        assert!(!derive_computer_use_eligible(
            Agent::Opencode,
            Some(&acp_http)
        ));
    }

    #[test]
    fn stored_capabilities_without_mcp_injection_field_deserialize() {
        // Capability JSON persisted before the field existed must still parse,
        // defaulting mcp_injection to all-false (not injectable).
        let legacy = r#"{
            "supportsCancel": true,
            "supportsPermissions": true,
            "supportsToolDeltas": true,
            "supportsLoadSession": true,
            "supportsResume": false,
            "supportsFork": false,
            "supportsImageAttachments": false,
            "supportsAudioAttachments": false,
            "supportsEmbeddedContext": false,
            "supportsAttachments": false,
            "supportsModes": false
        }"#;
        let caps = derive_runtime_capabilities(legacy).expect("legacy caps parse");
        assert!(!caps.mcp_injection.is_injectable());
    }
}
