pub mod agents;
pub mod cli;
pub mod config;
pub mod indexer;
pub mod memory;
pub mod models;
pub mod polling;
pub mod store;
pub mod watch;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agents::runtime::metadata::{
    runtime_agents_with_detected_capabilities, startup_probe_runtime_agents, RuntimeAgentsCache,
};
use agents::runtime::types::{
    AgentInput, AgentSessionConfigChange, AgentSessionHandle, AgentTurnHandle,
    EnsureAgentRuntimeSession, RuntimeStatus, StartAgentSession,
};
use agents::runtime::RuntimeManager;
use indexer::{IndexTask, IndexerHandle};
use memory::qmd::{query_project, search_project, QmdOptions};
use memory::service::MemoryService;
use memory::{MemoryBackendStatus, MemoryStore};
use models::{
    Agent, KanbanItem, KanbanStatus, ProjectInfo, ProjectType, SessionHistoryBlock,
    SessionHistoryPermissionOption, SessionHistoryPermissionRequest, SessionHistoryToolCall,
    SessionHistoryTurn, SessionInfo, SessionMessage,
};
use serde_json::{json, Value};
use store::cached::CachedStore;
use store::sqlite::SqliteStore;
use store::{SessionHistoryRecord, SessionStore};
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, RunEvent, State, WindowEvent,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeAgentOptionInput {
    value: String,
    label: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRuntimeAgentPreferencesRequest {
    agent: Agent,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<String>,
    models: Option<Vec<RuntimeAgentOptionInput>>,
    efforts: Option<Vec<RuntimeAgentOptionInput>>,
    permission_modes: Option<Vec<RuntimeAgentOptionInput>>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryResult {
    pub message_count: usize,
    pub indexed_through: Option<i64>,
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
fn list_projects(store: State<'_, Arc<dyn SessionStore>>) -> Result<Vec<ProjectInfo>, String> {
    store.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
fn add_existing_project(
    path: String,
    name: Option<String>,
    project_type: Option<ProjectType>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let project = store
        .add_project(
            &path,
            name.as_deref(),
            project_type.unwrap_or(ProjectType::Code),
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
    project_type: Option<ProjectType>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let project = store
        .create_project(
            &parent_path,
            &name,
            project_type.unwrap_or(ProjectType::Code),
        )
        .map_err(|e| e.to_string())?;
    app.emit("projects_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(project)
}

#[tauri::command]
fn create_default_project(
    name: String,
    project_type: Option<ProjectType>,
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
            project_type.unwrap_or(ProjectType::Code),
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
    project_type: Option<ProjectType>,
    app: AppHandle,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<ProjectInfo, String> {
    let project = store
        .update_project(&project_id, name.as_deref(), project_type)
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

#[cfg(test)]
mod ancestor_tests {
    use super::*;

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
        let root = session(Agent::Gemini, "root", None, None, "/tmp/gemini/logs.json");
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
            SessionMessage::new(
                "user",
                "review\n[file: spec.md|file:///tmp/spec.md]",
                Some(10),
            ),
            SessionMessage::new("tool_call", "[Read]\n{\"path\":\"spec.md\"}", Some(20))
                .with_tool_call_id(Some("tool-1".to_string())),
            SessionMessage::new("tool_result", "contents", Some(21))
                .with_tool_call_id(Some("tool-1".to_string())),
            SessionMessage::new("file_edit", "{\"edits\":[]}", Some(22)),
            SessionMessage::new("assistant", "done", Some(30)),
        ];
        let turns = session_history_turns(&messages);
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
        assert_eq!(turns[0].tools[0].tool_id, "tool-1");
        assert_eq!(
            turns[0].tools[0].raw_output,
            Value::String("contents".to_string())
        );
    }

    #[test]
    fn session_history_result_serializes_turn_envelope_without_legacy_messages() {
        let result = SessionHistoryResult {
            message_count: 1,
            indexed_through: Some(10),
            turns: session_history_turns(&[SessionMessage::new(
                "user",
                "review\n[file: spec.md|file:///tmp/spec.md]",
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
    fn session_history_api_prefers_fresh_canonical_store_turns() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "sessio-history-api-{}-{suffix}.db",
            std::process::id()
        ));
        let source_path = std::env::temp_dir().join(format!(
            "sessio-history-api-{}-{suffix}.jsonl",
            std::process::id()
        ));

        std::fs::write(&source_path, "not-jsonl\n").unwrap();
        let source_path_text = source_path.to_string_lossy().to_string();
        let store = SqliteStore::open(&db_path).unwrap();
        store.init().unwrap();
        let stored_turn = SessionHistoryTurn {
            turn_id: "db-turn".to_string(),
            status: "completed".to_string(),
            blocks: Vec::new(),
            tools: Vec::new(),
            permissions: Vec::new(),
            protocol_messages: Vec::new(),
            stop_reason: None,
            error: None,
            started_at: 10,
            updated_at: 20,
        };
        store
            .replace_session_history(&SessionHistoryRecord {
                agent: Agent::Codex,
                session_id: "session-db".to_string(),
                file_path: source_path_text.clone(),
                file_size: file_size_for(&source_path_text).unwrap(),
                file_mtime: file_mtime_for_history(&source_path_text),
                message_count: 7,
                indexed_through: Some(20),
                updated_at: 30,
                turns: vec![stored_turn],
            })
            .unwrap();

        let result = read_session_history_result_with_store(
            &store,
            Agent::Codex,
            &source_path_text,
            Some("session-db"),
        )
        .unwrap();

        assert_eq!(result.message_count, 7);
        assert_eq!(result.indexed_through, Some(20));
        assert_eq!(result.turns.len(), 1);
        assert_eq!(result.turns[0].turn_id, "db-turn");

        let _ = std::fs::remove_file(&source_path);
        let _ = std::fs::remove_file(&db_path);
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
fn get_session_messages(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SessionHistoryResult, String> {
    read_session_history_result_with_store(
        store.inner().as_ref(),
        agent,
        &file_path,
        session_id.as_deref(),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_session_history(
    agent: Agent,
    file_path: String,
    session_id: Option<String>,
    store: State<'_, Arc<dyn SessionStore>>,
) -> Result<SessionHistoryResult, String> {
    read_session_history_result_with_store(
        store.inner().as_ref(),
        agent,
        &file_path,
        session_id.as_deref(),
    )
    .map_err(|e| e.to_string())
}

pub fn read_session_history_result(
    agent: Agent,
    file_path: &str,
    session_id: Option<&str>,
) -> anyhow::Result<SessionHistoryResult> {
    read_session_history_result_from_source(agent, file_path, session_id)
}

fn read_session_history_result_with_store(
    store: &dyn SessionStore,
    agent: Agent,
    file_path: &str,
    session_id: Option<&str>,
) -> anyhow::Result<SessionHistoryResult> {
    let session_id = session_id.unwrap_or_default();
    if !session_id.is_empty() {
        if let Some(record) = store.get_session_history(agent, session_id, file_path)? {
            if session_history_record_is_fresh(&record, file_path) {
                return Ok(SessionHistoryResult {
                    message_count: record.message_count,
                    indexed_through: record.indexed_through,
                    turns: record.turns,
                });
            }
        }
    }

    let result = read_session_history_result_from_source(agent, file_path, Some(session_id))?;
    if !session_id.is_empty() {
        let record = SessionHistoryRecord {
            agent,
            session_id: session_id.to_string(),
            file_path: file_path.to_string(),
            file_size: file_size_for(file_path).unwrap_or_default(),
            file_mtime: file_mtime_for_history(file_path),
            message_count: result.message_count,
            indexed_through: result.indexed_through,
            updated_at: now_ms(),
            turns: result.turns.clone(),
        };
        store.replace_session_history(&record)?;
    }
    Ok(result)
}

fn session_history_record_is_fresh(record: &SessionHistoryRecord, file_path: &str) -> bool {
    record.file_size == file_size_for(file_path).unwrap_or_default()
        && record.file_mtime == file_mtime_for_history(file_path)
}

fn file_size_for(file_path: &str) -> Option<u64> {
    std::fs::metadata(file_path)
        .ok()
        .map(|metadata| metadata.len())
}

fn file_mtime_for_history(file_path: &str) -> Option<i64> {
    std::fs::metadata(file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as i64)
        })
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
        Agent::Codex => {
            let rows = crate::agents::sources::codex::parser::read_messages_with_locations(&path)
                .map_err(anyhow::Error::from)?;
            let count = count_source_lines(&rows);
            let messages = rows.into_iter().map(|(m, _)| m).collect();
            (messages, count)
        }
        Agent::Claude => {
            let rows = crate::agents::sources::claude::parser::read_messages_with_locations(&path)
                .map_err(anyhow::Error::from)?;
            let count = count_source_lines(&rows);
            let messages = rows.into_iter().map(|(m, _)| m).collect();
            (messages, count)
        }
        Agent::Gemini => {
            let sid = session_id.unwrap_or_default();
            let messages = crate::agents::sources::gemini::parser::read_messages(&path, &sid)
                .map_err(anyhow::Error::from)?;
            let count = messages.len();
            (messages, count)
        }
    };
    let indexed_through = latest_message_timestamp(&messages);
    let turns = session_history_turns(&messages);
    Ok(SessionHistoryResult {
        message_count,
        indexed_through,
        turns,
    })
}

fn latest_message_timestamp(messages: &[SessionMessage]) -> Option<i64> {
    messages
        .iter()
        .filter_map(|message| message.timestamp)
        .max()
}

fn session_history_turns(messages: &[SessionMessage]) -> Vec<SessionHistoryTurn> {
    let mut turns = Vec::new();
    let mut current: Option<SessionHistoryTurn> = None;
    let mut tool_result_indices = HashSet::new();

    for (index, message) in messages.iter().enumerate() {
        if is_tool_result_role(&message.role) && tool_result_indices.contains(&index) {
            continue;
        }
        let timestamp = message.timestamp.unwrap_or(index as i64);
        if message.role == "user" || current.is_none() {
            if let Some(turn) = current.take() {
                turns.push(turn);
            }
            current = Some(new_session_history_turn(index, timestamp));
        }
        let turn = current.get_or_insert_with(|| new_session_history_turn(index, timestamp));
        turn.started_at = turn.started_at.min(timestamp);
        turn.updated_at = turn.updated_at.max(timestamp);
        append_session_history_message(turn, messages, index, message, &mut tool_result_indices);
    }

    if let Some(turn) = current {
        turns.push(turn);
    }
    turns
}

fn new_session_history_turn(index: usize, timestamp: i64) -> SessionHistoryTurn {
    SessionHistoryTurn {
        turn_id: format!("history-turn-{index}"),
        status: "completed".to_string(),
        blocks: Vec::new(),
        tools: Vec::new(),
        permissions: Vec::new(),
        protocol_messages: Vec::new(),
        stop_reason: None,
        error: None,
        started_at: timestamp,
        updated_at: timestamp,
    }
}

fn append_session_history_message(
    turn: &mut SessionHistoryTurn,
    messages: &[SessionMessage],
    index: usize,
    message: &SessionMessage,
    tool_result_indices: &mut HashSet<usize>,
) {
    let timestamp = message.timestamp.unwrap_or(index as i64);
    match message.role.as_str() {
        "user" => turn
            .blocks
            .push(history_message_block("user", message, timestamp)),
        "assistant" => turn
            .blocks
            .push(history_message_block("assistant", message, timestamp)),
        "thinking" => turn
            .blocks
            .push(history_message_block("thought", message, timestamp)),
        "file_edit" => turn.blocks.push(history_session_update_block(
            "file_edit",
            message,
            timestamp,
        )),
        "runtime_status" | "turn_note" => turn.blocks.push(history_session_update_block(
            &message.role,
            message,
            timestamp,
        )),
        "permission_request" => {
            let request_id = message
                .tool_call_id
                .as_deref()
                .unwrap_or("")
                .split('|')
                .find_map(|part| part.strip_prefix("request:"))
                .unwrap_or("");
            let request_id = if request_id.is_empty() {
                format!("history-permission-{index}")
            } else {
                request_id.to_string()
            };
            turn.permissions.push(SessionHistoryPermissionRequest {
                request_id: request_id.clone(),
                tool_call: Value::Null,
                tool_name: history_tool_name(&message.text),
                input: Value::String(history_tool_body(&message.text)),
                options: vec![
                    SessionHistoryPermissionOption {
                        option_id: "allow_once".to_string(),
                        name: "Allow once".to_string(),
                        kind: "allow_once".to_string(),
                        meta: Value::Null,
                    },
                    SessionHistoryPermissionOption {
                        option_id: "reject_once".to_string(),
                        name: "Reject once".to_string(),
                        kind: "reject_once".to_string(),
                        meta: Value::Null,
                    },
                ],
                selected_option_id: Some("history_resolved".to_string()),
                cancelled: false,
                raw: json!({ "source": "history", "message": message }),
            });
            turn.blocks.push(SessionHistoryBlock {
                kind: "permission".to_string(),
                blocks: Vec::new(),
                raw: None,
                tool_id: None,
                request_id: Some(request_id),
                update_type: None,
                data: None,
                error: None,
                timestamp: Some(timestamp),
            });
        }
        role if is_tool_call_role(role) => {
            let result_index = find_tool_result_index(messages, index);
            if let Some(result_index) = result_index {
                tool_result_indices.insert(result_index);
            }
            let result = result_index.and_then(|idx| messages.get(idx));
            let tool_id = message
                .tool_call_id
                .clone()
                .unwrap_or_else(|| format!("history-tool-{index}"));
            turn.tools
                .push(history_tool_json(message, result, index, &tool_id, role));
            turn.blocks.push(history_tool_block(tool_id, timestamp));
        }
        role if is_tool_result_role(role) => {
            let tool_id = message
                .tool_call_id
                .clone()
                .unwrap_or_else(|| format!("history-tool-{index}"));
            turn.tools.push(history_tool_json(
                message,
                None,
                index,
                &tool_id,
                "tool_result",
            ));
            turn.blocks.push(history_tool_block(tool_id, timestamp));
        }
        _ => turn
            .blocks
            .push(history_message_block("assistant", message, timestamp)),
    }
}

fn history_message_block(
    kind: &str,
    message: &SessionMessage,
    timestamp: i64,
) -> SessionHistoryBlock {
    SessionHistoryBlock {
        kind: kind.to_string(),
        blocks: message.content_blocks.clone(),
        raw: Some(json!({ "source": "history", "message": message })),
        tool_id: None,
        request_id: None,
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    }
}

fn history_session_update_block(
    update_type: &str,
    message: &SessionMessage,
    timestamp: i64,
) -> SessionHistoryBlock {
    SessionHistoryBlock {
        kind: "sessionUpdate".to_string(),
        blocks: Vec::new(),
        raw: None,
        tool_id: None,
        request_id: None,
        update_type: Some(update_type.to_string()),
        data: Some(json!({ "text": message.text, "timestamp": message.timestamp })),
        error: None,
        timestamp: Some(timestamp),
    }
}

fn history_tool_block(tool_id: String, timestamp: i64) -> SessionHistoryBlock {
    SessionHistoryBlock {
        kind: "tool".to_string(),
        blocks: Vec::new(),
        raw: None,
        tool_id: Some(tool_id),
        request_id: None,
        update_type: None,
        data: None,
        error: None,
        timestamp: Some(timestamp),
    }
}

fn history_tool_json(
    message: &SessionMessage,
    result: Option<&SessionMessage>,
    index: usize,
    tool_id: &str,
    kind: &str,
) -> SessionHistoryToolCall {
    let title = history_tool_name(&message.text);
    let raw_input = history_tool_body(&message.text);
    SessionHistoryToolCall {
        tool_id: tool_id.to_string(),
        title,
        kind: kind.to_string(),
        status: if result.is_some() || kind == "todo" {
            "completed".to_string()
        } else {
            "unknown".to_string()
        },
        content: Vec::new(),
        locations: Vec::new(),
        raw_input: Value::String(raw_input),
        raw_output: result
            .map(|message| Value::String(message.text.clone()))
            .unwrap_or(Value::Null),
        meta: json!({ "source": "history", "role": message.role }),
        raw: json!({ "source": "history", "message": message, "toolResult": result }),
        updated_at: message.timestamp.unwrap_or(index as i64),
    }
}

fn find_tool_result_index(messages: &[SessionMessage], call_index: usize) -> Option<usize> {
    let call = messages.get(call_index)?;
    if let Some(call_id) = &call.tool_call_id {
        for (index, candidate) in messages.iter().enumerate().skip(call_index + 1) {
            if is_tool_result_role(&candidate.role)
                && candidate.tool_call_id.as_ref() == Some(call_id)
            {
                return Some(index);
            }
            if is_tool_call_role(&candidate.role)
                && candidate.tool_call_id.as_ref() == Some(call_id)
            {
                break;
            }
        }
        return None;
    }
    let next_index = call_index + 1;
    messages
        .get(next_index)
        .filter(|candidate| {
            is_tool_result_role(&candidate.role) && candidate.tool_call_id.is_none()
        })
        .map(|_| next_index)
}

fn is_tool_call_role(role: &str) -> bool {
    matches!(
        role,
        "tool" | "tool_call" | "tool_use" | "function_call" | "todo"
    )
}

fn is_tool_result_role(role: &str) -> bool {
    matches!(role, "tool_result" | "function_call_output")
}

fn history_tool_name(text: &str) -> String {
    if let Some(rest) = text.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            return rest[..close].to_string();
        }
    }
    "Tool Use".to_string()
}

fn history_tool_body(text: &str) -> String {
    if let Some(rest) = text.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            return rest[close + 1..].trim_start_matches('\n').to_string();
        }
    }
    text.to_string()
}

#[tauri::command]
fn update_session_message_count(
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

fn count_source_lines(
    rows: &[(
        SessionMessage,
        crate::agents::sources::types::SourceLocation,
    )],
) -> usize {
    let mut lines = HashSet::new();
    for (_, location) in rows {
        if let Some(line) = location.line_start {
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
) -> Result<Vec<models::RuntimeAgentMetadata>, String> {
    Ok(cache.get())
}

#[tauri::command]
fn get_debug_config() -> Result<config::DebugConfig, String> {
    config::load_config()
        .map(|config| config.debug)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn update_runtime_agent_preferences(
    req: UpdateRuntimeAgentPreferencesRequest,
    app: AppHandle,
    cache: State<'_, RuntimeAgentsCache>,
) -> Result<models::RuntimeAgentMetadata, String> {
    let update = config::AgentRuntimePreferencesUpdate {
        model: req.model,
        effort: req.effort,
        permission_mode: req.permission_mode,
        models: req
            .models
            .unwrap_or_default()
            .into_iter()
            .map(|option| config::AgentRuntimeOptionConfig {
                value: option.value,
                label: option.label,
            })
            .collect(),
        efforts: req
            .efforts
            .unwrap_or_default()
            .into_iter()
            .map(|option| config::AgentRuntimeOptionConfig {
                value: option.value,
                label: option.label,
            })
            .collect(),
        permission_modes: req
            .permission_modes
            .unwrap_or_default()
            .into_iter()
            .map(|option| config::AgentRuntimeOptionConfig {
                value: option.value,
                label: option.label,
            })
            .collect(),
    };
    config::update_agent_runtime_preferences(req.agent, update).map_err(|e| e.to_string())?;
    let agents = runtime_agents_with_detected_capabilities(
        app.state::<Arc<dyn SessionStore>>().inner().clone(),
    )
    .map_err(|e| e.to_string())?;
    let updated = agents
        .iter()
        .find(|metadata| metadata.agent == req.agent)
        .cloned()
        .ok_or_else(|| format!("runtime agent is not configured: {}", req.agent.as_str()))?;
    cache.set(agents);
    app.emit("runtime_agents_updated", ())
        .map_err(|e| e.to_string())?;
    Ok(updated)
}

#[tauri::command]
fn start_agent_session(
    req: StartAgentSession,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn fork_agent_session(
    req: StartAgentSession,
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
    runtime.start_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn ensure_agent_runtime_session(
    req: EnsureAgentRuntimeSession,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    runtime.ensure_session(req).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_agent_session(
    agent: Agent,
    runtime_session_id: String,
    workspace_path: String,
    agent_runtime_session_id: Option<String>,
    source_agent: Option<Agent>,
    runtime: State<'_, RuntimeManager>,
) -> Result<AgentSessionHandle, String> {
    runtime
        .ensure_session(EnsureAgentRuntimeSession {
            agent,
            sessio_runtime_session_id: runtime_session_id,
            workspace_path,
            agent_runtime_session_id,
            source_agent,
        })
        .map_err(|e| e.to_string())
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

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let data_dir = dirs::home_dir()
                .ok_or_else(|| anyhow::anyhow!("no home dir"))?
                .join(".sessio")
                .join("db-data");
            std::fs::create_dir_all(&data_dir).ok();
            let db_path = data_dir.join("sessio-index.db");
            let sqlite = Arc::new(SqliteStore::open(&db_path)?);
            sqlite.init()?;
            let inner: Arc<dyn SessionStore> = sqlite.clone();
            let memory_store: Arc<dyn MemoryStore> = sqlite;
            let store: Arc<dyn SessionStore> = Arc::new(CachedStore::new(inner)?);
            let app_config = config::load_config()?;
            let indexer_handle = indexer::spawn(
                app.handle().clone(),
                store.clone(),
                memory_store.clone(),
                app_config.memory.clone(),
            );
            log::info!("indexer spawned");

            polling::spawn_polling(store.clone(), indexer_handle.clone());
            log::info!("polling thread spawned");

            match watch::spawn(indexer_handle.clone()) {
                Ok(handle) => {
                    log::info!("watcher spawned successfully");
                    // Keep watcher alive for the lifetime of the process.
                    Box::leak(Box::new(handle));
                }
                Err(e) => log::warn!("watcher failed to start: {e}"),
            }
            app.manage(store.clone());
            app.manage(memory_store);
            app.manage(indexer_handle);
            let runtime_probe_store = store.clone();
            let runtime_agents_cache = RuntimeAgentsCache::default();
            runtime_agents_cache
                .set(runtime_agents_with_detected_capabilities(store.clone()).unwrap_or_default());
            app.manage(RuntimeManager::new(app.handle().clone()));
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
                        let _ = w.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            list_projects,
            add_existing_project,
            create_project,
            create_default_project,
            update_project,
            archive_project,
            list_kanban_items,
            create_kanban_item,
            update_kanban_item,
            update_kanban_item_status,
            delete_kanban_item,
            link_kanban_item_session,
            unlink_kanban_item_session,
            get_session_ancestors,
            get_session_history,
            get_session_messages,
            update_session_message_count,
            create_pending_session,
            read_local_image_data_url,
            read_local_text_file,
            set_window_appearance,
            get_system_appearance,
            rebuild_session_index,
            get_index_status,
            get_memory_backend_status,
            search_project_memory,
            write_cross_prompt,
            get_agent_runtime_status,
            list_runtime_agents,
            get_debug_config,
            update_runtime_agent_preferences,
            start_agent_session,
            fork_agent_session,
            ensure_agent_runtime_session,
            load_agent_session,
            send_agent_input,
            cancel_agent_turn,
            set_agent_session_config_option,
            respond_agent_permission,
            remove_session_files,
            remove_sessions_by_scope
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }
        });
}
