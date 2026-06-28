//! macOS implementation of [`ComputerUseProvider`].
//!
//! - **App enumeration**: `NSWorkspace.runningApplications`.
//! - **Screenshot**: the `screencapture` CLI targeting the app's frontmost
//!   on-screen window (same approach as Appshot's capture in `lib.rs`).
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
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

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

use crate::computer_use::provider::{
    AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppTarget, ComputerUseProvider,
    CoordinateSpace, DisplayMetadata, ElementId, InstalledApp, Point, ProviderError,
    ProviderResult, RawAppState, Rect, ScreenshotRef, ScrollDirection, UiElement,
};

#[derive(Clone)]
struct FrontmostApp {
    pid: i32,
    app: objc2::rc::Retained<objc2_app_kit::NSRunningApplication>,
}

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
        let display = display_metadata();
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

    fn click_element(&self, target: &AppTarget, element: &ElementId) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        click_ax_element(pid, element)
    }

    fn click_point(&self, target: &AppTarget, point: Point) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        left_click_at(Some(pid), cg_point(point))
    }

    fn secondary_click(&self, target: &AppTarget, point: Point) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        secondary_click_at(Some(pid), cg_point(point))
    }

    fn secondary_click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
    ) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        secondary_ax_element(pid, element)
    }

    fn double_click(&self, target: &AppTarget, point: Point) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        double_click_at(Some(pid), cg_point(point))
    }

    fn drag(&self, target: &AppTarget, from: Point, to: Point) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        drag_between(Some(pid), cg_point(from), cg_point(to))
    }

    fn set_value(
        &self,
        target: &AppTarget,
        element: &ElementId,
        value: &str,
    ) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        set_ax_value_for_id(pid, element, value)
    }

    fn type_text(&self, target: &AppTarget, text: &str) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        type_unicode(Some(pid), text)
    }

    fn press_key(&self, target: &AppTarget, key: &str) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        let (keycode, flags) = keycode_and_flags_for(key)
            .ok_or_else(|| ProviderError::Failed(format!("unknown key: {key}")))?;
        press_keycode(Some(pid), keycode, flags)
    }

    fn scroll(
        &self,
        target: &AppTarget,
        direction: ScrollDirection,
        amount: i32,
    ) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        scroll_wheel(Some(pid), direction, amount)
    }

    fn scroll_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        direction: ScrollDirection,
        amount: i32,
    ) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        scroll_ax_element(pid, element, direction, amount)
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
    use objc2_app_kit::NSApplicationActivationOptions;

    let mut launched = false;
    if !self_is_app_running(&target.app_id)? {
        launched = launch_app_background(target)?.launched;
    }
    let pid = resolve_pid(&target.app_id)?;
    restore_minimized_ax_windows(pid);
    let app = running_application(&target.app_id)
        .ok_or_else(|| ProviderError::AppNotFound(target.app_id.clone()))?;
    let _ = app.unhide();
    #[allow(deprecated)]
    let activated = app.activateWithOptions(
        NSApplicationActivationOptions::ActivateAllWindows
            | NSApplicationActivationOptions::ActivateIgnoringOtherApps,
    );
    restore_minimized_ax_windows(pid);
    let visible = wait_for_visible_window(pid, Duration::from_secs(2));
    Ok(AppRaiseResult {
        target: target.clone(),
        launched,
        running: true,
        activated,
        visible,
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
    let path = dir.join(format!("snapshot-{window_id}.png"));
    let status = std::process::Command::new("screencapture")
        .arg("-x") // no sound
        .arg("-o") // no window shadow
        .arg("-t")
        .arg("png")
        .arg("-l")
        .arg(window_id.to_string())
        .arg(&path)
        .status()
        .map_err(|e| ProviderError::Failed(format!("start screencapture: {e}")))?;
    if !status.success() {
        return Err(ProviderError::Failed(format!(
            "screencapture failed: {status}"
        )));
    }
    let meta = std::fs::metadata(&path)
        .map_err(|e| ProviderError::Failed(format!("stat capture: {e}")))?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(&path);
        return Err(ProviderError::Failed("empty capture".into()));
    }
    let (width, height) = image::image_dimensions(&path)
        .map_err(|e| ProviderError::Failed(format!("decode capture dimensions: {e}")))?;
    Ok(ScreenshotRef {
        handle: path.to_string_lossy().to_string(),
        format: "png".into(),
        byte_len: meta.len(),
        width,
        height,
        default_coordinate_space: CoordinateSpace::Screenshot,
        screen_bounds,
    })
}

fn display_metadata() -> DisplayMetadata {
    let main = CGDisplay::main();
    DisplayMetadata {
        width: main.pixels_wide() as u32,
        height: main.pixels_high() as u32,
        // CGDisplay does not expose the backing scale directly here; default to
        // 2.0 on Retina-era hardware. Refined against NSScreen in a later pass.
        scale: 2.0,
    }
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

/// Walk the AX tree for an app and flatten a conservative set of actionable
/// elements. Returns an empty vec on any AX failure (caller treats AX as
/// best-effort; absence of elements just means no element-targeted actions).
fn ax_elements_for_pid(pid: i32) -> Option<Vec<UiElement>> {
    use core_foundation::base::CFTypeRef;
    let root = unsafe { ax::AXUIElementCreateApplication(pid) };
    if root.is_null() {
        return None;
    }
    enable_electron_accessibility_flags(root);
    let mut out = Vec::new();
    let mut next_id = 0u64;
    walk_ax(root, 0, &mut next_id, &mut out);
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

    if is_actionable_role(&role) {
        *next_id += 1;
        out.push(UiElement {
            id: format!("ax-{}", *next_id),
            role: role.clone(),
            label,
            bounds,
            bounds_coordinate_space: bounds.map(|_| CoordinateSpace::Screen),
            actionable: enabled,
        });
    }

    // Recurse into children.
    if let Some(children) = ax_children(element) {
        for child in children {
            walk_ax(child, depth + 1, next_id, out);
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
    use core_foundation::base::CFTypeRef;
    let attr = CFString::new(ax::CHILDREN);
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        ax::AXUIElementCopyAttributeValue(element, attr.as_concrete_TypeRef(), &mut value)
    };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return None;
    }
    // value is a CFArray of AXUIElementRef. Take create-rule ownership of the
    // array; the elements are owned by the array.
    let array: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(value as _) };
    let mut out = Vec::with_capacity(array.len() as usize);
    for item in array.iter() {
        out.push(*item as ax::AXUIElementRef);
    }
    Some(out)
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
    let mut point = CGPoint { x: 0.0, y: 0.0 };
    let ok = unsafe {
        axvalue::AXValueGetValue(
            axvalue::as_axvalue(value),
            axvalue::kAXValueTypeCGPoint,
            &mut point as *mut CGPoint as *mut c_void,
        )
    };
    unsafe { cf_release(value) };
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
    let mut size = CGSize {
        width: 0.0,
        height: 0.0,
    };
    let ok = unsafe {
        axvalue::AXValueGetValue(
            axvalue::as_axvalue(value),
            axvalue::kAXValueTypeCGSize,
            &mut size as *mut CGSize as *mut c_void,
        )
    };
    unsafe { cf_release(value) };
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
    match element.and_then(|e| e.bounds) {
        Some(b) => Ok(Some(CGPoint {
            x: (b.x + b.width / 2.0) as f64,
            y: (b.y + b.height / 2.0) as f64,
        })),
        None => Ok(None),
    }
}

fn click_ax_element(pid: i32, element_id: &ElementId) -> ProviderResult<()> {
    let action = perform_ax_action_for_id(pid, element_id, ax::PRESS_ACTION)?;
    if action == ax::kAXErrorSuccess {
        return Ok(());
    }
    match ax_element_center(pid, element_id)? {
        Some(center) => left_click_at(Some(pid), center),
        None => Err(ProviderError::ElementNotFound(element_id.clone())),
    }
}

fn secondary_ax_element(pid: i32, element_id: &ElementId) -> ProviderResult<()> {
    let action = perform_ax_action_for_id(pid, element_id, ax::SHOW_MENU_ACTION)?;
    if action == ax::kAXErrorSuccess {
        return Ok(());
    }
    match ax_element_center(pid, element_id)? {
        Some(center) => secondary_click_at(Some(pid), center),
        None => Err(ProviderError::ElementNotFound(element_id.clone())),
    }
}

fn scroll_ax_element(
    pid: i32,
    element_id: &ElementId,
    direction: ScrollDirection,
    amount: i32,
) -> ProviderResult<()> {
    let action = match direction {
        ScrollDirection::Up => ax::SCROLL_UP_ACTION,
        ScrollDirection::Down => ax::SCROLL_DOWN_ACTION,
        ScrollDirection::Left => ax::SCROLL_LEFT_ACTION,
        ScrollDirection::Right => ax::SCROLL_RIGHT_ACTION,
    };
    let result = perform_ax_action_for_id(pid, element_id, action)?;
    if result == ax::kAXErrorSuccess {
        return Ok(());
    }
    let visible = perform_ax_action_for_id(pid, element_id, ax::SCROLL_TO_VISIBLE_ACTION)?;
    if visible == ax::kAXErrorSuccess {
        return Ok(());
    }
    scroll_wheel(Some(pid), direction, amount)
}

fn set_ax_value_for_id(pid: i32, element_id: &ElementId, value: &str) -> ProviderResult<()> {
    let result = with_ax_element_by_id(pid, element_id, |element| {
        Some(ax_set_string_value(element, value))
    })?;
    match result {
        ax::kAXErrorSuccess => Ok(()),
        err => Err(ProviderError::Failed(format!(
            "set AXValue failed for {element_id}: AXError {err}"
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
    let role = ax_string_attr(element, ax::ROLE).unwrap_or_default();
    if is_actionable_role(&role) {
        *next_id += 1;
        if format!("ax-{}", *next_id) == *element_id {
            return f(element);
        }
    }
    if let Some(children) = ax_children(element) {
        for child in children {
            if let Some(result) = with_ax_element_walk(child, depth + 1, next_id, element_id, f) {
                return Some(result);
            }
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
    use core_foundation::base::CFTypeRef;
    let attr = CFString::new(windows_attr);
    let mut value: CFTypeRef = std::ptr::null();
    let err =
        unsafe { ax::AXUIElementCopyAttributeValue(root, attr.as_concrete_TypeRef(), &mut value) };
    if err != ax::kAXErrorSuccess || value.is_null() {
        return;
    }
    let array: CFArray<*const c_void> = unsafe { CFArray::wrap_under_create_rule(value as _) };
    let minimized = CFString::new(ax::MINIMIZED);
    let false_value = CFBoolean::false_value();
    let raise = CFString::new(ax::RAISE_ACTION);
    for item in array.iter() {
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

fn left_click_at(pid: Option<i32>, point: CGPoint) -> ProviderResult<()> {
    mouse_click_at(
        pid,
        point,
        CGMouseButton::Left,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        1,
    )
}

fn secondary_click_at(pid: Option<i32>, point: CGPoint) -> ProviderResult<()> {
    mouse_click_at(
        pid,
        point,
        CGMouseButton::Right,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        1,
    )
}

fn mouse_click_at(
    pid: Option<i32>,
    point: CGPoint,
    button: CGMouseButton,
    down_type: CGEventType,
    up_type: CGEventType,
    click_state: i64,
) -> ProviderResult<()> {
    let source = event_source()?;
    let down = CGEvent::new_mouse_event(source.clone(), down_type, point, button)
        .map_err(|_| ProviderError::Failed("create mouse-down".into()))?;
    let up = CGEvent::new_mouse_event(source, up_type, point, button)
        .map_err(|_| ProviderError::Failed("create mouse-up".into()))?;
    down.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    up.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, click_state);
    post_event_sequence(pid, &[&down, &up]);
    Ok(())
}

fn double_click_at(pid: Option<i32>, point: CGPoint) -> ProviderResult<()> {
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
        post_event_sequence(pid, &[&down, &up]);
        thread::sleep(Duration::from_millis(40));
    }
    Ok(())
}

fn drag_between(pid: Option<i32>, from: CGPoint, to: CGPoint) -> ProviderResult<()> {
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
    post_event(pid, &down);
    thread::sleep(Duration::from_millis(20));
    post_event(pid, &drag);
    thread::sleep(Duration::from_millis(20));
    post_event(pid, &up);
    restore_frontmost_if_changed(original_frontmost);
    Ok(())
}

fn type_unicode(pid: Option<i32>, text: &str) -> ProviderResult<()> {
    let source = event_source()?;
    // A single keyboard event carrying the unicode string is the simplest
    // reliable path for arbitrary text (no per-char keycode mapping).
    let event = CGEvent::new_keyboard_event(source, 0, true)
        .map_err(|_| ProviderError::Failed("create keyboard event".into()))?;
    event.set_string(text);
    post_event_sequence(pid, &[&event]);
    Ok(())
}

fn press_keycode(pid: Option<i32>, keycode: u16, flags: CGEventFlags) -> ProviderResult<()> {
    let source = event_source()?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| ProviderError::Failed("create key-down".into()))?;
    let up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| ProviderError::Failed("create key-up".into()))?;
    down.set_flags(flags);
    up.set_flags(flags);
    post_event_sequence(pid, &[&down, &up]);
    Ok(())
}

fn scroll_wheel(pid: Option<i32>, direction: ScrollDirection, amount: i32) -> ProviderResult<()> {
    let source = event_source()?;
    let (dy, dx) = match direction {
        ScrollDirection::Up => (amount, 0),
        ScrollDirection::Down => (-amount, 0),
        ScrollDirection::Left => (0, amount),
        ScrollDirection::Right => (0, -amount),
    };
    let event = CGEvent::new_scroll_event(source, ScrollEventUnit::PIXEL, 2, dy, dx, 0)
        .map_err(|_| ProviderError::Failed("create scroll event".into()))?;
    post_event_sequence(pid, &[&event]);
    Ok(())
}

fn post_event_sequence(pid: Option<i32>, events: &[&CGEvent]) {
    let original_frontmost = frontmost_app();
    for event in events {
        post_event(pid, event);
    }
    restore_frontmost_if_changed(original_frontmost);
}

fn post_event(pid: Option<i32>, event: &CGEvent) {
    if let Some(pid) = pid {
        event.post_to_pid(pid);
    } else {
        event.post(CGEventTapLocation::HID);
    }
}

fn frontmost_app() -> Option<FrontmostApp> {
    use objc2_app_kit::NSWorkspace;
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    Some(FrontmostApp {
        pid: app.processIdentifier(),
        app,
    })
}

fn restore_frontmost_if_changed(original: Option<FrontmostApp>) {
    let Some(original) = original else {
        return;
    };
    thread::sleep(Duration::from_millis(10));
    if frontmost_app()
        .map(|current| current.pid == original.pid)
        .unwrap_or(false)
    {
        return;
    }
    let _ = original
        .app
        .activateWithOptions(objc2_app_kit::NSApplicationActivationOptions(0));
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
        // Containers / static text are not directly actionable.
        assert!(!is_actionable_role("AXGroup"));
        assert!(!is_actionable_role("AXStaticText"));
        assert!(!is_actionable_role(""));
    }

    #[test]
    fn provider_reports_control_supported() {
        assert!(MacosProvider::new().supports_control());
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
