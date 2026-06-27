//! Shared desktop-control permission layer.
//!
//! This is the single source of truth for "what desktop control is the Sessio
//! process currently allowed to do," consumed by two products:
//!
//! - **Appshot** (screenshot capture) — consumes the screenshot tier and keeps
//!   its existing UX; accessibility stays optional decoration for it.
//! - **Computer use** — consumes all three tiers; accessibility is a hard
//!   dependency (element-tree inspection / `click_element`).
//!
//! The module deliberately separates *OS permission state* (screen-capture
//! grant, accessibility trust, platform support) from *product policy* (session
//! approval, app approval, provider readiness, foreground takeover). Only the OS
//! layer lives here. The actual privileged FFI checks
//! (`ScreenCaptureAccess.preflight()`, `AXIsProcessTrustedWithOptions`) stay in
//! `lib.rs`; this module turns their raw booleans into the tiered status both
//! products render, so the derivation is pure and unit-testable.

use serde::Serialize;

/// The platform Sessio is running on, as reported to the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopPlatform {
    Macos,
    Windows,
    Linux,
    Other,
}

impl DesktopPlatform {
    pub fn as_str(self) -> &'static str {
        match self {
            DesktopPlatform::Macos => "macos",
            DesktopPlatform::Windows => "windows",
            DesktopPlatform::Linux => "linux",
            DesktopPlatform::Other => "other",
        }
    }

    /// The platform this binary was compiled for.
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            DesktopPlatform::Macos
        }
        #[cfg(all(not(target_os = "macos"), windows))]
        {
            DesktopPlatform::Windows
        }
        #[cfg(all(not(target_os = "macos"), target_os = "linux"))]
        {
            DesktopPlatform::Linux
        }
        #[cfg(all(not(target_os = "macos"), not(windows), not(target_os = "linux")))]
        {
            DesktopPlatform::Other
        }
    }
}

/// Grant state for a single OS permission, plus whether the platform even has
/// the concept (so the UI can distinguish "denied" from "not applicable here").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionTier {
    pub granted: bool,
    pub supported: bool,
}

impl PermissionTier {
    pub const fn new(granted: bool, supported: bool) -> Self {
        Self { granted, supported }
    }

    /// Whether this tier is usable right now: either the platform does not gate
    /// it (`!supported`, e.g. Linux screenshot today) or it is explicitly
    /// granted.
    pub fn usable(self) -> bool {
        !self.supported || self.granted
    }
}

/// The raw OS-permission inputs the derivation needs. Produced by the
/// platform-specific FFI checks in `lib.rs`; kept as a plain struct so the
/// derivation below is a pure function over booleans.
#[derive(Debug, Clone, Copy)]
pub struct DesktopControlInputs {
    pub platform: DesktopPlatform,
    /// Whether this platform gates desktop control behind OS permissions at all.
    pub requires_permission: bool,
    pub screenshots: PermissionTier,
    pub accessibility: PermissionTier,
    /// Whether the current platform + provider can perform input injection
    /// (`canControl`). This is an OS/provider-support fact, not a product
    /// policy decision — input injection is net-new (Phase 3) and stays `false`
    /// until a provider implements it for the platform.
    pub input_injection_supported: bool,
}

/// The tiered desktop-control permission status shared by Appshot and computer
/// use. `canObserve` / `canInspect` / `canControl` are the product-facing
/// capability gates derived from the raw tiers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopControlPermissionStatus {
    pub platform: String,
    pub requires_permission: bool,
    pub screenshots: PermissionTier,
    pub accessibility: PermissionTier,
    /// Capture screenshots / visual state. Gated by the screenshot tier.
    pub can_observe: bool,
    /// Inspect the accessibility / UI hierarchy. Gated by the accessibility tier.
    pub can_inspect: bool,
    /// Inject input under the current platform/provider policy. Requires
    /// accessibility trust (macOS needs it for synthesized events to land) AND
    /// platform/provider input-injection support.
    pub can_control: bool,
}

impl DesktopControlPermissionStatus {
    /// Pure derivation of the tiered status from raw OS inputs.
    pub fn derive(inputs: DesktopControlInputs) -> Self {
        let can_observe = inputs.screenshots.usable();
        let can_inspect = inputs.accessibility.usable();
        // Control needs the process to be accessibility-trusted (so synthesized
        // events are honored) and the platform/provider to actually support
        // injection. Observation/inspection alone never imply control.
        let can_control = inputs.input_injection_supported && inputs.accessibility.usable();
        Self {
            platform: inputs.platform.as_str().to_string(),
            requires_permission: inputs.requires_permission,
            screenshots: inputs.screenshots,
            accessibility: inputs.accessibility,
            can_observe,
            can_inspect,
            can_control,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macos(screens: bool, ax: bool, inject: bool) -> DesktopControlInputs {
        DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(screens, true),
            accessibility: PermissionTier::new(ax, true),
            input_injection_supported: inject,
        }
    }

    #[test]
    fn macos_no_grants_disables_all_tiers() {
        let s = DesktopControlPermissionStatus::derive(macos(false, false, false));
        assert!(!s.can_observe);
        assert!(!s.can_inspect);
        assert!(!s.can_control);
    }

    #[test]
    fn macos_screenshot_only_observes_but_cannot_inspect_or_control() {
        let s = DesktopControlPermissionStatus::derive(macos(true, false, false));
        assert!(s.can_observe);
        assert!(!s.can_inspect);
        assert!(!s.can_control);
    }

    #[test]
    fn macos_accessibility_enables_inspect_but_control_needs_provider_support() {
        // Accessibility granted, but no input-injection provider yet (Phase 3).
        let s = DesktopControlPermissionStatus::derive(macos(true, true, false));
        assert!(s.can_observe);
        assert!(s.can_inspect);
        assert!(!s.can_control, "control must wait for injection support");

        // With provider support, accessibility grant unlocks control.
        let s = DesktopControlPermissionStatus::derive(macos(true, true, true));
        assert!(s.can_control);
    }

    #[test]
    fn control_requires_accessibility_even_with_injection_support() {
        // Injection supported but process not accessibility-trusted → no control.
        let s = DesktopControlPermissionStatus::derive(macos(true, false, true));
        assert!(!s.can_control);
    }

    #[test]
    fn unsupported_platform_treats_tiers_as_usable() {
        // On a platform that does not gate desktop control, unsupported tiers
        // are "usable" so observation is not blocked by a permission that does
        // not exist there.
        let inputs = DesktopControlInputs {
            platform: DesktopPlatform::Linux,
            requires_permission: false,
            screenshots: PermissionTier::new(false, false),
            accessibility: PermissionTier::new(false, false),
            input_injection_supported: false,
        };
        let s = DesktopControlPermissionStatus::derive(inputs);
        assert!(s.can_observe);
        assert!(s.can_inspect);
        // Control still false: no injection provider.
        assert!(!s.can_control);
        assert_eq!(s.platform, "linux");
    }
}
