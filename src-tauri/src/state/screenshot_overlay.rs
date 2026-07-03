use std::collections::HashMap;
use std::sync::Mutex;

#[cfg(any(windows, target_os = "linux"))]
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct ScreenshotOverlayState {
    pub(crate) sources: Mutex<HashMap<String, ScreenshotOverlaySourceDto>>,
    pub(crate) reveal_main_on_finish: Mutex<HashMap<String, bool>>,
    #[cfg(any(windows, target_os = "linux"))]
    pub(crate) pending_results: Mutex<HashMap<String, ScreenshotOverlayResultSender>>,
    #[cfg(any(windows, target_os = "linux"))]
    pub(crate) completed_results: Mutex<HashMap<String, ScreenshotOverlayCompletion>>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotOverlayWindowDto {
    pub(crate) label: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotOverlayCancelledPayload {
    pub(crate) request_id: String,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComputerUsePointerOverlayReadyPayload {
    pub(crate) label: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotOverlaySourceDto {
    pub(crate) request_id: String,
    pub(crate) source_path: String,
    pub(crate) file_name: String,
    pub(crate) mode: String,
    pub(crate) windows: Vec<ScreenshotOverlayWindowCandidateDto>,
    pub(crate) initial_selection: Option<ScreenshotOverlayInitialSelectionDto>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotOverlayWindowCandidateDto {
    pub(crate) id: String,
    pub(crate) app_name: String,
    pub(crate) title: Option<String>,
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotOverlayInitialSelectionDto {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotOverlayCompleteRequest {
    pub(crate) request_id: String,
    pub(crate) path: Option<String>,
    pub(crate) cancelled: Option<bool>,
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) type ScreenshotOverlayResultSender =
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<crate::SavedPastedAttachment, String>>>>>;

#[cfg(any(windows, target_os = "linux"))]
#[derive(Clone)]
pub(crate) enum ScreenshotOverlayCompletion {
    Saved(String),
    Cancelled,
}
