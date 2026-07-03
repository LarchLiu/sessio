//! macOS implementation of [`ComputerUseProvider`].
//!
//! - **App enumeration**: `NSWorkspace.runningApplications`.
//! - **Screenshot**: ScreenCaptureKit targeting the app's frontmost
//!   desktop-independent window, with a CoreGraphics window-image compatibility
//!   fallback.
//! - **Element tree**: the Accessibility API (`AXUIElement*`).
//! - **Input injection**: `CGEvent` keyboard / mouse / scroll synthesis.
//!
//! Privileged: requires Screen Recording (capture) and Accessibility trust
//! (AX + synthesized-event delivery). The host gates these via the
//! desktop-control permission layer before any method here is called.
//!
//! The AX and CGEvent paths are net-new and must be verified on a real desktop;
//! see the module-level note in `platform/mod.rs`.

#![allow(unexpected_cfgs)]

use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use block2::RcBlock;
use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, CGMouseButton, EventField,
    ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGRect};
use foreign_types::ForeignType;
use image::{
    imageops::{crop_imm, FilterType},
    ImageFormat, ImageReader,
};
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2::{AnyThread, MainThreadMarker, Message};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSScreen};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSPoint, NSRect, NSSize};
use sha2::{Digest, Sha256};

use crate::computer_use::diagnostics;
use crate::computer_use::provider::{
    ActionExecutionKind, ActionExecutionOutcome, ActionExecutionResult, ActionExecutionRoute,
    AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppTarget, ClickDispatchRoute,
    ClickExecutionOutcome, ClickExecutionResult, ClickExecutionRoute, ComputerUseProvider,
    CoordinateSpace, DisplayMetadata, ElementId, InstalledApp, Point, ProviderCapabilities,
    ProviderError, ProviderResult, RawAppState, Rect, ScreenshotCaptureKind, ScreenshotRef,
    ScrollDirection, UiElement,
};

#[derive(Clone)]
struct FrontmostApp {
    pid: i32,
    app: objc2::rc::Retained<objc2_app_kit::NSRunningApplication>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum InputDispatchRoute {
    TargetPid,
    Hid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputActionKind {
    MouseClick,
    MouseSecondaryClick,
    MouseDoubleClick,
    MouseDrag,
    Scroll,
    KeyboardText,
    KeyboardKey,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AppInputSignals {
    has_embedded_web_runtime: bool,
    ax_element_count: usize,
    actionable_ax_element_count: usize,
    visible_window_actionable_ax_element_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventTarget {
    route: InputDispatchRoute,
    pid: i32,
    ensure_frontmost: bool,
    restore_frontmost: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickRouteKind {
    Ax,
    TargetPid,
    Hid,
}

#[derive(Debug, Clone, Copy)]
enum ClickIntent<'a> {
    Element(&'a ElementId),
    Point(CGPoint),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickAttemptOutcome {
    Succeeded,
    NoEffect,
    Uncertain,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WindowCaptureFingerprint {
    full_hash: [u8; 32],
    local_hash: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RoutedActionExecution {
    route: InputDispatchRoute,
    outcome: ClickAttemptOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickEffectProbeOutcome {
    ObservedEffect,
    NoEffect,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClickEffectProbeTiming {
    initial_delay_ms: u64,
    poll_interval_ms: u64,
    total_window_ms: u64,
}

impl ClickEffectProbeTiming {
    #[cfg(not(test))]
    const DEFAULT: Self = Self {
        initial_delay_ms: 500,
        poll_interval_ms: 500,
        total_window_ms: 2000,
    };

    #[cfg(test)]
    const DEFAULT: Self = Self {
        initial_delay_ms: 0,
        poll_interval_ms: 0,
        total_window_ms: 0,
    };

    fn attempt_count(self) -> usize {
        if self.total_window_ms <= self.initial_delay_ms || self.poll_interval_ms == 0 {
            1
        } else {
            1 + ((self.total_window_ms - self.initial_delay_ms) / self.poll_interval_ms) as usize
        }
    }
}

const CLICK_EFFECT_LOCAL_PROBE_RADIUS_PX: u32 = 72;

/// The macOS provider. Stateless: every call re-reads live system state.
pub struct MacosProvider {
    /// Directory for capture output. Defaults to the OS temp dir.
    capture_dir: PathBuf,
}

impl MacosProvider {
    pub fn new() -> Self {
        Self {
            capture_dir: std::env::temp_dir().join("sessio-computer-use"),
        }
    }
}

impl Default for MacosProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputerUseProvider for MacosProvider {
    fn supports_control(&self) -> bool {
        // CGEvent injection is available on macOS; the host still gates it on
        // accessibility trust + settings before calling control methods.
        true
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full()
    }

    fn list_apps(&self, options: AppListOptions) -> ProviderResult<Vec<InstalledApp>> {
        list_available_apps(options)
    }

    fn is_app_running(&self, app_id: &AppId) -> ProviderResult<bool> {
        match resolve_pid(app_id) {
            Ok(_) => Ok(true),
            Err(ProviderError::AppNotFound(_)) if installed_app_url(app_id).is_some() => Ok(false),
            Err(err) => Err(err),
        }
    }

    fn launch_app(&self, target: &AppTarget) -> ProviderResult<AppLaunchResult> {
        launch_app_background(target)
    }

    fn raise_app(&self, target: &AppTarget) -> ProviderResult<AppRaiseResult> {
        raise_app_foreground(target)
    }

    fn capture_app_state(&self, target: &AppTarget) -> ProviderResult<RawAppState> {
        let pid = resolve_pid(&target.app_id)?;
        let (window_id, bounds) = frontmost_window_for_pid(pid)?;
        let display = display_metadata_for_window_bounds(bounds);
        let screen_bounds = bounds.unwrap_or_else(|| display_screen_bounds(&display));
        let screenshot = capture_window(&self.capture_dir, window_id, screen_bounds)?;
        let elements = ax_elements_for_pid(pid).unwrap_or_default();
        Ok(RawAppState {
            target: target.clone(),
            display,
            screenshot,
            elements: elements_with_bounds(elements, Some(screen_bounds)),
        })
    }

    fn click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        execute_click_intent(
            &self.capture_dir,
            target,
            pid,
            ClickIntent::Element(element),
            route_hint,
        )
    }

    fn click_point(
        &self,
        target: &AppTarget,
        point: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        execute_click_intent(
            &self.capture_dir,
            target,
            pid,
            ClickIntent::Point(cg_point(point)),
            route_hint,
        )
    }

    fn secondary_click(
        &self,
        target: &AppTarget,
        point: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        execute_routed_action_with_effect_probe(
            &self.capture_dir,
            target,
            pid,
            InputActionKind::MouseSecondaryClick,
            Some(cg_point(point)),
            mouse_route_plan_for_point(route_hint),
            |route| {
                secondary_click_at(
                    event_target_for_route(
                        target,
                        pid,
                        InputActionKind::MouseSecondaryClick,
                        route,
                        Some(default_route_reason_for_action(
                            InputActionKind::MouseSecondaryClick,
                            route,
                        )),
                    ),
                    cg_point(point),
                )
            },
        )
        .map(|execution| {
            action_execution_result(
                ActionExecutionKind::SecondaryClick,
                execution.route,
                execution.outcome,
            )
        })
    }

    fn secondary_click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        secondary_ax_element(&self.capture_dir, target, pid, element, route_hint)
    }

    fn double_click(
        &self,
        target: &AppTarget,
        point: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        execute_routed_action_with_effect_probe(
            &self.capture_dir,
            target,
            pid,
            InputActionKind::MouseDoubleClick,
            Some(cg_point(point)),
            mouse_route_plan_for_point(route_hint),
            |route| {
                double_click_at(
                    event_target_for_route(
                        target,
                        pid,
                        InputActionKind::MouseDoubleClick,
                        route,
                        Some(default_route_reason_for_action(
                            InputActionKind::MouseDoubleClick,
                            route,
                        )),
                    ),
                    cg_point(point),
                )
            },
        )
        .map(|execution| {
            action_execution_result(
                ActionExecutionKind::DoubleClick,
                execution.route,
                execution.outcome,
            )
        })
    }

    fn drag(
        &self,
        target: &AppTarget,
        from: Point,
        to: Point,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        execute_routed_action_with_effect_probe(
            &self.capture_dir,
            target,
            pid,
            InputActionKind::MouseDrag,
            Some(cg_point(to)),
            mouse_route_plan_for_point(route_hint),
            |route| {
                drag_between(
                    event_target_for_route(
                        target,
                        pid,
                        InputActionKind::MouseDrag,
                        route,
                        Some(default_route_reason_for_action(
                            InputActionKind::MouseDrag,
                            route,
                        )),
                    ),
                    cg_point(from),
                    cg_point(to),
                )
            },
        )
        .map(|execution| {
            action_execution_result(
                ActionExecutionKind::Drag,
                execution.route,
                execution.outcome,
            )
        })
    }

    fn set_value(
        &self,
        target: &AppTarget,
        element: &ElementId,
        value: &str,
    ) -> ProviderResult<ActionExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        set_ax_value_for_id(pid, element, value)
    }

    fn type_text(&self, target: &AppTarget, text: &str) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        type_unicode(
            event_target_for(target, pid, InputActionKind::KeyboardText),
            text,
        )
    }

    fn press_key(&self, target: &AppTarget, key: &str) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        let (keycode, flags) = keycode_and_flags_for(key)
            .ok_or_else(|| ProviderError::Failed(format!("unknown key: {key}")))?;
        press_keycode(
            event_target_for(target, pid, InputActionKind::KeyboardKey),
            keycode,
            flags,
        )
    }

    fn scroll(
        &self,
        target: &AppTarget,
        direction: ScrollDirection,
        amount: i32,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let pid = resolve_pid(&target.app_id)?;
        execute_routed_action_with_effect_probe(
            &self.capture_dir,
            target,
            pid,
            InputActionKind::Scroll,
            None,
            mouse_route_plan_for_point(route_hint),
            |route| {
                scroll_wheel(
                    event_target_for_route(
                        target,
                        pid,
                        InputActionKind::Scroll,
                        route,
                        Some(default_route_reason_for_action(
                            InputActionKind::Scroll,
                            route,
                        )),
                    ),
                    direction,
                    amount,
                )
            },
        )
        .map(|execution| {
            action_execution_result(
                ActionExecutionKind::Scroll,
                execution.route,
                execution.outcome,
            )
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
        let pid = resolve_pid(&target.app_id)?;
        scroll_ax_element(
            &self.capture_dir,
            target,
            pid,
            element,
            direction,
            amount,
            route_hint,
        )
    }
}

// --- App enumeration -----------------------------------------------------

fn list_available_apps(options: AppListOptions) -> ProviderResult<Vec<InstalledApp>> {
    let mut by_id: HashMap<AppId, InstalledApp> = HashMap::new();
    for app in list_installed_apps() {
        by_id.entry(app.id.clone()).or_insert(app);
    }
    for app in list_running_apps()? {
        by_id.insert(app.id.clone(), app);
    }
    let mut apps: Vec<InstalledApp> = by_id.into_values().collect();
    annotate_recent_usage(&mut apps, options.days.unwrap_or(14));
    apps.sort_by(|a, b| {
        b.recent_use_count
            .unwrap_or(0)
            .cmp(&a.recent_use_count.unwrap_or(0))
            .then_with(|| {
                b.recent_last_used_at
                    .unwrap_or(0)
                    .cmp(&a.recent_last_used_at.unwrap_or(0))
            })
            .then_with(|| b.running.cmp(&a.running))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(apps)
}

fn list_running_apps() -> ProviderResult<Vec<InstalledApp>> {
    use objc2_app_kit::NSWorkspace;
    let mut out = Vec::new();
    let workspace = NSWorkspace::sharedWorkspace();
    let apps = workspace.runningApplications();
    for app in apps.iter() {
        // Only surface regular (UI) apps with a bundle id.
        let pid = app.processIdentifier();
        let bundle_id = app
            .bundleIdentifier()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if bundle_id.is_empty() {
            continue;
        }
        let name = app
            .localizedName()
            .map(|s| s.to_string())
            .unwrap_or_else(|| bundle_id.clone());
        out.push(InstalledApp {
            id: bundle_id,
            name,
            pid: Some(pid),
            running: true,
            recent_use_count: None,
            recent_last_used_at: None,
            recent_source: None,
        });
    }
    Ok(out)
}

fn list_installed_apps() -> Vec<InstalledApp> {
    let mut out = Vec::new();
    for root in app_search_roots() {
        scan_app_bundles(&root, 0, &mut out);
    }
    out
}

fn app_search_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
    ];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    roots
}

fn scan_app_bundles(dir: &Path, depth: usize, out: &mut Vec<InstalledApp>) {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_app_bundle_path(&path) {
            if let Some(app) = installed_app_from_bundle(&path) {
                out.push(app);
            }
            continue;
        }
        if path.is_dir() {
            scan_app_bundles(&path, depth + 1, out);
        }
    }
}

fn is_app_bundle_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("app"))
        .unwrap_or(false)
}

fn installed_app_from_bundle(path: &Path) -> Option<InstalledApp> {
    use objc2_foundation::{NSBundle, NSString, NSURL};
    let path_str = path.to_string_lossy();
    let ns_path = NSString::from_str(&path_str);
    let url = NSURL::fileURLWithPath_isDirectory(&ns_path, true);
    let bundle = NSBundle::bundleWithURL(&url)?;
    let id = bundle.bundleIdentifier()?.to_string();
    let name = path
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| id.clone());
    Some(InstalledApp {
        id,
        name,
        pid: None,
        running: false,
        recent_use_count: None,
        recent_last_used_at: None,
        recent_source: None,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecentUsage {
    count: u32,
    last_used_at: i64,
    source: &'static str,
}

fn annotate_recent_usage(apps: &mut [InstalledApp], days: u32) {
    let mut usage = recent_usage_from_knowledge(days).unwrap_or_default();
    merge_recent_usage(&mut usage, recent_usage_from_spotlight(apps, days));
    for app in apps {
        if let Some(recent) = usage.get(&app.id) {
            app.recent_use_count = Some(recent.count);
            app.recent_last_used_at = Some(recent.last_used_at);
            app.recent_source = Some(recent.source.to_string());
        }
    }
}

fn merge_recent_usage(
    target: &mut HashMap<AppId, RecentUsage>,
    fallback: HashMap<AppId, RecentUsage>,
) {
    for (app_id, incoming) in fallback {
        target
            .entry(app_id)
            .and_modify(|current| {
                if incoming.last_used_at > current.last_used_at {
                    current.last_used_at = incoming.last_used_at;
                }
                if current.count == 0 {
                    current.count = incoming.count;
                }
            })
            .or_insert(incoming);
    }
}

fn recent_usage_from_knowledge(days: u32) -> Option<HashMap<AppId, RecentUsage>> {
    let path = knowledge_db_path()?;
    if !path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let cutoff = unix_now().saturating_sub(i64::from(days) * 86_400);
    let cutoff_cf = cutoff.saturating_sub(978_307_200);
    let mut stmt = conn
        .prepare(
            r#"
            SELECT ZVALUESTRING, COUNT(*), CAST(MAX(ZSTARTDATE) AS INTEGER)
            FROM ZOBJECT
            WHERE ZSTREAMNAME = '/app/inFocus'
              AND ZSTARTDATE >= ?1
              AND ZVALUESTRING IS NOT NULL
            GROUP BY ZVALUESTRING
            "#,
        )
        .ok()?;
    let rows = stmt
        .query_map([cutoff_cf], |row| {
            let app_id: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            let last_cf: i64 = row.get(2)?;
            Ok((app_id, count, last_cf))
        })
        .ok()?;
    let mut out = HashMap::new();
    for row in rows.flatten() {
        let (app_id, count, last_cf) = row;
        if app_id.is_empty() {
            continue;
        }
        out.insert(
            app_id,
            RecentUsage {
                count: u32::try_from(count.max(0)).unwrap_or(u32::MAX),
                last_used_at: last_cf.saturating_add(978_307_200),
                source: "knowledgeC",
            },
        );
    }
    Some(out)
}

fn knowledge_db_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library/Application Support/Knowledge/knowledgeC.db"))
}

fn recent_usage_from_spotlight(apps: &[InstalledApp], days: u32) -> HashMap<AppId, RecentUsage> {
    let cutoff = unix_now().saturating_sub(i64::from(days) * 86_400);
    let known_ids: HashSet<&str> = apps.iter().map(|app| app.id.as_str()).collect();
    let mut out = HashMap::new();
    for path in spotlight_recent_app_paths(days) {
        let Some(app) = installed_app_from_bundle(&path) else {
            continue;
        };
        if !known_ids.contains(app.id.as_str()) {
            continue;
        }
        let Some(last_used_at) = spotlight_last_used_at(&path) else {
            continue;
        };
        if last_used_at < cutoff {
            continue;
        }
        out.insert(
            app.id,
            RecentUsage {
                count: 1,
                last_used_at,
                source: "spotlight",
            },
        );
    }
    out
}

fn spotlight_recent_app_paths(days: u32) -> Vec<PathBuf> {
    let query = format!(
        "kMDItemContentType == \"com.apple.application-bundle\" && kMDItemLastUsedDate >= $time.today(-{days})"
    );
    let output = std::process::Command::new("mdfind")
        .arg("-0")
        .arg(query)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .filter_map(|bytes| String::from_utf8(bytes.to_vec()).ok())
        .map(PathBuf::from)
        .collect()
}

fn spotlight_last_used_at(path: &Path) -> Option<i64> {
    let output = std::process::Command::new("mdls")
        .arg("-raw")
        .arg("-name")
        .arg("kMDItemLastUsedDate")
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() || raw == "(null)" {
        return None;
    }
    parse_mdls_utc_timestamp(&raw)
}

fn parse_mdls_utc_timestamp(raw: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(raw.trim(), "%Y-%m-%d %H:%M:%S %z")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn resolve_pid(app_id: &str) -> ProviderResult<i32> {
    list_running_apps()?
        .into_iter()
        .find(|app| app.id == app_id)
        .and_then(|app| app.pid)
        .ok_or_else(|| ProviderError::AppNotFound(app_id.to_string()))
}

fn installed_app_url(app_id: &str) -> Option<objc2::rc::Retained<objc2_foundation::NSURL>> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;
    let workspace = NSWorkspace::sharedWorkspace();
    workspace.URLForApplicationWithBundleIdentifier(&NSString::from_str(app_id))
}

fn event_target_for(target: &AppTarget, pid: i32, action: InputActionKind) -> EventTarget {
    let route = default_primary_route_for_action(action);
    event_target_for_route(
        target,
        pid,
        action,
        route,
        Some(default_route_reason_for_action(action, route)),
    )
}

fn event_target_for_route(
    target: &AppTarget,
    pid: i32,
    action: InputActionKind,
    route: InputDispatchRoute,
    route_reason_override: Option<&'static str>,
) -> EventTarget {
    let signals = app_input_signals(target, pid);
    let ensure_frontmost = matches!(route, InputDispatchRoute::Hid);
    let restore_frontmost = !ensure_frontmost;
    let route_reason =
        route_reason_override.unwrap_or_else(|| default_route_reason_for_action(action, route));

    crate::computer_use::diagnostics::write(
        "input_dispatch_route",
        serde_json::json!({
            "appId": target.app_id,
            "windowId": target.window_id,
            "pid": pid,
            "action": match action {
                InputActionKind::MouseClick => "mouse_click",
                InputActionKind::MouseSecondaryClick => "mouse_secondary_click",
                InputActionKind::MouseDoubleClick => "mouse_double_click",
                InputActionKind::MouseDrag => "mouse_drag",
                InputActionKind::Scroll => "scroll",
                InputActionKind::KeyboardText => "keyboard_text",
                InputActionKind::KeyboardKey => "keyboard_key",
            },
            "profile": "operation_based",
            "route": route,
            "routeReason": route_reason,
            "ensureFrontmost": ensure_frontmost,
            "restoreFrontmost": restore_frontmost,
            "hasEmbeddedWebRuntime": signals.has_embedded_web_runtime,
            "axElementCount": signals.ax_element_count,
            "actionableAxElementCount": signals.actionable_ax_element_count,
            "visibleWindowActionableAxElementCount": signals.visible_window_actionable_ax_element_count,
        }),
    );

    EventTarget {
        route,
        pid,
        ensure_frontmost,
        restore_frontmost,
    }
}

fn dispatch_route_for_action(action: InputActionKind) -> InputDispatchRoute {
    match action {
        InputActionKind::KeyboardText | InputActionKind::KeyboardKey => {
            InputDispatchRoute::TargetPid
        }
        InputActionKind::MouseClick
        | InputActionKind::MouseSecondaryClick
        | InputActionKind::MouseDoubleClick
        | InputActionKind::MouseDrag
        | InputActionKind::Scroll => InputDispatchRoute::TargetPid,
    }
}

fn default_primary_route_for_action(action: InputActionKind) -> InputDispatchRoute {
    dispatch_route_for_action(action)
}

fn default_route_reason_for_action(
    action: InputActionKind,
    route: InputDispatchRoute,
) -> &'static str {
    match action {
        InputActionKind::KeyboardText | InputActionKind::KeyboardKey => {
            "keyboard_actions_prefer_background_target_pid"
        }
        InputActionKind::MouseClick
        | InputActionKind::MouseSecondaryClick
        | InputActionKind::MouseDoubleClick
        | InputActionKind::MouseDrag
        | InputActionKind::Scroll => match route {
            InputDispatchRoute::TargetPid => "mouse_actions_default_target_pid",
            InputDispatchRoute::Hid => "mouse_actions_explicit_hid_fallback",
        },
    }
}

fn app_input_signals(target: &AppTarget, pid: i32) -> AppInputSignals {
    let elements = ax_elements_for_pid(pid).unwrap_or_default();
    AppInputSignals {
        has_embedded_web_runtime: bundle_has_embedded_web_runtime_markers(&target.app_id),
        ax_element_count: elements.len(),
        actionable_ax_element_count: elements.iter().filter(|element| element.actionable).count(),
        visible_window_actionable_ax_element_count: elements
            .iter()
            .filter(|element| {
                element.actionable
                    && element
                        .bounds
                        .map(|bounds| bounds.width > 1.0 && bounds.height > 1.0 && bounds.y > 30.0)
                        .unwrap_or(false)
            })
            .count(),
    }
}

fn bundle_has_embedded_web_runtime_markers(app_id: &str) -> bool {
    let Some(url) = installed_app_url(app_id) else {
        return false;
    };
    let Some(path) = url.path().map(|p| p.to_string()) else {
        return false;
    };
    let contents = PathBuf::from(path).join("Contents");
    let markers = [
        contents
            .join("Frameworks")
            .join("Electron Framework.framework"),
        contents
            .join("Frameworks")
            .join("Chromium Embedded Framework.framework"),
        contents
            .join("Frameworks")
            .join("QtWebEngineCore.framework"),
        contents
            .join("Frameworks")
            .join("QtWebEngineWidgets.framework"),
        contents
            .join("Frameworks")
            .join("QtWebEngineQuick.framework"),
    ];
    markers.iter().any(|path| path.exists())
}

fn running_application_for_pid(
    pid: i32,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>> {
    use objc2_app_kit::NSRunningApplication;
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
}

#[allow(deprecated)]
fn launch_app_background(target: &AppTarget) -> ProviderResult<AppLaunchResult> {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceLaunchOptions};
    use objc2_foundation::NSString;

    if resolve_pid(&target.app_id).is_ok() {
        return Ok(AppLaunchResult {
            target: target.clone(),
            launched: false,
            running: true,
        });
    }
    if installed_app_url(&target.app_id).is_none() {
        return Err(ProviderError::AppNotFound(target.app_id.clone()));
    }

    let workspace = NSWorkspace::sharedWorkspace();
    let bundle_id = NSString::from_str(&target.app_id);
    let options = NSWorkspaceLaunchOptions::Async
        | NSWorkspaceLaunchOptions::WithoutActivation
        | NSWorkspaceLaunchOptions::WithoutAddingToRecents;
    let launched = workspace
        .launchAppWithBundleIdentifier_options_additionalEventParamDescriptor_launchIdentifier(
            &bundle_id, options, None, None,
        );
    if !launched {
        return Err(ProviderError::Failed(format!(
            "failed to launch {}",
            target.app_id
        )));
    }

    let running = wait_for_running(&target.app_id, Duration::from_secs(3));
    if !running {
        return Err(ProviderError::Failed(format!(
            "launched {} but it did not report as running",
            target.app_id
        )));
    }
    Ok(AppLaunchResult {
        target: target.clone(),
        launched: true,
        running: true,
    })
}

fn wait_for_running(app_id: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if resolve_pid(app_id).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn running_application(
    app_id: &str,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSRunningApplication>> {
    use objc2_app_kit::NSRunningApplication;
    use objc2_foundation::NSString;
    let apps =
        NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(app_id));
    apps.iter().next()
}

fn raise_app_foreground(target: &AppTarget) -> ProviderResult<AppRaiseResult> {
    let mut launched = false;
    if !self_is_app_running(&target.app_id)? {
        launched = launch_app_background(target)?.launched;
    }
    let pid = resolve_pid(&target.app_id)?;
    let activation = activate_running_app(&target.app_id, pid);
    Ok(AppRaiseResult {
        target: target.clone(),
        launched,
        running: true,
        activated: activation.activated,
        visible: activation.visible,
    })
}

fn self_is_app_running(app_id: &AppId) -> ProviderResult<bool> {
    match resolve_pid(app_id) {
        Ok(_) => Ok(true),
        Err(ProviderError::AppNotFound(_)) if installed_app_url(app_id).is_some() => Ok(false),
        Err(err) => Err(err),
    }
}

fn wait_for_visible_window(pid: i32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if frontmost_window_for_pid(pid).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[derive(Debug, Clone, Copy)]
struct ActivationOutcome {
    activated: bool,
    visible: bool,
    reopen_attempted: bool,
    reopen_succeeded: bool,
}

fn activate_running_app(app_id: &str, pid: i32) -> ActivationOutcome {
    use objc2_app_kit::NSApplicationActivationOptions;

    let had_visible_window = frontmost_window_for_pid(pid).is_ok();
    let reopen_attempted = !had_visible_window;
    let reopen_succeeded = if reopen_attempted {
        reopen_running_app_windows(app_id)
    } else {
        false
    };

    restore_minimized_ax_windows(pid);

    let activated = running_application(app_id)
        .map(|app| {
            let _ = app.unhide();
            #[allow(deprecated)]
            app.activateWithOptions(
                NSApplicationActivationOptions::ActivateAllWindows
                    | NSApplicationActivationOptions::ActivateIgnoringOtherApps,
            )
        })
        .unwrap_or(false);

    restore_minimized_ax_windows(pid);
    let visible = wait_for_visible_window(pid, Duration::from_secs(2));

    crate::computer_use::diagnostics::write(
        "app_activation",
        serde_json::json!({
            "appId": app_id,
            "pid": pid,
            "hadVisibleWindow": had_visible_window,
            "reopenAttempted": reopen_attempted,
            "reopenSucceeded": reopen_succeeded,
            "activated": activated,
            "visible": visible,
        }),
    );

    ActivationOutcome {
        activated,
        visible,
        reopen_attempted,
        reopen_succeeded,
    }
}

fn reopen_running_app_windows(app_id: &str) -> bool {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace, NSWorkspaceOpenConfiguration};

    let Some(app_url) = installed_app_url(app_id) else {
        crate::computer_use::diagnostics::write(
            "app_reopen_request",
            serde_json::json!({
                "appId": app_id,
                "requested": false,
                "reason": "app_url_not_found",
            }),
        );
        return false;
    };

    let config = NSWorkspaceOpenConfiguration::configuration();
    config.setActivates(true);
    config.setAddsToRecentItems(false);
    config.setCreatesNewApplicationInstance(false);
    config.setAllowsRunningApplicationSubstitution(true);

    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<i32, String>>(1);
    let tx = Arc::new(Mutex::new(Some(tx)));
    let block = RcBlock::new({
        let tx = Arc::clone(&tx);
        move |app: *mut NSRunningApplication, error: *mut NSError| {
            let result = if !error.is_null() {
                Err(unsafe { ns_error_description(&*error) })
            } else if app.is_null() {
                Err("NSWorkspace returned no running application".to_string())
            } else {
                Ok(unsafe { (*app).processIdentifier() })
            };
            if let Ok(mut sender) = tx.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(result);
                }
            }
        }
    });

    NSWorkspace::sharedWorkspace().openApplicationAtURL_configuration_completionHandler(
        &app_url,
        &config,
        Some(&*block),
    );

    let reopen_result = match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(Ok(returned_pid)) => Ok(returned_pid),
        Ok(Err(error)) => Err(error),
        Err(error) => Err(format!("timed out waiting for NSWorkspace reopen: {error}")),
    };

    let succeeded = reopen_result.is_ok();
    crate::computer_use::diagnostics::write(
        "app_reopen_request",
        serde_json::json!({
            "appId": app_id,
            "requested": true,
            "succeeded": succeeded,
            "result": match reopen_result {
                Ok(returned_pid) => serde_json::json!({ "pid": returned_pid }),
                Err(error) => serde_json::json!({ "error": error }),
            },
        }),
    );
    succeeded
}

// --- Window discovery + capture ------------------------------------------

fn frontmost_window_for_pid(pid: i32) -> ProviderResult<(u32, Option<Rect>)> {
    use core_graphics::window::{
        kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerPID,
    };

    let windows = CGDisplay::window_list_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        None,
    )
    .ok_or_else(|| ProviderError::Failed("could not enumerate windows".into()))?;

    let key_owner_pid = unsafe { kCGWindowOwnerPID as *const c_void };
    let key_layer = unsafe { kCGWindowLayer as *const c_void };
    let key_alpha = unsafe { kCGWindowAlpha as *const c_void };
    let key_bounds = unsafe { kCGWindowBounds as *const c_void };
    let key_number = unsafe { kCGWindowNumber as *const c_void };

    fn dict_value(dict: &CFDictionary, key: *const c_void) -> Option<CFType> {
        dict.find(key)
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
    }
    fn dict_i64(dict: &CFDictionary, key: *const c_void) -> Option<i64> {
        dict_value(dict, key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_i64())
    }
    fn dict_f64(dict: &CFDictionary, key: *const c_void) -> Option<f64> {
        dict_value(dict, key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|v| v.to_f64())
    }

    for value in &windows {
        let cf_type = unsafe { CFType::wrap_under_get_rule(*value) };
        let Some(dict) = cf_type.downcast::<CFDictionary>() else {
            continue;
        };
        if dict_i64(&dict, key_owner_pid).unwrap_or_default() != i64::from(pid) {
            continue;
        }
        if dict_i64(&dict, key_layer).unwrap_or_default() != 0 {
            continue;
        }
        if dict_f64(&dict, key_alpha).unwrap_or(1.0) <= 0.0 {
            continue;
        }
        let bounds = dict_value(&dict, key_bounds)
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|d| CGRect::from_dict_representation(&d))
            .map(|r| Rect {
                x: r.origin.x as f32,
                y: r.origin.y as f32,
                width: r.size.width as f32,
                height: r.size.height as f32,
            });
        if let Some(b) = &bounds {
            if b.width < 2.0 || b.height < 2.0 {
                continue;
            }
        }
        if let Some(window_id) = dict_i64(&dict, key_number).and_then(|v| u32::try_from(v).ok()) {
            return Ok((window_id, bounds));
        }
    }
    Err(ProviderError::NoVisibleWindow)
}

fn capture_window(
    dir: &PathBuf,
    window_id: u32,
    screen_bounds: Rect,
) -> ProviderResult<ScreenshotRef> {
    std::fs::create_dir_all(dir)
        .map_err(|e| ProviderError::Failed(format!("create capture dir: {e}")))?;

    match capture_window_with_sck(dir, window_id, screen_bounds) {
        Ok(screenshot) => Ok(screenshot),
        Err(err) => {
            log::debug!(
                "ScreenCaptureKit window capture failed for {window_id}; falling back to screencapture: {err}"
            );
            capture_window_with_screencapture(dir, window_id, screen_bounds)
        }
    }
}

fn capture_window_with_screencapture(
    dir: &Path,
    window_id: u32,
    screen_bounds: Rect,
) -> ProviderResult<ScreenshotRef> {
    let path = capture_output_path(dir, window_id, "png");
    let image = cg_window_image(window_id, cg_rect(screen_bounds))?;
    write_cg_image_png(&image, &path)?;
    normalize_capture_image_to_screen_points(&path, screen_bounds)?;
    screenshot_ref_from_path(&path, screen_bounds, Some(ScreenshotCaptureKind::WindowCg))
}

fn capture_window_with_sck(
    dir: &Path,
    window_id: u32,
    screen_bounds: Rect,
) -> ProviderResult<ScreenshotRef> {
    let path = capture_output_path(dir, window_id, "sck.png");
    let image = sck_capture_window_image(window_id, screen_bounds)?;
    write_cg_image_png(&image, &path)?;
    normalize_capture_image_to_screen_points(&path, screen_bounds)?;
    screenshot_ref_from_path(&path, screen_bounds, Some(ScreenshotCaptureKind::WindowSck))
}

fn capture_output_path(dir: &Path, window_id: u32, suffix: &str) -> PathBuf {
    dir.join(format!(
        "snapshot-{window_id}-{}.{suffix}",
        uuid::Uuid::new_v4().simple()
    ))
}

fn screenshot_ref_from_path(
    path: &Path,
    screen_bounds: Rect,
    capture_kind: Option<ScreenshotCaptureKind>,
) -> ProviderResult<ScreenshotRef> {
    let meta =
        std::fs::metadata(path).map_err(|e| ProviderError::Failed(format!("stat capture: {e}")))?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(path);
        return Err(ProviderError::Failed("empty capture".into()));
    }
    let (width, height) = image::image_dimensions(path)
        .map_err(|e| ProviderError::Failed(format!("decode capture dimensions: {e}")))?;
    Ok(ScreenshotRef {
        handle: path.to_string_lossy().to_string(),
        format: "png".into(),
        byte_len: meta.len(),
        width,
        height,
        default_coordinate_space: CoordinateSpace::Screenshot,
        capture_kind,
        screen_bounds,
        click_marker: None,
    })
}

fn normalize_capture_image_to_screen_points(
    path: &Path,
    screen_bounds: Rect,
) -> ProviderResult<()> {
    let (current_width, current_height) = image::image_dimensions(path)
        .map_err(|e| ProviderError::Failed(format!("decode capture dimensions: {e}")))?;
    let Some((target_width, target_height)) =
        normalized_capture_size(current_width, current_height, screen_bounds)
    else {
        return Ok(());
    };

    let image = ImageReader::open(path)
        .map_err(|e| ProviderError::Failed(format!("open capture for resize: {e}")))?
        .with_guessed_format()
        .map_err(|e| ProviderError::Failed(format!("guess capture format: {e}")))?
        .decode()
        .map_err(|e| ProviderError::Failed(format!("decode capture for resize: {e}")))?;
    let resized = image.resize_exact(target_width, target_height, FilterType::Triangle);
    let temp_path = path.with_extension("normalized.png");
    let file = File::create(&temp_path)
        .map_err(|e| ProviderError::Failed(format!("create normalized capture: {e}")))?;
    let mut writer = BufWriter::new(file);
    resized
        .write_to(&mut writer, ImageFormat::Png)
        .map_err(|e| ProviderError::Failed(format!("write normalized capture: {e}")))?;
    std::fs::rename(&temp_path, path)
        .map_err(|e| ProviderError::Failed(format!("replace normalized capture: {e}")))?;
    Ok(())
}

fn normalized_capture_size(
    current_width: u32,
    current_height: u32,
    screen_bounds: Rect,
) -> Option<(u32, u32)> {
    let target_width = logical_capture_dimension(screen_bounds.width)?;
    let target_height = logical_capture_dimension(screen_bounds.height)?;
    if current_width == target_width && current_height == target_height {
        return None;
    }
    if current_width < target_width || current_height < target_height {
        return None;
    }

    let width_scale = current_width as f32 / target_width as f32;
    let height_scale = current_height as f32 / target_height as f32;
    if !width_scale.is_finite() || !height_scale.is_finite() {
        return None;
    }
    if (width_scale - height_scale).abs() > 0.08 {
        return None;
    }
    Some((target_width, target_height))
}

fn logical_capture_dimension(value: f32) -> Option<u32> {
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let rounded = value.round();
    if rounded < 1.0 || rounded > u32::MAX as f32 {
        return None;
    }
    Some(rounded as u32)
}

fn write_cg_image_png(image: &core_graphics::image::CGImage, path: &Path) -> ProviderResult<()> {
    let cg_image = unsafe { &*(image.as_ptr() as *const CGImage) };
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), cg_image);
    let properties = NSDictionary::<objc2_app_kit::NSBitmapImageRepPropertyKey, AnyObject>::new();
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .ok_or_else(|| ProviderError::Failed("ScreenCaptureKit PNG encoding failed".into()))?;
    let path_string = objc2_foundation::NSString::from_str(&path.to_string_lossy());
    if png.writeToFile_atomically(&path_string, true) {
        Ok(())
    } else {
        Err(ProviderError::Failed(
            "ScreenCaptureKit PNG write failed".into(),
        ))
    }
}

fn cg_window_image(
    window_id: u32,
    bounds: CGRect,
) -> ProviderResult<core_graphics::image::CGImage> {
    use core_graphics::window::{kCGWindowImageBestResolution, kCGWindowListOptionIncludingWindow};

    CGDisplay::screenshot(
        bounds,
        kCGWindowListOptionIncludingWindow,
        window_id,
        kCGWindowImageBestResolution,
    )
    .ok_or_else(|| {
        ProviderError::Failed(format!(
            "CoreGraphics window capture failed for window {window_id}"
        ))
    })
}

#[allow(clippy::arc_with_non_send_sync)]
fn sck_capture_window_image(
    window_id: u32,
    screen_bounds: Rect,
) -> ProviderResult<core_graphics::image::CGImage> {
    let window = sck_window_for_window_id(window_id)?;
    let filter = sck_content_filter_for_window(&window)?;
    let config = sck_stream_configuration(screen_bounds)?;
    let (tx, rx) =
        std::sync::mpsc::sync_channel::<Result<core_graphics::image::CGImage, String>>(1);
    let tx = Arc::new(Mutex::new(Some(tx)));
    let block = RcBlock::new({
        let tx = Arc::clone(&tx);
        move |image: *mut CGImage, error: *mut NSError| {
            let result = if !error.is_null() {
                Err(unsafe { ns_error_description(&*error) })
            } else if image.is_null() {
                Err("ScreenCaptureKit returned no image".to_string())
            } else {
                let image = unsafe {
                    // The completion image is valid for the callback; retain it
                    // for the Rust value that crosses back to the waiting thread.
                    core_graphics::image::CGImage::from_ptr(CGImageRetain(
                        image as core_graphics::sys::CGImageRef,
                    ))
                };
                Ok(image)
            };
            if let Ok(mut sender) = tx.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(result);
                }
            }
        }
    });

    unsafe {
        let _: () = msg_send![
            class!(SCScreenshotManager),
            captureImageWithFilter: &*filter,
            configuration: &*config,
            completionHandler: &*block
        ];
    }

    match rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(image)) => Ok(image),
        Ok(Err(error)) => Err(ProviderError::Failed(error)),
        Err(error) => Err(ProviderError::Failed(format!(
            "ScreenCaptureKit capture timed out or disconnected: {error}"
        ))),
    }
}

#[allow(clippy::arc_with_non_send_sync)]
fn sck_window_for_window_id(window_id: u32) -> ProviderResult<Retained<AnyObject>> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Retained<AnyObject>, String>>(1);
    let tx = Arc::new(Mutex::new(Some(tx)));
    let block = RcBlock::new({
        let tx = Arc::clone(&tx);
        move |content: *mut AnyObject, error: *mut NSError| {
            let result = if !error.is_null() {
                Err(unsafe { ns_error_description(&*error) })
            } else if content.is_null() {
                Err("ScreenCaptureKit returned no shareable content".to_string())
            } else {
                unsafe { sck_window_from_shareable_content(&*content, window_id) }
            };
            if let Ok(mut sender) = tx.lock() {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(result);
                }
            }
        }
    });

    unsafe {
        let _: () = msg_send![
            class!(SCShareableContent),
            getShareableContentExcludingDesktopWindows: true,
            onScreenWindowsOnly: true,
            completionHandler: &*block
        ];
    }

    match rx.recv_timeout(Duration::from_secs(4)) {
        Ok(Ok(window)) => Ok(window),
        Ok(Err(error)) => Err(ProviderError::Failed(error)),
        Err(error) => Err(ProviderError::Failed(format!(
            "ScreenCaptureKit shareable-content lookup timed out or disconnected: {error}"
        ))),
    }
}

unsafe fn sck_window_from_shareable_content(
    content: &AnyObject,
    window_id: u32,
) -> Result<Retained<AnyObject>, String> {
    let windows: Retained<NSArray<AnyObject>> = unsafe { msg_send![content, windows] };
    for window in windows.iter() {
        let candidate_id: u32 = unsafe { msg_send![&*window, windowID] };
        if candidate_id == window_id {
            return Ok(window.retain());
        }
    }
    Err(format!(
        "ScreenCaptureKit could not find shareable window {window_id}"
    ))
}

fn sck_content_filter_for_window(window: &AnyObject) -> ProviderResult<Retained<AnyObject>> {
    let filter: Retained<AnyObject> = unsafe {
        msg_send![
            msg_send![class!(SCContentFilter), alloc],
            initWithDesktopIndependentWindow: window
        ]
    };
    Ok(filter)
}

fn sck_stream_configuration(screen_bounds: Rect) -> ProviderResult<Retained<AnyObject>> {
    let config: Retained<AnyObject> = unsafe { msg_send![class!(SCStreamConfiguration), new] };
    let scale = display_scale_from_capture_bounds(screen_bounds);
    let (width, height) = capture_pixel_size_for_bounds(screen_bounds, scale);
    unsafe {
        let _: () = msg_send![&*config, setWidth: width];
        let _: () = msg_send![&*config, setHeight: height];
        let _: () = msg_send![&*config, setShowsCursor: false];
        let _: () = msg_send![&*config, setScalesToFit: false];
        let _: () = msg_send![&*config, setQueueDepth: 1isize];
    }
    Ok(config)
}

fn display_scale_from_capture_bounds(screen_bounds: Rect) -> f32 {
    display_metadata_for_window_bounds(Some(screen_bounds))
        .scale
        .max(1.0)
}

fn capture_pixel_size_for_bounds(screen_bounds: Rect, scale: f32) -> (usize, usize) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (
        ((screen_bounds.width * scale).round() as usize).max(1),
        ((screen_bounds.height * scale).round() as usize).max(1),
    )
}

unsafe fn ns_error_description(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGImageRetain(image: core_graphics::sys::CGImageRef) -> core_graphics::sys::CGImageRef;
}

fn display_metadata_for_window_bounds(bounds: Option<Rect>) -> DisplayMetadata {
    let screen = screen_for_rect(bounds)
        .or_else(main_screen)
        .or_else(|| screen_for_cg_display_bounds(CGDisplay::main().bounds()));

    if let Some(screen) = screen {
        let frame = screen.frame();
        let scale = screen.backingScaleFactor() as f32;
        return DisplayMetadata {
            width: logical_dimension_to_pixels(frame.size.width, scale),
            height: logical_dimension_to_pixels(frame.size.height, scale),
            scale: sanitize_scale(scale),
        };
    }

    let main = CGDisplay::main();
    DisplayMetadata {
        width: main.pixels_wide() as u32,
        height: main.pixels_high() as u32,
        scale: 1.0,
    }
}

fn screen_for_rect(bounds: Option<Rect>) -> Option<Retained<NSScreen>> {
    let bounds = bounds?;
    let rect = ns_rect_from_rect(bounds)?;
    let center = NSPoint::new(
        rect.origin.x + (rect.size.width / 2.0),
        rect.origin.y + (rect.size.height / 2.0),
    );
    let mtm = MainThreadMarker::new()?;

    NSScreen::screens(mtm)
        .iter()
        .find(|screen| ns_rect_contains_point(screen.frame(), center))
        .map(|screen| screen.retain())
        .or_else(|| {
            screen_for_cg_display_bounds(
                cg_display_bounds_for_point(center).unwrap_or_else(|| CGDisplay::main().bounds()),
            )
        })
}

fn main_screen() -> Option<Retained<NSScreen>> {
    let mtm = MainThreadMarker::new()?;
    NSScreen::mainScreen(mtm)
}

fn screen_for_cg_display_bounds(bounds: CGRect) -> Option<Retained<NSScreen>> {
    let mtm = MainThreadMarker::new()?;
    NSScreen::screens(mtm)
        .iter()
        .find(|screen| ns_rect_nearly_equals_cg_rect(screen.frame(), bounds))
        .map(|screen| screen.retain())
}

fn cg_display_bounds_for_point(point: NSPoint) -> Option<CGRect> {
    let window_center = CGPoint::new(point.x, point.y);
    CGDisplay::active_displays().ok().and_then(|display_ids| {
        display_ids.into_iter().find_map(|display_id| {
            let display = CGDisplay::new(display_id);
            let bounds = display.bounds();
            cg_rect_contains_point(bounds, window_center).then_some(bounds)
        })
    })
}

fn ns_rect_from_rect(rect: Rect) -> Option<NSRect> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return None;
    }
    Some(NSRect::new(
        NSPoint::new(rect.x as f64, rect.y as f64),
        NSSize::new(rect.width as f64, rect.height as f64),
    ))
}

fn logical_dimension_to_pixels(value: f64, scale: f32) -> u32 {
    ((value * f64::from(sanitize_scale(scale))).round() as u32).max(1)
}

fn sanitize_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn ns_rect_contains_point(rect: NSRect, point: NSPoint) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x <= rect.origin.x + rect.size.width
        && point.y <= rect.origin.y + rect.size.height
}

fn cg_rect_contains_point(rect: CGRect, point: CGPoint) -> bool {
    point.x >= rect.origin.x
        && point.y >= rect.origin.y
        && point.x <= rect.origin.x + rect.size.width
        && point.y <= rect.origin.y + rect.size.height
}

fn ns_rect_nearly_equals_cg_rect(ns_rect: NSRect, cg_rect: CGRect) -> bool {
    (ns_rect.origin.x - cg_rect.origin.x).abs() < 1.0
        && (ns_rect.origin.y - cg_rect.origin.y).abs() < 1.0
        && (ns_rect.size.width - cg_rect.size.width).abs() < 1.0
        && (ns_rect.size.height - cg_rect.size.height).abs() < 1.0
}

fn display_screen_bounds(display: &DisplayMetadata) -> Rect {
    let scale = if display.scale > 0.0 {
        display.scale
    } else {
        1.0
    };
    Rect {
        x: 0.0,
        y: 0.0,
        width: display.width as f32 / scale,
        height: display.height as f32 / scale,
    }
}

fn elements_with_bounds(elements: Vec<UiElement>, _window_bounds: Option<Rect>) -> Vec<UiElement> {
    // Element bounds come from AX directly (screen coordinates); window bounds
    // are retained for future relative-coordinate work.
    elements
}

// --- Accessibility (AX) element tree -------------------------------------

#[allow(non_upper_case_globals)]
mod ax {
    use super::*;
    use core_foundation::base::CFTypeRef;
    use core_foundation::string::CFStringRef;

    pub type AXUIElementRef = *const c_void;
    pub type AXError = i32;
    pub const kAXErrorSuccess: AXError = 0;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
        pub fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        pub fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> AXError;
    }

    // AX attribute names. Apple exposes these as exported `CFStringRef` statics
    // (kAXRoleAttribute, …), but those live in the HIServices sub-framework and
    // do not reliably resolve at link time across SDK layouts (notably the bare
    // `--lib` test binary). The underlying values are stable, documented
    // strings, so we construct them ourselves instead of importing the statics.
    pub const ROLE: &str = "AXRole";
    pub const TITLE: &str = "AXTitle";
    pub const CHILDREN: &str = "AXChildren";
    pub const POSITION: &str = "AXPosition";
    pub const SIZE: &str = "AXSize";
    pub const ENABLED: &str = "AXEnabled";
    pub const VALUE: &str = "AXValue";
    pub const WINDOWS: &str = "AXWindows";
    pub const MINIMIZED: &str = "AXMinimized";
    pub const ENHANCED_USER_INTERFACE: &str = "AXEnhancedUserInterface";
    pub const MANUAL_ACCESSIBILITY: &str = "AXManualAccessibility";
    pub const PRESS_ACTION: &str = "AXPress";
    pub const RAISE_ACTION: &str = "AXRaise";
    pub const SHOW_MENU_ACTION: &str = "AXShowMenu";
    pub const SCROLL_TO_VISIBLE_ACTION: &str = "AXScrollToVisible";
    pub const SCROLL_UP_ACTION: &str = "AXScrollUp";
    pub const SCROLL_DOWN_ACTION: &str = "AXScrollDown";
    pub const SCROLL_LEFT_ACTION: &str = "AXScrollLeft";
    pub const SCROLL_RIGHT_ACTION: &str = "AXScrollRight";

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        pub fn AXUIElementPerformAction(element: AXUIElementRef, action: CFStringRef) -> AXError;
    }
}

/// Walk the AX tree for the app's current UI surface and flatten the full set
/// of discovered nodes. `actionable` is only a marker; non-actionable nodes are
/// still surfaced so menus, groups, labels, and other contextual structure are
/// available to the model.
fn ax_elements_for_pid(pid: i32) -> Option<Vec<UiElement>> {
    use core_foundation::base::CFTypeRef;
    let root = unsafe { ax::AXUIElementCreateApplication(pid) };
    if root.is_null() {
        return None;
    }
    enable_electron_accessibility_flags(root);
    let mut out = Vec::new();
    let mut next_id = 0u64;
    if let Some(entries) = ax_traversal_roots(root) {
        for entry in entries {
            walk_ax(entry, 0, &mut next_id, &mut out);
            unsafe { cf_release(entry as CFTypeRef) };
        }
    } else {
        walk_ax(root, 0, &mut next_id, &mut out);
    }
    // Release the root we created.
    unsafe { cf_release(root as CFTypeRef) };
    Some(out)
}

const AX_MAX_DEPTH: usize = 12;
const AX_MAX_ELEMENTS: usize = 500;

fn walk_ax(element: ax::AXUIElementRef, depth: usize, next_id: &mut u64, out: &mut Vec<UiElement>) {
    if depth > AX_MAX_DEPTH || out.len() >= AX_MAX_ELEMENTS {
        return;
    }
    let role = ax_string_attr(element, ax::ROLE).unwrap_or_default();
    let label = ax_string_attr(element, ax::TITLE).or_else(|| ax_string_attr(element, ax::VALUE));
    let bounds = ax_bounds(element);
    let enabled = ax_bool_attr(element, ax::ENABLED).unwrap_or(true);
    let actionable = enabled && is_actionable_role(&role);

    *next_id += 1;
    out.push(UiElement {
        id: format!("ax-{}", *next_id),
        role: role.clone(),
        label,
        bounds,
        bounds_coordinate_space: bounds.map(|_| CoordinateSpace::Screen),
        actionable,
    });

    // Recurse into children.
    if let Some(children) = ax_children(element) {
        for child in children {
            walk_ax(child, depth + 1, next_id, out);
            unsafe { cf_release(child as core_foundation::base::CFTypeRef) };
        }
    }
}

fn is_actionable_role(role: &str) -> bool {
    matches!(
        role,
        "AXButton"
            | "AXCheckBox"
            | "AXRadioButton"
            | "AXMenuItem"
            | "AXLink"
            | "AXTextField"
            | "AXTextArea"
            | "AXPopUpButton"
            | "AXTab"
            | "AXSlider"
            | "AXComboBox"
    )
}

fn ax_string_attr(element: ax::AXUIElementRef, attr: &str) -> Option<String> {
    use core_foundation::base::CFTypeRef;
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let cf = unsafe { CFType::wrap_under_create_rule(value) };
    cf.downcast::<CFString>().map(|s| s.to_string())
}

fn ax_bool_attr(element: ax::AXUIElementRef, attr: &str) -> Option<bool> {
    use core_foundation::base::CFTypeRef;
    use core_foundation::boolean::CFBoolean;
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let cf = unsafe { CFType::wrap_under_create_rule(value) };
    cf.downcast::<CFBoolean>().map(|b| b.into())
}

fn ax_children(element: ax::AXUIElementRef) -> Option<Vec<ax::AXUIElementRef>> {
    let array = ax_array_attr(element, ax::CHILDREN)?;
    ax_retained_array_items(array)
}

fn ax_windows(element: ax::AXUIElementRef) -> Option<Vec<ax::AXUIElementRef>> {
    let array = ax_array_attr(element, ax::WINDOWS)?;
    ax_retained_array_items(array)
}

fn ax_traversal_roots(root: ax::AXUIElementRef) -> Option<Vec<ax::AXUIElementRef>> {
    let mut out = Vec::new();

    if let Some(focused) = ax_ui_element_attr(root, "AXFocusedWindow") {
        out.push(focused);
    }
    if let Some(main) = ax_ui_element_attr(root, "AXMainWindow") {
        push_unique_ax_element(&mut out, main);
    }
    if let Some(windows) = ax_windows(root) {
        for window in windows {
            push_unique_ax_element(&mut out, window);
        }
    }
    if let Some(all_windows) = ax_all_windows(root) {
        for window in all_windows {
            push_unique_ax_element(&mut out, window);
        }
    }
    if let Some(menu_bar) = ax_ui_element_attr(root, "AXMenuBar") {
        push_unique_ax_element(&mut out, menu_bar);
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn ax_all_windows(element: ax::AXUIElementRef) -> Option<Vec<ax::AXUIElementRef>> {
    let array = ax_array_attr(element, "AXAllWindows")?;
    ax_retained_array_items(array)
}

fn push_unique_ax_element(out: &mut Vec<ax::AXUIElementRef>, element: ax::AXUIElementRef) {
    let ptr = element as usize;
    if out.iter().any(|existing| *existing as usize == ptr) {
        unsafe { cf_release(element as core_foundation::base::CFTypeRef) };
        return;
    }
    out.push(element);
}

fn ax_retained_array_items(array: CFArray<*const c_void>) -> Option<Vec<ax::AXUIElementRef>> {
    let mut out = Vec::with_capacity(array.len() as usize);
    for item in array.iter() {
        if item.is_null() {
            continue;
        }
        // The returned CFArray owns its elements. Retain each child before the
        // array drops so recursive AX reads never operate on dangling refs.
        let child = *item as ax::AXUIElementRef;
        unsafe { cf_retain(child as core_foundation::base::CFTypeRef) };
        out.push(child);
    }
    Some(out)
}

fn ax_array_attr(element: ax::AXUIElementRef, attr: &str) -> Option<CFArray<*const c_void>> {
    use core_foundation::base::CFTypeRef;
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let cf = unsafe { CFType::wrap_under_create_rule(value) };
    if cf.type_of() != CFArray::<*const c_void>::type_id() {
        return None;
    }
    let array_ref = cf.as_CFTypeRef() as _;
    std::mem::forget(cf);
    Some(unsafe { CFArray::wrap_under_create_rule(array_ref) })
}

fn ax_ui_element_attr(element: ax::AXUIElementRef, attr: &str) -> Option<ax::AXUIElementRef> {
    use core_foundation::base::CFTypeRef;
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let cf = unsafe { CFType::wrap_under_create_rule(value) };
    let child = cf.as_CFTypeRef() as ax::AXUIElementRef;
    if child.is_null() {
        return None;
    }
    unsafe { cf_retain(child as CFTypeRef) };
    Some(child)
}

/// Read the on-screen bounds of an AX element via its position + size, which are
/// `AXValue`-wrapped `CGPoint` / `CGSize`. Returns `None` if unavailable.
fn ax_bounds(element: ax::AXUIElementRef) -> Option<Rect> {
    let position = ax_value_point(element, ax::POSITION)?;
    let size = ax_value_size(element, ax::SIZE)?;
    Some(Rect {
        x: position.0,
        y: position.1,
        width: size.0,
        height: size.1,
    })
}

#[allow(non_upper_case_globals, non_snake_case)]
mod axvalue {
    use super::*;
    use core_foundation::base::CFTypeRef;

    pub type AXValueRef = *const c_void;
    // AXValueType discriminants from <ApplicationServices/AXValue.h>.
    pub const kAXValueTypeCGPoint: u32 = 1;
    pub const kAXValueTypeCGSize: u32 = 2;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        pub fn AXValueGetTypeID() -> core_foundation::base::CFTypeID;
        pub fn AXValueGetValue(value: AXValueRef, the_type: u32, value_ptr: *mut c_void) -> bool;
    }

    pub unsafe fn as_axvalue(cf: CFTypeRef) -> AXValueRef {
        cf as AXValueRef
    }
}

fn ax_value_point(element: ax::AXUIElementRef, attr: &str) -> Option<(f32, f32)> {
    use core_foundation::base::CFTypeRef;
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let cf = unsafe { CFType::wrap_under_create_rule(value) };
    if cf.type_of() != unsafe { axvalue::AXValueGetTypeID() } {
        return None;
    }
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        axvalue::AXValueGetValue(
            axvalue::as_axvalue(cf.as_CFTypeRef()),
            axvalue::kAXValueTypeCGPoint,
            &mut point as *mut CGPoint as *mut c_void,
        )
    };
    if ok {
        Some((point.x as f32, point.y as f32))
    } else {
        None
    }
}

fn ax_value_size(element: ax::AXUIElementRef, attr: &str) -> Option<(f32, f32)> {
    use core_foundation::base::CFTypeRef;
    use core_graphics::geometry::CGSize;
    let attr = CFString::new(attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    let cf = unsafe { CFType::wrap_under_create_rule(value) };
    if cf.type_of() != unsafe { axvalue::AXValueGetTypeID() } {
        return None;
    }
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = unsafe {
        axvalue::AXValueGetValue(
            axvalue::as_axvalue(cf.as_CFTypeRef()),
            axvalue::kAXValueTypeCGSize,
            &mut size as *mut CGSize as *mut c_void,
        )
    };
    if ok {
        Some((size.width as f32, size.height as f32))
    } else {
        None
    }
}

/// Resolve the click center for an element id produced by `walk_ax`. Because the
/// AX tree is re-walked, we match by the same flat id ordering.
fn ax_element_center(pid: i32, element_id: &ElementId) -> ProviderResult<Option<CGPoint>> {
    let elements = ax_elements_for_pid(pid).unwrap_or_default();
    let element = elements.iter().find(|e| &e.id == element_id);
    match element.and_then(|e| {
        e.bounds
            .filter(|bounds| bounds.width > 0.0 && bounds.height > 0.0)
    }) {
        Some(b) => Ok(Some(CGPoint {
            x: (b.x + b.width / 2.0) as f64,
            y: (b.y + b.height / 2.0) as f64,
        })),
        None => Ok(None),
    }
}

fn execute_click_intent(
    capture_dir: &Path,
    _target: &AppTarget,
    pid: i32,
    intent: ClickIntent<'_>,
    route_hint: ClickDispatchRoute,
) -> ProviderResult<ClickExecutionResult> {
    let routes = click_route_plan(intent, route_hint);
    let probe_point = click_probe_point(pid, intent);
    let before = capture_window_fingerprint(capture_dir, pid, probe_point).ok();

    for route in routes {
        let outcome = attempt_click_route(capture_dir, _target, pid, intent, route, before)?;
        match outcome {
            ClickAttemptOutcome::Succeeded
            | ClickAttemptOutcome::NoEffect
            | ClickAttemptOutcome::Uncertain => {
                return Ok(ClickExecutionResult {
                    route: click_execution_route(route),
                    outcome: click_execution_outcome(route, outcome),
                    next_dispatch_route: None,
                });
            }
            ClickAttemptOutcome::Failed => {}
        }
    }

    Err(ProviderError::Failed(
        "all click routes failed to dispatch".into(),
    ))
}

fn click_route_plan(
    intent: ClickIntent<'_>,
    route_hint: ClickDispatchRoute,
) -> Vec<ClickRouteKind> {
    let mut routes = Vec::new();

    match route_hint {
        ClickDispatchRoute::Auto => {
            if matches!(intent, ClickIntent::Element(_)) {
                routes.push(ClickRouteKind::Ax);
            }
            routes.push(ClickRouteKind::TargetPid);
            routes.push(ClickRouteKind::Hid);
        }
        ClickDispatchRoute::Ax => {
            if matches!(intent, ClickIntent::Element(_)) {
                routes.push(ClickRouteKind::Ax);
            }
        }
        ClickDispatchRoute::TargetPid => {
            routes.push(ClickRouteKind::TargetPid);
        }
        ClickDispatchRoute::Hid => {
            routes.push(ClickRouteKind::Hid);
        }
    }

    dedupe_click_routes(&mut routes);
    routes
}

fn mouse_route_plan_for_point(route_hint: ClickDispatchRoute) -> Vec<InputDispatchRoute> {
    match route_hint {
        ClickDispatchRoute::Auto => vec![InputDispatchRoute::TargetPid, InputDispatchRoute::Hid],
        ClickDispatchRoute::TargetPid => vec![InputDispatchRoute::TargetPid],
        ClickDispatchRoute::Hid => vec![InputDispatchRoute::Hid],
        ClickDispatchRoute::Ax => Vec::new(),
    }
}

fn mouse_route_plan_for_element(route_hint: ClickDispatchRoute) -> (bool, Vec<InputDispatchRoute>) {
    match route_hint {
        ClickDispatchRoute::Auto => (
            true,
            vec![InputDispatchRoute::TargetPid, InputDispatchRoute::Hid],
        ),
        ClickDispatchRoute::Ax => (true, Vec::new()),
        ClickDispatchRoute::TargetPid => (false, vec![InputDispatchRoute::TargetPid]),
        ClickDispatchRoute::Hid => (false, vec![InputDispatchRoute::Hid]),
    }
}

fn click_execution_route(route: ClickRouteKind) -> ClickExecutionRoute {
    match route {
        ClickRouteKind::Ax => ClickExecutionRoute::Ax,
        ClickRouteKind::TargetPid => ClickExecutionRoute::TargetPid,
        ClickRouteKind::Hid => ClickExecutionRoute::Hid,
    }
}

fn click_execution_outcome(
    route: ClickRouteKind,
    attempt_outcome: ClickAttemptOutcome,
) -> ClickExecutionOutcome {
    match (route, attempt_outcome) {
        (ClickRouteKind::Ax, ClickAttemptOutcome::Succeeded) => {
            ClickExecutionOutcome::SemanticSuccess
        }
        (_, ClickAttemptOutcome::Succeeded) => ClickExecutionOutcome::ObservedEffect,
        (_, ClickAttemptOutcome::NoEffect) => ClickExecutionOutcome::NoEffect,
        (_, ClickAttemptOutcome::Uncertain) => ClickExecutionOutcome::Uncertain,
        (_, ClickAttemptOutcome::Failed) => ClickExecutionOutcome::NoEffect,
    }
}

fn dedupe_click_routes(routes: &mut Vec<ClickRouteKind>) {
    let mut deduped = Vec::with_capacity(routes.len());
    for route in routes.iter().copied() {
        if !deduped.contains(&route) {
            deduped.push(route);
        }
    }
    *routes = deduped;
}

fn attempt_click_route(
    capture_dir: &Path,
    target: &AppTarget,
    pid: i32,
    intent: ClickIntent<'_>,
    route: ClickRouteKind,
    before: Option<WindowCaptureFingerprint>,
) -> ProviderResult<ClickAttemptOutcome> {
    diagnostics::write(
        "click_route_attempt",
        serde_json::json!({
            "appId": target.app_id,
            "pid": pid,
            "route": match route {
                ClickRouteKind::Ax => "ax",
                ClickRouteKind::TargetPid => "target_pid",
                ClickRouteKind::Hid => "hid",
            },
            "intent": match intent {
                ClickIntent::Element(_) => "element",
                ClickIntent::Point(_) => "point",
            },
        }),
    );

    let attempt = match route {
        ClickRouteKind::Ax => attempt_ax_click(target, pid, intent),
        ClickRouteKind::TargetPid => attempt_routed_click(
            target,
            pid,
            intent,
            InputDispatchRoute::TargetPid,
            capture_dir,
            before,
        ),
        ClickRouteKind::Hid => attempt_routed_click(
            target,
            pid,
            intent,
            InputDispatchRoute::Hid,
            capture_dir,
            before,
        ),
    };

    if let Err(error) = &attempt {
        diagnostics::write(
            "click_route_attempt_failed",
            serde_json::json!({
                "appId": target.app_id,
                "pid": pid,
                "route": match route {
                    ClickRouteKind::Ax => "ax",
                    ClickRouteKind::TargetPid => "target_pid",
                    ClickRouteKind::Hid => "hid",
                },
                "error": error.to_string(),
            }),
        );
    }

    attempt
}

fn attempt_ax_click(
    _target: &AppTarget,
    pid: i32,
    intent: ClickIntent<'_>,
) -> ProviderResult<ClickAttemptOutcome> {
    let ClickIntent::Element(element_id) = intent else {
        return Ok(ClickAttemptOutcome::Failed);
    };
    let action = perform_ax_action_for_id(pid, element_id, ax::PRESS_ACTION)?;
    if action == ax::kAXErrorSuccess {
        Ok(ClickAttemptOutcome::Succeeded)
    } else {
        Ok(ClickAttemptOutcome::Failed)
    }
}

fn attempt_routed_click(
    target: &AppTarget,
    pid: i32,
    intent: ClickIntent<'_>,
    route: InputDispatchRoute,
    capture_dir: &Path,
    before: Option<WindowCaptureFingerprint>,
) -> ProviderResult<ClickAttemptOutcome> {
    let point = resolved_click_point(pid, intent)?;

    if let Err(error) = left_click_at(
        event_target_for_route(target, pid, InputActionKind::MouseClick, route, None),
        point,
    ) {
        diagnostics::write(
            "click_route_dispatch_failed",
            serde_json::json!({
                "appId": target.app_id,
                "pid": pid,
                "route": route,
                "error": error.to_string(),
            }),
        );
        return Ok(ClickAttemptOutcome::Failed);
    }

    match click_effect_probe(capture_dir, pid, before, Some(point))? {
        ClickEffectProbeOutcome::ObservedEffect => Ok(ClickAttemptOutcome::Succeeded),
        ClickEffectProbeOutcome::NoEffect => Ok(ClickAttemptOutcome::NoEffect),
        ClickEffectProbeOutcome::Uncertain => Ok(ClickAttemptOutcome::Uncertain),
    }
}

fn execute_routed_action_with_effect_probe(
    capture_dir: &Path,
    target: &AppTarget,
    pid: i32,
    action: InputActionKind,
    probe_point: Option<CGPoint>,
    routes: Vec<InputDispatchRoute>,
    mut perform: impl FnMut(InputDispatchRoute) -> ProviderResult<()>,
) -> ProviderResult<RoutedActionExecution> {
    let before = capture_window_fingerprint(capture_dir, pid, probe_point).ok();
    for route in routes {
        diagnostics::write(
            "routed_action_attempt",
            serde_json::json!({
                "appId": target.app_id,
                "pid": pid,
                "action": match action {
                    InputActionKind::MouseClick => "mouse_click",
                    InputActionKind::MouseSecondaryClick => "mouse_secondary_click",
                    InputActionKind::MouseDoubleClick => "mouse_double_click",
                    InputActionKind::MouseDrag => "mouse_drag",
                    InputActionKind::Scroll => "scroll",
                    InputActionKind::KeyboardText => "keyboard_text",
                    InputActionKind::KeyboardKey => "keyboard_key",
                },
                "route": route,
            }),
        );

        let attempt = perform(route);
        if let Err(error) = &attempt {
            diagnostics::write(
                "routed_action_attempt_failed",
                serde_json::json!({
                    "appId": target.app_id,
                    "pid": pid,
                    "action": match action {
                        InputActionKind::MouseClick => "mouse_click",
                        InputActionKind::MouseSecondaryClick => "mouse_secondary_click",
                        InputActionKind::MouseDoubleClick => "mouse_double_click",
                        InputActionKind::MouseDrag => "mouse_drag",
                        InputActionKind::Scroll => "scroll",
                        InputActionKind::KeyboardText => "keyboard_text",
                        InputActionKind::KeyboardKey => "keyboard_key",
                    },
                    "route": route,
                    "error": error.to_string(),
                }),
            );
            continue;
        }

        match click_effect_probe(capture_dir, pid, before, probe_point)? {
            ClickEffectProbeOutcome::ObservedEffect => {
                return Ok(RoutedActionExecution {
                    route,
                    outcome: ClickAttemptOutcome::Succeeded,
                });
            }
            ClickEffectProbeOutcome::NoEffect => {
                return Ok(RoutedActionExecution {
                    route,
                    outcome: ClickAttemptOutcome::NoEffect,
                });
            }
            ClickEffectProbeOutcome::Uncertain => {
                return Ok(RoutedActionExecution {
                    route,
                    outcome: ClickAttemptOutcome::Uncertain,
                });
            }
        }
    }

    Err(ProviderError::Failed(
        "all routes failed to dispatch".into(),
    ))
}

fn action_execution_route(route: InputDispatchRoute) -> ActionExecutionRoute {
    match route {
        InputDispatchRoute::TargetPid => ActionExecutionRoute::TargetPid,
        InputDispatchRoute::Hid => ActionExecutionRoute::Hid,
    }
}

fn action_execution_outcome(outcome: ClickAttemptOutcome) -> ActionExecutionOutcome {
    match outcome {
        ClickAttemptOutcome::Succeeded => ActionExecutionOutcome::Dispatched,
        ClickAttemptOutcome::NoEffect => ActionExecutionOutcome::NoEffect,
        ClickAttemptOutcome::Uncertain => ActionExecutionOutcome::Uncertain,
        ClickAttemptOutcome::Failed => ActionExecutionOutcome::NoEffect,
    }
}

fn action_execution_result(
    kind: ActionExecutionKind,
    route: InputDispatchRoute,
    outcome: ClickAttemptOutcome,
) -> ActionExecutionResult {
    ActionExecutionResult {
        kind,
        route: action_execution_route(route),
        outcome: action_execution_outcome(outcome),
        next_dispatch_route: None,
    }
}

fn click_effect_probe(
    capture_dir: &Path,
    pid: i32,
    before: Option<WindowCaptureFingerprint>,
    probe_point: Option<CGPoint>,
) -> ProviderResult<ClickEffectProbeOutcome> {
    let Some(before) = before else {
        diagnostics::write(
            "click_effect_probe",
            serde_json::json!({
                "pid": pid,
                "outcome": "uncertain",
                "reason": "missing_before_fingerprint",
            }),
        );
        return Ok(ClickEffectProbeOutcome::Uncertain);
    };
    let timing = ClickEffectProbeTiming::DEFAULT;
    let attempts_total = timing.attempt_count();

    if timing.initial_delay_ms > 0 {
        thread::sleep(Duration::from_millis(timing.initial_delay_ms));
    }

    let mut saw_remote_only_change = false;
    for attempt_index in 0..attempts_total {
        if attempt_index > 0 && timing.poll_interval_ms > 0 {
            thread::sleep(Duration::from_millis(timing.poll_interval_ms));
        }

        let after = match capture_window_fingerprint(capture_dir, pid, probe_point) {
            Ok(after) => after,
            Err(error) => {
                diagnostics::write(
                    "click_effect_probe_capture_failed",
                    serde_json::json!({
                        "pid": pid,
                        "attempt": attempt_index + 1,
                        "attemptsTotal": attempts_total,
                        "error": error.to_string(),
                    }),
                );
                return Ok(ClickEffectProbeOutcome::Uncertain);
            }
        };

        let (outcome, local_changed, full_changed) = classify_click_effect_sample(before, after);
        if matches!(outcome, ClickEffectProbeOutcome::Uncertain) {
            saw_remote_only_change = true;
        }

        diagnostics::write(
            "click_effect_probe_attempt",
            serde_json::json!({
                "pid": pid,
                "attempt": attempt_index + 1,
                "attemptsTotal": attempts_total,
                "localHashAvailableBefore": before.local_hash.is_some(),
                "localHashAvailableAfter": after.local_hash.is_some(),
                "localChanged": local_changed,
                "fullChanged": full_changed,
                "outcome": match outcome {
                    ClickEffectProbeOutcome::ObservedEffect => "observed_effect",
                    ClickEffectProbeOutcome::NoEffect => "no_effect",
                    ClickEffectProbeOutcome::Uncertain => "uncertain",
                },
            }),
        );

        if matches!(outcome, ClickEffectProbeOutcome::ObservedEffect) {
            return Ok(outcome);
        }
    }

    if saw_remote_only_change {
        Ok(ClickEffectProbeOutcome::Uncertain)
    } else {
        Ok(ClickEffectProbeOutcome::NoEffect)
    }
}

fn classify_click_effect_sample(
    before: WindowCaptureFingerprint,
    after: WindowCaptureFingerprint,
) -> (ClickEffectProbeOutcome, Option<bool>, bool) {
    let local_changed = before.local_hash.zip(after.local_hash).map(|(a, b)| a != b);
    let full_changed = after.full_hash != before.full_hash;
    let outcome = if local_changed == Some(true) {
        ClickEffectProbeOutcome::ObservedEffect
    } else if full_changed {
        ClickEffectProbeOutcome::Uncertain
    } else {
        ClickEffectProbeOutcome::NoEffect
    };
    (outcome, local_changed, full_changed)
}

fn capture_window_fingerprint(
    capture_dir: &Path,
    pid: i32,
    probe_point: Option<CGPoint>,
) -> ProviderResult<WindowCaptureFingerprint> {
    let (window_id, bounds) = frontmost_window_for_pid(pid)?;
    let display = display_metadata_for_window_bounds(bounds);
    let screen_bounds = bounds.unwrap_or_else(|| display_screen_bounds(&display));
    let screenshot = capture_window(&capture_dir.to_path_buf(), window_id, screen_bounds)?;
    window_capture_fingerprint_from_screenshot(&screenshot, probe_point)
}

fn window_capture_fingerprint_from_screenshot(
    screenshot: &ScreenshotRef,
    probe_point: Option<CGPoint>,
) -> ProviderResult<WindowCaptureFingerprint> {
    let rgba = ImageReader::open(&screenshot.handle)
        .map_err(|e| ProviderError::Failed(format!("open capture for fingerprint: {e}")))?
        .with_guessed_format()
        .map_err(|e| ProviderError::Failed(format!("guess capture format for fingerprint: {e}")))?
        .decode()
        .map_err(|e| ProviderError::Failed(format!("decode capture for fingerprint: {e}")))?
        .to_rgba8();

    let _ = std::fs::remove_file(&screenshot.handle);

    let full_hash = hash_rgba_image(&rgba);
    let local_hash = probe_point.and_then(|point| {
        local_probe_rect(screenshot, point).map(|(x, y, width, height)| {
            let cropped = crop_imm(&rgba, x, y, width, height).to_image();
            hash_rgba_image(&cropped)
        })
    });

    Ok(WindowCaptureFingerprint {
        full_hash,
        local_hash,
    })
}

fn hash_rgba_image(image: &image::RgbaImage) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(image.width().to_le_bytes());
    hasher.update(image.height().to_le_bytes());
    hasher.update(image.as_raw());
    hasher.finalize().into()
}

fn local_probe_rect(
    screenshot: &ScreenshotRef,
    probe_point: CGPoint,
) -> Option<(u32, u32, u32, u32)> {
    if screenshot.width == 0
        || screenshot.height == 0
        || screenshot.screen_bounds.width <= 0.0
        || screenshot.screen_bounds.height <= 0.0
    {
        return None;
    }

    let relative_x = ((probe_point.x as f32 - screenshot.screen_bounds.x)
        / screenshot.screen_bounds.width)
        .clamp(0.0, 1.0);
    let relative_y = ((probe_point.y as f32 - screenshot.screen_bounds.y)
        / screenshot.screen_bounds.height)
        .clamp(0.0, 1.0);
    let center_x = (relative_x * screenshot.width as f32).round() as i32;
    let center_y = (relative_y * screenshot.height as f32).round() as i32;
    let radius = CLICK_EFFECT_LOCAL_PROBE_RADIUS_PX as i32;

    let left = (center_x - radius).clamp(0, screenshot.width as i32);
    let top = (center_y - radius).clamp(0, screenshot.height as i32);
    let right = (center_x + radius).clamp(0, screenshot.width as i32);
    let bottom = (center_y + radius).clamp(0, screenshot.height as i32);

    let width = u32::try_from(right.saturating_sub(left)).ok()?;
    let height = u32::try_from(bottom.saturating_sub(top)).ok()?;
    if width == 0 || height == 0 {
        return None;
    }

    Some((left as u32, top as u32, width, height))
}

fn click_probe_point(pid: i32, intent: ClickIntent<'_>) -> Option<CGPoint> {
    match intent {
        ClickIntent::Element(element_id) => ax_element_center(pid, element_id).ok().flatten(),
        ClickIntent::Point(point) => Some(point),
    }
}

fn resolved_click_point(pid: i32, intent: ClickIntent<'_>) -> ProviderResult<CGPoint> {
    match intent {
        ClickIntent::Element(element_id) => match ax_element_center(pid, element_id)? {
            Some(point) => Ok(point),
            None => Err(ProviderError::ElementNotFound(element_id.clone())),
        },
        ClickIntent::Point(point) => Ok(point),
    }
}

fn secondary_ax_element(
    capture_dir: &Path,
    target: &AppTarget,
    pid: i32,
    element_id: &ElementId,
    route_hint: ClickDispatchRoute,
) -> ProviderResult<ActionExecutionResult> {
    let (attempt_ax, fallback_routes) = mouse_route_plan_for_element(route_hint);
    crate::computer_use::diagnostics::write(
        "element_action_strategy",
        serde_json::json!({
            "appId": target.app_id,
            "pid": pid,
            "elementId": element_id,
            "action": "secondary_click",
            "profile": "operation_based",
            "usesAxAction": attempt_ax,
            "requestedDispatchRoute": route_hint,
            "fallbackRoutes": fallback_routes,
        }),
    );
    if attempt_ax {
        let action = perform_ax_action_for_id(pid, element_id, ax::SHOW_MENU_ACTION)?;
        if action == ax::kAXErrorSuccess {
            return Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SecondaryClick,
                route: ActionExecutionRoute::Ax,
                outcome: ActionExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            });
        }
    }
    let center = ax_element_center(pid, element_id)?
        .ok_or_else(|| ProviderError::ElementNotFound(element_id.clone()))?;
    execute_routed_action_with_effect_probe(
        capture_dir,
        target,
        pid,
        InputActionKind::MouseSecondaryClick,
        Some(center),
        fallback_routes,
        |route| {
            secondary_click_at(
                event_target_for_route(
                    target,
                    pid,
                    InputActionKind::MouseSecondaryClick,
                    route,
                    Some(default_route_reason_for_action(
                        InputActionKind::MouseSecondaryClick,
                        route,
                    )),
                ),
                center,
            )
        },
    )
    .map(|execution| {
        action_execution_result(
            ActionExecutionKind::SecondaryClick,
            execution.route,
            execution.outcome,
        )
    })
}

fn scroll_ax_element(
    capture_dir: &Path,
    target: &AppTarget,
    pid: i32,
    element_id: &ElementId,
    direction: ScrollDirection,
    amount: i32,
    route_hint: ClickDispatchRoute,
) -> ProviderResult<ActionExecutionResult> {
    let (attempt_ax, fallback_routes) = mouse_route_plan_for_element(route_hint);
    crate::computer_use::diagnostics::write(
        "element_action_strategy",
        serde_json::json!({
            "appId": target.app_id,
            "pid": pid,
            "elementId": element_id,
            "action": "scroll",
            "profile": "operation_based",
            "usesAxAction": attempt_ax,
            "requestedDispatchRoute": route_hint,
            "fallbackRoutes": fallback_routes,
        }),
    );
    if attempt_ax {
        let action = match direction {
            ScrollDirection::Up => ax::SCROLL_UP_ACTION,
            ScrollDirection::Down => ax::SCROLL_DOWN_ACTION,
            ScrollDirection::Left => ax::SCROLL_LEFT_ACTION,
            ScrollDirection::Right => ax::SCROLL_RIGHT_ACTION,
        };
        let result = perform_ax_action_for_id(pid, element_id, action)?;
        if result == ax::kAXErrorSuccess {
            return Ok(ActionExecutionResult {
                kind: ActionExecutionKind::Scroll,
                route: ActionExecutionRoute::Ax,
                outcome: ActionExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            });
        }
        let visible = perform_ax_action_for_id(pid, element_id, ax::SCROLL_TO_VISIBLE_ACTION)?;
        if visible == ax::kAXErrorSuccess {
            return Ok(ActionExecutionResult {
                kind: ActionExecutionKind::Scroll,
                route: ActionExecutionRoute::Ax,
                outcome: ActionExecutionOutcome::SemanticSuccess,
                next_dispatch_route: None,
            });
        }
    }
    execute_routed_action_with_effect_probe(
        capture_dir,
        target,
        pid,
        InputActionKind::Scroll,
        None,
        fallback_routes,
        |route| {
            scroll_wheel(
                event_target_for_route(
                    target,
                    pid,
                    InputActionKind::Scroll,
                    route,
                    Some(default_route_reason_for_action(
                        InputActionKind::Scroll,
                        route,
                    )),
                ),
                direction,
                amount,
            )
        },
    )
    .map(|execution| {
        action_execution_result(
            ActionExecutionKind::Scroll,
            execution.route,
            execution.outcome,
        )
    })
}

fn set_ax_value_for_id(
    pid: i32,
    element_id: &ElementId,
    value: &str,
) -> ProviderResult<ActionExecutionResult> {
    let result = with_ax_element_by_id(pid, element_id, |element| {
        let set_result = ax_set_string_value(element, value);
        let readback = ax_string_attr(element, ax::VALUE);
        Some((set_result, readback))
    })?;
    match result {
        (ax::kAXErrorSuccess, Some(current)) if current == value => Ok(ActionExecutionResult {
            kind: ActionExecutionKind::SetValue,
            route: ActionExecutionRoute::Ax,
            outcome: ActionExecutionOutcome::SemanticSuccess,
            next_dispatch_route: None,
        }),
        (ax::kAXErrorSuccess, Some(_)) => Ok(ActionExecutionResult {
            kind: ActionExecutionKind::SetValue,
            route: ActionExecutionRoute::Ax,
            outcome: ActionExecutionOutcome::NoEffect,
            next_dispatch_route: None,
        }),
        (ax::kAXErrorSuccess, None) => Ok(ActionExecutionResult {
            kind: ActionExecutionKind::SetValue,
            route: ActionExecutionRoute::Ax,
            outcome: ActionExecutionOutcome::Uncertain,
            next_dispatch_route: None,
        }),
        err => Err(ProviderError::Failed(format!(
            "set AXValue failed for {element_id}: AXError {}",
            err.0
        ))),
    }
}

fn perform_ax_action_for_id(
    pid: i32,
    element_id: &ElementId,
    action: &str,
) -> ProviderResult<ax::AXError> {
    with_ax_element_by_id(pid, element_id, |element| {
        let action = CFString::new(action);
        Some(unsafe { ax::AXUIElementPerformAction(element, action.as_concrete_TypeRef()) })
    })
}

fn with_ax_element_by_id<T>(
    pid: i32,
    element_id: &ElementId,
    mut f: impl FnMut(ax::AXUIElementRef) -> Option<T>,
) -> ProviderResult<T> {
    use core_foundation::base::CFTypeRef;
    let root = unsafe { ax::AXUIElementCreateApplication(pid) };
    if root.is_null() {
        return Err(ProviderError::ElementNotFound(element_id.clone()));
    }
    enable_electron_accessibility_flags(root);
    let mut next_id = 0u64;
    let result = with_ax_element_walk(root, 0, &mut next_id, element_id, &mut f);
    unsafe { cf_release(root as CFTypeRef) };
    result.ok_or_else(|| ProviderError::ElementNotFound(element_id.clone()))
}

fn with_ax_element_walk<T>(
    element: ax::AXUIElementRef,
    depth: usize,
    next_id: &mut u64,
    element_id: &ElementId,
    f: &mut impl FnMut(ax::AXUIElementRef) -> Option<T>,
) -> Option<T> {
    if depth > AX_MAX_DEPTH || *next_id as usize >= AX_MAX_ELEMENTS {
        return None;
    }
    *next_id += 1;
    if format!("ax-{}", *next_id) == *element_id {
        return f(element);
    }
    if let Some(children) = ax_children(element) {
        let mut found = None;
        for child in children {
            if found.is_none() {
                found = with_ax_element_walk(child, depth + 1, next_id, element_id, f);
            }
            unsafe { cf_release(child as core_foundation::base::CFTypeRef) };
        }
        if found.is_some() {
            return found;
        }
    }
    None
}

fn ax_set_string_value(element: ax::AXUIElementRef, value: &str) -> ax::AXError {
    let attr = CFString::new(ax::VALUE);
    let value = CFString::new(value);
    unsafe {
        ax::AXUIElementSetAttributeValue(element, attr.as_concrete_TypeRef(), value.as_CFTypeRef())
    }
}

fn enable_electron_accessibility_flags(root: ax::AXUIElementRef) {
    let value = CFBoolean::true_value();
    for attr in [ax::ENHANCED_USER_INTERFACE, ax::MANUAL_ACCESSIBILITY] {
        let attr = CFString::new(attr);
        let _ = unsafe {
            ax::AXUIElementSetAttributeValue(root, attr.as_concrete_TypeRef(), value.as_CFTypeRef())
        };
    }
}

fn restore_minimized_ax_windows(pid: i32) {
    use core_foundation::base::CFTypeRef;
    let root = unsafe { ax::AXUIElementCreateApplication(pid) };
    if root.is_null() {
        return;
    }
    enable_electron_accessibility_flags(root);
    restore_minimized_ax_windows_for_root(root, ax::WINDOWS);
    restore_minimized_ax_windows_for_root(root, "AXAllWindows");
    unsafe { cf_release(root as CFTypeRef) };
}

fn restore_minimized_ax_windows_for_root(root: ax::AXUIElementRef, windows_attr: &str) {
    let Some(array) = ax_array_attr(root, windows_attr) else {
        return;
    };
    let minimized = CFString::new(ax::MINIMIZED);
    let false_value = CFBoolean::false_value();
    let raise = CFString::new(ax::RAISE_ACTION);
    for item in array.iter() {
        if item.is_null() {
            continue;
        }
        let window = *item as ax::AXUIElementRef;
        let _ = unsafe {
            ax::AXUIElementSetAttributeValue(
                window,
                minimized.as_concrete_TypeRef(),
                false_value.as_CFTypeRef(),
            )
        };
        let _ = unsafe { ax::AXUIElementPerformAction(window, raise.as_concrete_TypeRef()) };
    }
}

unsafe fn cf_release(cf: core_foundation::base::CFTypeRef) {
    use core_foundation::base::CFRelease;
    if !cf.is_null() {
        CFRelease(cf);
    }
}

unsafe fn cf_retain(cf: core_foundation::base::CFTypeRef) {
    use core_foundation::base::CFRetain;
    if !cf.is_null() {
        CFRetain(cf);
    }
}

// --- Input injection (CGEvent) -------------------------------------------

fn event_source() -> ProviderResult<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|_| ProviderError::Failed("could not create CGEventSource".into()))
}

fn cg_point(point: Point) -> CGPoint {
    CGPoint {
        x: f64::from(point.x),
        y: f64::from(point.y),
    }
}

fn cg_rect(rect: Rect) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: f64::from(rect.x),
            y: f64::from(rect.y),
        },
        size: core_graphics::geometry::CGSize {
            width: f64::from(rect.width),
            height: f64::from(rect.height),
        },
    }
}

fn left_click_at(target: EventTarget, point: CGPoint) -> ProviderResult<()> {
    mouse_click_at(
        target,
        point,
        CGMouseButton::Left,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        1,
    )
}

fn secondary_click_at(target: EventTarget, point: CGPoint) -> ProviderResult<()> {
    mouse_click_at(
        target,
        point,
        CGMouseButton::Right,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        1,
    )
}

fn move_mouse_to(target: EventTarget, point: CGPoint) -> ProviderResult<()> {
    let source = event_source()?;
    let moved =
        CGEvent::new_mouse_event(source, CGEventType::MouseMoved, point, CGMouseButton::Left)
            .map_err(|_| ProviderError::Failed("create mouse-moved".into()))?;
    post_event_sequence(target, &[&moved]);
    Ok(())
}

fn mouse_click_at(
    target: EventTarget,
    point: CGPoint,
    button: CGMouseButton,
    down_type: CGEventType,
    up_type: CGEventType,
    click_state: i64,
) -> ProviderResult<()> {
    move_mouse_to(target, point)?;
    thread::sleep(Duration::from_millis(20));
    let source = event_source()?;
    let down = CGEvent::new_mouse_event(source.clone(), down_type, point, button)
        .map_err(|_| ProviderError::Failed("create mouse-down".into()))?;
    let up = CGEvent::new_mouse_event(source, up_type, point, button)
        .map_err(|_| ProviderError::Failed("create mouse-up".into()))?;
    down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    post_event_sequence(target, &[&down, &up]);
    Ok(())
}

fn double_click_at(target: EventTarget, point: CGPoint) -> ProviderResult<()> {
    move_mouse_to(target, point)?;
    thread::sleep(Duration::from_millis(20));
    let source = event_source()?;
    for click_state in [1, 2] {
        let down = CGEvent::new_mouse_event(
            source.clone(),
            CGEventType::LeftMouseDown,
            point,
            CGMouseButton::Left,
        )
        .map_err(|_| ProviderError::Failed("create double-click mouse-down".into()))?;
        let up = CGEvent::new_mouse_event(
            source.clone(),
            CGEventType::LeftMouseUp,
            point,
            CGMouseButton::Left,
        )
        .map_err(|_| ProviderError::Failed("create double-click mouse-up".into()))?;
        down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
        up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
        post_event_sequence(target, &[&down, &up]);
        thread::sleep(Duration::from_millis(40));
    }
    Ok(())
}

fn drag_between(target: EventTarget, from: CGPoint, to: CGPoint) -> ProviderResult<()> {
    move_mouse_to(target, from)?;
    thread::sleep(Duration::from_millis(20));
    let source = event_source()?;
    let down = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseDown,
        from,
        CGMouseButton::Left,
    )
    .map_err(|_| ProviderError::Failed("create drag mouse-down".into()))?;
    let drag = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseDragged,
        to,
        CGMouseButton::Left,
    )
    .map_err(|_| ProviderError::Failed("create mouse-dragged".into()))?;
    let up = CGEvent::new_mouse_event(source, CGEventType::LeftMouseUp, to, CGMouseButton::Left)
        .map_err(|_| ProviderError::Failed("create drag mouse-up".into()))?;
    let original_frontmost = frontmost_app();
    post_event(target, &down);
    thread::sleep(Duration::from_millis(20));
    post_event(target, &drag);
    thread::sleep(Duration::from_millis(20));
    post_event(target, &up);
    restore_frontmost_if_changed(original_frontmost);
    Ok(())
}

fn type_unicode(target: EventTarget, text: &str) -> ProviderResult<()> {
    let source = event_source()?;
    // Send paired key-down/key-up events so Chromium/Qt text inputs observe a
    // complete keyboard lifecycle instead of a lone unicode key-down.
    let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
        .map_err(|_| ProviderError::Failed("create keyboard key-down".into()))?;
    let up = CGEvent::new_keyboard_event(source, 0, false)
        .map_err(|_| ProviderError::Failed("create keyboard key-up".into()))?;
    down.set_string(text);
    up.set_string(text);
    post_event_sequence(target, &[&down, &up]);
    Ok(())
}

fn press_keycode(target: EventTarget, keycode: u16, flags: CGEventFlags) -> ProviderResult<()> {
    let source = event_source()?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| ProviderError::Failed("create key-down".into()))?;
    let up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| ProviderError::Failed("create key-up".into()))?;
    down.set_flags(flags);
    up.set_flags(flags);
    post_event_sequence(target, &[&down, &up]);
    Ok(())
}

fn scroll_wheel(
    target: EventTarget,
    direction: ScrollDirection,
    amount: i32,
) -> ProviderResult<()> {
    let source = event_source()?;
    let (dy, dx) = match direction {
        ScrollDirection::Up => (amount, 0),
        ScrollDirection::Down => (-amount, 0),
        ScrollDirection::Left => (0, amount),
        ScrollDirection::Right => (0, -amount),
    };
    let event = CGEvent::new_scroll_event(source, ScrollEventUnit::PIXEL, 2, dy, dx, 0)
        .map_err(|_| ProviderError::Failed("create scroll event".into()))?;
    post_event_sequence(target, &[&event]);
    Ok(())
}

fn post_event_sequence(target: EventTarget, events: &[&CGEvent]) {
    let original_frontmost = prepare_event_dispatch(target);
    for event in events {
        post_event(target, event);
    }
    finish_event_dispatch(target, original_frontmost);
}

fn post_event(target: EventTarget, event: &CGEvent) {
    match target.route {
        InputDispatchRoute::TargetPid => event.post_to_pid(target.pid),
        InputDispatchRoute::Hid => event.post(CGEventTapLocation::HID),
    }
}

fn prepare_event_dispatch(target: EventTarget) -> Option<FrontmostApp> {
    let original_frontmost = frontmost_app();
    let frontmost_before_pid = original_frontmost.as_ref().map(|app| app.pid);
    let frontmost_before_bundle_id = original_frontmost
        .as_ref()
        .and_then(|app| app_bundle_id(&app.app));
    let target_bundle_id =
        running_application_for_pid(target.pid).and_then(|app| app_bundle_id(&app));
    let was_frontmost = frontmost_before_pid == Some(target.pid);
    let activated = if target.ensure_frontmost && !was_frontmost {
        activate_target_pid(target.pid)
    } else {
        false
    };
    let activation_confirmed = if target.ensure_frontmost {
        wait_for_frontmost_pid(target.pid, Duration::from_millis(300))
    } else {
        false
    };
    if target.ensure_frontmost && activation_confirmed {
        thread::sleep(Duration::from_millis(30));
    }
    let current_frontmost = frontmost_app();
    crate::computer_use::diagnostics::write(
        "input_frontmost_prepare",
        serde_json::json!({
            "route": target.route,
            "targetPid": target.pid,
            "targetBundleId": target_bundle_id,
            "ensureFrontmost": target.ensure_frontmost,
            "restoreFrontmost": target.restore_frontmost,
            "wasFrontmost": was_frontmost,
            "activated": activated,
            "activationConfirmed": activation_confirmed,
            "frontmostBeforePid": frontmost_before_pid,
            "frontmostBeforeBundleId": frontmost_before_bundle_id,
            "frontmostAfterPid": current_frontmost.as_ref().map(|app| app.pid),
            "frontmostAfterBundleId": current_frontmost
                .as_ref()
                .and_then(|app| app_bundle_id(&app.app)),
        }),
    );
    original_frontmost
}

fn finish_event_dispatch(target: EventTarget, original_frontmost: Option<FrontmostApp>) {
    if !target.restore_frontmost {
        crate::computer_use::diagnostics::write(
            "input_frontmost_restore",
            serde_json::json!({
                "route": target.route,
                "targetPid": target.pid,
                "restoreFrontmost": false,
                "skipped": true,
                "currentFrontmostPid": frontmost_app().as_ref().map(|app| app.pid),
            }),
        );
        return;
    }

    let restored = restore_frontmost_if_changed(original_frontmost.clone());
    crate::computer_use::diagnostics::write(
        "input_frontmost_restore",
        serde_json::json!({
            "route": target.route,
            "targetPid": target.pid,
            "restoreFrontmost": true,
            "restored": restored,
            "originalFrontmostPid": original_frontmost.as_ref().map(|app| app.pid),
            "currentFrontmostPid": frontmost_app().as_ref().map(|app| app.pid),
        }),
    );
}

fn frontmost_app() -> Option<FrontmostApp> {
    use objc2_app_kit::NSWorkspace;
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(FrontmostApp {
        pid: app.processIdentifier(),
        app,
    })
}

fn app_bundle_id(app: &objc2::rc::Retained<objc2_app_kit::NSRunningApplication>) -> Option<String> {
    app.bundleIdentifier()
        .map(|bundle_id| bundle_id.to_string())
}

fn activate_target_pid(pid: i32) -> bool {
    let Some(app) = running_application_for_pid(pid) else {
        return false;
    };
    let Some(app_id) = app_bundle_id(&app) else {
        return false;
    };
    let activation = activate_running_app(&app_id, pid);
    crate::computer_use::diagnostics::write(
        "activate_target_pid",
        serde_json::json!({
            "appId": app_id,
            "pid": pid,
            "activated": activation.activated,
            "visible": activation.visible,
            "reopenAttempted": activation.reopen_attempted,
            "reopenSucceeded": activation.reopen_succeeded,
        }),
    );
    activation.activated
}

fn wait_for_frontmost_pid(pid: i32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if frontmost_app()
            .map(|current| current.pid == pid)
            .unwrap_or(false)
        {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

fn restore_frontmost_if_changed(original: Option<FrontmostApp>) -> bool {
    let Some(original) = original else {
        return false;
    };
    thread::sleep(Duration::from_millis(10));
    if frontmost_app()
        .map(|current| current.pid == original.pid)
        .unwrap_or(false)
    {
        return false;
    }
    let _ = original
        .app
        .activateWithOptions(objc2_app_kit::NSApplicationActivationOptions(0));
    true
}

/// Minimal keycode map for common named keys (US layout virtual keycodes).
fn keycode_and_flags_for(key: &str) -> Option<(u16, CGEventFlags)> {
    let mut flags = CGEventFlags::empty();
    let parts: Vec<&str> = key
        .split(['+', '-'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let key_part = if parts.is_empty() {
        key
    } else {
        let mut key_part = None;
        for part in parts {
            match part.to_ascii_lowercase().as_str() {
                "cmd" | "command" | "meta" => flags |= CGEventFlags::CGEventFlagCommand,
                "ctrl" | "control" => flags |= CGEventFlags::CGEventFlagControl,
                "shift" => flags |= CGEventFlags::CGEventFlagShift,
                "alt" | "option" => flags |= CGEventFlags::CGEventFlagAlternate,
                _ => key_part = Some(part),
            }
        }
        key_part?
    };
    keycode_for(key_part).map(|keycode| (keycode, flags))
}

fn keycode_for(key: &str) -> Option<u16> {
    let k = key.to_ascii_lowercase();
    Some(match k.as_str() {
        "a" => 0x00,
        "s" => 0x01,
        "d" => 0x02,
        "f" => 0x03,
        "h" => 0x04,
        "g" => 0x05,
        "z" => 0x06,
        "x" => 0x07,
        "c" => 0x08,
        "v" => 0x09,
        "b" => 0x0B,
        "q" => 0x0C,
        "w" => 0x0D,
        "e" => 0x0E,
        "r" => 0x0F,
        "y" => 0x10,
        "t" => 0x11,
        "1" => 0x12,
        "2" => 0x13,
        "3" => 0x14,
        "4" => 0x15,
        "6" => 0x16,
        "5" => 0x17,
        "=" => 0x18,
        "9" => 0x19,
        "7" => 0x1A,
        "-" => 0x1B,
        "8" => 0x1C,
        "0" => 0x1D,
        "o" => 0x1F,
        "u" => 0x20,
        "i" => 0x22,
        "p" => 0x23,
        "l" => 0x25,
        "j" => 0x26,
        "k" => 0x28,
        "n" => 0x2D,
        "m" => 0x2E,
        "return" | "enter" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "delete" | "backspace" => 0x33,
        "escape" | "esc" => 0x35,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        "home" => 0x73,
        "end" => 0x77,
        "pageup" => 0x74,
        "pagedown" => 0x79,
        "f1" => 0x7A,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycode_map_resolves_common_keys() {
        assert_eq!(keycode_for("Enter"), Some(0x24));
        assert_eq!(keycode_for("escape"), Some(0x35));
        assert_eq!(keycode_for("Up"), Some(0x7E));
        assert_eq!(keycode_for("s"), Some(0x01));
        assert_eq!(keycode_for("unknown-key"), None);
    }

    #[test]
    fn key_chord_parser_resolves_modifiers() {
        let (keycode, flags) = keycode_and_flags_for("cmd+s").unwrap();
        assert_eq!(keycode, 0x01);
        assert!(flags.contains(CGEventFlags::CGEventFlagCommand));

        let (keycode, flags) = keycode_and_flags_for("ctrl+shift+t").unwrap();
        assert_eq!(keycode, 0x11);
        assert!(flags.contains(CGEventFlags::CGEventFlagControl));
        assert!(flags.contains(CGEventFlags::CGEventFlagShift));
    }

    #[test]
    fn actionable_roles_are_a_conservative_set() {
        assert!(is_actionable_role("AXButton"));
        assert!(is_actionable_role("AXTextField"));
        assert!(is_actionable_role("AXMenuItem"));
        // Containers / static text are still captured, just not marked actionable.
        assert!(!is_actionable_role("AXGroup"));
        assert!(!is_actionable_role("AXStaticText"));
        assert!(!is_actionable_role(""));
    }

    #[test]
    fn provider_reports_control_supported() {
        assert!(MacosProvider::new().supports_control());
    }

    #[test]
    fn capture_pixel_size_uses_window_points_and_scale() {
        let bounds = Rect {
            x: 10.0,
            y: 20.0,
            width: 320.0,
            height: 180.0,
        };

        assert_eq!(capture_pixel_size_for_bounds(bounds, 2.0), (640, 360));
        assert_eq!(capture_pixel_size_for_bounds(bounds, 0.0), (320, 180));
    }

    #[test]
    fn normalized_capture_size_shrinks_retina_window_capture_to_logical_points() {
        let bounds = Rect {
            x: 0.0,
            y: 0.0,
            width: 1057.0,
            height: 752.0,
        };

        assert_eq!(
            normalized_capture_size(2114, 1504, bounds),
            Some((1057, 752))
        );
        assert_eq!(normalized_capture_size(1057, 752, bounds), None);
        assert_eq!(normalized_capture_size(1000, 700, bounds), None);
    }

    #[test]
    fn capture_output_path_is_unique_per_snapshot() {
        let dir = PathBuf::from("/tmp/sessio-computer-use-test");

        let first = capture_output_path(&dir, 42, "png");
        let second = capture_output_path(&dir, 42, "png");

        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(dir.as_path()));
        assert!(first
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .starts_with("snapshot-42-"));
    }

    #[test]
    fn cg_rect_preserves_window_bounds_geometry() {
        let bounds = Rect {
            x: 12.5,
            y: 34.0,
            width: 640.0,
            height: 360.0,
        };

        let rect = cg_rect(bounds);
        assert_eq!(rect.origin.x, 12.5);
        assert_eq!(rect.origin.y, 34.0);
        assert_eq!(rect.size.width, 640.0);
        assert_eq!(rect.size.height, 360.0);
    }

    #[test]
    fn mdls_utc_timestamp_parser_handles_raw_output() {
        assert_eq!(
            parse_mdls_utc_timestamp("2026-06-28 11:22:33 +0000"),
            Some(1782645753)
        );
        assert_eq!(parse_mdls_utc_timestamp("(null)"), None);
    }

    #[test]
    fn operation_based_routing_keeps_keyboard_on_pid() {
        assert_eq!(
            dispatch_route_for_action(InputActionKind::KeyboardKey),
            InputDispatchRoute::TargetPid
        );
        assert_eq!(
            dispatch_route_for_action(InputActionKind::KeyboardText),
            InputDispatchRoute::TargetPid
        );
    }

    #[test]
    fn operation_based_routing_keeps_mouse_actions_on_target_pid_by_default() {
        assert_eq!(
            dispatch_route_for_action(InputActionKind::MouseClick),
            InputDispatchRoute::TargetPid
        );
        assert_eq!(
            dispatch_route_for_action(InputActionKind::MouseSecondaryClick),
            InputDispatchRoute::TargetPid
        );
        assert_eq!(
            dispatch_route_for_action(InputActionKind::MouseDoubleClick),
            InputDispatchRoute::TargetPid
        );
        assert_eq!(
            dispatch_route_for_action(InputActionKind::MouseDrag),
            InputDispatchRoute::TargetPid
        );
        assert_eq!(
            dispatch_route_for_action(InputActionKind::Scroll),
            InputDispatchRoute::TargetPid
        );
    }

    #[test]
    fn click_route_plan_prefers_ax_then_pid_then_hid_for_element_intent() {
        let element_id = "el-1".to_string();
        assert_eq!(
            click_route_plan(ClickIntent::Element(&element_id), ClickDispatchRoute::Auto),
            vec![
                ClickRouteKind::Ax,
                ClickRouteKind::TargetPid,
                ClickRouteKind::Hid,
            ]
        );
    }

    #[test]
    fn click_route_plan_uses_pid_then_hid_for_point_intent() {
        assert_eq!(
            click_route_plan(
                ClickIntent::Point(CGPoint { x: 10.0, y: 20.0 }),
                ClickDispatchRoute::Auto
            ),
            vec![ClickRouteKind::TargetPid, ClickRouteKind::Hid]
        );
    }

    #[test]
    fn click_route_plan_supports_explicit_dispatch_route() {
        let element_id = "el-1".to_string();
        assert_eq!(
            click_route_plan(ClickIntent::Element(&element_id), ClickDispatchRoute::Ax),
            vec![ClickRouteKind::Ax]
        );
        assert_eq!(
            click_route_plan(ClickIntent::Element(&element_id), ClickDispatchRoute::Hid),
            vec![ClickRouteKind::Hid]
        );
        assert_eq!(
            click_route_plan(
                ClickIntent::Point(CGPoint { x: 10.0, y: 20.0 }),
                ClickDispatchRoute::TargetPid
            ),
            vec![ClickRouteKind::TargetPid]
        );
    }

    #[test]
    fn click_effect_sample_detects_local_change_as_observed_effect() {
        let before = WindowCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };
        let after = WindowCaptureFingerprint {
            full_hash: [3; 32],
            local_hash: Some([4; 32]),
        };

        let (outcome, local_changed, full_changed) = classify_click_effect_sample(before, after);
        assert_eq!(outcome, ClickEffectProbeOutcome::ObservedEffect);
        assert_eq!(local_changed, Some(true));
        assert!(full_changed);
    }

    #[test]
    fn click_effect_sample_treats_remote_only_change_as_uncertain() {
        let before = WindowCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };
        let after = WindowCaptureFingerprint {
            full_hash: [3; 32],
            local_hash: Some([2; 32]),
        };

        let (outcome, local_changed, full_changed) = classify_click_effect_sample(before, after);
        assert_eq!(outcome, ClickEffectProbeOutcome::Uncertain);
        assert_eq!(local_changed, Some(false));
        assert!(full_changed);
    }

    #[test]
    fn click_effect_sample_treats_no_change_as_no_effect() {
        let before = WindowCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };
        let after = before;

        let (outcome, local_changed, full_changed) = classify_click_effect_sample(before, after);
        assert_eq!(outcome, ClickEffectProbeOutcome::NoEffect);
        assert_eq!(local_changed, Some(false));
        assert!(!full_changed);
    }

    #[test]
    fn click_execution_outcome_preserves_no_effect_signal() {
        assert_eq!(
            click_execution_outcome(ClickRouteKind::TargetPid, ClickAttemptOutcome::NoEffect),
            ClickExecutionOutcome::NoEffect
        );
        assert_eq!(
            click_execution_outcome(ClickRouteKind::TargetPid, ClickAttemptOutcome::Uncertain),
            ClickExecutionOutcome::Uncertain
        );
    }

    #[test]
    fn fingerprint_capture_removes_temp_file_after_hashing() {
        use image::{ImageBuffer, Rgba};

        let dir = std::env::temp_dir().join(format!(
            "sessio-cu-fingerprint-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fingerprint.png");

        let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_pixel(4, 4, Rgba([0, 0, 0, 255]));
        image.save(&path).unwrap();

        let screenshot = ScreenshotRef {
            handle: path.to_string_lossy().into_owned(),
            format: "png".into(),
            byte_len: std::fs::metadata(&path).unwrap().len(),
            width: 4,
            height: 4,
            default_coordinate_space: CoordinateSpace::Screenshot,
            capture_kind: None,
            screen_bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            click_marker: None,
        };

        let fingerprint = window_capture_fingerprint_from_screenshot(
            &screenshot,
            Some(CGPoint { x: 2.0, y: 2.0 }),
        )
        .unwrap();

        assert_ne!(fingerprint.full_hash, [0; 32]);
        assert!(!Path::new(&screenshot.handle).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn click_effect_sample_treats_missing_local_probe_with_global_change_as_uncertain() {
        let before = WindowCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: None,
        };
        let after = WindowCaptureFingerprint {
            full_hash: [9; 32],
            local_hash: None,
        };

        let (outcome, local_changed, full_changed) = classify_click_effect_sample(before, after);
        assert_eq!(outcome, ClickEffectProbeOutcome::Uncertain);
        assert_eq!(local_changed, None);
        assert!(full_changed);
    }

    #[test]
    fn click_effect_sample_treats_missing_local_probe_without_global_change_as_no_effect() {
        let before = WindowCaptureFingerprint {
            full_hash: [7; 32],
            local_hash: None,
        };
        let after = before;

        let (outcome, local_changed, full_changed) = classify_click_effect_sample(before, after);
        assert_eq!(outcome, ClickEffectProbeOutcome::NoEffect);
        assert_eq!(local_changed, None);
        assert!(!full_changed);
    }

    #[test]
    fn recent_usage_merge_keeps_knowledge_count_and_newer_spotlight_time() {
        let mut usage = HashMap::from([(
            "com.example.app".to_string(),
            RecentUsage {
                count: 3,
                last_used_at: 100,
                source: "knowledgeC",
            },
        )]);
        merge_recent_usage(
            &mut usage,
            HashMap::from([(
                "com.example.app".to_string(),
                RecentUsage {
                    count: 1,
                    last_used_at: 200,
                    source: "spotlight",
                },
            )]),
        );

        let app = usage.get("com.example.app").unwrap();
        assert_eq!(app.count, 3);
        assert_eq!(app.last_used_at, 200);
        assert_eq!(app.source, "knowledgeC");
    }
}
