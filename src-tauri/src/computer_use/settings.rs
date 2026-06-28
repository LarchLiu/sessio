//! Computer-use settings.
//!
//! Host-side policy for enabling computer use.

use serde::{Deserialize, Serialize};

/// Tunable host policy for computer use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseSettings {
    /// Master switch. When false, the host refuses all leases regardless of
    /// per-session approval (a global kill-switch).
    pub enabled: bool,
}

impl Default for ComputerUseSettings {
    fn default() -> Self {
        Self { enabled: false }
    }
}

impl ComputerUseSettings {
    /// Enabled preset: observation, inspection, and control are product-allowed
    /// whenever OS capability, approvals, and session state allow them.
    pub fn enabled() -> Self {
        Self { enabled: true }
    }

    /// Recommended desktop default.
    pub fn recommended() -> Self {
        Self::enabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_fully_disabled() {
        let s = ComputerUseSettings::default();
        assert!(!s.enabled);
    }

    #[test]
    fn enabled_preset_turns_on_computer_use() {
        let s = ComputerUseSettings::enabled();
        assert!(s.enabled);
    }

    #[test]
    fn recommended_enables_computer_use() {
        let s = ComputerUseSettings::recommended();
        assert!(s.enabled);
    }

    #[test]
    fn settings_round_trip_json() {
        let s = ComputerUseSettings { enabled: true };
        let json = serde_json::to_string(&s).unwrap();
        let back: ComputerUseSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
