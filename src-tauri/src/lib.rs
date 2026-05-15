pub mod indexer;
pub mod models;
pub mod polling;
pub mod readers;
pub mod store;
pub mod watch;

use std::path::PathBuf;
use std::sync::Arc;

use indexer::{IndexTask, IndexerHandle};
use models::{Agent, SessionInfo, SessionMessage};
use store::sqlite::SqliteStore;
use store::SessionStore;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, State, WindowEvent,
};

#[tauri::command]
fn list_sessions(store: State<'_, Arc<dyn SessionStore>>) -> Result<Vec<SessionInfo>, String> {
    store.list_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
fn rebuild_session_index(indexer: State<'_, IndexerHandle>) -> Result<(), String> {
    indexer
        .submit(IndexTask::FullRebuild)
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexStatus {
    indexing: bool,
    last_error: Option<String>,
}

#[tauri::command]
fn get_index_status(indexer: State<'_, IndexerHandle>) -> IndexStatus {
    let s = indexer.status();
    IndexStatus {
        indexing: s.indexing,
        last_error: s.last_error,
    }
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
    current_system_appearance()
}

fn current_system_appearance() -> String {
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

// macOS won't fire prefers-color-scheme change events into the webview once
// we've pinned the NSWindow's appearance, so we hook into the system-wide
// AppleInterfaceThemeChangedNotification (posted by macOS whenever the
// effective appearance flips, including the automatic sunset schedule) and
// push the new value down to the frontend. Other platforms don't pin
// appearance, so matchMedia in the webview already works there.
#[cfg(target_os = "macos")]
mod appearance_observer {
    use std::sync::OnceLock;

    use objc2::{
        class, msg_send, sel,
        runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
    };
    use objc2_foundation::NSString;
    use tauri::{AppHandle, Emitter};

    static HANDLE: OnceLock<AppHandle> = OnceLock::new();
    static OBSERVER_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

    extern "C" fn theme_changed(_this: &AnyObject, _cmd: Sel, _notif: *mut AnyObject) {
        if let Some(handle) = HANDLE.get() {
            let value = super::current_system_appearance();
            let _ = handle.emit("system_appearance_changed", value);
        }
    }

    pub fn install(handle: AppHandle) {
        if HANDLE.set(handle).is_err() {
            return;
        }

        let cls = OBSERVER_CLASS.get_or_init(|| {
            let mut builder = ClassBuilder::new(c"SessioAppearanceObserver", class!(NSObject))
                .expect("SessioAppearanceObserver name already registered");
            unsafe {
                let imp: extern "C" fn(_, _, _) = theme_changed;
                builder.add_method(sel!(themeChanged:), imp);
            }
            builder.register()
        });

        unsafe {
            // `new` returns a +1 retained instance. We deliberately drop the
            // pointer without releasing so the observer lives for the lifetime
            // of the app (NSDistributedNotificationCenter holds it weakly).
            let observer: *mut AnyObject = msg_send![*cls, new];
            let center: *mut AnyObject =
                msg_send![class!(NSDistributedNotificationCenter), defaultCenter];
            let name = NSString::from_str("AppleInterfaceThemeChangedNotification");
            let _: () = msg_send![
                center,
                addObserver: observer,
                selector: sel!(themeChanged:),
                name: &*name,
                object: std::ptr::null::<AnyObject>(),
            ];
        }
    }
}

fn install_appearance_observer(handle: AppHandle) {
    #[cfg(target_os = "macos")]
    appearance_observer::install(handle);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = handle;
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

            let data_dir = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("no home dir"))?
                .join(".sessio")
                .join("db-data");
            std::fs::create_dir_all(&data_dir).ok();
            let db_path = data_dir.join("sessio-index.db");
            let sqlite = SqliteStore::open(&db_path)?;
            sqlite.init()?;
            let store: Arc<dyn SessionStore> = Arc::new(sqlite);
            let needs_full_rebuild = store
                .list_sessions()
                .map(|v| v.is_empty())
                .unwrap_or(true);
            let indexer_handle =
                indexer::spawn(app.handle().clone(), store.clone(), needs_full_rebuild);
            log::info!("indexer spawned, needs_full_rebuild={}", needs_full_rebuild);

            polling::spawn_polling(store.clone(), indexer_handle.clone());
            log::info!("polling thread spawned");

            match watch::spawn(indexer_handle.clone()) {
                Ok(handle) => {
                    log::info!("watcher spawned successfully");
                    // Keep watcher alive for the lifetime of the process.
                    Box::leak(Box::new(handle));
                }
                Err(e) => log::warn!("watcher failed to start: {e}"),
            }
            app.manage(store);
            app.manage(indexer_handle);

            install_appearance_observer(app.handle().clone());

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
            get_system_appearance,
            rebuild_session_index,
            get_index_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
