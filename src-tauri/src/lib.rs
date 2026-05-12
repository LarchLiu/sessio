pub mod models;
pub mod readers;

use std::path::PathBuf;

use models::{Agent, SessionInfo, SessionMessage};

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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![list_sessions, get_session_messages])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
