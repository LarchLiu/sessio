//! Computer-use onboarding helpers exposed to agents.
//!
//! These helpers keep permission discovery and "open the right settings page"
//! available through the same MCP surface as the rest of computer use.

use serde::{Deserialize, Serialize};

use crate::desktop_control::DesktopControlPermissionStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Screenshots,
    Accessibility,
}

impl PermissionKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "screenshots" | "screen_capture" | "screen-capture" | "screenCapture" => {
                Ok(Self::Screenshots)
            }
            "accessibility" => Ok(Self::Accessibility),
            other => Err(format!("unknown computer-use permission: {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Screenshots => "screenshots",
            Self::Accessibility => "accessibility",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequirement {
    pub permission: PermissionKind,
    pub granted: bool,
    pub supported: bool,
    pub can_grant: bool,
    pub code: &'static str,
    pub guidance: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUsePermissions {
    pub status: DesktopControlPermissionStatus,
    pub ready: bool,
    pub missing: Vec<PermissionKind>,
    pub requirements: Vec<PermissionRequirement>,
    pub guidance: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantPermissionResult {
    pub permission: PermissionKind,
    pub opened: bool,
    pub status: DesktopControlPermissionStatus,
    pub guidance: String,
}

pub fn permissions_status(status: &DesktopControlPermissionStatus) -> ComputerUsePermissions {
    let requirements = vec![
        requirement(
            PermissionKind::Screenshots,
            status.screenshots.granted,
            status.screenshots.supported,
            status.platform.as_str(),
        ),
        requirement(
            PermissionKind::Accessibility,
            status.accessibility.granted,
            status.accessibility.supported,
            status.platform.as_str(),
        ),
    ];
    let missing: Vec<PermissionKind> = requirements
        .iter()
        .filter(|req| req.supported && !req.granted)
        .map(|req| req.permission)
        .collect();
    let mut guidance: Vec<String> = requirements
        .iter()
        .filter(|req| req.supported && !req.granted)
        .map(|req| req.guidance.clone())
        .collect();
    if status.accessibility.granted && !status.can_control {
        guidance.push(
            "Input control is not currently available even though Accessibility is granted; this platform/provider may not support input injection."
                .into(),
        );
    }
    if guidance.is_empty() {
        guidance.push("Computer-use permissions are ready.".into());
    }

    ComputerUsePermissions {
        ready: status.can_observe && status.can_inspect && status.can_control,
        status: status.clone(),
        missing,
        requirements,
        guidance,
    }
}

pub fn grant_permission(
    permission: PermissionKind,
    status: &DesktopControlPermissionStatus,
) -> Result<GrantPermissionResult, String> {
    let opened = open_permission_settings(permission)?;
    let guidance = if opened {
        format!(
            "Opened System Settings for {}. Enable Sessio there, then call computer_permissions again.",
            permission.as_str()
        )
    } else {
        format!(
            "No settings page was opened for {} on this platform. Call computer_permissions to inspect current status.",
            permission.as_str()
        )
    };
    Ok(GrantPermissionResult {
        permission,
        opened,
        status: status.clone(),
        guidance,
    })
}

fn requirement(
    permission: PermissionKind,
    granted: bool,
    supported: bool,
    platform: &str,
) -> PermissionRequirement {
    let can_grant = supported && !granted && platform == "macos";
    let code = if !supported {
        "not_required"
    } else if granted {
        "granted"
    } else {
        match permission {
            PermissionKind::Screenshots => "missing_screenshots",
            PermissionKind::Accessibility => "missing_accessibility",
        }
    };
    let guidance = match (permission, supported, granted, can_grant) {
        (_, false, _, _) => format!(
            "{} is not required on this platform.",
            permission.as_str()
        ),
        (PermissionKind::Screenshots, true, true, _) => "Screen capture is granted.".into(),
        (PermissionKind::Accessibility, true, true, _) => "Accessibility is granted.".into(),
        (PermissionKind::Screenshots, true, false, true) => {
            "Run computer_grant with permission=\"screenshots\", enable Sessio in Screen & System Audio Recording, then retry."
                .into()
        }
        (PermissionKind::Accessibility, true, false, true) => {
            "Run computer_grant with permission=\"accessibility\", enable Sessio in Accessibility, then retry."
                .into()
        }
        (PermissionKind::Screenshots, true, false, false) => {
            "Screen capture is missing and cannot be opened automatically on this platform.".into()
        }
        (PermissionKind::Accessibility, true, false, false) => {
            "Accessibility is missing and cannot be opened automatically on this platform.".into()
        }
    };

    PermissionRequirement {
        permission,
        granted,
        supported,
        can_grant,
        code,
        guidance,
    }
}

#[cfg(target_os = "macos")]
fn open_permission_settings(permission: PermissionKind) -> Result<bool, String> {
    let url = match permission {
        PermissionKind::Screenshots => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        PermissionKind::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
    };
    let status = std::process::Command::new("open")
        .arg(url)
        .status()
        .map_err(|e| format!("open permission settings: {e}"))?;
    if status.success() {
        Ok(true)
    } else {
        Err(format!("open permission settings failed: {status}"))
    }
}

#[cfg(not(target_os = "macos"))]
fn open_permission_settings(_permission: PermissionKind) -> Result<bool, String> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_control::{
        DesktopControlInputs, DesktopControlPermissionStatus, DesktopPlatform, PermissionTier,
    };

    fn macos_status(screenshots: bool, accessibility: bool) -> DesktopControlPermissionStatus {
        DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(screenshots, true),
            accessibility: PermissionTier::new(accessibility, true),
            input_injection_supported: true,
        })
    }

    #[test]
    fn permissions_status_lists_missing_grants() {
        let status = permissions_status(&macos_status(false, true));
        assert!(!status.ready);
        assert_eq!(status.missing, vec![PermissionKind::Screenshots]);
        assert_eq!(status.requirements[0].code, "missing_screenshots");
        assert!(status.requirements[0].can_grant);
    }

    #[test]
    fn permissions_status_reports_ready_when_all_tiers_work() {
        let status = permissions_status(&macos_status(true, true));
        assert!(status.ready);
        assert!(status.missing.is_empty());
        assert_eq!(status.guidance, vec!["Computer-use permissions are ready."]);
    }

    #[test]
    fn permission_kind_parses_aliases() {
        assert_eq!(
            PermissionKind::parse("screen_capture").unwrap(),
            PermissionKind::Screenshots
        );
        assert_eq!(
            PermissionKind::parse("accessibility").unwrap(),
            PermissionKind::Accessibility
        );
        assert!(PermissionKind::parse("camera").is_err());
    }
}
