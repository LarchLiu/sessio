//! Session-level and global app-level approvals.
//!
//! Computer use is gated by two approval layers, independent of OS permission:
//!
//! - **Session approval** — the user enabled computer use for this session.
//! - **App approval** — the user globally allowed control of a specific target
//!   app (so enabling the feature does not implicitly grant control of every
//!   app).
//!
//! This module is the in-memory policy store. Persistent app approvals are
//! synced through computer-use settings/config; the rules themselves live here
//! so they are unit-testable.

use std::collections::HashSet;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::provider::AppId;

/// A session's approval to use computer use at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionApproval {
    pub session_id: String,
    pub approved: bool,
}

/// A global per-app approval record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppApproval {
    pub app_id: AppId,
    pub approved: bool,
}

/// The outcome of an approval check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Allowed to proceed.
    Allowed,
    /// The session has not approved computer use.
    SessionNotApproved,
    /// The session is approved but this app has not been approved.
    AppNotApproved,
}

impl ApprovalDecision {
    pub fn is_allowed(self) -> bool {
        matches!(self, ApprovalDecision::Allowed)
    }
}

/// In-memory approval policy store.
#[derive(Default)]
pub struct ApprovalRegistry {
    sessions: Mutex<HashSet<String>>,
    apps: Mutex<HashSet<AppId>>,
}

impl ApprovalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn approve_session(&self, session_id: &str) {
        self.sessions.lock().unwrap().insert(session_id.to_string());
    }

    pub fn revoke_session(&self, session_id: &str) {
        self.sessions.lock().unwrap().remove(session_id);
    }

    pub fn session_approved(&self, session_id: &str) -> bool {
        self.sessions.lock().unwrap().contains(session_id)
    }

    pub fn approve_app(&self, app_id: &AppId) {
        self.apps.lock().unwrap().insert(app_id.clone());
    }

    pub fn revoke_app(&self, app_id: &AppId) {
        self.apps.lock().unwrap().remove(app_id);
    }

    pub fn app_approved(&self, app_id: &AppId) -> bool {
        self.apps.lock().unwrap().contains(app_id)
    }

    pub fn set_approved_apps(&self, app_ids: impl IntoIterator<Item = AppId>) {
        let mut next = app_ids
            .into_iter()
            .map(|app_id| app_id.trim().to_string())
            .filter(|app_id| !app_id.is_empty())
            .collect::<HashSet<_>>();
        std::mem::swap(&mut *self.apps.lock().unwrap(), &mut next);
    }

    pub fn approved_apps(&self) -> Vec<AppId> {
        let mut apps = self
            .apps
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        apps.sort();
        apps
    }

    /// Decide whether a session may control a given app. Session approval is a
    /// prerequisite for app approval mattering at all.
    pub fn decide(&self, session_id: &str, app_id: &AppId) -> ApprovalDecision {
        if !self.session_approved(session_id) {
            return ApprovalDecision::SessionNotApproved;
        }
        if !self.app_approved(app_id) {
            return ApprovalDecision::AppNotApproved;
        }
        ApprovalDecision::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_approval_is_prerequisite_for_app_control() {
        let reg = ApprovalRegistry::new();
        let app = "com.example.app".to_string();

        // Nothing approved.
        assert_eq!(reg.decide("s1", &app), ApprovalDecision::SessionNotApproved);

        // App approved but session not → still session-blocked.
        reg.approve_app(&app);
        assert_eq!(reg.decide("s1", &app), ApprovalDecision::SessionNotApproved);

        // Session approved, app not → app-blocked.
        reg.revoke_app(&app);
        reg.approve_session("s1");
        assert_eq!(reg.decide("s1", &app), ApprovalDecision::AppNotApproved);

        // Both approved → allowed.
        reg.approve_app(&app);
        assert_eq!(reg.decide("s1", &app), ApprovalDecision::Allowed);
        assert!(reg.decide("s1", &app).is_allowed());
    }

    #[test]
    fn revoking_session_keeps_global_app_approvals() {
        let reg = ApprovalRegistry::new();
        let app = "com.example.app".to_string();
        reg.approve_session("s1");
        reg.approve_app(&app);
        assert!(reg.decide("s1", &app).is_allowed());

        reg.revoke_session("s1");
        assert!(!reg.session_approved("s1"));
        assert!(reg.app_approved(&app));
        assert_eq!(reg.decide("s1", &app), ApprovalDecision::SessionNotApproved);
    }

    #[test]
    fn app_approvals_are_global_but_session_gate_remains_per_session() {
        let reg = ApprovalRegistry::new();
        let app = "com.example.app".to_string();
        reg.approve_session("s1");
        reg.approve_app(&app);
        assert!(reg.decide("s1", &app).is_allowed());

        // App approval is global, but s2 still needs its own session approval.
        assert_eq!(reg.decide("s2", &app), ApprovalDecision::SessionNotApproved);
        reg.approve_session("s2");
        assert!(reg.decide("s2", &app).is_allowed());
    }

    #[test]
    fn approved_apps_round_trip_sorted_and_trimmed() {
        let reg = ApprovalRegistry::new();
        reg.set_approved_apps(vec![
            " com.example.two ".to_string(),
            "com.example.one".to_string(),
            "com.example.two".to_string(),
            "".to_string(),
        ]);

        assert_eq!(
            reg.approved_apps(),
            vec!["com.example.one".to_string(), "com.example.two".to_string()]
        );
    }
}
