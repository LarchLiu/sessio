//! Local discovery and attach resources for external `sessio cu` clients.
//!
//! The broker is still hosted by the Sessio desktop app. External agents find
//! it through a private discovery file, then ask the app for a scoped MCP token.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

const COMPUTER_USE_DIR: &str = "computer-use";
const DISCOVERY_FILE: &str = "discovery.json";
const SESSION_FILE: &str = "session.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerDiscovery {
    pub schema_version: u32,
    pub app: String,
    pub variant: String,
    pub pid: u32,
    pub broker_url: String,
    pub mcp_url: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAttachRequest {
    pub client_name: Option<String>,
    pub client_pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalAttachResponse {
    pub schema_version: u32,
    pub session_id: String,
    pub mcp_url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSession {
    pub schema_version: u32,
    pub broker_url: String,
    pub mcp_url: String,
    pub session_id: String,
    pub token: String,
    pub updated_at: i64,
}

pub fn discovery_path() -> Result<PathBuf> {
    Ok(state_dir()?.join(DISCOVERY_FILE))
}

pub fn session_path() -> Result<PathBuf> {
    Ok(state_dir()?.join(SESSION_FILE))
}

pub fn write_discovery(broker_url: String, mcp_url: String) -> Result<BrokerDiscovery> {
    let discovery = BrokerDiscovery {
        schema_version: 1,
        app: "sessio".into(),
        variant: crate::app_paths::app_dir_name()
            .trim_start_matches('.')
            .into(),
        pid: std::process::id(),
        broker_url,
        mcp_url,
        updated_at: unix_now(),
    };
    write_private_json(&discovery_path()?, &discovery)?;
    Ok(discovery)
}

pub fn read_discovery() -> Result<Option<BrokerDiscovery>> {
    read_optional_json(&discovery_path()?)
}

pub fn write_session(session: &ExternalSession) -> Result<()> {
    write_private_json(&session_path()?, session)
}

pub fn read_session() -> Result<Option<ExternalSession>> {
    read_optional_json(&session_path()?)
}

pub fn remove_session() {
    if let Ok(path) = session_path() {
        let _ = std::fs::remove_file(path);
    }
}

pub fn session_from_attach(broker_url: String, attach: ExternalAttachResponse) -> ExternalSession {
    ExternalSession {
        schema_version: 1,
        broker_url,
        mcp_url: attach.mcp_url,
        session_id: attach.session_id,
        token: attach.token,
        updated_at: unix_now(),
    }
}

fn state_dir() -> Result<PathBuf> {
    Ok(crate::app_paths::app_home()?.join(COMPUTER_USE_DIR))
}

fn read_optional_json<T: DeserializeOwned>(path: &PathBuf) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let value =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(value))
}

fn write_private_json<T: Serialize>(path: &PathBuf, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent", path.display()))?;
    ensure_private_dir(parent)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(&bytes)
        .with_context(|| format!("write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("finish {}", path.display()))?;
    Ok(())
}

fn ensure_private_dir(path: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("chmod {}", path.display()))?;
    }
    Ok(())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_from_attach_preserves_broker_and_token() {
        let session = session_from_attach(
            "http://127.0.0.1:1234".into(),
            ExternalAttachResponse {
                schema_version: 1,
                session_id: "external-1".into(),
                mcp_url: "http://127.0.0.1:1234/mcp".into(),
                token: "tok".into(),
            },
        );

        assert_eq!(session.broker_url, "http://127.0.0.1:1234");
        assert_eq!(session.mcp_url, "http://127.0.0.1:1234/mcp");
        assert_eq!(session.session_id, "external-1");
        assert_eq!(session.token, "tok");
    }

    #[test]
    fn discovery_serialization_does_not_include_token() {
        let discovery = BrokerDiscovery {
            schema_version: 1,
            app: "sessio".into(),
            variant: "sessio-dev".into(),
            pid: 42,
            broker_url: "http://127.0.0.1:1234".into(),
            mcp_url: "http://127.0.0.1:1234/mcp".into(),
            updated_at: 100,
        };

        let json = serde_json::to_value(&discovery).unwrap();
        assert!(json.get("token").is_none());
        assert_eq!(json["brokerUrl"], "http://127.0.0.1:1234");
        assert_eq!(json["mcpUrl"], "http://127.0.0.1:1234/mcp");
    }
}
