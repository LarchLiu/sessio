use anyhow::Result;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use super::defaults::default_app_config;
use super::{AppConfig, ConfigRecoveryNotice};

static CONFIG_RECOVERY_NOTICE: OnceLock<Mutex<Option<ConfigRecoveryNotice>>> = OnceLock::new();

pub fn take_config_recovery_notice() -> Option<ConfigRecoveryNotice> {
    config_recovery_notice_slot()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

pub(super) fn recover_invalid_config(
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
