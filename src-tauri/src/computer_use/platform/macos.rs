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

use std::ffi::c_void;
use std::path::PathBuf;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::CGDisplay;
use core_graphics::event::{
    CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::{CGPoint, CGRect};

use crate::computer_use::provider::{
    AppTarget, ComputerUseProvider, DisplayMetadata, ElementId, InstalledApp, ProviderError,
    ProviderResult, RawAppState, Rect, ScreenshotRef, ScrollDirection, UiElement,
};

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

    fn list_apps(&self) -> ProviderResult<Vec<InstalledApp>> {
        list_running_apps()
    }

    fn capture_app_state(&self, target: &AppTarget) -> ProviderResult<RawAppState> {
        let pid = resolve_pid(&target.app_id)?;
        let (window_id, bounds) = frontmost_window_for_pid(pid)?;
        let screenshot = capture_window(&self.capture_dir, window_id)?;
        let elements = ax_elements_for_pid(pid).unwrap_or_default();
        let display = display_metadata();
        Ok(RawAppState {
            target: target.clone(),
            display,
            screenshot,
            elements: elements_with_bounds(elements, bounds),
        })
    }

    fn click_element(&self, target: &AppTarget, element: &ElementId) -> ProviderResult<()> {
        let pid = resolve_pid(&target.app_id)?;
        let center = ax_element_center(pid, element)?
            .ok_or_else(|| ProviderError::ElementNotFound(element.clone()))?;
        click_at(center)
    }

    fn type_text(&self, _target: &AppTarget, text: &str) -> ProviderResult<()> {
        type_unicode(text)
    }

    fn press_key(&self, _target: &AppTarget, key: &str) -> ProviderResult<()> {
        let keycode =
            keycode_for(key).ok_or_else(|| ProviderError::Failed(format!("unknown key: {key}")))?;
        press_keycode(keycode)
    }

    fn scroll(
        &self,
        _target: &AppTarget,
        direction: ScrollDirection,
        amount: i32,
    ) -> ProviderResult<()> {
        scroll_wheel(direction, amount)
    }
}

// --- App enumeration -----------------------------------------------------

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
        });
    }
    Ok(out)
}

fn resolve_pid(app_id: &str) -> ProviderResult<i32> {
    list_running_apps()?
        .into_iter()
        .find(|app| app.id == app_id)
        .and_then(|app| app.pid)
        .ok_or_else(|| ProviderError::AppNotFound(app_id.to_string()))
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
    Err(ProviderError::Failed(
        "no visible window found for application".into(),
    ))
}

fn capture_window(dir: &PathBuf, window_id: u32) -> ProviderResult<ScreenshotRef> {
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
    Ok(ScreenshotRef {
        handle: path.to_string_lossy().to_string(),
        format: "png".into(),
        byte_len: meta.len(),
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

fn click_at(point: CGPoint) -> ProviderResult<()> {
    let source = event_source()?;
    let down = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::LeftMouseDown,
        point,
        CGMouseButton::Left,
    )
    .map_err(|_| ProviderError::Failed("create mouse-down".into()))?;
    let up = CGEvent::new_mouse_event(source, CGEventType::LeftMouseUp, point, CGMouseButton::Left)
        .map_err(|_| ProviderError::Failed("create mouse-up".into()))?;
    down.post(CGEventTapLocation::HID);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

fn type_unicode(text: &str) -> ProviderResult<()> {
    let source = event_source()?;
    // A single keyboard event carrying the unicode string is the simplest
    // reliable path for arbitrary text (no per-char keycode mapping).
    let event = CGEvent::new_keyboard_event(source, 0, true)
        .map_err(|_| ProviderError::Failed("create keyboard event".into()))?;
    event.set_string(text);
    event.post(CGEventTapLocation::HID);
    Ok(())
}

fn press_keycode(keycode: u16) -> ProviderResult<()> {
    let source = event_source()?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| ProviderError::Failed("create key-down".into()))?;
    let up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| ProviderError::Failed("create key-up".into()))?;
    down.post(CGEventTapLocation::HID);
    up.post(CGEventTapLocation::HID);
    Ok(())
}

fn scroll_wheel(direction: ScrollDirection, amount: i32) -> ProviderResult<()> {
    let source = event_source()?;
    let (dy, dx) = match direction {
        ScrollDirection::Up => (amount, 0),
        ScrollDirection::Down => (-amount, 0),
        ScrollDirection::Left => (0, amount),
        ScrollDirection::Right => (0, -amount),
    };
    let event = CGEvent::new_scroll_event(source, ScrollEventUnit::PIXEL, 2, dy, dx, 0)
        .map_err(|_| ProviderError::Failed("create scroll event".into()))?;
    event.post(CGEventTapLocation::HID);
    Ok(())
}

/// Minimal keycode map for common named keys (US layout virtual keycodes).
fn keycode_for(key: &str) -> Option<u16> {
    let k = key.to_ascii_lowercase();
    Some(match k.as_str() {
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
        assert_eq!(keycode_for("unknown-key"), None);
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
}
