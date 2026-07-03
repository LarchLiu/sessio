use std::sync::Arc;

use crate::models::{AssistantAgentInfo, AssistantInfo, AssistantType};
use crate::store::{NewAssistant, SessionStore};
use tauri::{AppHandle, Emitter, State};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateAssistantRequest {
    name: String,
    agent: AssistantAgentInfo,
    system_prompt: Option<String>,
    color: Option<String>,
    #[serde(default)]
    selected_skill_ids: Vec<String>,
    #[serde(default)]
    selected_mcp_ids: Vec<String>,
    assistant_type: AssistantType,
    process_template_id: Option<String>,
    project_id: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateAssistantRequest {
    assistant_id: String,
    name: Option<String>,
    agent: Option<AssistantAgentInfo>,
    system_prompt: Option<Option<String>>,
    color: Option<Option<String>>,
    selected_skill_ids: Option<Vec<String>>,
    selected_mcp_ids: Option<Vec<String>>,
    enabled: Option<bool>,
}

#[tauri::command]
pub(crate) fn list_assistants(
    project_id: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<AssistantInfo>, String> {
    store
        .list_assistants(project_id.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn create_assistant(
    req: CreateAssistantRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<AssistantInfo, String> {
    let CreateAssistantRequest {
        name,
        agent,
        system_prompt,
        color,
        selected_skill_ids,
        selected_mcp_ids,
        assistant_type,
        process_template_id,
        project_id,
    } = req;
    let assistant = store
        .create_assistant(NewAssistant {
            name: &name,
            agent,
            system_prompt: system_prompt.as_deref(),
            color: color.as_deref(),
            selected_skill_ids,
            selected_mcp_ids,
            assistant_type,
            process_template_id,
            project_id: project_id.as_deref(),
        })
        .map_err(|e| e.to_string())?;
    app.emit("assistants_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(assistant)
}

#[tauri::command]
pub(crate) fn update_assistant(
    req: UpdateAssistantRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<AssistantInfo, String> {
    let UpdateAssistantRequest {
        assistant_id,
        name,
        agent,
        system_prompt,
        color,
        selected_skill_ids,
        selected_mcp_ids,
        enabled,
    } = req;
    let system_prompt_ref = system_prompt.as_ref().map(|value| value.as_deref());
    let color_ref = color.as_ref().map(|value| value.as_deref());
    let assistant = store
        .update_assistant(
            &assistant_id,
            name.as_deref(),
            agent,
            system_prompt_ref,
            color_ref,
            selected_skill_ids,
            selected_mcp_ids,
            enabled,
        )
        .map_err(|e| e.to_string())?;
    app.emit("assistants_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(assistant)
}

#[tauri::command]
pub(crate) fn delete_assistant(
    assistant_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .delete_assistant(&assistant_id)
        .map_err(|e| e.to_string())?;
    app.emit("assistants_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}
