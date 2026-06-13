pub mod agents;
pub mod astra;
pub mod cli;
pub mod config;
pub mod im_bridge;
pub mod indexer;
pub mod memory;
pub mod models;
pub mod network;
pub mod polling;
pub mod shell_env;
pub mod store;
pub mod turns;
pub mod watch;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::{thread, time::Duration};

use agents::runtime::metadata::{
    runtime_agents_from_db, startup_probe_runtime_agents, RuntimeAgentsCache,
};
use agents::runtime::pi_session_store::PiAcpSessionStore;
use agents::runtime::types::{
    AgentInput, AgentSessionConfigChange, AgentSessionHandle, AgentTurnHandle,
    EnsureAgentRuntimeSession, RuntimeStatus, StartAgentSession,
};
use agents::runtime::RuntimeManager;
use astra::{AstraHandle, AstraService, CancelAstraRunRequest, CreateAstraRunRequest};
use indexer::{IndexTask, IndexerHandle};
use memory::qmd::{query_project, search_project, QmdOptions};
use memory::service::MemoryService;
use memory::{MemoryBackendStatus, MemoryStore};
use models::{
    Agent, AgentAiProviderInfo, AgentInfo, AssistantAgentInfo, AssistantInfo, AssistantType,
    AstraConfig, IssueSeverity, IssueStatus, KanbanItem, KanbanStatus, PlanRoundInfo,
    PlanRoundMode, PlanRoundSource, PlanRoundStatus, PlanTaskInfo, PlanTaskRisk,
    PlanTaskSessionInfo, PlanTaskSessionRole, PlanTaskStatus, ProcessTemplateInfo, ProjectInfo,
    ProjectStageInfo, RuntimeAgentMetadata, SessionHistoryTurn, SessionInfo, StageInfo,
    StageIssueInfo, StageStatus, ThreadAgentInfo, ThreadIndexItemInfo, ThreadInfo, ThreadKind,
    ThreadReplayInfo,
};
use store::cached::CachedStore;
use store::sqlite::SqliteStore;
use store::{
    AgentPreferencesPatch, AstraConfigPatch, NewAssistant, NewPlanRound, NewPlanTask,
    NewPlanTaskSession, PlanTaskStatusPatch, ProjectStagePatch, SessionHistorySnapshotRecord,
    SessionStore, ThreadWorkSnapshotRecord,
};
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent,
};

const HISTORY_CACHE_VERSION: i64 = 1;
const THREAD_WORK_SNAPSHOT_VERSION: i64 = 2;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadsUpdatedPayload {
    project_id: Option<String>,
    thread_id: Option<String>,
}

fn emit_threads_updated(
    app: &AppHandle,
    project_id: Option<String>,
    thread_id: Option<String>,
) -> Result<(), String> {
    app.emit(
        "threads_updated",
        ThreadsUpdatedPayload {
            project_id,
            thread_id,
        },
    )
    .map_err(|e| e.to_string())
}

fn thread_project_id(store: &dyn SessionStore, thread_id: &str) -> Option<String> {
    store
        .get_thread_work_state(thread_id)
        .map(|thread| thread.project_id)
        .ok()
}

fn default_process_template_id() -> String {
    "code".to_string()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeAgentOptionInput {
    value: String,
    label: String,
    display_name: Option<String>,
    enabled: Option<bool>,
    order: Option<i64>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRuntimeAgentPreferencesRequest {
    agent: Agent,
    display_name: Option<String>,
    enabled: Option<bool>,
    order: Option<i64>,
    ai_provider: Option<String>,
    ai_providers: Option<Vec<AgentAiProviderInfo>>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    models: Option<Vec<RuntimeAgentOptionInput>>,
    efforts: Option<Vec<RuntimeAgentOptionInput>>,
    permission_modes: Option<Vec<RuntimeAgentOptionInput>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAgentPreferencesRequest {
    agent_id: String,
    display_name: Option<String>,
    enabled: Option<bool>,
    order: Option<i64>,
    ai_provider: Option<String>,
    ai_providers: Option<Vec<AgentAiProviderInfo>>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    models: Option<Vec<RuntimeAgentOptionInput>>,
    efforts: Option<Vec<RuntimeAgentOptionInput>>,
    permission_modes: Option<Vec<RuntimeAgentOptionInput>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateAssistantRequest {
    name: String,
    agent: AssistantAgentInfo,
    system_prompt: Option<String>,
    color: Option<String>,
    assistant_type: AssistantType,
    process_template_id: Option<String>,
    project_id: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProjectStageRequest {
    stage_id: String,
    name: Option<String>,
    description: Option<Option<String>>,
    icon: Option<Option<String>>,
    order: Option<i64>,
    enabled: Option<bool>,
    allow_empty_assistants: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateAssistantRequest {
    assistant_id: String,
    name: Option<String>,
    agent: Option<AssistantAgentInfo>,
    system_prompt: Option<Option<String>>,
    color: Option<Option<String>>,
    enabled: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePlanTaskRequest {
    thread_stage_id: Option<String>,
    assistant_id: Option<String>,
    agent_participant_id: Option<String>,
    target_agent: Agent,
    stage_snapshot_json: Option<String>,
    assistant_snapshot_json: Option<String>,
    agent_snapshot_json: String,
    title: String,
    prompt: String,
    expected_output: Option<String>,
    risk: PlanTaskRisk,
    sort_order: i64,
    status: PlanTaskStatus,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePlanRoundRequest {
    thread_id: String,
    astra_run_id: Option<String>,
    round_index: Option<i64>,
    summary: Option<String>,
    mode: PlanRoundMode,
    source: PlanRoundSource,
    status: PlanRoundStatus,
    tasks: Vec<CreatePlanTaskRequest>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdatePlanTaskStatusRequest {
    task_id: String,
    status: PlanTaskStatus,
    result_summary: Option<Option<String>>,
    error: Option<Option<String>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkPlanTaskSessionRequest {
    task_id: String,
    agent: Agent,
    session_id: String,
    role: PlanTaskSessionRole,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeAgentSelectionDto {
    agent: Agent,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    updated_at: i64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetRuntimeAgentSelectionRequest {
    agent: Agent,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryResult {
    pub message_count: usize,
    pub indexed_through: Option<i64>,
    pub turns: Vec<SessionHistoryTurn>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistorySnapshotGroup {
    pub ancestor_agent: Agent,
    pub ancestor_session_id: String,
    pub ancestor_index: i64,
    pub turns: Vec<SessionHistoryTurn>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistorySnapshotsResult {
    pub has_snapshot: bool,
    pub groups: Vec<SessionHistorySnapshotGroup>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSessionHistorySnapshotGroup {
    pub ancestor_agent: Agent,
    pub ancestor_session_id: String,
    pub ancestor_index: i64,
    pub turns: Vec<SessionHistoryTurn>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command]
fn list_sessions(store: State<'_, Arc<dyn SessionStore>>) -> Result<Vec<SessionInfo>, String> {
    store.list_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_channel_sessions(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<models::ChannelSessionInfo>, String> {
    store.list_channel_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_process_templates(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProcessTemplateInfo>, String> {
    store.list_process_templates().map_err(|e| e.to_string())
}

#[tauri::command]
fn create_process_template(
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
fn update_process_template(
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
fn delete_process_template(
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

#[tauri::command]
fn list_projects(store: State<'_, Arc<dyn SessionStore>>) -> Result<Vec<ProjectInfo>, String> {
    store.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
fn add_existing_project(
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
fn create_project(
    parent_path: String,
    name: String,
    process_template_id: Option<String>,
    enabled_stage_ids: Option<Vec<String>>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let project = store
        .create_project(
            &parent_path,
            &name,
            process_template_id.unwrap_or_else(default_process_template_id),
            enabled_stage_ids.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
fn create_default_project(
    name: String,
    process_template_id: Option<String>,
    enabled_stage_ids: Option<Vec<String>>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let parent = dirs::home_dir()
        .ok_or_else(|| "home directory not found".to_string())?
        .join(".sessio")
        .join("projects");
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let project = store
        .create_project(
            &parent.to_string_lossy(),
            &name,
            process_template_id.unwrap_or_else(default_process_template_id),
            enabled_stage_ids.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
fn update_project(
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
fn archive_project(
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

#[tauri::command]
fn list_agents(store: State<'_, Arc<dyn SessionStore>>) -> Result<Vec<AgentInfo>, String> {
    store.list_agents().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_astra_config(store: State<'_, Arc<dyn SessionStore>>) -> Result<AstraConfig, String> {
    store.get_astra_config().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_astra_config(
    config: serde_json::Value,
    store: State<'_, Arc<dyn SessionStore>>,
    astra: State<'_, AstraService>,
) -> Result<AstraConfig, String> {
    let patch = AstraConfigPatch {
        agent: config.get("agent").map(|v| v.as_str()),
        model: config.get("model").map(|v| v.as_str()),
        effort: config.get("effort").map(|v| v.as_str()),
        permission_mode: config.get("permissionMode").map(|v| v.as_str()),
    };

    let updated = store
        .update_astra_config(patch)
        .map_err(|e| e.to_string())?;

    astra.reload_config();

    Ok(updated)
}

#[tauri::command]
fn update_agent_preferences(
    req: UpdateAgentPreferencesRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
    astra: State<'_, AstraService>,
) -> Result<AgentInfo, String> {
    let models = runtime_option_inputs_to_metadata(req.models);
    let efforts = runtime_option_inputs_to_metadata(req.efforts);
    let permission_modes = runtime_option_inputs_to_metadata(req.permission_modes);
    let updated = store
        .update_agent_preferences_by_id(
            &req.agent_id,
            AgentPreferencesPatch {
                display_name: req.display_name.as_deref(),
                enabled: req.enabled,
                order: req.order,
                ai_provider: req.ai_provider.as_deref(),
                ai_providers: req.ai_providers.as_deref(),
                model: req.model.as_deref(),
                effort: req.effort.as_deref(),
                permission_mode: req.permission_mode.as_deref(),
                models: models.as_deref(),
                efforts: efforts.as_deref(),
                permission_modes: permission_modes.as_deref(),
            },
        )
        .map_err(|e| e.to_string())?;
    if updated.id == Agent::AstraPi.as_str() {
        astra.update_astra_preferences_cache(updated.clone());
    }
    app.emit("runtime_agents_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

fn runtime_option_inputs_to_metadata(
    options: Option<Vec<RuntimeAgentOptionInput>>,
) -> Option<Vec<models::RuntimeAgentOptionMetadata>> {
    options.map(|options| {
        options
            .into_iter()
            .enumerate()
            .map(|(index, option)| models::RuntimeAgentOptionMetadata {
                display_name: option.display_name.unwrap_or_else(|| option.label.clone()),
                enabled: option.enabled.unwrap_or(true),
                order: option.order.unwrap_or(index as i64),
                value: option.value,
                label: option.label,
            })
            .collect()
    })
}

fn runtime_agent_selection_to_dto(
    selection: store::RuntimeAgentSelection,
) -> RuntimeAgentSelectionDto {
    RuntimeAgentSelectionDto {
        agent: selection.agent,
        model: selection.model,
        effort: selection.effort,
        permission_mode: selection.permission_mode,
        updated_at: selection.updated_at,
    }
}

fn db_runtime_agents(
    store: &Arc<dyn SessionStore>,
    cache: &[RuntimeAgentMetadata],
) -> Result<Vec<RuntimeAgentMetadata>, String> {
    runtime_agents_from_db(store.clone(), cache).map_err(|e| e.to_string())
}

fn insert_option_if_missing(
    options: &mut agents::runtime::types::RuntimeMetadata,
    key: &str,
    value: Option<String>,
) {
    if options.contains_key(key) {
        return;
    }
    if let Some(value) = value.map(|value| value.trim().to_string()) {
        if !value.is_empty() {
            options.insert(key.to_string(), serde_json::Value::String(value));
        }
    }
}

fn runtime_transport_option(transport: agents::runtime::types::RuntimeTransportKind) -> String {
    match transport {
        agents::runtime::types::RuntimeTransportKind::Acp => "acp",
        agents::runtime::types::RuntimeTransportKind::CliStreamJson => "cliStreamJson",
        agents::runtime::types::RuntimeTransportKind::PlainCli => "plainCli",
        agents::runtime::types::RuntimeTransportKind::Sidecar => "sidecar",
        agents::runtime::types::RuntimeTransportKind::Fake => "fake",
    }
    .to_string()
}

fn hydrate_start_request_from_db(
    req: &mut StartAgentSession,
    store: &Arc<dyn SessionStore>,
) -> anyhow::Result<()> {
    let Some(agent) = store
        .list_agents()?
        .into_iter()
        .find(|agent| agent.id == req.agent.as_str())
    else {
        return Ok(());
    };
    insert_option_if_missing(&mut req.options, "model", agent.model);
    insert_option_if_missing(&mut req.options, "effort", agent.effort);
    insert_option_if_missing(&mut req.options, "permissionMode", agent.permission_mode);
    insert_option_if_missing(
        &mut req.options,
        "transport",
        Some(runtime_transport_option(agent.transport)),
    );
    if !req.options.contains_key("command") && !req.options.contains_key("acpCommand") {
        if let Some(command) = agent.commands.session.first().cloned() {
            insert_option_if_missing(&mut req.options, "command", Some(command));
        }
    }
    Ok(())
}

fn hydrate_ensure_request_from_db(
    req: &mut EnsureAgentRuntimeSession,
    store: &Arc<dyn SessionStore>,
) -> anyhow::Result<()> {
    let mut start_req = StartAgentSession {
        agent: req.agent,
        workspace_path: req.workspace_path.clone(),
        initial_prompt: None,
        source_session_id: None,
        source_agent: req.source_agent,
        options: std::mem::take(&mut req.options),
    };
    hydrate_start_request_from_db(&mut start_req, store)?;
    req.options = start_req.options;
    Ok(())
}

#[tauri::command]
fn list_assistants(
    project_id: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<AssistantInfo>, String> {
    store
        .list_assistants(project_id.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_assistant(
    req: CreateAssistantRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<AssistantInfo, String> {
    let CreateAssistantRequest {
        name,
        agent,
        system_prompt,
        color,
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
fn update_assistant(
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
            enabled,
        )
        .map_err(|e| e.to_string())?;
    app.emit("assistants_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(assistant)
}

#[tauri::command]
fn delete_assistant(
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

#[tauri::command]
fn list_threads(
    project_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ThreadInfo>, String> {
    store.list_threads(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_thread_work_state(
    thread_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadInfo, String> {
    store
        .get_thread_work_state(&thread_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_thread_replay(
    thread_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadReplayInfo, String> {
    store
        .get_thread_replay(&thread_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_thread_index(
    project_id: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ThreadIndexItemInfo>, String> {
    store
        .list_thread_index(project_id.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn create_thread(
    project_id: String,
    goal: String,
    description: Option<String>,
    kind: Option<ThreadKind>,
    assistant_ids: Option<Vec<String>>,
    agent_participants: Option<Vec<ThreadAgentInfo>>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadInfo, String> {
    let thread = store
        .create_thread_with_options(
            &project_id,
            &goal,
            description.as_deref(),
            kind.unwrap_or_default(),
            assistant_ids.as_deref().unwrap_or(&[]),
            agent_participants.as_deref().unwrap_or(&[]),
        )
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(thread.project_id.clone()),
        Some(thread.id.clone()),
    )?;
    Ok(thread)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn update_thread(
    thread_id: String,
    goal: Option<String>,
    description: Option<Option<String>>,
    enabled: Option<bool>,
    kind: Option<ThreadKind>,
    assistant_ids: Option<Vec<String>>,
    agent_participants: Option<Vec<ThreadAgentInfo>>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadInfo, String> {
    let description_ref = description.as_ref().map(|value| value.as_deref());
    let thread = store
        .update_thread_with_options(
            &thread_id,
            goal.as_deref(),
            description_ref,
            enabled,
            kind,
            assistant_ids.as_deref(),
            agent_participants.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(thread.project_id.clone()),
        Some(thread.id.clone()),
    )?;
    Ok(thread)
}

#[tauri::command]
fn delete_thread(
    thread_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    let project_id = thread_project_id(store.as_ref(), &thread_id);
    store.delete_thread(&thread_id).map_err(|e| e.to_string())?;
    emit_threads_updated(&app, project_id, Some(thread_id))?;
    Ok(())
}

#[tauri::command]
fn create_plan_round(
    req: CreatePlanRoundRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<PlanRoundInfo, String> {
    let tasks = req
        .tasks
        .iter()
        .map(|task| NewPlanTask {
            thread_stage_id: task.thread_stage_id.as_deref(),
            assistant_id: task.assistant_id.as_deref(),
            agent_participant_id: task.agent_participant_id.as_deref(),
            target_agent: task.target_agent,
            stage_snapshot_json: task.stage_snapshot_json.as_deref(),
            assistant_snapshot_json: task.assistant_snapshot_json.as_deref(),
            agent_snapshot_json: &task.agent_snapshot_json,
            title: &task.title,
            prompt: &task.prompt,
            expected_output: task.expected_output.as_deref(),
            risk: task.risk,
            sort_order: task.sort_order,
            status: task.status,
        })
        .collect();
    let round = store
        .create_plan_round(NewPlanRound {
            thread_id: &req.thread_id,
            astra_run_id: req.astra_run_id.as_deref(),
            round_index: req.round_index,
            summary: req.summary.as_deref(),
            mode: req.mode,
            source: req.source,
            status: req.status,
            tasks,
        })
        .map_err(|e| e.to_string())?;
    let project_id = thread_project_id(store.as_ref(), &round.thread_id);
    emit_threads_updated(&app, project_id, Some(round.thread_id.clone()))?;
    Ok(round)
}

#[tauri::command]
fn get_plan_round(
    round_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Option<PlanRoundInfo>, String> {
    store.get_plan_round(&round_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_plan_rounds(
    thread_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<PlanRoundInfo>, String> {
    store
        .list_plan_rounds(&thread_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_plan_task_status(
    req: UpdatePlanTaskStatusRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<PlanTaskInfo, String> {
    let result_summary = req.result_summary.as_ref().map(|value| value.as_deref());
    let error = req.error.as_ref().map(|value| value.as_deref());
    let task = store
        .update_plan_task_status(
            &req.task_id,
            PlanTaskStatusPatch {
                status: req.status,
                result_summary,
                error,
            },
        )
        .map_err(|e| e.to_string())?;
    let thread_id = store
        .get_plan_round(&task.round_id)
        .ok()
        .flatten()
        .map(|round| round.thread_id);
    let project_id = thread_id
        .as_deref()
        .and_then(|thread_id| thread_project_id(store.as_ref(), thread_id));
    emit_threads_updated(&app, project_id, thread_id)?;
    Ok(task)
}

#[tauri::command]
fn complete_plan_task_and_start_next(
    req: UpdatePlanTaskStatusRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<PlanRoundInfo, String> {
    let result_summary = req.result_summary.as_ref().map(|value| value.as_deref());
    let error = req.error.as_ref().map(|value| value.as_deref());
    let round = store
        .complete_plan_task_and_start_next(
            &req.task_id,
            PlanTaskStatusPatch {
                status: req.status,
                result_summary,
                error,
            },
        )
        .map_err(|e| e.to_string())?;
    let project_id = thread_project_id(store.as_ref(), &round.thread_id);
    emit_threads_updated(&app, project_id, Some(round.thread_id.clone()))?;
    Ok(round)
}

#[tauri::command]
fn link_plan_task_session(
    req: LinkPlanTaskSessionRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<PlanTaskSessionInfo, String> {
    let thread_id = store
        .get_plan_task_thread_id(&req.task_id)
        .map_err(|e| e.to_string())?;
    let session = store
        .link_plan_task_session(NewPlanTaskSession {
            task_id: &req.task_id,
            agent: req.agent,
            session_id: &req.session_id,
            role: req.role,
            attempt_id: None,
            attempt_count: 1,
        })
        .map_err(|e| e.to_string())?;
    let project_id = thread_id
        .as_deref()
        .and_then(|thread_id| thread_project_id(store.as_ref(), thread_id));
    emit_threads_updated(&app, project_id, thread_id)?;
    Ok(session)
}

#[tauri::command]
fn list_plan_task_sessions(
    task_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<PlanTaskSessionInfo>, String> {
    store
        .list_plan_task_sessions(&task_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_project_stages(
    project_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProjectStageInfo>, String> {
    store
        .list_project_stages(&project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_process_template_stages(
    process_template_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<ProjectStageInfo>, String> {
    store
        .list_process_template_stages(&process_template_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_project_stage(
    project_id: String,
    process_template_id: Option<String>,
    name: String,
    description: Option<String>,
    icon: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectStageInfo, String> {
    let stage = store
        .create_project_stage(
            &project_id,
            process_template_id,
            &name,
            description.as_deref(),
            icon.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, Some(project_id), None)?;
    Ok(stage)
}

#[tauri::command]
fn update_project_stage(
    req: UpdateProjectStageRequest,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectStageInfo, String> {
    let UpdateProjectStageRequest {
        stage_id,
        name,
        description,
        icon,
        order,
        enabled,
        allow_empty_assistants,
    } = req;
    let description_ref = description.as_ref().map(|value| value.as_deref());
    let icon_ref = icon.as_ref().map(|value| value.as_deref());
    let stage = store
        .update_project_stage(
            &stage_id,
            ProjectStagePatch {
                name: name.as_deref(),
                description: description_ref,
                icon: icon_ref,
                order,
                enabled,
                allow_empty_assistants,
            },
        )
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, stage.project_id.clone(), None)?;
    Ok(stage)
}

#[tauri::command]
fn update_project_stage_assistants(
    stage_id: String,
    assistant_ids: Vec<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectStageInfo, String> {
    let stage = store
        .update_project_stage_assistants(&stage_id, &assistant_ids)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, stage.project_id.clone(), None)?;
    Ok(stage)
}

#[tauri::command]
fn delete_project_stage(
    stage_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .delete_project_stage(&stage_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, None, None)?;
    Ok(())
}

#[tauri::command]
fn add_thread_stage(
    thread_id: String,
    stage_id: String,
    assistant_ids: Vec<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageInfo, String> {
    let stage = store
        .add_thread_stage(&thread_id, &stage_id, &assistant_ids)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(stage.project_id.clone()),
        Some(stage.thread_id.clone()),
    )?;
    Ok(stage)
}

#[tauri::command]
fn update_thread_stage(
    thread_stage_id: String,
    assistant_ids: Option<Vec<String>>,
    order: Option<i64>,
    enabled: Option<bool>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageInfo, String> {
    let stage = store
        .update_thread_stage(&thread_stage_id, assistant_ids.as_deref(), order, enabled)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(stage.project_id.clone()),
        Some(stage.thread_id.clone()),
    )?;
    Ok(stage)
}

#[tauri::command]
fn update_thread_stage_state(
    thread_stage_id: String,
    status: Option<String>,
    summary: Option<String>,
    outcome: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageInfo, String> {
    let status = match status {
        Some(value) => Some(
            StageStatus::from_db_str(&value)
                .ok_or_else(|| format!("invalid stage status: {value}"))?,
        ),
        None => None,
    };
    // An omitted field leaves the value unchanged; an empty string clears it.
    let summary = summary.map(|value| (!value.is_empty()).then_some(value));
    let outcome = outcome.map(|value| (!value.is_empty()).then_some(value));
    let stage = store
        .update_thread_stage_state(&thread_stage_id, status, summary, outcome)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(stage.project_id.clone()),
        Some(stage.thread_id.clone()),
    )?;
    Ok(stage)
}

#[tauri::command]
fn list_thread_stage_issues(
    thread_stage_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<StageIssueInfo>, String> {
    store
        .list_thread_stage_issues(&thread_stage_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_thread_stage_issue(
    thread_stage_id: String,
    title: String,
    description: Option<String>,
    severity: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageIssueInfo, String> {
    let severity = IssueSeverity::from_db_str(&severity)
        .ok_or_else(|| format!("invalid issue severity: {severity}"))?;
    let issue = store
        .create_thread_stage_issue(&thread_stage_id, &title, description.as_deref(), severity)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, None, None)?;
    Ok(issue)
}

#[tauri::command]
fn update_thread_stage_issue(
    issue_id: String,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    severity: Option<String>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageIssueInfo, String> {
    let status = match status {
        Some(value) => Some(
            IssueStatus::from_db_str(&value)
                .ok_or_else(|| format!("invalid issue status: {value}"))?,
        ),
        None => None,
    };
    let severity = match severity {
        Some(value) => Some(
            IssueSeverity::from_db_str(&value)
                .ok_or_else(|| format!("invalid issue severity: {value}"))?,
        ),
        None => None,
    };
    // An omitted field leaves the value unchanged; an empty string clears it.
    let description = description.map(|value| (!value.is_empty()).then_some(value));
    let issue = store
        .update_thread_stage_issue(
            &issue_id,
            title.as_deref(),
            description.as_ref().map(|inner| inner.as_deref()),
            status,
            severity,
        )
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, None, None)?;
    Ok(issue)
}

#[tauri::command]
fn delete_thread_stage_issue(
    issue_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .delete_thread_stage_issue(&issue_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, None, None)?;
    Ok(())
}

#[tauri::command]
fn update_thread_stage_assistant_agent(
    thread_stage_id: String,
    assistant_id: String,
    agent: AssistantAgentInfo,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageInfo, String> {
    let stage = store
        .update_thread_stage_assistant_agent(&thread_stage_id, &assistant_id, agent)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(stage.project_id.clone()),
        Some(stage.thread_id.clone()),
    )?;
    Ok(stage)
}

#[tauri::command]
fn delete_thread_stage(
    thread_stage_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .delete_thread_stage(&thread_stage_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(&app, None, None)?;
    Ok(())
}

#[tauri::command]
fn set_thread_stage(
    thread_id: String,
    stage_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadInfo, String> {
    let thread = store
        .set_thread_stage(&thread_id, &stage_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(thread.project_id.clone()),
        Some(thread.id.clone()),
    )?;
    Ok(thread)
}

#[tauri::command]
fn link_thread_session(
    thread_id: String,
    agent: Agent,
    session_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadInfo, String> {
    let thread = store
        .link_thread_session(&thread_id, agent, &session_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(thread.project_id.clone()),
        Some(thread.id.clone()),
    )?;
    Ok(thread)
}

#[tauri::command]
fn unlink_thread_session(
    thread_id: String,
    agent: Agent,
    session_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ThreadInfo, String> {
    let thread = store
        .unlink_thread_session(&thread_id, agent, &session_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(thread.project_id.clone()),
        Some(thread.id.clone()),
    )?;
    Ok(thread)
}

#[tauri::command]
fn link_stage_session(
    stage_id: String,
    agent: Agent,
    session_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageInfo, String> {
    let stage = store
        .link_stage_session(&stage_id, agent, &session_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(stage.project_id.clone()),
        Some(stage.thread_id.clone()),
    )?;
    Ok(stage)
}

#[tauri::command]
fn unlink_stage_session(
    stage_id: String,
    agent: Agent,
    session_id: String,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<StageInfo, String> {
    let stage = store
        .unlink_stage_session(&stage_id, agent, &session_id)
        .map_err(|e| e.to_string())?;
    emit_threads_updated(
        &app,
        Some(stage.project_id.clone()),
        Some(stage.thread_id.clone()),
    )?;
    Ok(stage)
}

#[tauri::command]
fn list_kanban_items(
    project_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<KanbanItem>, String> {
    store
        .list_kanban_items(&project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_kanban_item(
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
fn update_kanban_item(
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
fn update_kanban_item_status(
    item_id: String,
    status: KanbanStatus,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    store
        .update_kanban_item(&item_id, None, None, Some(status))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_kanban_item(
    item_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .delete_kanban_item(&item_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn link_kanban_item_session(
    item_id: String,
    agent: Agent,
    session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    store
        .link_kanban_item_session(&item_id, agent, &session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn unlink_kanban_item_session(
    item_id: String,
    agent: Agent,
    session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<KanbanItem, String> {
    store
        .unlink_kanban_item_session(&item_id, agent, &session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_session_ancestors(
    agent: Agent,
    session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<SessionInfo>, String> {
    let sessions = store.list_all_sessions().map_err(|e| e.to_string())?;
    Ok(session_ancestors_from_db(agent, &session_id, &sessions))
}

fn session_ancestors_from_db(
    agent: Agent,
    session_id: &str,
    sessions: &[SessionInfo],
) -> Vec<SessionInfo> {
    let mut by_identity: HashMap<(Agent, String), SessionInfo> = HashMap::new();
    for session in sessions {
        let key = (session.agent, session.id.clone());
        let replace = by_identity
            .get(&key)
            .map(|current| better_lineage_candidate(session, current))
            .unwrap_or(true);
        if replace {
            by_identity.insert(key, session.clone());
        }
    }

    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor = by_identity.get(&(agent, session_id.to_string())).cloned();
    seen.insert((agent, session_id.to_string()));

    for _ in 0..32 {
        let Some(current) = cursor else {
            break;
        };
        let Some(parent_id) = current.forked_from_id.clone() else {
            break;
        };
        let parent_agent = current.forked_from_agent.unwrap_or(current.agent);
        let key = (parent_agent, parent_id);
        if !seen.insert(key.clone()) {
            break;
        }
        let Some(parent) = by_identity.get(&key).cloned() else {
            break;
        };
        chain.push(parent.clone());
        cursor = Some(parent);
    }

    chain.reverse();
    chain
}

fn better_lineage_candidate(candidate: &SessionInfo, current: &SessionInfo) -> bool {
    if candidate.available != current.available {
        return candidate.available;
    }
    if candidate.file_path.is_empty() != current.file_path.is_empty() {
        return !candidate.file_path.is_empty();
    }
    candidate.updated_at.or(candidate.started_at).unwrap_or(0)
        > current.updated_at.or(current.started_at).unwrap_or(0)
}

#[tauri::command]
fn get_session_history_snapshots(
    child_agent: Agent,
    child_session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SessionHistorySnapshotsResult, String> {
    let snapshots = store
        .get_session_history_snapshots(child_agent, &child_session_id)
        .map_err(|e| e.to_string())?;
    let has_snapshot = !snapshots.is_empty();
    if snapshots
        .iter()
        .any(|snapshot| snapshot.history_cache_version != HISTORY_CACHE_VERSION)
    {
        return Ok(SessionHistorySnapshotsResult {
            has_snapshot,
            groups: Vec::new(),
        });
    }

    Ok(SessionHistorySnapshotsResult {
        has_snapshot,
        groups: snapshots
            .into_iter()
            .map(|snapshot| SessionHistorySnapshotGroup {
                ancestor_agent: snapshot.ancestor_agent,
                ancestor_session_id: snapshot.ancestor_session_id,
                ancestor_index: snapshot.ancestor_index,
                turns: snapshot.turns,
            })
            .collect(),
    })
}

#[tauri::command]
fn save_session_history_snapshots(
    child_agent: Agent,
    child_session_id: String,
    groups: Vec<SaveSessionHistorySnapshotGroup>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    let created_at = now_ms();
    let snapshots: Vec<SessionHistorySnapshotRecord> = groups
        .into_iter()
        .map(|group| SessionHistorySnapshotRecord {
            child_agent,
            child_session_id: child_session_id.clone(),
            ancestor_agent: group.ancestor_agent,
            ancestor_session_id: group.ancestor_session_id,
            ancestor_index: group.ancestor_index,
            history_cache_version: HISTORY_CACHE_VERSION,
            created_at,
            turns: group.turns,
        })
        .collect();
    store
        .replace_session_history_snapshots(child_agent, &child_session_id, &snapshots)
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadWorkSnapshotResult {
    child_agent: Agent,
    child_session_id: String,
    thread_id: String,
    stage_id: Option<String>,
    version: i64,
    created_at: i64,
    snapshot: serde_json::Value,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadWorkSnapshotSourcesResult {
    child_agent: Agent,
    child_session_id: String,
    thread_id: String,
    stage_id: Option<String>,
    sources: Vec<ThreadWorkSnapshotSourceRef>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadWorkSnapshotSourceRef {
    kind: String,
    id: String,
    label: String,
    thread_id: Option<String>,
    thread_stage_id: Option<String>,
    issue_id: Option<String>,
    agent: Option<Agent>,
    session_id: Option<String>,
    file_path: Option<String>,
    ancestor_index: Option<i64>,
}

#[tauri::command]
fn save_thread_work_snapshot(
    child_agent: Agent,
    child_session_id: String,
    thread_id: String,
    stage_id: Option<String>,
    snapshot: serde_json::Value,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    let snapshot_json = serde_json::to_string(&snapshot).map_err(|e| e.to_string())?;
    let record = ThreadWorkSnapshotRecord {
        child_agent,
        child_session_id,
        thread_id,
        stage_id,
        snapshot_json,
        version: THREAD_WORK_SNAPSHOT_VERSION,
        created_at: now_ms(),
    };
    store
        .save_thread_work_snapshot(&record)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_thread_work_snapshot(
    child_agent: Agent,
    child_session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Option<ThreadWorkSnapshotResult>, String> {
    let record = store
        .get_thread_work_snapshot(child_agent, &child_session_id)
        .map_err(|e| e.to_string())?;
    let Some(record) = record else {
        return Ok(None);
    };
    let snapshot: serde_json::Value =
        serde_json::from_str(&record.snapshot_json).map_err(|e| e.to_string())?;
    Ok(Some(ThreadWorkSnapshotResult {
        child_agent: record.child_agent,
        child_session_id: record.child_session_id,
        thread_id: record.thread_id,
        stage_id: record.stage_id,
        version: record.version,
        created_at: record.created_at,
        snapshot,
    }))
}

fn build_thread_work_snapshot_sources(
    record: &ThreadWorkSnapshotRecord,
    snapshot: &serde_json::Value,
    current_thread: Option<&ThreadInfo>,
    history_snapshots: &[SessionHistorySnapshotRecord],
) -> Vec<ThreadWorkSnapshotSourceRef> {
    let mut sources = Vec::new();
    let mut seen = HashSet::new();
    push_snapshot_source(
        &mut sources,
        &mut seen,
        ThreadWorkSnapshotSourceRef {
            kind: "thread".to_string(),
            id: record.thread_id.clone(),
            label: snapshot
                .get("goal")
                .and_then(|value| value.as_str())
                .unwrap_or("Thread")
                .to_string(),
            thread_id: Some(record.thread_id.clone()),
            thread_stage_id: None,
            issue_id: None,
            agent: None,
            session_id: None,
            file_path: None,
            ancestor_index: None,
        },
    );
    if let Some(stages) = snapshot.get("stages").and_then(|value| value.as_array()) {
        for stage in stages {
            let Some(thread_stage_id) = stage
                .get("threadStageId")
                .or_else(|| stage.get("thread_stage_id"))
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            let stage_label = stage
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("Stage")
                .to_string();
            push_snapshot_source(
                &mut sources,
                &mut seen,
                ThreadWorkSnapshotSourceRef {
                    kind: "stage".to_string(),
                    id: thread_stage_id.to_string(),
                    label: stage_label.clone(),
                    thread_id: Some(record.thread_id.clone()),
                    thread_stage_id: Some(thread_stage_id.to_string()),
                    issue_id: None,
                    agent: None,
                    session_id: None,
                    file_path: None,
                    ancestor_index: None,
                },
            );

            if let Some(issues) = stage.get("issues").and_then(|value| value.as_array()) {
                for issue in issues {
                    let Some(issue_id) = issue.get("id").and_then(|value| value.as_str()) else {
                        continue;
                    };
                    let title = issue
                        .get("title")
                        .and_then(|value| value.as_str())
                        .unwrap_or("Issue")
                        .to_string();
                    push_snapshot_source(
                        &mut sources,
                        &mut seen,
                        ThreadWorkSnapshotSourceRef {
                            kind: "issue".to_string(),
                            id: issue_id.to_string(),
                            label: format!("{stage_label}: {title}"),
                            thread_id: Some(record.thread_id.clone()),
                            thread_stage_id: Some(thread_stage_id.to_string()),
                            issue_id: Some(issue_id.to_string()),
                            agent: None,
                            session_id: None,
                            file_path: None,
                            ancestor_index: None,
                        },
                    );
                }
            }

            if let Some(session_refs) = stage.get("sessionRefs").and_then(|value| value.as_array())
            {
                for session_ref in session_refs {
                    push_session_source(
                        &mut sources,
                        &mut seen,
                        "stage_session",
                        thread_stage_id,
                        &record.thread_id,
                        session_ref,
                        &SessionSourceContext {
                            current_thread,
                            history_snapshots,
                        },
                    );
                }
            }
        }
    }

    if let Some(session_refs) = snapshot
        .get("threadSessionRefs")
        .and_then(|value| value.as_array())
    {
        for session_ref in session_refs {
            push_session_source(
                &mut sources,
                &mut seen,
                "thread_session",
                "",
                &record.thread_id,
                session_ref,
                &SessionSourceContext {
                    current_thread,
                    history_snapshots,
                },
            );
        }
    }

    for snapshot in history_snapshots {
        let id = format!(
            "{}:{}:{}",
            snapshot.ancestor_agent.as_str(),
            snapshot.ancestor_session_id,
            snapshot.ancestor_index
        );
        push_snapshot_source(
            &mut sources,
            &mut seen,
            ThreadWorkSnapshotSourceRef {
                kind: "history_snapshot".to_string(),
                id,
                label: format!(
                    "History snapshot {}:{}",
                    snapshot.ancestor_agent.as_str(),
                    snapshot.ancestor_session_id
                ),
                thread_id: Some(record.thread_id.clone()),
                thread_stage_id: None,
                issue_id: None,
                agent: Some(snapshot.ancestor_agent),
                session_id: Some(snapshot.ancestor_session_id.clone()),
                file_path: lookup_session_file_path(
                    current_thread,
                    snapshot.ancestor_agent,
                    &snapshot.ancestor_session_id,
                ),
                ancestor_index: Some(snapshot.ancestor_index),
            },
        );
    }

    sources
}

struct SessionSourceContext<'a> {
    current_thread: Option<&'a ThreadInfo>,
    history_snapshots: &'a [SessionHistorySnapshotRecord],
}

fn push_session_source(
    sources: &mut Vec<ThreadWorkSnapshotSourceRef>,
    seen: &mut HashSet<String>,
    kind: &str,
    thread_stage_id: &str,
    thread_id: &str,
    session_ref: &serde_json::Value,
    context: &SessionSourceContext<'_>,
) {
    let SessionSourceContext {
        current_thread,
        history_snapshots,
    } = *context;
    let Some(agent_raw) = session_ref.get("agent").and_then(|value| value.as_str()) else {
        return;
    };
    let Some(agent) = Agent::from_db_str(agent_raw) else {
        return;
    };
    let Some(session_id) = session_ref
        .get("sessionId")
        .or_else(|| session_ref.get("session_id"))
        .and_then(|value| value.as_str())
    else {
        return;
    };
    let title = session_ref
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or(session_id)
        .to_string();
    let file_path = session_ref
        .get("filePath")
        .or_else(|| session_ref.get("file_path"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
        .or_else(|| lookup_session_file_path(current_thread, agent, session_id));
    let ancestor_index = history_snapshots
        .iter()
        .find(|snapshot| {
            snapshot.ancestor_agent == agent && snapshot.ancestor_session_id == session_id
        })
        .map(|snapshot| snapshot.ancestor_index);
    push_snapshot_source(
        sources,
        seen,
        ThreadWorkSnapshotSourceRef {
            kind: kind.to_string(),
            id: format!("{}:{}", agent.as_str(), session_id),
            label: title,
            thread_id: Some(thread_id.to_string()),
            thread_stage_id: (!thread_stage_id.is_empty()).then(|| thread_stage_id.to_string()),
            issue_id: None,
            agent: Some(agent),
            session_id: Some(session_id.to_string()),
            file_path,
            ancestor_index,
        },
    );
}

fn lookup_session_file_path(
    thread: Option<&ThreadInfo>,
    agent: Agent,
    session_id: &str,
) -> Option<String> {
    let thread = thread?;
    for session in &thread.sessions {
        if session.agent == agent && session.id == session_id && !session.file_path.is_empty() {
            return Some(session.file_path.clone());
        }
    }
    for stage in &thread.stages {
        for session in &stage.sessions {
            if session.agent == agent && session.id == session_id && !session.file_path.is_empty() {
                return Some(session.file_path.clone());
            }
        }
    }
    None
}

fn push_snapshot_source(
    sources: &mut Vec<ThreadWorkSnapshotSourceRef>,
    seen: &mut HashSet<String>,
    source: ThreadWorkSnapshotSourceRef,
) {
    let key = format!("{}:{}", source.kind, source.id);
    if seen.insert(key) {
        sources.push(source);
    }
}

#[tauri::command]
fn get_thread_work_snapshot_sources(
    child_agent: Agent,
    child_session_id: String,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Option<ThreadWorkSnapshotSourcesResult>, String> {
    let record = store
        .get_thread_work_snapshot(child_agent, &child_session_id)
        .map_err(|e| e.to_string())?;
    let Some(record) = record else {
        return Ok(None);
    };
    let snapshot: serde_json::Value =
        serde_json::from_str(&record.snapshot_json).map_err(|e| e.to_string())?;
    let history_snapshots = store
        .get_session_history_snapshots(record.child_agent, &record.child_session_id)
        .map_err(|e| e.to_string())?;
    let current_thread = store.get_thread_work_state(&record.thread_id).ok();
    Ok(Some(ThreadWorkSnapshotSourcesResult {
        child_agent: record.child_agent,
        child_session_id: record.child_session_id.clone(),
        thread_id: record.thread_id.clone(),
        stage_id: record.stage_id.clone(),
        sources: build_thread_work_snapshot_sources(
            &record,
            &snapshot,
            current_thread.as_ref(),
            &history_snapshots,
        ),
    }))
}

#[cfg(test)]
mod ancestor_tests {
    use super::*;
    use crate::agents::runtime::types::AcpProtocolMessage;
    use crate::agents::sources::types::{HistoryAcpMessage, SourceLocation};
    use crate::turns::{
        history_assistant_message, history_session_update_message, history_tool_call_message,
        history_tool_result_message, history_user_message,
    };

    fn row(message: AcpProtocolMessage, timestamp: Option<i64>) -> HistoryAcpMessage {
        HistoryAcpMessage {
            message,
            timestamp,
            location: SourceLocation::file("/tmp/session.jsonl"),
            synthetic: true,
        }
    }

    fn session(
        agent: Agent,
        id: &str,
        forked_from_agent: Option<Agent>,
        forked_from_id: Option<&str>,
        file_path: &str,
    ) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            agent,
            forked_from_agent,
            forked_from_id: forked_from_id.map(String::from),
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(1),
            updated_at: Some(1),
            message_count: 1,
            rename_title: None,
            title: None,
            first_user_message: None,
            file_path: file_path.to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            subagents: Vec::new(),
        }
    }

    #[test]
    fn session_ancestors_from_db_follows_multihop_agent_lineage() {
        let root = session(
            Agent::Gemini,
            "root",
            None,
            None,
            "/tmp/gemini/project/chats/session-root.jsonl",
        );
        let middle = session(
            Agent::Claude,
            "middle",
            Some(Agent::Gemini),
            Some("root"),
            "/tmp/claude/middle.jsonl",
        );
        let child = session(
            Agent::Codex,
            "child",
            Some(Agent::Claude),
            Some("middle"),
            "/tmp/codex/child.jsonl",
        );

        let chain = session_ancestors_from_db(
            Agent::Codex,
            "child",
            &[child, middle.clone(), root.clone()],
        );

        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].agent, Agent::Gemini);
        assert_eq!(chain[0].id, "root");
        assert_eq!(chain[0].file_path, root.file_path);
        assert_eq!(chain[1].agent, Agent::Claude);
        assert_eq!(chain[1].id, "middle");
        assert_eq!(chain[1].file_path, middle.file_path);
    }

    #[test]
    fn session_history_turns_emit_acp_like_blocks_and_tools() {
        let messages = vec![
            row(
                history_user_message(
                    "review\n[file: __sessio_attachment__:spec.md|file:///tmp/spec.md]",
                    Some(10),
                ),
                Some(10),
            ),
            row(
                history_tool_call_message(
                    Some("tool-1".to_string()),
                    "Read",
                    serde_json::json!({ "path": "spec.md" }),
                    Some(20),
                ),
                Some(20),
            ),
            row(
                history_tool_result_message(
                    Some("tool-1".to_string()),
                    serde_json::Value::String("contents".to_string()),
                    Some(21),
                ),
                Some(21),
            ),
            row(
                history_session_update_message(
                    "file_edit",
                    serde_json::json!({ "edits": [] }),
                    Some(22),
                ),
                Some(22),
            ),
            row(history_assistant_message("done", Some(30)), Some(30)),
        ];
        let turns = turns::session_history_turns_from_acp_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_id, "history-turn-0");
        assert_eq!(turns[0].blocks.len(), 4);
        assert_eq!(turns[0].tools.len(), 1);
        assert_eq!(turns[0].blocks[0].kind, "user");
        assert_eq!(turns[0].blocks[0].blocks[1].kind, "resource");
        assert_eq!(turns[0].blocks[1].kind, "tool");
        assert_eq!(turns[0].blocks[2].kind, "sessionUpdate");
        assert_eq!(turns[0].blocks[2].update_type.as_deref(), Some("file_edit"));
        assert_eq!(turns[0].blocks[3].kind, "assistant");
        assert_eq!(turns[0].blocks[3].blocks[0].text.as_deref(), Some("done"));
        assert_eq!(turns[0].tools[0].tool_id, "tool-1");
        assert_eq!(
            turns[0].tools[0].raw_output,
            serde_json::Value::String("contents".to_string())
        );
    }

    #[test]
    fn session_history_result_serializes_turn_envelope_without_legacy_messages() {
        let result = SessionHistoryResult {
            message_count: 1,
            indexed_through: Some(10),
            turns: turns::session_history_turns_from_acp_messages(&[row(
                history_user_message(
                    "review\n[file: __sessio_attachment__:spec.md|file:///tmp/spec.md]",
                    Some(10),
                ),
                Some(10),
            )]),
        };

        let value = serde_json::to_value(&result).unwrap();
        assert!(value.get("messages").is_none());
        assert_eq!(value["messageCount"], 1);
        assert_eq!(value["indexedThrough"], 10);
        assert_eq!(value["turns"][0]["blocks"][0]["kind"], "user");
        assert_eq!(
            value["turns"][0]["blocks"][0]["blocks"][1]["type"],
            "resource"
        );
    }
}

#[tauri::command]
fn rebuild_session_index(indexer: State<'_, IndexerHandle>) -> Result<(), String> {
    indexer
        .submit(IndexTask::FullRebuild)
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
enum SessionScope {
    All,
    Agent { agent: Agent },
    Project { key: String },
}

#[tauri::command]
fn remove_session_files(session: SessionInfo) -> Result<(), String> {
    remove_session_files_inner(session).map_err(|e| e.to_string())
}

#[tauri::command]
fn remove_sessions_by_scope(
    scope: SessionScope,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    let sessions = store.list_sessions().map_err(|e| e.to_string())?;
    for session in sessions
        .iter()
        .filter(|s| s.available && !is_subagent_only(s) && matches_scope(&scope, s))
    {
        remove_session_files_inner(session.clone()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn is_subagent_only(session: &SessionInfo) -> bool {
    session.archived && session.message_count == 0 && !session.subagents.is_empty()
}

fn matches_scope(scope: &SessionScope, session: &SessionInfo) -> bool {
    match scope {
        SessionScope::All => true,
        SessionScope::Agent { agent } => session.agent == *agent,
        SessionScope::Project { key } => {
            let session_key = session
                .project_path
                .clone()
                .unwrap_or_else(|| format!("__unknown__:{}", session.agent.as_str()));
            session_key == *key
        }
    }
}

fn remove_session_files_inner(session: SessionInfo) -> anyhow::Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
    let removed_root = home.join(".sessio").join("removed-sessions");

    if session.agent == Agent::Gemini {
        if crate::agents::sources::gemini::parser::remove_session_from_logs(
            Path::new(&session.file_path),
            &session.id,
            &home,
            &removed_root,
        )? {
            for subagent in &session.subagents {
                let _ = crate::agents::sources::gemini::parser::remove_session_from_logs(
                    Path::new(&subagent.file_path),
                    &subagent.id,
                    &home,
                    &removed_root,
                )?;
            }
        }
        return Ok(());
    }

    move_session_file(&session.file_path, &home, &removed_root)?;

    for subagent in &session.subagents {
        move_session_file(&subagent.file_path, &home, &removed_root)?;
        move_claude_subagent_meta_file(&subagent.file_path, &home, &removed_root)?;
    }

    Ok(())
}

fn move_claude_subagent_meta_file(
    file_path: &str,
    home: &Path,
    removed_root: &Path,
) -> anyhow::Result<bool> {
    if file_path.is_empty() {
        return Ok(false);
    }
    let meta_path = PathBuf::from(file_path).with_extension("meta.json");
    move_session_file(&meta_path.to_string_lossy(), home, removed_root)
}

fn move_session_file(file_path: &str, home: &Path, removed_root: &Path) -> anyhow::Result<bool> {
    if file_path.is_empty() {
        return Ok(false);
    }
    let src = PathBuf::from(file_path);
    if !src.exists() {
        return Ok(false);
    }
    if !src.is_file() {
        anyhow::bail!("session path is not a file: {}", src.display());
    }

    let relative = src
        .strip_prefix(home)
        .map_err(|_| anyhow::anyhow!("session file is outside home: {}", src.display()))?;
    let dst = removed_root.join(relative);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let dst = available_removed_path(dst);
    move_file(&src, &dst)?;
    Ok(true)
}

fn available_removed_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "session".to_string());
    for i in 1.. {
        let candidate = parent.join(format!("{file_name}.{i}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn move_file(src: &Path, dst: &Path) -> anyhow::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            std::fs::copy(src, dst).map_err(|copy_err| {
                anyhow::anyhow!(
                    "move {} to {} failed: rename: {}; copy fallback: {}",
                    src.display(),
                    dst.display(),
                    rename_err,
                    copy_err
                )
            })?;
            std::fs::remove_file(src).map_err(|remove_err| {
                let _ = std::fs::remove_file(dst);
                anyhow::anyhow!(
                    "remove original after copying {} to {} failed: {}",
                    src.display(),
                    dst.display(),
                    remove_err
                )
            })?;
            Ok(())
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct IndexStatus {
    phase: indexer::IndexPhase,
    last_error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMemorySearchResult {
    title: Option<String>,
    snippet: Option<String>,
    score: Option<f64>,
    record_id: Option<String>,
    artifact_uri: Option<String>,
    raw: serde_json::Value,
}

#[tauri::command]
fn get_index_status(indexer: State<'_, IndexerHandle>) -> IndexStatus {
    let s = indexer.status();
    IndexStatus {
        phase: s.phase,
        last_error: s.last_error,
    }
}

#[tauri::command]
fn get_memory_backend_status(
    store: State<'_, Arc<dyn MemoryStore>>,
) -> Result<MemoryBackendStatus, String> {
    if config::load_config()
        .map_err(|e| e.to_string())?
        .memory
        .is_none()
    {
        return Ok(MemoryBackendStatus {
            backend: "memory".to_string(),
            available: false,
            error: Some("memory is not configured".to_string()),
            details: None,
        });
    }
    let service = MemoryService::new(
        store.inner().clone(),
        Arc::new(crate::agents::sources::builtin_agent_sources()),
    )
    .map_err(|e| e.to_string())?;
    Ok(service.backend_status())
}

#[tauri::command]
fn search_project_memory(
    store: State<'_, Arc<dyn MemoryStore>>,
    project_key: String,
    query: String,
) -> Result<Vec<ProjectMemorySearchResult>, String> {
    search_project_memory_inner(project_key, query, Some(store.inner().as_ref()))
        .map_err(|e| e.to_string())
}

fn search_project_memory_inner(
    project_key: String,
    query: String,
    store: Option<&dyn MemoryStore>,
) -> anyhow::Result<Vec<ProjectMemorySearchResult>> {
    let config = config::load_memory_config()?;
    let memory_project_key = resolve_memory_project_key(&project_key, store)?;
    let options = QmdOptions {
        binary: config.qmd.binary.clone(),
        index: config.qmd.index.clone(),
        install_command: config.qmd.install_command.clone(),
    };
    let result = if config.qmd.auto_embed {
        query_project(&options, &memory_project_key, &query)
    } else {
        search_project(&options, &memory_project_key, &query)
    }?;
    Ok(project_memory_results(&result.raw))
}

fn resolve_memory_project_key(
    project_filter_key: &str,
    store: Option<&dyn MemoryStore>,
) -> anyhow::Result<String> {
    if let Some(store) = store {
        if !store.list_project_records(project_filter_key)?.is_empty() {
            return Ok(project_filter_key.to_string());
        }
    }
    let slug = crate::agents::sources::shared::convert::project_key_for_path_or_name(
        Some(project_filter_key),
        None,
    );
    if let Some(store) = store {
        if !store.list_project_records(&slug)?.is_empty() {
            return Ok(slug);
        }
    }
    Ok(slug)
}

fn project_memory_results(raw: &serde_json::Value) -> Vec<ProjectMemorySearchResult> {
    let mut out = Vec::new();
    collect_project_memory_results(raw, &mut out);
    out
}

fn collect_project_memory_results(
    raw: &serde_json::Value,
    out: &mut Vec<ProjectMemorySearchResult>,
) {
    match raw {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_project_memory_results(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            let title = first_json_string(map, &["title", "name", "heading"]);
            let snippet = first_json_string(map, &["snippet", "text", "content", "preview"]);
            let artifact_uri =
                first_json_string(map, &["path", "file", "filePath", "filepath", "source"]);
            let record_id = first_json_string(map, &["recordId", "record_id", "id"])
                .and_then(record_id_from_text)
                .or_else(|| artifact_uri.clone().and_then(record_id_from_text));
            if title.is_some() || snippet.is_some() || artifact_uri.is_some() || record_id.is_some()
            {
                out.push(ProjectMemorySearchResult {
                    title,
                    snippet,
                    score: first_json_number(map, &["score", "rank", "similarity"]),
                    record_id,
                    artifact_uri,
                    raw: raw.clone(),
                });
            }
            for key in ["results", "hits", "documents", "items", "matches"] {
                if let Some(child) = map.get(key) {
                    collect_project_memory_results(child, out);
                }
            }
        }
        _ => {}
    }
}

fn first_json_string(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(|value| value.as_str())
            .map(str::to_string)
    })
}

fn first_json_number(
    map: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<f64> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(|value| value.as_f64()))
}

fn record_id_from_text(text: String) -> Option<String> {
    let path = Path::new(&text);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(&text);
    stem.starts_with("sessio-").then(|| stem.to_string())
}

#[tauri::command]
fn get_session_history(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
    _store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SessionHistoryResult, String> {
    read_session_history_result_from_source(agent, &file_path, session_id.as_deref())
        .map_err(|e| e.to_string())
}

pub fn read_session_history_result(
    agent: Agent,
    file_path: &str,
    session_id: Option<&str>,
) -> anyhow::Result<SessionHistoryResult> {
    read_session_history_result_from_source(agent, file_path, session_id)
}

fn read_session_history_result_from_source(
    agent: Agent,
    file_path: &str,
    session_id: Option<&str>,
) -> anyhow::Result<SessionHistoryResult> {
    let path = PathBuf::from(file_path);
    if file_path.is_empty() || !path.exists() {
        anyhow::bail!(
            "Session file no longer exists (likely cleaned by {}): {}",
            match agent {
                Agent::AstraPi => "Astra Pi",
                Agent::Codex => "Codex",
                Agent::Claude => "Claude Code",
                Agent::Gemini => "Gemini",
            },
            if file_path.is_empty() {
                "<empty>"
            } else {
                file_path
            }
        );
    }
    let (messages, message_count) = match agent {
        Agent::AstraPi => {
            let sid = session_id.unwrap_or_default();
            let rows =
                crate::agents::sources::pi::parser::read_history_acp_messages_with_locations(
                    &path, sid,
                )?;
            let count = rows.len();
            (rows, count)
        }
        Agent::Codex => {
            let rows =
                crate::agents::sources::codex::parser::read_history_acp_messages_with_locations(
                    &path,
                )?;
            let count = count_source_lines(&rows);
            (rows, count)
        }
        Agent::Claude => {
            let rows =
                crate::agents::sources::claude::parser::read_history_acp_messages_with_locations(
                    &path,
                )?;
            let count = count_source_lines(&rows);
            (rows, count)
        }
        Agent::Gemini => {
            let sid = session_id.unwrap_or_default();
            let rows =
                crate::agents::sources::gemini::parser::read_history_acp_messages_with_locations(
                    &path, sid,
                )?;
            let count = rows.len();
            (rows, count)
        }
    };
    let indexed_through = latest_history_event_timestamp(&messages);
    let turns = turns::session_history_turns_from_acp_messages(&messages);
    Ok(SessionHistoryResult {
        message_count,
        indexed_through,
        turns,
    })
}

fn latest_history_event_timestamp(
    messages: &[crate::agents::sources::types::HistoryAcpMessage],
) -> Option<i64> {
    messages.iter().filter_map(|event| event.timestamp).max()
}

#[tauri::command]
fn update_session_history_count(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
    message_count: usize,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    store
        .update_message_count(agent, session_id.as_deref(), &file_path, message_count)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_pending_session(
    session: SessionInfo,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<(), String> {
    let scope = session.file_path.clone();
    store
        .upsert_session(&scope, &session)
        .map_err(|e| e.to_string())?;
    app.emit("sessions_index_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn update_session_rename_title(
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

fn count_source_lines(rows: &[crate::agents::sources::types::HistoryAcpMessage]) -> usize {
    let mut lines = HashSet::new();
    for row in rows {
        if let Some(line) = row.location.line_start {
            lines.insert(line);
        }
    }
    lines.len()
}

#[tauri::command]
fn read_local_image_data_url(path: String) -> Result<String, String> {
    use base64::Engine;

    let path_buf = PathBuf::from(&path);
    if !path_buf.is_absolute() {
        return Err("Only absolute image paths can be loaded".to_string());
    }
    let mime = local_image_mime(&path_buf).ok_or_else(|| "Unsupported image type".to_string())?;
    let meta = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
    const MAX_IMAGE_BYTES: u64 = 24 * 1024 * 1024;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err("Image is too large to preview".to_string());
    }
    let bytes = std::fs::read(&path_buf).map_err(|e| e.to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

#[tauri::command]
fn read_local_text_file(path: String) -> Result<String, String> {
    let path_buf = PathBuf::from(&path);
    if !path_buf.is_absolute() {
        return Err("Only absolute file paths can be loaded".to_string());
    }
    let _mime =
        text_file_mime(&path_buf).ok_or_else(|| "Unsupported text file type".to_string())?;
    let meta = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Path is not a file".to_string());
    }
    const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
    if meta.len() > MAX_TEXT_BYTES {
        return Err("File is too large to preview".to_string());
    }
    std::fs::read_to_string(&path_buf).map_err(|e| e.to_string())
}

fn local_image_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        Some("svg") => Some("image/svg+xml"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

fn text_file_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("txt") => Some("text/plain"),
        Some("md") | Some("markdown") => Some("text/markdown"),
        Some("json") | Some("jsonl") => Some("application/json"),
        Some("yaml") | Some("yml") => Some("application/yaml"),
        Some("toml") => Some("application/toml"),
        Some("xml") => Some("application/xml"),
        Some("csv") => Some("text/csv"),
        Some("ts") | Some("tsx") | Some("js") | Some("jsx") | Some("mjs") | Some("cjs")
        | Some("py") | Some("rs") | Some("go") | Some("java") | Some("kt") | Some("swift")
        | Some("rb") | Some("php") | Some("css") | Some("scss") | Some("sass") | Some("less")
        | Some("html") | Some("htm") | Some("sh") | Some("zsh") | Some("bash") | Some("sql")
        | Some("c") | Some("h") | Some("cpp") | Some("hpp") | Some("cs") | Some("lua")
        | Some("pl") | Some("r") | Some("ex") | Some("exs") | Some("erl") | Some("clj")
        | Some("scala") | Some("dart") | Some("vue") | Some("svelte") | Some("dockerfile")
        | Some("gitignore") | Some("env") => Some("text/plain"),
        _ => None,
    }
}

#[tauri::command]
fn write_cross_prompt(session_id: String, content: String) -> Result<String, String> {
    let safe_id: String = session_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir = dirs::home_dir()
        .ok_or_else(|| "home directory not found".to_string())?
        .join(".sessio")
        .join("projects")
        .join(".cross-context");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("sessio-cross-context-{}-{}.md", safe_id, ts));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(content.as_bytes())
        })
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn get_agent_runtime_status(
    agent: Agent,
    runtime: State<'_, RuntimeManager>,
) -> Result<RuntimeStatus, String> {
    Ok(runtime.status(agent))
}

#[tauri::command]
fn list_runtime_agents(
    cache: State<'_, RuntimeAgentsCache>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<models::RuntimeAgentMetadata>, String> {
    db_runtime_agents(store.inner(), &cache.get())
}

#[tauri::command]
fn get_last_runtime_agent_selection(
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Option<RuntimeAgentSelectionDto>, String> {
    store
        .get_last_runtime_agent_selection()
        .map(|selection| selection.map(runtime_agent_selection_to_dto))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_last_runtime_agent_selection(
    req: SetRuntimeAgentSelectionRequest,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<RuntimeAgentSelectionDto, String> {
    store
        .set_last_runtime_agent_selection(
            req.agent,
            req.model.as_deref(),
            req.effort.as_deref(),
            req.permission_mode.as_deref(),
        )
        .map(runtime_agent_selection_to_dto)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_debug_config() -> Result<config::DebugConfig, String> {
    config::load_config()
        .map(|config| config.debug)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_network_config() -> Result<config::NetworkConfig, String> {
    network::load_network_config().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_network_config(config: config::NetworkConfig) -> Result<config::NetworkConfig, String> {
    network::save_network_config(config).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_im_bridge_config() -> Result<im_bridge::ImBridgeConfig, String> {
    im_bridge::load_config_or_default().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_im_bridge_config(
    config: im_bridge::ImBridgeConfig,
    bridge: State<'_, im_bridge::ImBridgeService>,
) -> Result<im_bridge::ImBridgeConfig, String> {
    im_bridge::save_config(&config).map_err(|e| e.to_string())?;
    bridge.update_config(config.clone());
    Ok(config)
}

#[tauri::command]
fn detect_telegram_user_ids(
    bot_token: String,
    api_base: Option<String>,
) -> Result<Vec<i64>, String> {
    im_bridge::detect_telegram_user_ids(&bot_token, api_base.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
async fn test_telegram_bot_connection(
    bot_token: String,
    api_base: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        im_bridge::test_telegram_bot_connection(&bot_token, api_base.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn test_discord_bot_connection(
    bot_token: String,
    api_base: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        im_bridge::test_discord_bot_connection(&bot_token, api_base.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn update_runtime_agent_preferences(
    req: UpdateRuntimeAgentPreferencesRequest,
    app: AppHandle,
    cache: State<'_, RuntimeAgentsCache>,
    indexer: State<'_, IndexerHandle>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<models::RuntimeAgentMetadata, String> {
    let models = runtime_option_inputs_to_metadata(req.models);
    let efforts = runtime_option_inputs_to_metadata(req.efforts);
    let permission_modes = runtime_option_inputs_to_metadata(req.permission_modes);
    store
        .update_builtin_agent_preferences(
            req.agent,
            AgentPreferencesPatch {
                display_name: req.display_name.as_deref(),
                enabled: req.enabled,
                order: req.order,
                ai_provider: req.ai_provider.as_deref(),
                ai_providers: req.ai_providers.as_deref(),
                model: req.model.as_deref(),
                effort: req.effort.as_deref(),
                permission_mode: req.permission_mode.as_deref(),
                models: models.as_deref(),
                efforts: efforts.as_deref(),
                permission_modes: permission_modes.as_deref(),
            },
        )
        .map_err(|e| e.to_string())?;
    let agents = db_runtime_agents(store.inner(), &cache.get())?;
    let updated = agents
        .iter()
        .find(|metadata| metadata.agent == req.agent)
        .cloned()
        .ok_or_else(|| format!("runtime agent is not configured: {}", req.agent.as_str()))?;
    indexer
        .refresh_enabled_agents(store.inner().as_ref())
        .map_err(|e| e.to_string())?;
    if let Some(watcher) = app.try_state::<watch::WatcherHandle>() {
        watcher.refresh().map_err(|e| e.to_string())?;
    }
    app.emit("runtime_agents_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

#[tauri::command]
fn start_agent_session(
    mut req: StartAgentSession,
    store: State<'_, Arc<dyn SessionStore>>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    hydrate_start_request_from_db(&mut req, store.inner()).map_err(|e| e.to_string())?;
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn fork_agent_session(
    mut req: StartAgentSession,
    store: State<'_, Arc<dyn SessionStore>>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    if req
        .source_session_id
        .as_deref()
        .unwrap_or("")
        .trim()
        .is_empty()
    {
        return Err("source_session_id is required".to_string());
    }
    hydrate_start_request_from_db(&mut req, store.inner()).map_err(|e| e.to_string())?;
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn ensure_agent_runtime_session(
    mut req: EnsureAgentRuntimeSession,
    store: State<'_, Arc<dyn SessionStore>>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    hydrate_ensure_request_from_db(&mut req, store.inner()).map_err(|e| e.to_string())?;
    runtime.ensure_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_agent_session(
    agent: Agent,
    runtime_session_id: String,
    workspace_path: String,
    agent_runtime_session_id: Option<String>,
    source_agent: Option<Agent>,
    store: State<'_, Arc<dyn SessionStore>>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    let mut req = EnsureAgentRuntimeSession {
        agent,
        sessio_runtime_session_id: runtime_session_id,
        workspace_path,
        agent_runtime_session_id,
        source_agent,
        options: Default::default(),
    };
    hydrate_ensure_request_from_db(&mut req, store.inner()).map_err(|e| e.to_string())?;
    runtime.ensure_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn send_agent_input(
    sessio_runtime_session_id: String,
    input: AgentInput,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentTurnHandle, String> {
    log::info!(
        "[sessio-runtime:backend:send] session={} text={:?}",
        sessio_runtime_session_id,
        input.text
    );
    runtime
        .send_input(&sessio_runtime_session_id, input)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_agent_turn(
    sessio_runtime_session_id: String,
    turn_id: String,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .cancel_turn(&sessio_runtime_session_id, &turn_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_agent_session_config_option(
    sessio_runtime_session_id: String,
    change: AgentSessionConfigChange,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .set_config_option(&sessio_runtime_session_id, change)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn respond_agent_permission(
    sessio_runtime_session_id: String,
    request_id: String,
    option_id: String,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .respond_permission(&sessio_runtime_session_id, &request_id, option_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_astra_run(
    req: CreateAstraRunRequest,
    astra: State<'_, AstraService>,
) -> Result<AstraHandle, String> {
    astra.create_astra_run(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel_astra_run(
    req: CancelAstraRunRequest,
    astra: State<'_, AstraService>,
) -> Result<AstraHandle, String> {
    astra.cancel_astra_run(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_astra_runs(
    thread_id: String,
    astra: State<'_, AstraService>,
) -> Result<Vec<AstraHandle>, String> {
    astra.list_astra_runs(&thread_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_astra_run(run_id: String, astra: State<'_, AstraService>) -> Result<AstraHandle, String> {
    astra.get_astra_run(&run_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_window_appearance(window: tauri::Window, theme: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::{class, msg_send, runtime::AnyObject};
        use objc2_foundation::NSString;

        let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())?;
        if ns_window_ptr.is_null() {
            return Err("ns_window is null".into());
        }
        let name = NSString::from_str(if theme == "dark" {
            "NSAppearanceNameDarkAqua"
        } else {
            "NSAppearanceNameAqua"
        });
        unsafe {
            let appearance: *mut AnyObject =
                msg_send![class!(NSAppearance), appearanceNamed: &*name];
            if appearance.is_null() {
                return Err(format!("unknown NSAppearance name for theme '{}'", theme));
            }
            let ns_window = ns_window_ptr as *mut AnyObject;
            let _: () = msg_send![ns_window, setAppearance: appearance];
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, theme);
    }
    Ok(())
}

#[tauri::command]
fn get_system_appearance() -> String {
    current_system_appearance()
}

fn current_system_appearance() -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2::{class, msg_send, runtime::AnyObject};
        use objc2_foundation::NSString;

        // Once we override the window's NSAppearance, webview matchMedia stops
        // reflecting the system. Read AppleInterfaceStyle directly so the
        // frontend can resolve "system" mode accurately. The key is absent in
        // light mode and equals "Dark" in dark mode.
        unsafe {
            let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
            if defaults.is_null() {
                return "light".into();
            }
            let key = NSString::from_str("AppleInterfaceStyle");
            let value: *mut AnyObject = msg_send![defaults, stringForKey: &*key];
            if value.is_null() {
                "light".into()
            } else {
                "dark".into()
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "light".into()
    }
}

// macOS won't fire prefers-color-scheme change events into the webview once
// we've pinned the NSWindow's appearance, so we hook into the system-wide
// AppleInterfaceThemeChangedNotification (posted by macOS whenever the
// effective appearance flips, including the automatic sunset schedule) and
// push the new value down to the frontend. Other platforms don't pin
// appearance, so matchMedia in the webview already works there.
#[cfg(target_os = "macos")]
mod appearance_observer {
    use std::sync::OnceLock;

    use objc2::{
        class, msg_send,
        runtime::{AnyClass, AnyObject, ClassBuilder, Sel},
        sel,
    };
    use objc2_foundation::NSString;
    use tauri::{AppHandle, Emitter};

    static HANDLE: OnceLock<AppHandle> = OnceLock::new();
    static OBSERVER_CLASS: OnceLock<&'static AnyClass> = OnceLock::new();

    extern "C" fn theme_changed(_this: &AnyObject, _cmd: Sel, _notif: *mut AnyObject) {
        if let Some(handle) = HANDLE.get() {
            let value = super::current_system_appearance();
            let _ = handle.emit("system_appearance_changed", value);
        }
    }

    pub fn install(handle: AppHandle) {
        if HANDLE.set(handle).is_err() {
            return;
        }

        let cls = OBSERVER_CLASS.get_or_init(|| {
            let mut builder = ClassBuilder::new(c"SessioAppearanceObserver", class!(NSObject))
                .expect("SessioAppearanceObserver name already registered");
            unsafe {
                let imp: extern "C" fn(_, _, _) = theme_changed;
                builder.add_method(sel!(themeChanged:), imp);
            }
            builder.register()
        });

        unsafe {
            // `new` returns a +1 retained instance. We deliberately drop the
            // pointer without releasing so the observer lives for the lifetime
            // of the app (NSDistributedNotificationCenter holds it weakly).
            let observer: *mut AnyObject = msg_send![*cls, new];
            let center: *mut AnyObject =
                msg_send![class!(NSDistributedNotificationCenter), defaultCenter];
            let name = NSString::from_str("AppleInterfaceThemeChangedNotification");
            let _: () = msg_send![
                center,
                addObserver: observer,
                selector: sel!(themeChanged:),
                name: &*name,
                object: std::ptr::null::<AnyObject>(),
            ];
        }
    }
}

fn install_appearance_observer(handle: AppHandle) {
    #[cfg(target_os = "macos")]
    appearance_observer::install(handle);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = handle;
    }
}

#[cfg(target_os = "macos")]
fn set_window_alpha(window: &WebviewWindow, alpha: f64) -> Result<(), String> {
    use objc2::{msg_send, runtime::AnyObject};

    let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())?;
    if ns_window_ptr.is_null() {
        return Err("ns_window is null".into());
    }
    unsafe {
        let ns_window = ns_window_ptr as *mut AnyObject;
        let _: () = msg_send![ns_window, setAlphaValue: alpha];
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn animate_window_alpha(window: WebviewWindow, from: f64, to: f64, duration_ms: u64) {
    const STEPS: u64 = 10;
    thread::spawn(move || {
        for step in 0..=STEPS {
            let t = step as f64 / STEPS as f64;
            let eased = 1.0 - (1.0 - t).powi(3);
            let alpha = from + (to - from) * eased;
            let w = window.clone();
            let _ = window.run_on_main_thread(move || {
                let _ = set_window_alpha(&w, alpha);
            });
            thread::sleep(Duration::from_millis(duration_ms / STEPS));
        }
    });
}

fn hide_main_window(window: WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        animate_window_alpha(window.clone(), 1.0, 0.0, 140);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            let w = window.clone();
            let _ = window.run_on_main_thread(move || {
                let _ = w.hide();
                let _ = set_window_alpha(&w, 1.0);
            });
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = window.hide();
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        #[cfg(target_os = "macos")]
        let was_visible = w.is_visible().unwrap_or(false);
        #[cfg(target_os = "macos")]
        if !was_visible {
            let _ = set_window_alpha(&w, 0.0);
        }
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        #[cfg(target_os = "macos")]
        if !was_visible {
            animate_window_alpha(w, 0.0, 1.0, 170);
        }
    }
}

#[tauri::command]
fn reveal_main_window(app: AppHandle) {
    show_main_window(&app);
}

/// Expose the running app binary through a stable path (~/.sessio/bin/sessio)
/// so agents working inside a project can invoke the Sessio CLI without knowing
/// where the app was installed. Best-effort: failures only warn.
fn link_cli_binary(sessio_home: &std::path::Path) {
    let current = match std::env::current_exe() {
        Ok(path) => path,
        Err(e) => {
            log::warn!("link_cli_binary: current_exe failed: {e}");
            return;
        }
    };
    let bin_dir = sessio_home.join("bin");
    if let Err(e) = std::fs::create_dir_all(&bin_dir) {
        log::warn!("link_cli_binary: create {bin_dir:?} failed: {e}");
        return;
    }
    #[cfg(unix)]
    {
        let link = bin_dir.join("sessio");
        if let Ok(existing) = std::fs::read_link(&link) {
            if existing == current {
                return;
            }
        }
        // Replace stale links/files pointing at a previous install.
        if link.exists() || std::fs::symlink_metadata(&link).is_ok() {
            let _ = std::fs::remove_file(&link);
        }
        if let Err(e) = std::os::unix::fs::symlink(&current, &link) {
            log::warn!("link_cli_binary: link {link:?} -> {current:?} failed: {e}");
        }
    }

    #[cfg(windows)]
    {
        let script = windows_cli_shim(&current);
        let cmd_path = bin_dir.join("sessio.cmd");
        let bare_path = bin_dir.join("sessio");
        write_cli_shim_if_changed(&cmd_path, &script);
        write_cli_shim_if_changed(&bare_path, &script);
    }
}

#[cfg(windows)]
fn windows_cli_shim(current: &std::path::Path) -> String {
    let exe = current.to_string_lossy();
    format!("@echo off\r\n\"{exe}\" %*\r\n")
}

#[cfg(windows)]
fn write_cli_shim_if_changed(path: &std::path::Path, content: &str) {
    if matches!(std::fs::read_to_string(path), Ok(existing) if existing == content) {
        return;
    }
    if path.exists() || std::fs::symlink_metadata(path).is_ok() {
        let _ = std::fs::remove_file(path);
    }
    if let Err(e) = std::fs::write(path, content) {
        log::warn!("link_cli_binary: write shim {path:?} failed: {e}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    builder
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            shell_env::import_login_shell_env();

            let sessio_home = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("no home dir"))?
                .join(".sessio");
            let data_dir = sessio_home.join("db-data");
            std::fs::create_dir_all(&data_dir).ok();
            link_cli_binary(&sessio_home);
            let db_path = data_dir.join("sessio-index.db");
            let sqlite = Arc::new(SqliteStore::open(&db_path)?);
            sqlite.init()?;
            let inner: Arc<dyn SessionStore> = sqlite.clone();
            let memory_store: Arc<dyn MemoryStore> = sqlite;
            let store: Arc<dyn SessionStore> = Arc::new(CachedStore::new(inner)?);
            let app_config = config::load_config()?;
            network::apply_network_proxy_env(&app_config.network.proxy);
            let runtime = RuntimeManager::new(app.handle().clone());
            app.manage(runtime.clone());
            let indexer_handle = indexer::spawn(
                app.handle().clone(),
                store.clone(),
                memory_store.clone(),
                runtime.clone(),
                app_config.memory.clone(),
            );
            log::info!("indexer spawned");

            polling::spawn_polling(
                store.clone(),
                indexer_handle.clone(),
                std::time::Duration::from_secs(app_config.index.poll_interval_seconds),
            );
            log::info!("polling thread spawned");

            match watch::spawn(store.clone(), indexer_handle.clone()) {
                Ok(handle) => {
                    log::info!("watcher spawned successfully");
                    app.manage(handle);
                }
                Err(e) => log::warn!("watcher failed to start: {e}"),
            }
            app.manage(store.clone());
            app.manage(memory_store.clone());
            app.manage(indexer_handle);
            let runtime_probe_store = store.clone();
            let runtime_agents_cache = RuntimeAgentsCache::default();
            let initial_runtime_agents =
                runtime_agents_from_db(store.clone(), &[]).unwrap_or_default();
            runtime_agents_cache.set(initial_runtime_agents);
            let astra_service =
                AstraService::new(app.handle().clone(), store.clone(), runtime.clone());
            if let Err(error) = astra_service.recover_interrupted_runs() {
                log::warn!("[astra:recover] {error}");
            }
            if let Err(error) = astra_service.watch_runtime_events() {
                log::warn!("[astra:runtime-watch] {error}");
            }
            let pi_session_store = PiAcpSessionStore::new(app.handle().clone(), store.clone());
            if let Err(error) = pi_session_store.watch_runtime_events(runtime.clone()) {
                log::warn!("[pi-acp-session-store] failed to watch runtime events: {error}");
            }
            let im_bridge_config = match im_bridge::load_config_or_default() {
                Ok(config) => config,
                Err(error) => {
                    log::warn!("[im-bridge] failed to load config: {error:#}");
                    im_bridge::ImBridgeConfig::default()
                }
            };
            let im_bridge_service = im_bridge::ImBridgeService::new(
                app.handle().clone(),
                store.clone(),
                runtime.clone(),
                im_bridge_config,
            );
            if let Err(error) = im_bridge_service.start() {
                log::warn!("[im-bridge] failed to start: {error:#}");
            }
            app.manage(im_bridge_service);
            app.manage(pi_session_store);
            app.manage(astra_service);
            app.manage(runtime_agents_cache);
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                match startup_probe_runtime_agents(runtime_probe_store) {
                    Ok(agents) => {
                        let cache = app_handle.state::<RuntimeAgentsCache>();
                        cache.set(agents);
                        let _ = app_handle.emit("runtime_agents_updated", ());
                    }
                    Err(error) => {
                        log::warn!("[sessio-runtime:metadata:start] {error}");
                    }
                }
            });

            install_appearance_observer(app.handle().clone());

            let show = MenuItem::with_id(app, "show", "Show Sessio", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Sessio", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &sep, &quit])?;

            TrayIconBuilder::with_id("main")
                .icon(tauri::include_image!("icons/tray-icon.png"))
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        show_main_window(app);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if let Some(win) = app.get_webview_window("main") {
                #[cfg(not(target_os = "macos"))]
                {
                    let _ = win.set_decorations(false);
                }
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        hide_main_window(w.clone());
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            list_channel_sessions,
            list_process_templates,
            create_process_template,
            update_process_template,
            delete_process_template,
            list_projects,
            add_existing_project,
            create_project,
            create_default_project,
            update_project,
            archive_project,
            list_agents,
            get_astra_config,
            update_astra_config,
            update_agent_preferences,
            list_assistants,
            create_assistant,
            update_assistant,
            delete_assistant,
            list_threads,
            get_thread_work_state,
            get_thread_replay,
            list_thread_index,
            create_thread,
            update_thread,
            delete_thread,
            create_plan_round,
            get_plan_round,
            list_plan_rounds,
            update_plan_task_status,
            complete_plan_task_and_start_next,
            link_plan_task_session,
            list_plan_task_sessions,
            list_project_stages,
            list_process_template_stages,
            create_project_stage,
            update_project_stage,
            update_project_stage_assistants,
            delete_project_stage,
            add_thread_stage,
            update_thread_stage,
            update_thread_stage_state,
            list_thread_stage_issues,
            create_thread_stage_issue,
            update_thread_stage_issue,
            delete_thread_stage_issue,
            update_thread_stage_assistant_agent,
            delete_thread_stage,
            set_thread_stage,
            link_thread_session,
            unlink_thread_session,
            link_stage_session,
            unlink_stage_session,
            list_kanban_items,
            create_kanban_item,
            update_kanban_item,
            update_kanban_item_status,
            delete_kanban_item,
            link_kanban_item_session,
            unlink_kanban_item_session,
            get_session_ancestors,
            get_session_history_snapshots,
            save_session_history_snapshots,
            save_thread_work_snapshot,
            get_thread_work_snapshot,
            get_thread_work_snapshot_sources,
            get_session_history,
            update_session_history_count,
            create_pending_session,
            update_session_rename_title,
            read_local_image_data_url,
            read_local_text_file,
            set_window_appearance,
            get_system_appearance,
            reveal_main_window,
            rebuild_session_index,
            get_index_status,
            get_memory_backend_status,
            search_project_memory,
            write_cross_prompt,
            get_agent_runtime_status,
            list_runtime_agents,
            get_last_runtime_agent_selection,
            set_last_runtime_agent_selection,
            get_debug_config,
            get_network_config,
            update_network_config,
            get_im_bridge_config,
            update_im_bridge_config,
            detect_telegram_user_ids,
            test_telegram_bot_connection,
            test_discord_bot_connection,
            update_runtime_agent_preferences,
            start_agent_session,
            fork_agent_session,
            ensure_agent_runtime_session,
            load_agent_session,
            send_agent_input,
            cancel_agent_turn,
            set_agent_session_config_option,
            respond_agent_permission,
            create_astra_run,
            cancel_astra_run,
            list_astra_runs,
            get_astra_run,
            remove_session_files,
            remove_sessions_by_scope
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = _event {
                show_main_window(_app);
            }
        });
}
