//! Bundled Pi extension deployment for Shape C computer-use injection.
//!
//! Sessio loads Pi via `pi --mode rpc -e <extension.ts>` instead of modifying
//! the user's global `~/.pi/agent/settings.json`. The extension source is
//! embedded in the desktop binary and materialized into Sessio's private app
//! state directory when a Pi session requests computer use.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const EXTENSION_SOURCE: &str =
    include_str!("../../resources/skills/computer-use/pi/sessio-computer-use.ts");

pub fn extension_path() -> Result<PathBuf> {
    Ok(crate::app_paths::app_home()?
        .join("computer-use")
        .join("pi-extension")
        .join("sessio-computer-use.ts"))
}

pub fn ensure_extension_file() -> Result<PathBuf> {
    let path = extension_path()?;
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent", path.display()))?;
    ensure_private_dir(parent)?;
    let needs_write = std::fs::read_to_string(&path)
        .map(|current| current != EXTENSION_SOURCE)
        .unwrap_or(true);
    if needs_write {
        write_private_file(&path, EXTENSION_SOURCE)?;
    }
    Ok(path)
}

fn write_private_file(path: &Path, contents: &str) -> Result<()> {
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
    file.write_all(contents.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_extension_registers_computer_tools() {
        assert!(EXTENSION_SOURCE.contains("export default function sessioComputerUse"));
        assert!(EXTENSION_SOURCE.contains("pi.registerTool"));
        assert!(EXTENSION_SOURCE.contains("computer_get_app_state"));
        assert!(EXTENSION_SOURCE.contains("SESSIO_COMPUTER_USE_MCP_URL"));
        assert!(EXTENSION_SOURCE.contains("SESSIO_COMPUTER_USE_TOKEN"));
    }
}
