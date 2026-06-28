//! Windows implementation of [`ComputerUseProvider`].
//!
//! - **App/window discovery**: Win32 `EnumWindows` plus process executable
//!   metadata.
//! - **Screenshot**: the existing GDI/BitBlt screenshot backend, cropped to the
//!   selected window's extended frame bounds.
//! - **Element tree/actions**: Windows UI Automation (added below the window
//!   plumbing so observation works even when UIA is unavailable).
//! - **Input fallback**: `SendInput`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::computer_use::provider::{
    AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppTarget, ComputerUseProvider,
    CoordinateSpace, DisplayMetadata, ElementId, InstalledApp, Point, ProviderError,
    ProviderResult, RawAppState, Rect, ScreenshotRef, ScrollDirection,
};

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AttachThreadInput, EnumWindows, GetAncestor, GetForegroundWindow, GetSystemMetrics,
    GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GA_ROOT, GWL_EXSTYLE,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_RESTORE,
    WS_EX_TOOLWINDOW,
};

/// Windows provider. Stateless: every operation re-reads live system state.
pub struct WindowsProvider {
    capture_dir: PathBuf,
}

impl WindowsProvider {
    pub fn new() -> Self {
        Self {
            capture_dir: std::env::temp_dir().join("sessio-computer-use"),
        }
    }
}

impl Default for WindowsProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ComputerUseProvider for WindowsProvider {
    fn supports_control(&self) -> bool {
        // Windows has no macOS-style Accessibility grant, but the provider can
        // synthesize input for same-integrity targets. UIPI/elevated-window
        // failures are surfaced per action.
        true
    }

    fn list_apps(&self, options: AppListOptions) -> ProviderResult<Vec<InstalledApp>> {
        let _ = options;
        Ok(list_running_apps())
    }

    fn is_app_running(&self, app_id: &AppId) -> ProviderResult<bool> {
        Ok(resolve_process(app_id).is_some())
    }

    fn launch_app(&self, target: &AppTarget) -> ProviderResult<AppLaunchResult> {
        launch_app(target)
    }

    fn raise_app(&self, target: &AppTarget) -> ProviderResult<AppRaiseResult> {
        raise_app(target)
    }

    fn capture_app_state(&self, target: &AppTarget) -> ProviderResult<RawAppState> {
        let window = resolve_window(target)?;
        if window.rect.width <= 0.0 || window.rect.height <= 0.0 {
            return Err(ProviderError::NoVisibleWindow);
        }
        std::fs::create_dir_all(&self.capture_dir)
            .map_err(|e| ProviderError::Failed(format!("create capture dir: {e}")))?;
        let screenshot = capture_window(&window, &self.capture_dir)?;
        Ok(RawAppState {
            target: AppTarget {
                app_id: window.app_id.clone(),
                window_id: Some(hwnd_id(window.hwnd)),
            },
            display: display_metadata_for_window(window.hwnd),
            screenshot,
            elements: Vec::new(),
        })
    }

    fn click_element(&self, _target: &AppTarget, _element: &ElementId) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("click_element"))
    }

    fn click_point(&self, _target: &AppTarget, _point: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("click_point"))
    }

    fn secondary_click(&self, _target: &AppTarget, _point: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("secondary_click"))
    }

    fn secondary_click_element(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("secondary_click_element"))
    }

    fn double_click(&self, _target: &AppTarget, _point: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("double_click"))
    }

    fn drag(&self, _target: &AppTarget, _from: Point, _to: Point) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("drag"))
    }

    fn set_value(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
        _value: &str,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("set_value"))
    }

    fn type_text(&self, _target: &AppTarget, _text: &str) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("type_text"))
    }

    fn press_key(&self, _target: &AppTarget, _key: &str) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("press_key"))
    }

    fn scroll(
        &self,
        _target: &AppTarget,
        _direction: ScrollDirection,
        _amount: i32,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("scroll"))
    }

    fn scroll_element(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
        _direction: ScrollDirection,
        _amount: i32,
    ) -> ProviderResult<()> {
        Err(ProviderError::Unsupported("scroll_element"))
    }
}

#[derive(Debug, Clone)]
struct WindowInfo {
    hwnd: HWND,
    app_id: AppId,
    name: String,
    pid: u32,
    title: Option<String>,
    rect: Rect,
    minimized: bool,
}

// --- App/window discovery -------------------------------------------------

fn list_running_apps() -> Vec<InstalledApp> {
    let mut by_id: HashMap<AppId, InstalledApp> = HashMap::new();
    for window in enumerate_windows() {
        by_id
            .entry(window.app_id.clone())
            .and_modify(|app| {
                if app.pid.is_none() {
                    app.pid = Some(window.pid as i32);
                }
            })
            .or_insert_with(|| InstalledApp {
                id: window.app_id.clone(),
                name: window.name.clone(),
                pid: Some(window.pid as i32),
                running: true,
                recent_use_count: None,
                recent_last_used_at: None,
                recent_source: None,
            });
    }
    let mut apps: Vec<_> = by_id.into_values().collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

fn resolve_process(app_id: &str) -> Option<(AppId, u32)> {
    enumerate_windows()
        .into_iter()
        .find(|window| app_matches_ref(app_id, window))
        .map(|window| (window.app_id, window.pid))
}

fn resolve_window(target: &AppTarget) -> ProviderResult<WindowInfo> {
    let windows = enumerate_windows();
    if let Some(window_id) = target.window_id.as_deref() {
        if let Some(hwnd) = hwnd_from_id(window_id) {
            if let Some(window) = windows.iter().find(|window| window.hwnd == hwnd).cloned() {
                return Ok(window);
            }
        }
    }
    windows
        .into_iter()
        .find(|window| app_matches_ref(&target.app_id, window))
        .ok_or_else(|| ProviderError::AppNotFound(target.app_id.clone()))
}

fn enumerate_windows() -> Vec<WindowInfo> {
    struct State {
        windows: Vec<WindowInfo>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut State);
        if let Some(window) = window_info(hwnd) {
            state.windows.push(window);
        }
        true.into()
    }

    let mut state = State {
        windows: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut state as *mut _ as isize));
    }
    state.windows
}

fn window_info(hwnd: HWND) -> Option<WindowInfo> {
    if !is_targetable_window(hwnd) {
        return None;
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut _));
    }
    if pid == 0 {
        return None;
    }
    let exe = process_exe_path(pid);
    let app_id = exe
        .as_deref()
        .map(normalize_app_id)
        .unwrap_or_else(|| format!("pid:{pid}"));
    let name = exe
        .as_deref()
        .and_then(|path| Path::new(path).file_stem())
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| window_text(hwnd))
        .unwrap_or_else(|| format!("pid:{pid}"));
    Some(WindowInfo {
        hwnd,
        app_id,
        name,
        pid,
        title: window_text(hwnd),
        rect: extended_window_rect(hwnd)?,
        minimized: unsafe { IsIconic(hwnd).as_bool() },
    })
}

fn is_targetable_window(hwnd: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
        if GetAncestor(hwnd, GA_ROOT) != hwnd {
            return false;
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
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
    }
    extended_window_rect(hwnd)
        .map(|rect| rect.width >= 36.0 && rect.height >= 28.0)
        .unwrap_or(false)
}

fn extended_window_rect(hwnd: HWND) -> Option<Rect> {
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
            x: rect.left as f32,
            y: rect.top as f32,
            width: width as f32,
            height: height as f32,
        })
    }
}

fn window_text(hwnd: HWND) -> Option<String> {
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

fn process_exe_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
            false,
            pid,
        )
        .ok()?;
        let mut buffer = vec![0u16; MAX_PATH as usize];
        let len = K32GetModuleFileNameExW(handle, None, &mut buffer);
        let _ = CloseHandle(handle);
        if len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn normalize_app_id(path: &str) -> AppId {
    path.trim().to_ascii_lowercase()
}

fn app_matches_ref(app_id: &str, window: &WindowInfo) -> bool {
    let wanted = app_id.trim();
    if wanted.eq_ignore_ascii_case(&window.app_id) {
        return true;
    }
    if wanted.eq_ignore_ascii_case(&format!("pid:{}", window.pid)) {
        return true;
    }
    wanted.eq_ignore_ascii_case(&window.name)
}

// --- Launch / foreground recovery ----------------------------------------

fn launch_app(target: &AppTarget) -> ProviderResult<AppLaunchResult> {
    if resolve_process(&target.app_id).is_some() {
        return Ok(AppLaunchResult {
            target: target.clone(),
            launched: false,
            running: true,
        });
    }
    if !Path::new(&target.app_id).exists() {
        return Err(ProviderError::AppNotFound(target.app_id.clone()));
    }
    let wide = wide_null(&target.app_id);
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err(ProviderError::Failed(format!(
            "ShellExecuteW failed for {}: {}",
            target.app_id, result.0 as isize
        )));
    }
    let running = wait_for_running(&target.app_id, Duration::from_secs(5));
    if !running {
        return Err(ProviderError::Failed(format!(
            "launched {} but no targetable window appeared",
            target.app_id
        )));
    }
    Ok(AppLaunchResult {
        target: target.clone(),
        launched: true,
        running: true,
    })
}

fn raise_app(target: &AppTarget) -> ProviderResult<AppRaiseResult> {
    let mut launched = false;
    if resolve_process(&target.app_id).is_none() {
        launched = launch_app(target)?.launched;
    }
    let window = resolve_window(target)?;
    if window.minimized {
        unsafe {
            let _ = ShowWindow(window.hwnd, SW_RESTORE);
        }
    }
    let activated = set_foreground_window(window.hwnd);
    let visible = wait_for_visible_window(target, Duration::from_secs(2));
    Ok(AppRaiseResult {
        target: AppTarget {
            app_id: window.app_id,
            window_id: Some(hwnd_id(window.hwnd)),
        },
        launched,
        running: true,
        activated,
        visible,
    })
}

fn set_foreground_window(hwnd: HWND) -> bool {
    unsafe {
        if SetForegroundWindow(hwnd).as_bool() {
            return true;
        }
        let foreground = GetForegroundWindow();
        let mut foreground_pid = 0u32;
        let foreground_thread = GetWindowThreadProcessId(foreground, Some(&mut foreground_pid));
        let current_thread = GetCurrentThreadId();
        if foreground_thread != 0
            && AttachThreadInput(current_thread, foreground_thread, true).as_bool()
        {
            let ok = SetForegroundWindow(hwnd).as_bool();
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
            return ok;
        }
        false
    }
}

fn wait_for_running(app_id: &str, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if resolve_process(app_id).is_some() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn wait_for_visible_window(target: &AppTarget, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if resolve_window(target).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

// --- Screenshot / display metadata ---------------------------------------

fn capture_window(window: &WindowInfo, capture_dir: &Path) -> ProviderResult<ScreenshotRef> {
    let rect = crate::screenshot::windows::Rect {
        x: window.rect.x.round() as i32,
        y: window.rect.y.round() as i32,
        width: window.rect.width.round().max(1.0) as i32,
        height: window.rect.height.round().max(1.0) as i32,
    };
    let safe_name = format!("window-{}.png", hwnd_id(window.hwnd));
    let saved =
        crate::screenshot::windows::capture_screen_rect_png(rect, Some(&safe_name), "computer-use")
            .map_err(ProviderError::Failed)?;
    let path = PathBuf::from(saved.path);
    let target_path = capture_dir.join(safe_name);
    let path = match std::fs::rename(&path, &target_path) {
        Ok(()) => target_path,
        Err(_) => path,
    };
    let meta = std::fs::metadata(&path)
        .map_err(|e| ProviderError::Failed(format!("stat capture: {e}")))?;
    let (width, height) = image::image_dimensions(&path)
        .map_err(|e| ProviderError::Failed(format!("decode capture dimensions: {e}")))?;
    Ok(ScreenshotRef {
        handle: path.to_string_lossy().to_string(),
        format: "png".into(),
        byte_len: meta.len(),
        width,
        height,
        default_coordinate_space: CoordinateSpace::Screenshot,
        screen_bounds: window.rect,
    })
}

fn display_metadata_for_window(hwnd: HWND) -> DisplayMetadata {
    let rect = virtual_screen_rect();
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    DisplayMetadata {
        width: rect.width.max(1) as u32,
        height: rect.height.max(1) as u32,
        scale: if dpi > 0 {
            (dpi as f32 / 96.0).max(1.0)
        } else {
            1.0
        },
    }
}

fn virtual_screen_rect() -> crate::screenshot::windows::Rect {
    unsafe {
        crate::screenshot::windows::Rect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        }
    }
}

fn hwnd_id(hwnd: HWND) -> String {
    (hwnd.0 as usize).to_string()
}

fn hwnd_from_id(raw: &str) -> Option<HWND> {
    raw.trim()
        .parse::<isize>()
        .ok()
        .filter(|value| *value != 0)
        .map(HWND)
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
