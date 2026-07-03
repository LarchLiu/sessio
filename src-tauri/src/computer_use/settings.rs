//! Computer-use settings.
//!
//! Host-side policy for enabling computer use.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::provider::ClickDispatchRoute;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationRoutePreference {
    Ax,
    TargetPid,
    Hid,
}

impl OperationRoutePreference {
    pub fn to_dispatch_route(&self) -> ClickDispatchRoute {
        match self {
            OperationRoutePreference::Ax => ClickDispatchRoute::Ax,
            OperationRoutePreference::TargetPid => ClickDispatchRoute::TargetPid,
            OperationRoutePreference::Hid => ClickDispatchRoute::Hid,
        }
    }

    pub fn from_click_route(route: ClickDispatchRoute) -> Option<Self> {
        match route {
            ClickDispatchRoute::Auto => None,
            ClickDispatchRoute::Ax => Some(Self::Ax),
            ClickDispatchRoute::TargetPid => Some(Self::TargetPid),
            ClickDispatchRoute::Hid => Some(Self::Hid),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppRoutePreferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_element: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_at: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_click_element: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_click_at: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_click: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drag: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_element: Option<OperationRoutePreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<OperationRoutePreference>,
}

/// Tunable host policy for computer use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseSettings {
    /// Master switch. When false, the host refuses all leases regardless of
    /// per-session approval (a global kill-switch).
    pub enabled: bool,
    /// Optional prompt-oriented description shown in MCP selectors and injected
    /// MCP guidance for the built-in computer-use server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_description: Option<String>,
    /// Global allowlist of app bundle identifiers approved for computer-use
    /// control. Session approval is still required separately.
    #[serde(default)]
    pub approved_apps: Vec<String>,
    /// Sticky per-app preferred dispatch routes learned from successful auto
    /// actions, so future auto actions can start with the known-good path.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub app_route_preferences: BTreeMap<String, AppRoutePreferences>,
}

impl ComputerUseSettings {
    /// Enabled preset: observation, inspection, and control are product-allowed
    /// whenever OS capability, approvals, and session state allow them.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            mcp_description: None,
            approved_apps: Vec::new(),
            app_route_preferences: BTreeMap::new(),
        }
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
        let s = ComputerUseSettings {
            enabled: true,
            mcp_description: Some("Use for desktop control".into()),
            approved_apps: vec!["com.example.app".into()],
            app_route_preferences: BTreeMap::from([(
                "com.example.app".into(),
                AppRoutePreferences {
                    click_at: Some(OperationRoutePreference::Hid),
                    ..AppRoutePreferences::default()
                },
            )]),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ComputerUseSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
