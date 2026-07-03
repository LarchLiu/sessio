pub mod agents;
pub mod app_paths;
pub mod astra;
pub mod cli;
pub mod commands;
pub mod computer_use;
pub mod config;
pub mod config_watch;
pub mod desktop_control;
pub mod file_preview_watch;
pub mod im_bridge;
pub mod indexer;
pub mod mcp;
pub mod memory;
pub mod models;
pub mod network;
pub mod polling;
pub mod prompt_markers;
pub mod scheduled_tasks;
mod screenshot;
pub mod shell_env;
pub mod skills;
pub mod store;
pub mod terminal;
pub mod turns;
pub mod watch;
pub mod work_state_skill_resource;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use std::thread;
#[cfg(any(target_os = "macos", windows, target_os = "linux"))]
use std::time::Duration;
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
};

use agents::runtime::metadata::{
    runtime_agents_from_db, startup_probe_runtime_agents, RuntimeAgentsCache,
};
use agents::runtime::types::{
    AgentInput, AgentSessionConfigChange, AgentSessionHandle, AgentTurnHandle,
    EnsureAgentRuntimeSession, RuntimeStatus, StartAgentSession,
};
use agents::runtime::RuntimeManager;
use app_paths::{
    app_home, cross_context_dir, db_path as default_db_path, paste_cache_dir, projects_dir,
    removed_sessions_dir, session_canvas_dir,
};
use astra::{AstraHandle, AstraService, CancelAstraRunRequest, CreateAstraRunRequest};
use indexer::{IndexTask, IndexerHandle};
use memory::qmd::{query_project, search_project, QmdOptions};
use memory::service::MemoryService;
use memory::{MemoryBackendStatus, MemoryStore};
use models::{
    Agent, AgentAiProviderInfo, AgentCommandsInfo, AgentInfo, AssistantAgentInfo, AstraConfig,
    CanvasBlockKind, CanvasBlockRecord, CanvasBlockSourceType, CanvasContextAnchor,
    CanvasDocumentState, IssueSeverity, IssueStatus, KanbanItem, PlanRoundInfo, PlanRoundMode,
    PlanRoundSource, PlanRoundStatus, PlanTaskInfo, PlanTaskRisk, PlanTaskSessionInfo,
    PlanTaskSessionRole, PlanTaskStatus, ProjectInfo, ProjectStageInfo, RuntimeAgentMetadata,
    SessionHistoryTurn, SessionInfo, StageInfo, StageIssueInfo, StageStatus, ThreadAgentInfo,
    ThreadIndexItemInfo, ThreadInfo, ThreadKind, ThreadReplayInfo,
};
use store::cached::CachedStore;
use store::sqlite::SqliteStore;
use store::{
    AgentPreferencesPatch, AstraConfigPatch, NewPlanRound, NewPlanTask, NewPlanTaskSession,
    PlanTaskStatusPatch, ProjectStagePatch, SessionHistorySnapshotRecord, SessionStore,
    ThreadWorkSnapshotRecord, UpsertCanvasBlockRecord,
};
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_global_shortcut::GlobalShortcutExt;
use terminal::{
    CloseTerminalRequest, CreateTerminalRequest, ResizeTerminalRequest, TerminalService,
    TerminalSessionSummary, WriteTerminalInputRequest,
};

const HISTORY_CACHE_VERSION: i64 = 1;
const THREAD_WORK_SNAPSHOT_VERSION: i64 = 2;

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadsUpdatedPayload {
    project_id: Option<String>,
    thread_id: Option<String>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppshotCapturedPayload {
    path: String,
    shortcut: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppshotPermissionStateDto {
    granted: bool,
    supported: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppshotPermissionStatusDto {
    platform: String,
    requires_permission: bool,
    screenshots: AppshotPermissionStateDto,
    accessibility: AppshotPermissionStateDto,
    can_capture: bool,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppshotPermissionRequiredPayload {
    shortcut: String,
    status: AppshotPermissionStatusDto,
}

#[derive(Default)]
struct AppshotShortcutState {
    registered_shortcut: Mutex<Option<String>>,
    suspended_shortcut: Mutex<Option<String>>,
}

/// How long a command-driven screenshot overlay waits for the user to
/// finish a selection before it gives up.
#[cfg(any(windows, target_os = "linux"))]
const SCREENSHOT_OVERLAY_CAPTURE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Default)]
struct ScreenshotOverlayState {
    sources: Mutex<HashMap<String, ScreenshotOverlaySourceDto>>,
    reveal_main_on_finish: Mutex<HashMap<String, bool>>,
    #[cfg(any(windows, target_os = "linux"))]
    pending_results: Mutex<HashMap<String, ScreenshotOverlayResultSender>>,
    #[cfg(any(windows, target_os = "linux"))]
    completed_results: Mutex<HashMap<String, ScreenshotOverlayCompletion>>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlayWindowDto {
    label: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlayCancelledPayload {
    request_id: String,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComputerUsePointerOverlayReadyPayload {
    label: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlaySourceDto {
    request_id: String,
    source_path: String,
    file_name: String,
    mode: String,
    windows: Vec<ScreenshotOverlayWindowCandidateDto>,
    initial_selection: Option<ScreenshotOverlayInitialSelectionDto>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlayWindowCandidateDto {
    id: String,
    app_name: String,
    title: Option<String>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlayInitialSelectionDto {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlayCompleteRequest {
    request_id: String,
    path: Option<String>,
    cancelled: Option<bool>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeAgentSessionConfigDto {
    agent: Agent,
    adapter_version: String,
    available_commands_json: String,
    config_options_json: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppshotPermissionKind {
    Screenshots,
    Accessibility,
}

impl AppshotPermissionKind {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "screenshots" | "screen_capture" | "screen-capture" => Ok(Self::Screenshots),
            "accessibility" => Ok(Self::Accessibility),
            other => Err(format!("Unknown appshot permission: {other}")),
        }
    }
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

fn emit_appshot_captured(app: &AppHandle, payload: AppshotCapturedPayload) -> Result<(), String> {
    app.emit("appshot_captured", payload)
        .map_err(|e| e.to_string())
}

fn emit_appshot_permission_required(
    app: &AppHandle,
    payload: AppshotPermissionRequiredPayload,
) -> Result<(), String> {
    app.emit("appshot_permission_required", payload)
        .map_err(|e| e.to_string())
}

fn thread_project_id(store: &dyn SessionStore, thread_id: &str) -> Option<String> {
    store
        .get_thread_work_state(thread_id)
        .map(|thread| thread.project_id)
        .ok()
}

pub(crate) fn default_process_template_id() -> String {
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
    commands: Option<AgentCommandsInfo>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    models: Option<Vec<RuntimeAgentOptionInput>>,
    efforts: Option<Vec<RuntimeAgentOptionInput>>,
    permission_modes: Option<Vec<RuntimeAgentOptionInput>>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavePastedAttachmentRequest {
    file_name: Option<String>,
    mime_type: Option<String>,
    data_base64: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedPastedAttachment {
    path: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureWindowAreaRequest {
    file_name: Option<String>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotCaptureRequest {
    file_name: Option<String>,
    hide_self: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenshotOverlayCaptureRequest {
    request_id: String,
    file_name: Option<String>,
    hide_self: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CanvasKey {
    kind: CanvasKeyKind,
    id: String,
}

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum CanvasKeyKind {
    Session,
    Thread,
}

impl CanvasKey {
    fn storage_key(&self) -> Result<String, String> {
        let id = self.id.trim();
        if id.is_empty() {
            return Err("Canvas key id cannot be empty".to_string());
        }
        let kind = match self.kind {
            CanvasKeyKind::Session => "session",
            CanvasKeyKind::Thread => "thread",
        };
        Ok(format!("{kind}:{id}"))
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveCanvasDraftRequest {
    canvas_key: CanvasKey,
    title: Option<String>,
    snapshot_json: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveCanvasRevisionRequest {
    canvas_key: CanvasKey,
    title: Option<String>,
    snapshot_json: String,
    source: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertCanvasBlockRecordInput {
    block_id: String,
    block_kind: CanvasBlockKind,
    source_type: CanvasBlockSourceType,
    source_key: Option<String>,
    source_path: Option<String>,
    metadata_json: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCanvasBlocksRequest {
    canvas_key: CanvasKey,
    blocks: Vec<UpsertCanvasBlockRecordInput>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertCanvasAnchorRequest {
    canvas_key: CanvasKey,
    anchor_block_id: Option<String>,
    selection_block_ids_json: String,
    selection_element_ids_json: String,
    turn_id: String,
    summary: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildCanvasContextFileRequest {
    canvas_key: CanvasKey,
    kind: String,
    file_name_prefix: String,
    content: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedCanvasDraft {
    document: crate::models::CanvasDocumentInfo,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SavedCanvasRevision {
    document: crate::models::CanvasDocumentInfo,
    revision: crate::models::CanvasRevisionInfo,
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
    commands: Option<AgentCommandsInfo>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    models: Option<Vec<RuntimeAgentOptionInput>>,
    efforts: Option<Vec<RuntimeAgentOptionInput>>,
    permission_modes: Option<Vec<RuntimeAgentOptionInput>>,
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
fn list_terminals(
    terminal_service: State<'_, TerminalService>,
) -> Result<Vec<TerminalSessionSummary>, String> {
    terminal_service.list_sessions()
}

#[tauri::command]
fn create_terminal(
    req: CreateTerminalRequest,
    terminal_service: State<'_, TerminalService>,
) -> Result<TerminalSessionSummary, String> {
    terminal_service.create_session(req)
}

#[tauri::command]
fn write_terminal_input(
    req: WriteTerminalInputRequest,
    terminal_service: State<'_, TerminalService>,
) -> Result<(), String> {
    terminal_service.write_input(req)
}

#[tauri::command]
fn resize_terminal(
    req: ResizeTerminalRequest,
    terminal_service: State<'_, TerminalService>,
) -> Result<(), String> {
    terminal_service.resize_session(req)
}

#[tauri::command]
fn close_terminal(
    req: CloseTerminalRequest,
    terminal_service: State<'_, TerminalService>,
) -> Result<(), String> {
    terminal_service.close_session(req)
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
    let parent = projects_dir().map_err(|e| e.to_string())?;
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
                commands: req.commands.as_ref(),
                model: req.model.as_deref(),
                effort: req.effort.as_deref(),
                permission_mode: req.permission_mode.as_deref(),
                models: models.as_deref(),
                efforts: efforts.as_deref(),
                permission_modes: permission_modes.as_deref(),
            },
        )
        .map_err(|e| e.to_string())?;
    astra.update_astra_preferences_cache(updated.clone());
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

fn runtime_agent_session_config_to_dto(
    record: store::RuntimeAgentSessionConfigRecord,
) -> RuntimeAgentSessionConfigDto {
    RuntimeAgentSessionConfigDto {
        agent: record.agent,
        adapter_version: record.adapter_version,
        available_commands_json: record.available_commands_json,
        config_options_json: record.config_options_json,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn resolve_runtime_agent_session_config_record(
    agent: Agent,
    cache: &[RuntimeAgentMetadata],
    store: &Arc<dyn SessionStore>,
) -> Result<Option<store::RuntimeAgentSessionConfigRecord>, String> {
    let mut preferred_versions = Vec::new();
    let mut seen = HashSet::new();

    let push_version = |versions: &mut Vec<String>, seen: &mut HashSet<String>, value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        let normalized = trimmed.to_string();
        if seen.insert(normalized.clone()) {
            versions.push(normalized);
        }
    };

    if let Some(detected_version) = cache
        .iter()
        .find(|item| item.agent == agent)
        .and_then(|item| item.detected_version.as_deref())
    {
        push_version(&mut preferred_versions, &mut seen, detected_version);
    }

    if let Some(capability_version) = store
        .get_runtime_agent_capability(agent)
        .map_err(|e| e.to_string())?
        .and_then(|record| record.version)
    {
        push_version(&mut preferred_versions, &mut seen, &capability_version);
    }

    for adapter_version in preferred_versions {
        if let Some(record) = store
            .get_runtime_agent_session_config(agent, &adapter_version)
            .map_err(|e| e.to_string())?
        {
            return Ok(Some(record));
        }
    }

    store
        .list_runtime_agent_session_configs(agent)
        .map(|records| records.into_iter().next())
        .map_err(|e| e.to_string())
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
        agents::runtime::types::RuntimeTransportKind::PiRpc => "piRpc",
        agents::runtime::types::RuntimeTransportKind::Fake => "fake",
    }
    .to_string()
}

pub(crate) fn hydrate_start_request_from_db(
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

fn hydrate_skill_options(
    options: &mut agents::runtime::types::RuntimeMetadata,
    cache: &skills::SkillsCache,
) {
    let available_skills = cache.get();
    skills::hydrate_selected_skills_option(options, &available_skills);
}

fn hydrate_mcp_options(
    options: &mut agents::runtime::types::RuntimeMetadata,
    cache: &mcp::McpSettingsCache,
) {
    let settings = cache.get();
    mcp::hydrate_selected_mcps_option(options, &settings);
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        }
    }

    fn temp_workspace(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("sessio-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    fn write_until_mtime_changes(path: &Path, previous_mtime_ms: u64, content: &str) {
        for _ in 0..20 {
            std::fs::write(path, content).expect("write changed content");
            let meta = std::fs::metadata(path).expect("metadata after write");
            if file_mtime_ms(&meta).expect("mtime after write") != previous_mtime_ms {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("mtime did not change after repeated writes");
    }

    #[test]
    fn workspace_text_file_write_updates_content_and_mtime() {
        let workspace = temp_workspace("write-ok");
        let path = workspace.join("note.txt");
        std::fs::write(&path, "old").expect("write file");

        let loaded = read_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
        )
        .expect("read workspace file");
        let saved = write_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
            "new".to_string(),
            loaded.mtime_ms,
        )
        .expect("write workspace file");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read saved file"),
            "new"
        );
        assert!(saved.mtime_ms >= loaded.mtime_ms);
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn workspace_text_file_write_rejects_mtime_conflict() {
        let workspace = temp_workspace("write-conflict");
        let path = workspace.join("note.txt");
        std::fs::write(&path, "old").expect("write file");

        let loaded = read_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
        )
        .expect("read workspace file");
        write_until_mtime_changes(&path, loaded.mtime_ms, "external");

        let err = write_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
            "new".to_string(),
            loaded.mtime_ms,
        )
        .expect_err("mtime conflict should fail");
        assert!(err.contains("changed on disk"));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read conflicted file"),
            "external"
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn workspace_text_file_path_accepts_relative_paths_inside_workspace() {
        let workspace = temp_workspace("relative-path-ok");
        let nested = workspace.join("docs");
        let path = nested.join("note.md");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        std::fs::write(&path, "# hello").expect("write nested file");

        let loaded = read_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            "docs/note.md".to_string(),
        )
        .expect("relative workspace file should load");

        assert_eq!(loaded.content, "# hello");
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn workspace_text_file_path_rejects_outside_workspace() {
        let workspace = temp_workspace("path-guard");
        let outside = temp_workspace("outside").join("note.txt");
        std::fs::write(&outside, "outside").expect("write outside file");

        let err = read_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            outside.to_string_lossy().into_owned(),
        )
        .expect_err("outside workspace should fail");
        assert!(err.contains("outside the workspace"));
        let _ = std::fs::remove_dir_all(workspace);
        if let Some(parent) = outside.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_text_file_path_rejects_symlink_escape() {
        let workspace = temp_workspace("symlink-guard");
        let outside_dir = temp_workspace("symlink-outside");
        let outside = outside_dir.join("note.txt");
        let link = workspace.join("linked.txt");
        std::fs::write(&outside, "outside").expect("write outside file");
        std::os::unix::fs::symlink(&outside, &link).expect("create symlink");

        let err = read_workspace_text_file(
            workspace.to_string_lossy().into_owned(),
            link.to_string_lossy().into_owned(),
        )
        .expect_err("symlink escape should fail");
        assert!(err.contains("outside the workspace"));
        let _ = std::fs::remove_dir_all(workspace);
        let _ = std::fs::remove_dir_all(outside_dir);
    }

    #[test]
    fn session_ancestors_from_db_follows_multihop_agent_lineage() {
        let root = session(
            Agent::Pi,
            "root",
            None,
            None,
            "/tmp/pi/project/session-root.jsonl",
        );
        let middle = session(
            Agent::Claude,
            "middle",
            Some(Agent::Pi),
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
        assert_eq!(chain[0].agent, Agent::Pi);
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

    #[test]
    fn runtime_session_config_falls_back_to_capability_version_from_db() {
        let workspace = temp_workspace("runtime-session-config-capability-fallback");
        let db_path = workspace.join("sessio-index.db");
        let sqlite = Arc::new(SqliteStore::open(&db_path).expect("open sqlite"));
        sqlite.init().expect("init sqlite");
        sqlite
            .upsert_runtime_agent_capability(&store::RuntimeAgentCapabilityRecord {
                agent: Agent::Codex,
                transport: agents::runtime::types::RuntimeTransportKind::Acp,
                version: Some("1.0.2".to_string()),
                protocol_version: Some("1".to_string()),
                raw_initialize_response_json: "{}".to_string(),
                raw_capabilities_json: "{}".to_string(),
                updated_at: 20,
            })
            .expect("upsert capability");
        sqlite
            .upsert_runtime_agent_session_config(&store::RuntimeAgentSessionConfigRecord {
                agent: Agent::Codex,
                adapter_version: "1.0.2".to_string(),
                available_commands_json: r#"[{"name":"mcp"}]"#.to_string(),
                config_options_json: "[]".to_string(),
                created_at: 10,
                updated_at: 20,
            })
            .expect("upsert session config");
        let store: Arc<dyn SessionStore> = sqlite;

        let record =
            resolve_runtime_agent_session_config_record(Agent::Codex, &[], &store).expect("load");

        assert_eq!(
            record.expect("record").available_commands_json,
            r#"[{"name":"mcp"}]"#
        );
        let _ = std::fs::remove_dir_all(workspace);
    }

    #[test]
    fn runtime_session_config_falls_back_to_latest_db_record_without_detected_version() {
        let workspace = temp_workspace("runtime-session-config-latest-fallback");
        let db_path = workspace.join("sessio-index.db");
        let sqlite = Arc::new(SqliteStore::open(&db_path).expect("open sqlite"));
        sqlite.init().expect("init sqlite");
        sqlite
            .upsert_runtime_agent_session_config(&store::RuntimeAgentSessionConfigRecord {
                agent: Agent::Claude,
                adapter_version: "0.53.0".to_string(),
                available_commands_json: r#"[{"name":"legacy"}]"#.to_string(),
                config_options_json: "[]".to_string(),
                created_at: 10,
                updated_at: 10,
            })
            .expect("upsert old session config");
        sqlite
            .upsert_runtime_agent_session_config(&store::RuntimeAgentSessionConfigRecord {
                agent: Agent::Claude,
                adapter_version: "0.54.1".to_string(),
                available_commands_json: r#"[{"name":"deep-research"}]"#.to_string(),
                config_options_json: "[]".to_string(),
                created_at: 20,
                updated_at: 20,
            })
            .expect("upsert latest session config");
        let store: Arc<dyn SessionStore> = sqlite;

        let record =
            resolve_runtime_agent_session_config_record(Agent::Claude, &[], &store).expect("load");

        let record = record.expect("record");
        assert_eq!(record.adapter_version, "0.54.1");
        assert_eq!(
            record.available_commands_json,
            r#"[{"name":"deep-research"}]"#
        );
        let _ = std::fs::remove_dir_all(workspace);
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
    let removed_root = removed_sessions_dir()?;
    if let Ok(canvas_dir) = session_canvas_dir(&session.id) {
        if canvas_dir.exists() {
            let _ = std::fs::remove_dir_all(canvas_dir);
        }
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
    let is_opencode_sqlite = agent == Agent::Opencode && file_path.starts_with("sqlite:");
    if file_path.is_empty() || (!is_opencode_sqlite && !path.exists()) {
        anyhow::bail!(
            "Session file no longer exists (likely cleaned by {}): {}",
            match agent {
                Agent::Pi => "Pi",
                Agent::Codex => "Codex",
                Agent::Claude => "Claude Code",
                Agent::Opencode => "OpenCode",
            },
            if file_path.is_empty() {
                "<empty>"
            } else {
                file_path
            }
        );
    }
    let (messages, message_count) = match agent {
        Agent::Pi => {
            let source = crate::agents::sources::types::SessionSource {
                agent: crate::agents::sources::types::AgentKind::new(Agent::Pi.as_str()),
                session_id: session_id.unwrap_or_default().to_string(),
                scope: path
                    .parent()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_default(),
                file_path: file_path.to_string(),
                project: None,
                source_kind: crate::agents::sources::types::SourceKind::MainSession,
                metadata: Default::default(),
            };
            let events = crate::agents::sources::pi::parser::read_message_events(&path, &source)?;
            let count = events.len();
            let rows =
                crate::agents::sources::pi::parser::message_events_to_history_acp_messages(events);
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
        Agent::Opencode => {
            let sid = session_id.unwrap_or_default();
            let rows =
                crate::agents::sources::opencode::parser::read_history_acp_messages_with_locations(
                    file_path, sid,
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
fn save_pasted_attachment(
    req: SavePastedAttachmentRequest,
) -> Result<SavedPastedAttachment, String> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::io::Write;

    const MAX_PASTE_ATTACHMENT_BYTES: usize = 32 * 1024 * 1024;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.data_base64.as_bytes())
        .map_err(|e| format!("Invalid pasted attachment data: {e}"))?;
    if bytes.len() > MAX_PASTE_ATTACHMENT_BYTES {
        return Err("Pasted attachment is too large".to_string());
    }

    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let file_name =
        safe_pasted_attachment_file_name(req.file_name.as_deref(), req.mime_type.as_deref());
    let hash = hex::encode(Sha256::digest(&bytes));
    for index in 0..1000 {
        let candidate_name = if index == 0 {
            format!("sha256-{hash}-{file_name}")
        } else {
            format!("sha256-{hash}-{index}-{file_name}")
        };
        let path = dir.join(candidate_name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                return Ok(SavedPastedAttachment {
                    path: path.to_string_lossy().to_string(),
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(&path)
                    .map(|existing| existing == bytes)
                    .unwrap_or(false)
                {
                    return Ok(SavedPastedAttachment {
                        path: path.to_string_lossy().to_string(),
                    });
                }
                continue;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("Could not allocate pasted attachment path".to_string())
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn capture_window_area_png(
    window: WebviewWindow,
    req: CaptureWindowAreaRequest,
) -> Result<SavedPastedAttachment, String> {
    if !req.x.is_finite()
        || !req.y.is_finite()
        || !req.width.is_finite()
        || !req.height.is_finite()
        || req.width <= 0.0
        || req.height <= 0.0
    {
        return Err("Invalid snapshot capture area".to_string());
    }

    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let origin = window.inner_position().map_err(|e| e.to_string())?;
    let x = origin.x as f64 + req.x * scale;
    let y = origin.y as f64 + req.y * scale;
    let width = req.width * scale;
    let height = req.height * scale;
    if width < 1.0 || height < 1.0 {
        return Err("Snapshot capture area is too small".to_string());
    }

    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_name = safe_pasted_attachment_file_name(req.file_name.as_deref(), Some("image/png"));
    let path = dir.join(format!(
        "native-window-area-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));

    if let Ok(saved) = capture_webview_area_png(&window, &req, &path).await {
        return Ok(saved);
    }

    let image = cg_image_for_screen_rect(x, y, width, height)?;
    write_cg_image_png_to_path(&image, &path)
}

#[cfg(target_os = "macos")]
async fn capture_webview_area_png(
    window: &WebviewWindow,
    req: &CaptureWindowAreaRequest,
    path: &Path,
) -> Result<SavedPastedAttachment, String> {
    use block2::RcBlock;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    let sender = Arc::new(Mutex::new(Some(tx)));
    let path_string = path.to_string_lossy().to_string();
    let x = req.x;
    let y = req.y;
    let width = req.width;
    let height = req.height;

    window
        .with_webview(move |platform_webview| {
            let sender = Arc::clone(&sender);
            let send = move |result: Result<String, String>| {
                if let Ok(mut tx) = sender.lock() {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(result);
                    }
                }
            };
            unsafe {
                let webview = &*(platform_webview.inner() as *mut WKWebView);
                let bounds = webview.bounds();
                let origin_x = x.max(0.0);
                let origin_y = y.max(0.0);
                let max_width = bounds.size.width - origin_x;
                let max_height = bounds.size.height - origin_y;
                if max_width <= 0.0 || max_height <= 0.0 {
                    send(Err(
                        "WKWebView snapshot area is outside the webview bounds".to_string()
                    ));
                    return;
                }
                let crop = CGRect::new(
                    CGPoint::new(origin_x, origin_y),
                    CGSize::new(
                        width.min(max_width).max(1.0),
                        height.min(max_height).max(1.0),
                    ),
                );
                let config = WKSnapshotConfiguration::new(objc2::MainThreadMarker::new_unchecked());
                config.setRect(crop);
                config.setAfterScreenUpdates(true);
                let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                    if !error.is_null() {
                        let error = &*error;
                        send(Err(format!(
                            "WKWebView snapshot failed: {}",
                            error.localizedDescription()
                        )));
                        return;
                    }
                    if image.is_null() {
                        send(Err("WKWebView snapshot returned no image".to_string()));
                        return;
                    }
                    let image = &*image;
                    let result = image
                        .TIFFRepresentation()
                        .ok_or_else(|| "WKWebView snapshot did not return image data".to_string())
                        .and_then(|tiff| {
                            NSBitmapImageRep::imageRepWithData(&tiff).ok_or_else(|| {
                                "WKWebView snapshot image data was not decodable".to_string()
                            })
                        })
                        .and_then(|bitmap| {
                            let properties = NSDictionary::<
                                objc2_app_kit::NSBitmapImageRepPropertyKey,
                                AnyObject,
                            >::dictionary();
                            bitmap
                                .representationUsingType_properties(
                                    NSBitmapImageFileType::PNG,
                                    &properties,
                                )
                                .ok_or_else(|| "WKWebView snapshot PNG encoding failed".to_string())
                        })
                        .and_then(|png| {
                            let path = objc2_foundation::NSString::from_str(&path_string);
                            if png.writeToFile_atomically(&path, true) {
                                Ok(path_string.clone())
                            } else {
                                Err("WKWebView snapshot PNG write failed".to_string())
                            }
                        });
                    send(result);
                });
                webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
            }
        })
        .map_err(|e| e.to_string())?;

    match tokio::time::timeout(Duration::from_secs(6), rx).await {
        Ok(Ok(Ok(path))) => {
            let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
            if meta.len() == 0 {
                let _ = std::fs::remove_file(&path);
                return Err("WKWebView snapshot produced an empty PNG".to_string());
            }
            Ok(SavedPastedAttachment { path })
        }
        Ok(Ok(Err(error))) => Err(error),
        Ok(Err(_)) => Err("WKWebView snapshot callback was cancelled".to_string()),
        Err(_) => Err("WKWebView snapshot timed out".to_string()),
    }
}

#[cfg(target_os = "macos")]
fn write_cg_image_png_to_path(
    image: &core_graphics::image::CGImage,
    path: &Path,
) -> Result<SavedPastedAttachment, String> {
    use foreign_types::ForeignType;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_core_graphics::CGImage;
    use objc2_foundation::NSDictionary;

    let cg_image = unsafe { &*(image.as_ptr() as *const CGImage) };
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), cg_image);
    let properties = NSDictionary::<objc2_app_kit::NSBitmapImageRepPropertyKey, AnyObject>::new();
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &properties)
    }
    .ok_or_else(|| "CGImage PNG encoding failed".to_string())?;
    let path_string = objc2_foundation::NSString::from_str(&path.to_string_lossy());
    if !png.writeToFile_atomically(&path_string, true) {
        return Err("CGImage PNG write failed".to_string());
    }
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(path);
        return Err("CGImage PNG write produced an empty file".to_string());
    }
    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

#[cfg(target_os = "macos")]
fn cg_image_for_screen_rect(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<core_graphics::image::CGImage, String> {
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};

    let rect = CGRect::new(
        &CGPoint::new(x, y),
        &CGSize::new(width.max(1.0), height.max(1.0)),
    );
    CGDisplay::active_displays()
        .map_err(|e| format!("Could not enumerate displays: {e:?}"))?
        .into_iter()
        .find_map(|display_id| {
            let display = CGDisplay::new(display_id);
            let bounds = display.bounds();
            let display_x2 = bounds.origin.x + bounds.size.width;
            let display_y2 = bounds.origin.y + bounds.size.height;
            let rect_x2 = rect.origin.x + rect.size.width;
            let rect_y2 = rect.origin.y + rect.size.height;
            let intersects = rect.origin.x < display_x2
                && rect_x2 > bounds.origin.x
                && rect.origin.y < display_y2
                && rect_y2 > bounds.origin.y;
            intersects.then(|| display.image_for_rect(rect)).flatten()
        })
        .ok_or_else(|| "Could not capture screen rect image".to_string())
}

#[cfg(target_os = "macos")]
fn cg_image_for_window(
    window_id: u32,
    bounds: core_graphics::geometry::CGRect,
) -> Result<core_graphics::image::CGImage, String> {
    use core_graphics::display::CGDisplay;
    use core_graphics::window::{
        kCGWindowImageBestResolution, kCGWindowImageBoundsIgnoreFraming,
        kCGWindowListOptionIncludingWindow,
    };

    CGDisplay::screenshot(
        bounds,
        kCGWindowListOptionIncludingWindow,
        window_id,
        kCGWindowImageBoundsIgnoreFraming | kCGWindowImageBestResolution,
    )
    .ok_or_else(|| format!("Could not capture window image for {window_id}"))
}

#[cfg(target_os = "macos")]
fn capture_frontmost_window_png(
    app: &AppHandle,
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    use core::ffi::c_void;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::CGRect;
    use core_graphics::window::{
        kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerPID,
    };
    use objc2_app_kit::NSWorkspace;

    let file_name = safe_pasted_attachment_file_name(file_name.as_deref(), Some("image/png"));
    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "appshot-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));

    let frontmost_pid = {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let result = NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .map(|frontmost| frontmost.processIdentifier())
                .ok_or_else(|| "Could not determine the frontmost application".to_string());
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|_| "Frontmost application lookup was cancelled".to_string())??
    };

    let windows = CGDisplay::window_list_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        None,
    )
    .ok_or_else(|| "Could not enumerate on-screen windows".to_string())?;
    let key_owner_pid = unsafe { kCGWindowOwnerPID as *const c_void };
    let key_window_layer = unsafe { kCGWindowLayer as *const c_void };
    let key_window_alpha = unsafe { kCGWindowAlpha as *const c_void };
    let key_window_bounds = unsafe { kCGWindowBounds as *const c_void };
    let key_window_number = unsafe { kCGWindowNumber as *const c_void };

    fn dict_value(dict: &CFDictionary, key: *const c_void) -> Option<CFType> {
        dict.find(key)
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
    }

    fn dict_number_i64(dict: &CFDictionary, key: *const c_void) -> Option<i64> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_i64())
    }

    fn dict_number_f64(dict: &CFDictionary, key: *const c_void) -> Option<f64> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_f64())
    }

    let mut target_window: Option<(u32, CGRect)> = None;
    for value in &windows {
        let cf_type = unsafe { CFType::wrap_under_get_rule(*value) };
        let Some(dict) = cf_type.downcast::<CFDictionary>() else {
            continue;
        };
        let pid = dict_number_i64(&dict, key_owner_pid).unwrap_or_default();
        if pid != i64::from(frontmost_pid) {
            continue;
        }
        let layer = dict_number_i64(&dict, key_window_layer).unwrap_or_default();
        if layer != 0 {
            continue;
        }
        let alpha = dict_number_f64(&dict, key_window_alpha).unwrap_or(1.0);
        if alpha <= 0.0 {
            continue;
        }
        let Some(bounds_cf) = dict_value(&dict, key_window_bounds) else {
            continue;
        };
        let Some(bounds_dict) = bounds_cf.downcast::<CFDictionary>() else {
            continue;
        };
        let Some(bounds) = CGRect::from_dict_representation(&bounds_dict) else {
            continue;
        };
        if bounds.is_empty() || bounds.size.width < 2.0 || bounds.size.height < 2.0 {
            continue;
        }
        let Some(window_id) =
            dict_number_i64(&dict, key_window_number).and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        target_window = Some((window_id, bounds));
        break;
    }

    let (window_id, bounds) = target_window
        .ok_or_else(|| "Could not find a visible frontmost window to capture".to_string())?;
    let image = cg_image_for_window(window_id, bounds)?;
    write_cg_image_png_to_path(&image, &path)
}

#[cfg(target_os = "macos")]
fn ensure_appshot_can_capture(app: &AppHandle) -> Result<(), String> {
    let permissions = appshot_permission_status();
    if permissions.can_capture {
        return Ok(());
    }
    if let Err(error) = appshot_permission_panel::show(app.clone()) {
        log::warn!("[appshot] failed to show permission panel from manual screenshot: {error}");
    }
    let _ = emit_appshot_permission_required(
        app,
        AppshotPermissionRequiredPayload {
            shortcut: "manual".to_string(),
            status: permissions,
        },
    );
    Err("Appshot needs screen capture permission before it can capture other apps".to_string())
}

#[cfg(target_os = "macos")]
fn with_optional_hidden_main_window<F>(
    app: &AppHandle,
    hide_self: bool,
    capture: F,
) -> Result<SavedPastedAttachment, String>
where
    F: FnOnce() -> Result<SavedPastedAttachment, String>,
{
    let was_visible = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    if hide_self {
        if let Some(window) = app.get_webview_window("main") {
            hide_main_window(window);
            thread::sleep(Duration::from_millis(260));
        }
    }

    let result = capture();
    if hide_self && was_visible {
        show_main_window(app);
    }
    result
}

#[cfg(target_os = "macos")]
fn capture_selected_screen_area_png_impl(
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    let file_name = safe_pasted_attachment_file_name(file_name.as_deref(), Some("image/png"));
    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "screen-selection-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));

    let status = std::process::Command::new("screencapture")
        .arg("-i")
        .arg("-s")
        .arg("-x")
        .arg("-t")
        .arg("png")
        .arg(&path)
        .status()
        .map_err(|e| format!("Failed to start selected area capture: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err("Screenshot selection was cancelled".to_string());
    }

    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(&path);
        return Err("Selected area capture produced an empty PNG".to_string());
    }

    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

#[cfg(target_os = "macos")]
fn capture_interactive_screen_png_impl(
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    let file_name = safe_pasted_attachment_file_name(file_name.as_deref(), Some("image/png"));
    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "screen-interactive-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));

    let status = std::process::Command::new("screencapture")
        .arg("-i")
        .arg("-x")
        .arg("-t")
        .arg("png")
        .arg(&path)
        .status()
        .map_err(|e| format!("Failed to start interactive screenshot: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err("Screenshot selection was cancelled".to_string());
    }

    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(&path);
        return Err("Interactive screenshot produced an empty PNG".to_string());
    }

    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn capture_frontmost_app_window_png(
    app: AppHandle,
    req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    ensure_appshot_can_capture(&app)?;
    with_optional_hidden_main_window(&app, req.hide_self.unwrap_or(false), || {
        capture_frontmost_window_png(&app, req.file_name)
    })
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn capture_selected_screen_area_png(
    app: AppHandle,
    req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    ensure_appshot_can_capture(&app)?;
    with_optional_hidden_main_window(&app, req.hide_self.unwrap_or(false), || {
        capture_selected_screen_area_png_impl(req.file_name)
    })
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn capture_interactive_screen_png(
    app: AppHandle,
    req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    ensure_appshot_can_capture(&app)?;
    with_optional_hidden_main_window(&app, req.hide_self.unwrap_or(false), || {
        capture_interactive_screen_png_impl(req.file_name)
    })
}

#[cfg(target_os = "macos")]
fn capture_monitor_background_png(
    monitor: &tauri::Monitor,
    file_name: Option<&str>,
) -> Result<SavedPastedAttachment, String> {
    let file_name = safe_pasted_attachment_file_name(file_name, Some("image/png"));
    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!(
        "screen-overlay-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));
    let pos = monitor.position();
    let size = monitor.size();
    let image = cg_image_for_screen_rect(
        f64::from(pos.x),
        f64::from(pos.y),
        f64::from(size.width),
        f64::from(size.height),
    )?;
    write_cg_image_png_to_path(&image, &path)
}

#[cfg(target_os = "macos")]
fn screenshot_overlay_window_candidates(
    monitor: &tauri::Monitor,
) -> Vec<ScreenshotOverlayWindowCandidateDto> {
    use core::ffi::c_void;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::CGRect;
    use core_graphics::window::{
        kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowName, kCGWindowNumber, kCGWindowOwnerName,
    };

    fn dict_value(dict: &CFDictionary, key: *const c_void) -> Option<CFType> {
        dict.find(key)
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
    }

    fn dict_number_i64(dict: &CFDictionary, key: *const c_void) -> Option<i64> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_i64())
    }

    fn dict_number_f64(dict: &CFDictionary, key: *const c_void) -> Option<f64> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_f64())
    }

    fn dict_string(dict: &CFDictionary, key: *const c_void) -> Option<String> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFString>())
            .map(|value| value.to_string())
    }

    let windows = match CGDisplay::window_list_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        None,
    ) {
        Some(windows) => windows,
        None => return Vec::new(),
    };
    let key_window_number = unsafe { kCGWindowNumber as *const c_void };
    let key_owner_name = unsafe { kCGWindowOwnerName as *const c_void };
    let key_window_name = unsafe { kCGWindowName as *const c_void };
    let key_window_layer = unsafe { kCGWindowLayer as *const c_void };
    let key_window_alpha = unsafe { kCGWindowAlpha as *const c_void };
    let key_window_bounds = unsafe { kCGWindowBounds as *const c_void };
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();
    let monitor_x = f64::from(monitor_pos.x);
    let monitor_y = f64::from(monitor_pos.y);
    let monitor_width = f64::from(monitor_size.width);
    let monitor_height = f64::from(monitor_size.height);
    let mut candidates = Vec::new();

    for value in &windows {
        let cf_type = unsafe { CFType::wrap_under_get_rule(*value) };
        let Some(dict) = cf_type.downcast::<CFDictionary>() else {
            continue;
        };
        let layer = dict_number_i64(&dict, key_window_layer).unwrap_or_default();
        if layer != 0 {
            continue;
        }
        let alpha = dict_number_f64(&dict, key_window_alpha).unwrap_or(1.0);
        if alpha <= 0.0 {
            continue;
        }
        let Some(bounds_cf) = dict_value(&dict, key_window_bounds) else {
            continue;
        };
        let Some(bounds_dict) = bounds_cf.downcast::<CFDictionary>() else {
            continue;
        };
        let Some(bounds) = CGRect::from_dict_representation(&bounds_dict) else {
            continue;
        };
        if bounds.is_empty() || bounds.size.width < 36.0 || bounds.size.height < 28.0 {
            continue;
        }
        let x = bounds.origin.x - monitor_x;
        let y = bounds.origin.y - monitor_y;
        let right = x + bounds.size.width;
        let bottom = y + bounds.size.height;
        if right <= 0.0 || bottom <= 0.0 || x >= monitor_width || y >= monitor_height {
            continue;
        }
        let Some(id) =
            dict_number_i64(&dict, key_window_number).and_then(|value| u32::try_from(value).ok())
        else {
            continue;
        };
        let app_name = dict_string(&dict, key_owner_name).unwrap_or_else(|| "Window".to_string());
        candidates.push(ScreenshotOverlayWindowCandidateDto {
            id: id.to_string(),
            app_name,
            title: dict_string(&dict, key_window_name),
            x: clamp_f64(x, 0.0, monitor_width),
            y: clamp_f64(y, 0.0, monitor_height),
            width: clamp_f64(right, 0.0, monitor_width) - clamp_f64(x, 0.0, monitor_width),
            height: clamp_f64(bottom, 0.0, monitor_height) - clamp_f64(y, 0.0, monitor_height),
        });
    }

    candidates
}

#[cfg(target_os = "macos")]
fn clamp_f64(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

#[cfg(target_os = "macos")]
fn hide_main_window_now(window: WebviewWindow) {
    let _ = window.hide();
    let _ = set_window_alpha(&window, 1.0);
}

#[cfg(target_os = "macos")]
fn cleanup_screenshot_overlay(app: &AppHandle, label: &str, reveal_main: bool) {
    let mut cancelled_request_id = None;
    if let Some(state) = app.try_state::<ScreenshotOverlayState>() {
        if let Ok(mut sources) = state.sources.lock() {
            cancelled_request_id = sources.remove(label).map(|source| {
                let _ = std::fs::remove_file(&source.source_path);
                source.request_id
            });
        }
        if let Ok(mut reveal) = state.reveal_main_on_finish.lock() {
            reveal.remove(label);
        }
    }
    if let Some(request_id) = cancelled_request_id {
        let _ = app.emit(
            "screenshot_overlay_cancelled",
            ScreenshotOverlayCancelledPayload { request_id },
        );
    }
    if reveal_main {
        show_main_window(app);
    }
}

#[cfg(target_os = "macos")]
#[tauri::command]
async fn open_screenshot_overlay_capture(
    app: AppHandle,
    state: State<'_, ScreenshotOverlayState>,
    req: ScreenshotOverlayCaptureRequest,
) -> Result<ScreenshotOverlayWindowDto, String> {
    ensure_appshot_can_capture(&app)?;
    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is not available".to_string())?;
    let monitor = main_window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| "Could not find a screen for screenshot overlay".to_string())?;

    let hide_self = req.hide_self.unwrap_or(false);
    let reveal_main_on_finish = hide_self && main_window.is_visible().unwrap_or(false);
    if hide_self {
        hide_main_window_now(main_window);
        thread::sleep(Duration::from_millis(90));
    }

    let windows = screenshot_overlay_window_candidates(&monitor);
    let source = match capture_monitor_background_png(&monitor, Some("Screenshot.png")) {
        Ok(source) => source,
        Err(error) => {
            if reveal_main_on_finish {
                show_main_window(&app);
            }
            return Err(error);
        }
    };
    let label = format!(
        "screenshot-overlay-{}",
        chrono::Utc::now().timestamp_millis()
    );
    let file_name = safe_pasted_attachment_file_name(req.file_name.as_deref(), Some("image/png"));
    let source_path_for_cleanup = source.path.clone();
    {
        let mut sources = match state.sources.lock() {
            Ok(sources) => sources,
            Err(error) => {
                if reveal_main_on_finish {
                    show_main_window(&app);
                }
                let _ = std::fs::remove_file(&source_path_for_cleanup);
                return Err(error.to_string());
            }
        };
        sources.insert(
            label.clone(),
            ScreenshotOverlaySourceDto {
                request_id: req.request_id,
                source_path: source.path,
                file_name,
                mode: "interactive".to_string(),
                windows,
                initial_selection: None,
            },
        );
    }
    {
        let mut reveal = match state.reveal_main_on_finish.lock() {
            Ok(reveal) => reveal,
            Err(error) => {
                if let Ok(mut sources) = state.sources.lock() {
                    sources.remove(&label);
                }
                if reveal_main_on_finish {
                    show_main_window(&app);
                }
                let _ = std::fs::remove_file(&source_path_for_cleanup);
                return Err(error.to_string());
            }
        };
        reveal.insert(label.clone(), reveal_main_on_finish);
    }

    let pos = monitor.position();
    let size = monitor.size();
    let scale = monitor.scale_factor();
    let url = WebviewUrl::App("index.html?screenshotOverlay=1".into());
    let init_script = r#"
        document.documentElement.style.background = 'transparent';
        document.documentElement.style.backgroundColor = 'transparent';
        if (document.body) {
          document.body.style.background = 'transparent';
          document.body.style.backgroundColor = 'transparent';
        } else {
          window.addEventListener('DOMContentLoaded', () => {
            document.body.style.background = 'transparent';
            document.body.style.backgroundColor = 'transparent';
          }, { once: true });
        }
    "#;
    let overlay = match WebviewWindowBuilder::new(&app, &label, url)
        .title("Screenshot")
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .transparent(true)
        .background_color(tauri::utils::config::Color(0, 0, 0, 0))
        .always_on_top(true)
        .skip_taskbar(true)
        .initialization_script(init_script)
        .position(pos.x as f64 / scale, pos.y as f64 / scale)
        .inner_size(size.width as f64 / scale, size.height as f64 / scale)
        .focused(true)
        .build()
    {
        Ok(overlay) => overlay,
        Err(error) => {
            cleanup_screenshot_overlay(&app, &label, reveal_main_on_finish);
            return Err(error.to_string());
        }
    };
    let _ = overlay.show();
    let _ = overlay.set_focus();
    let cleanup_app = app.clone();
    let cleanup_label = label.clone();
    overlay.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let reveal_main = cleanup_app
                .try_state::<ScreenshotOverlayState>()
                .and_then(|state| {
                    state
                        .reveal_main_on_finish
                        .lock()
                        .ok()
                        .and_then(|reveal| reveal.get(&cleanup_label).copied())
                })
                .unwrap_or(false);
            cleanup_screenshot_overlay(&cleanup_app, &cleanup_label, reveal_main);
        }
    });

    Ok(ScreenshotOverlayWindowDto { label })
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn get_screenshot_overlay_source(
    window: WebviewWindow,
    state: State<ScreenshotOverlayState>,
) -> Result<ScreenshotOverlaySourceDto, String> {
    let sources = state.sources.lock().map_err(|e| e.to_string())?;
    sources
        .get(window.label())
        .cloned()
        .ok_or_else(|| "Screenshot overlay source is not available".to_string())
}

#[tauri::command]
fn computer_use_pointer_overlay_ready(
    app: AppHandle,
    payload: ComputerUsePointerOverlayReadyPayload,
) -> Result<(), String> {
    computer_use::pointer_overlay::mark_pointer_overlay_ready(&app, &payload.label)
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn finish_screenshot_overlay(
    app: AppHandle,
    window: WebviewWindow,
    state: State<ScreenshotOverlayState>,
) -> Result<(), String> {
    let should_reveal_main = state
        .reveal_main_on_finish
        .lock()
        .ok()
        .and_then(|mut reveal| reveal.remove(window.label()))
        .unwrap_or(false);
    {
        let mut sources = state.sources.lock().map_err(|e| e.to_string())?;
        if let Some(source) = sources.remove(window.label()) {
            let _ = std::fs::remove_file(&source.source_path);
        }
    }
    let close_result = window.close().map_err(|e| e.to_string());
    if should_reveal_main {
        show_main_window(&app);
    }
    close_result
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn complete_screenshot_overlay_capture(
    _state: State<ScreenshotOverlayState>,
    req: ScreenshotOverlayCompleteRequest,
) -> Result<(), String> {
    let _ = (req.request_id, req.path, req.cancelled);
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn ensure_appshot_can_capture(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(any(windows, target_os = "linux"))]
fn with_optional_hidden_main_window<F>(
    app: &AppHandle,
    hide_self: bool,
    capture: F,
) -> Result<SavedPastedAttachment, String>
where
    F: FnOnce() -> Result<SavedPastedAttachment, String>,
{
    let was_visible = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    if hide_self {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
            thread::sleep(Duration::from_millis(110));
        }
    }

    let result = capture();
    if hide_self && was_visible {
        show_main_window(app);
    }
    result
}

#[cfg(windows)]
fn capture_frontmost_window_png(
    _app: &AppHandle,
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    screenshot::windows::capture_frontmost_window_png(file_name)
}

#[cfg(target_os = "linux")]
fn capture_frontmost_window_png(
    _app: &AppHandle,
    file_name: Option<String>,
) -> Result<SavedPastedAttachment, String> {
    screenshot::linux::capture_frontmost_window_png(file_name)
}

#[cfg(any(windows, target_os = "linux"))]
type ScreenshotOverlayResultSender =
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<SavedPastedAttachment, String>>>>>;

#[cfg(any(windows, target_os = "linux"))]
#[derive(Clone)]
enum ScreenshotOverlayCompletion {
    Saved(String),
    Cancelled,
}

#[cfg(any(windows, target_os = "linux"))]
fn start_platform_screenshot_overlay_capture(
    app: &AppHandle,
    state: &ScreenshotOverlayState,
    request_id: String,
    file_name: Option<String>,
    mode: &str,
    reveal_main_on_finish: bool,
    cancel_sender: Option<ScreenshotOverlayResultSender>,
) -> Result<ScreenshotOverlayWindowDto, String> {
    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is not available".to_string())?;

    #[cfg(windows)]
    let screen_rect = screenshot::windows::monitor_rect_for_window(&main_window);

    #[cfg(target_os = "linux")]
    let monitor = main_window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| "Could not find a screen for screenshot overlay".to_string())?;

    #[cfg(target_os = "linux")]
    let screen_rect = screenshot::linux::monitor_rect(&monitor);

    #[cfg(windows)]
    let windows = if mode == "selection" {
        Vec::new()
    } else {
        screenshot::windows::window_candidates_for_rect(screen_rect)
    };

    #[cfg(target_os = "linux")]
    let windows = if mode == "selection" {
        Vec::new()
    } else {
        screenshot::linux::window_candidates_for_rect(screen_rect)
    };

    #[cfg(windows)]
    let source = match screenshot::windows::capture_monitor_background_png(
        screen_rect,
        Some("Screenshot.png"),
    ) {
        Ok(source) => source,
        Err(error) => {
            if reveal_main_on_finish {
                show_main_window(app);
            }
            return Err(error);
        }
    };

    #[cfg(target_os = "linux")]
    let source = match screenshot::linux::capture_monitor_background_png(
        screen_rect,
        Some("Screenshot.png"),
    ) {
        Ok(source) => source,
        Err(error) => {
            if reveal_main_on_finish {
                show_main_window(app);
            }
            return Err(error);
        }
    };

    let label = format!(
        "screenshot-overlay-{}",
        chrono::Utc::now().timestamp_millis()
    );
    let file_name = safe_pasted_attachment_file_name(file_name.as_deref(), Some("image/png"));
    let source_path_for_cleanup = source.path.clone();
    {
        let mut sources = match state.sources.lock() {
            Ok(sources) => sources,
            Err(error) => {
                let _ = std::fs::remove_file(&source_path_for_cleanup);
                if reveal_main_on_finish {
                    show_main_window(app);
                }
                return Err(error.to_string());
            }
        };
        sources.insert(
            label.clone(),
            ScreenshotOverlaySourceDto {
                request_id: request_id.clone(),
                source_path: source.path,
                file_name,
                mode: mode.to_string(),
                windows,
                initial_selection: None,
            },
        );
    }
    {
        let mut reveal = match state.reveal_main_on_finish.lock() {
            Ok(reveal) => reveal,
            Err(error) => {
                if let Ok(mut sources) = state.sources.lock() {
                    sources.remove(&label);
                }
                let _ = std::fs::remove_file(&source_path_for_cleanup);
                if reveal_main_on_finish {
                    show_main_window(app);
                }
                return Err(error.to_string());
            }
        };
        reveal.insert(label.clone(), reveal_main_on_finish);
    }
    if let Some(sender) = cancel_sender.as_ref() {
        let mut pending = match state.pending_results.lock() {
            Ok(pending) => pending,
            Err(error) => {
                cleanup_screenshot_overlay(app, &label, reveal_main_on_finish);
                return Err(error.to_string());
            }
        };
        pending.insert(request_id.clone(), Arc::clone(sender));
    }

    #[cfg(windows)]
    let scale = screenshot::windows::window_scale_factor(&main_window);
    #[cfg(target_os = "linux")]
    let scale = monitor.scale_factor().max(1.0);

    let url = WebviewUrl::App("index.html?screenshotOverlay=1".into());
    let overlay = match WebviewWindowBuilder::new(app, &label, url)
        .title("Screenshot")
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .position(screen_rect.x as f64 / scale, screen_rect.y as f64 / scale)
        .inner_size(
            screen_rect.width as f64 / scale,
            screen_rect.height as f64 / scale,
        )
        .focused(true)
        .build()
    {
        Ok(overlay) => overlay,
        Err(error) => {
            cleanup_screenshot_overlay(app, &label, reveal_main_on_finish);
            if let Ok(mut pending) = state.pending_results.lock() {
                pending.remove(&request_id);
            }
            return Err(error.to_string());
        }
    };
    let cleanup_app = app.clone();
    let cleanup_label = label.clone();
    let cleanup_request_id = request_id.clone();
    overlay.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            if let Some(sender) = &cancel_sender {
                let completion = cleanup_app
                    .try_state::<ScreenshotOverlayState>()
                    .and_then(|state| {
                        take_screenshot_overlay_completion(state.inner(), &cleanup_request_id)
                    })
                    .unwrap_or(ScreenshotOverlayCompletion::Cancelled);
                send_screenshot_overlay_completion(sender, completion);
            }
            let reveal_main = cleanup_app
                .try_state::<ScreenshotOverlayState>()
                .and_then(|state| {
                    state
                        .reveal_main_on_finish
                        .lock()
                        .ok()
                        .and_then(|reveal| reveal.get(&cleanup_label).copied())
                })
                .unwrap_or(false);
            cleanup_screenshot_overlay(&cleanup_app, &cleanup_label, reveal_main);
        }
    });
    let _ = overlay.set_focus();

    Ok(ScreenshotOverlayWindowDto { label })
}

#[cfg(any(windows, target_os = "linux"))]
fn send_screenshot_overlay_cancelled(sender: &ScreenshotOverlayResultSender) {
    send_screenshot_overlay_completion(sender, ScreenshotOverlayCompletion::Cancelled);
}

#[cfg(any(windows, target_os = "linux"))]
fn send_screenshot_overlay_completion(
    sender: &ScreenshotOverlayResultSender,
    completion: ScreenshotOverlayCompletion,
) {
    if let Ok(mut sender) = sender.lock() {
        if let Some(sender) = sender.take() {
            let result = match completion {
                ScreenshotOverlayCompletion::Saved(path) => Ok(SavedPastedAttachment { path }),
                ScreenshotOverlayCompletion::Cancelled => {
                    Err("Screenshot selection was cancelled".to_string())
                }
            };
            let _ = sender.send(result);
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn take_screenshot_overlay_completion(
    state: &ScreenshotOverlayState,
    request_id: &str,
) -> Option<ScreenshotOverlayCompletion> {
    state
        .completed_results
        .lock()
        .ok()
        .and_then(|mut completed| completed.remove(request_id))
}

#[cfg(any(windows, target_os = "linux"))]
fn send_stored_screenshot_overlay_completion(state: &ScreenshotOverlayState, request_id: &str) {
    let Some(completion) = take_screenshot_overlay_completion(state, request_id) else {
        return;
    };
    let sender = state
        .pending_results
        .lock()
        .ok()
        .and_then(|pending| pending.get(request_id).cloned());
    if let Some(sender) = sender {
        send_screenshot_overlay_completion(&sender, completion);
    }
}

#[cfg(any(windows, target_os = "linux"))]
async fn with_optional_hidden_main_window_async<F, Fut>(
    app: &AppHandle,
    hide_self: bool,
    capture: F,
) -> Result<SavedPastedAttachment, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<SavedPastedAttachment, String>>,
{
    let was_visible = app
        .get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false);
    if hide_self {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
            tokio::time::sleep(Duration::from_millis(110)).await;
        }
    }

    let result = capture().await;
    if hide_self && was_visible {
        show_main_window(app);
    }
    result
}

#[cfg(any(windows, target_os = "linux"))]
async fn capture_screenshot_overlay_png_impl(
    app: AppHandle,
    state: &ScreenshotOverlayState,
    file_name: Option<String>,
    mode: &str,
    reveal_main_on_finish: bool,
) -> Result<SavedPastedAttachment, String> {
    let request_id = format!(
        "screenshot-command-{}",
        chrono::Utc::now().timestamp_millis()
    );
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<SavedPastedAttachment, String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let pending_request_id = request_id.clone();

    let overlay_result = start_platform_screenshot_overlay_capture(
        &app,
        state,
        request_id,
        file_name,
        mode,
        reveal_main_on_finish,
        Some(Arc::clone(&tx)),
    );
    if let Err(error) = overlay_result {
        send_screenshot_overlay_cancelled(&tx);
        return Err(error);
    }

    let result = match tokio::time::timeout(SCREENSHOT_OVERLAY_CAPTURE_TIMEOUT, rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("Screenshot selection was cancelled".to_string()),
        Err(_) => Err("Screenshot selection timed out".to_string()),
    };
    if let Ok(mut pending) = state.pending_results.lock() {
        pending.remove(&pending_request_id);
    }
    if let Ok(mut completed) = state.completed_results.lock() {
        completed.remove(&pending_request_id);
    }
    result
}

#[cfg(any(windows, target_os = "linux"))]
fn cleanup_screenshot_overlay(app: &AppHandle, label: &str, reveal_main: bool) {
    let mut cancelled_request_id = None;
    if let Some(state) = app.try_state::<ScreenshotOverlayState>() {
        if let Ok(mut sources) = state.sources.lock() {
            cancelled_request_id = sources.remove(label).map(|source| {
                let _ = std::fs::remove_file(&source.source_path);
                source.request_id
            });
        }
        if let Ok(mut reveal) = state.reveal_main_on_finish.lock() {
            reveal.remove(label);
        }
    }
    if let Some(request_id) = cancelled_request_id {
        let _ = app.emit(
            "screenshot_overlay_cancelled",
            ScreenshotOverlayCancelledPayload { request_id },
        );
    }
    if reveal_main {
        show_main_window(app);
    }
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
fn capture_frontmost_app_window_png(
    app: AppHandle,
    req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    ensure_appshot_can_capture(&app)?;
    with_optional_hidden_main_window(&app, req.hide_self.unwrap_or(false), || {
        capture_frontmost_window_png(&app, req.file_name)
    })
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
async fn capture_selected_screen_area_png(
    app: AppHandle,
    state: State<'_, ScreenshotOverlayState>,
    req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    ensure_appshot_can_capture(&app)?;
    let state = state.inner();
    with_optional_hidden_main_window_async(&app, req.hide_self.unwrap_or(false), || {
        capture_screenshot_overlay_png_impl(app.clone(), state, req.file_name, "selection", false)
    })
    .await
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
async fn capture_interactive_screen_png(
    app: AppHandle,
    state: State<'_, ScreenshotOverlayState>,
    req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    ensure_appshot_can_capture(&app)?;
    let state = state.inner();
    with_optional_hidden_main_window_async(&app, req.hide_self.unwrap_or(false), || {
        capture_screenshot_overlay_png_impl(app.clone(), state, req.file_name, "interactive", false)
    })
    .await
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
async fn open_screenshot_overlay_capture(
    app: AppHandle,
    state: State<'_, ScreenshotOverlayState>,
    req: ScreenshotOverlayCaptureRequest,
) -> Result<ScreenshotOverlayWindowDto, String> {
    ensure_appshot_can_capture(&app)?;
    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window is not available".to_string())?;

    let hide_self = req.hide_self.unwrap_or(false);
    let reveal_main_on_finish = hide_self && main_window.is_visible().unwrap_or(false);
    if hide_self {
        let _ = main_window.hide();
        tokio::time::sleep(Duration::from_millis(110)).await;
    }

    start_platform_screenshot_overlay_capture(
        &app,
        &state,
        req.request_id,
        req.file_name,
        "interactive",
        reveal_main_on_finish,
        None,
    )
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
fn get_screenshot_overlay_source(
    window: WebviewWindow,
    state: State<ScreenshotOverlayState>,
) -> Result<ScreenshotOverlaySourceDto, String> {
    let sources = state.sources.lock().map_err(|e| e.to_string())?;
    sources
        .get(window.label())
        .cloned()
        .ok_or_else(|| "Screenshot overlay source is not available".to_string())
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
fn finish_screenshot_overlay(
    app: AppHandle,
    window: WebviewWindow,
    state: State<ScreenshotOverlayState>,
) -> Result<(), String> {
    let should_reveal_main = state
        .reveal_main_on_finish
        .lock()
        .ok()
        .and_then(|mut reveal| reveal.remove(window.label()))
        .unwrap_or(false);
    let request_id = {
        let mut sources = state.sources.lock().map_err(|e| e.to_string())?;
        sources.remove(window.label()).map(|source| {
            let _ = std::fs::remove_file(&source.source_path);
            source.request_id
        })
    };
    let close_result = window.close().map_err(|e| e.to_string());
    if should_reveal_main {
        show_main_window(&app);
    }
    if let Some(request_id) = request_id {
        send_stored_screenshot_overlay_completion(&state, &request_id);
    }
    close_result
}

#[cfg(any(windows, target_os = "linux"))]
#[tauri::command]
fn complete_screenshot_overlay_capture(
    state: State<ScreenshotOverlayState>,
    req: ScreenshotOverlayCompleteRequest,
) -> Result<(), String> {
    let sender = {
        let pending = state.pending_results.lock().map_err(|e| e.to_string())?;
        pending.get(&req.request_id).cloned()
    };
    if sender.is_none() {
        return Ok(());
    }
    let completion = if req.cancelled.unwrap_or(false) {
        ScreenshotOverlayCompletion::Cancelled
    } else {
        ScreenshotOverlayCompletion::Saved(
            req.path
                .ok_or_else(|| "Screenshot completion did not include an image path".to_string())?,
        )
    };
    let mut completed = state.completed_results.lock().map_err(|e| e.to_string())?;
    completed.insert(req.request_id, completion);
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn capture_frontmost_app_window_png(
    _app: AppHandle,
    _req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    Err("Frontmost app screenshot is not implemented on this platform yet".to_string())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn capture_selected_screen_area_png(
    _app: AppHandle,
    _req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    Err("Selected area screenshot is not implemented on this platform yet".to_string())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn capture_interactive_screen_png(
    _app: AppHandle,
    _req: ScreenshotCaptureRequest,
) -> Result<SavedPastedAttachment, String> {
    Err("Interactive screenshot is not implemented on this platform yet".to_string())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
async fn open_screenshot_overlay_capture(
    _app: AppHandle,
    _state: State<'_, ScreenshotOverlayState>,
    _req: ScreenshotOverlayCaptureRequest,
) -> Result<ScreenshotOverlayWindowDto, String> {
    Err("Screenshot overlay editing is not implemented on this platform yet".to_string())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn get_screenshot_overlay_source(
    _window: WebviewWindow,
    _state: State<ScreenshotOverlayState>,
) -> Result<ScreenshotOverlaySourceDto, String> {
    Err("Screenshot overlay editing is not implemented on this platform yet".to_string())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn finish_screenshot_overlay(
    _app: AppHandle,
    _window: WebviewWindow,
    _state: State<ScreenshotOverlayState>,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn complete_screenshot_overlay_capture(
    _state: State<ScreenshotOverlayState>,
    req: ScreenshotOverlayCompleteRequest,
) -> Result<(), String> {
    let _ = (req.request_id, req.path, req.cancelled);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn desktop_control_inputs() -> desktop_control::DesktopControlInputs {
    let input_injection_supported =
        crate::computer_use::platform::default_provider().supports_control();
    use desktop_control::{DesktopControlInputs, DesktopPlatform, PermissionTier};
    DesktopControlInputs {
        platform: DesktopPlatform::current(),
        requires_permission: false,
        // Non-macOS platforms do not gate screen capture / accessibility behind
        // an OS permission Sessio checks today; treat them as not-gated (usable)
        // rather than denied.
        screenshots: PermissionTier::new(true, false),
        accessibility: PermissionTier::new(true, false),
        // Reflect the current platform provider's real control support so the
        // shared desktop-control status stays aligned with computer-use runtime
        // capabilities.
        input_injection_supported,
    }
}

#[cfg(target_os = "macos")]
fn desktop_control_inputs() -> desktop_control::DesktopControlInputs {
    let input_injection_supported =
        crate::computer_use::platform::default_provider().supports_control();
    use desktop_control::{DesktopControlInputs, DesktopPlatform, PermissionTier};
    DesktopControlInputs {
        platform: DesktopPlatform::Macos,
        requires_permission: true,
        screenshots: PermissionTier::new(appshot_screenshots_permission_granted(), true),
        accessibility: PermissionTier::new(appshot_accessibility_permission_granted(), true),
        // Reflect the current platform provider's real control support so the
        // shared desktop-control status stays aligned with computer-use runtime
        // capabilities.
        input_injection_supported,
    }
}

/// The shared, tiered desktop-control permission status. Single source of truth
/// for both Appshot and computer use.
pub fn desktop_control_permission_status() -> desktop_control::DesktopControlPermissionStatus {
    desktop_control::DesktopControlPermissionStatus::derive(desktop_control_inputs())
}

/// Appshot's view of the permission state, derived from the shared layer so its
/// UX does not regress: it consumes only the screenshot tier and `canObserve`.
fn appshot_permission_status() -> AppshotPermissionStatusDto {
    let status = desktop_control_permission_status();
    AppshotPermissionStatusDto {
        platform: status.platform,
        requires_permission: status.requires_permission,
        screenshots: AppshotPermissionStateDto {
            granted: status.screenshots.granted,
            supported: status.screenshots.supported,
        },
        accessibility: AppshotPermissionStateDto {
            granted: status.accessibility.granted,
            supported: status.accessibility.supported,
        },
        can_capture: status.can_observe,
    }
}

#[cfg(target_os = "macos")]
fn appshot_screenshots_permission_granted() -> bool {
    core_graphics::access::ScreenCaptureAccess.preflight()
}

#[cfg(target_os = "macos")]
fn appshot_request_screenshots_permission() -> bool {
    // Keep screenshots onboarding inside our native panel + drag guide flow
    // instead of triggering the macOS prompt immediately.
    core_graphics::access::ScreenCaptureAccess.preflight()
}

#[cfg(target_os = "macos")]
fn appshot_accessibility_permission_granted() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::CFString;
    use core_foundation::string::CFStringRef;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    }

    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        let value = CFBoolean::false_value();
        let options = CFDictionary::from_CFType_pairs(&[(key, value)]);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}

#[cfg(target_os = "macos")]
fn appshot_request_accessibility_permission() -> bool {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::dictionary::CFDictionaryRef;
    use core_foundation::string::CFString;
    use core_foundation::string::CFStringRef;

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    }

    unsafe {
        let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
        // Keep accessibility onboarding inside our native panel + drag guide flow
        // instead of triggering the macOS prompt immediately.
        let value = CFBoolean::false_value();
        let options = CFDictionary::from_CFType_pairs(&[(key, value)]);
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
    }
}

#[cfg(target_os = "macos")]
mod appshot_permission_panel {
    use std::{
        cell::Cell,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicUsize, Ordering},
            OnceLock,
        },
        thread,
        time::Duration,
    };

    use core::ffi::c_void;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::display::CGDisplay;
    use core_graphics::geometry::{CGPoint, CGRect};
    use core_graphics::window::{
        kCGWindowAlpha, kCGWindowBounds, kCGWindowLayer, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly, kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
    };
    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, ProtocolObject},
        sel, AnyThread, ClassType, DefinedClass, DowncastTarget, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSBezelStyle, NSBezierPath, NSBox, NSBoxType, NSButton,
        NSButtonType, NSCellImagePosition, NSColor, NSDragOperation, NSDraggingContext,
        NSDraggingItem, NSDraggingSession, NSDraggingSource, NSEvent, NSFloatingWindowLevel,
        NSFont, NSImage, NSImageScaling, NSImageView, NSPanel, NSPopUpMenuWindowLevel,
        NSRunningApplication, NSScreen, NSTextAlignment, NSTextField, NSView, NSWindowDelegate,
        NSWindowStyleMask, NSWindowTitleVisibility, NSWorkspace,
    };
    use objc2_foundation::{
        NSArray, NSBundle, NSCopying, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize,
        NSString, NSURL,
    };
    use tauri::AppHandle;

    use super::{
        appshot_accessibility_permission_granted, appshot_request_accessibility_permission,
        appshot_request_screenshots_permission, appshot_screenshots_permission_granted,
        open_appshot_permission_settings_impl, AppshotPermissionKind,
    };

    static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
    static PANEL_PTR: AtomicUsize = AtomicUsize::new(0);
    static CONTROLLER_PTR: AtomicUsize = AtomicUsize::new(0);
    static DRAG_GUIDE_PANEL_PTR: AtomicUsize = AtomicUsize::new(0);
    static DRAG_GUIDE_KIND: AtomicUsize = AtomicUsize::new(0);
    static DRAG_GUIDE_COMPLETED_MASK: AtomicUsize = AtomicUsize::new(0);
    static WATCHER_ACTIVE: AtomicUsize = AtomicUsize::new(0);

    const TAG_SCREENSHOTS_STATUS: isize = 1101;
    const TAG_SCREENSHOTS_BUTTON: isize = 1102;
    const TAG_ACCESSIBILITY_STATUS: isize = 1201;
    const TAG_ACCESSIBILITY_BUTTON: isize = 1202;
    const TAG_DONE_BUTTON: isize = 1301;
    const TAG_SCREENSHOTS_CARD: isize = 1401;
    const TAG_ACCESSIBILITY_CARD: isize = 1402;
    const TAG_SCREENSHOTS_PLACEHOLDER: isize = 1501;
    const TAG_ACCESSIBILITY_PLACEHOLDER: isize = 1502;
    const SYSTEM_SETTINGS_BUNDLE_ID: &str = "com.apple.systempreferences";

    #[derive(Clone)]
    struct DragSourceIvars {
        bundle_url: Retained<NSURL>,
        drag_image: Retained<NSImage>,
        drag_origin: Cell<NSPoint>,
        drag_started: Cell<bool>,
    }

    #[derive(Clone)]
    struct TaggedIvars {
        tag: Cell<isize>,
    }

    struct TargetWindowFrame {
        frame: NSRect,
    }

    fn kind_code(kind: AppshotPermissionKind) -> usize {
        match kind {
            AppshotPermissionKind::Screenshots => 1,
            AppshotPermissionKind::Accessibility => 2,
        }
    }

    fn kind_from_code(code: usize) -> Option<AppshotPermissionKind> {
        match code {
            1 => Some(AppshotPermissionKind::Screenshots),
            2 => Some(AppshotPermissionKind::Accessibility),
            _ => None,
        }
    }

    define_class!(
        #[unsafe(super = NSView)]
        #[thread_kind = MainThreadOnly]
        #[ivars = DragSourceIvars]
        struct AppBundleDragSourceView;

        unsafe impl NSObjectProtocol for AppBundleDragSourceView {}

        unsafe impl NSDraggingSource for AppBundleDragSourceView {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn dragging_session_source_operation_mask_for_dragging_context(
                &self,
                _session: &NSDraggingSession,
                _context: NSDraggingContext,
            ) -> NSDragOperation {
                NSDragOperation::Copy
            }

            #[unsafe(method(draggingSession:endedAtPoint:operation:))]
            fn dragging_session_ended_at_point_operation(
                &self,
                _session: &NSDraggingSession,
                _screen_point: NSPoint,
                _operation: NSDragOperation,
            ) {
                self.ivars().drag_started.set(false);
                if !_operation.is_empty() {
                    finish_drag_guide_after_completed_drop();
                }
            }
        }

        impl AppBundleDragSourceView {
            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, event: &NSEvent) {
                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                self.ivars().drag_origin.set(point);
                self.ivars().drag_started.set(false);
            }

            #[unsafe(method(mouseDragged:))]
            fn mouse_dragged(&self, event: &NSEvent) {
                if self.ivars().drag_started.get() {
                    return;
                }

                let point = self.convertPoint_fromView(event.locationInWindow(), None);
                let start = self.ivars().drag_origin.get();
                let dx = point.x - start.x;
                let dy = point.y - start.y;
                if (dx * dx) + (dy * dy) < 9.0 {
                    return;
                }

                self.ivars().drag_started.set(true);
                self.begin_bundle_drag(event);
            }

            #[unsafe(method(mouseUp:))]
            fn mouse_up(&self, _event: &NSEvent) {
                self.ivars().drag_started.set(false);
            }

            #[unsafe(method(hitTest:))]
            fn hit_test(&self, point: NSPoint) -> *mut NSView {
                if self.mouse_inRect(point, self.bounds()) {
                    self.as_super() as *const NSView as *mut NSView
                } else {
                    std::ptr::null_mut()
                }
            }

            #[unsafe(method(acceptsFirstMouse:))]
            fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
                true
            }

            #[unsafe(method(mouseDownCanMoveWindow))]
            fn mouse_down_can_move_window(&self) -> bool {
                false
            }
        }
    );

    define_class!(
        #[unsafe(super = NSBox)]
        #[thread_kind = MainThreadOnly]
        #[ivars = TaggedIvars]
        struct TaggedBox;

        unsafe impl NSObjectProtocol for TaggedBox {}

        impl TaggedBox {
            #[unsafe(method(tag))]
            fn tag(&self) -> isize {
                self.ivars().tag.get()
            }

            #[unsafe(method(setTag:))]
            fn set_tag(&self, tag: isize) {
                self.ivars().tag.set(tag);
            }
        }
    );

    define_class!(
        #[unsafe(super = NSView)]
        #[thread_kind = MainThreadOnly]
        #[ivars = TaggedIvars]
        struct DashedPlaceholderView;

        unsafe impl NSObjectProtocol for DashedPlaceholderView {}

        impl DashedPlaceholderView {
            #[unsafe(method(tag))]
            fn tag(&self) -> isize {
                self.ivars().tag.get()
            }

            #[unsafe(method(setTag:))]
            fn set_tag(&self, tag: isize) {
                self.ivars().tag.set(tag);
            }

            #[unsafe(method(drawRect:))]
            fn draw_rect(&self, _dirty_rect: NSRect) {
                let bounds = self.bounds();
                let rect = NSRect::new(
                    NSPoint::new(0.5, 0.5),
                    NSSize::new((bounds.size.width - 1.0).max(1.0), (bounds.size.height - 1.0).max(1.0)),
                );
                let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 16.0, 16.0);
                let pattern = [4.0_f64, 5.0_f64];
                unsafe {
                    path.setLineDash_count_phase(pattern.as_ptr(), pattern.len() as isize, 0.0);
                }
                path.setLineWidth(1.0);
                NSColor::quaternaryLabelColor()
                    .colorWithAlphaComponent(0.55)
                    .setStroke();
                path.stroke();
            }
        }
    );

    impl AppBundleDragSourceView {
        fn new(
            mtm: MainThreadMarker,
            frame: NSRect,
            bundle_url: Retained<NSURL>,
            drag_image: Retained<NSImage>,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(DragSourceIvars {
                bundle_url,
                drag_image,
                drag_origin: Cell::new(NSPoint::new(0.0, 0.0)),
                drag_started: Cell::new(false),
            });
            let Some(view) = (unsafe { msg_send![super(this), initWithFrame: frame] }) else {
                unreachable!("AppBundleDragSourceView initWithFrame returned nil");
            };
            view
        }

        fn begin_bundle_drag(&self, event: &NSEvent) {
            let item = NSDraggingItem::initWithPasteboardWriter(
                NSDraggingItem::alloc(),
                ProtocolObject::<dyn objc2_app_kit::NSPasteboardWriting>::from_ref(
                    &*self.ivars().bundle_url,
                ),
            );
            let drag_image = self.ivars().drag_image.copy();
            let image_size = drag_image.size();
            let preview_width = image_size.width.max(36.0);
            let preview_height = image_size.height.max(36.0);
            let location = self.convertPoint_fromView(event.locationInWindow(), None);
            let preview_rect = NSRect::new(
                NSPoint::new(
                    location.x - (preview_width / 2.0),
                    location.y - (preview_height / 2.0),
                ),
                NSSize::new(preview_width, preview_height),
            );
            unsafe {
                item.setDraggingFrame_contents(preview_rect, Some(drag_image.as_ref()));
            }
            let items = NSArray::from_retained_slice(&[item]);
            let _ = self.beginDraggingSessionWithItems_event_source(
                &items,
                event,
                ProtocolObject::from_ref(self),
            );
        }
    }

    impl TaggedBox {
        fn new(mtm: MainThreadMarker, frame: NSRect, tag: isize) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TaggedIvars {
                tag: Cell::new(tag),
            });
            let Some(view) = (unsafe { msg_send![super(this), initWithFrame: frame] }) else {
                unreachable!("TaggedBox initWithFrame returned nil");
            };
            view
        }
    }

    impl DashedPlaceholderView {
        fn new(mtm: MainThreadMarker, frame: NSRect, tag: isize) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(TaggedIvars {
                tag: Cell::new(tag),
            });
            let Some(view) = (unsafe { msg_send![super(this), initWithFrame: frame] }) else {
                unreachable!("DashedPlaceholderView initWithFrame returned nil");
            };
            view
        }
    }

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        struct PermissionPanelController;

        unsafe impl NSObjectProtocol for PermissionPanelController {}

        unsafe impl NSWindowDelegate for PermissionPanelController {
            #[unsafe(method(windowWillClose:))]
            fn window_will_close(&self, _notification: &NSNotification) {
                release_handles();
            }
        }

        impl PermissionPanelController {
            #[unsafe(method(allowScreenshots:))]
            fn allow_screenshots(&self, _sender: Option<&AnyObject>) {
                request_permission(AppshotPermissionKind::Screenshots);
            }

            #[unsafe(method(allowAccessibility:))]
            fn allow_accessibility(&self, _sender: Option<&AnyObject>) {
                request_permission(AppshotPermissionKind::Accessibility);
            }

            #[unsafe(method(donePressed:))]
            fn done_pressed(&self, _sender: Option<&AnyObject>) {
                close_panel();
            }

            #[unsafe(method(backFromDragGuide:))]
            fn back_from_drag_guide(&self, _sender: Option<&AnyObject>) {
                cancel_drag_guide();
            }
        }
    );

    impl PermissionPanelController {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

    fn panel_ptr() -> Option<*mut NSPanel> {
        let ptr = PANEL_PTR.load(Ordering::Acquire);
        if ptr == 0 {
            None
        } else {
            Some(ptr as *mut NSPanel)
        }
    }

    fn drag_guide_panel_ptr() -> Option<*mut NSPanel> {
        let ptr = DRAG_GUIDE_PANEL_PTR.load(Ordering::Acquire);
        if ptr == 0 {
            None
        } else {
            Some(ptr as *mut NSPanel)
        }
    }

    fn content_view() -> Option<Retained<NSView>> {
        let panel = panel_ptr()?;
        unsafe { (&*panel).contentView() }
    }

    fn view_by_tag<T: DowncastTarget>(root: &NSView, tag: isize) -> Option<Retained<T>> {
        root.viewWithTag(tag)
            .and_then(|view| view.downcast::<T>().ok())
    }

    fn bundle_file_url(bundle_path: &Path) -> Result<Retained<NSURL>, String> {
        NSURL::from_file_path(bundle_path).ok_or_else(|| {
            format!(
                "Could not convert bundle path to file URL: {}",
                bundle_path.display()
            )
        })
    }

    fn app_bundle_path() -> Option<PathBuf> {
        let bundle = NSBundle::mainBundle();
        let bundle_path = bundle.bundlePath().to_string();
        let bundle_path = PathBuf::from(bundle_path);
        if bundle_path.extension().and_then(|ext| ext.to_str()) == Some("app") {
            return Some(bundle_path);
        }

        let exe = std::env::current_exe().ok()?;
        for ancestor in exe.ancestors() {
            if ancestor.extension().and_then(|ext| ext.to_str()) == Some("app") {
                return Some(ancestor.to_path_buf());
            }
        }

        let bundle = PathBuf::from(std::env::var_os("TAURI_BUNDLE_PATH")?);
        if bundle.extension().and_then(|ext| ext.to_str()) == Some("app") {
            return Some(bundle);
        }
        None
    }

    fn permission_status(kind: AppshotPermissionKind) -> (bool, &'static str, Retained<NSColor>) {
        match kind {
            AppshotPermissionKind::Screenshots => {
                if appshot_screenshots_permission_granted() {
                    (true, "Allowed", NSColor::systemGreenColor())
                } else {
                    (false, "Required", NSColor::systemOrangeColor())
                }
            }
            AppshotPermissionKind::Accessibility => {
                if appshot_accessibility_permission_granted() {
                    (true, "Allowed", NSColor::systemGreenColor())
                } else {
                    (false, "Optional", NSColor::secondaryLabelColor())
                }
            }
        }
    }

    fn permission_granted(kind: AppshotPermissionKind) -> bool {
        match kind {
            AppshotPermissionKind::Screenshots => appshot_screenshots_permission_granted(),
            AppshotPermissionKind::Accessibility => appshot_accessibility_permission_granted(),
        }
    }

    fn guide_message(kind: AppshotPermissionKind) -> &'static str {
        match kind {
            AppshotPermissionKind::Screenshots => {
                "Drag Sessio to the list above to allow Screenshots"
            }
            AppshotPermissionKind::Accessibility => {
                "Drag Sessio to the list above to allow Accessibility"
            }
        }
    }

    fn app_display_name(bundle_path: &Path) -> String {
        bundle_path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Sessio")
            .to_string()
    }

    fn dict_value(dict: &CFDictionary, key: *const c_void) -> Option<CFType> {
        dict.find(key)
            .map(|value| unsafe { CFType::wrap_under_get_rule(*value) })
    }

    fn dict_number_i64(dict: &CFDictionary, key: *const c_void) -> Option<i64> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_i64())
    }

    fn dict_number_f64(dict: &CFDictionary, key: *const c_void) -> Option<f64> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|value| value.to_f64())
    }

    fn dict_string(dict: &CFDictionary, key: *const c_void) -> Option<String> {
        dict_value(dict, key)
            .and_then(|value| value.downcast::<CFString>())
            .map(|value| value.to_string())
    }

    fn system_settings_pid() -> Option<i32> {
        let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
            &NSString::from_str(SYSTEM_SETTINGS_BUNDLE_ID),
        );
        apps.iter()
            .find_map(|app| i32::try_from(app.processIdentifier()).ok())
    }

    fn rect_contains_point(rect: CGRect, point: CGPoint) -> bool {
        point.x >= rect.origin.x
            && point.x <= rect.origin.x + rect.size.width
            && point.y >= rect.origin.y
            && point.y <= rect.origin.y + rect.size.height
    }

    fn rects_nearly_equal(a: CGRect, b: NSRect) -> bool {
        (a.origin.x - b.origin.x).abs() < 1.0
            && (a.size.width - b.size.width).abs() < 1.0
            && (a.size.height - b.size.height).abs() < 1.0
    }

    fn appkit_frame_from_cg_window_bounds(bounds: CGRect, mtm: MainThreadMarker) -> Option<NSRect> {
        let window_center = CGPoint::new(
            bounds.origin.x + (bounds.size.width / 2.0),
            bounds.origin.y + (bounds.size.height / 2.0),
        );
        let display_bounds = CGDisplay::active_displays()
            .ok()
            .and_then(|display_ids| {
                display_ids.into_iter().find_map(|display_id| {
                    let display = CGDisplay::new(display_id);
                    let display_bounds = display.bounds();
                    rect_contains_point(display_bounds, window_center).then_some(display_bounds)
                })
            })
            .unwrap_or_else(|| CGDisplay::main().bounds());

        let screen = NSScreen::screens(mtm)
            .iter()
            .find(|screen| rects_nearly_equal(display_bounds, screen.frame()))
            .or_else(|| NSScreen::mainScreen(mtm))?;
        let screen_frame = screen.frame();

        let y_from_screen_top = bounds.origin.y - display_bounds.origin.y;
        let appkit_y = screen_frame.origin.y + screen_frame.size.height
            - y_from_screen_top
            - bounds.size.height;
        Some(NSRect::new(
            NSPoint::new(
                screen_frame.origin.x + (bounds.origin.x - display_bounds.origin.x),
                appkit_y,
            ),
            NSSize::new(bounds.size.width, bounds.size.height),
        ))
    }

    fn system_settings_window_frame() -> Option<TargetWindowFrame> {
        let target_pid = system_settings_pid()?;
        let windows = CGDisplay::window_list_info(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            None,
        )?;
        let key_owner_pid = unsafe { kCGWindowOwnerPID as *const c_void };
        let key_owner_name = unsafe { kCGWindowOwnerName as *const c_void };
        let key_window_layer = unsafe { kCGWindowLayer as *const c_void };
        let key_window_alpha = unsafe { kCGWindowAlpha as *const c_void };
        let key_window_bounds = unsafe { kCGWindowBounds as *const c_void };
        let _key_window_number = unsafe { kCGWindowNumber as *const c_void };

        let mut best_frame: Option<CGRect> = None;
        let mut best_area = 0.0_f64;

        for value in &windows {
            let cf_type = unsafe { CFType::wrap_under_get_rule(*value) };
            let Some(dict) = cf_type.downcast::<CFDictionary>() else {
                continue;
            };

            let pid = dict_number_i64(&dict, key_owner_pid).unwrap_or_default();
            if pid != i64::from(target_pid) {
                continue;
            }

            let owner_name = dict_string(&dict, key_owner_name).unwrap_or_default();
            if owner_name != "System Settings" && owner_name != "System Preferences" {
                continue;
            }

            let layer = dict_number_i64(&dict, key_window_layer).unwrap_or_default();
            if layer != 0 {
                continue;
            }

            let alpha = dict_number_f64(&dict, key_window_alpha).unwrap_or(1.0);
            if alpha <= 0.0 {
                continue;
            }

            let Some(bounds_cf) = dict_value(&dict, key_window_bounds) else {
                continue;
            };
            let Some(bounds_dict) = bounds_cf.downcast::<CFDictionary>() else {
                continue;
            };
            let Some(bounds) = CGRect::from_dict_representation(&bounds_dict) else {
                continue;
            };
            if bounds.is_empty() || bounds.size.width < 80.0 || bounds.size.height < 80.0 {
                continue;
            }

            let area = bounds.size.width * bounds.size.height;
            if area > best_area {
                best_area = area;
                best_frame = Some(bounds);
            }
        }

        let bounds = best_frame?;
        let mtm = MainThreadMarker::new()?;
        Some(TargetWindowFrame {
            frame: appkit_frame_from_cg_window_bounds(bounds, mtm)?,
        })
    }

    fn drag_guide_origin_for_target(target: &TargetWindowFrame, panel_frame: NSRect) -> NSPoint {
        let margin = 18.0;
        NSPoint::new(
            target.frame.origin.x + target.frame.size.width - panel_frame.size.width - margin,
            target.frame.origin.y + margin,
        )
    }

    fn permission_card_tag(kind: AppshotPermissionKind) -> isize {
        match kind {
            AppshotPermissionKind::Screenshots => TAG_SCREENSHOTS_CARD,
            AppshotPermissionKind::Accessibility => TAG_ACCESSIBILITY_CARD,
        }
    }

    fn drag_guide_origin_for_permission_card(
        kind: AppshotPermissionKind,
        panel_frame: NSRect,
    ) -> Option<NSPoint> {
        let root = content_view()?;
        let card = view_by_tag::<NSView>(&root, permission_card_tag(kind))?;
        let window = card.window()?;
        let card_rect = card.convertRect_toView(card.bounds(), None);
        let screen_rect = window.convertRectToScreen(card_rect);

        Some(NSPoint::new(
            screen_rect.origin.x + ((screen_rect.size.width - panel_frame.size.width) / 2.0),
            screen_rect.origin.y + ((screen_rect.size.height - panel_frame.size.height) / 2.0),
        ))
    }

    fn drag_guide_fallback_origin(panel_frame: NSRect, mtm: MainThreadMarker) -> NSPoint {
        let Some(screen) = NSScreen::mainScreen(mtm) else {
            return NSPoint::new(0.0, 0.0);
        };
        let visible = screen.visibleFrame();
        NSPoint::new(
            visible.origin.x + ((visible.size.width - panel_frame.size.width) / 2.0).max(0.0),
            visible.origin.y + 22.0,
        )
    }

    fn drag_guide_final_origin(panel_frame: NSRect, mtm: MainThreadMarker) -> NSPoint {
        system_settings_window_frame()
            .map(|target| drag_guide_origin_for_target(&target, panel_frame))
            .unwrap_or_else(|| drag_guide_fallback_origin(panel_frame, mtm))
    }

    fn set_view_frame_if_needed(view: &NSView, frame: NSRect) {
        let current = view.frame();
        if (current.origin.x - frame.origin.x).abs() < 0.5
            && (current.origin.y - frame.origin.y).abs() < 0.5
            && (current.size.width - frame.size.width).abs() < 0.5
            && (current.size.height - frame.size.height).abs() < 0.5
        {
            return;
        }
        view.setFrame(frame);
    }

    fn release_handles() {
        WATCHER_ACTIVE.store(0, Ordering::Release);
        DRAG_GUIDE_COMPLETED_MASK.store(0, Ordering::Release);
        close_drag_guide_panel();

        let panel_ptr = PANEL_PTR.swap(0, Ordering::AcqRel);
        if panel_ptr != 0 {
            let _ = unsafe { Retained::from_raw(panel_ptr as *mut NSPanel) };
        }

        let controller_ptr = CONTROLLER_PTR.swap(0, Ordering::AcqRel);
        if controller_ptr != 0 {
            let _ = unsafe { Retained::from_raw(controller_ptr as *mut PermissionPanelController) };
        }
    }

    fn close_panel() {
        WATCHER_ACTIVE.store(0, Ordering::Release);
        DRAG_GUIDE_COMPLETED_MASK.store(0, Ordering::Release);
        close_drag_guide_panel();

        let panel_ptr = PANEL_PTR.swap(0, Ordering::AcqRel);
        let controller_ptr = CONTROLLER_PTR.swap(0, Ordering::AcqRel);

        if panel_ptr != 0 {
            if let Some(panel) = unsafe { Retained::from_raw(panel_ptr as *mut NSPanel) } {
                panel.close();
            }
        }

        if controller_ptr != 0 {
            let _ = unsafe { Retained::from_raw(controller_ptr as *mut PermissionPanelController) };
        }
    }

    fn close_drag_guide_panel() {
        DRAG_GUIDE_KIND.store(0, Ordering::Release);

        let panel_ptr = DRAG_GUIDE_PANEL_PTR.swap(0, Ordering::AcqRel);
        if panel_ptr != 0 {
            if let Some(panel) = unsafe { Retained::from_raw(panel_ptr as *mut NSPanel) } {
                panel.close();
            }
        }
    }

    fn cancel_drag_guide() {
        let Some(kind) = kind_from_code(DRAG_GUIDE_KIND.load(Ordering::Acquire)) else {
            close_drag_guide_panel();
            update_panel_state();
            return;
        };
        let code = kind_code(kind);
        let panel_ptr = DRAG_GUIDE_PANEL_PTR.load(Ordering::Acquire);

        DRAG_GUIDE_KIND.store(0, Ordering::Release);
        DRAG_GUIDE_COMPLETED_MASK.fetch_and(!code, Ordering::AcqRel);

        if panel_ptr != 0 {
            if let Some(mtm) = MainThreadMarker::new() {
                let panel = unsafe { &*(panel_ptr as *mut NSPanel) };
                let frame = panel.frame();
                let target_origin = drag_guide_origin_for_permission_card(kind, frame)
                    .unwrap_or_else(|| drag_guide_fallback_origin(frame, mtm));
                panel.setFrame_display_animate(NSRect::new(target_origin, frame.size), true, true);
            }
        }

        let Some(app) = APP_HANDLE.get().cloned() else {
            close_drag_guide_panel();
            update_panel_state();
            return;
        };

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(280));
            let _ = app.run_on_main_thread(move || {
                if DRAG_GUIDE_PANEL_PTR.load(Ordering::Acquire) == panel_ptr {
                    close_drag_guide_panel();
                    update_panel_state();
                }
            });
        });
    }

    fn mark_active_drag_guide_completed() {
        let code = DRAG_GUIDE_KIND.load(Ordering::Acquire);
        if code != 0 {
            DRAG_GUIDE_COMPLETED_MASK.fetch_or(code, Ordering::AcqRel);
        }
    }

    fn finish_drag_guide_after_completed_drop() {
        let Some(app) = APP_HANDLE.get().cloned() else {
            mark_active_drag_guide_completed();
            close_drag_guide_panel();
            update_panel_state();
            return;
        };

        thread::spawn(move || {
            thread::sleep(Duration::from_millis(120));
            let _ = app.run_on_main_thread(move || {
                mark_active_drag_guide_completed();
                close_drag_guide_panel();
                update_panel_state();
            });
        });
    }

    fn update_panel_state() {
        let Some(root) = content_view() else {
            return;
        };
        let active_guide_kind = kind_from_code(DRAG_GUIDE_KIND.load(Ordering::Acquire));
        let completed_mask = DRAG_GUIDE_COMPLETED_MASK.load(Ordering::Acquire);
        let screenshots_granted = appshot_screenshots_permission_granted();
        let accessibility_granted = appshot_accessibility_permission_granted();
        let screenshots_guide_active =
            matches!(active_guide_kind, Some(AppshotPermissionKind::Screenshots));
        let accessibility_guide_active = matches!(
            active_guide_kind,
            Some(AppshotPermissionKind::Accessibility)
        );
        let screenshots_drag_completed =
            completed_mask & kind_code(AppshotPermissionKind::Screenshots) != 0;
        let accessibility_drag_completed =
            completed_mask & kind_code(AppshotPermissionKind::Accessibility) != 0;
        let screenshots_card_hidden =
            screenshots_granted || screenshots_guide_active || screenshots_drag_completed;
        let accessibility_card_hidden =
            accessibility_granted || accessibility_guide_active || accessibility_drag_completed;
        let screenshots_placeholder_visible =
            screenshots_guide_active && !screenshots_granted && !screenshots_drag_completed;
        let accessibility_placeholder_visible =
            accessibility_guide_active && !accessibility_granted && !accessibility_drag_completed;

        if let Some(status) = view_by_tag::<NSTextField>(&root, TAG_SCREENSHOTS_STATUS) {
            let (granted, text, color) = permission_status(AppshotPermissionKind::Screenshots);
            status.setStringValue(&NSString::from_str(text));
            status.setTextColor(Some(&color));
            if let Some(button) = view_by_tag::<NSButton>(&root, TAG_SCREENSHOTS_BUTTON) {
                button.setTitle(&NSString::from_str(if granted {
                    "Allowed"
                } else {
                    "Allow"
                }));
                button.setEnabled(!granted);
                button.setHidden(screenshots_card_hidden);
            }
            if let Some(card) = view_by_tag::<NSView>(&root, TAG_SCREENSHOTS_CARD) {
                card.setHidden(screenshots_card_hidden);
            }
        }

        if let Some(status) = view_by_tag::<NSTextField>(&root, TAG_ACCESSIBILITY_STATUS) {
            let (granted, text, color) = permission_status(AppshotPermissionKind::Accessibility);
            status.setStringValue(&NSString::from_str(text));
            status.setTextColor(Some(&color));
            if let Some(button) = view_by_tag::<NSButton>(&root, TAG_ACCESSIBILITY_BUTTON) {
                button.setTitle(&NSString::from_str(if granted {
                    "Allowed"
                } else {
                    "Allow"
                }));
                button.setEnabled(!granted);
                button.setHidden(accessibility_card_hidden);
            }
            if let Some(card) = view_by_tag::<NSView>(&root, TAG_ACCESSIBILITY_CARD) {
                card.setHidden(accessibility_card_hidden);
            }
        }

        if let Some(accessibility_card) = view_by_tag::<NSView>(&root, TAG_ACCESSIBILITY_CARD) {
            let y = 154.0;
            set_view_frame_if_needed(
                accessibility_card.as_ref(),
                NSRect::new(NSPoint::new(40.0, y), NSSize::new(520.0, 80.0)),
            );
        }
        if let Some(screenshots_card) = view_by_tag::<NSView>(&root, TAG_SCREENSHOTS_CARD) {
            let y = 64.0;
            set_view_frame_if_needed(
                screenshots_card.as_ref(),
                NSRect::new(NSPoint::new(40.0, y), NSSize::new(520.0, 80.0)),
            );
        }
        if let Some(placeholder) = view_by_tag::<NSView>(&root, TAG_ACCESSIBILITY_PLACEHOLDER) {
            placeholder.setHidden(!accessibility_placeholder_visible);
            set_view_frame_if_needed(
                placeholder.as_ref(),
                NSRect::new(NSPoint::new(40.0, 154.0), NSSize::new(520.0, 80.0)),
            );
        }
        if let Some(placeholder) = view_by_tag::<NSView>(&root, TAG_SCREENSHOTS_PLACEHOLDER) {
            placeholder.setHidden(!screenshots_placeholder_visible);
            set_view_frame_if_needed(
                placeholder.as_ref(),
                NSRect::new(NSPoint::new(40.0, 64.0), NSSize::new(520.0, 80.0)),
            );
        }

        if let Some(button) = view_by_tag::<NSButton>(&root, TAG_DONE_BUTTON) {
            button.setEnabled(screenshots_granted);
        }
    }

    fn build_button(
        mtm: MainThreadMarker,
        title: &str,
        frame: NSRect,
        tag: isize,
        target: &PermissionPanelController,
        action: objc2::runtime::Sel,
    ) -> Retained<NSButton> {
        let button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(target.as_ref()),
                Some(action),
                mtm,
            )
        };
        button.setFrame(frame);
        button.setBezelStyle(NSBezelStyle::Push);
        button.setTag(tag);
        button
    }

    fn build_permission_card(
        mtm: MainThreadMarker,
        content: &NSView,
        tag: isize,
        icon_kind: AppshotPermissionKind,
        title: &str,
        description: &str,
        frame: NSRect,
        status_tag: isize,
        button_tag: isize,
        target: &PermissionPanelController,
        action: objc2::runtime::Sel,
    ) {
        let card = TaggedBox::new(mtm, frame, tag);
        card.setBoxType(NSBoxType::Custom);
        card.setBorderWidth(1.0);
        card.setCornerRadius(16.0);
        card.setBorderColor(&NSColor::separatorColor());
        card.setFillColor(&NSColor::controlBackgroundColor());
        card.setTransparent(false);
        card.setContentViewMargins(NSSize::new(0.0, 0.0));

        let card_content = NSView::new(mtm);
        card_content.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));

        let icon_ring = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(16.0, 13.0), NSSize::new(54.0, 54.0)),
        );
        icon_ring.setBoxType(NSBoxType::Custom);
        icon_ring.setBorderWidth(1.0);
        icon_ring.setCornerRadius(27.0);
        icon_ring.setBorderColor(&NSColor::separatorColor());
        icon_ring.setFillColor(&NSColor::clearColor());
        icon_ring.setContentViewMargins(NSSize::new(0.0, 0.0));

        let icon_frame = NSRect::new(NSPoint::new(7.0, 7.0), NSSize::new(40.0, 40.0));
        match icon_kind {
            AppshotPermissionKind::Accessibility => {
                let icon_fill = NSBox::initWithFrame(NSBox::alloc(mtm), icon_frame);
                icon_fill.setBoxType(NSBoxType::Custom);
                icon_fill.setBorderWidth(0.0);
                icon_fill.setCornerRadius(20.0);
                icon_fill.setFillColor(&NSColor::controlAccentColor());
                icon_fill.setContentViewMargins(NSSize::new(0.0, 0.0));

                let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("figure.stand"),
                    None,
                )
                .unwrap_or_else(|| {
                    NSImage::initWithSize(NSImage::alloc(), NSSize::new(20.0, 20.0))
                });
                icon.setSize(NSSize::new(28.0, 28.0));
                let icon_view = NSImageView::imageViewWithImage(&icon, mtm);
                icon_view.setFrame(NSRect::new(NSPoint::new(6.0, 6.0), NSSize::new(28.0, 28.0)));
                icon_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                icon_view.setContentTintColor(Some(&NSColor::whiteColor()));
                icon_fill.addSubview(&icon_view);
                icon_ring.addSubview(&icon_fill);
            }
            AppshotPermissionKind::Screenshots => {
                let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("camera.viewfinder"),
                    None,
                )
                .unwrap_or_else(|| {
                    NSImage::initWithSize(NSImage::alloc(), NSSize::new(28.0, 28.0))
                });
                icon.setSize(NSSize::new(34.0, 34.0));
                let icon_view = NSImageView::imageViewWithImage(&icon, mtm);
                icon_view.setFrame(NSRect::new(
                    NSPoint::new(10.0, 10.0),
                    NSSize::new(34.0, 34.0),
                ));
                icon_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                icon_view.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                icon_ring.addSubview(&icon_view);
            }
        }

        let title_label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
        title_label.setFrame(NSRect::new(
            NSPoint::new(84.0, frame.size.height - 42.0),
            NSSize::new(250.0, 24.0),
        ));
        title_label.setFont(Some(&NSFont::boldSystemFontOfSize(15.0)));

        let description_label = NSTextField::labelWithString(&NSString::from_str(description), mtm);
        description_label.setFrame(NSRect::new(
            NSPoint::new(84.0, 20.0),
            NSSize::new(290.0, 20.0),
        ));
        description_label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        description_label.setFont(Some(&NSFont::systemFontOfSize(13.0)));

        let status_label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        status_label.setFrame(NSRect::new(
            NSPoint::new(frame.size.width - 170.0, frame.size.height - 42.0),
            NSSize::new(72.0, 22.0),
        ));
        status_label.setTag(status_tag);
        status_label.setHidden(true);

        let button = build_button(
            mtm,
            "Allow",
            NSRect::new(
                NSPoint::new(frame.size.width - 78.0, 28.0),
                NSSize::new(56.0, 26.0),
            ),
            button_tag,
            target,
            action,
        );
        button.setFont(Some(&NSFont::systemFontOfSize(13.0)));

        card_content.addSubview(&icon_ring);
        card_content.addSubview(&title_label);
        card_content.addSubview(&description_label);
        card_content.addSubview(&status_label);
        card_content.addSubview(&button);
        card.setContentView(Some(&card_content));
        content.addSubview(&card.into_super());
    }

    fn build_panel(
        mtm: MainThreadMarker,
        bundle_path: &Path,
        controller: &PermissionPanelController,
    ) -> Retained<NSPanel> {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(600.0, 460.0));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::FullSizeContentView
            | NSWindowStyleMask::UtilityWindow;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        );

        let content = NSView::new(mtm);
        content.setFrame(frame);

        let workspace = NSWorkspace::sharedWorkspace();
        let path_string = NSString::from_str(&bundle_path.to_string_lossy());
        let app_icon = workspace.iconForFile(&path_string);
        app_icon.setSize(NSSize::new(48.0, 48.0));
        let app_icon_view = NSImageView::imageViewWithImage(&app_icon, mtm);
        app_icon_view.setFrame(NSRect::new(
            NSPoint::new((frame.size.width - 48.0) / 2.0, 366.0),
            NSSize::new(48.0, 48.0),
        ));

        let title = NSTextField::labelWithString(&NSString::from_str("Enable Appshots"), mtm);
        title.setFrame(NSRect::new(
            NSPoint::new((frame.size.width - 240.0) / 2.0, 318.0),
            NSSize::new(240.0, 40.0),
        ));
        title.setAlignment(NSTextAlignment::Center);
        title.setFont(Some(&NSFont::boldSystemFontOfSize(24.0)));

        let subtitle = NSTextField::wrappingLabelWithString(
            &NSString::from_str(
                "Codex needs these permissions to take appshots. Appshots are captured when you attach from the + menu or press both command keys simultaneously.",
            ),
            mtm,
        );
        subtitle.setFrame(NSRect::new(
            NSPoint::new(48.0, 264.0),
            NSSize::new(504.0, 44.0),
        ));
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        subtitle.setAlignment(NSTextAlignment::Center);
        subtitle.setFont(Some(&NSFont::systemFontOfSize(13.0)));

        build_permission_card(
            mtm,
            &content,
            TAG_ACCESSIBILITY_CARD,
            AppshotPermissionKind::Accessibility,
            "Accessibility",
            "Allows Codex to read the text in apps",
            NSRect::new(NSPoint::new(40.0, 154.0), NSSize::new(520.0, 80.0)),
            TAG_ACCESSIBILITY_STATUS,
            TAG_ACCESSIBILITY_BUTTON,
            controller,
            sel!(allowAccessibility:),
        );

        build_permission_card(
            mtm,
            &content,
            TAG_SCREENSHOTS_CARD,
            AppshotPermissionKind::Screenshots,
            "Screenshots",
            "Allows Codex to see the visuals in apps",
            NSRect::new(NSPoint::new(40.0, 64.0), NSSize::new(520.0, 80.0)),
            TAG_SCREENSHOTS_STATUS,
            TAG_SCREENSHOTS_BUTTON,
            controller,
            sel!(allowScreenshots:),
        );

        let accessibility_placeholder = DashedPlaceholderView::new(
            mtm,
            NSRect::new(NSPoint::new(40.0, 154.0), NSSize::new(520.0, 80.0)),
            TAG_ACCESSIBILITY_PLACEHOLDER,
        );
        accessibility_placeholder.setHidden(true);
        let accessibility_placeholder_label =
            NSTextField::labelWithString(&NSString::from_str("COMPLETE IN SYSTEM SETTINGS"), mtm);
        accessibility_placeholder_label.setFrame(NSRect::new(
            NSPoint::new(0.0, 28.0),
            NSSize::new(520.0, 22.0),
        ));
        accessibility_placeholder_label.setAlignment(NSTextAlignment::Center);
        accessibility_placeholder_label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        accessibility_placeholder_label.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.23)));
        accessibility_placeholder.addSubview(&accessibility_placeholder_label);

        let screenshots_placeholder = DashedPlaceholderView::new(
            mtm,
            NSRect::new(NSPoint::new(40.0, 64.0), NSSize::new(520.0, 80.0)),
            TAG_SCREENSHOTS_PLACEHOLDER,
        );
        screenshots_placeholder.setHidden(true);
        let screenshots_placeholder_label =
            NSTextField::labelWithString(&NSString::from_str("COMPLETE IN SYSTEM SETTINGS"), mtm);
        screenshots_placeholder_label.setFrame(NSRect::new(
            NSPoint::new(0.0, 28.0),
            NSSize::new(520.0, 22.0),
        ));
        screenshots_placeholder_label.setAlignment(NSTextAlignment::Center);
        screenshots_placeholder_label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        screenshots_placeholder_label.setFont(Some(&NSFont::systemFontOfSize_weight(13.0, 0.23)));
        screenshots_placeholder.addSubview(&screenshots_placeholder_label);

        let done_button = build_button(
            mtm,
            "Done",
            NSRect::new(NSPoint::new(470.0, 18.0), NSSize::new(88.0, 30.0)),
            TAG_DONE_BUTTON,
            controller,
            sel!(donePressed:),
        );
        done_button.setHidden(true);

        content.addSubview(&app_icon_view);
        content.addSubview(&title);
        content.addSubview(&subtitle);
        content.addSubview(accessibility_placeholder.as_super());
        content.addSubview(screenshots_placeholder.as_super());
        content.addSubview(&done_button);

        panel.setTitle(&NSString::from_str("Enable Appshots"));
        panel.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        panel.setTitlebarAppearsTransparent(true);
        panel.setMovableByWindowBackground(true);
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(false);
        panel.setWorksWhenModal(true);
        panel.setLevel(NSFloatingWindowLevel);
        panel.setHidesOnDeactivate(false);
        panel.setHasShadow(true);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
        panel.setContentView(Some(&content));
        panel.setDelegate(Some(ProtocolObject::from_ref(controller)));
        panel.center();
        unsafe {
            panel.setReleasedWhenClosed(false);
        }

        update_panel_state();
        panel
    }

    fn build_drag_guide_panel(
        mtm: MainThreadMarker,
        bundle_path: &Path,
        kind: AppshotPermissionKind,
        controller: &PermissionPanelController,
    ) -> Retained<NSPanel> {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(520.0, 110.0));
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            false,
        );

        let content = NSView::new(mtm);
        content.setFrame(frame);

        let background = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
        );
        background.setBoxType(NSBoxType::Custom);
        background.setBorderWidth(0.0);
        background.setCornerRadius(16.0);
        background.setFillColor(&NSColor::windowBackgroundColor());
        background.setTransparent(false);
        background.setContentViewMargins(NSSize::new(0.0, 0.0));
        let background_content = NSView::new(mtm);
        background_content.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));

        let back_icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("chevron.left"),
            Some(&NSString::from_str("Back")),
        )
        .unwrap_or_else(|| NSImage::initWithSize(NSImage::alloc(), NSSize::new(14.0, 14.0)));
        back_icon.setSize(NSSize::new(14.0, 14.0));
        let back_button = unsafe {
            NSButton::buttonWithImage_target_action(
                &back_icon,
                Some(controller.as_ref()),
                Some(sel!(backFromDragGuide:)),
                mtm,
            )
        };
        back_button.setFrame(NSRect::new(
            NSPoint::new(18.0, 30.0),
            NSSize::new(34.0, 34.0),
        ));
        back_button.setBezelStyle(NSBezelStyle::Circular);
        back_button.setButtonType(NSButtonType::MomentaryChange);
        back_button.setImagePosition(NSCellImagePosition::ImageOnly);
        back_button.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        background_content.addSubview(&back_button);

        let workspace = NSWorkspace::sharedWorkspace();
        let path_string = NSString::from_str(&bundle_path.to_string_lossy());
        let app_name = app_display_name(bundle_path);
        let icon = workspace.iconForFile(&path_string);
        icon.setSize(NSSize::new(28.0, 28.0));

        let arrow_icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("arrow.up"),
            None,
        )
        .unwrap_or_else(|| icon.copy());
        let arrow_view = NSImageView::imageViewWithImage(&arrow_icon, mtm);
        arrow_view.setFrame(NSRect::new(
            NSPoint::new(72.0, 76.0),
            NSSize::new(20.0, 20.0),
        ));
        arrow_view.setContentTintColor(Some(&NSColor::controlAccentColor()));
        background_content.addSubview(&arrow_view);

        let detail = NSTextField::labelWithString(&NSString::from_str(guide_message(kind)), mtm);
        detail.setFrame(NSRect::new(
            NSPoint::new(106.0, 75.0),
            NSSize::new(388.0, 22.0),
        ));
        detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
        detail.setFont(Some(&NSFont::systemFontOfSize(14.0)));
        background_content.addSubview(&detail);

        let card_width = 438.0;
        let card_height = 42.0;
        let card = NSBox::initWithFrame(
            NSBox::alloc(mtm),
            NSRect::new(
                NSPoint::new(64.0, 18.0),
                NSSize::new(card_width, card_height),
            ),
        );
        card.setBoxType(NSBoxType::Custom);
        card.setBorderWidth(1.0);
        card.setCornerRadius(7.0);
        card.setBorderColor(&NSColor::separatorColor());
        card.setFillColor(&NSColor::controlBackgroundColor());
        card.setTransparent(false);
        card.setContentViewMargins(NSSize::new(0.0, 0.0));

        let card_content = NSView::new(mtm);
        card_content.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(card_width, card_height),
        ));

        let app_icon_view = NSImageView::imageViewWithImage(&icon, mtm);
        app_icon_view.setFrame(NSRect::new(
            NSPoint::new(12.0, 7.0),
            NSSize::new(28.0, 28.0),
        ));
        app_icon_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        card_content.addSubview(&app_icon_view);

        let app_label = NSTextField::labelWithString(&NSString::from_str(&app_name), mtm);
        app_label.setFrame(NSRect::new(
            NSPoint::new(52.0, 11.0),
            NSSize::new(330.0, 20.0),
        ));
        app_label.setFont(Some(&NSFont::systemFontOfSize_weight(14.0, 0.3)));
        card_content.addSubview(&app_label);

        card.setContentView(Some(&card_content));

        let overlay = AppBundleDragSourceView::new(
            mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(card_width, card_height)),
            bundle_file_url(bundle_path)
                .unwrap_or_else(|_| NSURL::from_file_path(bundle_path).unwrap()),
            icon.copy(),
        );
        overlay.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(card_width, card_height),
        ));
        if let Some(card_inner) = card.contentView() {
            card_inner.addSubview(&overlay);
        }

        background_content.addSubview(&card.into_super());
        background.setContentView(Some(&background_content));
        content.addSubview(&background.into_super());

        panel.setTitle(&NSString::from_str(""));
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setWorksWhenModal(true);
        panel.setLevel(NSPopUpMenuWindowLevel);
        panel.setHidesOnDeactivate(false);
        panel.setHasShadow(true);
        panel.setOpaque(false);
        panel.setExcludedFromWindowsMenu(true);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setAlphaValue(0.0);
        panel.setContentView(Some(&content));
        unsafe {
            panel.setReleasedWhenClosed(false);
        }

        panel
    }

    fn position_drag_guide_panel(panel: &NSPanel, mtm: MainThreadMarker) {
        let frame = panel.frame();
        let target_origin = drag_guide_final_origin(frame, mtm);
        panel.setFrame_display_animate(NSRect::new(target_origin, frame.size), true, true);
    }

    fn show_panel(bundle_path: &Path) -> Result<(), String> {
        let mtm =
            MainThreadMarker::new().ok_or_else(|| "Must run on the main thread".to_string())?;
        if let Some(existing) = panel_ptr() {
            let panel = unsafe { &*existing };
            panel.makeKeyAndOrderFront(None);
            panel.orderFrontRegardless();
            update_panel_state();
            return Ok(());
        }

        let controller = PermissionPanelController::new(mtm);
        let panel = build_panel(mtm, bundle_path, &controller);
        let app = NSApplication::sharedApplication(mtm);

        panel.makeKeyAndOrderFront(None);
        panel.orderFrontRegardless();
        app.activate();

        CONTROLLER_PTR.store(Retained::into_raw(controller) as usize, Ordering::Release);
        PANEL_PTR.store(Retained::into_raw(panel) as usize, Ordering::Release);
        Ok(())
    }

    fn show_drag_guide_panel(
        bundle_path: &Path,
        kind: AppshotPermissionKind,
    ) -> Result<(), String> {
        let mtm =
            MainThreadMarker::new().ok_or_else(|| "Must run on the main thread".to_string())?;
        if let Some(existing_ptr) = drag_guide_panel_ptr() {
            if kind_from_code(DRAG_GUIDE_KIND.load(Ordering::Acquire)) == Some(kind) {
                let panel = unsafe { &*existing_ptr };
                position_drag_guide_panel(panel, mtm);
                panel.setAlphaValue(1.0);
                panel.orderFrontRegardless();
                return Ok(());
            }
            close_drag_guide_panel();
        }

        let controller_ptr = CONTROLLER_PTR.load(Ordering::Acquire);
        if controller_ptr == 0 {
            return Err("Permission panel controller is not available".to_string());
        }
        let controller = unsafe { &*(controller_ptr as *mut PermissionPanelController) };
        let panel = build_drag_guide_panel(mtm, bundle_path, kind, controller);
        let frame = panel.frame();
        let source_origin = drag_guide_origin_for_permission_card(kind, frame)
            .unwrap_or_else(|| drag_guide_final_origin(frame, mtm));
        let final_origin = drag_guide_final_origin(frame, mtm);
        panel.setFrameOrigin(source_origin);
        panel.setAlphaValue(1.0);
        panel.orderFrontRegardless();

        DRAG_GUIDE_KIND.store(kind_code(kind), Ordering::Release);
        let raw = Retained::into_raw(panel) as usize;
        DRAG_GUIDE_PANEL_PTR.store(raw, Ordering::Release);
        let panel = unsafe { &*(raw as *mut NSPanel) };
        panel.setFrame_display_animate(NSRect::new(final_origin, frame.size), true, true);
        Ok(())
    }

    fn bring_drag_guide_to_front() {
        if let Some(panel_ptr) = drag_guide_panel_ptr() {
            let panel = unsafe { &*panel_ptr };
            panel.orderFrontRegardless();
        }
    }

    fn show_drag_guide(kind: AppshotPermissionKind) {
        let Some(app) = APP_HANDLE.get().cloned() else {
            return;
        };
        let Some(bundle_path) = app_bundle_path() else {
            return;
        };

        DRAG_GUIDE_COMPLETED_MASK.fetch_and(!kind_code(kind), Ordering::AcqRel);
        DRAG_GUIDE_KIND.store(kind_code(kind), Ordering::Release);
        update_panel_state();

        thread::spawn(move || {
            for delay_ms in [260_u64, 700, 1400] {
                thread::sleep(Duration::from_millis(delay_ms));
                if PANEL_PTR.load(Ordering::Acquire) == 0
                    || kind_from_code(DRAG_GUIDE_KIND.load(Ordering::Acquire)) != Some(kind)
                    || permission_granted(kind)
                {
                    break;
                }
                let app = app.clone();
                let guide_path = bundle_path.clone();
                let _ = app.run_on_main_thread(move || {
                    if PANEL_PTR.load(Ordering::Acquire) == 0 {
                        close_drag_guide_panel();
                        return;
                    }
                    if kind_from_code(DRAG_GUIDE_KIND.load(Ordering::Acquire)) != Some(kind) {
                        return;
                    }
                    if permission_granted(kind) {
                        close_drag_guide_panel();
                        update_panel_state();
                        return;
                    }
                    let _ = show_drag_guide_panel(&guide_path, kind);
                    bring_drag_guide_to_front();
                });
            }
        });
    }

    fn request_permission(kind: AppshotPermissionKind) {
        let granted = match kind {
            AppshotPermissionKind::Screenshots => appshot_request_screenshots_permission(),
            AppshotPermissionKind::Accessibility => appshot_request_accessibility_permission(),
        };

        if !granted {
            let _ = open_appshot_permission_settings_impl(kind);
            show_drag_guide(kind);
        }

        update_panel_state();
    }

    fn spawn_state_watcher(app: AppHandle) {
        if WATCHER_ACTIVE.swap(1, Ordering::AcqRel) != 0 {
            return;
        }

        thread::spawn(move || loop {
            if PANEL_PTR.load(Ordering::Acquire) == 0 {
                WATCHER_ACTIVE.store(0, Ordering::Release);
                break;
            }

            let screenshots_granted = appshot_screenshots_permission_granted();
            let accessibility_granted = appshot_accessibility_permission_granted();
            let drag_guide_granted = kind_from_code(DRAG_GUIDE_KIND.load(Ordering::Acquire))
                .map(|kind| match kind {
                    AppshotPermissionKind::Screenshots => screenshots_granted,
                    AppshotPermissionKind::Accessibility => accessibility_granted,
                })
                .unwrap_or(false);

            let _ = app.run_on_main_thread(move || {
                if PANEL_PTR.load(Ordering::Acquire) == 0 {
                    return;
                }
                if drag_guide_granted {
                    close_drag_guide_panel();
                }
                update_panel_state();
                if screenshots_granted && accessibility_granted {
                    return;
                }
            });

            thread::sleep(Duration::from_secs(1));
        });
    }

    pub fn show(app: AppHandle) -> Result<(), String> {
        let _ = APP_HANDLE.set(app.clone());
        let bundle_path = app_bundle_path().ok_or_else(|| {
            "Could not resolve Sessio.app bundle path for native permission panel".to_string()
        })?;

        app.run_on_main_thread(move || {
            let _ = show_panel(&bundle_path);
        })
        .map_err(|e| e.to_string())?;
        spawn_state_watcher(app);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn open_appshot_permission_settings_impl(kind: AppshotPermissionKind) -> Result<(), String> {
    let url = match kind {
        AppshotPermissionKind::Screenshots => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
        AppshotPermissionKind::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
    };
    std::process::Command::new("open")
        .arg(url)
        .status()
        .map_err(|e| format!("Failed to open System Settings: {e}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "System Settings returned an unexpected status {status}"
                ))
            }
        })
}

#[cfg(not(target_os = "macos"))]
fn open_appshot_permission_settings_impl(_kind: AppshotPermissionKind) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
async fn capture_window_area_png(
    window: WebviewWindow,
    req: CaptureWindowAreaRequest,
) -> Result<SavedPastedAttachment, String> {
    if !req.x.is_finite()
        || !req.y.is_finite()
        || !req.width.is_finite()
        || !req.height.is_finite()
        || req.width <= 0.0
        || req.height <= 0.0
    {
        return Err("Invalid snapshot capture area".to_string());
    }

    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_name = safe_pasted_attachment_file_name(req.file_name.as_deref(), Some("image/png"));
    let path = dir.join(format!(
        "native-window-area-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));

    let saved = capture_webview2_area_png(&window, &req, &path).await?;
    let meta = std::fs::metadata(&saved.path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(&saved.path);
        return Err("WebView2 snapshot produced an empty PNG".to_string());
    }
    Ok(saved)
}

#[cfg(windows)]
async fn capture_webview2_area_png(
    window: &WebviewWindow,
    req: &CaptureWindowAreaRequest,
    path: &Path,
) -> Result<SavedPastedAttachment, String> {
    use image::GenericImageView;
    use webview2_com::{
        CapturePreviewCompletedHandler,
        Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
    };
    use windows::Win32::{
        Foundation::HGLOBAL, System::Com::StructuredStorage::CreateStreamOnHGlobal,
    };

    let device_scale = window.scale_factor().map_err(|e| e.to_string())?;
    let device_scale = if device_scale.is_finite() && device_scale > 0.0 {
        device_scale
    } else {
        1.0
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(Vec<u8>, u32, u32), String>>();
    let sender = Arc::new(Mutex::new(Some(tx)));

    window
        .with_webview(move |platform_webview| {
            let immediate_sender = Arc::clone(&sender);

            unsafe {
                let result = (|| -> Result<(), String> {
                    let controller = platform_webview.controller();
                    let mut bounds = windows::Win32::Foundation::RECT::default();
                    controller
                        .Bounds(&mut bounds)
                        .map_err(|e| format!("WebView2 bounds query failed: {e}"))?;
                    let webview_width = (bounds.right - bounds.left).max(0) as u32;
                    let webview_height = (bounds.bottom - bounds.top).max(0) as u32;
                    if webview_width == 0 || webview_height == 0 {
                        return Err("WebView2 snapshot bounds are empty".to_string());
                    }

                    let webview = controller
                        .CoreWebView2()
                        .map_err(|e| format!("WebView2 instance query failed: {e}"))?;
                    let stream = CreateStreamOnHGlobal(HGLOBAL::default(), true)
                        .map_err(|e| format!("WebView2 snapshot stream creation failed: {e}"))?;
                    let stream_for_callback = stream.clone();
                    let callback_sender = Arc::clone(&sender);
                    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                        if let Err(error) = result {
                            send_webview2_snapshot_result(
                                &callback_sender,
                                Err(format!("WebView2 snapshot capture failed: {error}")),
                            );
                            return Ok(());
                        }

                        match read_windows_stream_to_vec(&stream_for_callback) {
                            Ok(bytes) if !bytes.is_empty() => send_webview2_snapshot_result(
                                &callback_sender,
                                Ok((bytes, webview_width, webview_height)),
                            ),
                            Ok(_) => send_webview2_snapshot_result(
                                &callback_sender,
                                Err("WebView2 snapshot returned an empty stream".to_string()),
                            ),
                            Err(error) => {
                                send_webview2_snapshot_result(&callback_sender, Err(error))
                            }
                        }
                        Ok(())
                    }));
                    webview
                        .CapturePreview(
                            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                            &stream,
                            &handler,
                        )
                        .map_err(|e| format!("WebView2 snapshot capture failed to start: {e}"))?;
                    Ok(())
                })();

                if let Err(error) = result {
                    send_webview2_snapshot_result(&immediate_sender, Err(error));
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let (png_bytes, webview_width, webview_height) =
        match tokio::time::timeout(Duration::from_secs(6), rx).await {
            Ok(Ok(Ok(result))) => result,
            Ok(Ok(Err(error))) => return Err(error),
            Ok(Err(_)) => return Err("WebView2 snapshot callback was cancelled".to_string()),
            Err(_) => return Err("WebView2 snapshot timed out".to_string()),
        };

    let image = image::load_from_memory(&png_bytes)
        .map_err(|e| format!("WebView2 snapshot PNG decode failed: {e}"))?;
    let (image_width, image_height) = image.dimensions();
    if image_width == 0 || image_height == 0 {
        return Err("WebView2 snapshot PNG was empty".to_string());
    }

    let viewport_css_width = (webview_width.max(1) as f64 / device_scale).max(1.0);
    let viewport_css_height = (webview_height.max(1) as f64 / device_scale).max(1.0);
    let scale_x = image_width as f64 / viewport_css_width;
    let scale_y = image_height as f64 / viewport_css_height;
    let left = (req.x.max(0.0) * scale_x).floor().max(0.0) as u32;
    let top = (req.y.max(0.0) * scale_y).floor().max(0.0) as u32;
    if left >= image_width || top >= image_height {
        return Err("WebView2 snapshot crop area is outside the webview bounds".to_string());
    }
    let crop_width = (req.width * scale_x)
        .ceil()
        .max(1.0)
        .min((image_width - left) as f64) as u32;
    let crop_height = (req.height * scale_y)
        .ceil()
        .max(1.0)
        .min((image_height - top) as f64) as u32;
    if crop_width == 0 || crop_height == 0 {
        return Err("WebView2 snapshot crop area is empty".to_string());
    }

    let cropped = image.crop_imm(left, top, crop_width, crop_height);
    cropped
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("WebView2 snapshot PNG write failed: {e}"))?;
    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

#[cfg(windows)]
type WebView2SnapshotSender =
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<(Vec<u8>, u32, u32), String>>>>>;

#[cfg(windows)]
fn send_webview2_snapshot_result(
    sender: &WebView2SnapshotSender,
    result: Result<(Vec<u8>, u32, u32), String>,
) {
    if let Ok(mut tx) = sender.lock() {
        if let Some(tx) = tx.take() {
            let _ = tx.send(result);
        }
    }
}

#[cfg(windows)]
fn read_windows_stream_to_vec(
    stream: &windows::Win32::System::Com::IStream,
) -> Result<Vec<u8>, String> {
    use windows::Win32::System::Com::{STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET};

    unsafe {
        let mut stat = STATSTG::default();
        stream
            .Stat(&mut stat, STATFLAG_NONAME)
            .map_err(|e| format!("WebView2 snapshot stream stat failed: {e}"))?;
        if stat.cbSize == 0 {
            return Ok(Vec::new());
        }
        if stat.cbSize > u32::MAX as u64 {
            return Err("WebView2 snapshot stream is too large".to_string());
        }
        stream
            .Seek(0, STREAM_SEEK_SET, None)
            .map_err(|e| format!("WebView2 snapshot stream seek failed: {e}"))?;
        let mut bytes = vec![0_u8; stat.cbSize as usize];
        let mut total_read = 0_usize;
        while total_read < bytes.len() {
            let remaining = bytes.len() - total_read;
            let request = remaining.min(u32::MAX as usize) as u32;
            let mut read = 0_u32;
            stream
                .Read(
                    bytes[total_read..].as_mut_ptr() as *mut core::ffi::c_void,
                    request,
                    Some(&mut read),
                )
                .ok()
                .map_err(|e| format!("WebView2 snapshot stream read failed: {e}"))?;
            if read == 0 {
                break;
            }
            total_read += read as usize;
        }
        bytes.truncate(total_read);
        Ok(bytes)
    }
}

#[cfg(target_os = "linux")]
#[tauri::command]
async fn capture_window_area_png(
    window: WebviewWindow,
    req: CaptureWindowAreaRequest,
) -> Result<SavedPastedAttachment, String> {
    if !req.x.is_finite()
        || !req.y.is_finite()
        || !req.width.is_finite()
        || !req.height.is_finite()
        || req.width <= 0.0
        || req.height <= 0.0
    {
        return Err("Invalid snapshot capture area".to_string());
    }

    let dir = paste_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_name = safe_pasted_attachment_file_name(req.file_name.as_deref(), Some("image/png"));
    let path = dir.join(format!(
        "native-window-area-{}-{file_name}",
        chrono::Utc::now().timestamp_millis()
    ));

    let saved = capture_webkitgtk_area_png(&window, &req, &path).await?;
    let meta = std::fs::metadata(&saved.path).map_err(|e| e.to_string())?;
    if meta.len() == 0 {
        let _ = std::fs::remove_file(&saved.path);
        return Err("WebKitGTK snapshot produced an empty PNG".to_string());
    }
    Ok(saved)
}

#[cfg(target_os = "linux")]
async fn capture_webkitgtk_area_png(
    window: &WebviewWindow,
    req: &CaptureWindowAreaRequest,
    path: &Path,
) -> Result<SavedPastedAttachment, String> {
    use gtk::prelude::WidgetExt;
    use image::GenericImageView;
    use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(Vec<u8>, i32, i32), String>>();
    let sender = Arc::new(Mutex::new(Some(tx)));

    window
        .with_webview(move |platform_webview| {
            let webview = platform_webview.inner();
            let allocation = webview.allocation();
            let webview_width = allocation.width().max(0);
            let webview_height = allocation.height().max(0);
            let immediate_sender = Arc::clone(&sender);
            if webview_width == 0 || webview_height == 0 {
                send_webkitgtk_snapshot_result(
                    &immediate_sender,
                    Err("WebKitGTK snapshot bounds are empty".to_string()),
                );
                return;
            }

            let callback_sender = Arc::clone(&sender);
            webview.snapshot(
                SnapshotRegion::Visible,
                SnapshotOptions::NONE,
                None::<&webkit2gtk::gio::Cancellable>,
                move |result| {
                    let result = result
                        .map_err(|error| format!("WebKitGTK snapshot failed: {error}"))
                        .and_then(|surface| {
                            let mut bytes = Vec::new();
                            surface.write_to_png(&mut bytes).map_err(|error| {
                                format!("WebKitGTK snapshot PNG encoding failed: {error}")
                            })?;
                            if bytes.is_empty() {
                                Err("WebKitGTK snapshot returned an empty PNG".to_string())
                            } else {
                                Ok((bytes, webview_width, webview_height))
                            }
                        });
                    send_webkitgtk_snapshot_result(&callback_sender, result);
                },
            );
        })
        .map_err(|e| e.to_string())?;

    let (png_bytes, webview_width, webview_height) =
        match tokio::time::timeout(Duration::from_secs(6), rx).await {
            Ok(Ok(Ok(result))) => result,
            Ok(Ok(Err(error))) => return Err(error),
            Ok(Err(_)) => return Err("WebKitGTK snapshot callback was cancelled".to_string()),
            Err(_) => return Err("WebKitGTK snapshot timed out".to_string()),
        };

    let image = image::load_from_memory(&png_bytes)
        .map_err(|e| format!("WebKitGTK snapshot PNG decode failed: {e}"))?;
    let (image_width, image_height) = image.dimensions();
    if image_width == 0 || image_height == 0 {
        return Err("WebKitGTK snapshot PNG was empty".to_string());
    }

    let viewport_css_width = webview_width.max(1) as f64;
    let viewport_css_height = webview_height.max(1) as f64;
    let scale_x = image_width as f64 / viewport_css_width;
    let scale_y = image_height as f64 / viewport_css_height;
    let left = (req.x.max(0.0) * scale_x).floor().max(0.0) as u32;
    let top = (req.y.max(0.0) * scale_y).floor().max(0.0) as u32;
    if left >= image_width || top >= image_height {
        return Err("WebKitGTK snapshot crop area is outside the webview bounds".to_string());
    }
    let crop_width = (req.width * scale_x)
        .ceil()
        .max(1.0)
        .min((image_width - left) as f64) as u32;
    let crop_height = (req.height * scale_y)
        .ceil()
        .max(1.0)
        .min((image_height - top) as f64) as u32;
    if crop_width == 0 || crop_height == 0 {
        return Err("WebKitGTK snapshot crop area is empty".to_string());
    }

    let cropped = image.crop_imm(left, top, crop_width, crop_height);
    cropped
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(|e| format!("WebKitGTK snapshot PNG write failed: {e}"))?;
    Ok(SavedPastedAttachment {
        path: path.to_string_lossy().to_string(),
    })
}

#[cfg(target_os = "linux")]
type WebKitGtkSnapshotSender =
    Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<(Vec<u8>, i32, i32), String>>>>>;

#[cfg(target_os = "linux")]
fn send_webkitgtk_snapshot_result(
    sender: &WebKitGtkSnapshotSender,
    result: Result<(Vec<u8>, i32, i32), String>,
) {
    if let Ok(mut tx) = sender.lock() {
        if let Some(tx) = tx.take() {
            let _ = tx.send(result);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
#[tauri::command]
fn capture_window_area_png(
    _window: WebviewWindow,
    _req: CaptureWindowAreaRequest,
) -> Result<SavedPastedAttachment, String> {
    Err("Native area screenshot is not implemented on this platform yet".to_string())
}

#[tauri::command]
fn list_project_files(path: String) -> Result<Vec<String>, String> {
    const MAX_FILES: usize = 20_000;
    const MAX_DEPTH: usize = 12;
    const IGNORED_DIR_NAMES: &[&str] = &[
        ".git",
        ".hg",
        ".svn",
        "node_modules",
        ".pnpm",
        ".yarn",
        "target",
        "dist",
        "build",
        ".next",
        ".nuxt",
        ".turbo",
        ".cache",
        ".parcel-cache",
        ".vercel",
        ".vite",
        ".output",
        ".idea",
        ".vscode",
        "__pycache__",
        ".mypy_cache",
        ".pytest_cache",
        ".tox",
        ".venv",
        "venv",
        ".gradle",
        ".angular",
        ".terraform",
        "DerivedData",
        ".DS_Store",
    ];

    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err("Project path must be absolute".to_string());
    }
    let meta = std::fs::metadata(&root).map_err(|e| e.to_string())?;
    if !meta.is_dir() {
        return Err("Project path is not a directory".to_string());
    }

    let mut entries: Vec<String> = Vec::new();
    let walker = walkdir::WalkDir::new(&root)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            if IGNORED_DIR_NAMES.iter().any(|n| name == *n) {
                return false;
            }
            true
        });

    for entry in walker.flatten() {
        if entry.depth() == 0 {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(&root) else {
            continue;
        };
        // Convert to POSIX-style separators for @pierre/trees.
        let mut rel_str = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if rel_str.is_empty() {
            continue;
        }
        if entry.file_type().is_dir() {
            rel_str.push('/');
        } else if !entry.file_type().is_file() {
            // Skip symlinks, sockets, etc.
            continue;
        }
        entries.push(rel_str);
        if entries.len() >= MAX_FILES {
            break;
        }
    }

    entries.sort();
    Ok(entries)
}

#[tauri::command]
fn get_project_git_status(path: String) -> Result<Vec<GitStatusRow>, String> {
    let root = validate_project_dir(&path)?;
    let output = run_git(
        &root,
        &[
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v1",
            "-uall",
        ],
    )
    .map_err(|e| format!("Failed to run git: {e}"))?;
    if !output.status.success() {
        // Not a git repo or git not available — return empty rather than error.
        return Ok(Vec::new());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut by_path: HashMap<String, String> = HashMap::new();
    for change in parse_git_status_stdout(&stdout) {
        by_path.entry(change.path).or_insert(change.status);
    }
    let mut entries: Vec<GitStatusRow> = Vec::new();
    for (path, status) in by_path {
        entries.push(GitStatusRow { path, status });
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

#[derive(serde::Serialize)]
struct GitStatusRow {
    path: String,
    status: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGitSummary {
    is_repo: bool,
    root: Option<String>,
    branch: Option<String>,
    head: Option<String>,
    upstream: Option<String>,
    ahead: i64,
    behind: i64,
    has_changes: bool,
    staged_count: usize,
    unstaged_count: usize,
    untracked_count: usize,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGitChange {
    path: String,
    original_path: Option<String>,
    status: String,
    staged: bool,
    index_status: String,
    worktree_status: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGitState {
    summary: ProjectGitSummary,
    changes: Vec<ProjectGitChange>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGitCommit {
    hash: String,
    short_hash: String,
    parents: Vec<String>,
    author: String,
    timestamp: i64,
    refs: Vec<String>,
    subject: String,
    message: String,
    pushed: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGitCommitPage {
    commits: Vec<ProjectGitCommit>,
    has_more: bool,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectGitActionResult {
    stdout: String,
    stderr: String,
}

#[tauri::command]
fn get_project_git_summary(path: String) -> Result<ProjectGitSummary, String> {
    let project = validate_project_dir(&path)?;
    let Some(root) = discover_git_root(&project)? else {
        return Ok(empty_project_git_summary());
    };
    let changes = load_project_git_changes(&root)?;
    Ok(build_project_git_summary(&root, &changes))
}

#[tauri::command]
fn get_project_git_state(path: String) -> Result<ProjectGitState, String> {
    let project = validate_project_dir(&path)?;
    let Some(root) = discover_git_root(&project)? else {
        return Ok(ProjectGitState {
            summary: empty_project_git_summary(),
            changes: Vec::new(),
        });
    };
    let changes = load_project_git_changes(&root)?;
    let summary = build_project_git_summary(&root, &changes);
    Ok(ProjectGitState { summary, changes })
}

#[tauri::command]
fn list_project_git_commits(
    path: String,
    offset: usize,
    limit: usize,
) -> Result<ProjectGitCommitPage, String> {
    let project = validate_project_dir(&path)?;
    let Some(root) = discover_git_root(&project)? else {
        return Ok(ProjectGitCommitPage {
            commits: Vec::new(),
            has_more: false,
        });
    };
    let safe_limit = limit.clamp(1, 100);
    let request_limit = safe_limit + 1;
    let pushed_hashes = pushed_commit_hashes(&root);
    let args = vec![
        "log".to_string(),
        "--topo-order".to_string(),
        "--all".to_string(),
        "--skip".to_string(),
        offset.to_string(),
        "-n".to_string(),
        request_limit.to_string(),
        "--pretty=format:%x1e%H%x1f%h%x1f%P%x1f%an%x1f%at%x1f%D%x1f%s%x1f%B%x1d".to_string(),
    ];
    let output = run_git_owned(&root, &args).map_err(|e| format!("Failed to run git log: {e}"))?;
    if !output.status.success() {
        return Err(git_output_error(&output, "Failed to load git commits"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = parse_git_log_commits(&stdout, &pushed_hashes);
    let has_more = commits.len() > safe_limit;
    commits.truncate(safe_limit);
    Ok(ProjectGitCommitPage { commits, has_more })
}

#[tauri::command]
async fn run_project_git_action(
    path: String,
    action: String,
    paths: Option<Vec<String>>,
    message: Option<String>,
) -> Result<ProjectGitActionResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        run_project_git_action_sync(path, action, paths, message)
    })
    .await
    .map_err(|e| format!("Failed to join git action task: {e}"))?
}

fn run_project_git_action_sync(
    path: String,
    action: String,
    paths: Option<Vec<String>>,
    message: Option<String>,
) -> Result<ProjectGitActionResult, String> {
    let project = validate_project_dir(&path)?;
    let root = git_root_or_error(&project)?;
    match action.as_str() {
        "fetch" => run_git_action(&root, &["fetch", "--prune"]),
        "pull" => run_git_action(&root, &["pull", "--ff-only"]),
        "push" => run_git_action(&root, &["push"]),
        "sync" => {
            let mut combined = ProjectGitActionResult {
                stdout: String::new(),
                stderr: String::new(),
            };
            append_git_action_result(
                &mut combined,
                run_git_action(&root, &["pull", "--ff-only"])?,
            );
            append_git_action_result(&mut combined, run_git_action(&root, &["push"])?);
            Ok(combined)
        }
        "stageAll" => run_git_action(&root, &["add", "-A"]),
        "unstageAll" => run_git_action(&root, &["restore", "--staged", "."]),
        "discardAll" => {
            let mut combined = ProjectGitActionResult {
                stdout: String::new(),
                stderr: String::new(),
            };
            append_git_action_result(
                &mut combined,
                run_git_action(&root, &["restore", "--worktree", "."])?,
            );
            Ok(combined)
        }
        "cleanAll" => run_git_action(&root, &["clean", "-fd", "--", "."]),
        "stage" => {
            let paths = validate_git_relative_paths(paths, true)?;
            let mut args = vec!["add".to_string(), "--".to_string()];
            args.extend(paths);
            run_git_action_owned(&root, &args)
        }
        "unstage" => {
            let paths = validate_git_relative_paths(paths, true)?;
            let mut args = vec![
                "restore".to_string(),
                "--staged".to_string(),
                "--".to_string(),
            ];
            args.extend(paths);
            run_git_action_owned(&root, &args)
        }
        "discard" => {
            let paths = validate_git_relative_paths(paths, true)?;
            let mut combined = ProjectGitActionResult {
                stdout: String::new(),
                stderr: String::new(),
            };
            for path in paths {
                let result = discard_git_path(&root, &path)?;
                append_git_action_result(&mut combined, result);
            }
            Ok(combined)
        }
        "clean" => {
            let paths = validate_git_relative_paths(paths, true)?;
            let mut args = vec!["clean".to_string(), "-fd".to_string(), "--".to_string()];
            args.extend(paths);
            run_git_action_owned(&root, &args)
        }
        "commit" => {
            let message = message.unwrap_or_default().trim().to_string();
            if message.is_empty() {
                return Err("Commit message is required".to_string());
            }
            run_git_action_owned(&root, &["commit".to_string(), "-m".to_string(), message])
        }
        _ => Err(format!("Unsupported git action: {action}")),
    }
}

fn validate_project_dir(path: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(path);
    if !root.is_absolute() || !root.is_dir() {
        return Err("Invalid project path".to_string());
    }
    Ok(root)
}

fn run_git(root: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
}

fn run_git_owned(root: &Path, args: &[String]) -> std::io::Result<std::process::Output> {
    let mut command = std::process::Command::new("git");
    command.arg("-C").arg(root);
    for arg in args {
        command.arg(arg);
    }
    command.output()
}

fn discover_git_root(project: &Path) -> Result<Option<PathBuf>, String> {
    let output = match run_git(project, &["rev-parse", "--show-toplevel"]) {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        return Ok(None);
    }
    Ok(Some(PathBuf::from(stdout)))
}

fn git_root_or_error(project: &Path) -> Result<PathBuf, String> {
    discover_git_root(project)?.ok_or_else(|| "Not a git repository".to_string())
}

fn git_stdout_optional(root: &Path, args: &[&str]) -> Option<String> {
    let output = run_git(root, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn git_output_error(output: &std::process::Output, fallback: &str) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return stdout;
    }
    fallback.to_string()
}

fn empty_project_git_summary() -> ProjectGitSummary {
    ProjectGitSummary {
        is_repo: false,
        root: None,
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
        has_changes: false,
        staged_count: 0,
        unstaged_count: 0,
        untracked_count: 0,
    }
}

fn load_project_git_changes(root: &Path) -> Result<Vec<ProjectGitChange>, String> {
    let output = run_git(
        root,
        &[
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v1",
            "-uall",
        ],
    )
    .map_err(|e| format!("Failed to run git status: {e}"))?;
    if !output.status.success() {
        return Err(git_output_error(&output, "Failed to load git status"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_git_status_stdout(&stdout))
}

fn build_project_git_summary(root: &Path, changes: &[ProjectGitChange]) -> ProjectGitSummary {
    let branch = git_stdout_optional(root, &["branch", "--show-current"]);
    let head = git_stdout_optional(root, &["rev-parse", "--short", "HEAD"]);
    let upstream = git_stdout_optional(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
    );
    let (ahead, behind) = if upstream.is_some() {
        git_stdout_optional(
            root,
            &["rev-list", "--left-right", "--count", "HEAD...@{u}"],
        )
        .and_then(|value| {
            let mut parts = value.split_whitespace();
            let ahead = parts.next()?.parse::<i64>().ok()?;
            let behind = parts.next()?.parse::<i64>().ok()?;
            Some((ahead, behind))
        })
        .unwrap_or((0, 0))
    } else {
        (0, 0)
    };
    let staged_count = changes.iter().filter(|change| change.staged).count();
    let unstaged_count = changes.iter().filter(|change| !change.staged).count();
    let untracked_count = changes
        .iter()
        .filter(|change| !change.staged && change.status == "untracked")
        .count();
    ProjectGitSummary {
        is_repo: true,
        root: Some(root.to_string_lossy().to_string()),
        branch,
        head,
        upstream,
        ahead,
        behind,
        has_changes: !changes.is_empty(),
        staged_count,
        unstaged_count,
        untracked_count,
    }
}

fn parse_git_status_stdout(stdout: &str) -> Vec<ProjectGitChange> {
    let mut changes = Vec::new();
    for line in stdout.lines() {
        changes.extend(parse_git_status_line(line));
    }
    changes.sort_by(|a, b| {
        a.staged
            .cmp(&b.staged)
            .reverse()
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.status.cmp(&b.status))
    });
    changes
}

fn parse_git_status_line(line: &str) -> Vec<ProjectGitChange> {
    if line.len() < 4 {
        return Vec::new();
    }
    let mut chars = line.chars();
    let index = chars.next().unwrap_or(' ');
    let worktree = chars.next().unwrap_or(' ');
    let raw_path = &line[3..];
    let (path, original_path) = if let Some(arrow_pos) = raw_path.find(" -> ") {
        (
            raw_path[arrow_pos + 4..].to_string(),
            Some(raw_path[..arrow_pos].to_string()),
        )
    } else {
        (raw_path.to_string(), None)
    };

    if index == '?' && worktree == '?' {
        return vec![ProjectGitChange {
            path,
            original_path,
            status: "untracked".to_string(),
            staged: false,
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
        }];
    }

    if index == '!' && worktree == '!' {
        return vec![ProjectGitChange {
            path,
            original_path,
            status: "ignored".to_string(),
            staged: false,
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
        }];
    }

    let mut changes = Vec::new();
    if index != ' ' {
        changes.push(ProjectGitChange {
            path: path.clone(),
            original_path: original_path.clone(),
            status: git_status_from_code(index).to_string(),
            staged: true,
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
        });
    }
    if worktree != ' ' {
        changes.push(ProjectGitChange {
            path,
            original_path,
            status: git_status_from_code(worktree).to_string(),
            staged: false,
            index_status: index.to_string(),
            worktree_status: worktree.to_string(),
        });
    }
    changes
}

fn git_status_from_code(code: char) -> &'static str {
    match code {
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        '?' => "untracked",
        '!' => "ignored",
        _ => "modified",
    }
}

fn pushed_commit_hashes(root: &Path) -> HashSet<String> {
    let output = match run_git(root, &["rev-list", "--remotes"]) {
        Ok(output) => output,
        Err(_) => return HashSet::new(),
    };
    if !output.status.success() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_git_log_commits(stdout: &str, pushed_hashes: &HashSet<String>) -> Vec<ProjectGitCommit> {
    const RECORD: char = '\x1e';
    const FIELD: char = '\x1f';
    const END: char = '\x1d';
    let mut commits = Vec::new();
    for record in stdout.split(END) {
        let record = record.trim_start_matches('\n');
        let Some(record) = record.strip_prefix(RECORD) else {
            continue;
        };
        let fields: Vec<&str> = record.split(FIELD).collect();
        if fields.len() < 8 {
            continue;
        }
        let timestamp = fields[4].parse::<i64>().unwrap_or(0) * 1000;
        let refs = fields[5]
            .split(", ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect();
        commits.push(ProjectGitCommit {
            hash: fields[0].to_string(),
            short_hash: fields[1].to_string(),
            parents: fields[2]
                .split_whitespace()
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect(),
            author: fields[3].to_string(),
            timestamp,
            refs,
            subject: fields[6].to_string(),
            message: fields[7].trim().to_string(),
            pushed: pushed_hashes.contains(fields[0]),
        });
    }
    commits
}

fn validate_git_relative_paths(
    paths: Option<Vec<String>>,
    require_non_empty: bool,
) -> Result<Vec<String>, String> {
    let paths = paths.unwrap_or_default();
    if require_non_empty && paths.is_empty() {
        return Err("Select at least one file".to_string());
    }
    for path in &paths {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err("Invalid git path".to_string());
        }
        let path_buf = Path::new(trimmed);
        if path_buf.is_absolute()
            || path_buf.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err("Git path must stay inside the repository".to_string());
        }
    }
    Ok(paths)
}

fn run_git_action(root: &Path, args: &[&str]) -> Result<ProjectGitActionResult, String> {
    let output = run_git(root, args).map_err(|e| format!("Failed to run git: {e}"))?;
    git_action_result(output)
}

fn run_git_action_owned(root: &Path, args: &[String]) -> Result<ProjectGitActionResult, String> {
    let output = run_git_owned(root, args).map_err(|e| format!("Failed to run git: {e}"))?;
    git_action_result(output)
}

fn git_action_result(output: std::process::Output) -> Result<ProjectGitActionResult, String> {
    if !output.status.success() {
        return Err(git_output_error(&output, "Git command failed"));
    }
    Ok(ProjectGitActionResult {
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn append_git_action_result(target: &mut ProjectGitActionResult, result: ProjectGitActionResult) {
    if !result.stdout.is_empty() {
        if !target.stdout.is_empty() {
            target.stdout.push('\n');
        }
        target.stdout.push_str(&result.stdout);
    }
    if !result.stderr.is_empty() {
        if !target.stderr.is_empty() {
            target.stderr.push('\n');
        }
        target.stderr.push_str(&result.stderr);
    }
}

fn discard_git_path(root: &Path, path: &str) -> Result<ProjectGitActionResult, String> {
    let status_args = vec![
        "-c".to_string(),
        "core.quotePath=false".to_string(),
        "status".to_string(),
        "--porcelain=v1".to_string(),
        "-uall".to_string(),
        "--".to_string(),
        path.to_string(),
    ];
    let output =
        run_git_owned(root, &status_args).map_err(|e| format!("Failed to run git status: {e}"))?;
    if !output.status.success() {
        return Err(git_output_error(&output, "Failed to inspect git path"));
    }
    let status = String::from_utf8_lossy(&output.stdout);
    let untracked = status.lines().any(|line| line.starts_with("?? "));
    if untracked {
        run_git_action_owned(
            root,
            &[
                "clean".to_string(),
                "-fd".to_string(),
                "--".to_string(),
                path.to_string(),
            ],
        )
    } else {
        run_git_action_owned(
            root,
            &[
                "restore".to_string(),
                "--worktree".to_string(),
                "--".to_string(),
                path.to_string(),
            ],
        )
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FileGitDiff {
    status: String,
    patch: Option<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTextFile {
    content: String,
    mtime_ms: u64,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTextFileWrite {
    mtime_ms: u64,
}

const MAX_EDITOR_TEXT_BYTES: u64 = 10 * 1024 * 1024;

#[tauri::command]
fn get_file_git_diff(workspace_path: String, file_path: String) -> Result<FileGitDiff, String> {
    let workspace = PathBuf::from(&workspace_path);
    if !workspace.is_absolute() || !workspace.is_dir() {
        return Err("Invalid workspace path".to_string());
    }

    let file = PathBuf::from(&file_path);
    if !file.is_absolute() {
        return Err("Only absolute file paths are supported".to_string());
    }

    let relative = file
        .strip_prefix(&workspace)
        .map_err(|_| "File path is outside the workspace".to_string())?;
    let relative_string = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");

    let status_output = std::process::Command::new("git")
        .arg("-C")
        .arg(&workspace_path)
        .args(["status", "--porcelain=v1", "-uall", "--"])
        .arg(&relative_string)
        .output()
        .map_err(|e| format!("Failed to run git status: {e}"))?;
    if !status_output.status.success() {
        return Ok(FileGitDiff {
            status: "clean".to_string(),
            patch: None,
        });
    }

    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    let first_status_line = status_stdout.lines().next().unwrap_or("").trim_end();
    if first_status_line.is_empty() {
        return Ok(FileGitDiff {
            status: "clean".to_string(),
            patch: None,
        });
    }

    let status = parse_git_status_kind(first_status_line);
    if status == "untracked" {
        let patch = build_untracked_file_patch(&file, &relative_string)?;
        return Ok(FileGitDiff {
            status,
            patch: Some(patch),
        });
    }

    let diff_output = std::process::Command::new("git")
        .arg("-C")
        .arg(&workspace_path)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--unified=0",
            "HEAD",
            "--",
        ])
        .arg(&relative_string)
        .output()
        .map_err(|e| format!("Failed to run git diff: {e}"))?;

    if !diff_output.status.success() {
        return Ok(FileGitDiff {
            status,
            patch: None,
        });
    }

    let patch = String::from_utf8_lossy(&diff_output.stdout).to_string();
    Ok(FileGitDiff {
        status,
        patch: if patch.trim().is_empty() {
            None
        } else {
            Some(patch)
        },
    })
}

fn parse_git_status_kind(line: &str) -> String {
    if line.starts_with("?? ") {
        return "untracked".to_string();
    }
    if line.starts_with("!! ") {
        return "ignored".to_string();
    }
    let xy = line.chars().take(2).collect::<String>();
    let x = xy.chars().next().unwrap_or(' ');
    let y = xy.chars().nth(1).unwrap_or(' ');
    if x == 'R' || y == 'R' {
        return "renamed".to_string();
    }
    if x == 'D' || y == 'D' {
        return "deleted".to_string();
    }
    if x == 'A' || y == 'A' {
        return "added".to_string();
    }
    "modified".to_string()
}

fn build_untracked_file_patch(file: &Path, relative: &str) -> Result<String, String> {
    let contents = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let line_count = if contents.is_empty() {
        0
    } else {
        contents.lines().count()
    };
    let mut patch = String::new();
    patch.push_str(&format!("diff --git a/{relative} b/{relative}\n"));
    patch.push_str("new file mode 100644\n");
    patch.push_str("index 0000000..0000000\n");
    patch.push_str("--- /dev/null\n");
    patch.push_str(&format!("+++ b/{relative}\n"));
    patch.push_str(&format!("@@ -0,0 +1,{} @@\n", line_count));
    for line in contents.lines() {
        patch.push('+');
        patch.push_str(line);
        patch.push('\n');
    }
    if contents.ends_with('\n') || contents.is_empty() {
        return Ok(patch);
    }
    patch.push_str("\\ No newline at end of file\n");
    Ok(patch)
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

#[tauri::command]
fn read_workspace_text_file(
    workspace_path: String,
    path: String,
) -> Result<WorkspaceTextFile, String> {
    let path_buf = workspace_text_file_path(&workspace_path, &path)?;
    let _mime =
        text_file_mime(&path_buf).ok_or_else(|| "Unsupported text file type".to_string())?;
    let meta = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Path is not a file".to_string());
    }
    if meta.len() > MAX_EDITOR_TEXT_BYTES {
        return Err("File is too large to edit".to_string());
    }
    let mtime_ms = file_mtime_ms(&meta)?;
    let content = std::fs::read_to_string(&path_buf).map_err(|e| e.to_string())?;
    Ok(WorkspaceTextFile { content, mtime_ms })
}

#[tauri::command]
fn write_workspace_text_file(
    workspace_path: String,
    path: String,
    content: String,
    expected_mtime_ms: u64,
) -> Result<WorkspaceTextFileWrite, String> {
    let path_buf = workspace_text_file_path(&workspace_path, &path)?;
    let _mime =
        text_file_mime(&path_buf).ok_or_else(|| "Unsupported text file type".to_string())?;
    if content.as_bytes().len() as u64 > MAX_EDITOR_TEXT_BYTES {
        return Err("File is too large to edit".to_string());
    }
    let meta = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("Path is not a file".to_string());
    }
    let current_mtime_ms = file_mtime_ms(&meta)?;
    if current_mtime_ms != expected_mtime_ms {
        return Err("File changed on disk; reload before saving".to_string());
    }

    std::fs::write(&path_buf, content).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
    Ok(WorkspaceTextFileWrite {
        mtime_ms: file_mtime_ms(&meta)?,
    })
}

fn workspace_text_file_path(workspace_path: &str, path: &str) -> Result<PathBuf, String> {
    let workspace = PathBuf::from(workspace_path);
    if !workspace.is_absolute() || !workspace.is_dir() {
        return Err("Invalid workspace path".to_string());
    }
    let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
    let file = PathBuf::from(path);
    let file = if file.is_absolute() {
        file
    } else {
        workspace.join(file)
    };
    let file = file.canonicalize().map_err(|e| e.to_string())?;
    if !file.starts_with(&workspace) {
        return Err("File path is outside the workspace".to_string());
    }
    Ok(file)
}

fn safe_canvas_file_component(raw: &str, fallback: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let compact = sanitized.trim_matches('-');
    if compact.is_empty() {
        fallback.to_string()
    } else {
        compact.to_string()
    }
}

fn canvas_draft_path(canvas_storage_key: &str) -> Result<PathBuf, String> {
    Ok(session_canvas_dir(canvas_storage_key)
        .map_err(|e| e.to_string())?
        .join("draft.canvas.json"))
}

fn canvas_revisions_dir(canvas_storage_key: &str) -> Result<PathBuf, String> {
    Ok(session_canvas_dir(canvas_storage_key)
        .map_err(|e| e.to_string())?
        .join("revisions"))
}

fn canvas_context_dir(canvas_storage_key: &str) -> Result<PathBuf, String> {
    Ok(session_canvas_dir(canvas_storage_key)
        .map_err(|e| e.to_string())?
        .join("context"))
}

const CANVAS_REVISION_RETENTION_LIMIT: usize = 24;
const CANVAS_CONTEXT_FILE_RETENTION_LIMIT: usize = 24;

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

fn read_optional_text_file(path: Option<&str>) -> Option<String> {
    path.and_then(|value| std::fs::read_to_string(value).ok())
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Target path has no parent".to_string())?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "Target path has no file name".to_string())?;
    let temp_path = parent.join(format!(".{file_name}.tmp"));
    std::fs::write(&temp_path, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        e.to_string()
    })
}

fn prune_canvas_context_files(canvas_storage_key: &str, keep_latest: usize) -> Result<(), String> {
    let dir = canvas_context_dir(canvas_storage_key)?;
    let entries = std::fs::read_dir(&dir);
    let Ok(entries) = entries else {
        return Ok(());
    };
    let mut files = entries
        .filter_map(|entry| entry.ok().map(|item| item.path()))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.cmp(left));
    for stale in files.into_iter().skip(keep_latest) {
        let _ = std::fs::remove_file(stale);
    }
    Ok(())
}

fn file_mtime_ms(meta: &std::fs::Metadata) -> Result<u64, String> {
    let modified = meta.modified().map_err(|e| e.to_string())?;
    u64::try_from(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis(),
    )
    .map_err(|_| "File modified time is too large".to_string())
}

#[tauri::command]
fn watch_preview_file(
    path: String,
    watcher: State<'_, file_preview_watch::PreviewFileWatcher>,
) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if !path_buf.is_absolute() {
        return Err("Only absolute file paths can be watched".to_string());
    }
    watcher.watch_path(&path_buf).map_err(|e| e.to_string())
}

#[tauri::command]
fn unwatch_preview_file(
    path: String,
    watcher: State<'_, file_preview_watch::PreviewFileWatcher>,
) -> Result<(), String> {
    let path_buf = PathBuf::from(&path);
    if !path_buf.is_absolute() {
        return Err("Only absolute file paths can be unwatched".to_string());
    }
    watcher.unwatch_path(&path_buf).map_err(|e| e.to_string())
}

fn safe_pasted_attachment_file_name(file_name: Option<&str>, mime_type: Option<&str>) -> String {
    let raw_name = file_name
        .unwrap_or("")
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or("")
        .trim();
    let sanitized: String = raw_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ' ') {
                ch
            } else {
                '_'
            }
        })
        .take(160)
        .collect();
    let mut name = sanitized
        .trim_matches(|ch| ch == '.' || ch == ' ')
        .to_string();
    if name.is_empty() || name == "." || name == ".." {
        name = match mime_type.and_then(extension_for_pasted_mime) {
            Some(ext) if ext == "png" => "pasted-image.png".to_string(),
            Some(ext) => format!("pasted-file.{ext}"),
            None => "pasted-file".to_string(),
        };
    } else if Path::new(&name).extension().is_none() {
        if let Some(ext) = mime_type.and_then(extension_for_pasted_mime) {
            name.push('.');
            name.push_str(ext);
        }
    }
    name
}

fn extension_for_pasted_mime(mime_type: &str) -> Option<&'static str> {
    match mime_type.trim().to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/svg+xml" => Some("svg"),
        "image/bmp" => Some("bmp"),
        "image/heic" => Some("heic"),
        "image/heif" => Some("heif"),
        "text/plain" => Some("txt"),
        "text/markdown" => Some("md"),
        "text/csv" => Some("csv"),
        "text/html" => Some("html"),
        "text/css" => Some("css"),
        "application/json" => Some("json"),
        "application/xml" => Some("xml"),
        "application/yaml" | "application/x-yaml" => Some("yaml"),
        "application/toml" => Some("toml"),
        _ => None,
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn normalized_appshot_shortcut(shortcut: &str) -> Result<String, String> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::Shortcut;

    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return Err("Shortcut cannot be empty".to_string());
    }
    Shortcut::from_str(trimmed)
        .map(|parsed| parsed.to_string())
        .map_err(|e| format!("Invalid appshot shortcut: {e}"))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn parsed_appshot_shortcut(
    shortcut: &str,
) -> Result<tauri_plugin_global_shortcut::Shortcut, String> {
    use std::str::FromStr;
    tauri_plugin_global_shortcut::Shortcut::from_str(shortcut)
        .map_err(|e| format!("Invalid appshot shortcut: {e}"))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn normalized_appshot_shortcut(shortcut: &str) -> Result<String, String> {
    let trimmed = shortcut.trim();
    if trimmed.is_empty() {
        return Err("Shortcut cannot be empty".to_string());
    }
    Ok(trimmed.to_string())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn update_appshot_shortcut_registration(app: &AppHandle, shortcut: &str) -> Result<String, String> {
    let normalized = normalized_appshot_shortcut(shortcut)?;
    let parsed = parsed_appshot_shortcut(&normalized)?;
    let state = app.state::<AppshotShortcutState>();
    let mut guard = state
        .registered_shortcut
        .lock()
        .map_err(|_| "Appshot shortcut state is poisoned".to_string())?;
    let current = guard.clone();
    if current.as_deref() == Some(normalized.as_str()) {
        return Ok(normalized);
    }

    let manager = app.global_shortcut();
    manager
        .on_shortcut(parsed, {
            let normalized = normalized.clone();
            move |app, _shortcut, event| {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    #[allow(deprecated)]
                    let is_pressed = matches!(
                        event.state,
                        tauri_plugin_global_shortcut::ShortcutState::Pressed
                    );
                    if is_pressed {
                        handle_appshot_shortcut_pressed(app.clone(), normalized.clone());
                    }
                }));
                if result.is_err() {
                    log::warn!("[appshot] global shortcut callback panicked");
                }
            }
        })
        .map_err(|e| e.to_string())?;

    if let Some(previous) = current {
        if previous != normalized {
            if let Ok(previous_shortcut) = parsed_appshot_shortcut(&previous) {
                if let Err(error) = manager.unregister(previous_shortcut) {
                    log::warn!("[appshot] failed to unregister shortcut {previous}: {error}");
                }
            } else {
                log::warn!("[appshot] previous shortcut could not be reparsed: {previous}");
            }
        }
    }
    *guard = Some(normalized.clone());
    Ok(normalized)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn suspend_appshot_shortcut_registration(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppshotShortcutState>();
    let mut registered_guard = state
        .registered_shortcut
        .lock()
        .map_err(|_| "Appshot shortcut state is poisoned".to_string())?;
    let current = registered_guard.clone();
    if let Some(current_shortcut) = current {
        let parsed = parsed_appshot_shortcut(&current_shortcut)?;
        app.global_shortcut()
            .unregister(parsed)
            .map_err(|e| e.to_string())?;
        let mut suspended_guard = state
            .suspended_shortcut
            .lock()
            .map_err(|_| "Appshot shortcut state is poisoned".to_string())?;
        *suspended_guard = Some(current_shortcut);
        *registered_guard = None;
    }
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn suspend_appshot_shortcut_registration(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn resume_appshot_shortcut_registration(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppshotShortcutState>();
    let suspended = {
        let mut suspended_guard = state
            .suspended_shortcut
            .lock()
            .map_err(|_| "Appshot shortcut state is poisoned".to_string())?;
        suspended_guard.take()
    };
    if let Some(shortcut) = suspended {
        let _ = update_appshot_shortcut_registration(app, &shortcut)?;
    }
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn resume_appshot_shortcut_registration(_app: &AppHandle) -> Result<(), String> {
    Ok(())
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn update_appshot_shortcut_registration(
    _app: &AppHandle,
    shortcut: &str,
) -> Result<String, String> {
    normalized_appshot_shortcut(shortcut)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn handle_appshot_shortcut_pressed(app: AppHandle, shortcut: String) {
    thread::spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            capture_and_emit_frontmost_appshot(&app, shortcut)
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("[appshot] capture failed: {error}"),
            Err(_) => log::warn!("[appshot] capture task panicked"),
        }
    });
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn capture_and_emit_frontmost_appshot(app: &AppHandle, shortcut: String) -> Result<(), String> {
    let permissions = appshot_permission_status();
    if !permissions.can_capture {
        #[cfg(target_os = "macos")]
        {
            if let Err(error) = appshot_permission_panel::show(app.clone()) {
                log::warn!("[appshot] failed to show permission panel from shortcut: {error}");
            }
        }
        show_main_window(app);
        emit_appshot_permission_required(
            app,
            AppshotPermissionRequiredPayload {
                shortcut,
                status: permissions,
            },
        )?;
        return Ok(());
    }
    let saved = capture_frontmost_window_png(app, Some("appshot.png".to_string()))?;
    let result = emit_appshot_captured(
        app,
        AppshotCapturedPayload {
            path: saved.path.clone(),
            shortcut,
        },
    );
    show_main_window(app);
    result
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
        Some("log") => Some("text/plain"),
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
        None => Some("text/plain"),
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
    let dir = cross_context_dir().map_err(|e| e.to_string())?;
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
fn get_canvas(
    canvas_key: CanvasKey,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<CanvasDocumentState, String> {
    let canvas_storage_key = canvas_key.storage_key()?;
    let mut state = store
        .get_canvas_document_state(&canvas_storage_key)
        .map_err(|e| e.to_string())?;
    state.draft_snapshot = read_optional_text_file(state.document.draft_snapshot_path.as_deref());
    state.saved_snapshot = read_optional_text_file(
        state
            .saved_revision
            .as_ref()
            .map(|revision| revision.snapshot_path.as_str()),
    );
    Ok(state)
}

#[tauri::command]
fn save_canvas_draft(
    req: SaveCanvasDraftRequest,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SavedCanvasDraft, String> {
    let canvas_storage_key = req.canvas_key.storage_key()?;
    let path = canvas_draft_path(&canvas_storage_key)?;
    write_atomic_bytes(&path, req.snapshot_json.as_bytes())?;
    let hash = sha256_hex(req.snapshot_json.as_bytes());
    let document = store
        .save_canvas_draft(
            &canvas_storage_key,
            req.title.as_deref(),
            &path.to_string_lossy(),
            &hash,
        )
        .map_err(|e| e.to_string())?;
    Ok(SavedCanvasDraft { document })
}

#[tauri::command]
fn save_canvas_revision(
    req: SaveCanvasRevisionRequest,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SavedCanvasRevision, String> {
    let canvas_storage_key = req.canvas_key.storage_key()?;
    let revisions_dir = canvas_revisions_dir(&canvas_storage_key)?;
    std::fs::create_dir_all(&revisions_dir).map_err(|e| e.to_string())?;
    let snapshot_bytes = req.snapshot_json.into_bytes();
    let hash = sha256_hex(&snapshot_bytes);
    let current = store
        .get_canvas_document_state(&canvas_storage_key)
        .map_err(|e| e.to_string())?;
    let next_revision = current.document.current_saved_revision.unwrap_or(0) + 1;
    let path = revisions_dir.join(format!("{next_revision:06}.canvas.json"));
    write_atomic_bytes(&path, &snapshot_bytes)?;
    let (document, revision) = store
        .save_canvas_revision(
            &canvas_storage_key,
            req.title.as_deref(),
            &path.to_string_lossy(),
            &hash,
            snapshot_bytes.len() as i64,
            req.source.trim(),
        )
        .map_err(|e| e.to_string())?;
    let stale_paths = store
        .prune_canvas_revisions(&canvas_storage_key, CANVAS_REVISION_RETENTION_LIMIT)
        .map_err(|e| e.to_string())?;
    for stale_path in stale_paths {
        let _ = std::fs::remove_file(stale_path);
    }
    Ok(SavedCanvasRevision { document, revision })
}

#[tauri::command]
fn update_canvas_blocks(
    req: UpdateCanvasBlocksRequest,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Vec<CanvasBlockRecord>, String> {
    let canvas_storage_key = req.canvas_key.storage_key()?;
    let blocks = req
        .blocks
        .into_iter()
        .map(|item| UpsertCanvasBlockRecord {
            block_id: item.block_id,
            block_kind: item.block_kind,
            source_type: item.source_type,
            source_key: item.source_key,
            source_path: item.source_path,
            metadata_json: item.metadata_json.unwrap_or_else(|| "{}".to_string()),
        })
        .collect::<Vec<_>>();
    store
        .replace_canvas_blocks(&canvas_storage_key, &blocks)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_canvas_context_file(req: BuildCanvasContextFileRequest) -> Result<String, String> {
    let canvas_storage_key = req.canvas_key.storage_key()?;
    let dir = canvas_context_dir(&canvas_storage_key)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let prefix = safe_canvas_file_component(&req.file_name_prefix, "canvas");
    let kind = safe_canvas_file_component(&req.kind, "selection");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{prefix}-{kind}-{ts}.md"));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut file| {
            use std::io::Write;
            file.write_all(req.content.as_bytes())
        })
        .map_err(|e| e.to_string())?;
    prune_canvas_context_files(&canvas_storage_key, CANVAS_CONTEXT_FILE_RETENTION_LIMIT)?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn create_canvas_anchor(
    req: UpsertCanvasAnchorRequest,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<CanvasContextAnchor, String> {
    let canvas_storage_key = req.canvas_key.storage_key()?;
    store
        .create_canvas_context_anchor(
            &canvas_storage_key,
            req.anchor_block_id.as_deref(),
            &req.selection_block_ids_json,
            &req.selection_element_ids_json,
            &req.turn_id,
            req.summary.as_deref(),
        )
        .map_err(|e| e.to_string())
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
fn get_runtime_agent_session_config(
    agent: Agent,
    cache: State<'_, RuntimeAgentsCache>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<Option<RuntimeAgentSessionConfigDto>, String> {
    resolve_runtime_agent_session_config_record(agent, &cache.get(), store.inner())
        .map(|record| record.map(runtime_agent_session_config_to_dto))
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
fn list_skills(
    cache: State<'_, skills::SkillsCache>,
) -> Result<Vec<skills::SkillMetadata>, String> {
    Ok(cache.get())
}

#[tauri::command]
fn install_skill(
    req: skills::InstallSkillRequest,
    app: AppHandle,
    cache: State<'_, skills::SkillsCache>,
) -> Result<skills::SkillMetadata, String> {
    let installed = skills::install_skill(req).map_err(|e| e.to_string())?;
    cache.refresh_from_disk().map_err(|e| e.to_string())?;
    app.emit(skills::SKILLS_UPDATED_EVENT, ())
        .map_err(|e| e.to_string())?;
    Ok(installed)
}

#[tauri::command]
fn update_mcp_settings(
    settings: mcp::McpSettings,
    cache: State<'_, mcp::McpSettingsCache>,
) -> Result<mcp::McpSettings, String> {
    let settings = mcp::save_settings(settings).map_err(|e| e.to_string())?;
    cache.set(settings.clone());
    Ok(settings)
}

#[tauri::command]
fn get_computer_use_settings() -> Result<computer_use::settings::ComputerUseSettings, String> {
    computer_use::config::load_settings().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_appshot_permission_status() -> AppshotPermissionStatusDto {
    appshot_permission_status()
}

#[tauri::command]
fn get_desktop_control_permission_status() -> desktop_control::DesktopControlPermissionStatus {
    desktop_control_permission_status()
}

#[tauri::command]
fn request_appshot_permission(
    app: AppHandle,
    permission: String,
) -> Result<AppshotPermissionStatusDto, String> {
    #[cfg(not(target_os = "macos"))]
    let _ = &app;
    let kind = AppshotPermissionKind::parse(&permission)?;
    match kind {
        #[cfg(target_os = "macos")]
        AppshotPermissionKind::Screenshots => {
            let _ = appshot_request_screenshots_permission();
        }
        #[cfg(target_os = "macos")]
        AppshotPermissionKind::Accessibility => {
            let _ = appshot_request_accessibility_permission();
        }
        #[cfg(not(target_os = "macos"))]
        _ => {}
    }
    let status = appshot_permission_status();
    #[cfg(target_os = "macos")]
    {
        let still_missing = match kind {
            AppshotPermissionKind::Screenshots => !status.screenshots.granted,
            AppshotPermissionKind::Accessibility => !status.accessibility.granted,
        };
        if still_missing {
            let _ = open_appshot_permission_settings_impl(kind);
            let _ = appshot_permission_panel::show(app);
        }
    }
    Ok(status)
}

#[tauri::command]
fn open_appshot_permissions_panel(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        appshot_permission_panel::show(app)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Ok(())
    }
}

#[tauri::command]
fn open_appshot_permission_settings(permission: String) -> Result<(), String> {
    let kind = AppshotPermissionKind::parse(&permission)?;
    open_appshot_permission_settings_impl(kind)
}

#[tauri::command]
fn update_appshot_config(
    app: AppHandle,
    mut config: config::AppshotConfig,
) -> Result<config::AppshotConfig, String> {
    let normalized_shortcut = normalized_appshot_shortcut(&config.shortcut)?;
    let state = app.state::<AppshotShortcutState>();
    let is_suspended = state
        .suspended_shortcut
        .lock()
        .map_err(|_| "Appshot shortcut state is poisoned".to_string())?
        .is_some();
    if is_suspended {
        let mut suspended_guard = state
            .suspended_shortcut
            .lock()
            .map_err(|_| "Appshot shortcut state is poisoned".to_string())?;
        *suspended_guard = Some(normalized_shortcut.clone());
        config.shortcut = normalized_shortcut;
    } else {
        config.shortcut = update_appshot_shortcut_registration(&app, &normalized_shortcut)?;
    }
    let mut app_config = config::load_config().map_err(|e| e.to_string())?;
    app_config.appshot = config.clone();
    config::save_config(&app_config).map_err(|e| e.to_string())?;
    Ok(config)
}

#[tauri::command]
fn set_appshot_shortcut_recording(app: AppHandle, recording: bool) -> Result<(), String> {
    if recording {
        suspend_appshot_shortcut_registration(&app)
    } else {
        resume_appshot_shortcut_registration(&app)
    }
}

#[tauri::command]
fn update_computer_use_settings(
    settings: computer_use::settings::ComputerUseSettings,
    runtime: State<'_, RuntimeManager>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
) -> Result<computer_use::settings::ComputerUseSettings, String> {
    let settings = computer_use::config::save_settings(settings).map_err(|e| e.to_string())?;
    runtime.update_computer_use_settings(settings.clone());
    mcp_cache.refresh_from_disk().map_err(|e| e.to_string())?;
    Ok(settings)
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
fn get_scheduled_tasks(
    service: State<'_, scheduled_tasks::ScheduledTasksService>,
) -> Result<Vec<scheduled_tasks::ScheduledTask>, String> {
    Ok(service.list())
}

#[tauri::command]
fn save_scheduled_tasks(
    tasks: Vec<scheduled_tasks::ScheduledTask>,
    service: State<'_, scheduled_tasks::ScheduledTasksService>,
) -> Result<Vec<scheduled_tasks::ScheduledTask>, String> {
    service
        .save(scheduled_tasks::ScheduledTasksConfig { tasks })
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn run_scheduled_task_now(
    id: String,
    service: State<'_, scheduled_tasks::ScheduledTasksService>,
) -> Result<(), String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || service.run_now(&id).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn force_unlock_scheduled_task(
    id: String,
    service: State<'_, scheduled_tasks::ScheduledTasksService>,
) -> Result<(), String> {
    let service = service.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.force_unlock(&id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
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
async fn test_feishu_bot_connection(
    app_id: String,
    app_secret: String,
    domain: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        im_bridge::test_feishu_bot_connection(&app_id, &app_secret, domain.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn test_wechat_bot_connection(
    bot_token: String,
    base_url: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        im_bridge::test_wechat_bot_connection(&bot_token, base_url.as_deref())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_wechat_qrcode(base_url: Option<String>) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        im_bridge::get_wechat_qrcode(base_url.as_deref()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn poll_wechat_qrcode_status(
    qrcode: String,
    base_url: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        im_bridge::poll_wechat_qrcode_status(&qrcode, base_url.as_deref())
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
                commands: req.commands.as_ref(),
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
    skills_cache: State<'_, skills::SkillsCache>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    hydrate_start_request_from_db(&mut req, store.inner()).map_err(|e| e.to_string())?;
    hydrate_skill_options(&mut req.options, skills_cache.inner());
    hydrate_mcp_options(&mut req.options, mcp_cache.inner());
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_computer_use_status(
    sessio_runtime_session_id: String,
    runtime: State<'_, RuntimeManager>,
) -> Option<computer_use::host::ComputerUseStatus> {
    runtime.computer_use_status(&sessio_runtime_session_id)
}

#[tauri::command]
fn set_computer_use_app_approval(
    sessio_runtime_session_id: String,
    app_id: String,
    approved: bool,
    runtime: State<'_, RuntimeManager>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
) -> Result<computer_use::settings::ComputerUseSettings, String> {
    let settings = runtime
        .set_computer_use_app_approval(&sessio_runtime_session_id, &app_id, approved)
        .map_err(|e| e.to_string())?;
    mcp_cache.refresh_from_disk().map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn set_computer_use_session_approval(
    sessio_runtime_session_id: String,
    approved: bool,
    runtime: State<'_, RuntimeManager>,
) {
    runtime.set_computer_use_session_approval(&sessio_runtime_session_id, approved);
}

#[tauri::command]
fn computer_use_abort(sessio_runtime_session_id: String, runtime: State<'_, RuntimeManager>) {
    runtime.computer_use_abort(&sessio_runtime_session_id);
}

#[tauri::command]
fn fork_agent_session(
    mut req: StartAgentSession,
    store: State<'_, Arc<dyn SessionStore>>,
    skills_cache: State<'_, skills::SkillsCache>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
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
    hydrate_skill_options(&mut req.options, skills_cache.inner());
    hydrate_mcp_options(&mut req.options, mcp_cache.inner());
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn ensure_agent_runtime_session(
    mut req: EnsureAgentRuntimeSession,
    store: State<'_, Arc<dyn SessionStore>>,
    skills_cache: State<'_, skills::SkillsCache>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    hydrate_ensure_request_from_db(&mut req, store.inner()).map_err(|e| e.to_string())?;
    hydrate_skill_options(&mut req.options, skills_cache.inner());
    hydrate_mcp_options(&mut req.options, mcp_cache.inner());
    runtime.ensure_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn dispose_agent_runtime_session(
    sessio_runtime_session_id: String,
    runtime: State<'_, RuntimeManager>,
) -> Result<(), String> {
    runtime
        .dispose_session_silent(&sessio_runtime_session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn load_agent_session(
    agent: Agent,
    runtime_session_id: String,
    workspace_path: String,
    agent_runtime_session_id: Option<String>,
    source_agent: Option<Agent>,
    store: State<'_, Arc<dyn SessionStore>>,
    skills_cache: State<'_, skills::SkillsCache>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
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
    hydrate_skill_options(&mut req.options, skills_cache.inner());
    hydrate_mcp_options(&mut req.options, mcp_cache.inner());
    runtime.ensure_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn send_agent_input(
    sessio_runtime_session_id: String,
    mut input: AgentInput,
    skills_cache: State<'_, skills::SkillsCache>,
    mcp_cache: State<'_, mcp::McpSettingsCache>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentTurnHandle, String> {
    log::info!(
        "[sessio-runtime:backend:send] session={} text={:?}",
        sessio_runtime_session_id,
        input.text
    );
    hydrate_skill_options(&mut input.options, skills_cache.inner());
    hydrate_mcp_options(&mut input.options, mcp_cache.inner());
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

/// Expose the running app binary through a stable path under the current app home
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
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init());

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let builder = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build());

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

            let sessio_home = app_home()?;
            let data_dir = sessio_home.join("db-data");
            std::fs::create_dir_all(&data_dir).ok();
            link_cli_binary(&sessio_home);
            let db_path = default_db_path()?;
            let sqlite = Arc::new(SqliteStore::open(&db_path)?);
            sqlite.init()?;
            let inner: Arc<dyn SessionStore> = sqlite.clone();
            let memory_store: Arc<dyn MemoryStore> = sqlite;
            let store: Arc<dyn SessionStore> = Arc::new(CachedStore::new(inner)?);
            let app_config = config::load_config()?;
            let mcp_settings_cache = mcp::McpSettingsCache::default();
            mcp_settings_cache.set(mcp::merged_settings(
                &app_config.mcp,
                &app_config.computer_use,
            ));
            let skills_cache = skills::SkillsCache::default();
            skills_cache.refresh_from_disk()?;
            app.manage(AppshotShortcutState::default());
            app.manage(ScreenshotOverlayState::default());
            app.manage(mcp_settings_cache);
            app.manage(skills_cache);
            if let Err(error) = update_appshot_shortcut_registration(
                &app.handle().clone(),
                &app_config.appshot.shortcut,
            ) {
                log::warn!("[appshot] failed to register shortcut at startup: {error}");
            }
            network::apply_network_proxy_env(&app_config.network.proxy);
            let runtime = RuntimeManager::new(app.handle().clone());
            if let Err(error) = runtime.start_computer_use_broker() {
                log::warn!("[computer-use:broker] failed to start at app startup: {error}");
            }
            app.manage(runtime.clone());
            let config_watcher = config_watch::ConfigWatcher::new(app.handle().clone())?;
            app.manage(config_watcher);
            let skills_watcher = skills::SkillsWatcher::new(app.handle().clone())?;
            app.manage(skills_watcher);
            app.manage(TerminalService::new(app.handle().clone()));
            let preview_file_watcher =
                file_preview_watch::PreviewFileWatcher::new(app.handle().clone())?;
            app.manage(preview_file_watcher);
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
            let scheduled_tasks_service = scheduled_tasks::ScheduledTasksService::new(
                store.clone(),
                runtime.clone(),
                astra_service.clone(),
                Some(im_bridge_service.clone()),
            );
            if let Err(error) = scheduled_tasks_service.start() {
                log::warn!("[scheduled-tasks] failed to start: {error:#}");
            }
            app.manage(scheduled_tasks_service);
            app.manage(im_bridge_service);
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
            list_terminals,
            create_terminal,
            write_terminal_input,
            resize_terminal,
            close_terminal,
            commands::sessions::list_sessions,
            commands::sessions::list_channel_sessions,
            commands::process_templates::list_process_templates,
            commands::process_templates::create_process_template,
            commands::process_templates::update_process_template,
            commands::process_templates::delete_process_template,
            commands::projects::list_projects,
            commands::projects::add_existing_project,
            create_project,
            create_default_project,
            commands::projects::update_project,
            commands::projects::archive_project,
            list_agents,
            get_astra_config,
            update_astra_config,
            update_agent_preferences,
            commands::assistants::list_assistants,
            commands::assistants::create_assistant,
            commands::assistants::update_assistant,
            commands::assistants::delete_assistant,
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
            commands::kanban::list_kanban_items,
            commands::kanban::create_kanban_item,
            commands::kanban::update_kanban_item,
            commands::kanban::update_kanban_item_status,
            commands::kanban::delete_kanban_item,
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
            commands::sessions::update_session_rename_title,
            read_local_image_data_url,
            save_pasted_attachment,
            capture_window_area_png,
            capture_frontmost_app_window_png,
            capture_selected_screen_area_png,
            capture_interactive_screen_png,
            open_screenshot_overlay_capture,
            get_screenshot_overlay_source,
            computer_use_pointer_overlay_ready,
            finish_screenshot_overlay,
            complete_screenshot_overlay_capture,
            read_local_text_file,
            read_workspace_text_file,
            write_workspace_text_file,
            get_canvas,
            save_canvas_draft,
            save_canvas_revision,
            update_canvas_blocks,
            create_canvas_context_file,
            create_canvas_anchor,
            get_file_git_diff,
            watch_preview_file,
            unwatch_preview_file,
            list_project_files,
            get_project_git_status,
            get_project_git_summary,
            get_project_git_state,
            list_project_git_commits,
            run_project_git_action,
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
            get_runtime_agent_session_config,
            get_last_runtime_agent_selection,
            set_last_runtime_agent_selection,
            commands::settings::get_debug_config,
            commands::settings::get_network_config,
            commands::settings::update_network_config,
            commands::settings::get_mcp_settings,
            list_skills,
            install_skill,
            update_mcp_settings,
            commands::settings::get_appshot_config,
            commands::settings::take_config_recovery_notice,
            get_computer_use_settings,
            get_appshot_permission_status,
            get_desktop_control_permission_status,
            get_computer_use_status,
            set_computer_use_app_approval,
            set_computer_use_session_approval,
            computer_use_abort,
            request_appshot_permission,
            open_appshot_permissions_panel,
            open_appshot_permission_settings,
            set_appshot_shortcut_recording,
            update_appshot_config,
            update_computer_use_settings,
            get_im_bridge_config,
            update_im_bridge_config,
            get_scheduled_tasks,
            save_scheduled_tasks,
            run_scheduled_task_now,
            force_unlock_scheduled_task,
            detect_telegram_user_ids,
            test_telegram_bot_connection,
            test_discord_bot_connection,
            test_feishu_bot_connection,
            test_wechat_bot_connection,
            get_wechat_qrcode,
            poll_wechat_qrcode_status,
            update_runtime_agent_preferences,
            start_agent_session,
            fork_agent_session,
            ensure_agent_runtime_session,
            dispose_agent_runtime_session,
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
