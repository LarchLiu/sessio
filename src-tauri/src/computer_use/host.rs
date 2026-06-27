//! Computer-use host: orchestration.
//!
//! The host is the agent-agnostic entry point the injected tools call into. It
//! enforces the full gating chain for every operation:
//!
//! 1. **settings** — global enable + control/foreground policy,
//! 2. **OS permission** — observe / inspect / control tiers,
//! 3. **approval** — session approval + per-app approval,
//! 4. **lease + snapshot** — one app at a time, act only on the latest snapshot,
//!
//! then dispatches to the [`ComputerUseProvider`]. The provider performs the
//! privileged work (Phase 3); the host owns the policy.

use std::sync::Arc;

use crate::desktop_control::DesktopControlPermissionStatus;

use super::approvals::{ApprovalDecision, ApprovalRegistry};
use super::lease::{LeaseError, LeaseRegistry, SnapshotError, SnapshotId};
use super::permissions::{self, PermissionDenied, RequiredCapability};
use super::provider::{
    AllowedAction, AppState, AppTarget, ComputerUseProvider, InstalledApp, ProviderError,
    ScrollDirection,
};
use super::settings::ComputerUseSettings;

/// Errors surfaced to the agent (mapped into tool errors by the injection layer).
#[derive(Debug, thiserror::Error)]
pub enum ComputerUseError {
    #[error("computer use is disabled")]
    Disabled,
    #[error("permission denied: {0}")]
    Permission(#[from] PermissionDenied),
    #[error("approval required: {0:?}")]
    Approval(ApprovalDecision),
    #[error("lease error: {0}")]
    Lease(#[from] LeaseError),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("input control is disabled by settings")]
    ControlDisabled,
}

/// The host. Cheap to clone (`Arc` internals) so it can be shared across the
/// injection layer and Tauri commands.
#[derive(Clone)]
pub struct ComputerUseHost {
    provider: Arc<dyn ComputerUseProvider>,
    leases: Arc<LeaseRegistry>,
    approvals: Arc<ApprovalRegistry>,
    settings: Arc<ComputerUseSettings>,
}

impl ComputerUseHost {
    pub fn new(
        provider: Arc<dyn ComputerUseProvider>,
        settings: ComputerUseSettings,
    ) -> Self {
        Self {
            provider,
            leases: Arc::new(LeaseRegistry::new()),
            approvals: Arc::new(ApprovalRegistry::new()),
            settings: Arc::new(settings),
        }
    }

    /// Construct a host backed by the platform's real provider (macOS today,
    /// an unsupported stub elsewhere). Used by the runtime/injection layer.
    pub fn with_platform_provider(settings: ComputerUseSettings) -> Self {
        Self::new(super::platform::default_provider(), settings)
    }

    pub fn approvals(&self) -> &ApprovalRegistry {
        &self.approvals
    }

    fn require_enabled(&self) -> Result<(), ComputerUseError> {
        if self.settings.enabled {
            Ok(())
        } else {
            Err(ComputerUseError::Disabled)
        }
    }

    fn require_permission(
        &self,
        status: &DesktopControlPermissionStatus,
        cap: RequiredCapability,
    ) -> Result<(), ComputerUseError> {
        permissions::check(status, cap).map_err(ComputerUseError::from)
    }

    fn require_approval(
        &self,
        session_id: &str,
        app_id: &str,
    ) -> Result<(), ComputerUseError> {
        match self.approvals.decide(session_id, &app_id.to_string()) {
            ApprovalDecision::Allowed => Ok(()),
            other => Err(ComputerUseError::Approval(other)),
        }
    }

    // --- Tool surface ----------------------------------------------------

    /// `computer_status` — report whether the session can use computer use and
    /// what it currently may do. Never fails; it is the agent's discovery probe.
    pub fn status(
        &self,
        session_id: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> ComputerUseStatus {
        ComputerUseStatus {
            enabled: self.settings.enabled,
            session_approved: self.approvals.session_approved(session_id),
            has_lease: self.leases.has_lease(session_id),
            can_observe: perm.can_observe,
            can_inspect: perm.can_inspect,
            can_control: perm.can_control && self.settings.allow_input_injection,
        }
    }

    /// `computer_list_apps` — enumerate targetable apps. Requires the feature on
    /// and observe permission.
    pub fn list_apps(
        &self,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<Vec<InstalledApp>, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        Ok(self.provider.list_apps()?)
    }

    /// `computer_start` — open a lease on a target app. Requires the feature on,
    /// observe permission, and session+app approval.
    pub fn start(
        &self,
        session_id: &str,
        target: AppTarget,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<String, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        self.require_approval(session_id, &target.app_id)?;
        Ok(self.leases.open(session_id, target)?)
    }

    /// `computer_get_app_state` — capture screenshot + elements, stamp a fresh
    /// snapshot id, and report the allowed actions. Requires observe; inspection
    /// elements are only included when inspect permission is present.
    pub fn get_app_state(
        &self,
        session_id: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        let target = self.leases.target(session_id)?;
        let mut raw = self.provider.capture_app_state(&target)?;

        // Inspection is a separate tier: without it, do not expose the AX tree
        // and do not allow element-targeted actions.
        let can_inspect = perm.can_inspect;
        if !can_inspect {
            raw.elements.clear();
        }
        let snapshot = self
            .leases
            .with_lease(session_id, |lease| lease.next_snapshot())?;

        let allowed_actions = self.allowed_actions(perm, can_inspect);

        Ok(AppState {
            snapshot_id: snapshot.0,
            target: raw.target,
            display: raw.display,
            screenshot: raw.screenshot,
            elements: raw.elements,
            allowed_actions,
        })
    }

    fn allowed_actions(
        &self,
        perm: &DesktopControlPermissionStatus,
        can_inspect: bool,
    ) -> Vec<AllowedAction> {
        let control_ready = perm.can_control
            && self.settings.allow_input_injection
            && self.provider.supports_control();
        if !control_ready {
            return Vec::new();
        }
        AllowedAction::ALL
            .into_iter()
            .filter(|action| match action {
                // Element clicks require an inspected element tree.
                AllowedAction::ClickElement => can_inspect,
                _ => true,
            })
            .collect()
    }

    fn require_control(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppTarget, ComputerUseError> {
        self.require_enabled()?;
        if !self.settings.allow_input_injection {
            return Err(ComputerUseError::ControlDisabled);
        }
        self.require_permission(perm, RequiredCapability::Control)?;
        let target = self.leases.target(session_id)?;
        self.require_approval(session_id, &target.app_id)?;
        // Act only against the latest snapshot.
        self.leases
            .with_lease(session_id, |lease| lease.check_snapshot(snapshot))??;
        if !self.provider.supports_control() {
            return Err(ComputerUseError::Provider(ProviderError::Unsupported(
                "input injection",
            )));
        }
        Ok(target)
    }

    /// `computer_click_element` — click an inspected element. Requires control +
    /// inspect and a fresh snapshot.
    pub fn click_element(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        element_id: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<(), ComputerUseError> {
        self.require_permission(perm, RequiredCapability::Inspect)?;
        let target = self.require_control(session_id, snapshot, perm)?;
        Ok(self.provider.click_element(&target, &element_id.to_string())?)
    }

    /// `computer_type_text` — type text into the focused element.
    pub fn type_text(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        text: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<(), ComputerUseError> {
        let target = self.require_control(session_id, snapshot, perm)?;
        Ok(self.provider.type_text(&target, text)?)
    }

    /// `computer_press_key` — press a key / chord.
    pub fn press_key(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        key: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<(), ComputerUseError> {
        let target = self.require_control(session_id, snapshot, perm)?;
        Ok(self.provider.press_key(&target, key)?)
    }

    /// `computer_scroll` — scroll the target.
    pub fn scroll(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        direction: ScrollDirection,
        amount: i32,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<(), ComputerUseError> {
        let target = self.require_control(session_id, snapshot, perm)?;
        Ok(self.provider.scroll(&target, direction, amount)?)
    }

    /// `computer_stop` — release the session's lease. Idempotent.
    pub fn stop(&self, session_id: &str) {
        self.leases.close(session_id);
    }
}

/// Result of `computer_status`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseStatus {
    pub enabled: bool,
    pub session_approved: bool,
    pub has_lease: bool,
    pub can_observe: bool,
    pub can_inspect: bool,
    pub can_control: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::provider::FakeProvider;
    use crate::desktop_control::{
        DesktopControlInputs, DesktopControlPermissionStatus, DesktopPlatform, PermissionTier,
    };

    fn perm(screens: bool, ax: bool, inject: bool) -> DesktopControlPermissionStatus {
        DesktopControlPermissionStatus::derive(DesktopControlInputs {
            platform: DesktopPlatform::Macos,
            requires_permission: true,
            screenshots: PermissionTier::new(screens, true),
            accessibility: PermissionTier::new(ax, true),
            input_injection_supported: inject,
        })
    }

    fn target() -> AppTarget {
        AppTarget {
            app_id: "com.example.app".into(),
            window_id: None,
        }
    }

    fn host(settings: ComputerUseSettings) -> ComputerUseHost {
        ComputerUseHost::new(Arc::new(FakeProvider::default()), settings)
    }

    #[test]
    fn disabled_host_refuses_everything() {
        let h = host(ComputerUseSettings::default());
        let p = perm(true, true, true);
        assert!(matches!(
            h.list_apps(&p),
            Err(ComputerUseError::Disabled)
        ));
        assert!(matches!(
            h.start("s1", target(), &p),
            Err(ComputerUseError::Disabled)
        ));
    }

    #[test]
    fn start_requires_session_and_app_approval() {
        let h = host(ComputerUseSettings::observe_only());
        let p = perm(true, true, false);

        // No approval at all.
        assert!(matches!(
            h.start("s1", target(), &p),
            Err(ComputerUseError::Approval(ApprovalDecision::SessionNotApproved))
        ));

        // Session approved, app not.
        h.approvals().approve_session("s1");
        assert!(matches!(
            h.start("s1", target(), &p),
            Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
        ));

        // Both approved → lease opens.
        h.approvals().approve_app("s1", &target().app_id);
        let lease = h.start("s1", target(), &p).unwrap();
        assert!(lease.starts_with("lease-s1-"));
    }

    #[test]
    fn get_app_state_without_inspect_hides_elements_and_click() {
        let h = host(ComputerUseSettings {
            enabled: true,
            allow_input_injection: true,
            allow_foreground_takeover: false,
        });
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        // observe but NOT inspect. On macOS accessibility gates both inspect and
        // control, so with accessibility off there is neither element data nor
        // any allowed action.
        let p = perm(true, false, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        assert!(state.elements.is_empty(), "elements hidden without inspect");
        assert!(
            !state.allowed_actions.contains(&AllowedAction::ClickElement),
            "click disabled without inspect"
        );
        // Control also requires accessibility, so no actions are offered at all.
        assert!(
            state.allowed_actions.is_empty(),
            "no control actions without accessibility"
        );
    }

    #[test]
    fn observe_and_control_without_inspect_is_not_reachable_on_macos() {
        // Document the macOS coupling: control requires accessibility, and
        // accessibility is also what unlocks inspect. So "control but not
        // inspect" cannot occur on macOS — both are gated by the same grant.
        let p = perm(true, false, true);
        assert!(!p.can_inspect);
        assert!(!p.can_control, "no accessibility ⇒ no control on macOS");
    }

    #[test]
    fn get_app_state_with_full_permission_allows_all_actions() {
        let h = host(ComputerUseSettings {
            enabled: true,
            allow_input_injection: true,
            allow_foreground_takeover: false,
        });
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        assert!(!state.elements.is_empty());
        assert_eq!(state.allowed_actions.len(), AllowedAction::ALL.len());
    }

    #[test]
    fn control_action_rejects_stale_snapshot() {
        let h = host(ComputerUseSettings {
            enabled: true,
            allow_input_injection: true,
            allow_foreground_takeover: false,
        });
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();

        let first = h.get_app_state("s1", &p).unwrap();
        let stale = SnapshotId(first.snapshot_id.clone());
        // Capture again → `stale` is now outdated.
        let fresh = h.get_app_state("s1", &p).unwrap();

        assert!(matches!(
            h.type_text("s1", &stale, "hi", &p),
            Err(ComputerUseError::Snapshot(SnapshotError::Stale))
        ));
        // Fresh snapshot works.
        h.type_text("s1", &SnapshotId(fresh.snapshot_id), "hi", &p)
            .unwrap();
    }

    #[test]
    fn control_blocked_when_injection_disabled_in_settings() {
        let h = host(ComputerUseSettings::observe_only()); // injection off
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        let snap = SnapshotId(state.snapshot_id);
        assert!(matches!(
            h.type_text("s1", &snap, "hi", &p),
            Err(ComputerUseError::ControlDisabled)
        ));
    }

    #[test]
    fn control_blocked_without_control_permission() {
        let h = host(ComputerUseSettings {
            enabled: true,
            allow_input_injection: true,
            allow_foreground_takeover: false,
        });
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        // inject=false → no control permission.
        let p = perm(true, true, false);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        let snap = SnapshotId(state.snapshot_id);
        assert!(matches!(
            h.press_key("s1", &snap, "Enter", &p),
            Err(ComputerUseError::Permission(PermissionDenied::Control))
        ));
    }

    #[test]
    fn stop_releases_lease_and_is_idempotent() {
        let h = host(ComputerUseSettings::observe_only());
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        let p = perm(true, true, false);
        h.start("s1", target(), &p).unwrap();
        assert!(h.leases.has_lease("s1"));
        h.stop("s1");
        assert!(!h.leases.has_lease("s1"));
        h.stop("s1"); // no panic
    }

    #[test]
    fn successful_click_reaches_provider() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(
            provider.clone(),
            ComputerUseSettings {
                enabled: true,
                allow_input_injection: true,
                allow_foreground_takeover: false,
            },
        );
        h.approvals().approve_session("s1");
        h.approvals().approve_app("s1", &target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        let snap = SnapshotId(state.snapshot_id);
        h.click_element("s1", &snap, "el-1", &p).unwrap();
        assert_eq!(
            provider.actions(),
            vec!["click:com.example.app:el-1".to_string()]
        );
    }

    #[test]
    fn status_reports_capabilities_without_failing() {
        let h = host(ComputerUseSettings::observe_only());
        let p = perm(true, false, false);
        let s = h.status("s1", &p);
        assert!(s.enabled);
        assert!(!s.session_approved);
        assert!(s.can_observe);
        assert!(!s.can_inspect);
        assert!(!s.can_control);
    }

    #[test]
    fn platform_provider_host_constructs() {
        // The platform provider host is the constructor the Phase 4 injection
        // layer uses. On non-macOS it wraps the unsupported stub; either way
        // construction must succeed and stay disabled by default policy.
        let h = ComputerUseHost::with_platform_provider(ComputerUseSettings::default());
        let p = perm(false, false, false);
        let s = h.status("s1", &p);
        assert!(!s.enabled);
    }
}
