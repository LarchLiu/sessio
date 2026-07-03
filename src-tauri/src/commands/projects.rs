use std::sync::Arc;

use crate::default_process_template_id;
use crate::models::ProjectInfo;
use crate::store::{capabilities::ProjectStore, SessionStore};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) fn list_projects(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProjectInfo>, String> {
    list_projects_with_store(store.inner().as_ref())
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
    let project = add_existing_project_with_store(
        store.inner().as_ref(),
        &path,
        name.as_deref(),
        process_template_id,
        enabled_stage_ids.as_deref(),
    )?;
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
    let project = update_project_with_store(
        store.inner().as_ref(),
        &project_id,
        name.as_deref(),
        process_template_id,
    )?;
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
    archive_project_with_store(store.inner().as_ref(), &project_id)?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    app.emit("sessions_index_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_projects_with_store<S: ProjectStore + ?Sized>(
    store: &S,
) -> Result<Vec<ProjectInfo>, String> {
    store.list_projects().map_err(|e| e.to_string())
}

fn add_existing_project_with_store<S: ProjectStore + ?Sized>(
    store: &S,
    path: &str,
    name: Option<&str>,
    process_template_id: Option<String>,
    enabled_stage_ids: Option<&[String]>,
) -> Result<ProjectInfo, String> {
    store
        .add_project(
            path,
            name,
            process_template_id.unwrap_or_else(default_process_template_id),
            enabled_stage_ids,
        )
        .map_err(|e| e.to_string())
}

fn update_project_with_store<S: ProjectStore + ?Sized>(
    store: &S,
    project_id: &str,
    name: Option<&str>,
    process_template_id: Option<String>,
) -> Result<ProjectInfo, String> {
    store
        .update_project(project_id, name, process_template_id)
        .map_err(|e| e.to_string())
}

fn archive_project_with_store<S: ProjectStore + ?Sized>(
    store: &S,
    project_id: &str,
) -> Result<(), String> {
    store.archive_project(project_id).map_err(|e| e.to_string())
}
