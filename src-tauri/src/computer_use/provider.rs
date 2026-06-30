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
    /// Number of observed launches/activations in the requested recent window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_use_count: Option<u32>,
    /// Unix timestamp for the latest observed use in the requested recent window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_last_used_at: Option<i64>,
    /// Where the recent-use metadata came from, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_source: Option<String>,
}

/// Options for app discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppListOptions {
    /// Recent-use window in days. `None` means the provider default.
    pub days: Option<u32>,
}

impl Default for AppListOptions {
    fn default() -> Self {
        Self { days: None }
    }
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

/// Result of bringing an app back to the foreground.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRaiseResult {
    pub target: AppTarget,
    /// True when this call had to start the app before raising it.
    pub launched: bool,
    pub running: bool,
    /// True when the platform accepted the foreground activation request.
    pub activated: bool,
    /// Best-effort check that the app now has a visible on-screen window.
    pub visible: bool,
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

/// A point used by coordinate-based actions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// Coordinate spaces accepted by coordinate-based actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    /// Pixel coordinates in the screenshot returned by the latest snapshot.
    Screenshot,
    /// Raw display-space points, useful for platform/provider internals.
    Screen,
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
    /// OS display-space bounds reported by Accessibility, when available.
    pub bounds: Option<Rect>,
    /// Coordinate space for `bounds`. Element actions should pass `id` instead
    /// of converting these bounds into screenshot pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bounds_coordinate_space: Option<CoordinateSpace>,
    /// Whether the element is currently actionable (enabled + on-screen).
    pub actionable: bool,
}

/// Actions the host will currently accept against a snapshot. Surfaced to the
/// model so it never attempts an action the host would reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowedAction {
    ClickElement,
    ClickAt,
    SecondaryClick,
    DoubleClick,
    Drag,
    SetValue,
    TypeText,
    PressKey,
    Scroll,
}

impl AllowedAction {
    pub const ALL: [AllowedAction; 9] = [
        AllowedAction::ClickElement,
        AllowedAction::ClickAt,
        AllowedAction::SecondaryClick,
        AllowedAction::DoubleClick,
        AllowedAction::Drag,
        AllowedAction::SetValue,
        AllowedAction::TypeText,
        AllowedAction::PressKey,
        AllowedAction::Scroll,
    ];
}

/// A reference to a captured screenshot. The image bytes are stored out-of-band
/// (temp file / handle) rather than inlined, per the tool-model truncation
/// guidance; the model receives the handle, not the pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotCaptureKind {
    WindowSck,
    WindowCg,
    ScreenRectCg,
    ScreenRectGdi,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotRef {
    /// Opaque handle the broker/host can resolve back to bytes.
    pub handle: String,
    pub format: String,
    pub byte_len: u64,
    /// Pixel dimensions of the screenshot image referenced by `handle`.
    pub width: u32,
    pub height: u32,
    /// The default space future coordinate tools should interpret `x`/`y` in.
    pub default_coordinate_space: CoordinateSpace,
    /// Optional capture implementation metadata for diagnostics and strategy
    /// selection. Older snapshots may omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_kind: Option<ScreenshotCaptureKind>,
    /// The display-space rectangle, in screen points, represented by the whole
    /// screenshot image. Screenshot pixels map linearly into this rect.
    pub screen_bounds: Rect,
    /// Best-effort click replay point in this screenshot's pixel space.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click_marker: Option<Point>,
}

impl ScreenshotRef {
    pub fn resolve_point(
        &self,
        point: Point,
        coordinate_space: CoordinateSpace,
    ) -> Result<Point, String> {
        match coordinate_space {
            CoordinateSpace::Screen => Ok(point),
            CoordinateSpace::Screenshot => self.screenshot_point_to_screen_point(point),
        }
    }

    pub fn screenshot_point_to_screen_point(&self, point: Point) -> Result<Point, String> {
        if self.width == 0 || self.height == 0 {
            return Err("screenshot has empty dimensions".into());
        }
        if self.screen_bounds.width <= 0.0 || self.screen_bounds.height <= 0.0 {
            return Err("screenshot has empty screen bounds".into());
        }
        if point.x < 0.0
            || point.y < 0.0
            || point.x > self.width as f32
            || point.y > self.height as f32
        {
            return Err(format!(
                "screenshot coordinate out of bounds: ({}, {}) outside {}x{}",
                point.x, point.y, self.width, self.height
            ));
        }

        let screen_x =
            self.screen_bounds.x + (point.x / self.width as f32) * self.screen_bounds.width;
        let screen_y =
            self.screen_bounds.y + (point.y / self.height as f32) * self.screen_bounds.height;
        Ok(Point {
            x: screen_x,
            y: screen_y,
        })
    }

    pub fn screen_point_to_screenshot_point(&self, point: Point) -> Result<Point, String> {
        if self.width == 0 || self.height == 0 {
            return Err("screenshot has empty dimensions".into());
        }
        if self.screen_bounds.width <= 0.0 || self.screen_bounds.height <= 0.0 {
            return Err("screenshot has empty screen bounds".into());
        }

        let screenshot_x =
            ((point.x - self.screen_bounds.x) / self.screen_bounds.width) * self.width as f32;
        let screenshot_y =
            ((point.y - self.screen_bounds.y) / self.screen_bounds.height) * self.height as f32;
        if !screenshot_x.is_finite() || !screenshot_y.is_finite() {
            return Err("screen point could not be projected into screenshot space".into());
        }
        if screenshot_x < 0.0
            || screenshot_y < 0.0
            || screenshot_x > self.width as f32
            || screenshot_y > self.height as f32
        {
            return Err(format!(
                "screen point out of screenshot bounds: ({}, {}) outside {}x{}",
                screenshot_x, screenshot_y, self.width, self.height
            ));
        }

        Ok(Point {
            x: screenshot_x,
            y: screenshot_y,
        })
    }
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
    /// Best-effort provider-side observation for the click that produced this
    /// state, when the preceding action was a primary click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_click_result: Option<ClickExecutionResult>,
    /// Best-effort provider-side execution summary for the action that
    /// produced this state, when the host/provider currently surfaces one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action_result: Option<ActionExecutionResult>,
    pub allowed_actions: Vec<AllowedAction>,
}

/// Agent-requested primary-click dispatch hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickDispatchRoute {
    Auto,
    Ax,
    TargetPid,
    Hid,
}

/// Provider-reported primary-click dispatch channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickExecutionRoute {
    Ax,
    TargetPid,
    Hid,
    Uia,
    Native,
}

/// Provider-reported primary-click observation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickExecutionOutcome {
    SemanticSuccess,
    ObservedEffect,
    NoEffect,
    Uncertain,
}

/// Provider-side execution summary for a primary click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickExecutionResult {
    pub route: ClickExecutionRoute,
    pub outcome: ClickExecutionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_dispatch_route: Option<ClickDispatchRoute>,
}

/// Provider-reported non-click action category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionKind {
    RaiseApp,
    SecondaryClick,
    DoubleClick,
    Drag,
    Scroll,
    SetValue,
    TypeText,
    PressKey,
}

/// Provider-reported non-click action dispatch channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionRoute {
    Ax,
    TargetPid,
    Hid,
    Uia,
    Native,
}

/// Provider-reported generic action outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionOutcome {
    SemanticSuccess,
    Dispatched,
    NoEffect,
    Uncertain,
}

/// Provider-side execution summary for a non-click action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionExecutionResult {
    pub kind: ActionExecutionKind,
    pub route: ActionExecutionRoute,
    pub outcome: ActionExecutionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_dispatch_route: Option<ClickDispatchRoute>,
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
    #[error("no visible window found for application")]
    NoVisibleWindow,
    #[error("provider operation failed: {0}")]
    Failed(String),
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// The single interface through which the host performs privileged desktop
/// operations. Implementations must be `Send + Sync` (called from async tasks).
pub trait ComputerUseProvider: Send + Sync {
    /// Whether this provider can inject input on the current platform/policy.
    fn supports_control(&self) -> bool;

    fn list_apps(&self, options: AppListOptions) -> ProviderResult<Vec<InstalledApp>>;

    fn is_app_running(&self, app_id: &AppId) -> ProviderResult<bool>;

    fn launch_app(&self, target: &AppTarget) -> ProviderResult<AppLaunchResult>;

    fn raise_app(&self, target: &AppTarget) -> ProviderResult<AppRaiseResult>;

    /// Capture a fresh screenshot + AX element tree for the target.
    fn capture_app_state(&self, target: &AppTarget) -> ProviderResult<RawAppState>;

    fn click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult>;
    /// Click a screen-space point resolved by the host from snapshot metadata.
    fn click_point(
        &self,
        target: &AppTarget,
        point: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult>;
    /// Secondary/right click a screen-space point.
    fn secondary_click(
        &self,
        target: &AppTarget,
        point: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult>;
    /// Open an element's secondary/context action when AX exposes one.
    fn secondary_click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult>;
    /// Double click a screen-space point.
    fn double_click(
        &self,
        target: &AppTarget,
        point: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult>;
    /// Drag between two screen-space points.
    fn drag(
        &self,
        target: &AppTarget,
        from: Point,
        to: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult>;
    /// Set an accessibility element's value directly.
    fn set_value(&self, target: &AppTarget, element: &ElementId, value: &str)
        -> ProviderResult<ActionExecutionResult>;
    fn type_text(&self, target: &AppTarget, text: &str) -> ProviderResult<()>;
    fn press_key(&self, target: &AppTarget, key: &str) -> ProviderResult<()>;
    fn scroll(
        &self,
        target: &AppTarget,
        direction: ScrollDirection,
        amount: i32,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult>;
    /// Scroll an accessibility element when AX exposes a scroll action.
    fn scroll_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        direction: ScrollDirection,
        amount: i32,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult>;
}

#[cfg(test)]
pub use fake::FakeProvider;

#[cfg(test)]
mod fake {
    use super::*;
    use std::sync::Mutex;

    fn fake_click_route(route_hint: ClickDispatchRoute, default: ClickExecutionRoute) -> ClickExecutionRoute {
        match route_hint {
            ClickDispatchRoute::Auto => default,
            ClickDispatchRoute::Ax => ClickExecutionRoute::Ax,
            ClickDispatchRoute::TargetPid => ClickExecutionRoute::TargetPid,
            ClickDispatchRoute::Hid => ClickExecutionRoute::Hid,
        }
    }

    fn fake_action_route(route_hint: ClickDispatchRoute, default: ActionExecutionRoute) -> ActionExecutionRoute {
        match route_hint {
            ClickDispatchRoute::Auto => default,
            ClickDispatchRoute::Ax => ActionExecutionRoute::Ax,
            ClickDispatchRoute::TargetPid => ActionExecutionRoute::TargetPid,
            ClickDispatchRoute::Hid => ActionExecutionRoute::Hid,
        }
    }

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
                    recent_use_count: None,
                    recent_last_used_at: None,
                    recent_source: None,
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
                    bounds_coordinate_space: Some(CoordinateSpace::Screen),
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

        pub fn capture_count(&self) -> u64 {
            *self.capture_counter.lock().unwrap()
        }
    }

    fn point_label(point: Point) -> String {
        format!("{:.1},{:.1}", point.x, point.y)
    }

    impl ComputerUseProvider for FakeProvider {
        fn supports_control(&self) -> bool {
            self.supports_control
        }

        fn list_apps(&self, options: AppListOptions) -> ProviderResult<Vec<InstalledApp>> {
            if let Some(days) = options.days {
                self.record(format!("list_apps:{days}"));
            }
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

        fn raise_app(&self, target: &AppTarget) -> ProviderResult<AppRaiseResult> {
            let mut apps = self.apps.lock().unwrap();
            let app = apps
                .iter_mut()
                .find(|a| a.id == target.app_id)
                .ok_or_else(|| ProviderError::AppNotFound(target.app_id.clone()))?;
            let launched = !app.running;
            if launched {
                app.running = true;
                app.pid = app.pid.or(Some(1234));
            }
            self.record(format!("raise:{}", target.app_id));
            Ok(AppRaiseResult {
                target: target.clone(),
                launched,
                running: true,
                activated: true,
                visible: true,
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
                    width: 720,
                    height: 450,
                    default_coordinate_space: CoordinateSpace::Screenshot,
                    capture_kind: None,
                    screen_bounds: Rect {
                        x: 10.0,
                        y: 20.0,
                        width: 360.0,
                        height: 225.0,
                    },
                    click_marker: None,
                },
                elements: self.elements.clone(),
            })
        }

        fn click_element(
            &self,
            target: &AppTarget,
            element: &ElementId,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ClickExecutionResult> {
            if !self.elements.iter().any(|e| &e.id == element) {
                return Err(ProviderError::ElementNotFound(element.clone()));
            }
            self.record(format!(
                "click:{}:{}:{route_hint:?}",
                target.app_id, element
            ));
            Ok(ClickExecutionResult {
                route: fake_click_route(route_hint, ClickExecutionRoute::Ax),
                outcome: ClickExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            })
        }
        fn click_point(
            &self,
            target: &AppTarget,
            point: Point,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ClickExecutionResult> {
            self.record(format!(
                "click_at:{}:{}:{route_hint:?}",
                target.app_id,
                point_label(point)
            ));
            Ok(ClickExecutionResult {
                route: fake_click_route(route_hint, ClickExecutionRoute::Native),
                outcome: ClickExecutionOutcome::ObservedEffect,
                next_dispatch_route: None,
            })
        }
        fn secondary_click(
            &self,
            target: &AppTarget,
            point: Point,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ActionExecutionResult> {
            if matches!(route_hint, ClickDispatchRoute::Auto) {
                self.record(format!(
                    "secondary_click:{}:{}",
                    target.app_id,
                    point_label(point)
                ));
            } else {
                self.record(format!(
                    "secondary_click:{}:{}:{route_hint:?}",
                    target.app_id,
                    point_label(point)
                ));
            }
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SecondaryClick,
                route: fake_action_route(route_hint, ActionExecutionRoute::TargetPid),
                outcome: ActionExecutionOutcome::Dispatched,
                next_dispatch_route: None,
            })
        }
        fn secondary_click_element(
            &self,
            target: &AppTarget,
            element: &ElementId,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ActionExecutionResult> {
            if !self.elements.iter().any(|e| &e.id == element) {
                return Err(ProviderError::ElementNotFound(element.clone()));
            }
            if matches!(route_hint, ClickDispatchRoute::Auto) {
                self.record(format!(
                    "secondary_click_element:{}:{}",
                    target.app_id, element
                ));
            } else {
                self.record(format!(
                    "secondary_click_element:{}:{}:{route_hint:?}",
                    target.app_id, element
                ));
            }
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SecondaryClick,
                route: fake_action_route(route_hint, ActionExecutionRoute::Ax),
                outcome: ActionExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            })
        }
        fn double_click(
            &self,
            target: &AppTarget,
            point: Point,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ActionExecutionResult> {
            if matches!(route_hint, ClickDispatchRoute::Auto) {
                self.record(format!(
                    "double_click:{}:{}",
                    target.app_id,
                    point_label(point)
                ));
            } else {
                self.record(format!(
                    "double_click:{}:{}:{route_hint:?}",
                    target.app_id,
                    point_label(point)
                ));
            }
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::DoubleClick,
                route: fake_action_route(route_hint, ActionExecutionRoute::TargetPid),
                outcome: ActionExecutionOutcome::Dispatched,
                next_dispatch_route: None,
            })
        }
        fn drag(
            &self,
            target: &AppTarget,
            from: Point,
            to: Point,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ActionExecutionResult> {
            if matches!(route_hint, ClickDispatchRoute::Auto) {
                self.record(format!(
                    "drag:{}:{}->{}",
                    target.app_id,
                    point_label(from),
                    point_label(to)
                ));
            } else {
                self.record(format!(
                    "drag:{}:{}->{}:{route_hint:?}",
                    target.app_id,
                    point_label(from),
                    point_label(to)
                ));
            }
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::Drag,
                route: fake_action_route(route_hint, ActionExecutionRoute::TargetPid),
                outcome: ActionExecutionOutcome::Dispatched,
                next_dispatch_route: None,
            })
        }
        fn set_value(
            &self,
            target: &AppTarget,
            element: &ElementId,
            value: &str,
        ) -> ProviderResult<ActionExecutionResult> {
            if !self.elements.iter().any(|e| &e.id == element) {
                return Err(ProviderError::ElementNotFound(element.clone()));
            }
            self.record(format!("set_value:{}:{}:{}", target.app_id, element, value));
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SetValue,
                route: ActionExecutionRoute::Ax,
                outcome: ActionExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            })
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
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ActionExecutionResult> {
            if matches!(route_hint, ClickDispatchRoute::Auto) {
                self.record(format!(
                    "scroll:{}:{:?}:{}",
                    target.app_id, direction, amount
                ));
            } else {
                self.record(format!(
                    "scroll:{}:{:?}:{}:{route_hint:?}",
                    target.app_id, direction, amount
                ));
            }
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::Scroll,
                route: fake_action_route(route_hint, ActionExecutionRoute::TargetPid),
                outcome: ActionExecutionOutcome::Dispatched,
                next_dispatch_route: None,
            })
        }
        fn scroll_element(
            &self,
            target: &AppTarget,
            element: &ElementId,
            direction: ScrollDirection,
            amount: i32,
            route_hint: ClickDispatchRoute,
        ) -> ProviderResult<ActionExecutionResult> {
            if !self.elements.iter().any(|e| &e.id == element) {
                return Err(ProviderError::ElementNotFound(element.clone()));
            }
            if matches!(route_hint, ClickDispatchRoute::Auto) {
                self.record(format!(
                    "scroll_element:{}:{}:{:?}:{}",
                    target.app_id, element, direction, amount
                ));
            } else {
                self.record(format!(
                    "scroll_element:{}:{}:{:?}:{}:{route_hint:?}",
                    target.app_id, element, direction, amount
                ));
            }
            Ok(ActionExecutionResult {
                kind: ActionExecutionKind::Scroll,
                route: fake_action_route(route_hint, ActionExecutionRoute::Ax),
                outcome: ActionExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_coordinates_map_to_screen_points() {
        let screenshot = ScreenshotRef {
            handle: "snap".into(),
            format: "png".into(),
            byte_len: 1,
            width: 200,
            height: 100,
            default_coordinate_space: CoordinateSpace::Screenshot,
            capture_kind: None,
            screen_bounds: Rect {
                x: 50.0,
                y: 20.0,
                width: 100.0,
                height: 50.0,
            },
            click_marker: None,
        };

        assert_eq!(
            screenshot
                .resolve_point(Point { x: 100.0, y: 50.0 }, CoordinateSpace::Screenshot)
                .unwrap(),
            Point { x: 100.0, y: 45.0 }
        );
        assert_eq!(
            screenshot
                .resolve_point(Point { x: 7.0, y: 9.0 }, CoordinateSpace::Screen)
                .unwrap(),
            Point { x: 7.0, y: 9.0 }
        );
    }

    #[test]
    fn screenshot_coordinate_mapping_rejects_out_of_bounds_points() {
        let screenshot = ScreenshotRef {
            handle: "snap".into(),
            format: "png".into(),
            byte_len: 1,
            width: 200,
            height: 100,
            default_coordinate_space: CoordinateSpace::Screenshot,
            capture_kind: None,
            screen_bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
            click_marker: None,
        };

        assert!(screenshot
            .screenshot_point_to_screen_point(Point { x: 201.0, y: 50.0 })
            .is_err());
    }

    #[test]
    fn screen_coordinates_map_back_to_screenshot_points() {
        let screenshot = ScreenshotRef {
            handle: "snap".into(),
            format: "png".into(),
            byte_len: 1,
            width: 200,
            height: 100,
            default_coordinate_space: CoordinateSpace::Screenshot,
            capture_kind: None,
            screen_bounds: Rect {
                x: 50.0,
                y: 20.0,
                width: 100.0,
                height: 50.0,
            },
            click_marker: None,
        };

        assert_eq!(
            screenshot
                .screen_point_to_screenshot_point(Point { x: 100.0, y: 45.0 })
                .unwrap(),
            Point { x: 100.0, y: 50.0 }
        );
    }
}
