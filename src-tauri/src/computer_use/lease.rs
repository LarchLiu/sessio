//! Lease lifecycle and snapshot staleness.
//!
//! A *lease* is a session's open claim on a target app/window. Within a lease,
//! every `get_app_state` produces a *snapshot* with a monotonically increasing
//! id; actions must reference the latest snapshot id. This prevents the model
//! acting against a stale view (the classic computer-use race where the UI
//! changed between observe and act).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::provider::{AppTarget, ScreenshotRef, UiElement};

/// A per-lease snapshot identifier. Encodes the lease and a monotonic counter so
/// staleness is checkable by string comparison and never collides across leases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotId(pub String);

impl SnapshotId {
    fn new(lease_id: &str, counter: u64) -> Self {
        SnapshotId(format!("{lease_id}#{counter}"))
    }
}

/// Reasons a snapshot reference is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotError {
    #[error("no snapshot has been captured for this lease yet")]
    NoSnapshot,
    #[error("snapshot is stale; capture a fresh app state before acting")]
    Stale,
    #[error("snapshot does not belong to this lease")]
    WrongLease,
}

/// One session's lease on a target.
#[derive(Debug, Clone)]
pub struct Lease {
    pub id: String,
    pub session_id: String,
    pub target: AppTarget,
    snapshot_counter: u64,
    latest_snapshot: Option<SnapshotId>,
    latest_screenshot: Option<ScreenshotRef>,
    latest_elements: Vec<UiElement>,
}

impl Lease {
    fn new(id: String, session_id: String, target: AppTarget) -> Self {
        Self {
            id,
            session_id,
            target,
            snapshot_counter: 0,
            latest_snapshot: None,
            latest_screenshot: None,
            latest_elements: Vec::new(),
        }
    }

    /// Stamp a new snapshot, making it the only currently-valid one.
    pub fn next_snapshot(
        &mut self,
        screenshot: ScreenshotRef,
        elements: Vec<UiElement>,
    ) -> SnapshotId {
        self.snapshot_counter += 1;
        let id = SnapshotId::new(&self.id, self.snapshot_counter);
        self.latest_snapshot = Some(id.clone());
        self.latest_screenshot = Some(screenshot);
        self.latest_elements = elements;
        id
    }

    pub fn latest_snapshot(&self) -> Option<&SnapshotId> {
        self.latest_snapshot.as_ref()
    }

    /// Validate that `candidate` is the lease's current snapshot.
    pub fn check_snapshot(&self, candidate: &SnapshotId) -> Result<(), SnapshotError> {
        let latest = self
            .latest_snapshot
            .as_ref()
            .ok_or(SnapshotError::NoSnapshot)?;
        if !candidate.0.starts_with(&format!("{}#", self.id)) {
            return Err(SnapshotError::WrongLease);
        }
        if candidate != latest {
            return Err(SnapshotError::Stale);
        }
        Ok(())
    }

    /// Return the host-recorded screenshot metadata for the latest snapshot.
    pub fn screenshot_for_snapshot(
        &self,
        candidate: &SnapshotId,
    ) -> Result<ScreenshotRef, SnapshotError> {
        self.check_snapshot(candidate)?;
        self.latest_screenshot
            .clone()
            .ok_or(SnapshotError::NoSnapshot)
    }

    /// Return an inspected element from the exact snapshot the agent is acting
    /// against. The host stores only elements that were exposed to the agent.
    pub fn element_for_snapshot(
        &self,
        candidate: &SnapshotId,
        element_id: &str,
    ) -> Result<Option<UiElement>, SnapshotError> {
        self.check_snapshot(candidate)?;
        Ok(self
            .latest_elements
            .iter()
            .find(|element| element.id == element_id)
            .cloned())
    }
}

/// Errors from lease bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    #[error("no active computer-use lease for this session")]
    NoLease,
    #[error("session already holds an active lease")]
    AlreadyLeased,
}

/// Holds at most one lease per session. A session must `stop` before opening a
/// new lease, keeping the "one app under control at a time" invariant.
#[derive(Default)]
pub struct LeaseRegistry {
    inner: Mutex<HashMap<String, Lease>>,
    counter: Mutex<u64>,
}

impl LeaseRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_lease_id(&self, session_id: &str) -> String {
        let mut counter = self.counter.lock().unwrap();
        *counter += 1;
        format!("lease-{session_id}-{}", *counter)
    }

    /// Open a lease for a session against a target. Fails if one already exists.
    pub fn open(&self, session_id: &str, target: AppTarget) -> Result<String, LeaseError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.contains_key(session_id) {
            return Err(LeaseError::AlreadyLeased);
        }
        let lease_id = self.next_lease_id(session_id);
        inner.insert(
            session_id.to_string(),
            Lease::new(lease_id.clone(), session_id.to_string(), target),
        );
        Ok(lease_id)
    }

    pub fn has_lease(&self, session_id: &str) -> bool {
        self.inner.lock().unwrap().contains_key(session_id)
    }

    /// Run `f` against the session's lease (mutably), e.g. to stamp a snapshot.
    pub fn with_lease<T>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut Lease) -> T,
    ) -> Result<T, LeaseError> {
        let mut inner = self.inner.lock().unwrap();
        let lease = inner.get_mut(session_id).ok_or(LeaseError::NoLease)?;
        Ok(f(lease))
    }

    /// Close the session's lease. Idempotent: closing a missing lease is Ok.
    pub fn close(&self, session_id: &str) {
        self.inner.lock().unwrap().remove(session_id);
    }

    pub fn target(&self, session_id: &str) -> Result<AppTarget, LeaseError> {
        self.inner
            .lock()
            .unwrap()
            .get(session_id)
            .map(|lease| lease.target.clone())
            .ok_or(LeaseError::NoLease)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::computer_use::provider::{CoordinateSpace, Rect};

    fn target() -> AppTarget {
        AppTarget {
            app_id: "com.example.app".into(),
            window_id: None,
        }
    }

    fn screenshot() -> ScreenshotRef {
        ScreenshotRef {
            handle: "snap".into(),
            format: "png".into(),
            byte_len: 1,
            width: 100,
            height: 50,
            default_coordinate_space: CoordinateSpace::Screenshot,
            screen_bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 25.0,
            },
        }
    }

    #[test]
    fn open_close_is_one_lease_per_session() {
        let reg = LeaseRegistry::new();
        assert!(!reg.has_lease("s1"));
        reg.open("s1", target()).unwrap();
        assert!(reg.has_lease("s1"));
        assert_eq!(reg.open("s1", target()), Err(LeaseError::AlreadyLeased));
        reg.close("s1");
        assert!(!reg.has_lease("s1"));
        // Closing again is fine.
        reg.close("s1");
    }

    #[test]
    fn snapshot_staleness_rejects_old_ids() {
        let reg = LeaseRegistry::new();
        reg.open("s1", target()).unwrap();

        // No snapshot yet.
        let latest = reg
            .with_lease("s1", |l| l.latest_snapshot().cloned())
            .unwrap();
        assert!(latest.is_none());

        let first = reg
            .with_lease("s1", |l| l.next_snapshot(screenshot(), Vec::new()))
            .unwrap();
        reg.with_lease("s1", |l| l.check_snapshot(&first))
            .unwrap()
            .unwrap();

        // Capture again → first becomes stale.
        let second = reg
            .with_lease("s1", |l| l.next_snapshot(screenshot(), Vec::new()))
            .unwrap();
        assert_eq!(
            reg.with_lease("s1", |l| l.check_snapshot(&first)).unwrap(),
            Err(SnapshotError::Stale)
        );
        reg.with_lease("s1", |l| l.check_snapshot(&second))
            .unwrap()
            .unwrap();
    }

    #[test]
    fn snapshot_from_other_lease_is_rejected() {
        let reg = LeaseRegistry::new();
        reg.open("s1", target()).unwrap();
        reg.open("s2", target()).unwrap();
        let s1_snap = reg
            .with_lease("s1", |l| l.next_snapshot(screenshot(), Vec::new()))
            .unwrap();
        reg.with_lease("s2", |l| l.next_snapshot(screenshot(), Vec::new()))
            .unwrap();
        // s1's snapshot id must not validate against s2's lease.
        assert_eq!(
            reg.with_lease("s2", |l| l.check_snapshot(&s1_snap))
                .unwrap(),
            Err(SnapshotError::WrongLease)
        );
    }

    #[test]
    fn acting_without_capture_reports_no_snapshot() {
        let reg = LeaseRegistry::new();
        reg.open("s1", target()).unwrap();
        let bogus = SnapshotId("lease-s1-1#1".into());
        assert_eq!(
            reg.with_lease("s1", |l| l.check_snapshot(&bogus)).unwrap(),
            Err(SnapshotError::NoSnapshot)
        );
    }

    #[test]
    fn operations_without_lease_error() {
        let reg = LeaseRegistry::new();
        assert_eq!(reg.target("missing"), Err(LeaseError::NoLease));
        assert_eq!(
            reg.with_lease("missing", |_| ()).err(),
            Some(LeaseError::NoLease)
        );
    }

    #[test]
    fn latest_snapshot_carries_screenshot_metadata() {
        let reg = LeaseRegistry::new();
        reg.open("s1", target()).unwrap();
        let screenshot = screenshot();
        let elements = vec![UiElement {
            id: "el-1".into(),
            role: "AXButton".into(),
            label: Some("OK".into()),
            bounds: None,
            bounds_coordinate_space: None,
            actionable: true,
        }];
        let snap = reg
            .with_lease("s1", |l| {
                l.next_snapshot(screenshot.clone(), elements.clone())
            })
            .unwrap();

        let stored = reg
            .with_lease("s1", |l| l.screenshot_for_snapshot(&snap))
            .unwrap()
            .unwrap();
        assert_eq!(stored, screenshot);

        let stored_element = reg
            .with_lease("s1", |l| l.element_for_snapshot(&snap, "el-1"))
            .unwrap()
            .unwrap();
        assert_eq!(stored_element, elements.first().cloned());
    }
}
