use std::sync::Arc;

use crate::models::ProcessTemplateInfo;
use crate::store::SessionStore;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub(crate) fn list_process_templates(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProcessTemplateInfo>, String> {
    store.list_process_templates().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn create_process_template(
    name: String,
    description: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProcessTemplateInfo, String> {
    let process_template = store
        .create_process_template(&name, description.as_deref())
        .map_err(|e| e.to_string())?;
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
    let process_template = store
        .update_process_template(
            &process_template_id,
            name.as_deref(),
            description.as_ref().map(|value| value.as_deref()),
        )
        .map_err(|e| e.to_string())?;
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
    store
        .delete_process_template(&process_template_id)
        .map_err(|e| e.to_string())?;
    app.emit("process_templates_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}
