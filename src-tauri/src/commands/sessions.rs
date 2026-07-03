use std::sync::Arc;

use crate::models::{Agent, ChannelSessionInfo, SessionInfo};
use crate::store::SessionStore;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) fn list_sessions(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<SessionInfo>, String> {
    store.list_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn list_channel_sessions(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ChannelSessionInfo>, String> {
    store.list_channel_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn update_session_rename_title(
    agent: Agent,
    session_id: String,
    rename_title: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .update_session_rename_title(agent, &session_id, rename_title.as_deref())
        .map_err(|e| e.to_string())?;
    app.emit("sessions_index_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}
