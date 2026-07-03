use std::sync::Arc;

use crate::models::{Agent, ChannelSessionInfo, SessionInfo};
use crate::store::{capabilities::SessionCommandStore, SessionStore};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) fn list_sessions(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<SessionInfo>, String> {
    list_sessions_with_store(store.inner().as_ref())
}

#[tauri::command]
pub(crate) fn list_channel_sessions(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ChannelSessionInfo>, String> {
    list_channel_sessions_with_store(store.inner().as_ref())
}

#[tauri::command]
pub(crate) fn update_session_rename_title(
    agent: Agent,
    session_id: String,
    rename_title: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    update_session_rename_title_with_store(
        store.inner().as_ref(),
        agent,
        &session_id,
        rename_title.as_deref(),
    )?;
    app.emit("sessions_index_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_sessions_with_store<S: SessionCommandStore + ?Sized>(
    store: &S,
) -> Result<Vec<SessionInfo>, String> {
    store.list_sessions().map_err(|e| e.to_string())
}

fn list_channel_sessions_with_store<S: SessionCommandStore + ?Sized>(
    store: &S,
) -> Result<Vec<ChannelSessionInfo>, String> {
    store.list_channel_sessions().map_err(|e| e.to_string())
}

fn update_session_rename_title_with_store<S: SessionCommandStore + ?Sized>(
    store: &S,
    agent: Agent,
    session_id: &str,
    rename_title: Option<&str>,
) -> Result<(), String> {
    store
        .update_session_rename_title(agent, session_id, rename_title)
        .map_err(|e| e.to_string())
}
