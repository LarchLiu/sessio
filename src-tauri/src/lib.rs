pub mod agents;
pub mod cli;
pub mod config;
pub mod indexer;
pub mod memory;
pub mod models;
pub mod polling;
pub mod store;
pub mod watch;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agents::runtime::types::{
    AgentInput, AgentSessionConfigChange, AgentSessionHandle, AgentTurnHandle,
    EnsureAgentRuntimeSession, RuntimeStatus, StartAgentSession,
};
use agents::runtime::RuntimeManager;
use indexer::{IndexTask, IndexerHandle};
use memory::qmd::{query_project, search_project, QmdOptions};
use memory::service::MemoryService;
use memory::{MemoryBackendStatus, MemoryStore};
use models::{Agent, SessionInfo, SessionMessage};
use store::cached::CachedStore;
use store::sqlite::SqliteStore;
use store::SessionStore;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, State, WindowEvent,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionMessagesResult {
    messages: Vec<SessionMessage>,
    message_count: usize,
}

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

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
enum SessionScope {
    All,
    Agent { agent: Agent },
    Project { key: String },
}

#[tauri::command]
fn remove_session_files(session: SessionInfo) -> Result<(), String> {
    remove_session_files_inner(session).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_sessions_by_scope(
    scope: SessionScope,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    let sessions = store.list_sessions().map_err(|e| e.to_string())?;
    for session in sessions
        .iter()
        .filter(|s| s.available && !is_subagent_only(s) && matches_scope(&scope, s))
    {
        remove_session_files_inner(session.clone()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn is_subagent_only(session: &SessionInfo) -> bool {
    session.archived && session.message_count == 0 && !session.subagents.is_empty()
}

fn matches_scope(scope: &SessionScope, session: &SessionInfo) -> bool {
    match scope {
        SessionScope::All => true,
        SessionScope::Agent { agent } => session.agent == *agent,
        SessionScope::Project { key } => {
            let session_key = session
                .project_path
                .clone()
                .unwrap_or_else(|| format!("__unknown__:{}", session.agent.as_str()));
            session_key == *key
        }
    }
}

fn remove_session_files_inner(session: SessionInfo) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    let removed_root = home.join(".sessio").join("removed-sessions");

    if session.agent == Agent::Gemini {
        if crate::agents::sources::gemini::parser::remove_session_from_logs(
            Path::new(&session.file_path),
            &session.id,
            &home,
            &removed_root,
        )? {
            for subagent in &session.subagents {
                let _ = crate::agents::sources::gemini::parser::remove_session_from_logs(
                    Path::new(&subagent.file_path),
                    &subagent.id,
                    &home,
                    &removed_root,
                )?;
            }
        }
        return Ok(());
    }

    move_session_file(&session.file_path, &home, &removed_root)?;

    for subagent in &session.subagents {
        move_session_file(&subagent.file_path, &home, &removed_root)?;
        move_claude_subagent_meta_file(&subagent.file_path, &home, &removed_root)?;
    }

    Ok(())
}

fn move_claude_subagent_meta_file(
    file_path: &str,
    home: &Path,
    removed_root: &Path,
) -> anyhow::Result<bool> {
    if file_path.is_empty() {
        return Ok(false);
    }
    let meta_path = PathBuf::from(file_path).with_extension("meta.json");
    move_session_file(&meta_path.to_string_lossy(), home, removed_root)
}

fn move_session_file(file_path: &str, home: &Path, removed_root: &Path) -> anyhow::Result<bool> {
    if file_path.is_empty() {
        return Ok(false);
    }
    let src = PathBuf::from(file_path);
    if !src.exists() {
        return Ok(false);
    }
    if !src.is_file() {
        anyhow::bail!("session path is not a file: {}", src.display());
    }

    let relative = src
        .strip_prefix(home)
        .map_err(|_| anyhow::anyhow!("session file is outside home: {}", src.display()))?;
    let dst = removed_root.join(relative);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dst = available_removed_path(dst);
    move_file(&src, &dst)?;
    Ok(true)
}

fn available_removed_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".to_string());
    for i in 1.. {
        let candidate = parent.join(format!("{file_name}.{i}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn move_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            std::fs::copy(src, dst).map_err(|copy_err| {
                anyhow::anyhow!(
                    "move {} to {} failed: rename: {}; copy fallback: {}",
                    src.display(),
                    dst.display(),
                    rename_err,
                    copy_err
                )
            })?;
            std::fs::remove_file(src).map_err(|remove_err| {
                let _ = std::fs::remove_file(dst);
                anyhow::anyhow!(
                    "remove original after copying {} to {} failed: {}",
                    src.display(),
                    dst.display(),
                    remove_err
                )
            })?;
            Ok(())
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexStatus {
    phase: indexer::IndexPhase,
    last_error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMemorySearchResult {
    title: Option<String>,
    snippet: Option<String>,
    score: Option<f64>,
    record_id: Option<String>,
    artifact_uri: Option<String>,
    raw: serde_json::Value,
}

#[tauri::command]
fn get_index_status(indexer: State<'_, IndexerHandle>) -> IndexStatus {
    let s = indexer.status();
    IndexStatus {
        phase: s.phase,
        last_error: s.last_error,
    }
}

#[tauri::command]
fn get_memory_backend_status(
    store: State<'_, Arc<dyn MemoryStore>>,
) -> Result<MemoryBackendStatus, String> {
    let service = MemoryService::new(
        store.inner().clone(),
        Arc::new(crate::agents::sources::builtin_agent_sources()),
    )
    .map_err(|e| e.to_string())?;
    Ok(service.backend_status())
}

#[tauri::command]
fn search_project_memory(
    store: State<'_, Arc<dyn MemoryStore>>,
    project_key: String,
    query: String,
) -> Result<Vec<ProjectMemorySearchResult>, String> {
    search_project_memory_inner(project_key, query, Some(store.inner().as_ref()))
        .map_err(|e| e.to_string())
}

fn search_project_memory_inner(
    project_key: String,
    query: String,
    store: Option<&dyn MemoryStore>,
) -> anyhow::Result<Vec<ProjectMemorySearchResult>> {
    let config = config::load_memory_config()?;
    let memory_project_key = resolve_memory_project_key(&project_key, store)?;
    let options = QmdOptions {
        binary: config.qmd.binary.clone(),
        index: config.qmd.index.clone(),
        install_command: config.qmd.install_command.clone(),
    };
    let result = if config.qmd.auto_embed {
        query_project(&options, &memory_project_key, &query)
    } else {
        search_project(&options, &memory_project_key, &query)
    }?;
    Ok(project_memory_results(&result.raw))
}

fn resolve_memory_project_key(
    project_filter_key: &str,
    store: Option<&dyn MemoryStore>,
) -> anyhow::Result<String> {
    if let Some(store) = store {
        if !store.list_project_records(project_filter_key)?.is_empty() {
            return Ok(project_filter_key.to_string());
        }
    }
    let slug = crate::agents::sources::shared::convert::project_key_for_path_or_name(
        Some(project_filter_key),
        None,
    );
    if let Some(store) = store {
        if !store.list_project_records(&slug)?.is_empty() {
            return Ok(slug);
        }
    }
    Ok(slug)
}

fn project_memory_results(raw: &serde_json::Value) -> Vec<ProjectMemorySearchResult> {
    let mut out = Vec::new();
    collect_project_memory_results(raw, &mut out);
    out
}

fn collect_project_memory_results(
    raw: &serde_json::Value,
    out: &mut Vec<ProjectMemorySearchResult>,
) {
    match raw {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_project_memory_results(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            let title = first_json_string(map, &["title", "name", "heading"]);
            let snippet = first_json_string(map, &["snippet", "text", "content", "preview"]);
            let artifact_uri =
                first_json_string(map, &["path", "file", "filePath", "filepath", "source"]);
            let record_id = first_json_string(map, &["recordId", "record_id", "id"])
                .and_then(record_id_from_text)
                .or_else(|| artifact_uri.clone().and_then(record_id_from_text));
            if title.is_some() || snippet.is_some() || artifact_uri.is_some() || record_id.is_some()
            {
                out.push(ProjectMemorySearchResult {
                    title,
                    snippet,
                    score: first_json_number(map, &["score", "rank", "similarity"]),
                    record_id,
                    artifact_uri,
                    raw: raw.clone(),
                });
            }
            for key in ["results", "hits", "documents", "items", "matches"] {
                if let Some(child) = map.get(key) {
                    collect_project_memory_results(child, out);
                }
            }
        }
        _ => {}
    }
}

fn first_json_string(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(|value| value.as_str())
            .map(str::to_string)
    })
}

fn first_json_number(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<f64> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(|value| value.as_f64()))
}

fn record_id_from_text(text: String) -> Option<String> {
    let path = Path::new(&text);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&text);
    stem.starts_with("sessio-").then(|| stem.to_string())
}

#[tauri::command]
fn get_session_messages(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
) -> Result<SessionMessagesResult, String> {
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
    let (messages, message_count) = match agent {
        Agent::Codex => {
            let rows = crate::agents::sources::codex::parser::read_messages_with_locations(&path)
                .map_err(|e| e.to_string())?;
            let count = count_source_lines(&rows);
            let messages = rows.into_iter().map(|(m, _)| m).collect();
            (messages, count)
        }
        Agent::Claude => {
            let rows = crate::agents::sources::claude::parser::read_messages_with_locations(&path)
                .map_err(|e| e.to_string())?;
            let count = count_source_lines(&rows);
            let messages = rows.into_iter().map(|(m, _)| m).collect();
            (messages, count)
        }
        Agent::Gemini => {
            let sid = session_id.clone().unwrap_or_default();
            let messages = crate::agents::sources::gemini::parser::read_messages(&path, &sid)
                .map_err(|e| e.to_string())?;
            let count = messages.len();
            (messages, count)
        }
    };
    Ok(SessionMessagesResult {
        messages,
        message_count,
    })
}

#[tauri::command]
fn update_session_message_count(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
    message_count: usize,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .update_message_count(agent, session_id.as_deref(), &file_path, message_count)
        .map_err(|e| e.to_string())
}

fn count_source_lines(
    rows: &[(
        SessionMessage,
        crate::agents::sources::types::SourceLocation,
    )],
) -> usize {
    let mut lines = HashSet::new();
    for (_, location) in rows {
        if let Some(line) = location.line_start {
            lines.insert(line);
        }
    }
    lines.len()
}

#[tauri::command]
fn read_local_image_data_url(path: String) -> Result<String, String> {
    use base64::Engine;

    let path_buf = PathBuf::from(&path);
    if !path_buf.is_absolute() {
        return Err("Only absolute image paths can be loaded".to_string());
    }
    let mime = local_image_mime(&path_buf).ok_or_else(|| "Unsupported image type".to_string())?;
    let meta = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
    const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err("Image is too large to preview".to_string());
    }
    let bytes = std::fs::read(&path_buf).map_err(|e| e.to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn local_image_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("svg") => Some("image/svg+xml"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

#[tauri::command]
fn write_cross_prompt(session_id: String, content: String) -> Result<String, String> {
    let safe_id: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = std::env::temp_dir().join(format!("sessio-cross-{}.txt", safe_id));
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn get_agent_runtime_status(
    agent: Agent,
    runtime: State<'_, RuntimeManager>,
) -> Result<RuntimeStatus, String> {
    Ok(runtime.status(agent))
}

#[tauri::command]
fn start_agent_session(
    req: StartAgentSession,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn fork_agent_session(
    req: StartAgentSession,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    if req.source_session_id.as_deref().unwrap_or("").trim().is_empty() {
        return Err("source_session_id is required".to_string());
    }
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn ensure_agent_runtime_session(
    req: EnsureAgentRuntimeSession,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    runtime.ensure_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_agent_session(
    agent: Agent,
    runtime_session_id: String,
    workspace_path: String,
    agent_runtime_session_id: Option<String>,
    source_agent: Option<Agent>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    runtime
        .ensure_session(EnsureAgentRuntimeSession {
            agent,
            sessio_runtime_session_id: runtime_session_id,
            workspace_path,
            agent_runtime_session_id,
            source_agent,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn send_agent_input(
    sessio_runtime_session_id: String,
    input: AgentInput,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentTurnHandle, String> {
    log::info!(
        "[sessio-runtime:backend:send] session={} text={:?}",
        sessio_runtime_session_id,
        input.text
    );
    runtime
        .send_input(&sessio_runtime_session_id, input)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_agent_turn(
    sessio_runtime_session_id: String,
    turn_id: String,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .cancel_turn(&sessio_runtime_session_id, &turn_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_agent_session_config_option(
    sessio_runtime_session_id: String,
    change: AgentSessionConfigChange,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .set_config_option(&sessio_runtime_session_id, change)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn respond_agent_permission(
    sessio_runtime_session_id: String,
    request_id: String,
    option_id: String,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .respond_permission(&sessio_runtime_session_id, &request_id, option_id)
        .map_err(|e| e.to_string())
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
            let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
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
        class, msg_send,
        runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
        sel,
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
        .plugin(tauri_plugin_opener::init())
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
            let sqlite = Arc::new(SqliteStore::open(&db_path)?);
            sqlite.init()?;
            let inner: Arc<dyn SessionStore> = sqlite.clone();
            let memory_store: Arc<dyn MemoryStore> = sqlite;
            let store: Arc<dyn SessionStore> = Arc::new(CachedStore::new(inner)?);
            let indexer_handle =
                indexer::spawn(app.handle().clone(), store.clone(), memory_store.clone());
            log::info!("indexer spawned");

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
            app.manage(memory_store);
            app.manage(indexer_handle);
            app.manage(RuntimeManager::new(app.handle().clone()));

            install_appearance_observer(app.handle().clone());

            let show = MenuItem::with_id(app, "show", "Show Sessio", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Sessio", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &sep, &quit])?;

            TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray-icon.png"))
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.unminimize();
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if let Some(win) = app.get_webview_window("main") {
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = win.set_decorations(false);
                }
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
            update_session_message_count,
            read_local_image_data_url,
            set_window_appearance,
            get_system_appearance,
            rebuild_session_index,
            get_index_status,
            get_memory_backend_status,
            search_project_memory,
            write_cross_prompt,
            get_agent_runtime_status,
            start_agent_session,
            fork_agent_session,
            ensure_agent_runtime_session,
            load_agent_session,
            send_agent_input,
            cancel_agent_turn,
            set_agent_session_config_option,
            respond_agent_permission,
            remove_session_files,
            remove_sessions_by_scope
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
