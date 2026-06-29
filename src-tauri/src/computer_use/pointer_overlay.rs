//! Visual pointer overlay for computer-use actions.
//!
//! This layer is deliberately presentation-only: providers still perform the
//! real AX/UIA and input-injection work. The overlay only mirrors intent so the
//! user can see where an agent is about to act.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{
    utils::config::Color, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
    WindowEvent,
};

use super::provider::Point;

pub const POINTER_EVENT_NAME: &str = "computer_use_pointer_event";
pub const POINTER_OVERLAY_READY_EVENT: &str = "computer_use_pointer_overlay_ready";
const POINTER_OVERLAY_LABEL_PREFIX: &str = "computer-use-pointer-overlay";
const POINTER_OVERLAY_EVENT_TTL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
struct PendingPointerEvent {
    event: ComputerUsePointerEvent,
    queued_at: Instant,
}

#[derive(Default)]
struct PointerOverlayState {
    windows: HashMap<String, bool>,
    pending: HashMap<String, VecDeque<PendingPointerEvent>>,
}

pub type PointerEventSink = Arc<dyn Fn(ComputerUsePointerEvent) + Send + Sync>;

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

fn pointer_overlay_state() -> Arc<Mutex<PointerOverlayState>> {
    POINTER_OVERLAY_STATE
        .get_or_init(|| Arc::new(Mutex::new(PointerOverlayState::default())))
        .clone()
}

pub fn mark_pointer_overlay_ready(app: &AppHandle, label: &str) -> Result<(), String> {
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
