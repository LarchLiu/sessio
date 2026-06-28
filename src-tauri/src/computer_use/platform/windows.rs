//! Windows implementation of [`ComputerUseProvider`].
//!
//! This provider is backed by Win32 for app/window discovery, foreground
//! recovery, GDI capture for the first screenshot backend, UI Automation for
//! element inspection/actions, and `SendInput` for physical input fallbacks.

use std::path::PathBuf;

use crate::computer_use::provider::{
    AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppTarget, ComputerUseProvider,
    ElementId, InstalledApp, Point, ProviderError, ProviderResult, RawAppState, ScrollDirection,
};

/// Windows provider. The capture directory mirrors the macOS provider so
/// screenshot handles remain temp-file paths across platforms.
pub struct WindowsProvider {
    capture_dir: PathBuf,
}

impl WindowsProvider {
    pub fn new() -> Self {
        Self {
            capture_dir: std::env::temp_dir().join("sessio-computer-use"),
        }
    }
}

impl Default for WindowsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputerUseProvider for WindowsProvider {
    fn supports_control(&self) -> bool {
        let _ = &self.capture_dir;
        false
    }

    fn list_apps(&self, _options: AppListOptions) -> ProviderResult<Vec<InstalledApp>> {
        Err(ProviderError::Unsupported("list_apps"))
    }

    fn is_app_running(&self, _app_id: &AppId) -> ProviderResult<bool> {
        Err(ProviderError::Unsupported("is_app_running"))
    }

    fn launch_app(&self, _target: &AppTarget) -> ProviderResult<AppLaunchResult> {
        Err(ProviderError::Unsupported("launch_app"))
    }

    fn raise_app(&self, _target: &AppTarget) -> ProviderResult<AppRaiseResult> {
        Err(ProviderError::Unsupported("raise_app"))
    }

    fn capture_app_state(&self, _target: &AppTarget) -> ProviderResult<RawAppState> {
        Err(ProviderError::Unsupported("capture_app_state"))
    }

    fn click_element(&self, _target: &AppTarget, _element: &ElementId) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("click_element"))
    }

    fn click_point(&self, _target: &AppTarget, _point: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("click_point"))
    }

    fn secondary_click(&self, _target: &AppTarget, _point: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("secondary_click"))
    }

    fn secondary_click_element(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("secondary_click_element"))
    }

    fn double_click(&self, _target: &AppTarget, _point: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("double_click"))
    }

    fn drag(&self, _target: &AppTarget, _from: Point, _to: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("drag"))
    }

    fn set_value(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
        _value: &str,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("set_value"))
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

    fn scroll_element(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
        _direction: ScrollDirection,
        _amount: i32,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("scroll_element"))
    }
}
