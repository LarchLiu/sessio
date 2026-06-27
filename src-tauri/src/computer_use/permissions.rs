//! Permission gating for computer use.
//!
//! Bridges the shared [`crate::desktop_control`] OS-permission layer to the
//! computer-use host. The host asks "may I observe / inspect / control right
//! now?"; this module answers from the desktop-control status plus the
//! provider's control support, keeping OS-permission concerns out of the host's
//! orchestration logic.

use crate::desktop_control::DesktopControlPermissionStatus;

/// Which capability a host operation requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredCapability {
    /// Screenshot / visual capture.
    Observe,
    /// Accessibility / element-tree inspection.
    Inspect,
    /// Input injection.
    Control,
}

/// Why a permission check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PermissionDenied {
    #[error("screen capture permission is required")]
    Observe,
    #[error("accessibility permission is required")]
    Inspect,
    #[error("input control is not permitted or not supported on this platform")]
    Control,
}

/// Evaluate whether the current desktop-control status permits a capability.
pub fn check(
    status: &DesktopControlPermissionStatus,
    capability: RequiredCapability,
) -> Result<(), PermissionDenied> {
    match capability {
        RequiredCapability::Observe if status.can_observe => Ok(()),
        RequiredCapability::Observe => Err(PermissionDenied::Observe),
        RequiredCapability::Inspect if status.can_inspect => Ok(()),
        RequiredCapability::Inspect => Err(PermissionDenied::Inspect),
        RequiredCapability::Control if status.can_control => Ok(()),
        RequiredCapability::Control => Err(PermissionDenied::Control),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_control::{
        DesktopControlInputs, DesktopControlPermissionStatus, DesktopPlatform, PermissionTier,
    };

    fn status(screens: bool, ax: bool, inject: bool) -> DesktopControlPermissionStatus {
        DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(screens, true),
            accessibility: PermissionTier::new(ax, true),
            input_injection_supported: inject,
        })
    }

    #[test]
    fn observe_requires_screenshot_permission() {
        assert_eq!(
            check(&status(false, false, false), RequiredCapability::Observe),
            Err(PermissionDenied::Observe)
        );
        assert!(check(&status(true, false, false), RequiredCapability::Observe).is_ok());
    }

    #[test]
    fn inspect_requires_accessibility() {
        assert_eq!(
            check(&status(true, false, false), RequiredCapability::Inspect),
            Err(PermissionDenied::Inspect)
        );
        assert!(check(&status(true, true, false), RequiredCapability::Inspect).is_ok());
    }

    #[test]
    fn control_requires_accessibility_and_injection_support() {
        // Accessibility but no injection support.
        assert_eq!(
            check(&status(true, true, false), RequiredCapability::Control),
            Err(PermissionDenied::Control)
        );
        assert!(check(&status(true, true, true), RequiredCapability::Control).is_ok());
    }
}
