use std::collections::HashMap;
use std::path::{Path, PathBuf};

use image::{ImageBuffer, Rgba};
use x11rb::connection::Connection;

use crate::{
    app_paths::paste_cache_dir, safe_pasted_attachment_file_name, SavedPastedAttachment,
    ScreenshotOverlayWindowCandidateDto,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct LinuxScreenRect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinuxScreenshotBackend {
    X11,
    Portal,
}

pub(super) fn preferred_backend() -> LinuxScreenshotBackend {
    let session = std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let display = std::env::var_os("DISPLAY").is_some();
    let wayland_display = std::env::var_os("WAYLAND_DISPLAY").is_some();
    backend_for_session(&session, display, wayland_display)
}

fn backend_for_session(
    session: &str,
    display: bool,
    wayland_display: bool,
) -> LinuxScreenshotBackend {
    if session == "wayland" || wayland_display {
        LinuxScreenshotBackend::Portal
    } else if session == "x11" || display {
        LinuxScreenshotBackend::X11
    } else {
        LinuxScreenshotBackend::Portal
    }
}

pub(crate) fn monitor_rect(monitor: &tauri::Monitor) -> LinuxScreenRect {
    let pos = monitor.position();
    let size = monitor.size();
    LinuxScreenRect {
        x: pos.x,
        y: pos.y,
        width: size.width.max(1),
        height: size.height.max(1),
    }
}

pub(crate) fn capture_frontmost_window_png(
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    match preferred_backend() {
        LinuxScreenshotBackend::X11 => x11_capture_frontmost_window(file_name),
        LinuxScreenshotBackend::Portal => portal_screenshot(file_name.as_deref(), "appshot"),
    }
}

pub(crate) fn capture_monitor_background_png(
    rect: LinuxScreenRect,
    file_name: Option<&str>,
) -> Result<SavedPastedAttachment, String> {
    match preferred_backend() {
        LinuxScreenshotBackend::X11 => x11_capture_rect_png(rect, file_name, "screen-overlay"),
        LinuxScreenshotBackend::Portal => portal_screenshot(file_name, "screen-overlay"),
    }
}

pub(crate) fn window_candidates_for_rect(
    rect: LinuxScreenRect,
) -> Vec<ScreenshotOverlayWindowCandidateDto> {
    if preferred_backend() != LinuxScreenshotBackend::X11 {
        return Vec::new();
    }
    x11_window_candidates_for_rect(rect).unwrap_or_default()
}

fn allocate_png_path(prefix: &str, file_name: Option<&str>) -> Result<PathBuf, String> {
    let file_name = safe_pasted_attachment_file_name(file_name, Some("image/png"));
    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(format!(
        "{prefix}-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    )))
}

fn ensure_non_empty_png(path: &Path, label: &str) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(path);
        return Err(format!("{label} produced an empty PNG"));
    }
    Ok(())
}

fn portal_screenshot(
    file_name: Option<&str>,
    prefix: &str,
) -> Result<SavedPastedAttachment, String> {
    use zbus::blocking::Proxy;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    let connection = zbus::blocking::Connection::session()
        .map_err(|e| format!("Failed to connect to desktop portal: {e}"))?;
    let proxy = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(|e| format!("Failed to create screenshot portal proxy: {e}"))?;

    let token = format!("sessio_{}", chrono::Utc::now().timestamp_millis());
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(token.as_str()));
    options.insert("interactive", Value::from(true));

    let handle: OwnedObjectPath = proxy
        .call("Screenshot", &("", options))
        .map_err(|e| format!("Desktop portal screenshot request failed: {e}"))?;
    let request_proxy = Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        handle.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(|e| format!("Failed to create portal request proxy: {e}"))?;
    let mut signals = request_proxy
        .receive_signal("Response")
        .map_err(|e| format!("Failed to watch portal screenshot response: {e}"))?;

    let message = signals
        .next()
        .ok_or_else(|| "Desktop portal screenshot response was cancelled".to_string())?;
    let (response, results): (u32, HashMap<String, OwnedValue>) = message
        .body()
        .deserialize()
        .map_err(|e| format!("Desktop portal screenshot response was invalid: {e}"))?;
    if response != 0 {
        return Err("Screenshot selection was cancelled".to_string());
    }
    let uri_value = results
        .get("uri")
        .ok_or_else(|| "Desktop portal screenshot response did not include a URI".to_string())?
        .clone();
    let uri = String::try_from(uri_value)
        .map_err(|_| "Desktop portal screenshot URI was not a string".to_string())?;
    let source = portal_file_uri_to_path(&uri)?;

    let path = allocate_png_path(prefix, file_name)?;
    std::fs::copy(&source, &path)
        .map_err(|e| format!("Failed to copy portal screenshot into paste cache: {e}"))?;
    ensure_non_empty_png(&path, "Desktop portal screenshot")?;
    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

fn portal_file_uri_to_path(uri: &str) -> Result<PathBuf, String> {
    url::Url::parse(uri)
        .map_err(|e| format!("Desktop portal screenshot URI was invalid: {e}"))?
        .to_file_path()
        .map_err(|_| "Desktop portal screenshot URI was not a local file".to_string())
}

fn x11_capture_frontmost_window(
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    let context = X11Context::connect()?;
    let active = context.active_window()?;
    let rect = context.window_rect(active)?;
    x11_capture_rect_png(rect, file_name.as_deref(), "appshot")
}

fn x11_capture_rect_png(
    rect: LinuxScreenRect,
    file_name: Option<&str>,
    prefix: &str,
) -> Result<SavedPastedAttachment, String> {
    let context = X11Context::connect()?;
    let image = context.capture_rect(rect)?;
    let path = allocate_png_path(prefix, file_name)?;
    image
        .save_with_format(&path, image::ImageFormat::Png)
        .map_err(|e| format!("Failed to write X11 screenshot PNG: {e}"))?;
    ensure_non_empty_png(&path, "X11 screenshot")?;
    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

fn x11_window_candidates_for_rect(
    screen_rect: LinuxScreenRect,
) -> Result<Vec<ScreenshotOverlayWindowCandidateDto>, String> {
    let context = X11Context::connect()?;
    let hidden_atom = context.intern_atom("_NET_WM_STATE_HIDDEN")?;
    let mut items = Vec::new();
    for window in topmost_first_stacking_order(context.client_list_stacking()?) {
        if !context.is_viewable_window(window)? {
            continue;
        }
        if context.window_has_state(window, hidden_atom)? {
            continue;
        }
        let Ok(rect) = context.window_rect(window) else {
            continue;
        };
        if rect.width < 36 || rect.height < 28 {
            continue;
        }
        let Some(intersection) = intersect_rect(rect, screen_rect) else {
            continue;
        };
        items.push(ScreenshotOverlayWindowCandidateDto {
            id: window.to_string(),
            app_name: context
                .window_class(window)
                .or_else(|| context.window_name(window))
                .unwrap_or_else(|| "Window".to_string()),
            title: context.window_name(window),
            x: f64::from(intersection.x - screen_rect.x),
            y: f64::from(intersection.y - screen_rect.y),
            width: f64::from(intersection.width),
            height: f64::from(intersection.height),
        });
    }
    Ok(items)
}

fn topmost_first_stacking_order(
    mut windows: Vec<x11rb::protocol::xproto::Window>,
) -> Vec<x11rb::protocol::xproto::Window> {
    windows.reverse();
    windows
}

fn intersect_rect(a: LinuxScreenRect, b: LinuxScreenRect) -> Option<LinuxScreenRect> {
    let left = i64::from(a.x.max(b.x));
    let top = i64::from(a.y.max(b.y));
    let right = (i64::from(a.x) + i64::from(a.width)).min(i64::from(b.x) + i64::from(b.width));
    let bottom = (i64::from(a.y) + i64::from(a.height)).min(i64::from(b.y) + i64::from(b.height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(LinuxScreenRect {
        x: left as i32,
        y: top as i32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
}

struct X11Context {
    connection: x11rb::rust_connection::RustConnection,
    screen_num: usize,
}

impl X11Context {
    fn connect() -> Result<Self, String> {
        let (connection, screen_num) =
            x11rb::connect(None).map_err(|e| format!("Failed to connect to X11: {e}"))?;
        Ok(Self {
            connection,
            screen_num,
        })
    }

    fn screen(&self) -> &x11rb::protocol::xproto::Screen {
        &self.connection.setup().roots[self.screen_num]
    }

    fn root_rect(&self) -> LinuxScreenRect {
        LinuxScreenRect {
            x: 0,
            y: 0,
            width: u32::from(self.screen().width_in_pixels).max(1),
            height: u32::from(self.screen().height_in_pixels).max(1),
        }
    }

    fn intern_atom(&self, name: &str) -> Result<x11rb::protocol::xproto::Atom, String> {
        use x11rb::protocol::xproto::ConnectionExt;
        self.connection
            .intern_atom(false, name.as_bytes())
            .map_err(|e| format!("Failed to request X11 atom {name}: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to read X11 atom {name}: {e}"))
            .map(|reply| reply.atom)
    }

    fn active_window(&self) -> Result<x11rb::protocol::xproto::Window, String> {
        let atom = self.intern_atom("_NET_ACTIVE_WINDOW")?;
        self.window_property_u32(
            self.screen().root,
            atom,
            x11rb::protocol::xproto::AtomEnum::WINDOW,
        )
        .and_then(|mut values| {
            values
                .drain(..)
                .next()
                .ok_or_else(|| "X11 active window property was empty".to_string())
        })
    }

    fn client_list_stacking(&self) -> Result<Vec<x11rb::protocol::xproto::Window>, String> {
        let atom = self.intern_atom("_NET_CLIENT_LIST_STACKING")?;
        self.window_property_u32(
            self.screen().root,
            atom,
            x11rb::protocol::xproto::AtomEnum::WINDOW,
        )
    }

    fn window_property_u32(
        &self,
        window: x11rb::protocol::xproto::Window,
        property: x11rb::protocol::xproto::Atom,
        kind: x11rb::protocol::xproto::AtomEnum,
    ) -> Result<Vec<u32>, String> {
        use x11rb::protocol::xproto::ConnectionExt;
        let reply = self
            .connection
            .get_property(false, window, property, kind, 0, u32::MAX)
            .map_err(|e| format!("Failed to request X11 property: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to read X11 property: {e}"))?;
        Ok(reply
            .value32()
            .map(|values| values.collect())
            .unwrap_or_default())
    }

    fn window_property_string(
        &self,
        window: x11rb::protocol::xproto::Window,
        property: x11rb::protocol::xproto::Atom,
        kind: x11rb::protocol::xproto::Atom,
    ) -> Option<String> {
        use x11rb::protocol::xproto::ConnectionExt;
        let reply = self
            .connection
            .get_property(false, window, property, kind, 0, 2048)
            .ok()?
            .reply()
            .ok()?;
        if reply.format != 8 || reply.value.is_empty() {
            return None;
        }
        Some(
            String::from_utf8_lossy(&reply.value)
                .trim_matches('\0')
                .trim()
                .to_string(),
        )
        .filter(|value| !value.is_empty())
    }

    fn window_name(&self, window: x11rb::protocol::xproto::Window) -> Option<String> {
        let utf8 = self.intern_atom("UTF8_STRING").ok()?;
        self.intern_atom("_NET_WM_NAME")
            .ok()
            .and_then(|atom| self.window_property_string(window, atom, utf8))
            .or_else(|| {
                self.window_property_string(
                    window,
                    x11rb::protocol::xproto::AtomEnum::WM_NAME.into(),
                    x11rb::protocol::xproto::AtomEnum::STRING.into(),
                )
            })
    }

    fn window_class(&self, window: x11rb::protocol::xproto::Window) -> Option<String> {
        let value = self.window_property_string(
            window,
            x11rb::protocol::xproto::AtomEnum::WM_CLASS.into(),
            x11rb::protocol::xproto::AtomEnum::STRING.into(),
        )?;
        value
            .split('\0')
            .filter(|part| !part.trim().is_empty())
            .last()
            .map(|part| part.trim().to_string())
    }

    fn is_viewable_window(&self, window: x11rb::protocol::xproto::Window) -> Result<bool, String> {
        use x11rb::protocol::xproto::{ConnectionExt, MapState};
        let attrs = self
            .connection
            .get_window_attributes(window)
            .map_err(|e| format!("Failed to request X11 window attributes: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to read X11 window attributes: {e}"))?;
        Ok(attrs.map_state == MapState::VIEWABLE)
    }

    fn window_has_state(
        &self,
        window: x11rb::protocol::xproto::Window,
        state: x11rb::protocol::xproto::Atom,
    ) -> Result<bool, String> {
        let atom = self.intern_atom("_NET_WM_STATE")?;
        Ok(self
            .window_property_u32(window, atom, x11rb::protocol::xproto::AtomEnum::ATOM)?
            .into_iter()
            .any(|value| value == state))
    }

    fn window_rect(
        &self,
        window: x11rb::protocol::xproto::Window,
    ) -> Result<LinuxScreenRect, String> {
        use x11rb::protocol::xproto::ConnectionExt;
        let geometry = self
            .connection
            .get_geometry(window)
            .map_err(|e| format!("Failed to request X11 window geometry: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to read X11 window geometry: {e}"))?;
        let translated = self
            .connection
            .translate_coordinates(window, self.screen().root, 0, 0)
            .map_err(|e| format!("Failed to request X11 window coordinates: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to read X11 window coordinates: {e}"))?;
        Ok(LinuxScreenRect {
            x: i32::from(translated.dst_x),
            y: i32::from(translated.dst_y),
            width: u32::from(geometry.width).max(1),
            height: u32::from(geometry.height).max(1),
        })
    }

    fn capture_rect(
        &self,
        rect: LinuxScreenRect,
    ) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>, String> {
        use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, ImageOrder};

        if rect.width == 0 || rect.height == 0 {
            return Err("X11 screenshot area is empty".to_string());
        }
        let rect = intersect_rect(rect, self.root_rect())
            .ok_or_else(|| "X11 screenshot area is outside the visible screen".to_string())?;
        let width = rect.width.min(u16::MAX as u32) as u16;
        let height = rect.height.min(u16::MAX as u32) as u16;
        let reply = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.screen().root,
                clamp_i16(rect.x),
                clamp_i16(rect.y),
                width,
                height,
                u32::MAX,
            )
            .map_err(|e| format!("Failed to request X11 screen image: {e}"))?
            .reply()
            .map_err(|e| format!("Failed to read X11 screen image: {e}"))?;
        let format = self
            .connection
            .setup()
            .pixmap_formats
            .iter()
            .find(|format| format.depth == reply.depth)
            .or_else(|| {
                self.connection
                    .setup()
                    .pixmap_formats
                    .iter()
                    .find(|format| format.depth == self.screen().root_depth)
            })
            .ok_or_else(|| "Could not determine X11 screenshot pixel format".to_string())?;
        let visual = self
            .root_visual()
            .ok_or_else(|| "Could not determine X11 root visual".to_string())?;
        let image_order = self.connection.setup().image_byte_order;
        let rgba = x11_image_to_rgba(
            &reply.data,
            u32::from(width),
            u32::from(height),
            u32::from(format.bits_per_pixel),
            u32::from(format.scanline_pad),
            image_order == ImageOrder::LSB_FIRST,
            visual.red_mask,
            visual.green_mask,
            visual.blue_mask,
        )?;
        ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(u32::from(width), u32::from(height), rgba)
            .ok_or_else(|| "Failed to build X11 screenshot image buffer".to_string())
    }

    fn root_visual(&self) -> Option<x11rb::protocol::xproto::Visualtype> {
        let root_visual = self.screen().root_visual;
        for depth in &self.screen().allowed_depths {
            for visual in &depth.visuals {
                if visual.visual_id == root_visual {
                    return Some(*visual);
                }
            }
        }
        None
    }
}

fn clamp_i16(value: i32) -> i16 {
    value.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn x11_image_to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    bits_per_pixel: u32,
    scanline_pad: u32,
    little_endian: bool,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
) -> Result<Vec<u8>, String> {
    let bytes_per_pixel = (bits_per_pixel / 8).max(1) as usize;
    let scanline_pad = scanline_pad.max(8) as usize;
    let row_bits = width as usize * bits_per_pixel as usize;
    let stride = row_bits.div_ceil(scanline_pad) * (scanline_pad / 8);
    let required = stride
        .checked_mul(height as usize)
        .ok_or_else(|| "X11 screenshot image is too large".to_string())?;
    if data.len() < required {
        return Err("X11 screenshot image data was truncated".to_string());
    }

    let mut out = vec![0u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let offset = y * stride + x * bytes_per_pixel;
            let pixel = read_x11_pixel(&data[offset..offset + bytes_per_pixel], little_endian);
            let out_offset = (y * width as usize + x) * 4;
            out[out_offset] = color_component(pixel, red_mask);
            out[out_offset + 1] = color_component(pixel, green_mask);
            out[out_offset + 2] = color_component(pixel, blue_mask);
            out[out_offset + 3] = 255;
        }
    }
    Ok(out)
}

fn read_x11_pixel(bytes: &[u8], little_endian: bool) -> u32 {
    let mut pixel = 0u32;
    if little_endian {
        for (index, byte) in bytes.iter().enumerate().take(4) {
            pixel |= u32::from(*byte) << (index * 8);
        }
    } else {
        for byte in bytes.iter().take(4) {
            pixel = (pixel << 8) | u32::from(*byte);
        }
    }
    pixel
}

fn color_component(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let bits = mask.count_ones();
    let raw = (pixel & mask) >> shift;
    let max = (1u32 << bits) - 1;
    ((raw * 255 + max / 2) / max) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersects_rectangles() {
        let a = LinuxScreenRect {
            x: 10,
            y: 20,
            width: 100,
            height: 80,
        };
        let b = LinuxScreenRect {
            x: 60,
            y: 10,
            width: 100,
            height: 40,
        };

        let rect = intersect_rect(a, b).unwrap();
        assert_eq!(rect.x, 60);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.width, 50);
        assert_eq!(rect.height, 30);
    }

    #[test]
    fn intersects_rectangles_without_i32_overflow() {
        let a = LinuxScreenRect {
            x: i32::MAX - 10,
            y: i32::MAX - 10,
            width: 100,
            height: 100,
        };
        let b = LinuxScreenRect {
            x: i32::MAX - 5,
            y: i32::MAX - 7,
            width: 20,
            height: 30,
        };

        let rect = intersect_rect(a, b).unwrap();
        assert_eq!(rect.x, i32::MAX - 5);
        assert_eq!(rect.y, i32::MAX - 7);
        assert_eq!(rect.width, 20);
        assert_eq!(rect.height, 30);
    }

    #[test]
    fn chooses_portal_for_wayland_displays() {
        assert_eq!(
            backend_for_session("wayland", true, true),
            LinuxScreenshotBackend::Portal
        );
        assert_eq!(
            backend_for_session("", true, true),
            LinuxScreenshotBackend::Portal
        );
        assert_eq!(
            backend_for_session("x11", true, true),
            LinuxScreenshotBackend::Portal
        );
    }

    #[test]
    fn chooses_x11_for_x11_sessions_or_display() {
        assert_eq!(
            backend_for_session("x11", true, false),
            LinuxScreenshotBackend::X11
        );
        assert_eq!(
            backend_for_session("", true, false),
            LinuxScreenshotBackend::X11
        );
        assert_eq!(
            backend_for_session("tty", true, false),
            LinuxScreenshotBackend::X11
        );
    }

    #[test]
    fn rejects_sessions_without_display() {
        assert_eq!(
            backend_for_session("", false, false),
            LinuxScreenshotBackend::Portal
        );
    }

    #[test]
    fn extracts_color_components_from_masks() {
        let pixel = 0x00_12_34_56;
        assert_eq!(color_component(pixel, 0x00ff0000), 0x12);
        assert_eq!(color_component(pixel, 0x0000ff00), 0x34);
        assert_eq!(color_component(pixel, 0x000000ff), 0x56);
    }

    #[test]
    fn converts_x11_pixels_to_rgba() {
        let data = [0x56, 0x34, 0x12, 0x00];
        let rgba = x11_image_to_rgba(
            &data, 1, 1, 32, 32, true, 0x00ff0000, 0x0000ff00, 0x000000ff,
        )
        .unwrap();
        assert_eq!(rgba, vec![0x12, 0x34, 0x56, 255]);
    }

    #[test]
    fn parses_portal_file_uri_to_path() {
        let path = portal_file_uri_to_path("file:///tmp/sessio%20screenshot.png").unwrap();
        assert_eq!(path, PathBuf::from("/tmp/sessio screenshot.png"));
    }

    #[test]
    fn rejects_non_file_portal_uri() {
        assert!(portal_file_uri_to_path("https://example.test/screenshot.png").is_err());
    }

    #[test]
    fn returns_x11_candidates_topmost_first() {
        assert_eq!(topmost_first_stacking_order(vec![1, 2, 3]), vec![3, 2, 1]);
    }
}
