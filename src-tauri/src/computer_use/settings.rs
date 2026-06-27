//! Computer-use settings.
//!
//! Host-side knobs that shape what the feature permits, independent of OS
//! permission and per-session approval.

use serde::{Deserialize, Serialize};

/// Tunable host policy for computer use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseSettings {
    /// Master switch. When false, the host refuses all leases regardless of
    /// per-session approval (a global kill-switch).
    pub enabled: bool,
    /// Whether input injection (`canControl`) may be used at all. Defaults off
    /// so observation/inspection can ship before control is trusted.
    pub allow_input_injection: bool,
    /// Whether an action that needs the target window foregrounded may proceed
    /// only behind the takeover UI (Phase 5). Off here means such actions are
    /// rejected until the takeover path exists.
    pub allow_foreground_takeover: bool,
}

impl Default for ComputerUseSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_input_injection: false,
            allow_foreground_takeover: false,
        }
    }
}

impl ComputerUseSettings {
    /// Observation-only preset: capture + inspect, no control.
    pub fn observe_only() -> Self {
        Self {
            enabled: true,
            allow_input_injection: false,
            allow_foreground_takeover: false,
        }
    }

    /// Recommended desktop default: computer use is on and full control is
    /// available when the OS permission layer allows it.
    pub fn recommended() -> Self {
        Self {
            enabled: true,
            allow_input_injection: true,
            allow_foreground_takeover: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_fully_disabled() {
        let s = ComputerUseSettings::default();
        assert!(!s.enabled);
        assert!(!s.allow_input_injection);
        assert!(!s.allow_foreground_takeover);
    }

    #[test]
    fn observe_only_enables_without_control() {
        let s = ComputerUseSettings::observe_only();
        assert!(s.enabled);
        assert!(!s.allow_input_injection);
    }

    #[test]
    fn recommended_enables_control_and_takeover() {
        let s = ComputerUseSettings::recommended();
        assert!(s.enabled);
        assert!(s.allow_input_injection);
        assert!(s.allow_foreground_takeover);
    }

    #[test]
    fn settings_round_trip_json() {
        let s = ComputerUseSettings {
            enabled: true,
            allow_input_injection: true,
            allow_foreground_takeover: false,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ComputerUseSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
