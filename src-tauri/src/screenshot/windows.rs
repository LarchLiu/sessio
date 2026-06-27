use crate::{
    app_paths::paste_cache_dir, safe_pasted_attachment_file_name, SavedPastedAttachment,
    ScreenshotOverlayWindowCandidateDto,
};
use tauri::{Manager, WebviewWindow};

#[derive(Clone, Copy)]
pub(crate) struct Rect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

fn virtual_screen_rect() -> Rect {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        Rect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        }
    }
}

pub(crate) fn monitor_rect_for_window(window: &WebviewWindow) -> Rect {
    use windows::Win32::{
        Foundation::RECT,
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
    };
    let hwnd = window.hwnd().ok();
    unsafe {
        let monitor = hwnd
            .map(|hwnd| MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST))
            .unwrap_or_default();
        if !monitor.is_invalid() {
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info as *mut _).as_bool() {
                let RECT {
                    left,
                    top,
                    right,
                    bottom,
                } = info.rcMonitor;
                return Rect {
                    x: left,
                    y: top,
                    width: (right - left).max(1),
                    height: (bottom - top).max(1),
                };
            }
        }
    }
    virtual_screen_rect()
}

pub(crate) fn window_scale_factor(window: &WebviewWindow) -> f64 {
    if let Ok(hwnd) = window.hwnd() {
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(hwnd) };
        if dpi > 0 {
            return (dpi as f64 / 96.0).max(1.0);
        }
    }
    window.scale_factor().unwrap_or(1.0).max(1.0)
}

pub(crate) fn capture_screen_rect_png(
    rect: Rect,
    file_name: Option<&str>,
    prefix: &str,
) -> Result<SavedPastedAttachment, String> {
    use image::{ImageBuffer, Rgba};
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT,
        DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, RGBQUAD, SRCCOPY,
    };

    struct ScreenDc(HDC);
    impl Drop for ScreenDc {
        fn drop(&mut self) {
            unsafe {
                let _ = ReleaseDC(None, self.0);
            }
        }
    }
    struct MemDc(HDC);
    impl Drop for MemDc {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
    struct Bitmap(HBITMAP);
    impl Drop for Bitmap {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(HGDIOBJ::from(self.0));
            }
        }
    }

    if rect.width <= 0 || rect.height <= 0 {
        return Err("Screenshot capture area is empty".to_string());
    }

    let width = rect.width;
    let height = rect.height;
    let mut buffer = vec![0u8; (width as usize) * (height as usize) * 4];
    unsafe {
        let screen_dc = ScreenDc(GetDC(None));
        if screen_dc.0.is_invalid() {
            return Err("Failed to get screen device context".to_string());
        }
        let mem_dc = MemDc(CreateCompatibleDC(Some(screen_dc.0)));
        if mem_dc.0.is_invalid() {
            return Err("Failed to create screenshot memory device context".to_string());
        }
        let bitmap = Bitmap(CreateCompatibleBitmap(screen_dc.0, width, height));
        if bitmap.0.is_invalid() {
            return Err("Failed to create screenshot bitmap".to_string());
        }
        let old = SelectObject(mem_dc.0, HGDIOBJ::from(bitmap.0));
        if old.is_invalid() {
            return Err("Failed to select screenshot bitmap".to_string());
        }
        if let Err(error) = BitBlt(
            mem_dc.0,
            0,
            0,
            width,
            height,
            Some(screen_dc.0),
            rect.x,
            rect.y,
            SRCCOPY | CAPTUREBLT,
        ) {
            let _ = SelectObject(mem_dc.0, old);
            return Err(format!("Failed to copy screen pixels: {error}"));
        }
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            bmiColors: [RGBQUAD::default(); 1],
        };
        let lines = GetDIBits(
            mem_dc.0,
            bitmap.0,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut bmi as *mut _,
            DIB_RGB_COLORS,
        );
        let _ = SelectObject(mem_dc.0, old);
        if lines == 0 {
            return Err("Failed to read screenshot pixels".to_string());
        }
    }

    for px in buffer.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    let image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width as u32, height as u32, buffer)
        .ok_or_else(|| "Failed to build screenshot image buffer".to_string())?;

    let file_name = safe_pasted_attachment_file_name(file_name, Some("image/png"));
    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "{prefix}-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));
    image.save(&path).map_err(|e| e.to_string())?;
    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

fn window_text(hwnd: windows::Win32::Foundation::HWND) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buffer = vec![0u16; (len + 1) as usize];
        let read = GetWindowTextW(hwnd, &mut buffer);
        if read <= 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..read as usize]))
    }
}

fn process_name(pid: u32) -> Option<String> {
    use windows::Win32::{
        Foundation::{CloseHandle, MAX_PATH},
        System::{
            ProcessStatus::K32GetModuleBaseNameW,
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
    };
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .ok()?;
        let mut buffer = vec![0u16; MAX_PATH as usize];
        let len = K32GetModuleBaseNameW(handle, None, &mut buffer);
        let _ = CloseHandle(handle);
        if len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn extended_window_rect(hwnd: windows::Win32::Foundation::HWND) -> Option<Rect> {
    use windows::Win32::{
        Foundation::RECT,
        Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
        UI::WindowsAndMessaging::GetWindowRect,
    };
    unsafe {
        let mut rect = RECT::default();
        let dwm_ok = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if !dwm_ok && GetWindowRect(hwnd, &mut rect as *mut _).is_err() {
            return None;
        }
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 {
            return None;
        }
        Some(Rect {
            x: rect.left,
            y: rect.top,
            width,
            height,
        })
    }
}

fn is_capture_candidate(hwnd: windows::Win32::Foundation::HWND) -> bool {
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetWindowLongW, IsIconic, IsWindowVisible, GA_ROOT, GWL_EXSTYLE,
        WS_EX_TOOLWINDOW,
    };
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut cloaked = 0i32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<i32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return false;
        }
        if GetAncestor(hwnd, GA_ROOT) != hwnd {
            return false;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return false;
        }
    }
    extended_window_rect(hwnd)
        .map(|rect| rect.width >= 36 && rect.height >= 28)
        .unwrap_or(false)
}

pub(crate) fn window_candidates_for_rect(
    screen_rect: Rect,
) -> Vec<ScreenshotOverlayWindowCandidateDto> {
    use windows::core::BOOL;
    use windows::Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId},
    };
    struct EnumState {
        screen_rect: Rect,
        items: Vec<ScreenshotOverlayWindowCandidateDto>,
    }
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut EnumState);
        if !is_capture_candidate(hwnd) {
            return true.into();
        }
        let Some(rect) = extended_window_rect(hwnd) else {
            return true.into();
        };
        let left = rect.x.max(state.screen_rect.x);
        let top = rect.y.max(state.screen_rect.y);
        let right = (rect.x + rect.width).min(state.screen_rect.x + state.screen_rect.width);
        let bottom = (rect.y + rect.height).min(state.screen_rect.y + state.screen_rect.height);
        if right <= left || bottom <= top {
            return true.into();
        }
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut _));
        }
        state.items.push(ScreenshotOverlayWindowCandidateDto {
            id: (hwnd.0 as usize).to_string(),
            app_name: process_name(pid).unwrap_or_else(|| "Window".to_string()),
            title: window_text(hwnd),
            x: f64::from(left - state.screen_rect.x),
            y: f64::from(top - state.screen_rect.y),
            width: f64::from(right - left),
            height: f64::from(bottom - top),
        });
        true.into()
    }

    let mut state = EnumState {
        screen_rect,
        items: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut state as *mut _ as isize));
    }
    state.items
}

pub(crate) fn capture_frontmost_window_png(
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return Err("Could not determine the foreground window".to_string());
    }
    let rect = extended_window_rect(hwnd)
        .ok_or_else(|| "Could not determine the foreground window bounds".to_string())?;
    capture_screen_rect_png(rect, file_name.as_deref(), "appshot")
}

pub(crate) fn capture_monitor_background_png(
    rect: Rect,
    file_name: Option<&str>,
) -> Result<SavedPastedAttachment, String> {
    capture_screen_rect_png(rect, file_name, "screen-overlay")
}
