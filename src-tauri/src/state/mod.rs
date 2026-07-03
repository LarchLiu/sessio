mod appshot;
mod screenshot_overlay;

pub(crate) use appshot::AppshotShortcutState;
pub(crate) use screenshot_overlay::{
    ComputerUsePointerOverlayReadyPayload, ScreenshotOverlayCancelledPayload,
    ScreenshotOverlayCompleteRequest, ScreenshotOverlaySourceDto, ScreenshotOverlayState,
    ScreenshotOverlayWindowCandidateDto, ScreenshotOverlayWindowDto,
};

#[cfg(any(windows, target_os = "linux"))]
pub(crate) use screenshot_overlay::{ScreenshotOverlayCompletion, ScreenshotOverlayResultSender};
