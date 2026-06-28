//! Visual pointer overlay for computer-use actions.
//!
//! This layer is deliberately presentation-only: providers still perform the
//! real AX/UIA and input-injection work. The overlay only mirrors intent so the
//! user can see where an agent is about to act.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use super::provider::Point;

pub const POINTER_EVENT_NAME: &str = "computer_use_pointer_event";
const POINTER_OVERLAY_LABEL_PREFIX: &str = "computer-use-pointer-overlay";

pub type PointerEventSink = Arc<dyn Fn(ComputerUsePointerEvent) + Send + Sync>;

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
    Arc::new(move |event| {
        if let Err(error) = show_and_emit_pointer_event(&app, event) {
            log::debug!("[computer-use:pointer-overlay] {error}");
        }
    })
}

fn show_and_emit_pointer_event(
    app: &AppHandle,
    event: ComputerUsePointerEvent,
) -> Result<(), String> {
    let labels = ensure_pointer_overlay_windows(app)?;
    for label in labels {
        app.emit_to(&label, POINTER_EVENT_NAME, &event)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn ensure_pointer_overlay_windows(app: &AppHandle) -> Result<Vec<String>, String> {
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

        let window = WebviewWindowBuilder::new(app, &label, url)
            .title("Computer Use Pointer")
            .decorations(false)
            .shadow(false)
            .resizable(false)
            .transparent(true)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .focused(false)
            .position(origin_x, origin_y)
            .inner_size(width, height)
            .build()
            .map_err(|error| error.to_string())?;

        let _ = window.set_ignore_cursor_events(true);
        let _ = window.set_visible_on_all_workspaces(true);
        window.on_window_event(|event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
            }
        });
    }

    Ok(labels)
}
