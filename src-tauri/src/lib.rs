pub mod models;
pub mod readers;

use std::path::PathBuf;

use models::{Agent, SessionInfo, SessionMessage};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
};

#[tauri::command]
fn list_sessions() -> Vec<SessionInfo> {
    let mut sessions = readers::list_all();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

#[tauri::command]
fn get_session_messages(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
) -> Result<Vec<SessionMessage>, String> {
    let path = PathBuf::from(&file_path);
    if file_path.is_empty() || !path.exists() {
        return Err(format!(
            "Session file no longer exists (likely cleaned by {}): {}",
            match agent {
                Agent::Codex => "Codex",
                Agent::Claude => "Claude Code",
                Agent::Gemini => "Gemini",
            },
            if file_path.is_empty() {
                "<empty>"
            } else {
                file_path.as_str()
            }
        ));
    }
    match agent {
        Agent::Codex => readers::codex::read_messages(&path).map_err(|e| e.to_string()),
        Agent::Claude => readers::claude::read_messages(&path).map_err(|e| e.to_string()),
        Agent::Gemini => {
            let sid = session_id.unwrap_or_default();
            readers::gemini::read_messages(&path, &sid).map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
fn set_window_appearance(window: tauri::Window, theme: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{class, msg_send, runtime::AnyObject};
        use objc2_foundation::NSString;

        let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())?;
        if ns_window_ptr.is_null() {
            return Err("ns_window is null".into());
        }
        let name = NSString::from_str(if theme == "dark" {
            "NSAppearanceNameDarkAqua"
        } else {
            "NSAppearanceNameAqua"
        });
        unsafe {
            let appearance: *mut AnyObject =
                msg_send![class!(NSAppearance), appearanceNamed: &*name];
            if appearance.is_null() {
                return Err(format!("unknown NSAppearance name for theme '{}'", theme));
            }
            let ns_window = ns_window_ptr as *mut AnyObject;
            let _: () = msg_send![ns_window, setAppearance: appearance];
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, theme);
    }
    Ok(())
}

#[tauri::command]
fn get_system_appearance() -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2::{class, msg_send, runtime::AnyObject};
        use objc2_foundation::NSString;

        // Once we override the window's NSAppearance, webview matchMedia stops
        // reflecting the system. Read AppleInterfaceStyle directly so the
        // frontend can resolve "system" mode accurately. The key is absent in
        // light mode and equals "Dark" in dark mode.
        unsafe {
            let defaults: *mut AnyObject =
                msg_send![class!(NSUserDefaults), standardUserDefaults];
            if defaults.is_null() {
                return "light".into();
            }
            let key = NSString::from_str("AppleInterfaceStyle");
            let value: *mut AnyObject = msg_send![defaults, stringForKey: &*key];
            if value.is_null() {
                "light".into()
            } else {
                "dark".into()
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "light".into()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let show = MenuItem::with_id(app, "show", "Show Sessio", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "Hide Sessio", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Sessio", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &sep, &quit])?;

            TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray-icon.png"))
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.hide();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if let Some(win) = app.get_webview_window("main") {
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            get_session_messages,
            set_window_appearance,
            get_system_appearance
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
