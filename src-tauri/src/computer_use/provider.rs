//! Provider abstraction: capture / inspect / control behind one interface.
//!
//! All privileged OS access is funneled through [`ComputerUseProvider`]. The
//! host orchestrates leases, snapshots, and approvals against this trait without
//! knowing the platform. Tests use [`FakeProvider`].

use serde::{Deserialize, Serialize};

/// Stable identifier for an installed/running application target.
pub type AppId = String;

/// Stable identifier for a UI element within a captured snapshot.
pub type ElementId = String;

/// An application the agent can target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub id: AppId,
    pub name: String,
    /// OS process id when the app is running; `None` if only installed.
    pub pid: Option<i32>,
    pub running: bool,
}

/// Result of an app launch request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLaunchResult {
    pub target: AppTarget,
    /// True when this call started the app. False means it was already running.
    pub launched: bool,
    pub running: bool,
}

/// The concrete target a lease is opened against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTarget {
    pub app_id: AppId,
    /// Optional specific window; `None` targets the app's frontmost window.
    pub window_id: Option<String>,
}

/// Display geometry accompanying a snapshot so the model can reason about
/// coordinates and scaling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayMetadata {
    pub width: u32,
    pub height: u32,
    /// Backing scale factor (e.g. 2.0 on Retina).
    pub scale: f32,
}

/// A bounding rectangle in display points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A single accessibility element exposed to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiElement {
    pub id: ElementId,
    /// Accessibility role (e.g. "AXButton").
    pub role: String,
    /// Human-visible label/title where available.
    pub label: Option<String>,
    pub bounds: Option<Rect>,
    /// Whether the element is currently actionable (enabled + on-screen).
    pub actionable: bool,
}

/// Actions the host will currently accept against a snapshot. Surfaced to the
/// model so it never attempts an action the host would reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedAction {
    ClickElement,
    TypeText,
    PressKey,
    Scroll,
}

impl AllowedAction {
    pub const ALL: [AllowedAction; 4] = [
        AllowedAction::ClickElement,
        AllowedAction::TypeText,
        AllowedAction::PressKey,
        AllowedAction::Scroll,
    ];
}

/// A reference to a captured screenshot. The image bytes are stored out-of-band
/// (temp file / handle) rather than inlined, per the tool-model truncation
/// guidance; the model receives the handle, not the pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRef {
    /// Opaque handle the broker/host can resolve back to bytes.
    pub handle: String,
    pub format: String,
    pub byte_len: u64,
}

/// The raw app state a provider returns for one capture. The host stamps it with
/// a snapshot id before handing it to the agent (see [`crate::computer_use::lease`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawAppState {
    pub target: AppTarget,
    pub display: DisplayMetadata,
    pub screenshot: ScreenshotRef,
    pub elements: Vec<UiElement>,
}

/// App state as delivered to the agent: a provider capture plus the host-stamped
/// snapshot id and the actions currently allowed against it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub snapshot_id: String,
    pub target: AppTarget,
    /// True when this state capture first had to launch the target app.
    pub launched: bool,
    pub display: DisplayMetadata,
    pub screenshot: ScreenshotRef,
    pub elements: Vec<UiElement>,
    pub allowed_actions: Vec<AllowedAction>,
}

/// Scroll direction for [`ComputerUseProvider::scroll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Errors a provider can return. The host maps these into tool errors.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("target application not found: {0}")]
    AppNotFound(AppId),
    #[error("element not found in current snapshot: {0}")]
    ElementNotFound(ElementId),
    #[error("provider does not support {0} on this platform")]
    Unsupported(&'static str),
    #[error("provider operation failed: {0}")]
    Failed(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// The single interface through which the host performs privileged desktop
/// operations. Implementations must be `Send + Sync` (called from async tasks).
pub trait ComputerUseProvider: Send + Sync {
    /// Whether this provider can inject input on the current platform/policy.
    fn supports_control(&self) -> bool;

    fn list_apps(&self) -> ProviderResult<Vec<InstalledApp>>;

    fn is_app_running(&self, app_id: &AppId) -> ProviderResult<bool>;

    fn launch_app(&self, target: &AppTarget) -> ProviderResult<AppLaunchResult>;

    /// Capture a fresh screenshot + AX element tree for the target.
    fn capture_app_state(&self, target: &AppTarget) -> ProviderResult<RawAppState>;

    fn click_element(&self, target: &AppTarget, element: &ElementId) -> ProviderResult<()>;
    fn type_text(&self, target: &AppTarget, text: &str) -> ProviderResult<()>;
    fn press_key(&self, target: &AppTarget, key: &str) -> ProviderResult<()>;
    fn scroll(
        &self,
        target: &AppTarget,
        direction: ScrollDirection,
        amount: i32,
    ) -> ProviderResult<()>;
}

#[cfg(test)]
pub use fake::FakeProvider;

#[cfg(test)]
mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Deterministic in-memory provider for host tests. Records actions so tests
    /// can assert orchestration without touching the OS.
    pub struct FakeProvider {
        pub apps: Mutex<Vec<InstalledApp>>,
        pub elements: Vec<UiElement>,
        pub supports_control: bool,
        pub recorded: Mutex<Vec<String>>,
        capture_counter: Mutex<u64>,
    }

    impl Default for FakeProvider {
        fn default() -> Self {
            Self {
                apps: Mutex::new(vec![InstalledApp {
                    id: "com.example.app".into(),
                    name: "Example".into(),
                    pid: Some(1234),
                    running: true,
                }]),
                elements: vec![UiElement {
                    id: "el-1".into(),
                    role: "AXButton".into(),
                    label: Some("OK".into()),
                    bounds: Some(Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 80.0,
                        height: 24.0,
                    }),
                    actionable: true,
                }],
                supports_control: true,
                recorded: Mutex::new(Vec::new()),
                capture_counter: Mutex::new(0),
            }
        }
    }

    impl FakeProvider {
        pub fn with_apps(apps: Vec<InstalledApp>) -> Self {
            Self {
                apps: Mutex::new(apps),
                ..Self::default()
            }
        }

        pub fn record(&self, action: impl Into<String>) {
            self.recorded.lock().unwrap().push(action.into());
        }
        pub fn actions(&self) -> Vec<String> {
            self.recorded.lock().unwrap().clone()
        }
    }

    impl ComputerUseProvider for FakeProvider {
        fn supports_control(&self) -> bool {
            self.supports_control
        }

        fn list_apps(&self) -> ProviderResult<Vec<InstalledApp>> {
            Ok(self.apps.lock().unwrap().clone())
        }

        fn is_app_running(&self, app_id: &AppId) -> ProviderResult<bool> {
            self.apps
                .lock()
                .unwrap()
                .iter()
                .find(|a| &a.id == app_id)
                .map(|a| a.running)
                .ok_or_else(|| ProviderError::AppNotFound(app_id.clone()))
        }

        fn launch_app(&self, target: &AppTarget) -> ProviderResult<AppLaunchResult> {
            let mut apps = self.apps.lock().unwrap();
            let app = apps
                .iter_mut()
                .find(|a| a.id == target.app_id)
                .ok_or_else(|| ProviderError::AppNotFound(target.app_id.clone()))?;
            let launched = !app.running;
            if launched {
                app.running = true;
                app.pid = app.pid.or(Some(1234));
                self.record(format!("launch:{}", target.app_id));
            }
            Ok(AppLaunchResult {
                target: target.clone(),
                launched,
                running: true,
            })
        }

        fn capture_app_state(&self, target: &AppTarget) -> ProviderResult<RawAppState> {
            let apps = self.apps.lock().unwrap();
            if !apps.iter().any(|a| a.id == target.app_id && a.running) {
                return Err(ProviderError::AppNotFound(target.app_id.clone()));
            }
            drop(apps);
            let mut counter = self.capture_counter.lock().unwrap();
            *counter += 1;
            Ok(RawAppState {
                target: target.clone(),
                display: DisplayMetadata {
                    width: 1440,
                    height: 900,
                    scale: 2.0,
                },
                screenshot: ScreenshotRef {
                    handle: format!("snap-{}", *counter),
                    format: "png".into(),
                    byte_len: 1024,
                },
                elements: self.elements.clone(),
            })
        }

        fn click_element(&self, target: &AppTarget, element: &ElementId) -> ProviderResult<()> {
            if !self.elements.iter().any(|e| &e.id == element) {
                return Err(ProviderError::ElementNotFound(element.clone()));
            }
            self.record(format!("click:{}:{}", target.app_id, element));
            Ok(())
        }
        fn type_text(&self, target: &AppTarget, text: &str) -> ProviderResult<()> {
            self.record(format!("type:{}:{}", target.app_id, text));
            Ok(())
        }
        fn press_key(&self, target: &AppTarget, key: &str) -> ProviderResult<()> {
            self.record(format!("key:{}:{}", target.app_id, key));
            Ok(())
        }
        fn scroll(
            &self,
            target: &AppTarget,
            direction: ScrollDirection,
            amount: i32,
        ) -> ProviderResult<()> {
            self.record(format!(
                "scroll:{}:{:?}:{}",
                target.app_id, direction, amount
            ));
            Ok(())
        }
    }
}
