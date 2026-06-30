//! Visual pointer overlay for computer-use actions.
//!
//! This layer is deliberately presentation-only: providers still perform the
//! real AX/UIA and input-injection work. The overlay only mirrors intent so the
//! user can see where an agent is about to act.

#[cfg(not(target_os = "macos"))]
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
#[cfg(not(target_os = "macos"))]
use std::sync::{Mutex, OnceLock};
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[cfg(not(target_os = "macos"))]
use tauri::{
    utils::config::Color, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

use super::provider::Point;

pub const POINTER_EVENT_NAME: &str = "computer_use_pointer_event";
const POINTER_OVERLAY_LABEL_PREFIX: &str = "computer-use-pointer-overlay";
#[cfg(not(target_os = "macos"))]
const POINTER_OVERLAY_EVENT_TTL: Duration = Duration::from_secs(3);

#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone)]
struct PendingPointerEvent {
    event: ComputerUsePointerEvent,
    queued_at: Instant,
}

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
struct PointerOverlayState {
    windows: HashMap<String, bool>,
    pending: HashMap<String, VecDeque<PendingPointerEvent>>,
}

pub type PointerEventSink = Arc<dyn Fn(ComputerUsePointerEvent) + Send + Sync>;

#[cfg(not(target_os = "macos"))]
static POINTER_OVERLAY_STATE: OnceLock<Arc<Mutex<PointerOverlayState>>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerUsePointerAction {
    Click,
    SecondaryClick,
    DoubleClick,
    Drag,
    Semantic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUsePointerEvent {
    pub action: ComputerUsePointerAction,
    pub session_id: String,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub to_x: Option<f32>,
    pub to_y: Option<f32>,
    pub label: Option<String>,
}

impl ComputerUsePointerEvent {
    pub fn point(session_id: &str, action: ComputerUsePointerAction, point: Point) -> Self {
        Self {
            action,
            session_id: session_id.to_string(),
            x: Some(point.x),
            y: Some(point.y),
            to_x: None,
            to_y: None,
            label: None,
        }
    }

    pub fn point_with_label(
        session_id: &str,
        action: ComputerUsePointerAction,
        point: Point,
        label: impl Into<String>,
    ) -> Self {
        let mut event = Self::point(session_id, action, point);
        event.label = Some(label.into());
        event
    }

    pub fn drag(session_id: &str, from: Point, to: Point) -> Self {
        Self {
            action: ComputerUsePointerAction::Drag,
            session_id: session_id.to_string(),
            x: Some(from.x),
            y: Some(from.y),
            to_x: Some(to.x),
            to_y: Some(to.y),
            label: None,
        }
    }

    pub fn semantic(session_id: &str, label: impl Into<String>) -> Self {
        Self {
            action: ComputerUsePointerAction::Semantic,
            session_id: session_id.to_string(),
            x: None,
            y: None,
            to_x: None,
            to_y: None,
            label: Some(label.into()),
        }
    }
}

pub fn tauri_pointer_event_sink(app: AppHandle) -> PointerEventSink {
    #[cfg(target_os = "macos")]
    {
        native_macos::register_app_handle(app.clone());
        return Arc::new(move |event| {
            if let Err(error) = native_macos::show_pointer_event(&app, event) {
                super::diagnostics::write(
                    "pointer_overlay_emit_failed",
                    serde_json::json!({ "error": error }),
                );
            }
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let state = pointer_overlay_state();
        if let Err(error) = ensure_pointer_overlay_windows(&app, &state) {
            super::diagnostics::write(
                "pointer_overlay_ensure_failed",
                serde_json::json!({ "error": error }),
            );
        }
        Arc::new(move |event| {
            if let Err(error) = show_and_emit_pointer_event(&app, &state, event) {
                super::diagnostics::write(
                    "pointer_overlay_emit_failed",
                    serde_json::json!({ "error": error }),
                );
            }
        })
    }
}

pub fn release_session(session_id: &str) {
    #[cfg(target_os = "macos")]
    native_macos::release_session(session_id);

    #[cfg(not(target_os = "macos"))]
    let _ = session_id;
}

pub fn mark_pointer_overlay_ready(app: &AppHandle, label: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        let _ = label;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let state = POINTER_OVERLAY_STATE
            .get()
            .cloned()
            .ok_or_else(|| "pointer overlay state is not initialized".to_string())?;
        let overlay = app
            .get_webview_window(label)
            .ok_or_else(|| "Pointer overlay window is not available".to_string())?;
        let _ = overlay.show();
        let _ = overlay.set_ignore_cursor_events(true);
        flush_pointer_overlay_queue(app, &state, label)
    }
}

#[cfg(not(target_os = "macos"))]
fn pointer_overlay_state() -> Arc<Mutex<PointerOverlayState>> {
    POINTER_OVERLAY_STATE
        .get_or_init(|| Arc::new(Mutex::new(PointerOverlayState::default())))
        .clone()
}

#[cfg(target_os = "macos")]
mod native_macos {
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;
    use std::time::Duration;

    use objc2::{rc::Retained, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSBox, NSBoxType, NSColor, NSEvent, NSFont, NSPanel, NSScreen,
        NSScreenSaverWindowLevel, NSStatusWindowLevel, NSTextField, NSView,
        NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use tauri::AppHandle;

    use super::{ComputerUsePointerAction, ComputerUsePointerEvent, POINTER_OVERLAY_LABEL_PREFIX};

    const HIDE_DELAY_MS: u64 = 600_000;
    const MOTION_STEP_DELAY_MS: u64 = 14;
    const MOTION_MIN_STEPS: usize = 6;
    const MOTION_MAX_STEPS: usize = 18;
    const MOTION_DISTANCE_PER_STEP: f64 = 52.0;
    const MOTION_CURVE_MIN: f64 = 10.0;
    const MOTION_CURVE_MAX: f64 = 40.0;
    const POINTER_SIZE: f64 = 18.0;
    const DOT_SIZE: f64 = 6.0;
    const LABEL_WIDTH: f64 = 220.0;
    const LABEL_HEIGHT: f64 = 18.0;
    const LABEL_BOX_HEIGHT: f64 = 24.0;
    const LABEL_BOX_PADDING_X: f64 = 10.0;
    const LABEL_TEXT_INSET_X: f64 = 9.0;
    const LABEL_TEXT_INSET_Y: f64 = 4.0;
    const LABEL_OFFSET_X: f64 = 16.0;
    const LABEL_OFFSET_Y: f64 = 12.0;
    const PULSE_SIZE: f64 = 34.0;
    const OVERLAY_EDGE_PADDING: f64 = 8.0;

    #[derive(Debug, Clone, Copy)]
    struct ScreenPoint {
        x: f64,
        y: f64,
    }

    impl ScreenPoint {
        fn distance_to(self, other: ScreenPoint) -> f64 {
            let dx = other.x - self.x;
            let dy = other.y - self.y;
            (dx * dx + dy * dy).sqrt()
        }
    }

    #[derive(Debug, Default)]
    struct NativePointerCursorState {
        current_point: Option<ScreenPoint>,
    }

    #[derive(Debug)]
    struct PointerMotionSummary {
        start: ScreenPoint,
        end: ScreenPoint,
        seeded_from_real_mouse: bool,
        frame_count: usize,
    }

    #[derive(Debug, Clone)]
    struct NativeOverlayWindow {
        label: String,
        panel_ptr: usize,
        ring_ptr: usize,
        dot_ptr: usize,
        pulse_ptr: usize,
        label_box_ptr: usize,
        label_ptr: usize,
        origin_x: f64,
        origin_y: f64,
        width: f64,
        height: f64,
        hide_generation: u64,
    }

    static NATIVE_POINTER_WINDOWS: OnceLock<Arc<Mutex<HashMap<String, NativeOverlayWindow>>>> =
        OnceLock::new();
    static ACTIVE_POINTER_SESSIONS: OnceLock<Arc<Mutex<HashSet<String>>>> = OnceLock::new();
    static NATIVE_POINTER_CURSOR_STATE: OnceLock<Arc<Mutex<NativePointerCursorState>>> =
        OnceLock::new();
    static NATIVE_POINTER_ANIMATION_LOCK: OnceLock<Arc<Mutex<()>>> = OnceLock::new();
    static POINTER_OVERLAY_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

    fn native_pointer_windows() -> Arc<Mutex<HashMap<String, NativeOverlayWindow>>> {
        NATIVE_POINTER_WINDOWS
            .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
            .clone()
    }

    fn active_pointer_sessions() -> Arc<Mutex<HashSet<String>>> {
        ACTIVE_POINTER_SESSIONS
            .get_or_init(|| Arc::new(Mutex::new(HashSet::new())))
            .clone()
    }

    fn native_pointer_cursor_state() -> Arc<Mutex<NativePointerCursorState>> {
        NATIVE_POINTER_CURSOR_STATE
            .get_or_init(|| Arc::new(Mutex::new(NativePointerCursorState::default())))
            .clone()
    }

    fn native_pointer_animation_lock() -> Arc<Mutex<()>> {
        NATIVE_POINTER_ANIMATION_LOCK
            .get_or_init(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub(super) fn register_app_handle(app: AppHandle) {
        let _ = POINTER_OVERLAY_APP_HANDLE.set(app);
    }

    pub(super) fn release_session(session_id: &str) {
        let sessions = active_pointer_sessions();
        let should_destroy = {
            let Ok(mut sessions) = sessions.lock() else {
                return;
            };
            sessions.remove(session_id);
            sessions.is_empty()
        };
        if !should_destroy {
            return;
        }
        let Some(app) = POINTER_OVERLAY_APP_HANDLE.get().cloned() else {
            return;
        };
        let _ = app.run_on_main_thread(move || {
            destroy_all_pointer_overlay_windows_on_main();
        });
    }

    pub(super) fn ensure_pointer_overlay_windows(app: &AppHandle) -> Result<(), String> {
        let windows = native_pointer_windows();
        let (tx, rx) = std::sync::mpsc::channel();
        let app = app.clone();
        app.run_on_main_thread(move || {
            let result = ensure_pointer_overlay_windows_on_main(&windows);
            let _ = tx.send(result);
        })
        .map_err(|error| error.to_string())?;
        rx.recv()
            .map_err(|_| "pointer overlay creation was cancelled".to_string())?
    }

    fn ensure_pointer_overlay_windows_on_main(
        windows: &Arc<Mutex<HashMap<String, NativeOverlayWindow>>>,
    ) -> Result<(), String> {
        let mtm =
            MainThreadMarker::new().ok_or_else(|| "Must run pointer overlay on main thread".to_string())?;
        let screens = NSScreen::screens(mtm);
        let mut overlays = windows.lock().map_err(|error| error.to_string())?;

        for (index, screen) in screens.iter().enumerate() {
            let label = format!("{POINTER_OVERLAY_LABEL_PREFIX}-{index}");
            let frame = screen.frame();
            let scale = screen.backingScaleFactor().max(1.0);
            let origin_x = frame.origin.x;
            let origin_y = frame.origin.y;
            let width = frame.size.width;
            let height = frame.size.height;

            if let Some(existing) = overlays.get_mut(&label) {
                existing.origin_x = origin_x;
                existing.origin_y = origin_y;
                existing.width = width;
                existing.height = height;
                let panel = unsafe { &*(existing.panel_ptr as *mut NSPanel) };
                panel.setFrame_display(
                    NSRect::new(NSPoint::new(origin_x, origin_y), NSSize::new(width, height)),
                    true,
                );
                continue;
            }

            let panel = build_overlay_panel(mtm, frame);
            let content = panel
                .contentView()
                .ok_or_else(|| "Pointer overlay content view is unavailable".to_string())?;

            let ring = build_ring_view(mtm);
            let dot = build_dot_view(mtm);
            let pulse = build_pulse_view(mtm);
            let label_box = build_label_box_view(mtm);
            let label_view = build_label_view(mtm);

            content.addSubview(&pulse);
            content.addSubview(&label_box);
            content.addSubview(&ring);
            content.addSubview(&dot);
            content.addSubview(&label_view);

            pulse.setHidden(true);
            label_box.setHidden(true);
            ring.setHidden(true);
            dot.setHidden(true);
            label_view.setHidden(true);
            panel.orderOut(None);

            let panel_ptr = Retained::into_raw(panel) as usize;
            let ring_ptr = (&*ring as *const NSBox) as usize;
            let dot_ptr = (&*dot as *const NSBox) as usize;
            let pulse_ptr = (&*pulse as *const NSBox) as usize;
            let label_box_ptr = (&*label_box as *const NSBox) as usize;
            let label_ptr = (&*label_view as *const NSTextField) as usize;

            overlays.insert(
                label.clone(),
                NativeOverlayWindow {
                    label: label.clone(),
                    panel_ptr,
                    ring_ptr,
                    dot_ptr,
                    pulse_ptr,
                    label_box_ptr,
                    label_ptr,
                    origin_x,
                    origin_y,
                    width,
                    height,
                    hide_generation: 0,
                },
            );

            super::super::diagnostics::write(
                "pointer_overlay_window_created",
                serde_json::json!({
                    "label": label,
                    "originX": origin_x,
                    "originY": origin_y,
                    "width": width,
                    "height": height,
                    "monitorScale": scale,
                    "native": true,
                }),
            );
        }

        Ok(())
    }

    pub(super) fn show_pointer_event(
        app: &AppHandle,
        event: ComputerUsePointerEvent,
    ) -> Result<(), String> {
        ensure_pointer_overlay_windows(app)?;
        if let Ok(mut sessions) = active_pointer_sessions().lock() {
            sessions.insert(event.session_id.clone());
        }
        let windows = native_pointer_windows();
        let animation_lock = native_pointer_animation_lock();
        let _animation_guard = animation_lock
            .lock()
            .map_err(|error| error.to_string())?;
        let overlays = {
            let mut overlays = windows.lock().map_err(|error| error.to_string())?;
            let label = overlay_label_for_event(&overlays, &event)
                .ok_or_else(|| "No native pointer overlay matches the event".to_string())?;
            let target_generation = overlays
                .get_mut(&label)
                .ok_or_else(|| "Native pointer overlay is no longer available".to_string())?;
            target_generation.hide_generation = target_generation.hide_generation.saturating_add(1);
            let generation = target_generation.hide_generation;
            for overlay in overlays.values_mut() {
                if overlay.label != label {
                    overlay.hide_generation = overlay.hide_generation.saturating_add(1);
                }
            }
            (
                overlays.values().cloned().collect::<Vec<_>>(),
                label,
                generation,
            )
        };
        let (overlays, label, generation) = overlays;

        let event_for_show = event.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let app_for_show = app.clone();
        let prior_point = native_pointer_cursor_state()
            .lock()
            .map_err(|error| error.to_string())?
            .current_point;
        app_for_show
            .run_on_main_thread(move || {
                let result = show_pointer_event_on_main(&overlays, &event_for_show, prior_point);
                let _ = tx.send(result);
            })
            .map_err(|error| error.to_string())?;
        let motion = rx
            .recv()
            .map_err(|_| "pointer overlay display was cancelled".to_string())??;
        if let Ok(mut state) = native_pointer_cursor_state().lock() {
            state.current_point = Some(motion.end);
        }

        super::super::diagnostics::write(
            "pointer_overlay_emit",
            serde_json::json!({
                "label": label,
                "ready": true,
                "native": true,
                "motion": {
                    "fromX": motion.start.x,
                    "fromY": motion.start.y,
                    "toX": motion.end.x,
                    "toY": motion.end.y,
                    "seededFromRealMouse": motion.seeded_from_real_mouse,
                    "frameCount": motion.frame_count,
                },
                "event": event,
            }),
        );

        let app_for_hide = app.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(HIDE_DELAY_MS));
            let windows = native_pointer_windows();
            let should_hide = windows
                .lock()
                .ok()
                .and_then(|overlays| overlays.get(&label).cloned())
                .map(|overlay| overlay.hide_generation == generation)
                .unwrap_or(false);
            if !should_hide {
                return;
            }
            let label_for_hide = label.clone();
            let _ = app_for_hide.run_on_main_thread(move || {
                let windows = native_pointer_windows();
                if let Ok(overlays) = windows.lock() {
                    if let Some(overlay) = overlays.get(&label_for_hide) {
                        hide_pointer_overlay_on_main(overlay);
                    }
                };
            });
        });

        Ok(())
    }

    fn overlay_label_for_event(
        overlays: &HashMap<String, NativeOverlayWindow>,
        event: &ComputerUsePointerEvent,
    ) -> Option<String> {
        let point = event
            .to_x
            .zip(event.to_y)
            .or_else(|| event.x.zip(event.y))
            .map(|(x, y)| (f64::from(x), f64::from(y)));
        if let Some((x, y)) = point {
            for overlay in overlays.values() {
                if x >= overlay.origin_x
                    && x <= overlay.origin_x + overlay.width
                    && y >= overlay.origin_y
                    && y <= overlay.origin_y + overlay.height
                {
                    return Some(overlay.label.clone());
                }
            }
        }
        overlays.values().next().map(|overlay| overlay.label.clone())
    }

    fn show_pointer_event_on_main(
        overlays: &[NativeOverlayWindow],
        event: &ComputerUsePointerEvent,
        prior_point: Option<ScreenPoint>,
    ) -> Result<PointerMotionSummary, String> {
        let end = event_terminal_screen_point(overlays, event);
        let (start, seeded_from_real_mouse) = match prior_point {
            Some(point) => (point, false),
            None => (current_system_mouse_location().unwrap_or(end), true),
        };
        let frames = motion_frames_for_event(start, end, event);
        let label_text = action_label(event.action.clone(), event.label.as_deref());
        for (index, frame) in frames.iter().enumerate() {
            render_pointer_frame_on_main(
                overlays,
                *frame,
                event.action.clone(),
                if index + 1 == frames.len() {
                    Some(label_text.as_str())
                } else {
                    None
                },
                index + 1 == frames.len(),
            )?;
            if index + 1 != frames.len() {
                thread::sleep(Duration::from_millis(MOTION_STEP_DELAY_MS));
            }
        }
        Ok(PointerMotionSummary {
            start,
            end,
            seeded_from_real_mouse,
            frame_count: frames.len(),
        })
    }

    fn hide_pointer_overlay_on_main(overlay: &NativeOverlayWindow) {
        conceal_pointer_overlay_on_main(overlay);
        super::super::diagnostics::write(
            "pointer_overlay_hide",
            serde_json::json!({
                "label": overlay.label,
                "native": true,
            }),
        );
    }

    fn conceal_pointer_overlay_on_main(overlay: &NativeOverlayWindow) {
        let panel = unsafe { &*(overlay.panel_ptr as *mut NSPanel) };
        let ring = unsafe { &*(overlay.ring_ptr as *mut NSBox) };
        let dot = unsafe { &*(overlay.dot_ptr as *mut NSBox) };
        let pulse = unsafe { &*(overlay.pulse_ptr as *mut NSBox) };
        let label_box = unsafe { &*(overlay.label_box_ptr as *mut NSBox) };
        let label = unsafe { &*(overlay.label_ptr as *mut NSTextField) };
        pulse.setHidden(true);
        ring.setHidden(true);
        dot.setHidden(true);
        label_box.setHidden(true);
        label.setHidden(true);
        panel.orderOut(None);
    }

    fn render_pointer_frame_on_main(
        overlays: &[NativeOverlayWindow],
        screen_point: ScreenPoint,
        action: ComputerUsePointerAction,
        label_text: Option<&str>,
        show_pulse: bool,
    ) -> Result<(), String> {
        let active_label = overlay_label_for_screen_point(overlays, screen_point)
            .or_else(|| overlays.first().map(|overlay| overlay.label.clone()))
            .ok_or_else(|| "No native pointer overlays are available".to_string())?;
        let color = action_color(action);

        for overlay in overlays {
            if overlay.label != active_label {
                conceal_pointer_overlay_on_main(overlay);
                continue;
            }

            let panel = unsafe { &*(overlay.panel_ptr as *mut NSPanel) };
            let ring = unsafe { &*(overlay.ring_ptr as *mut NSBox) };
            let dot = unsafe { &*(overlay.dot_ptr as *mut NSBox) };
            let pulse = unsafe { &*(overlay.pulse_ptr as *mut NSBox) };
            let label_box = unsafe { &*(overlay.label_box_ptr as *mut NSBox) };
            let label = unsafe { &*(overlay.label_ptr as *mut NSTextField) };
            let point = overlay_local_point(panel, overlay, screen_point);
            let ring_origin =
                NSPoint::new(point.x - (POINTER_SIZE / 2.0), point.y - (POINTER_SIZE / 2.0));
            let dot_origin = NSPoint::new(point.x - (DOT_SIZE / 2.0), point.y - (DOT_SIZE / 2.0));
            let pulse_origin =
                NSPoint::new(point.x - (PULSE_SIZE / 2.0), point.y - (PULSE_SIZE / 2.0));

            pulse.setFrame(NSRect::new(
                pulse_origin,
                NSSize::new(PULSE_SIZE, PULSE_SIZE),
            ));
            ring.setFrame(NSRect::new(
                ring_origin,
                NSSize::new(POINTER_SIZE, POINTER_SIZE),
            ));
            dot.setFrame(NSRect::new(dot_origin, NSSize::new(DOT_SIZE, DOT_SIZE)));

            pulse.setBorderColor(&color.colorWithAlphaComponent(0.52));
            ring.setBorderColor(&color);
            dot.setFillColor(&color);
            pulse.setHidden(!show_pulse);
            ring.setHidden(false);
            dot.setHidden(false);

            if let Some(text) = label_text.filter(|text| !text.is_empty()) {
                let label_box_width = label_width_for_text(text);
                let label_origin = clamped_label_origin(overlay, point, label_box_width);
                label_box.setFrame(NSRect::new(
                    label_origin,
                    NSSize::new(label_box_width, LABEL_BOX_HEIGHT),
                ));
                label.setStringValue(&NSString::from_str(text));
                label.setFrame(NSRect::new(
                    NSPoint::new(
                        label_origin.x + LABEL_TEXT_INSET_X,
                        label_origin.y + LABEL_TEXT_INSET_Y,
                    ),
                    NSSize::new(label_box_width - (LABEL_TEXT_INSET_X * 2.0), LABEL_HEIGHT),
                ));
                label_box.setHidden(false);
                label.setHidden(false);
            } else {
                label_box.setHidden(true);
                label.setHidden(true);
            }

            panel.orderFrontRegardless();
            panel.displayIfNeeded();
        }
        Ok(())
    }

    fn overlay_local_point(
        panel: &NSPanel,
        overlay: &NativeOverlayWindow,
        point: ScreenPoint,
    ) -> NSPoint {
        let converted = panel.convertPointFromScreen(NSPoint::new(point.x, point.y));
        NSPoint::new(
            converted.x.clamp(0.0, overlay.width),
            converted.y.clamp(0.0, overlay.height),
        )
    }

    fn clamped_label_origin(
        overlay: &NativeOverlayWindow,
        point: NSPoint,
        label_box_width: f64,
    ) -> NSPoint {
        let max_x = (overlay.width - label_box_width - OVERLAY_EDGE_PADDING).max(OVERLAY_EDGE_PADDING);
        let max_y =
            (overlay.height - LABEL_BOX_HEIGHT - OVERLAY_EDGE_PADDING).max(OVERLAY_EDGE_PADDING);
        NSPoint::new(
            (point.x + LABEL_OFFSET_X).clamp(OVERLAY_EDGE_PADDING, max_x),
            (point.y + LABEL_OFFSET_Y).clamp(OVERLAY_EDGE_PADDING, max_y),
        )
    }

    fn overlay_label_for_screen_point(
        overlays: &[NativeOverlayWindow],
        point: ScreenPoint,
    ) -> Option<String> {
        overlays
            .iter()
            .find(|overlay| {
                point.x >= overlay.origin_x
                    && point.x <= overlay.origin_x + overlay.width
                    && point.y >= overlay.origin_y
                    && point.y <= overlay.origin_y + overlay.height
            })
            .map(|overlay| overlay.label.clone())
    }

    fn event_terminal_screen_point(
        overlays: &[NativeOverlayWindow],
        event: &ComputerUsePointerEvent,
    ) -> ScreenPoint {
        if let Some((x, y)) = event.to_x.zip(event.to_y) {
            return ScreenPoint {
                x: f64::from(x),
                y: f64::from(y),
            };
        }
        if let Some((x, y)) = event.x.zip(event.y) {
            return ScreenPoint {
                x: f64::from(x),
                y: f64::from(y),
            };
        }
        overlays
            .first()
            .map(|overlay| ScreenPoint {
                x: overlay.origin_x + (overlay.width / 2.0),
                y: overlay.origin_y + (overlay.height / 2.0),
            })
            .unwrap_or(ScreenPoint { x: 0.0, y: 0.0 })
    }

    fn motion_frames_for_event(
        start: ScreenPoint,
        end: ScreenPoint,
        event: &ComputerUsePointerEvent,
    ) -> Vec<ScreenPoint> {
        match event.to_x.zip(event.to_y).zip(event.x.zip(event.y)) {
            Some(((to_x, to_y), (from_x, from_y))) => {
                let drag_start = ScreenPoint {
                    x: f64::from(from_x),
                    y: f64::from(from_y),
                };
                let drag_end = ScreenPoint {
                    x: f64::from(to_x),
                    y: f64::from(to_y),
                };
                let mut frames = motion_frames_between(start, drag_start);
                let mut drag_frames = motion_frames_between(drag_start, drag_end);
                if !frames.is_empty() {
                    drag_frames.drain(..drag_frames.len().min(1));
                }
                frames.extend(drag_frames);
                if frames.is_empty() {
                    frames.push(end);
                }
                frames
            }
            None => motion_frames_between(start, end),
        }
    }

    fn motion_frames_between(start: ScreenPoint, end: ScreenPoint) -> Vec<ScreenPoint> {
        let distance = start.distance_to(end);
        if distance <= 1.0 {
            return vec![end];
        }

        let steps = ((distance / MOTION_DISTANCE_PER_STEP).ceil() as usize)
            .clamp(MOTION_MIN_STEPS, MOTION_MAX_STEPS);
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        let curve_sign = if ((start.x.round() as i64) ^ (end.y.round() as i64) ^ (dx.round() as i64))
            & 1
            == 0
        {
            1.0
        } else {
            -1.0
        };
        let curve = (distance * 0.14).clamp(MOTION_CURVE_MIN, MOTION_CURVE_MAX);
        let (forward_x, forward_y, perp_x, perp_y) = if distance > 0.0 {
            let inv = 1.0 / distance;
            let fx = dx * inv;
            let fy = dy * inv;
            (fx, fy, -fy, fx)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let control = ScreenPoint {
            x: (start.x + end.x) / 2.0 + (perp_x * curve * curve_sign) + (forward_x * curve * 0.18),
            y: (start.y + end.y) / 2.0 + (perp_y * curve * curve_sign) + (forward_y * curve * 0.18),
        };

        (1..=steps)
            .map(|step| {
                let linear_t = step as f64 / steps as f64;
                let t = ease_in_out_cubic(linear_t);
                quadratic_bezier(start, control, end, t)
            })
            .collect()
    }

    fn quadratic_bezier(start: ScreenPoint, control: ScreenPoint, end: ScreenPoint, t: f64) -> ScreenPoint {
        let one_minus_t = 1.0 - t;
        ScreenPoint {
            x: (one_minus_t * one_minus_t * start.x)
                + (2.0 * one_minus_t * t * control.x)
                + (t * t * end.x),
            y: (one_minus_t * one_minus_t * start.y)
                + (2.0 * one_minus_t * t * control.y)
                + (t * t * end.y),
        }
    }

    fn ease_in_out_cubic(t: f64) -> f64 {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            1.0 - ((-2.0 * t + 2.0).powi(3) / 2.0)
        }
    }

    fn current_system_mouse_location() -> Option<ScreenPoint> {
        let point = NSEvent::mouseLocation();
        if point.x.is_finite() && point.y.is_finite() {
            Some(ScreenPoint {
                x: point.x,
                y: point.y,
            })
        } else {
            None
        }
    }

    fn action_label(action: ComputerUsePointerAction, label: Option<&str>) -> String {
        if let Some(label) = label {
            return label.to_string();
        }
        match action {
            ComputerUsePointerAction::Click => "click".to_string(),
            ComputerUsePointerAction::SecondaryClick => "right click".to_string(),
            ComputerUsePointerAction::DoubleClick => "double click".to_string(),
            ComputerUsePointerAction::Drag => "drag".to_string(),
            ComputerUsePointerAction::Semantic => "computer use".to_string(),
        }
    }

    fn action_color(action: ComputerUsePointerAction) -> Retained<NSColor> {
        match action {
            ComputerUsePointerAction::SecondaryClick => NSColor::systemOrangeColor(),
            ComputerUsePointerAction::DoubleClick => NSColor::systemPinkColor(),
            ComputerUsePointerAction::Drag => NSColor::systemBlueColor(),
            ComputerUsePointerAction::Semantic => NSColor::systemPurpleColor(),
            ComputerUsePointerAction::Click => NSColor::controlAccentColor(),
        }
    }

    fn label_width_for_text(text: &str) -> f64 {
        let approx = (text.chars().count() as f64 * 7.2) + (LABEL_BOX_PADDING_X * 2.0);
        approx.clamp(72.0, LABEL_WIDTH)
    }

    fn destroy_all_pointer_overlay_windows_on_main() {
        let windows = native_pointer_windows();
        let overlays = {
            let Ok(mut overlays) = windows.lock() else {
                return;
            };
            overlays.drain().map(|(_, overlay)| overlay).collect::<Vec<_>>()
        };

        for overlay in overlays {
            let panel_ptr = overlay.panel_ptr;
            if panel_ptr == 0 {
                continue;
            }
            if let Some(panel) = unsafe { Retained::from_raw(panel_ptr as *mut NSPanel) } {
                panel.close();
            }
        }
        if let Ok(mut state) = native_pointer_cursor_state().lock() {
            state.current_point = None;
        }

        super::super::diagnostics::write(
            "pointer_overlay_destroy_all",
            serde_json::json!({
                "native": true,
            }),
        );
    }

    fn build_overlay_panel(mtm: MainThreadMarker, frame: NSRect) -> Retained<NSPanel> {
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        );
        let content = NSView::new(mtm);
        content.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(frame.size.width, frame.size.height),
        ));

        panel.setContentView(Some(&content));
        panel.setLevel(NSStatusWindowLevel.max(NSScreenSaverWindowLevel - 1));
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::Transient,
        );
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setWorksWhenModal(true);
        panel.setHasShadow(false);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(false);
        panel.setAlphaValue(1.0);
        unsafe {
            panel.setReleasedWhenClosed(false);
        }
        panel
    }

    fn build_ring_view(mtm: MainThreadMarker) -> Retained<NSBox> {
        let ring = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(POINTER_SIZE, POINTER_SIZE)),
        );
        ring.setBoxType(NSBoxType::Custom);
        ring.setBorderWidth(2.0);
        ring.setCornerRadius(POINTER_SIZE / 2.0);
        ring.setBorderColor(&NSColor::controlAccentColor());
        ring.setFillColor(&NSColor::clearColor());
        ring.setTransparent(false);
        ring.setContentViewMargins(NSSize::new(0.0, 0.0));
        ring
    }

    fn build_pulse_view(mtm: MainThreadMarker) -> Retained<NSBox> {
        let pulse = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PULSE_SIZE, PULSE_SIZE)),
        );
        pulse.setBoxType(NSBoxType::Custom);
        pulse.setBorderWidth(1.6);
        pulse.setCornerRadius(PULSE_SIZE / 2.0);
        pulse.setBorderColor(&NSColor::controlAccentColor().colorWithAlphaComponent(0.52));
        pulse.setFillColor(&NSColor::clearColor());
        pulse.setTransparent(false);
        pulse.setContentViewMargins(NSSize::new(0.0, 0.0));
        pulse
    }

    fn build_dot_view(mtm: MainThreadMarker) -> Retained<NSBox> {
        let dot = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(DOT_SIZE, DOT_SIZE)),
        );
        dot.setBoxType(NSBoxType::Custom);
        dot.setBorderWidth(0.0);
        dot.setCornerRadius(DOT_SIZE / 2.0);
        dot.setFillColor(&NSColor::controlAccentColor());
        dot.setTransparent(false);
        dot.setContentViewMargins(NSSize::new(0.0, 0.0));
        dot
    }

    fn build_label_box_view(mtm: MainThreadMarker) -> Retained<NSBox> {
        let box_view = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(96.0, LABEL_BOX_HEIGHT)),
        );
        box_view.setBoxType(NSBoxType::Custom);
        box_view.setBorderWidth(1.0);
        box_view.setCornerRadius(LABEL_BOX_HEIGHT / 2.0);
        box_view.setBorderColor(&NSColor::controlAccentColor().colorWithAlphaComponent(0.34));
        box_view.setFillColor(&NSColor::blackColor().colorWithAlphaComponent(0.78));
        box_view.setTransparent(false);
        box_view.setContentViewMargins(NSSize::new(0.0, 0.0));
        box_view
    }

    fn build_label_view(mtm: MainThreadMarker) -> Retained<NSTextField> {
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setTextColor(Some(&NSColor::whiteColor()));
        label.setFont(Some(&NSFont::systemFontOfSize_weight(12.0, 0.45)));
        label.setUsesSingleLineMode(true);
        label.setBezeled(false);
        label.setBordered(false);
        label.setDrawsBackground(false);
        label
    }
}

#[cfg(not(target_os = "macos"))]
fn show_and_emit_pointer_event(
    app: &AppHandle,
    state: &Arc<Mutex<PointerOverlayState>>,
    event: ComputerUsePointerEvent,
) -> Result<(), String> {
    let labels = ensure_pointer_overlay_windows(app, state)?;
    for label in labels {
        let ready = {
            let mut state = state.lock().map_err(|error| error.to_string())?;
            prune_stale_pending(&mut state);
            *state.windows.entry(label.clone()).or_insert(false)
        };
        if ready {
            app.emit_to(&label, POINTER_EVENT_NAME, &event)
                .map_err(|error| error.to_string())?;
            super::diagnostics::write(
                "pointer_overlay_emit",
                serde_json::json!({
                    "label": label,
                    "ready": true,
                    "event": event,
                }),
            );
        } else {
            let mut state = state.lock().map_err(|error| error.to_string())?;
            state
                .pending
                .entry(label.clone())
                .or_default()
                .push_back(PendingPointerEvent {
                    event: event.clone(),
                    queued_at: Instant::now(),
                });
            super::diagnostics::write(
                "pointer_overlay_queue",
                serde_json::json!({
                    "label": label,
                    "ready": false,
                    "event": event,
                }),
            );
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn ensure_pointer_overlay_windows(
    app: &AppHandle,
    state: &Arc<Mutex<PointerOverlayState>>,
) -> Result<Vec<String>, String> {
    let monitors = app
        .available_monitors()
        .map_err(|error| error.to_string())?;
    let mut labels = Vec::new();

    for (index, monitor) in monitors.iter().enumerate() {
        let label = format!("{POINTER_OVERLAY_LABEL_PREFIX}-{index}");
        labels.push(label.clone());
        if app.get_webview_window(&label).is_some() {
            continue;
        }
        {
            let mut overlay_state = state.lock().map_err(|error| error.to_string())?;
            overlay_state.windows.insert(label.clone(), false);
        }

        let scale = monitor.scale_factor().max(1.0);
        let pos = monitor.position();
        let size = monitor.size();
        let origin_x = pos.x as f64 / scale;
        let origin_y = pos.y as f64 / scale;
        let width = size.width as f64 / scale;
        let height = size.height as f64 / scale;
        let url = WebviewUrl::App(
            format!(
                "index.html?computerUsePointerOverlay=1&originX={origin_x}&originY={origin_y}&width={width}&height={height}"
            )
            .into(),
        );
        let init_script = r#"
            document.documentElement.style.background = 'transparent';
            document.documentElement.style.backgroundColor = 'transparent';
            if (document.body) {
              document.body.style.background = 'transparent';
              document.body.style.backgroundColor = 'transparent';
            } else {
              window.addEventListener('DOMContentLoaded', () => {
                document.body.style.background = 'transparent';
                document.body.style.backgroundColor = 'transparent';
              }, { once: true });
            }
        "#;

        let window = WebviewWindowBuilder::new(app, &label, url)
            .title("Computer Use Pointer")
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .transparent(true)
            .background_color(Color(0, 0, 0, 0))
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .focused(false)
            .visible(false)
            .initialization_script(init_script)
            .position(origin_x, origin_y)
            .inner_size(width, height)
            .build()
            .map_err(|error| error.to_string())?;

        let _ = window.set_ignore_cursor_events(true);
        let _ = window.set_visible_on_all_workspaces(true);
        let _ = window.show();
        super::diagnostics::write(
            "pointer_overlay_window_created",
            serde_json::json!({
                "label": label,
                "originX": origin_x,
                "originY": origin_y,
                "width": width,
                "height": height,
                "monitorScale": scale,
            }),
        );
        window.on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
            }
        });
    }

    Ok(labels)
}

#[cfg(not(target_os = "macos"))]
fn flush_pointer_overlay_queue(
    app: &AppHandle,
    state: &Arc<Mutex<PointerOverlayState>>,
    label: &str,
) -> Result<(), String> {
    let pending = {
        let mut state = state.lock().map_err(|error| error.to_string())?;
        state.windows.insert(label.to_string(), true);
        prune_stale_pending(&mut state);
        state.pending.remove(label).unwrap_or_default()
    };

    super::diagnostics::write(
        "pointer_overlay_ready",
        serde_json::json!({
            "label": label,
            "queuedCount": pending.len(),
        }),
    );

    for queued in pending {
        if let Err(error) = app.emit_to(label, POINTER_EVENT_NAME, &queued.event) {
            super::diagnostics::write(
                "pointer_overlay_flush_failed",
                serde_json::json!({
                    "label": label,
                    "event": queued.event,
                    "error": error.to_string(),
                }),
            );
        } else {
            super::diagnostics::write(
                "pointer_overlay_flush",
                serde_json::json!({
                    "label": label,
                    "event": queued.event,
                }),
            );
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn prune_stale_pending(state: &mut PointerOverlayState) {
    let now = Instant::now();
    for pending in state.pending.values_mut() {
        while let Some(front) = pending.front() {
            if now.duration_since(front.queued_at) <= POINTER_OVERLAY_EVENT_TTL {
                break;
            }
            pending.pop_front();
        }
    }
    state.pending.retain(|_, pending| !pending.is_empty());
}
