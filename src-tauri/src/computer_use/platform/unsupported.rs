//! Stub provider for non-macOS platforms.
//!
//! Computer use targets macOS first; on other platforms the provider compiles
//! but reports every privileged operation as unsupported, so the host degrades
//! cleanly (observation-only or fully disabled) rather than failing to build.

use crate::computer_use::provider::{
    AppId, AppLaunchResult, AppTarget, ComputerUseProvider, ElementId, InstalledApp, ProviderError,
    ProviderResult, RawAppState, ScrollDirection,
};

pub struct UnsupportedProvider;

impl UnsupportedProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UnsupportedProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputerUseProvider for UnsupportedProvider {
    fn supports_control(&self) -> bool {
        false
    }

    fn list_apps(&self) -> ProviderResult<Vec<InstalledApp>> {
        Err(ProviderError::Unsupported("list_apps"))
    }

    fn is_app_running(&self, _app_id: &AppId) -> ProviderResult<bool> {
        Err(ProviderError::Unsupported("is_app_running"))
    }

    fn launch_app(&self, _target: &AppTarget) -> ProviderResult<AppLaunchResult> {
        Err(ProviderError::Unsupported("launch_app"))
    }

    fn capture_app_state(&self, _target: &AppTarget) -> ProviderResult<RawAppState> {
        Err(ProviderError::Unsupported("capture_app_state"))
    }

    fn click_element(&self, _target: &AppTarget, _element: &ElementId) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("click_element"))
    }

    fn type_text(&self, _target: &AppTarget, _text: &str) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("type_text"))
    }

    fn press_key(&self, _target: &AppTarget, _key: &str) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("press_key"))
    }

    fn scroll(
        &self,
        _target: &AppTarget,
        _direction: ScrollDirection,
        _amount: i32,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("scroll"))
    }
}
