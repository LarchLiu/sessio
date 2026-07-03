use std::sync::Arc;

use crate::models::ProcessTemplateInfo;
use crate::store::{capabilities::ProcessTemplateStore, SessionStore};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) fn list_process_templates(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProcessTemplateInfo>, String> {
    list_process_templates_with_store(store.inner().as_ref())
}

#[tauri::command]
pub(crate) fn create_process_template(
    name: String,
    description: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProcessTemplateInfo, String> {
    let process_template =
        create_process_template_with_store(store.inner().as_ref(), &name, description.as_deref())?;
    app.emit("process_templates_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(process_template)
}

#[tauri::command]
pub(crate) fn update_process_template(
    process_template_id: String,
    name: Option<String>,
    description: Option<Option<String>>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProcessTemplateInfo, String> {
    let process_template = update_process_template_with_store(
        store.inner().as_ref(),
        &process_template_id,
        name.as_deref(),
        description.as_ref().map(|value| value.as_deref()),
    )?;
    app.emit("process_templates_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(process_template)
}

#[tauri::command]
pub(crate) fn delete_process_template(
    process_template_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    delete_process_template_with_store(store.inner().as_ref(), &process_template_id)?;
    app.emit("process_templates_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn list_process_templates_with_store<S: ProcessTemplateStore + ?Sized>(
    store: &S,
) -> Result<Vec<ProcessTemplateInfo>, String> {
    store.list_process_templates().map_err(|e| e.to_string())
}

fn create_process_template_with_store<S: ProcessTemplateStore + ?Sized>(
    store: &S,
    name: &str,
    description: Option<&str>,
) -> Result<ProcessTemplateInfo, String> {
    store
        .create_process_template(name, description)
        .map_err(|e| e.to_string())
}

fn update_process_template_with_store<S: ProcessTemplateStore + ?Sized>(
    store: &S,
    process_template_id: &str,
    name: Option<&str>,
    description: Option<Option<&str>>,
) -> Result<ProcessTemplateInfo, String> {
    store
        .update_process_template(process_template_id, name, description)
        .map_err(|e| e.to_string())
}

fn delete_process_template_with_store<S: ProcessTemplateStore + ?Sized>(
    store: &S,
    process_template_id: &str,
) -> Result<(), String> {
    store
        .delete_process_template(process_template_id)
        .map_err(|e| e.to_string())
}
