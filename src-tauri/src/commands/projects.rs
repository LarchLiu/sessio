use std::sync::Arc;

use crate::default_process_template_id;
use crate::models::ProjectInfo;
use crate::store::SessionStore;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) fn list_projects(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProjectInfo>, String> {
    store.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn add_existing_project(
    path: String,
    name: Option<String>,
    process_template_id: Option<String>,
    enabled_stage_ids: Option<Vec<String>>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let project = store
        .add_project(
            &path,
            name.as_deref(),
            process_template_id.unwrap_or_else(default_process_template_id),
            enabled_stage_ids.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
pub(crate) fn update_project(
    project_id: String,
    name: Option<String>,
    process_template_id: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let project = store
        .update_project(&project_id, name.as_deref(), process_template_id)
        .map_err(|e| e.to_string())?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
pub(crate) fn archive_project(
    project_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .archive_project(&project_id)
        .map_err(|e| e.to_string())?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    app.emit("sessions_index_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}
