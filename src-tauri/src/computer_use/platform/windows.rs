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
    ActionExecutionKind, ActionExecutionOutcome, ActionExecutionResult, ActionExecutionRoute,
    AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppTarget, ClickExecutionOutcome,
    ClickDispatchRoute, ClickExecutionResult, ClickExecutionRoute, ComputerUseProvider,
    CoordinateSpace, DisplayMetadata, ElementId, InstalledApp, Point, ProviderError,
    ProviderResult, RawAppState, Rect, ScreenshotCaptureKind, ScreenshotRef, ScrollDirection,
    UiElement,
};

use windows::core::{w, BOOL, BSTR, PCWSTR};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, MAX_PATH, RECT, RPC_E_CHANGED_MODE,
};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationScrollItemPattern, IUIAutomationSelectionItemPattern, IUIAutomationValuePattern,
    TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_CheckBoxControlTypeId,
    UIA_ComboBoxControlTypeId, UIA_CustomControlTypeId, UIA_DataGridControlTypeId,
    UIA_DataItemControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
    UIA_GroupControlTypeId, UIA_HyperlinkControlTypeId, UIA_ImageControlTypeId,
    UIA_InvokePatternId, UIA_ListControlTypeId, UIA_ListItemControlTypeId, UIA_MenuControlTypeId,
    UIA_MenuItemControlTypeId, UIA_PaneControlTypeId, UIA_RadioButtonControlTypeId,
    UIA_ScrollItemPatternId, UIA_SelectionItemPatternId, UIA_SliderControlTypeId,
    UIA_TabControlTypeId, UIA_TabItemControlTypeId, UIA_TextControlTypeId,
    UIA_ToolBarControlTypeId, UIA_TreeControlTypeId, UIA_TreeItemControlTypeId, UIA_ValuePatternId,
    UIA_WindowControlTypeId, UIA_CONTROLTYPE_ID,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL, MOUSEINPUT, VIRTUAL_KEY, VK_BACK, VK_CONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4,
    VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};
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
            elements: uia_elements_for_window(window.hwnd).unwrap_or_default(),
        })
    }

    fn click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult> {
        let window = prepare_target_for_control(target)?;
        let entry = uia_entry_for_id(window.hwnd, element)?;
        invoke_uia_element(&entry.element)?;
        Ok(ClickExecutionResult {
            route: ClickExecutionRoute::Uia,
            outcome: ClickExecutionOutcome::SemanticSuccess,
        })
    }

    fn click_point(
        &self,
        target: &AppTarget,
        point: Point,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult> {
        let _ = prepare_target_for_control(target)?;
        left_click_at(point)?;
        Ok(ClickExecutionResult {
            route: ClickExecutionRoute::Native,
            outcome: ClickExecutionOutcome::ObservedEffect,
        })
    }

    fn secondary_click(
        &self,
        target: &AppTarget,
        point: Point,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let _ = prepare_target_for_control(target)?;
        right_click_at(point)?;
        Ok(ActionExecutionResult {
            kind: ActionExecutionKind::SecondaryClick,
            route: ActionExecutionRoute::Native,
            outcome: ActionExecutionOutcome::Dispatched,
        })
    }

    fn secondary_click_element(
        &self,
        _target: &AppTarget,
        _element: &ElementId,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        Err(ProviderError::Unsupported("secondary_click_element"))
    }

    fn double_click(
        &self,
        target: &AppTarget,
        point: Point,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let _ = prepare_target_for_control(target)?;
        double_click_at(point)?;
        Ok(ActionExecutionResult {
            kind: ActionExecutionKind::DoubleClick,
            route: ActionExecutionRoute::Native,
            outcome: ActionExecutionOutcome::Dispatched,
        })
    }

    fn drag(
        &self,
        target: &AppTarget,
        from: Point,
        to: Point,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let _ = prepare_target_for_control(target)?;
        drag_between(from, to)?;
        Ok(ActionExecutionResult {
            kind: ActionExecutionKind::Drag,
            route: ActionExecutionRoute::Native,
            outcome: ActionExecutionOutcome::Dispatched,
        })
    }

    fn set_value(
        &self,
        target: &AppTarget,
        element: &ElementId,
        value: &str,
    ) -> ProviderResult<ActionExecutionResult> {
        let window = prepare_target_for_control(target)?;
        let entry = uia_entry_for_id(window.hwnd, element)?;
        set_uia_value(&entry.element, value)
    }

    fn type_text(&self, target: &AppTarget, text: &str) -> ProviderResult<()> {
        let _ = prepare_target_for_control(target)?;
        type_unicode(text)
    }

    fn press_key(&self, target: &AppTarget, key: &str) -> ProviderResult<()> {
        let _ = prepare_target_for_control(target)?;
        press_key_chord(key)
    }

    fn scroll(
        &self,
        target: &AppTarget,
        direction: ScrollDirection,
        amount: i32,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let _ = prepare_target_for_control(target)?;
        scroll_wheel(direction, amount)?;
        Ok(ActionExecutionResult {
            kind: ActionExecutionKind::Scroll,
            route: ActionExecutionRoute::Native,
            outcome: ActionExecutionOutcome::Dispatched,
        })
    }

    fn scroll_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        _direction: ScrollDirection,
        _amount: i32,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let window = prepare_target_for_control(target)?;
        let entry = uia_entry_for_id(window.hwnd, element)?;
        scroll_uia_element_into_view(&entry.element)?;
        Ok(ActionExecutionResult {
            kind: ActionExecutionKind::Scroll,
            route: ActionExecutionRoute::Uia,
            outcome: ActionExecutionOutcome::SemanticSuccess,
        })
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

// --- Control eligibility --------------------------------------------------

fn prepare_target_for_control(target: &AppTarget) -> ProviderResult<WindowInfo> {
    let raised = raise_app(target)?;
    let window = resolve_window(&raised.target)?;
    ensure_can_control_window(&window)?;
    Ok(window)
}

fn ensure_can_control_window(window: &WindowInfo) -> ProviderResult<()> {
    let current = current_process_integrity_rid()?;
    let target = process_integrity_rid(window.pid)?;
    if target > current {
        return Err(ProviderError::Failed(format!(
            "cannot control {} because its window has a higher Windows integrity level and is blocked by UIPI (current integrity RID {current}, target RID {target})",
            window.name
        )));
    }
    Ok(())
}

fn current_process_integrity_rid() -> ProviderResult<u32> {
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|e| ProviderError::Failed(format!("open current process token: {e}")))?;
    }
    let result = token_integrity_rid(token);
    unsafe {
        let _ = CloseHandle(token);
    }
    result
}

fn process_integrity_rid(pid: u32) -> ProviderResult<u32> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| ProviderError::Failed(format!("open target process {pid}: {e}")))?;
        let mut token = HANDLE::default();
        let result = OpenProcessToken(process, TOKEN_QUERY, &mut token)
            .map_err(|e| ProviderError::Failed(format!("open target process token {pid}: {e}")))
            .and_then(|_| {
                let rid = token_integrity_rid(token);
                let _ = CloseHandle(token);
                rid
            });
        let _ = CloseHandle(process);
        result
    }
}

fn token_integrity_rid(token: HANDLE) -> ProviderResult<u32> {
    unsafe {
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        if needed < std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
            return Err(ProviderError::Failed(
                "query token integrity level size returned no data".into(),
            ));
        }
        let mut buffer = vec![0u8; needed as usize];
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        )
        .map_err(|e| ProviderError::Failed(format!("query token integrity level: {e}")))?;
        let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        if sid.is_invalid() {
            return Err(ProviderError::Failed(
                "token integrity SID was unavailable".into(),
            ));
        }
        let count_ptr = GetSidSubAuthorityCount(sid);
        if count_ptr.is_null() || *count_ptr == 0 {
            return Err(ProviderError::Failed(
                "token integrity SID had no sub-authorities".into(),
            ));
        }
        let rid_ptr = GetSidSubAuthority(sid, (*count_ptr - 1) as u32);
        if rid_ptr.is_null() {
            return Err(ProviderError::Failed(
                "token integrity RID was unavailable".into(),
            ));
        }
        Ok(*rid_ptr)
    }
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
        capture_kind: Some(ScreenshotCaptureKind::ScreenRectGdi),
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

// --- Physical input fallback ---------------------------------------------

fn left_click_at(point: Point) -> ProviderResult<()> {
    let (x, y) = absolute_mouse_point(point);
    send_inputs(&[
        mouse_input(
            x,
            y,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse_input(x, y, 0, MOUSEEVENTF_LEFTDOWN),
        mouse_input(x, y, 0, MOUSEEVENTF_LEFTUP),
    ])
}

fn right_click_at(point: Point) -> ProviderResult<()> {
    let (x, y) = absolute_mouse_point(point);
    send_inputs(&[
        mouse_input(
            x,
            y,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse_input(x, y, 0, MOUSEEVENTF_RIGHTDOWN),
        mouse_input(x, y, 0, MOUSEEVENTF_RIGHTUP),
    ])
}

fn double_click_at(point: Point) -> ProviderResult<()> {
    left_click_at(point)?;
    thread::sleep(Duration::from_millis(60));
    left_click_at(point)
}

fn drag_between(from: Point, to: Point) -> ProviderResult<()> {
    let (from_x, from_y) = absolute_mouse_point(from);
    let (to_x, to_y) = absolute_mouse_point(to);
    send_inputs(&[
        mouse_input(
            from_x,
            from_y,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse_input(from_x, from_y, 0, MOUSEEVENTF_LEFTDOWN),
        mouse_input(
            to_x,
            to_y,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        mouse_input(to_x, to_y, 0, MOUSEEVENTF_LEFTUP),
    ])
}

fn type_unicode(text: &str) -> ProviderResult<()> {
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        inputs.push(keyboard_input(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
        inputs.push(keyboard_input(
            VIRTUAL_KEY(0),
            unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }
    send_inputs(&inputs)
}

fn press_key_chord(raw: &str) -> ProviderResult<()> {
    let mut modifiers = Vec::new();
    let mut main = None;
    for part in raw
        .split(|ch| ch == '+' || ch == '-')
        .map(|part| part.trim().to_ascii_lowercase())
        .filter(|part| !part.is_empty())
    {
        match part.as_str() {
            "ctrl" | "control" => modifiers.push(VK_CONTROL),
            "shift" => modifiers.push(VK_LSHIFT),
            "alt" | "option" => modifiers.push(VK_LMENU),
            "win" | "windows" | "cmd" | "command" | "meta" => modifiers.push(VK_LWIN),
            _ => main = key_from_name(&part),
        }
    }
    let Some(main) = main else {
        return Err(ProviderError::Failed(format!("unknown key: {raw}")));
    };

    let mut inputs = Vec::new();
    for modifier in &modifiers {
        inputs.push(keyboard_input(*modifier, 0, Default::default()));
    }
    inputs.push(keyboard_input(main, 0, Default::default()));
    inputs.push(keyboard_input(main, 0, KEYEVENTF_KEYUP));
    for modifier in modifiers.iter().rev() {
        inputs.push(keyboard_input(*modifier, 0, KEYEVENTF_KEYUP));
    }
    send_inputs(&inputs)
}

fn scroll_wheel(direction: ScrollDirection, amount: i32) -> ProviderResult<()> {
    let amount = amount.max(1).min(20);
    let delta = 120_i32.saturating_mul(amount);
    let (flags, signed_delta) = match direction {
        ScrollDirection::Up => (MOUSEEVENTF_WHEEL, delta),
        ScrollDirection::Down => (MOUSEEVENTF_WHEEL, -delta),
        ScrollDirection::Left => (MOUSEEVENTF_HWHEEL, -delta),
        ScrollDirection::Right => (MOUSEEVENTF_HWHEEL, delta),
    };
    send_inputs(&[mouse_input(0, 0, signed_delta as u32, flags)])
}

fn absolute_mouse_point(point: Point) -> (i32, i32) {
    let screen = virtual_screen_rect();
    let width = (screen.width - 1).max(1) as f32;
    let height = (screen.height - 1).max(1) as f32;
    let x = (((point.x - screen.x as f32) / width) * 65_535.0)
        .round()
        .clamp(0.0, 65_535.0) as i32;
    let y = (((point.y - screen.y as f32) / height) * 65_535.0)
        .round()
        .clamp(0.0, 65_535.0) as i32;
    (x, y)
}

fn mouse_input(
    dx: i32,
    dy: i32,
    mouse_data: u32,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::MOUSE_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: mouse_data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn keyboard_input(
    key: VIRTUAL_KEY,
    scan: u16,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_inputs(inputs: &[INPUT]) -> ProviderResult<()> {
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(ProviderError::Failed(format!(
            "SendInput delivered {sent}/{} events; the target may be elevated or blocked by UIPI",
            inputs.len()
        )))
    }
}

fn key_from_name(name: &str) -> Option<VIRTUAL_KEY> {
    match name {
        "enter" | "return" => Some(VK_RETURN),
        "esc" | "escape" => Some(VK_ESCAPE),
        "tab" => Some(VK_TAB),
        "space" => Some(VK_SPACE),
        "backspace" => Some(VK_BACK),
        "delete" | "del" => Some(VK_DELETE),
        "insert" | "ins" => Some(VK_INSERT),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pageup" | "page_up" => Some(VK_PRIOR),
        "pagedown" | "page_down" => Some(VK_NEXT),
        "left" => Some(VK_LEFT),
        "right" => Some(VK_RIGHT),
        "up" => Some(VK_UP),
        "down" => Some(VK_DOWN),
        "f1" => Some(VK_F1),
        "f2" => Some(VK_F2),
        "f3" => Some(VK_F3),
        "f4" => Some(VK_F4),
        "f5" => Some(VK_F5),
        "f6" => Some(VK_F6),
        "f7" => Some(VK_F7),
        "f8" => Some(VK_F8),
        "f9" => Some(VK_F9),
        "f10" => Some(VK_F10),
        "f11" => Some(VK_F11),
        "f12" => Some(VK_F12),
        _ => {
            let mut chars = name.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if ch.is_ascii_alphabetic() {
                return Some(VIRTUAL_KEY(ch.to_ascii_uppercase() as u16));
            }
            if ch.is_ascii_digit() {
                return Some(VIRTUAL_KEY(ch as u16));
            }
            None
        }
    }
}

// --- UI Automation --------------------------------------------------------

const MAX_UIA_ELEMENTS: i32 = 500;

struct ComApartment {
    should_uninitialize: bool,
}

impl ComApartment {
    fn init() -> ProviderResult<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            // The thread already has COM initialized with a different apartment
            // model. UIA can still work; we just must not uninitialize it.
            return Ok(Self {
                should_uninitialize: false,
            });
        }
        hr.ok()
            .map_err(|e| ProviderError::Failed(format!("initialize COM for UIA: {e}")))?;
        Ok(Self {
            should_uninitialize: true,
        })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct UiaElementEntry {
    ui: UiElement,
    element: IUIAutomationElement,
}

fn with_uia<T>(f: impl FnOnce(&IUIAutomation) -> ProviderResult<T>) -> ProviderResult<T> {
    let _apartment = ComApartment::init()?;
    let automation: IUIAutomation =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|e| ProviderError::Failed(format!("create UI Automation client: {e}")))?;
    f(&automation)
}

fn uia_elements_for_window(hwnd: HWND) -> ProviderResult<Vec<UiElement>> {
    Ok(uia_entries_for_window(hwnd)?
        .into_iter()
        .map(|entry| entry.ui)
        .collect())
}

fn uia_entry_for_id(hwnd: HWND, element_id: &ElementId) -> ProviderResult<UiaElementEntry> {
    uia_entries_for_window(hwnd)?
        .into_iter()
        .find(|entry| &entry.ui.id == element_id)
        .ok_or_else(|| ProviderError::ElementNotFound(element_id.clone()))
}

fn uia_entries_for_window(hwnd: HWND) -> ProviderResult<Vec<UiaElementEntry>> {
    with_uia(|automation| unsafe {
        let root = automation
            .ElementFromHandle(hwnd)
            .map_err(|e| ProviderError::Failed(format!("get UIA root for window: {e}")))?;
        let condition = automation
            .CreateTrueCondition()
            .map_err(|e| ProviderError::Failed(format!("create UIA condition: {e}")))?;
        let descendants = root
            .FindAll(TreeScope_Descendants, &condition)
            .map_err(|e| ProviderError::Failed(format!("enumerate UIA descendants: {e}")))?;
        let count = descendants.Length().unwrap_or(0).clamp(0, MAX_UIA_ELEMENTS);
        let mut out = Vec::with_capacity(count as usize + 1);
        if let Some(entry) = uia_entry_from_element(hwnd, 0, root) {
            out.push(entry);
        }
        for index in 0..count {
            if let Ok(element) = descendants.GetElement(index) {
                if let Some(entry) = uia_entry_from_element(hwnd, index + 1, element) {
                    out.push(entry);
                }
            }
        }
        Ok(out)
    })
}

fn uia_entry_from_element(
    hwnd: HWND,
    index: i32,
    element: IUIAutomationElement,
) -> Option<UiaElementEntry> {
    let control_type = unsafe { element.CurrentControlType().ok()? };
    let role = uia_role(control_type);
    let label = unsafe { element.CurrentName().ok() }
        .and_then(non_empty_bstr)
        .or_else(|| unsafe { element.CurrentAutomationId().ok() }.and_then(non_empty_bstr));
    let bounds = unsafe { element.CurrentBoundingRectangle().ok() }.and_then(rect_from_win32);
    let enabled = unsafe { element.CurrentIsEnabled().ok() }
        .map(|value| value.as_bool())
        .unwrap_or(true);
    let actionable = enabled && uia_element_actionable(&element, control_type);
    Some(UiaElementEntry {
        ui: UiElement {
            id: format!("win:{}:uia:{index}", hwnd_id(hwnd)),
            role,
            label,
            bounds,
            bounds_coordinate_space: bounds.map(|_| CoordinateSpace::Screen),
            actionable,
        },
        element,
    })
}

fn rect_from_win32(rect: RECT) -> Option<Rect> {
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

fn non_empty_bstr(value: BSTR) -> Option<String> {
    let text = value.to_string();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn uia_role(control_type: UIA_CONTROLTYPE_ID) -> String {
    let name = if control_type == UIA_ButtonControlTypeId {
        "UIAButton"
    } else if control_type == UIA_CheckBoxControlTypeId {
        "UIACheckBox"
    } else if control_type == UIA_ComboBoxControlTypeId {
        "UIAComboBox"
    } else if control_type == UIA_CustomControlTypeId {
        "UIACustom"
    } else if control_type == UIA_DataGridControlTypeId {
        "UIADataGrid"
    } else if control_type == UIA_DataItemControlTypeId {
        "UIADataItem"
    } else if control_type == UIA_DocumentControlTypeId {
        "UIADocument"
    } else if control_type == UIA_EditControlTypeId {
        "UIAEdit"
    } else if control_type == UIA_GroupControlTypeId {
        "UIAGroup"
    } else if control_type == UIA_HyperlinkControlTypeId {
        "UIAHyperlink"
    } else if control_type == UIA_ImageControlTypeId {
        "UIAImage"
    } else if control_type == UIA_ListControlTypeId {
        "UIAList"
    } else if control_type == UIA_ListItemControlTypeId {
        "UIAListItem"
    } else if control_type == UIA_MenuControlTypeId {
        "UIAMenu"
    } else if control_type == UIA_MenuItemControlTypeId {
        "UIAMenuItem"
    } else if control_type == UIA_PaneControlTypeId {
        "UIAPane"
    } else if control_type == UIA_RadioButtonControlTypeId {
        "UIARadioButton"
    } else if control_type == UIA_SliderControlTypeId {
        "UIASlider"
    } else if control_type == UIA_TabControlTypeId {
        "UIATab"
    } else if control_type == UIA_TabItemControlTypeId {
        "UIATabItem"
    } else if control_type == UIA_TextControlTypeId {
        "UIAText"
    } else if control_type == UIA_ToolBarControlTypeId {
        "UIAToolBar"
    } else if control_type == UIA_TreeControlTypeId {
        "UIATree"
    } else if control_type == UIA_TreeItemControlTypeId {
        "UIATreeItem"
    } else if control_type == UIA_WindowControlTypeId {
        "UIAWindow"
    } else {
        return format!("UIAControlType({})", control_type.0);
    };
    name.to_string()
}

fn uia_element_actionable(
    element: &IUIAutomationElement,
    control_type: UIA_CONTROLTYPE_ID,
) -> bool {
    has_pattern::<IUIAutomationInvokePattern>(element, UIA_InvokePatternId)
        || has_pattern::<IUIAutomationSelectionItemPattern>(element, UIA_SelectionItemPatternId)
        || has_pattern::<IUIAutomationValuePattern>(element, UIA_ValuePatternId)
        || has_pattern::<IUIAutomationScrollItemPattern>(element, UIA_ScrollItemPatternId)
        || control_type == UIA_ButtonControlTypeId
        || control_type == UIA_CheckBoxControlTypeId
        || control_type == UIA_ComboBoxControlTypeId
        || control_type == UIA_EditControlTypeId
        || control_type == UIA_HyperlinkControlTypeId
        || control_type == UIA_ListItemControlTypeId
        || control_type == UIA_MenuItemControlTypeId
        || control_type == UIA_RadioButtonControlTypeId
        || control_type == UIA_TabItemControlTypeId
        || control_type == UIA_TreeItemControlTypeId
}

fn has_pattern<T>(
    element: &IUIAutomationElement,
    pattern: windows::Win32::UI::Accessibility::UIA_PATTERN_ID,
) -> bool
where
    T: windows::core::Interface,
{
    unsafe { element.GetCurrentPatternAs::<T>(pattern).is_ok() }
}

fn invoke_uia_element(element: &IUIAutomationElement) -> ProviderResult<()> {
    unsafe {
        let _ = element.SetFocus();
        if let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
        {
            return pattern
                .Invoke()
                .map_err(|e| ProviderError::Failed(format!("UIA InvokePattern failed: {e}")));
        }
        if let Ok(pattern) = element
            .GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
        {
            return pattern.Select().map_err(|e| {
                ProviderError::Failed(format!("UIA SelectionItemPattern failed: {e}"))
            });
        }
    }
    Err(ProviderError::Unsupported("click_element"))
}

fn set_uia_value(
    element: &IUIAutomationElement,
    value: &str,
) -> ProviderResult<ActionExecutionResult> {
    unsafe {
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .map_err(|_| ProviderError::Unsupported("set_value"))?;
        pattern
            .SetValue(&BSTR::from(value))
            .map_err(|e| ProviderError::Failed(format!("UIA ValuePattern failed: {e}")))?;

        match pattern.CurrentValue() {
            Ok(current) if current.to_string() == value => Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SetValue,
                route: ActionExecutionRoute::Uia,
                outcome: ActionExecutionOutcome::SemanticSuccess,
            }),
            Ok(_) => Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SetValue,
                route: ActionExecutionRoute::Uia,
                outcome: ActionExecutionOutcome::NoEffect,
            }),
            Err(_) => Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SetValue,
                route: ActionExecutionRoute::Uia,
                outcome: ActionExecutionOutcome::Uncertain,
            }),
        }
    }
}

fn scroll_uia_element_into_view(element: &IUIAutomationElement) -> ProviderResult<()> {
    unsafe {
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationScrollItemPattern>(UIA_ScrollItemPatternId)
            .map_err(|_| ProviderError::Unsupported("scroll_element"))?;
        pattern
            .ScrollIntoView()
            .map_err(|e| ProviderError::Failed(format!("UIA ScrollItemPattern failed: {e}")))
    }
}
