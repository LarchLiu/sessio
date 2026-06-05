use std::collections::{BTreeMap, HashSet};
#[cfg(windows)]
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;
#[cfg(not(windows))]
use std::time::Instant;

use anyhow::{Context, Result};

const SHELL_ENV_TIMEOUT_NOTE: &str = "shell env import skipped";
const SHELL_ENV_TIMEOUT: Duration = Duration::from_secs(2);

pub fn import_login_shell_env() {
    match load_login_shell_env() {
        Ok(env) => {
            let changed = apply_shell_env(env);
            log::info!("[shell-env] imported {changed} variables from login shell");
        }
        Err(error) => {
            log::warn!("[shell-env] {SHELL_ENV_TIMEOUT_NOTE}: {error}");
        }
    }
}

fn load_login_shell_env() -> Result<BTreeMap<String, String>> {
    #[cfg(windows)]
    {
        return load_windows_shell_env();
    }

    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/bin/zsh".to_string());
        let mut child = Command::new(shell)
            .arg("-lc")
            .arg("env -0")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("run login shell env")?;
        let started = Instant::now();
        loop {
            if child.try_wait().context("poll login shell env")?.is_some() {
                break;
            }
            if started.elapsed() >= SHELL_ENV_TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("login shell env timed out after {:?}", SHELL_ENV_TIMEOUT);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let output = child.wait_with_output().context("read login shell env")?;
        if !output.status.success() {
            anyhow::bail!(
                "login shell env failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(parse_env0(&output.stdout))
    }
}

#[cfg(windows)]
fn load_windows_shell_env() -> Result<BTreeMap<String, String>> {
    let output = Command::new("cmd")
        .args(["/C", "set"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("run Windows shell env")?;
    if !output.status.success() {
        anyhow::bail!(
            "Windows shell env failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut env = parse_env_lines(&output.stdout);
    let path_key = if env.contains_key("PATH") {
        Some("PATH")
    } else if env.contains_key("Path") {
        Some("Path")
    } else {
        None
    };
    if let Some(path) = path_key.and_then(|key| env.get_mut(key)) {
        append_windows_tool_dirs(path);
    } else {
        let mut path = String::new();
        append_windows_tool_dirs(&mut path);
        if !path.is_empty() {
            env.insert("PATH".to_string(), path);
        }
    }
    Ok(env)
}

#[cfg(windows)]
fn append_windows_tool_dirs(path: &mut String) {
    let mut dirs = Vec::new();
    for key in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA", "APPDATA"] {
        if let Ok(root) = std::env::var(key) {
            match key {
                "ProgramFiles" | "ProgramFiles(x86)" => {
                    dirs.push(format!(r"{}\nodejs", root));
                }
                "LOCALAPPDATA" => {
                    dirs.push(format!(r"{}\Programs\nodejs", root));
                }
                "APPDATA" => {
                    dirs.push(format!(r"{}\npm", root));
                }
                _ => {}
            }
        }
    }
    append_existing_path_dirs(path, dirs);
}

#[cfg(windows)]
fn append_existing_path_dirs(path: &mut String, dirs: Vec<String>) {
    let mut seen: HashSet<String> = path
        .split(';')
        .filter(|part| !part.is_empty())
        .map(|part| part.to_ascii_lowercase())
        .collect();
    for dir in dirs {
        if !Path::new(&dir).exists() || !seen.insert(dir.to_ascii_lowercase()) {
            continue;
        }
        if !path.is_empty() {
            path.push(';');
        }
        path.push_str(&dir);
    }
}

/// Serializes process-wide environment mutations. `std::env::set_var`/`remove_var`
/// are not safe to run while other threads read the environment, so every writer in
/// this crate holds this lock for the duration of its mutations.
pub(crate) fn env_write_guard() -> std::sync::MutexGuard<'static, ()> {
    static ENV_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    ENV_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn apply_shell_env(env: BTreeMap<String, String>) -> usize {
    let _env_guard = env_write_guard();
    let allowed = shell_env_allowlist();
    let mut changed = 0;
    for (key, value) in env {
        if value.is_empty() {
            continue;
        }
        if key.eq_ignore_ascii_case("PATH") {
            if merge_path_from_shell(&value) {
                changed += 1;
            }
            continue;
        }
        if !allowed.contains(key.as_str()) {
            continue;
        }
        if std::env::var_os(&key).is_none() {
            std::env::set_var(&key, value);
            changed += 1;
        }
    }
    changed
}

fn merge_path_from_shell(shell_path: &str) -> bool {
    let Some(next) = merged_path(shell_path, std::env::var("PATH").ok().as_deref()) else {
        return false;
    };
    if std::env::var("PATH").ok().as_deref() == Some(next.as_str()) {
        return false;
    }
    std::env::set_var("PATH", next);
    true
}

fn merged_path(shell_path: &str, current_path: Option<&str>) -> Option<String> {
    merged_path_with_separator(shell_path, current_path, if cfg!(windows) { ';' } else { ':' })
}

fn merged_path_with_separator(
    shell_path: &str,
    current_path: Option<&str>,
    separator: char,
) -> Option<String> {
    let mut seen = HashSet::new();
    let mut parts = Vec::new();
    for source in [Some(shell_path), current_path].into_iter().flatten() {
        for part in source.split(separator).filter(|part| !part.is_empty()) {
            if seen.insert(part.to_string()) {
                parts.push(part.to_string());
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(&separator.to_string()))
}

fn parse_env0(bytes: &[u8]) -> BTreeMap<String, String> {
    bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            if entry.is_empty() {
                return None;
            }
            let text = String::from_utf8_lossy(entry);
            let (key, value) = text.split_once('=')?;
            is_valid_env_key(key).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg(windows)]
fn parse_env_lines(bytes: &[u8]) -> BTreeMap<String, String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            is_valid_env_key(key).then(|| (key.to_string(), value.to_string()))
        })
        .collect()
}

fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn shell_env_allowlist() -> HashSet<&'static str> {
    [
        "PATH",
        "HOME",
        "SHELL",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TERM",
        "TMPDIR",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
        "NPM_CONFIG_PREFIX",
        "NPM_CONFIG_CACHE",
        "NPM_CONFIG_REGISTRY",
        "NPM_CONFIG_USERCONFIG",
        "NODE_EXTRA_CA_CERTS",
        "NODE_OPTIONS",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GOOGLE_API_KEY",
        "GEMINI_API_KEY",
        "CODEX_HOME",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env0_ignores_invalid_keys() {
        let parsed = parse_env0(b"PATH=/bin\0BAD-KEY=x\0HTTP_PROXY=http://proxy\0");
        assert_eq!(parsed.get("PATH").map(String::as_str), Some("/bin"));
        assert_eq!(
            parsed.get("HTTP_PROXY").map(String::as_str),
            Some("http://proxy")
        );
        assert!(!parsed.contains_key("BAD-KEY"));
    }

    #[test]
    fn merged_path_prepends_shell_entries_and_dedupes() {
        assert_eq!(
            merged_path("/opt/bin:/usr/bin", Some("/usr/bin:/bin")).as_deref(),
            Some("/opt/bin:/usr/bin:/bin")
        );
    }

    #[test]
    fn merged_windows_path_uses_semicolon_separator() {
        assert_eq!(
            merged_path_with_separator(
                r"C:\Users\alex\AppData\Roaming\npm;C:\Program Files\nodejs",
                Some(r"C:\Program Files\nodejs;C:\Windows\System32"),
                ';'
            )
            .as_deref(),
            Some(r"C:\Users\alex\AppData\Roaming\npm;C:\Program Files\nodejs;C:\Windows\System32")
        );
    }
}
