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
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use image::{imageops::crop_imm, ImageReader};
use sha2::{Digest, Sha256};

use crate::computer_use::provider::{
    ActionExecutionKind, ActionExecutionOutcome, ActionExecutionResult, ActionExecutionRoute,
    AppId, AppLaunchResult, AppListOptions, AppRaiseResult, AppTarget, ClickDispatchRoute,
    ClickExecutionOutcome, ClickExecutionResult, ClickExecutionRoute, ComputerUseProvider,
    CoordinateSpace, DisplayMetadata, ElementId, InstalledApp, Point, ProviderCapabilities,
    ProviderError, ProviderResult, RawAppState, Rect, ScreenshotCaptureKind, ScreenshotRef,
    ScrollDirection, UiElement,
};

use windows::core::{w, Interface, BOOL, BSTR, PCWSTR};
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
    CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, STGM_READ,
};
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationInvokePattern,
    IUIAutomationScrollItemPattern, IUIAutomationScrollPattern, IUIAutomationSelectionItemPattern,
    IUIAutomationValuePattern, ScrollAmount, ScrollAmount_NoAmount, ScrollAmount_SmallDecrement,
    ScrollAmount_SmallIncrement, TreeScope_Descendants, UIA_ButtonControlTypeId,
    UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_CustomControlTypeId,
    UIA_DataGridControlTypeId, UIA_DataItemControlTypeId, UIA_DocumentControlTypeId,
    UIA_EditControlTypeId, UIA_GroupControlTypeId, UIA_HyperlinkControlTypeId,
    UIA_ImageControlTypeId, UIA_InvokePatternId, UIA_ListControlTypeId, UIA_ListItemControlTypeId,
    UIA_MenuControlTypeId, UIA_MenuItemControlTypeId, UIA_PaneControlTypeId,
    UIA_RadioButtonControlTypeId, UIA_ScrollItemPatternId, UIA_ScrollPatternId,
    UIA_SelectionItemPatternId, UIA_SliderControlTypeId, UIA_TabControlTypeId,
    UIA_TabItemControlTypeId, UIA_TextControlTypeId, UIA_ToolBarControlTypeId,
    UIA_TreeControlTypeId, UIA_TreeItemControlTypeId, UIA_ValuePatternId, UIA_WindowControlTypeId,
    UIA_CONTROLTYPE_ID,
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
use windows::Win32::UI::Shell::{IShellLinkW, ShellExecuteW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetAncestor, GetForegroundWindow, GetSystemMetrics, GetWindowLongW, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    SetForegroundWindow, ShowWindow, GA_ROOT, GWL_EXSTYLE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_RESTORE, WS_EX_TOOLWINDOW,
};

/// Windows provider. Most operations re-read live system state; installed-app
/// discovery is cached briefly to avoid repeated Start Menu scans.
pub struct WindowsProvider {
    capture_dir: PathBuf,
    installed_apps_cache: Mutex<Option<InstalledAppsCache>>,
}

impl WindowsProvider {
    pub fn new() -> Self {
        Self {
            capture_dir: std::env::temp_dir().join("sessio-computer-use"),
            installed_apps_cache: Mutex::new(None),
        }
    }

    fn list_installed_apps_cached(&self) -> Vec<InstalledApp> {
        let now = Instant::now();
        {
            let cache = self
                .installed_apps_cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(cached) = cache.as_ref() {
                if now.duration_since(cached.fetched_at) < INSTALLED_APPS_CACHE_TTL {
                    return cached.apps.clone();
                }
            }
        }

        let apps = list_installed_apps();
        let mut cache = self
            .installed_apps_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *cache = Some(InstalledAppsCache {
            fetched_at: now,
            apps: apps.clone(),
        });
        apps
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

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            click_element_routes: vec![ClickDispatchRoute::Auto, ClickDispatchRoute::Ax],
            click_at_routes: vec![ClickDispatchRoute::Auto],
            secondary_click_element_routes: vec![ClickDispatchRoute::Auto, ClickDispatchRoute::Ax],
            secondary_click_at_routes: vec![ClickDispatchRoute::Auto],
            double_click_at_routes: vec![ClickDispatchRoute::Auto],
            drag_routes: vec![ClickDispatchRoute::Auto],
            scroll_element_routes: vec![ClickDispatchRoute::Auto, ClickDispatchRoute::Ax],
            scroll_at_routes: vec![ClickDispatchRoute::Auto],
            supports_set_value: true,
            supports_type_text: true,
            supports_press_key: true,
        }
    }

    fn list_apps(&self, options: AppListOptions) -> ProviderResult<Vec<InstalledApp>> {
        let _ = options;
        Ok(merge_available_apps(
            self.list_installed_apps_cached(),
            list_running_apps(),
        ))
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
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult> {
        let window = prepare_target_for_control(target)?;
        let entry = uia_entry_for_id(window.hwnd, element)?;
        click_uia_element(&self.capture_dir, &entry, route_hint)
    }

    fn click_point(
        &self,
        target: &AppTarget,
        point: Point,
        _route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ClickExecutionResult> {
        let _ = prepare_target_for_control(target)?;
        perform_click_with_effect_probe(
            &self.capture_dir,
            Some(point),
            ClickExecutionRoute::Native,
            || left_click_at(point),
        )
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
            next_dispatch_route: None,
        })
    }

    fn secondary_click_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let window = prepare_target_for_control(target)?;
        let entry = uia_entry_for_id(window.hwnd, element)?;
        secondary_click_uia_element(&self.capture_dir, &entry, route_hint)
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
            next_dispatch_route: None,
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
            next_dispatch_route: None,
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
            next_dispatch_route: None,
        })
    }

    fn scroll_element(
        &self,
        target: &AppTarget,
        element: &ElementId,
        direction: ScrollDirection,
        amount: i32,
        route_hint: ClickDispatchRoute,
    ) -> ProviderResult<ActionExecutionResult> {
        let window = prepare_target_for_control(target)?;
        let entry = uia_entry_for_id(window.hwnd, element)?;
        scroll_uia_element(&self.capture_dir, &entry, direction, amount, route_hint)
    }
}

#[derive(Debug, Clone)]
struct WindowInfo {
    hwnd: HWND,
    app_id: AppId,
    name: String,
    pid: u32,
    rect: Rect,
    minimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenCaptureFingerprint {
    full_hash: [u8; 32],
    local_hash: Option<[u8; 32]>,
}

#[derive(Debug, Clone)]
struct InstalledAppsCache {
    fetched_at: Instant,
    apps: Vec<InstalledApp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EffectProbeOutcome {
    ObservedEffect,
    NoEffect,
    Uncertain,
}

#[derive(Debug, Clone, Copy)]
struct EffectProbeTiming {
    initial_delay_ms: u64,
    poll_interval_ms: u64,
    total_window_ms: u64,
}

impl EffectProbeTiming {
    #[cfg(not(test))]
    const DEFAULT: Self = Self {
        initial_delay_ms: 500,
        poll_interval_ms: 500,
        total_window_ms: 2000,
    };

    #[cfg(test)]
    const DEFAULT: Self = Self {
        initial_delay_ms: 0,
        poll_interval_ms: 0,
        total_window_ms: 0,
    };

    fn attempt_count(self) -> usize {
        if self.total_window_ms <= self.initial_delay_ms || self.poll_interval_ms == 0 {
            1
        } else {
            1 + ((self.total_window_ms - self.initial_delay_ms) / self.poll_interval_ms) as usize
        }
    }
}

const EFFECT_LOCAL_PROBE_RADIUS_PX: u32 = 72;
const INSTALLED_APPS_CACHE_TTL: Duration = Duration::from_secs(30);
// --- App/window discovery -------------------------------------------------

fn merge_available_apps(
    installed: Vec<InstalledApp>,
    running: Vec<InstalledApp>,
) -> Vec<InstalledApp> {
    let mut by_id: HashMap<AppId, InstalledApp> = HashMap::new();
    for app in installed {
        by_id.entry(app.id.clone()).or_insert(app);
    }
    for app in running {
        by_id
            .entry(app.id.clone())
            .and_modify(|existing| {
                existing.running = true;
                existing.pid = app.pid;
                if existing.name.trim().is_empty() {
                    existing.name = app.name.clone();
                }
            })
            .or_insert(app);
    }
    let mut apps: Vec<_> = by_id.into_values().collect();
    apps.sort_by(|a, b| {
        b.running
            .cmp(&a.running)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    apps
}

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

fn list_installed_apps() -> Vec<InstalledApp> {
    match ShortcutResolver::new() {
        Ok(resolver) => list_installed_apps_from_roots(start_menu_roots(), |path| {
            installed_app_from_shortcut(path, Some(&resolver))
        }),
        Err(_) => list_installed_apps_from_roots(start_menu_roots(), |path| {
            installed_app_from_shortcut(path, None)
        }),
    }
}

fn list_installed_apps_from_roots<F>(
    roots: Vec<PathBuf>,
    mut resolve_shortcut: F,
) -> Vec<InstalledApp>
where
    F: FnMut(&Path) -> Option<InstalledApp>,
{
    let mut by_id: HashMap<AppId, InstalledApp> = HashMap::new();
    for root in roots {
        scan_start_menu_dir_with(&root, &mut by_id, &mut resolve_shortcut);
    }
    by_id.into_values().collect()
}

fn start_menu_roots() -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from(
        r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs",
    )];
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(r"AppData\Roaming\Microsoft\Windows\Start Menu\Programs"));
    }
    out
}

fn scan_start_menu_dir_with<F>(
    root: &Path,
    out: &mut HashMap<AppId, InstalledApp>,
    resolve_shortcut: &mut F,
) where
    F: FnMut(&Path) -> Option<InstalledApp>,
{
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_start_menu_dir_with(&path, out, resolve_shortcut);
            continue;
        }
        if !is_start_menu_shortcut(&path) {
            continue;
        }
        let Some(app) = resolve_shortcut(&path) else {
            continue;
        };
        out.entry(app.id.clone()).or_insert(app);
    }
}

fn is_start_menu_shortcut(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("lnk"))
        .unwrap_or(false)
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
        let len = K32GetModuleFileNameExW(Some(handle), None, &mut buffer);
        let _ = CloseHandle(handle);
        if len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

fn installed_app_from_shortcut(
    path: &Path,
    resolver: Option<&ShortcutResolver>,
) -> Option<InstalledApp> {
    let target = match resolver {
        Some(resolver) => resolver.resolve_target(path),
        None => resolve_shortcut_target(path),
    }?;
    installed_app_from_shortcut_target(path, &target)
}

fn installed_app_from_shortcut_target(path: &Path, target: &Path) -> Option<InstalledApp> {
    if !target.exists() || !target.is_file() {
        return None;
    }
    let target_str = target.to_string_lossy().to_string();
    if !target_str.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    let id = normalize_app_id(&target_str);
    let name = path
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            target
                .file_stem()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| !name.trim().is_empty())
        })
        .unwrap_or_else(|| id.clone());
    Some(InstalledApp {
        id,
        name,
        pid: None,
        running: false,
        recent_use_count: None,
        recent_last_used_at: None,
        recent_source: None,
    })
}

fn resolve_shortcut_target(path: &Path) -> Option<PathBuf> {
    let resolver = ShortcutResolver::new().ok()?;
    resolver.resolve_target(path)
}

fn normalize_app_id(path: &str) -> AppId {
    path.trim().to_ascii_lowercase()
}

fn app_matches_ref(app_id: &str, window: &WindowInfo) -> bool {
    let wanted = normalize_app_id(app_id);
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
    let normalized = normalize_app_id(&target.app_id);
    if resolve_process(&normalized).is_some() {
        return Ok(AppLaunchResult {
            target: AppTarget {
                app_id: normalized,
                window_id: target.window_id.clone(),
            },
            launched: false,
            running: true,
        });
    }
    let path = PathBuf::from(&target.app_id);
    if !path.is_absolute()
        || !path.exists()
        || !path.is_file()
        || !target.app_id.to_ascii_lowercase().ends_with(".exe")
    {
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
    let running = wait_for_running(&normalized, Duration::from_secs(5));
    if !running {
        return Err(ProviderError::Failed(format!(
            "launched {} but no targetable window appeared",
            target.app_id
        )));
    }
    Ok(AppLaunchResult {
        target: AppTarget {
            app_id: normalized,
            window_id: target.window_id.clone(),
        },
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
        click_marker: None,
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
        .map(|value| HWND(value as *mut core::ffi::c_void))
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
    let amount = amount.clamp(1, 20);
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

struct ShortcutResolver {
    shortcut: IShellLinkW,
    persist: IPersistFile,
    // Drop COM interface fields before tearing down the apartment.
    _apartment: ComApartment,
}

impl ShortcutResolver {
    fn new() -> ProviderResult<Self> {
        let apartment = ComApartment::init()?;
        let shortcut: IShellLinkW =
            unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.map_err(
                |e| ProviderError::Failed(format!("create ShellLink COM instance: {e}")),
            )?;
        let persist: IPersistFile = shortcut
            .cast()
            .map_err(|e| ProviderError::Failed(format!("cast ShellLink to IPersistFile: {e}")))?;
        Ok(Self {
            shortcut,
            persist,
            _apartment: apartment,
        })
    }

    fn resolve_target(&self, path: &Path) -> Option<PathBuf> {
        let wide = wide_null(path.to_string_lossy().as_ref());
        unsafe {
            self.persist.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        }

        let mut raw_path = vec![0u16; MAX_PATH as usize];
        unsafe {
            self.shortcut
                .GetPath(&mut raw_path, std::ptr::null_mut(), 0)
                .ok()?;
        }
        let len = raw_path
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(raw_path.len());
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&raw_path[..len])))
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

fn center_point(rect: Rect) -> Point {
    Point {
        x: rect.x + (rect.width / 2.0),
        y: rect.y + (rect.height / 2.0),
    }
}

fn click_route_plan(route_hint: ClickDispatchRoute) -> Vec<ClickDispatchRoute> {
    match route_hint {
        ClickDispatchRoute::Auto => vec![
            ClickDispatchRoute::Ax,
            ClickDispatchRoute::TargetPid,
            ClickDispatchRoute::Hid,
        ],
        ClickDispatchRoute::Ax => vec![ClickDispatchRoute::Ax],
        ClickDispatchRoute::TargetPid => vec![ClickDispatchRoute::TargetPid],
        ClickDispatchRoute::Hid => vec![ClickDispatchRoute::Hid],
    }
}

fn element_action_route_plan(
    route_hint: ClickDispatchRoute,
    has_native_fallback: bool,
) -> Vec<ClickDispatchRoute> {
    match route_hint {
        ClickDispatchRoute::Auto if has_native_fallback => {
            // Windows only advertises `Auto` and `Ax` for element actions.
            // Any concrete native route stays an internal detail behind `Auto`.
            vec![ClickDispatchRoute::TargetPid, ClickDispatchRoute::Ax]
        }
        ClickDispatchRoute::Auto => vec![ClickDispatchRoute::Ax],
        ClickDispatchRoute::Ax => vec![ClickDispatchRoute::Ax],
        ClickDispatchRoute::TargetPid => vec![ClickDispatchRoute::TargetPid],
        ClickDispatchRoute::Hid => vec![ClickDispatchRoute::Hid],
    }
}

fn click_execution_route_for_dispatch_route(route: ClickDispatchRoute) -> ClickExecutionRoute {
    match route {
        ClickDispatchRoute::Ax => ClickExecutionRoute::Uia,
        ClickDispatchRoute::Auto | ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid => {
            ClickExecutionRoute::Native
        }
    }
}

#[cfg(test)]
fn click_result_for_probe_outcome(
    route: ClickDispatchRoute,
    outcome: EffectProbeOutcome,
) -> ClickExecutionResult {
    ClickExecutionResult {
        route: click_execution_route_for_dispatch_route(route),
        outcome: click_outcome_for_probe_outcome(outcome),
        next_dispatch_route: None,
    }
}

fn click_outcome_for_probe_outcome(outcome: EffectProbeOutcome) -> ClickExecutionOutcome {
    match outcome {
        EffectProbeOutcome::ObservedEffect => ClickExecutionOutcome::ObservedEffect,
        EffectProbeOutcome::NoEffect => ClickExecutionOutcome::NoEffect,
        EffectProbeOutcome::Uncertain => ClickExecutionOutcome::Uncertain,
    }
}

fn action_result_for_probe_outcome(
    kind: ActionExecutionKind,
    route: ActionExecutionRoute,
    outcome: EffectProbeOutcome,
) -> ActionExecutionResult {
    ActionExecutionResult {
        kind,
        route,
        outcome: match outcome {
            EffectProbeOutcome::ObservedEffect => ActionExecutionOutcome::Dispatched,
            EffectProbeOutcome::NoEffect => ActionExecutionOutcome::NoEffect,
            EffectProbeOutcome::Uncertain => ActionExecutionOutcome::Uncertain,
        },
        next_dispatch_route: None,
    }
}

fn perform_mouse_click_for_route(route: ClickDispatchRoute, point: Point) -> ProviderResult<()> {
    match route {
        ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid | ClickDispatchRoute::Auto => {
            left_click_at(point)
        }
        ClickDispatchRoute::Ax => Err(ProviderError::Unsupported("click_element")),
    }
}

fn perform_secondary_click_for_route(
    route: ClickDispatchRoute,
    point: Point,
) -> ProviderResult<()> {
    match route {
        ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid | ClickDispatchRoute::Auto => {
            right_click_at(point)
        }
        ClickDispatchRoute::Ax => Err(ProviderError::Unsupported("secondary_click_element")),
    }
}

fn perform_scroll_for_route(
    route: ClickDispatchRoute,
    point: Point,
    direction: ScrollDirection,
    amount: i32,
) -> ProviderResult<()> {
    match route {
        ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid | ClickDispatchRoute::Auto => {
            let (x, y) = absolute_mouse_point(point);
            send_inputs(&[mouse_input(
                x,
                y,
                0,
                MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            )])?;
            scroll_wheel(direction, amount)
        }
        ClickDispatchRoute::Ax => Err(ProviderError::Unsupported("scroll_element")),
    }
}

fn click_uia_element(
    capture_dir: &Path,
    entry: &UiaElementEntry,
    route_hint: ClickDispatchRoute,
) -> ProviderResult<ClickExecutionResult> {
    let fallback_point = entry.ui.bounds.map(center_point);
    for route in click_route_plan(route_hint) {
        match route {
            ClickDispatchRoute::Ax => match invoke_uia_element(&entry.element) {
                Ok(()) => {
                    return Ok(ClickExecutionResult {
                        route: ClickExecutionRoute::Uia,
                        outcome: ClickExecutionOutcome::SemanticSuccess,
                        next_dispatch_route: None,
                    });
                }
                Err(ProviderError::Unsupported(_)) if route_hint == ClickDispatchRoute::Auto => {}
                Err(error) => return Err(error),
            },
            ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid => {
                let point = fallback_point
                    .ok_or_else(|| ProviderError::ElementNotFound(entry.ui.id.clone()))?;
                return perform_click_with_effect_probe(
                    capture_dir,
                    Some(point),
                    click_execution_route_for_dispatch_route(route),
                    || perform_mouse_click_for_route(route, point),
                );
            }
            ClickDispatchRoute::Auto => {}
        }
    }
    Err(ProviderError::Unsupported("click_element"))
}

fn secondary_click_uia_element(
    capture_dir: &Path,
    entry: &UiaElementEntry,
    route_hint: ClickDispatchRoute,
) -> ProviderResult<ActionExecutionResult> {
    let fallback_point = entry.ui.bounds.map(center_point);
    let mut last_error = None;
    for route in element_action_route_plan(route_hint, fallback_point.is_some()) {
        match route {
            ClickDispatchRoute::Ax => {
                match perform_action_with_effect_probe(capture_dir, fallback_point, || {
                    show_uia_element_menu(&entry.element)
                }) {
                    Ok(outcome) => {
                        return Ok(action_result_for_probe_outcome(
                            ActionExecutionKind::SecondaryClick,
                            ActionExecutionRoute::Uia,
                            outcome,
                        ));
                    }
                    Err(error) if route_hint == ClickDispatchRoute::Auto => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid => {
                let point = fallback_point
                    .ok_or_else(|| ProviderError::ElementNotFound(entry.ui.id.clone()))?;
                match perform_action_with_effect_probe_result(
                    ActionExecutionKind::SecondaryClick,
                    capture_dir,
                    Some(point),
                    ActionExecutionRoute::Native,
                    || perform_secondary_click_for_route(route, point),
                ) {
                    Ok(result) => return Ok(result),
                    Err(error) if route_hint == ClickDispatchRoute::Auto => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            ClickDispatchRoute::Auto => {}
        }
    }
    if let Some(error) = last_error {
        return Err(error);
    }
    Err(ProviderError::Unsupported("secondary_click_element"))
}

fn scroll_uia_element(
    capture_dir: &Path,
    entry: &UiaElementEntry,
    direction: ScrollDirection,
    amount: i32,
    route_hint: ClickDispatchRoute,
) -> ProviderResult<ActionExecutionResult> {
    let fallback_point = entry.ui.bounds.map(center_point);
    let mut last_error = None;
    for route in element_action_route_plan(route_hint, fallback_point.is_some()) {
        match route {
            ClickDispatchRoute::Ax => {
                match perform_action_with_effect_probe(capture_dir, fallback_point, || {
                    scroll_uia_element_by_direction(&entry.element, direction, amount)
                }) {
                    Ok(outcome) => {
                        return Ok(action_result_for_probe_outcome(
                            ActionExecutionKind::Scroll,
                            ActionExecutionRoute::Uia,
                            outcome,
                        ));
                    }
                    Err(error) if route_hint == ClickDispatchRoute::Auto => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            ClickDispatchRoute::TargetPid | ClickDispatchRoute::Hid => {
                let point = fallback_point
                    .ok_or_else(|| ProviderError::ElementNotFound(entry.ui.id.clone()))?;
                match perform_action_with_effect_probe_result(
                    ActionExecutionKind::Scroll,
                    capture_dir,
                    Some(point),
                    ActionExecutionRoute::Native,
                    || perform_scroll_for_route(route, point, direction, amount),
                ) {
                    Ok(result) => return Ok(result),
                    Err(error) if route_hint == ClickDispatchRoute::Auto => {
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
            ClickDispatchRoute::Auto => {}
        }
    }
    if let Some(error) = last_error {
        return Err(error);
    }
    Err(ProviderError::Unsupported("scroll_element"))
}

fn focus_uia_element(element: &IUIAutomationElement) -> ProviderResult<()> {
    unsafe {
        element
            .SetFocus()
            .map_err(|e| ProviderError::Failed(format!("UIA SetFocus failed: {e}")))
    }
}

fn show_uia_element_menu(element: &IUIAutomationElement) -> ProviderResult<()> {
    focus_uia_element(element)?;
    press_key_chord("shift+f10")
}

fn scroll_uia_element_by_direction(
    element: &IUIAutomationElement,
    direction: ScrollDirection,
    amount: i32,
) -> ProviderResult<()> {
    if try_scroll_uia_element_with_pattern(element, direction, amount)? {
        return Ok(());
    }

    match scroll_uia_element_into_view(element) {
        Ok(()) | Err(ProviderError::Unsupported(_)) => {}
        Err(error) => return Err(error),
    }
    focus_uia_element(element)?;
    scroll_uia_element_with_keys(direction, amount)
}

fn perform_action_with_effect_probe(
    capture_dir: &Path,
    probe_point: Option<Point>,
    perform: impl FnOnce() -> ProviderResult<()>,
) -> ProviderResult<EffectProbeOutcome> {
    let before = capture_screen_fingerprint(capture_dir, probe_point).ok();
    perform()?;
    probe_effect_after_capture(capture_dir, before, probe_point)
}

fn perform_click_with_effect_probe(
    capture_dir: &Path,
    probe_point: Option<Point>,
    route: ClickExecutionRoute,
    perform: impl FnOnce() -> ProviderResult<()>,
) -> ProviderResult<ClickExecutionResult> {
    let before = capture_screen_fingerprint(capture_dir, probe_point).ok();
    perform()?;
    let outcome = probe_effect_after_capture(capture_dir, before, probe_point)?;
    Ok(ClickExecutionResult {
        route,
        outcome: click_outcome_for_probe_outcome(outcome),
        next_dispatch_route: None,
    })
}

fn perform_action_with_effect_probe_result(
    kind: ActionExecutionKind,
    capture_dir: &Path,
    probe_point: Option<Point>,
    route: ActionExecutionRoute,
    perform: impl FnOnce() -> ProviderResult<()>,
) -> ProviderResult<ActionExecutionResult> {
    let before = capture_screen_fingerprint(capture_dir, probe_point).ok();
    perform()?;
    let outcome = probe_effect_after_capture(capture_dir, before, probe_point)?;
    Ok(action_result_for_probe_outcome(kind, route, outcome))
}

fn probe_effect_after_capture(
    capture_dir: &Path,
    before: Option<ScreenCaptureFingerprint>,
    probe_point: Option<Point>,
) -> ProviderResult<EffectProbeOutcome> {
    let Some(before) = before else {
        return Ok(EffectProbeOutcome::Uncertain);
    };
    let timing = EffectProbeTiming::DEFAULT;
    let attempts_total = timing.attempt_count();

    if timing.initial_delay_ms > 0 {
        thread::sleep(Duration::from_millis(timing.initial_delay_ms));
    }

    let mut saw_remote_only_change = false;
    for attempt_index in 0..attempts_total {
        if attempt_index > 0 && timing.poll_interval_ms > 0 {
            thread::sleep(Duration::from_millis(timing.poll_interval_ms));
        }

        let after = match capture_screen_fingerprint(capture_dir, probe_point) {
            Ok(after) => after,
            Err(_) => return Ok(EffectProbeOutcome::Uncertain),
        };

        let outcome = classify_effect_sample(before, after);
        if matches!(outcome, EffectProbeOutcome::Uncertain) {
            saw_remote_only_change = true;
        }
        if matches!(outcome, EffectProbeOutcome::ObservedEffect) {
            return Ok(outcome);
        }
    }

    if saw_remote_only_change {
        Ok(EffectProbeOutcome::Uncertain)
    } else {
        Ok(EffectProbeOutcome::NoEffect)
    }
}

fn classify_effect_sample(
    before: ScreenCaptureFingerprint,
    after: ScreenCaptureFingerprint,
) -> EffectProbeOutcome {
    let local_changed = before.local_hash.zip(after.local_hash).map(|(a, b)| a != b);
    let full_changed = before.full_hash != after.full_hash;
    if local_changed == Some(true) {
        EffectProbeOutcome::ObservedEffect
    } else if full_changed {
        EffectProbeOutcome::Uncertain
    } else {
        EffectProbeOutcome::NoEffect
    }
}

fn capture_screen_fingerprint(
    _capture_dir: &Path,
    probe_point: Option<Point>,
) -> ProviderResult<ScreenCaptureFingerprint> {
    let screen_rect = virtual_screen_rect();
    let saved = crate::screenshot::windows::capture_screen_rect_png(
        screen_rect,
        None,
        "computer-use-effect-probe",
    )
    .map_err(ProviderError::Failed)?;
    let path = PathBuf::from(saved.path);
    let rgba = ImageReader::open(&path)
        .map_err(|e| ProviderError::Failed(format!("open capture for effect probe: {e}")))?
        .with_guessed_format()
        .map_err(|e| ProviderError::Failed(format!("guess capture format for effect probe: {e}")))?
        .decode()
        .map_err(|e| ProviderError::Failed(format!("decode capture for effect probe: {e}")))?
        .to_rgba8();

    let _ = std::fs::remove_file(&path);

    let full_hash = hash_rgba_image(&rgba);
    let screen_bounds = Rect {
        x: screen_rect.x as f32,
        y: screen_rect.y as f32,
        width: screen_rect.width as f32,
        height: screen_rect.height as f32,
    };
    let local_hash = probe_point.and_then(|point| {
        local_probe_rect(rgba.width(), rgba.height(), screen_bounds, point).map(
            |(x, y, width, height)| {
                let cropped = crop_imm(&rgba, x, y, width, height).to_image();
                hash_rgba_image(&cropped)
            },
        )
    });

    Ok(ScreenCaptureFingerprint {
        full_hash,
        local_hash,
    })
}

fn hash_rgba_image(image: &image::RgbaImage) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(image.width().to_le_bytes());
    hasher.update(image.height().to_le_bytes());
    hasher.update(image.as_raw());
    hasher.finalize().into()
}

fn local_probe_rect(
    image_width: u32,
    image_height: u32,
    screen_bounds: Rect,
    probe_point: Point,
) -> Option<(u32, u32, u32, u32)> {
    if image_width == 0
        || image_height == 0
        || screen_bounds.width <= 0.0
        || screen_bounds.height <= 0.0
    {
        return None;
    }

    let relative_x = ((probe_point.x - screen_bounds.x) / screen_bounds.width).clamp(0.0, 1.0);
    let relative_y = ((probe_point.y - screen_bounds.y) / screen_bounds.height).clamp(0.0, 1.0);
    let center_x = (relative_x * image_width as f32).round() as i32;
    let center_y = (relative_y * image_height as f32).round() as i32;
    let radius = EFFECT_LOCAL_PROBE_RADIUS_PX as i32;

    let left = (center_x - radius).clamp(0, image_width as i32);
    let top = (center_y - radius).clamp(0, image_height as i32);
    let right = (center_x + radius).clamp(0, image_width as i32);
    let bottom = (center_y + radius).clamp(0, image_height as i32);

    let width = u32::try_from(right.saturating_sub(left)).ok()?;
    let height = u32::try_from(bottom.saturating_sub(top)).ok()?;
    if width == 0 || height == 0 {
        return None;
    }

    Some((left as u32, top as u32, width, height))
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
                next_dispatch_route: None,
            }),
            Ok(_) => Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SetValue,
                route: ActionExecutionRoute::Uia,
                outcome: ActionExecutionOutcome::NoEffect,
                next_dispatch_route: None,
            }),
            Err(_) => Ok(ActionExecutionResult {
                kind: ActionExecutionKind::SetValue,
                route: ActionExecutionRoute::Uia,
                outcome: ActionExecutionOutcome::Uncertain,
                next_dispatch_route: None,
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

fn try_scroll_uia_element_with_pattern(
    element: &IUIAutomationElement,
    direction: ScrollDirection,
    amount: i32,
) -> ProviderResult<bool> {
    unsafe {
        let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId)
        else {
            return Ok(false);
        };

        let (is_scrollable, horizontal_amount, vertical_amount) = match direction {
            ScrollDirection::Up => (
                pattern
                    .CurrentVerticallyScrollable()
                    .map_err(|e| {
                        ProviderError::Failed(format!(
                            "UIA ScrollPattern vertical scrollability check failed: {e}"
                        ))
                    })?
                    .as_bool(),
                ScrollAmount_NoAmount,
                scroll_amount_for_direction(true),
            ),
            ScrollDirection::Down => (
                pattern
                    .CurrentVerticallyScrollable()
                    .map_err(|e| {
                        ProviderError::Failed(format!(
                            "UIA ScrollPattern vertical scrollability check failed: {e}"
                        ))
                    })?
                    .as_bool(),
                ScrollAmount_NoAmount,
                scroll_amount_for_direction(false),
            ),
            ScrollDirection::Left => (
                pattern
                    .CurrentHorizontallyScrollable()
                    .map_err(|e| {
                        ProviderError::Failed(format!(
                            "UIA ScrollPattern horizontal scrollability check failed: {e}"
                        ))
                    })?
                    .as_bool(),
                scroll_amount_for_direction(true),
                ScrollAmount_NoAmount,
            ),
            ScrollDirection::Right => (
                pattern
                    .CurrentHorizontallyScrollable()
                    .map_err(|e| {
                        ProviderError::Failed(format!(
                            "UIA ScrollPattern horizontal scrollability check failed: {e}"
                        ))
                    })?
                    .as_bool(),
                scroll_amount_for_direction(false),
                ScrollAmount_NoAmount,
            ),
        };

        if !is_scrollable {
            return Ok(false);
        }

        for _ in 0..amount.clamp(1, 20) {
            pattern
                .Scroll(horizontal_amount, vertical_amount)
                .map_err(|e| ProviderError::Failed(format!("UIA ScrollPattern failed: {e}")))?;
        }
        Ok(true)
    }
}

fn scroll_amount_for_direction(decrement: bool) -> ScrollAmount {
    match decrement {
        true => ScrollAmount_SmallDecrement,
        false => ScrollAmount_SmallIncrement,
    }
}

fn scroll_uia_element_with_keys(direction: ScrollDirection, amount: i32) -> ProviderResult<()> {
    let key = match direction {
        ScrollDirection::Up => "up",
        ScrollDirection::Down => "down",
        ScrollDirection::Left => "left",
        ScrollDirection::Right => "right",
    };
    for _ in 0..amount.clamp(1, 20) {
        press_key_chord(key)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed_app(id: &str, name: &str) -> InstalledApp {
        InstalledApp {
            id: id.into(),
            name: name.into(),
            pid: None,
            running: false,
            recent_use_count: None,
            recent_last_used_at: None,
            recent_source: None,
        }
    }

    fn running_app(id: &str, name: &str, pid: i32) -> InstalledApp {
        InstalledApp {
            id: id.into(),
            name: name.into(),
            pid: Some(pid),
            running: true,
            recent_use_count: None,
            recent_last_used_at: None,
            recent_source: None,
        }
    }

    #[test]
    fn merge_available_apps_dedupes_by_id_and_sorts_running_first_then_name() {
        let apps = merge_available_apps(
            vec![
                installed_app(r"c:\apps\zeta.exe", "Zeta"),
                installed_app(r"c:\apps\alpha.exe", "Alpha"),
            ],
            vec![
                running_app(r"c:\apps\alpha.exe", "Alpha Window", 42),
                running_app(r"c:\apps\beta.exe", "Beta", 7),
            ],
        );

        assert_eq!(apps.len(), 3);
        assert_eq!(apps[0].name, "Alpha");
        assert!(apps[0].running);
        assert_eq!(apps[0].pid, Some(42));
        assert_eq!(apps[1].name, "Beta");
        assert!(apps[1].running);
        assert_eq!(apps[2].name, "Zeta");
        assert!(!apps[2].running);
    }

    #[test]
    fn installed_app_from_shortcut_target_skips_missing_or_non_exe_targets() {
        let root = std::env::temp_dir().join(format!(
            "sessio-cu-windows-shortcut-target-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();

        let shortcut = root.join("Example App.lnk");
        std::fs::write(&shortcut, b"placeholder").unwrap();

        let missing = root.join("missing.exe");
        assert!(installed_app_from_shortcut_target(&shortcut, &missing).is_none());

        let text_file = root.join("readme.txt");
        std::fs::write(&text_file, b"hello").unwrap();
        assert!(installed_app_from_shortcut_target(&shortcut, &text_file).is_none());

        let exe_file = root.join("Example.EXE");
        std::fs::write(&exe_file, b"MZ").unwrap();
        let app = installed_app_from_shortcut_target(&shortcut, &exe_file).unwrap();
        assert_eq!(
            app.id,
            normalize_app_id(exe_file.to_string_lossy().as_ref())
        );
        assert_eq!(app.name, "Example App");
        assert!(!app.running);
        assert!(app.pid.is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_start_menu_dir_skips_bad_shortcuts_and_dedupes_targets() {
        let root = std::env::temp_dir().join(format!(
            "sessio-cu-windows-start-menu-{}",
            uuid::Uuid::new_v4()
        ));
        let nested = root.join("Nested");
        std::fs::create_dir_all(&nested).unwrap();

        let good = root.join("Good App.lnk");
        let bad = root.join("Bad App.lnk");
        let duplicate = nested.join("Alias App.lnk");
        let ignored = root.join("Readme.txt");
        for path in [&good, &bad, &duplicate] {
            std::fs::write(path, b"placeholder").unwrap();
        }
        std::fs::write(&ignored, b"skip").unwrap();

        let mut apps = HashMap::new();
        let mut resolver = |path: &Path| match path.file_name().and_then(|name| name.to_str()) {
            Some("Good App.lnk") => Some(installed_app(r"c:\apps\good.exe", "Good App")),
            Some("Alias App.lnk") => Some(installed_app(r"c:\apps\good.exe", "Alias App")),
            Some("Bad App.lnk") => None,
            _ => None,
        };

        scan_start_menu_dir_with(&root, &mut apps, &mut resolver);

        assert_eq!(apps.len(), 1);
        let app = apps.get(r"c:\apps\good.exe").unwrap();
        assert_eq!(app.name, "Good App");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn windows_provider_capabilities_expose_element_level_ax_routes() {
        let caps = WindowsProvider::new().capabilities();

        assert_eq!(
            caps.click_element_routes,
            vec![ClickDispatchRoute::Auto, ClickDispatchRoute::Ax]
        );
        assert_eq!(
            caps.secondary_click_element_routes,
            vec![ClickDispatchRoute::Auto, ClickDispatchRoute::Ax]
        );
        assert_eq!(
            caps.scroll_element_routes,
            vec![ClickDispatchRoute::Auto, ClickDispatchRoute::Ax]
        );
        assert_eq!(caps.click_at_routes, vec![ClickDispatchRoute::Auto]);
        assert_eq!(caps.scroll_at_routes, vec![ClickDispatchRoute::Auto]);
        assert!(caps.supports_set_value);
        assert!(caps.supports_type_text);
        assert!(caps.supports_press_key);
    }

    #[test]
    fn click_route_plan_prefers_ax_then_internal_mouse_fallbacks() {
        assert_eq!(
            click_route_plan(ClickDispatchRoute::Auto),
            vec![
                ClickDispatchRoute::Ax,
                ClickDispatchRoute::TargetPid,
                ClickDispatchRoute::Hid
            ]
        );
        assert_eq!(
            click_route_plan(ClickDispatchRoute::Ax),
            vec![ClickDispatchRoute::Ax]
        );
        assert_eq!(
            click_route_plan(ClickDispatchRoute::TargetPid),
            vec![ClickDispatchRoute::TargetPid]
        );
        assert_eq!(
            click_route_plan(ClickDispatchRoute::Hid),
            vec![ClickDispatchRoute::Hid]
        );
    }

    #[test]
    fn element_action_route_plan_prefers_native_fallback_before_ax_when_available() {
        assert_eq!(
            element_action_route_plan(ClickDispatchRoute::Auto, true),
            vec![ClickDispatchRoute::TargetPid, ClickDispatchRoute::Ax]
        );
        assert_eq!(
            element_action_route_plan(ClickDispatchRoute::Auto, false),
            vec![ClickDispatchRoute::Ax]
        );
        assert_eq!(
            element_action_route_plan(ClickDispatchRoute::Ax, true),
            vec![ClickDispatchRoute::Ax]
        );
    }

    #[test]
    fn windows_internal_mouse_fallbacks_report_native_routes() {
        let click = click_result_for_probe_outcome(
            ClickDispatchRoute::TargetPid,
            EffectProbeOutcome::Uncertain,
        );
        assert_eq!(click.route, ClickExecutionRoute::Native);
        assert_eq!(click.outcome, ClickExecutionOutcome::Uncertain);

        let action = action_result_for_probe_outcome(
            ActionExecutionKind::Scroll,
            ActionExecutionRoute::Native,
            EffectProbeOutcome::NoEffect,
        );
        assert_eq!(action.route, ActionExecutionRoute::Native);
        assert_eq!(action.outcome, ActionExecutionOutcome::NoEffect);
    }

    #[test]
    fn scroll_amount_for_direction_matches_expected_uia_increment() {
        assert_eq!(
            scroll_amount_for_direction(true),
            ScrollAmount_SmallDecrement
        );
        assert_eq!(
            scroll_amount_for_direction(false),
            ScrollAmount_SmallIncrement
        );
    }

    #[test]
    fn effect_sample_detects_local_change_as_observed_effect() {
        let before = ScreenCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };
        let after = ScreenCaptureFingerprint {
            full_hash: [3; 32],
            local_hash: Some([4; 32]),
        };

        assert_eq!(
            classify_effect_sample(before, after),
            EffectProbeOutcome::ObservedEffect
        );
    }

    #[test]
    fn effect_sample_treats_remote_only_change_as_uncertain() {
        let before = ScreenCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };
        let after = ScreenCaptureFingerprint {
            full_hash: [3; 32],
            local_hash: Some([2; 32]),
        };

        assert_eq!(
            classify_effect_sample(before, after),
            EffectProbeOutcome::Uncertain
        );
    }

    #[test]
    fn effect_sample_treats_no_change_as_no_effect() {
        let before = ScreenCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };
        let after = ScreenCaptureFingerprint {
            full_hash: [1; 32],
            local_hash: Some([2; 32]),
        };

        assert_eq!(
            classify_effect_sample(before, after),
            EffectProbeOutcome::NoEffect
        );
    }
}
