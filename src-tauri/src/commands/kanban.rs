use std::sync::Arc;

use crate::models::{KanbanItem, KanbanStatus};
use crate::store::{capabilities::KanbanStore, SessionStore};
use tauri::State;

#[tauri::command]
pub(crate) fn list_kanban_items(
    project_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<KanbanItem>, String> {
    list_kanban_items_with_store(store.inner().as_ref(), &project_id)
}

#[tauri::command]
pub(crate) fn create_kanban_item(
    project_id: String,
    title: String,
    description: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    create_kanban_item_with_store(
        store.inner().as_ref(),
        &project_id,
        &title,
        description.as_deref(),
    )
}

#[tauri::command]
pub(crate) fn update_kanban_item(
    item_id: String,
    title: Option<String>,
    description: Option<Option<String>>,
    status: Option<KanbanStatus>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    update_kanban_item_with_store(
        store.inner().as_ref(),
        &item_id,
        title.as_deref(),
        description.as_ref().map(|value| value.as_deref()),
        status,
    )
}

#[tauri::command]
pub(crate) fn update_kanban_item_status(
    item_id: String,
    status: KanbanStatus,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    update_kanban_item_with_store(store.inner().as_ref(), &item_id, None, None, Some(status))
}

#[tauri::command]
pub(crate) fn delete_kanban_item(
    item_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    delete_kanban_item_with_store(store.inner().as_ref(), &item_id)
}

fn list_kanban_items_with_store<S: KanbanStore + ?Sized>(
    store: &S,
    project_id: &str,
) -> Result<Vec<KanbanItem>, String> {
    store
        .list_kanban_items(project_id)
        .map_err(|e| e.to_string())
}

fn create_kanban_item_with_store<S: KanbanStore + ?Sized>(
    store: &S,
    project_id: &str,
    title: &str,
    description: Option<&str>,
) -> Result<KanbanItem, String> {
    store
        .create_kanban_item(project_id, title, description)
        .map_err(|e| e.to_string())
}

fn update_kanban_item_with_store<S: KanbanStore + ?Sized>(
    store: &S,
    item_id: &str,
    title: Option<&str>,
    description: Option<Option<&str>>,
    status: Option<KanbanStatus>,
) -> Result<KanbanItem, String> {
    store
        .update_kanban_item(item_id, title, description, status)
        .map_err(|e| e.to_string())
}

fn delete_kanban_item_with_store<S: KanbanStore + ?Sized>(
    store: &S,
    item_id: &str,
) -> Result<(), String> {
    store.delete_kanban_item(item_id).map_err(|e| e.to_string())
}
