//! Computer-use host: orchestration.
//!
//! The host is the agent-agnostic entry point the injected tools call into. It
//! enforces the full gating chain for every operation:
//!
//! 1. **settings** — global enable policy,
//! 2. **OS permission** — observe / inspect / control tiers,
//! 3. **approval** — session approval + per-app approval,
//! 4. **lease + snapshot** — one app at a time, act only on the latest snapshot,
//!
//! then dispatches to the [`ComputerUseProvider`]. The provider performs the
//! privileged work (Phase 3); the host owns the policy.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use crate::desktop_control::DesktopControlPermissionStatus;

use super::approvals::{ApprovalDecision, ApprovalRegistry};
use super::lease::{LeaseError, LeaseRegistry, SnapshotError, SnapshotId};
use super::permissions::{self, PermissionDenied, RequiredCapability};
use super::provider::{
    AllowedAction, AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppState, AppTarget,
    ComputerUseProvider, CoordinateSpace, InstalledApp, Point, ProviderError, ScreenshotRef,
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
    #[error("coordinate error: {0}")]
    Coordinate(String),
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
}

/// The host. Cheap to clone (`Arc` internals) so it can be shared across the
/// injection layer and Tauri commands.
#[derive(Clone)]
pub struct ComputerUseHost {
    provider: Arc<dyn ComputerUseProvider>,
    leases: Arc<LeaseRegistry>,
    approvals: Arc<ApprovalRegistry>,
    settings: Arc<RwLock<ComputerUseSettings>>,
    /// Sessions whose most recent computer-use action was blocked on app
    /// approval before a lease could be established. This lets the chat overlay
    /// still tell the user which app needs approval.
    pending_app_approvals: Arc<Mutex<HashMap<String, AppId>>>,
    /// Sessions currently performing a foreground takeover (an agent is actively
    /// driving input). Drives the takeover warning overlay + abort affordance.
    foreground: Arc<Mutex<HashSet<String>>>,
}

impl ComputerUseHost {
    pub fn new(provider: Arc<dyn ComputerUseProvider>, settings: ComputerUseSettings) -> Self {
        Self {
            provider,
            leases: Arc::new(LeaseRegistry::new()),
            approvals: Arc::new(ApprovalRegistry::new()),
            settings: Arc::new(RwLock::new(settings)),
            pending_app_approvals: Arc::new(Mutex::new(HashMap::new())),
            foreground: Arc::new(Mutex::new(HashSet::new())),
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

    pub fn settings(&self) -> ComputerUseSettings {
        self.settings.read().unwrap().clone()
    }

    pub fn update_settings(&self, settings: ComputerUseSettings) {
        *self.settings.write().unwrap() = settings;
    }

    fn require_enabled(&self) -> Result<(), ComputerUseError> {
        if self.settings().enabled {
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

    fn require_approval(&self, session_id: &str, app_id: &str) -> Result<(), ComputerUseError> {
        match self.approvals.decide(session_id, &app_id.to_string()) {
            ApprovalDecision::Allowed => {
                self.clear_pending_app_approval(session_id);
                Ok(())
            }
            ApprovalDecision::AppNotApproved => {
                self.note_pending_app_approval(session_id, app_id);
                Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
            }
            other => Err(ComputerUseError::Approval(other)),
        }
    }

    fn require_session_approval(&self, session_id: &str) -> Result<(), ComputerUseError> {
        if self.approvals.session_approved(session_id) {
            Ok(())
        } else {
            Err(ComputerUseError::Approval(
                ApprovalDecision::SessionNotApproved,
            ))
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
        let active_app_id = self
            .leases
            .target(session_id)
            .ok()
            .map(|target| target.app_id)
            .or_else(|| self.pending_app_approval(session_id));
        let active_app_approved = active_app_id
            .as_ref()
            .map(|app_id| self.approvals.app_approved(app_id))
            .unwrap_or(false);
        let settings = self.settings();
        ComputerUseStatus {
            enabled: settings.enabled,
            session_approved: self.approvals.session_approved(session_id),
            has_lease: self.leases.has_lease(session_id),
            can_observe: settings.enabled && perm.can_observe,
            can_inspect: settings.enabled && perm.can_inspect,
            can_control: settings.enabled && perm.can_control && self.provider.supports_control(),
            foreground_active: self.foreground_active(session_id),
            active_app_id,
            active_app_approved,
        }
    }

    /// `computer_list_apps` — enumerate targetable apps. Requires the feature on
    /// and observe permission.
    pub fn list_apps(
        &self,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<Vec<InstalledApp>, ComputerUseError> {
        self.list_apps_with_options(perm, AppListOptions::default())
    }

    /// `computer_list_apps` with provider-specific discovery options such as a
    /// recent-use ranking window.
    pub fn list_apps_with_options(
        &self,
        perm: &DesktopControlPermissionStatus,
        options: AppListOptions,
    ) -> Result<Vec<InstalledApp>, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        Ok(self.provider.list_apps(options)?)
    }

    /// `computer_start` — open a lease on a target app. Requires the feature on,
    /// observe permission, and session approval. App approval is deferred to
    /// control actions so observe/inspect remain usable in the current
    /// observe-first rollout.
    pub fn start(
        &self,
        session_id: &str,
        target: AppTarget,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<String, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        self.require_session_approval(session_id)?;
        Ok(self.leases.open(session_id, target)?)
    }

    /// `computer_launch_app` — launch a target app without activating it,
    /// opening a lease if the session does not already have one for that app.
    pub fn launch_app(
        &self,
        session_id: &str,
        target: AppTarget,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppLaunchResult, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        self.require_session_approval(session_id)?;
        self.require_approval(session_id, &target.app_id)?;
        let needs_lease = self.require_compatible_lease(session_id, &target)?;
        let result = self.provider.launch_app(&target)?;
        if needs_lease {
            self.leases.open(session_id, target)?;
        }
        Ok(result)
    }

    /// `computer_raise_app` — bring a target app/window to the foreground,
    /// restoring minimized or hidden windows when the platform can.
    pub fn raise_app(
        &self,
        session_id: &str,
        target: AppTarget,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppRaiseResult, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        self.require_session_approval(session_id)?;
        self.require_approval(session_id, &target.app_id)?;
        let needs_lease = self.require_compatible_lease(session_id, &target)?;
        let result = self.provider.raise_app(&target)?;
        if needs_lease {
            self.leases.open(session_id, target)?;
        }
        self.begin_foreground(session_id);
        Ok(result)
    }

    /// `computer_get_app_state` — capture screenshot + elements, stamp a fresh
    /// snapshot id, and report the allowed actions. Requires observe; inspection
    /// elements are only included when inspect permission is present.
    pub fn get_app_state(
        &self,
        session_id: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.get_app_state_inner(session_id, None, perm)
    }

    /// `computer_get_app_state` with an explicit target. If the app is not
    /// running, the host launches it only after the same app approval required
    /// by the explicit launch/control paths.
    pub fn get_app_state_for_target(
        &self,
        session_id: &str,
        target: AppTarget,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.get_app_state_inner(session_id, Some(target), perm)
    }

    fn get_app_state_inner(
        &self,
        session_id: &str,
        target: Option<AppTarget>,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.require_enabled()?;
        self.require_permission(perm, RequiredCapability::Observe)?;
        let (target, needs_lease) = match target {
            Some(target) => {
                self.require_session_approval(session_id)?;
                let needs_lease = self.require_compatible_lease(session_id, &target)?;
                (target, needs_lease)
            }
            None => (self.leases.target(session_id)?, false),
        };
        let launched = if self.provider.is_app_running(&target.app_id)? {
            false
        } else {
            self.require_approval(session_id, &target.app_id)?;
            self.provider.launch_app(&target)?.launched
        };
        if needs_lease {
            self.leases.open(session_id, target.clone())?;
        }
        let raw = self.provider.capture_app_state(&target)?;
        let snapshot = self.next_snapshot(session_id, raw.screenshot.clone())?;

        Ok(self.app_state_from_raw(snapshot, raw, perm, launched))
    }

    fn next_snapshot(
        &self,
        session_id: &str,
        screenshot: ScreenshotRef,
    ) -> Result<SnapshotId, ComputerUseError> {
        Ok(self
            .leases
            .with_lease(session_id, |lease| lease.next_snapshot(screenshot))?)
    }

    fn capture_post_action_state(
        &self,
        session_id: &str,
        target: &AppTarget,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let raw = self.provider.capture_app_state(target)?;
        let snapshot = self.next_snapshot(session_id, raw.screenshot.clone())?;
        Ok(self.app_state_from_raw(snapshot, raw, perm, false))
    }

    fn app_state_from_raw(
        &self,
        snapshot: SnapshotId,
        mut raw: super::provider::RawAppState,
        perm: &DesktopControlPermissionStatus,
        launched: bool,
    ) -> AppState {
        // Inspection is a separate tier: without it, do not expose the AX tree
        // and do not allow element-targeted actions.
        let can_inspect = perm.can_inspect;
        if !can_inspect {
            raw.elements.clear();
        }

        let allowed_actions = self.allowed_actions(perm, can_inspect);

        AppState {
            snapshot_id: snapshot.0,
            target: raw.target,
            launched,
            display: raw.display,
            screenshot: raw.screenshot,
            elements: raw.elements,
            allowed_actions,
        }
    }

    fn allowed_actions(
        &self,
        perm: &DesktopControlPermissionStatus,
        can_inspect: bool,
    ) -> Vec<AllowedAction> {
        let control_ready = perm.can_control && self.provider.supports_control();
        if !control_ready {
            return Vec::new();
        }
        AllowedAction::ALL
            .into_iter()
            .filter(|action| match action {
                // Element-targeted actions require an inspected element tree.
                AllowedAction::ClickElement | AllowedAction::SetValue => can_inspect,
                _ => true,
            })
            .collect()
    }

    fn require_compatible_lease(
        &self,
        session_id: &str,
        target: &AppTarget,
    ) -> Result<bool, ComputerUseError> {
        match self.leases.target(session_id) {
            Ok(existing) if existing == *target => Ok(false),
            Ok(_) => Err(ComputerUseError::Lease(LeaseError::AlreadyLeased)),
            Err(LeaseError::NoLease) => Ok(true),
            Err(err) => Err(ComputerUseError::Lease(err)),
        }
    }

    fn require_control_target(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppTarget, ComputerUseError> {
        self.require_enabled()?;
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

    fn snapshot_screenshot(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
    ) -> Result<ScreenshotRef, ComputerUseError> {
        let screenshot = self
            .leases
            .with_lease(session_id, |lease| lease.screenshot_for_snapshot(snapshot))??;
        Ok(screenshot)
    }

    fn resolve_point_for_snapshot(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        point: Point,
        coordinate_space: CoordinateSpace,
    ) -> Result<Point, ComputerUseError> {
        let screenshot = self.snapshot_screenshot(session_id, snapshot)?;
        screenshot
            .resolve_point(point, coordinate_space)
            .map_err(ComputerUseError::Coordinate)
    }

    fn require_control(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppTarget, ComputerUseError> {
        let target = self.require_control_target(session_id, snapshot, perm)?;
        // A control action is about to run: the agent is driving input, so the
        // session is in foreground takeover until it stops/aborts. This is what
        // the takeover warning overlay observes.
        self.begin_foreground(session_id);
        Ok(target)
    }

    pub fn resolve_coordinate_action_point(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        point: Point,
        coordinate_space: CoordinateSpace,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<(AppTarget, Point), ComputerUseError> {
        let target = self.require_control_target(session_id, snapshot, perm)?;
        let screen_point =
            self.resolve_point_for_snapshot(session_id, snapshot, point, coordinate_space)?;
        self.begin_foreground(session_id);
        Ok((target, screen_point))
    }

    /// `computer_click_element` — click an inspected element. Requires control +
    /// inspect and a fresh snapshot.
    pub fn click_element(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        element_id: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.require_permission(perm, RequiredCapability::Inspect)?;
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider
            .click_element(&target, &element_id.to_string())?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_click_at` — click a point in the latest snapshot's screenshot
    /// coordinate space by default.
    pub fn click_at(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        point: Point,
        coordinate_space: CoordinateSpace,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let (target, screen_point) = self.resolve_coordinate_action_point(
            session_id,
            snapshot,
            point,
            coordinate_space,
            perm,
        )?;
        self.provider.click_point(&target, screen_point)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_secondary_click` — right/secondary click a point in the latest
    /// snapshot's screenshot coordinate space by default.
    pub fn secondary_click(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        point: Point,
        coordinate_space: CoordinateSpace,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let (target, screen_point) = self.resolve_coordinate_action_point(
            session_id,
            snapshot,
            point,
            coordinate_space,
            perm,
        )?;
        self.provider.secondary_click(&target, screen_point)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_perform_secondary_action` / ref-targeted secondary click —
    /// prefer AXShowMenu over coordinate right-click when the element is known.
    pub fn secondary_click_element(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        element_id: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.require_permission(perm, RequiredCapability::Inspect)?;
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider
            .secondary_click_element(&target, &element_id.to_string())?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_double_click` — double click a point in the latest snapshot's
    /// screenshot coordinate space by default.
    pub fn double_click(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        point: Point,
        coordinate_space: CoordinateSpace,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let (target, screen_point) = self.resolve_coordinate_action_point(
            session_id,
            snapshot,
            point,
            coordinate_space,
            perm,
        )?;
        self.provider.double_click(&target, screen_point)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_drag` — drag between two points in the latest snapshot's
    /// screenshot coordinate space by default.
    pub fn drag(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        from: Point,
        to: Point,
        coordinate_space: CoordinateSpace,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let target = self.require_control_target(session_id, snapshot, perm)?;
        let screen_from =
            self.resolve_point_for_snapshot(session_id, snapshot, from, coordinate_space)?;
        let screen_to =
            self.resolve_point_for_snapshot(session_id, snapshot, to, coordinate_space)?;
        self.begin_foreground(session_id);
        self.provider.drag(&target, screen_from, screen_to)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_set_value` — set an inspected element's value directly.
    pub fn set_value(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        element_id: &str,
        value: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.require_permission(perm, RequiredCapability::Inspect)?;
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider
            .set_value(&target, &element_id.to_string(), value)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_type_text` — type text into the focused element.
    pub fn type_text(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        text: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider.type_text(&target, text)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_press_key` — press a key / chord.
    pub fn press_key(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        key: &str,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider.press_key(&target, key)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_scroll` — scroll the target.
    pub fn scroll(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        direction: ScrollDirection,
        amount: i32,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider.scroll(&target, direction, amount)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// Ref-targeted scroll — uses AX scroll actions when available, with the
    /// existing wheel path left as the coordinate-less fallback.
    pub fn scroll_element(
        &self,
        session_id: &str,
        snapshot: &SnapshotId,
        element_id: &str,
        direction: ScrollDirection,
        amount: i32,
        perm: &DesktopControlPermissionStatus,
    ) -> Result<AppState, ComputerUseError> {
        self.require_permission(perm, RequiredCapability::Inspect)?;
        let target = self.require_control(session_id, snapshot, perm)?;
        self.provider
            .scroll_element(&target, &element_id.to_string(), direction, amount)?;
        self.capture_post_action_state(session_id, &target, perm)
    }

    /// `computer_stop` — release the session's lease. Idempotent.
    pub fn stop(&self, session_id: &str) {
        self.leases.close(session_id);
        self.end_foreground(session_id);
        self.clear_pending_app_approval(session_id);
    }

    // --- Foreground takeover + abort ------------------------------------

    fn begin_foreground(&self, session_id: &str) {
        self.foreground
            .lock()
            .unwrap()
            .insert(session_id.to_string());
    }

    fn end_foreground(&self, session_id: &str) {
        self.foreground.lock().unwrap().remove(session_id);
    }

    /// Whether the session is currently in a foreground takeover.
    pub fn foreground_active(&self, session_id: &str) -> bool {
        self.foreground.lock().unwrap().contains(session_id)
    }

    /// Reliable cancel path for the takeover overlay: end the foreground
    /// takeover and release the lease. Idempotent; safe to call when nothing is
    /// active.
    pub fn abort(&self, session_id: &str) {
        self.end_foreground(session_id);
        self.leases.close(session_id);
        self.clear_pending_app_approval(session_id);
    }

    /// Clear the app-approval hint for a session once the user has responded or
    /// the session has moved on to a different target.
    pub fn clear_pending_app_approval(&self, session_id: &str) {
        self.pending_app_approvals
            .lock()
            .unwrap()
            .remove(session_id);
    }

    fn note_pending_app_approval(&self, session_id: &str, app_id: &str) {
        self.pending_app_approvals
            .lock()
            .unwrap()
            .insert(session_id.to_string(), app_id.to_string());
    }

    fn pending_app_approval(&self, session_id: &str) -> Option<AppId> {
        self.pending_app_approvals
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
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
    /// True while the session is actively driving input (foreground takeover).
    pub foreground_active: bool,
    /// The app currently leased for control, when the session has an active
    /// target. When a launch/raise/state request is blocked on app approval
    /// before a lease exists, this falls back to that pending target so chat
    /// can still show the approval affordance.
    pub active_app_id: Option<String>,
    /// Whether the currently leased app has been approved for this session.
    pub active_app_approved: bool,
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

    fn host_with_apps(apps: Vec<InstalledApp>) -> (ComputerUseHost, Arc<FakeProvider>) {
        let provider = Arc::new(FakeProvider::with_apps(apps));
        (
            ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled()),
            provider,
        )
    }

    fn installed_app(running: bool) -> InstalledApp {
        InstalledApp {
            id: "com.example.installed".into(),
            name: "Installed".into(),
            pid: running.then_some(4321),
            running,
            recent_use_count: None,
            recent_last_used_at: None,
            recent_source: None,
        }
    }

    fn installed_target() -> AppTarget {
        AppTarget {
            app_id: "com.example.installed".into(),
            window_id: None,
        }
    }

    #[test]
    fn disabled_host_refuses_everything() {
        let h = host(ComputerUseSettings::default());
        let p = perm(true, true, true);
        let status = h.status("s1", &p);
        assert!(!status.enabled);
        assert!(!status.can_observe);
        assert!(!status.can_inspect);
        assert!(!status.can_control);
        assert!(matches!(h.list_apps(&p), Err(ComputerUseError::Disabled)));
        assert!(matches!(
            h.start("s1", target(), &p),
            Err(ComputerUseError::Disabled)
        ));
    }

    #[test]
    fn start_requires_session_approval_only() {
        let h = host(ComputerUseSettings::enabled());
        let p = perm(true, true, false);

        // No approval at all.
        assert!(matches!(
            h.start("s1", target(), &p),
            Err(ComputerUseError::Approval(
                ApprovalDecision::SessionNotApproved
            ))
        ));

        // Session approval is enough to begin observe/inspect.
        h.approvals().approve_session("s1");
        let lease = h.start("s1", target(), &p).unwrap();
        assert!(lease.starts_with("lease-s1-"));
    }

    #[test]
    fn list_apps_forwards_recent_window_to_provider() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        let p = perm(true, true, false);

        h.list_apps_with_options(&p, AppListOptions { days: Some(7) })
            .unwrap();

        assert_eq!(provider.actions(), vec!["list_apps:7".to_string()]);
    }

    #[test]
    fn control_still_requires_app_approval_after_start() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        let snap = SnapshotId(state.snapshot_id);
        let status = h.status("s1", &p);
        assert_eq!(status.active_app_id.as_deref(), Some("com.example.app"));
        assert!(!status.active_app_approved);

        assert!(matches!(
            h.type_text("s1", &snap, "hi", &p),
            Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
        ));

        h.approvals().approve_app(&target().app_id);
        assert!(h.status("s1", &p).active_app_approved);
    }

    #[test]
    fn launch_app_requires_app_approval_and_opens_lease() {
        let (h, provider) = host_with_apps(vec![installed_app(false)]);
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);

        assert!(matches!(
            h.launch_app("s1", installed_target(), &p),
            Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
        ));
        assert!(provider.actions().is_empty());
        assert!(!h.leases.has_lease("s1"));

        h.approvals().approve_app(&installed_target().app_id);
        let result = h.launch_app("s1", installed_target(), &p).unwrap();
        assert!(result.launched);
        assert!(result.running);
        assert!(h.leases.has_lease("s1"));
        assert_eq!(
            provider.actions(),
            vec!["launch:com.example.installed".to_string()]
        );
    }

    #[test]
    fn raise_app_requires_app_approval_opens_lease_and_marks_foreground() {
        let (h, provider) = host_with_apps(vec![installed_app(true)]);
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);

        assert!(matches!(
            h.raise_app("s1", installed_target(), &p),
            Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
        ));
        assert!(provider.actions().is_empty());
        assert!(!h.leases.has_lease("s1"));
        assert!(!h.foreground_active("s1"));

        h.approvals().approve_app(&installed_target().app_id);
        let result = h.raise_app("s1", installed_target(), &p).unwrap();
        assert!(!result.launched);
        assert!(result.running);
        assert!(result.activated);
        assert!(result.visible);
        assert!(h.leases.has_lease("s1"));
        assert!(h.foreground_active("s1"));
        assert_eq!(
            provider.actions(),
            vec!["raise:com.example.installed".to_string()]
        );
    }

    #[test]
    fn get_app_state_launches_stopped_target_only_after_app_approval() {
        let (h, provider) = host_with_apps(vec![installed_app(false)]);
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);

        assert!(matches!(
            h.get_app_state_for_target("s1", installed_target(), &p),
            Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
        ));
        assert!(provider.actions().is_empty());
        assert!(!h.leases.has_lease("s1"));

        h.approvals().approve_app(&installed_target().app_id);
        let state = h
            .get_app_state_for_target("s1", installed_target(), &p)
            .unwrap();
        assert!(state.launched);
        assert!(h.leases.has_lease("s1"));
        assert_eq!(
            provider.actions(),
            vec!["launch:com.example.installed".to_string()]
        );
    }

    #[test]
    fn get_app_state_for_running_target_does_not_require_app_approval() {
        let (h, provider) = host_with_apps(vec![installed_app(true)]);
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);

        let state = h
            .get_app_state_for_target("s1", installed_target(), &p)
            .unwrap();
        assert!(!state.launched);
        assert!(h.leases.has_lease("s1"));
        assert!(provider.actions().is_empty());
    }

    #[test]
    fn get_app_state_without_inspect_hides_elements_and_click() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
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
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        assert!(!state.elements.is_empty());
        assert_eq!(state.allowed_actions.len(), AllowedAction::ALL.len());
    }

    #[test]
    fn control_action_rejects_stale_snapshot() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
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
        // Fresh snapshot works and returns the new authoritative post-action
        // state. The pre-action snapshot is immediately stale afterwards.
        let post_action = h
            .type_text("s1", &SnapshotId(fresh.snapshot_id.clone()), "hi", &p)
            .unwrap();
        assert_ne!(post_action.snapshot_id, fresh.snapshot_id);
        assert!(matches!(
            h.press_key("s1", &SnapshotId(fresh.snapshot_id), "Enter", &p),
            Err(ComputerUseError::Snapshot(SnapshotError::Stale))
        ));
        h.press_key("s1", &SnapshotId(post_action.snapshot_id), "Enter", &p)
            .unwrap();
    }

    #[test]
    fn coordinate_action_resolution_uses_latest_snapshot_mapping() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();

        let first = h.get_app_state("s1", &p).unwrap();
        let stale = SnapshotId(first.snapshot_id);
        let fresh = h.get_app_state("s1", &p).unwrap();

        assert!(matches!(
            h.resolve_coordinate_action_point(
                "s1",
                &stale,
                Point { x: 360.0, y: 225.0 },
                CoordinateSpace::Screenshot,
                &p
            ),
            Err(ComputerUseError::Snapshot(SnapshotError::Stale))
        ));

        let (resolved_target, screen_point) = h
            .resolve_coordinate_action_point(
                "s1",
                &SnapshotId(fresh.snapshot_id),
                Point { x: 360.0, y: 225.0 },
                CoordinateSpace::Screenshot,
                &p,
            )
            .unwrap();

        assert_eq!(resolved_target, target());
        assert_eq!(screen_point, Point { x: 190.0, y: 132.5 });
        assert!(h.foreground_active("s1"));
    }

    #[test]
    fn enabled_computer_use_allows_control_when_permissions_are_ready() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        let snap = SnapshotId(state.snapshot_id);
        h.type_text("s1", &snap, "hi", &p).unwrap();
    }

    #[test]
    fn control_blocked_without_control_permission() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
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
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
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
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
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
    fn coordinate_actions_reach_provider_with_screen_points() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();

        let state = h.get_app_state("s1", &p).unwrap();
        let post_click = h
            .click_at(
                "s1",
                &SnapshotId(state.snapshot_id),
                Point { x: 360.0, y: 225.0 },
                CoordinateSpace::Screenshot,
                &p,
            )
            .unwrap();

        h.drag(
            "s1",
            &SnapshotId(post_click.snapshot_id),
            Point { x: 0.0, y: 0.0 },
            Point { x: 720.0, y: 450.0 },
            CoordinateSpace::Screenshot,
            &p,
        )
        .unwrap();

        assert_eq!(
            provider.actions(),
            vec![
                "click_at:com.example.app:190.0,132.5".to_string(),
                "drag:com.example.app:10.0,20.0->370.0,245.0".to_string(),
            ]
        );
    }

    #[test]
    fn set_value_reaches_provider() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        h.set_value("s1", &SnapshotId(state.snapshot_id), "el-1", "42", &p)
            .unwrap();

        assert_eq!(
            provider.actions(),
            vec!["set_value:com.example.app:el-1:42".to_string()]
        );
    }

    #[test]
    fn element_secondary_and_scroll_reach_provider() {
        let provider = Arc::new(FakeProvider::default());
        let h = ComputerUseHost::new(provider.clone(), ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();

        let post_secondary = h
            .secondary_click_element("s1", &SnapshotId(state.snapshot_id), "el-1", &p)
            .unwrap();
        h.scroll_element(
            "s1",
            &SnapshotId(post_secondary.snapshot_id),
            "el-1",
            ScrollDirection::Down,
            300,
            &p,
        )
        .unwrap();

        assert_eq!(
            provider.actions(),
            vec![
                "secondary_click_element:com.example.app:el-1".to_string(),
                "scroll_element:com.example.app:el-1:Down:300".to_string(),
            ]
        );
    }

    #[test]
    fn status_reports_capabilities_without_failing() {
        let h = host(ComputerUseSettings::enabled());
        let p = perm(true, false, false);
        let s = h.status("s1", &p);
        assert!(s.enabled);
        assert!(!s.session_approved);
        assert_eq!(s.active_app_id, None);
        assert!(s.can_observe);
        assert!(!s.can_inspect);
        assert!(!s.can_control);
    }

    #[test]
    fn status_includes_active_app_when_session_holds_lease() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, false);
        h.start("s1", target(), &p).unwrap();

        let status = h.status("s1", &p);
        assert_eq!(status.active_app_id.as_deref(), Some("com.example.app"));
    }

    #[test]
    fn status_surfaces_pending_app_when_launch_is_blocked_on_approval() {
        let (h, provider) = host_with_apps(vec![installed_app(false)]);
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);

        assert!(matches!(
            h.launch_app("s1", installed_target(), &p),
            Err(ComputerUseError::Approval(ApprovalDecision::AppNotApproved))
        ));
        assert!(provider.actions().is_empty());

        let status = h.status("s1", &p);
        assert!(!status.has_lease);
        assert_eq!(
            status.active_app_id.as_deref(),
            Some("com.example.installed")
        );
        assert!(!status.active_app_approved);
    }

    #[test]
    fn stop_clears_pending_app_approval_hint() {
        let (h, _provider) = host_with_apps(vec![installed_app(false)]);
        h.approvals().approve_session("s1");
        let p = perm(true, true, true);

        let _ = h.launch_app("s1", installed_target(), &p);
        assert_eq!(
            h.status("s1", &p).active_app_id.as_deref(),
            Some("com.example.installed")
        );

        h.stop("s1");
        assert_eq!(h.status("s1", &p).active_app_id, None);
    }

    #[test]
    fn control_action_marks_foreground_and_abort_clears_it() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        let snap = SnapshotId(state.snapshot_id);

        assert!(!h.foreground_active("s1"));
        h.type_text("s1", &snap, "hi", &p).unwrap();
        assert!(
            h.foreground_active("s1"),
            "control action enters foreground"
        );
        assert!(h.status("s1", &p).foreground_active);

        // Abort clears foreground and releases the lease; idempotent.
        h.abort("s1");
        assert!(!h.foreground_active("s1"));
        assert!(!h.leases.has_lease("s1"));
        h.abort("s1"); // no panic
    }

    #[test]
    fn stop_also_clears_foreground() {
        let h = host(ComputerUseSettings::enabled());
        h.approvals().approve_session("s1");
        h.approvals().approve_app(&target().app_id);
        let p = perm(true, true, true);
        h.start("s1", target(), &p).unwrap();
        let state = h.get_app_state("s1", &p).unwrap();
        h.press_key("s1", &SnapshotId(state.snapshot_id), "Enter", &p)
            .unwrap();
        assert!(h.foreground_active("s1"));
        h.stop("s1");
        assert!(!h.foreground_active("s1"));
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
