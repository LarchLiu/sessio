use std::sync::Arc;

use crate::models::{KanbanItem, KanbanStatus};
use crate::store::SessionStore;
use tauri::State;

#[tauri::command]
pub(crate) fn list_kanban_items(
    project_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<KanbanItem>, String> {
    store
        .list_kanban_items(&project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn create_kanban_item(
    project_id: String,
    title: String,
    description: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    store
        .create_kanban_item(&project_id, &title, description.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn update_kanban_item(
    item_id: String,
    title: Option<String>,
    description: Option<Option<String>>,
    status: Option<KanbanStatus>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    let description_ref = description.as_ref().map(|value| value.as_deref());
    store
        .update_kanban_item(&item_id, title.as_deref(), description_ref, status)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn update_kanban_item_status(
    item_id: String,
    status: KanbanStatus,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    store
        .update_kanban_item(&item_id, None, None, Some(status))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn delete_kanban_item(
    item_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .delete_kanban_item(&item_id)
        .map_err(|e| e.to_string())
}
