# Linux screenshot implementation plan

## Summary

Implement Linux screenshot support by mirroring the current Windows architecture while preserving the existing frontend flow and Tauri command names. The target behavior is:

- Appshot/global shortcut captures an image and inserts it into the active composer.
- The composer screenshot button opens the existing overlay editor.
- Users can select an area, select a window where the platform allows it, draw boxes or lines, apply mosaic, undo, cancel, and save as a PNG attachment.
- The "hide Sessio while capturing" option works on Linux too.

Use the same capture shape as Windows:

- Native code captures a background image first.
- The existing frontend overlay edits that captured source image.
- Rust owns the overlay lifetime, pending/completed result coordination, cancellation, and cleanup.

Use a dual Linux backend strategy:

- X11 provides the closest Windows-style parity, including active-window capture, monitor background capture, and window candidates for click-to-select.
- Wayland uses `xdg-desktop-portal` for secure user-authorized screenshots, and gracefully degrades where Wayland intentionally blocks silent window enumeration or frontmost-window access.

This follows the native desktop principle of adopting platform capabilities instead of competing with them.

## Key changes

- Add a Linux screenshot backend module at `src-tauri/src/screenshot/linux.rs`, and keep the existing command surface in `src-tauri/src/lib.rs`.
- Add Linux-only dependencies in `src-tauri/Cargo.toml`, using versions already present in `Cargo.lock` where possible:
  - `zbus = "5.15"` for `xdg-desktop-portal`.
  - `x11rb = "0.13"` for X11 window enumeration and root-window pixel capture.
  - `url = "2.5"` for portal `file://` URI handling.
- Keep the existing Linux `capture_window_area_png` WebKitGTK snapshot implementation for WebView-local captures.
- Replace the current non-macOS/non-Windows screenshot stubs with Linux implementations for:
  - `capture_frontmost_app_window_png`
  - `capture_selected_screen_area_png`
  - `capture_interactive_screen_png`
  - `open_screenshot_overlay_capture`
  - `get_screenshot_overlay_source`
  - `finish_screenshot_overlay`
  - `complete_screenshot_overlay_capture`
- Share the Windows overlay completion machinery with Linux:
  - 300 second timeout for command-driven selections.
  - pending/completed result maps in `ScreenshotOverlayState`.
  - completion delivery when the overlay closes after save or cancel.
- Keep resource ownership deterministic, following the Windows RAII model:
  - X11 connection and captured pixel buffers are owned by short-lived Rust values.
  - Overlay source entries, reveal-main flags, pending senders, and completed results are removed on finish, close, timeout, or failure.
- Fix Linux compilation paths around Appshot:
  - Include Linux in the `std::thread` cfg import.
  - Provide a Linux `capture_frontmost_window_png` used by the global shortcut path.

## Linux backend behavior

### X11

- Detect X11 through `XDG_SESSION_TYPE=x11` or a usable `DISPLAY`.
- Capture the frontmost window by reading `_NET_ACTIVE_WINDOW`, obtaining its geometry, and capturing pixels from the root window.
- Build overlay window candidates from `_NET_CLIENT_LIST_STACKING`, filtering out hidden, minimized, tiny, or non-viewable windows.
- Return candidates in topmost-first order so the existing frontend hit test selects the visible window first.
- Capture monitor backgrounds from the root window for the monitor containing the main Sessio window.
- Convert X11 pixel data to PNG and save through the existing paste cache path and safe filename helper.

### Wayland

- Detect Wayland through `XDG_SESSION_TYPE=wayland` or `WAYLAND_DISPLAY`.
- Use `org.freedesktop.portal.Screenshot` through D-Bus for screenshots.
- Copy the returned `file://` URI into Sessio's paste cache and validate the output file is a non-empty PNG.
- Do not claim silent frontmost-window parity on Wayland, because the protocol intentionally prevents ordinary apps from reading the active window or enumerating other apps' windows.
- For `capture_frontmost_app_window_png`, use the portal screenshot flow as an interactive fallback.
- For overlay capture, open the existing Sessio overlay editor over the captured source image. Return no window candidates, so users can drag-select any area and still annotate/save.

## API and frontend changes

- Keep existing Tauri command names and TypeScript function names unchanged.
- Extend the overlay source shape with an optional initial selection:

```ts
export interface ScreenshotOverlayInitialSelection {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface ScreenshotOverlaySource {
  requestId: string;
  sourcePath: string;
  fileName: string;
  mode?: "interactive" | "selection";
  windows: ScreenshotOverlayWindowCandidate[];
  initialSelection?: ScreenshotOverlayInitialSelection | null;
}
```

- In `ScreenshotOverlayWindow`, initialize `selection` from `source.initialSelection` when present. This supports future portal implementations that can return a selected region, while preserving current behavior when it is absent.
- Keep existing annotation, save, cancel, and attachment insertion logic unchanged.
- Update permission/settings presentation:
  - Linux should not show macOS-style managed permission buttons.
  - Linux copy should explain that screenshot permission is handled by the desktop portal at capture time.
  - Windows remains "no extra system permission required."

## Test plan

Run automated checks:

- `pnpm test`
- `pnpm run build`
- `cargo check --manifest-path src-tauri/Cargo.toml`
- On a Linux desktop or CI image with the Linux system libraries installed, run `cargo check --manifest-path src-tauri/Cargo.toml` again to exercise Linux-only cfg paths.

Add or update focused tests:

- `appshotPermissionPresentation` for Linux portal permission presentation.
- Overlay geometry and source initialization for `initialSelection`.
- Linux backend pure helpers for session detection, rectangle clipping, safe URI handling, X11 candidate ordering, and pixel format conversion.

Manual Linux acceptance scenarios:

- X11: global Appshot captures the active external app window.
- X11: screenshot button opens overlay, shows window hover candidates, supports click-to-select, drag-to-select, annotation, save, and cancel.
- X11: hide-self hides Sessio before capture and restores it after save/cancel.
- Wayland GNOME/KDE: portal prompt appears, captured image opens in the existing overlay editor, drag selection and annotation work, save inserts a PNG attachment.
- Wayland: canceling the portal or overlay reports a clean cancellation and restores Sessio when hide-self was enabled.
- Multi-monitor: overlay opens on the monitor containing the main Sessio window; X11 candidate rectangles are clipped to that monitor.

## Assumptions and limits

- X11 is the Linux path for full Windows-style automatic active-window capture and window click selection.
- Wayland cannot provide silent frontmost-window capture or arbitrary window enumeration without compositor-specific/private protocols, so the supported fallback is portal-authorized interactive capture.
- No macOS or Windows behavior should change, except for sharing generic overlay completion code where useful.
- The first implementation should avoid external screenshot command dependencies such as `grim`, `slurp`, `scrot`, or `gnome-screenshot`.
