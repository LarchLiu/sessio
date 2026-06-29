//! File-backed diagnostics for computer-use coordination and calibration.
//!
//! These records are written as JSON Lines so screenshot/coordinate issues can
//! be inspected after the fact without relying on the global app log level.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};

const COMPUTER_USE_DIR: &str = "computer-use";
const DIAGNOSTICS_LOG_FILE: &str = "diagnostics.log";
const TEST_DIAGNOSTICS_DIR: &str = "sessio-computer-use";

/// Best-effort append of one JSONL diagnostics record.
pub fn write(event: &str, payload: Value) {
    if let Err(error) = write_inner(event, payload) {
        log::warn!("[computer-use:diagnostics] failed to append {event}: {error}");
    }
}

pub fn diagnostics_log_path() -> Result<PathBuf> {
    let dir = if cfg!(test) {
        std::env::temp_dir().join(TEST_DIAGNOSTICS_DIR)
    } else {
        crate::app_paths::app_home()?.join(COMPUTER_USE_DIR)
    };
    Ok(dir.join(DIAGNOSTICS_LOG_FILE))
}

fn write_inner(event: &str, payload: Value) -> Result<()> {
    let path = diagnostics_log_path()?;
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent", path.display()))?;
    ensure_private_dir(parent)?;

    let mut record = match payload {
        Value::Object(map) => map,
        other => {
            let mut map = Map::new();
            map.insert("payload".into(), other);
            map
        }
    };
    record.insert(
        "ts".into(),
        Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
    );
    record.insert("event".into(), Value::String(event.to_string()));

    let mut line = serde_json::to_vec(&record)?;
    line.push(b'\n');

    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(&line)
        .with_context(|| format!("append {}", path.display()))?;
    Ok(())
}

fn ensure_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("chmod {}", path.display()))?;
    }
    Ok(())
}
