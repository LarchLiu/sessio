use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use super::defaults::{default_app_config, raw_config_with_defaults};
use super::parser::parse_raw_config;
use super::raw::RawConfig;
use super::{
    resolve_app_config, serialize_app_config, AppConfig, ConfigRecoveryNotice, MemoryConfig,
};
use crate::app_paths;

static CONFIG_RECOVERY_NOTICE: OnceLock<Mutex<Option<ConfigRecoveryNotice>>> = OnceLock::new();

pub fn load_config() -> Result<AppConfig> {
    load_config_from_path(&config_path()?)
}

pub(crate) fn load_config_strict() -> Result<AppConfig> {
    load_config_from_path_strict(&config_path()?)
}

pub fn take_config_recovery_notice() -> Option<ConfigRecoveryNotice> {
    config_recovery_notice_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

pub fn load_memory_config() -> Result<MemoryConfig> {
    load_config()?.memory.context("memory is not configured")
}

pub fn save_memory_config(config: &MemoryConfig) -> Result<()> {
    let mut app_config = load_config().or_else(|_| default_app_config())?;
    app_config.memory = Some(config.clone());
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

pub(super) fn load_config_from_path(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        write_default_config_file(path)?;
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    if contents.trim().is_empty() {
        return finalize_loaded_config(path, Some(contents.as_str()), RawConfig::default());
    }
    let raw = match parse_raw_config(&contents) {
        Ok(raw) => raw,
        Err(error) => {
            return recover_invalid_config(path, Some(contents.as_str()), &error)
                .with_context(|| format!("parse config {}", path.display()));
        }
    };
    finalize_loaded_config(path, Some(contents.as_str()), raw)
}

fn load_config_from_path_strict(path: &Path) -> Result<AppConfig> {
    if !path.exists() {
        write_default_config_file(path)?;
    }
    let contents =
        fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    if contents.trim().is_empty() {
        return finalize_loaded_config_strict(RawConfig::default());
    }
    let raw = parse_raw_config(&contents)?;
    finalize_loaded_config_strict(raw)
}

fn finalize_loaded_config(
    path: &Path,
    contents: Option<&str>,
    raw: RawConfig,
) -> Result<AppConfig> {
    let (raw, added_defaults) = raw_config_with_defaults(raw)?;
    let config = match resolve_app_config(raw.clone(), true) {
        Ok(config) => config,
        Err(error) => {
            return recover_invalid_config(path, contents, &error)
                .with_context(|| format!("resolve config {}", path.display()));
        }
    };
    if added_defaults {
        save_config(&resolve_app_config(raw, false)?)?;
    }
    Ok(config)
}

fn finalize_loaded_config_strict(raw: RawConfig) -> Result<AppConfig> {
    let (raw, _) = raw_config_with_defaults(raw)?;
    resolve_app_config(raw, true)
}

fn recover_invalid_config(
    path: &Path,
    contents: Option<&str>,
    error: &anyhow::Error,
) -> Result<AppConfig> {
    let default = default_app_config()?;
    let error_text = format!("{error:#}");
    let line_number = extract_line_number(&error_text);
    let line_text = line_number
        .and_then(|line| contents.and_then(|text| text.lines().nth(line.saturating_sub(1))))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string);
    log::warn!(
        "[config] invalid config at {}: {error:#}. Keeping the file unchanged and using defaults for this launch.",
        path.display(),
    );

    set_config_recovery_notice(ConfigRecoveryNotice {
        path: path.display().to_string(),
        backup_path: None,
        error: error_text,
        line_number,
        line_text,
        used_defaults: true,
    });

    Ok(default)
}

fn config_recovery_notice_slot() -> &'static Mutex<Option<ConfigRecoveryNotice>> {
    CONFIG_RECOVERY_NOTICE.get_or_init(|| Mutex::new(None))
}

fn set_config_recovery_notice(notice: ConfigRecoveryNotice) {
    *config_recovery_notice_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(notice);
}

fn extract_line_number(message: &str) -> Option<usize> {
    let needle = "line ";
    let start = message.find(needle)? + needle.len();
    let digits = message[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<usize>().ok()
    }
}

fn config_path() -> Result<std::path::PathBuf> {
    app_paths::config_path()
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
