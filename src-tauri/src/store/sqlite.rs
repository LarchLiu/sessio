use anyhow::{Context, Result};
use rusqlite::{
    params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension, ToSql,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::agents::runtime::types::RuntimeTransportKind;
use crate::agents::sources::types::SourceLocation;
use crate::memory::{
    MemoryArtifact, MemoryJob, MemoryRecord, MemoryRecordKind, MemorySource, MemoryStore,
    RecordContinuation, SessionTimeInfo, TurnFingerprint, TurnFingerprintCandidate,
};
use crate::models::{
    Agent, AgentAiProviderInfo, AgentCommandsInfo, AgentInfo, AgentType, AssistantAgentInfo,
    AssistantInfo, AssistantType, AstraConfig, CanvasBlockRecord, CanvasContextAnchor,
    CanvasDocumentInfo, CanvasDocumentState, CanvasRevisionInfo, ChannelSessionInfo, IssueSeverity,
    IssueStatus, KanbanItem, KanbanStatus, PlanRoundInfo, PlanTaskInfo, PlanTaskSessionInfo,
    PlanTaskSessionRole, ProcessTemplateInfo, ProcessTemplateType, ProjectInfo, ProjectStageInfo,
    ProjectStageType, RuntimeAgentOptionMetadata, SessionInfo, SessionOrigin, StageAssistantInfo,
    StageInfo, StageIssueInfo, StageStatus, StageType, SubagentInfo, ThreadAgentInfo,
    ThreadIndexItemInfo, ThreadInfo, ThreadKind, ThreadOrigin,
};
#[cfg(test)]
use crate::models::{PlanRoundMode, PlanRoundStatus, PlanTaskStatus, SessionHistoryTurn};
#[cfg(test)]
use crate::store::NewPlanTask;
use crate::store::{
    better_session_candidate, file_mtime_for, is_virtual_session_ref, now_ms,
    AgentPreferencesPatch, AstraConfigPatch, AstraRunRecord, AstraRunSessionRecord,
    ChannelSessionRecord, IndexedSessionRecord, IndexedSubagentRecord, NewAssistant, NewPlanRound,
    NewPlanTaskSession, PlanTaskStatusPatch, ProjectStagePatch, RuntimeAgentCapabilityRecord,
    RuntimeAgentSelection, RuntimeAgentSessionConfigRecord, ScheduledTaskRecord,
    ScheduledTaskRunRecord, SessionHistorySnapshotRecord, SessionRef, SessionStore,
    ThreadWorkSnapshotRecord, UpsertCanvasBlockRecord,
};

mod assistants;
mod astra;
mod bootstrap;
mod canvas;
mod channel_sessions;
mod identity;
mod plan_queries;
mod plans;
mod projects;
mod runtime_agents;
mod scheduled_tasks;
mod schema;
mod seed;
mod snapshots;
mod thread_index;
mod thread_queries;
mod threads;

use self::assistants::{assistant_from_row, load_assistant_by_id};
use self::identity::{
    downgrade_session_origin_when_unlinked, insert_session, upgrade_session_origin_to_thread,
};
use self::projects::load_project_by_id;
use self::thread_queries::{
    load_stage_sessions, load_thread_by_id, load_thread_stages, thread_stage_from_row,
};

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn =
            Connection::open(path).with_context(|| format!("open sqlite at {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(SqliteStore {
            conn: Mutex::new(conn),
        })
    }
}

const RUNTIME_SELECTION_KEY: &str = "last";

fn unique_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{count}")
}

fn normalize_adapter_version_key(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
fn unique_suffix() -> String {
    unique_nonce()
}

fn runtime_option(value: &str, label: &str) -> RuntimeAgentOptionMetadata {
    RuntimeAgentOptionMetadata {
        value: value.to_string(),
        label: label.to_string(),
        display_name: label.to_string(),
        enabled: true,
        order: 0,
    }
}

fn runtime_options(
    mut options: Vec<RuntimeAgentOptionMetadata>,
) -> Vec<RuntimeAgentOptionMetadata> {
    for (index, option) in options.iter_mut().enumerate() {
        option.order = index as i64;
    }
    options
}

fn runtime_options_json(options: &[RuntimeAgentOptionMetadata]) -> Result<String> {
    let map = options
        .iter()
        .enumerate()
        .map(|(index, option)| {
            (
                option.value.clone(),
                RuntimeAgentModelConfig {
                    display_name: option.display_name.clone(),
                    enabled: option.enabled,
                    order: if option.order == 0 {
                        index as i64
                    } else {
                        option.order
                    },
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&map).map_err(Into::into)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct RuntimeAgentModelConfig {
    display_name: String,
    enabled: bool,
    order: i64,
}

fn runtime_options_from_json(json: &str) -> Vec<RuntimeAgentOptionMetadata> {
    let Ok(map) = serde_json::from_str::<BTreeMap<String, RuntimeAgentModelConfig>>(json) else {
        return Vec::new();
    };
    let mut options = map
        .into_iter()
        .map(|(value, config)| RuntimeAgentOptionMetadata {
            label: config.display_name.clone(),
            display_name: config.display_name,
            value,
            enabled: config.enabled,
            order: config.order,
        })
        .collect::<Vec<_>>();
    options.sort_by(|a, b| {
        a.order
            .cmp(&b.order)
            .then_with(|| a.display_name.cmp(&b.display_name))
            .then_with(|| a.value.cmp(&b.value))
    });
    options
}

fn ai_providers_from_json(json: &str) -> Vec<AgentAiProviderInfo> {
    serde_json::from_str::<Vec<AgentAiProviderInfo>>(json).unwrap_or_default()
}

fn normalize_ai_providers_for_save(values: &[AgentAiProviderInfo]) -> Vec<AgentAiProviderInfo> {
    let mut seen = HashSet::new();
    values
        .iter()
        .enumerate()
        .map(|(index, provider)| {
            let mut id = provider.id.trim().to_string();
            if id.is_empty() || seen.contains(&id) {
                id = unique_ai_provider_id(&seen);
            }
            seen.insert(id.clone());
            let provider_id = provider.provider.trim();
            let mut next = provider.clone();
            next.id = id.clone();
            next.display_name = provider
                .display_name
                .trim()
                .to_string()
                .if_empty_then(|| provider_id.to_string())
                .if_empty_then(|| id.clone());
            next.provider = provider_id.to_string().if_empty_then(|| id.clone());
            next.api = trimmed_string(provider.api.as_deref());
            next.base_url = trimmed_string(provider.base_url.as_deref());
            next.api_key = trimmed_string(provider.api_key.as_deref());
            next.model = trimmed_string(provider.model.as_deref());
            next.models = runtime_options(provider.models.clone());
            next.order = index as i64;
            next
        })
        .collect()
}

fn unique_ai_provider_id(existing: &HashSet<String>) -> String {
    loop {
        let candidate = format!("custom-provider-{}", unique_nonce());
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
}

fn trimmed_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

trait EmptyStringFallback {
    fn if_empty_then(self, fallback: impl FnOnce() -> String) -> String;
}

impl EmptyStringFallback for String {
    fn if_empty_then(self, fallback: impl FnOnce() -> String) -> String {
        if self.is_empty() {
            fallback()
        } else {
            self
        }
    }
}

fn selected_ai_provider_id(
    providers: &[AgentAiProviderInfo],
    preferred: Option<&str>,
) -> Option<String> {
    if let Some(preferred) = preferred {
        if providers.iter().any(|provider| provider.id == preferred) {
            return Some(preferred.to_string());
        }
    }
    providers
        .iter()
        .find(|provider| provider.enabled)
        .or_else(|| providers.first())
        .map(|provider| provider.id.clone())
        .filter(|id| !id.is_empty())
}

fn transport_kind_to_db(transport: RuntimeTransportKind) -> &'static str {
    match transport {
        RuntimeTransportKind::Acp => "acp",
        RuntimeTransportKind::PiRpc => "piRpc",
        RuntimeTransportKind::Fake => "fake",
    }
}

fn transport_kind_from_db(value: &str) -> RuntimeTransportKind {
    match value {
        "piRpc" | "pi_rpc" => RuntimeTransportKind::PiRpc,
        "cliStreamJson" | "plainCli" | "sidecar" => RuntimeTransportKind::Fake,
        "fake" => RuntimeTransportKind::Fake,
        _ => RuntimeTransportKind::Acp,
    }
}

fn runtime_agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Pi => "Pi",
        Agent::Codex => "Codex",
        Agent::Claude => "Claude",
        Agent::Opencode => "OpenCode",
    }
}

fn runtime_agent_display_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Pi => "Pi",
        Agent::Codex => "Codex CLI",
        Agent::Claude => "Claude Code",
        Agent::Opencode => "OpenCode",
    }
}

fn runtime_agent_order(agent: Agent) -> i64 {
    match agent {
        Agent::Codex => 0,
        Agent::Claude => 1,
        Agent::Opencode => 2,
        Agent::Pi => 3,
    }
}

fn existing_subagent_count_state(
    conn: &Connection,
    parent_agent: Agent,
    parent_session_id: &str,
    subagent_id: &str,
) -> Result<Option<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT message_count, partial FROM subagents
         WHERE parent_agent = ? AND parent_session_id = ? AND subagent_id = ?",
    )?;
    let state = stmt
        .query_row(
            params![parent_agent.as_str(), parent_session_id, subagent_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(state)
}

fn opt_u64_to_i64(v: Option<u64>) -> Option<i64> {
    v.map(|n| n as i64)
}

fn opt_i64_to_u64(v: Option<i64>) -> Option<u64> {
    v.map(|n| n as u64)
}

fn stable_kanban_id(project_id: &str, title: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("kanban-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_issue_id(thread_stage_id: &str, title: &str, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_stage_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("issue-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_process_template_id(name: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("process-template-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_project_builtin_assistant_id(project_id: &str, template_assistant_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(template_assistant_id.as_bytes());
    format!("assistant-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_project_assistant_id(project_id: &str, template_assistant_id: &str) -> String {
    stable_project_builtin_assistant_id(project_id, template_assistant_id)
}

fn stable_process_template_builtin_assistant_id(
    process_template_id: &str,
    source_assistant_id: &str,
) -> String {
    format!("assistant-process-template-{process_template_id}-{source_assistant_id}")
}

fn stable_project_stage_id(
    process_template_id: Option<&str>,
    project_id: Option<&str>,
    stage_name: &str,
    order: i64,
    now: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(process_template_id.unwrap_or("").as_bytes());
    hasher.update(project_id.unwrap_or("").as_bytes());
    hasher.update(stage_name.as_bytes());
    hasher.update(order.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("stage-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_project_builtin_stage_id(project_id: &str, template_stage_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(template_stage_id.as_bytes());
    format!("stage-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_thread_stage_id(
    thread_id: &str,
    stage_id: &str,
    assistant_id: &str,
    order: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_id.as_bytes());
    hasher.update(stage_id.as_bytes());
    hasher.update(assistant_id.as_bytes());
    hasher.update(order.to_string().as_bytes());
    format!("thread-stage-{}", &hex::encode(hasher.finalize())[..16])
}

#[cfg(test)]
fn temp_child_path(parent: &Path, name: &str) -> std::path::PathBuf {
    parent.join(format!("{name}-{}", unique_suffix()))
}

fn process_template_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProcessTemplateInfo> {
    let process_template_type_raw: String = row.get(3)?;
    Ok(ProcessTemplateInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        process_template_type: ProcessTemplateType::from_db_str(&process_template_type_raw)
            .unwrap_or(ProcessTemplateType::Custom),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn kanban_item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<KanbanItem> {
    let status_raw: String = row.get(4)?;
    Ok(KanbanItem {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: KanbanStatus::from_db_str(&status_raw).unwrap_or(KanbanStatus::Todo),
        sort_order: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        sessions: Vec::new(),
    })
}

fn issue_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StageIssueInfo> {
    let status_raw: String = row.get(4)?;
    let severity_raw: String = row.get(5)?;
    Ok(StageIssueInfo {
        id: row.get(0)?,
        thread_stage_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: IssueStatus::from_db_str(&status_raw).unwrap_or(IssueStatus::Open),
        severity: IssueSeverity::from_db_str(&severity_raw).unwrap_or(IssueSeverity::Medium),
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn agent_info_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentInfo> {
    let ai_providers_json: String = row.get(5)?;
    let models_json: String = row.get(7)?;
    let efforts_json: String = row.get(9)?;
    let permission_modes_json: String = row.get(11)?;
    let agent_type_raw: String = row.get(12)?;
    let transport_raw: String = row.get(14)?;
    let commands_json: String = row.get(15)?;
    let ai_providers = ai_providers_from_json(&ai_providers_json);
    let models = runtime_options_from_json(&models_json);
    let efforts =
        serde_json::from_str::<Vec<RuntimeAgentOptionMetadata>>(&efforts_json).unwrap_or_default();
    let permission_modes =
        serde_json::from_str::<Vec<RuntimeAgentOptionMetadata>>(&permission_modes_json)
            .unwrap_or_default();
    let commands = serde_json::from_str::<AgentCommandsInfo>(&commands_json).unwrap_or_else(|_| {
        AgentCommandsInfo {
            session: serde_json::from_str::<Vec<String>>(&commands_json).unwrap_or_default(),
            version: Vec::new(),
        }
    });
    Ok(AgentInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        display_name: row.get(2)?,
        icon: row.get(3)?,
        ai_provider: row.get(4)?,
        ai_providers,
        model: row.get(6)?,
        models,
        effort: row.get(8)?,
        efforts,
        permission_mode: row.get(10)?,
        permission_modes,
        agent_type: AgentType::from_db_str(&agent_type_raw).unwrap_or(AgentType::Custom),
        enabled: row.get::<_, i64>(13)? != 0,
        transport: transport_kind_from_db(&transport_raw),
        commands,
        order: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
    })
}

pub(super) fn parse_string_array_json(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value)
        .unwrap_or_default()
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .fold(Vec::<String>::new(), |mut out, item| {
            if !out.contains(&item) {
                out.push(item);
            }
            out
        })
}

fn project_stage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectStageInfo> {
    let stage_type_raw: String = row.get(2)?;
    let process_template_id_raw: Option<String> = row.get(3)?;
    let stage_kind_raw: Option<String> = row.get(4)?;
    Ok(ProjectStageInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        stage_type: ProjectStageType::from_db_str(&stage_type_raw)
            .unwrap_or(ProjectStageType::Custom),
        process_template_id: process_template_id_raw,
        kind: stage_kind_raw.and_then(|value| StageType::from_db_str(&value)),
        name: row.get(5)?,
        description: row.get(6)?,
        icon: row.get(7)?,
        order: row.get(8)?,
        enabled: row.get::<_, i64>(9)? != 0,
        allow_empty_assistants: row.get::<_, i64>(10)? != 0,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        assistants: Vec::new(),
    })
}

pub(super) fn ensure_process_template_exists(
    conn: &Connection,
    process_template_id: &str,
) -> Result<()> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM process_templates WHERE id = ? LIMIT 1",
            params![process_template_id],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        anyhow::bail!("process template not found: {process_template_id}");
    }
    Ok(())
}

fn load_process_template_by_id(
    conn: &Connection,
    process_template_id: &str,
) -> Result<ProcessTemplateInfo> {
    conn.query_row(
        "SELECT id, name, description, type, created_at, updated_at
         FROM process_templates
         WHERE id = ?",
        params![process_template_id],
        process_template_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("process template not found: {process_template_id}"))
}

pub(super) fn load_agent_by_id(conn: &Connection, agent_id: &str) -> Result<AgentInfo> {
    conn.query_row(
        "SELECT id, name, display_name, icon, ai_provider, ai_providers_json,
                model, models_json, effort, efforts_json,
                permission_mode, permission_modes_json, type, enabled, transport,
                commands_json, sort_order, created_at, updated_at
         FROM agents
         WHERE id = ?",
        params![agent_id],
        agent_info_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("agent not found: {agent_id}"))
}

fn load_agents(conn: &Connection) -> Result<Vec<AgentInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, display_name, icon, ai_provider, ai_providers_json,
                model, models_json, effort, efforts_json,
                permission_mode, permission_modes_json, type, enabled, transport,
                commands_json, sort_order, created_at, updated_at
         FROM agents
         ORDER BY type ASC, sort_order ASC, display_name COLLATE NOCASE ASC",
    )?;
    let rows = stmt.query_map([], agent_info_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_project_stage_by_id(conn: &Connection, stage_id: &str) -> Result<ProjectStageInfo> {
    let mut stage = conn.query_row(
        "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
         FROM stages
         WHERE id = ?",
        params![stage_id],
        project_stage_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("project stage not found: {stage_id}"))?;
    stage.assistants = load_project_stage_assistants(conn, &stage.id)?;
    Ok(stage)
}

pub(super) fn instantiate_project_assistants(
    conn: &Connection,
    project_id: &str,
    process_template_id: &str,
    now: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
         FROM assistants
         WHERE project_id IS NULL
           AND enabled = 1
           AND (process_template_id = ? OR (process_template_id IS NULL AND type = 'custom'))
         ORDER BY CASE WHEN process_template_id = ? THEN 0 ELSE 1 END, type ASC, updated_at DESC, name COLLATE NOCASE ASC",
    )?;
    let templates = stmt
        .query_map(
            params![process_template_id, process_template_id],
            assistant_from_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for template in templates {
        let id = stable_project_assistant_id(project_id, &template.id);
        conn.execute(
            "INSERT OR IGNORE INTO assistants (
                id, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, project_id, enabled, created_at, updated_at
             )
             SELECT ?, name, agent_json, system_prompt, color, selected_skill_ids_json, selected_mcp_ids_json, type, process_template_id, ?, 1, ?, ?
             FROM assistants
             WHERE id = ?",
            params![id, project_id, now, now, template.id],
        )?;
    }
    Ok(())
}

pub(super) fn instantiate_project_builtin_stages(
    conn: &Connection,
    project_id: &str,
    process_template_id: &str,
    enabled_stage_ids: Option<&[String]>,
    now: i64,
) -> Result<()> {
    let selected_template_ids = enabled_stage_ids.map(|ids| {
        ids.iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>()
    });
    let mut stmt = conn.prepare(
        "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
         FROM stages
         WHERE project_id IS NULL AND process_template_id = ? AND type = 'builtin'
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let templates = stmt
        .query_map(params![process_template_id], project_stage_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for template in templates {
        let selected = selected_template_ids
            .as_ref()
            .map(|ids| ids.contains(template.id.as_str()))
            .unwrap_or(template.enabled);
        if !selected {
            continue;
        }
        let Some(kind) = template.kind else {
            continue;
        };
        let id = stable_project_builtin_stage_id(project_id, &template.id);
        conn.execute(
            "INSERT OR IGNORE INTO stages (
                id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
             ) VALUES (?, ?, 'builtin', ?, ?, NULL, ?, ?, ?, 1, ?, ?, ?)",
            params![
                id,
                project_id,
                process_template_id,
                kind.as_str(),
                template.description,
                template.icon,
                template.order,
                template.allow_empty_assistants as i64,
                now,
                now
            ],
        )?;
    }
    Ok(())
}

pub(super) fn link_project_stage_assistants(
    conn: &Connection,
    project_id: &str,
    process_template_id: &str,
    now: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id
         FROM stages
         WHERE project_id IS NULL AND process_template_id = ? AND type = 'builtin'
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let template_stage_ids = stmt
        .query_map(params![process_template_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for template_stage_id in template_stage_ids {
        let project_stage_id = stable_project_builtin_stage_id(project_id, &template_stage_id);
        let exists = conn
            .query_row(
                "SELECT 1 FROM stages WHERE id = ? AND project_id = ?",
                params![project_stage_id, project_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if exists {
            copy_project_stage_assistants(
                conn,
                project_id,
                &template_stage_id,
                &project_stage_id,
                now,
            )?;
        }
    }
    Ok(())
}

fn stage_has_assistants(conn: &Connection, stage_id: &str) -> Result<bool> {
    let existing_count: i64 = conn.query_row(
        "SELECT count(*) FROM stage_assistants WHERE stage_id = ?",
        params![stage_id],
        |row| row.get(0),
    )?;
    Ok(existing_count > 0)
}

fn copy_project_stage_assistants(
    conn: &Connection,
    project_id: &str,
    from_stage_id: &str,
    to_stage_id: &str,
    now: i64,
) -> Result<()> {
    if stage_has_assistants(conn, to_stage_id)? {
        return Ok(());
    }
    let mut stmt = conn.prepare(
        "SELECT assistant_id, sort_order
         FROM stage_assistants
         WHERE stage_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let bindings = stmt
        .query_map(params![from_stage_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (assistant_id, order) in bindings {
        let project_assistant_id = stable_project_assistant_id(project_id, &assistant_id);
        let Some(target_assistant_id) = conn
            .query_row(
                "SELECT id FROM assistants WHERE id = ? AND project_id = ? AND enabled = 1",
                params![project_assistant_id, project_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        else {
            continue;
        };
        conn.execute(
            "INSERT OR IGNORE INTO stage_assistants (stage_id, assistant_id, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
            params![to_stage_id, target_assistant_id, order, now, now],
        )?;
    }
    Ok(())
}

fn load_thread_stage_by_id(conn: &Connection, thread_stage_id: &str) -> Result<StageInfo> {
    let mut stage = conn
        .query_row(
            "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.process_template_id, s.kind, s.name, s.description, s.icon,
                    ts.sort_order, s.enabled, s.allow_empty_assistants, ts.created_at, ts.updated_at,
                    tss.status, tss.summary, tss.outcome
             FROM thread_stages ts
             INNER JOIN threads t ON t.id = ts.thread_id
             INNER JOIN stages s ON s.id = ts.stage_id
             LEFT JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
             WHERE ts.id = ?",
            params![thread_stage_id],
            thread_stage_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread stage not found: {thread_stage_id}"))?;
    let has_stored_state: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM thread_stage_states WHERE thread_stage_id = ? LIMIT 1",
            params![thread_stage_id],
            |row| row.get(0),
        )
        .optional()?;
    if has_stored_state.is_none() {
        let stages = load_thread_stages(conn, &stage.thread_id)?;
        if let Some(effective) = stages.into_iter().find(|item| item.id == stage.id) {
            stage.status = effective.status;
        }
    }
    stage.assistants = load_stage_assistants(conn, &stage.id)?;
    stage.assistant_ids = stage
        .assistants
        .iter()
        .map(|assistant| assistant.assistant_id.clone())
        .collect();
    stage.sessions = load_stage_sessions(conn, &stage.id)?;
    stage.issues = load_stage_issues(conn, &stage.id)?;
    Ok(stage)
}

pub(super) fn validate_assistant_for_project(
    conn: &Connection,
    project_id: &str,
    assistant_id: &str,
) -> Result<AssistantInfo> {
    let assistant = load_assistant_by_id(conn, assistant_id)?;
    if !assistant.enabled {
        anyhow::bail!("assistant is disabled");
    }
    if assistant.project_id.as_deref() == Some(project_id) {
        return Ok(assistant);
    }
    anyhow::bail!("assistant is not available for this project")
}

pub(super) fn validate_assistants_for_project(
    conn: &Connection,
    project_id: &str,
    assistant_ids: &[String],
) -> Result<Vec<AssistantInfo>> {
    let mut seen = HashSet::new();
    let mut assistants = Vec::new();
    for assistant_id in assistant_ids {
        let assistant_id = assistant_id.trim();
        if assistant_id.is_empty() || !seen.insert(assistant_id.to_string()) {
            continue;
        }
        assistants.push(validate_assistant_for_project(
            conn,
            project_id,
            assistant_id,
        )?);
    }
    Ok(assistants)
}

fn validate_assistant_for_stage(
    conn: &Connection,
    stage: &ProjectStageInfo,
    assistant_id: &str,
) -> Result<AssistantInfo> {
    let assistant = load_assistant_by_id(conn, assistant_id)?;
    if !assistant.enabled {
        anyhow::bail!("assistant is disabled");
    }
    if stage.project_id.is_some() && assistant.project_id == stage.project_id {
        return Ok(assistant);
    }
    if stage.project_id.is_none()
        && stage.process_template_id.is_some()
        && assistant.project_id.is_none()
        && assistant.process_template_id == stage.process_template_id
    {
        return Ok(assistant);
    }
    if stage.project_id.is_none()
        && stage.process_template_id.is_some()
        && assistant.project_id.is_none()
        && assistant.process_template_id.is_none()
        && assistant.assistant_type == AssistantType::Custom
    {
        return Ok(assistant);
    }
    anyhow::bail!("assistant is not available for this stage")
}

fn validate_assistants_for_stage(
    conn: &Connection,
    stage: &ProjectStageInfo,
    assistant_ids: &[String],
) -> Result<Vec<AssistantInfo>> {
    let mut seen = HashSet::new();
    let mut assistants = Vec::new();
    for assistant_id in assistant_ids {
        let assistant_id = assistant_id.trim();
        if assistant_id.is_empty() || !seen.insert(assistant_id.to_string()) {
            continue;
        }
        assistants.push(validate_assistant_for_stage(conn, stage, assistant_id)?);
    }
    Ok(assistants)
}

fn usage_list(items: &[String]) -> String {
    const LIMIT: usize = 8;
    let mut visible = items.iter().take(LIMIT).cloned().collect::<Vec<_>>();
    if items.len() > LIMIT {
        visible.push(format!("and {} more", items.len() - LIMIT));
    }
    visible.join("; ")
}

pub(super) fn ensure_assistant_can_be_disabled(
    conn: &Connection,
    assistant_id: &str,
) -> Result<()> {
    let assistant = load_assistant_by_id(conn, assistant_id)?;
    let project_id = assistant.project_id.as_deref();
    let project_stage_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(p.name, 'Unknown'),
                COALESCE(s.name, s.kind, s.id)
             FROM stage_assistants sa
             INNER JOIN stages s ON s.id = sa.stage_id
             LEFT JOIN projects p ON p.id = s.project_id
             WHERE sa.assistant_id = ?
               AND s.project_id = ?
             ORDER BY p.name COLLATE NOCASE ASC, s.sort_order ASC",
        )?;
        let rows = stmt.query_map(params![assistant_id, project_id], |row| {
            let project_name: String = row.get(0)?;
            let stage_name: String = row.get(1)?;
            Ok(format!("project \"{project_name}\" stage \"{stage_name}\""))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let process_template_stage_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(w.name, 'Unknown'),
                COALESCE(s.name, s.kind, s.id)
             FROM stage_assistants sa
             INNER JOIN stages s ON s.id = sa.stage_id
             LEFT JOIN process_templates w ON w.id = s.process_template_id
             WHERE sa.assistant_id = ?
               AND s.project_id IS NULL
               AND s.process_template_id IS NOT NULL
             ORDER BY w.name COLLATE NOCASE ASC, s.sort_order ASC",
        )?;
        let rows = stmt.query_map(params![assistant_id], |row| {
            let process_template_name: String = row.get(0)?;
            let stage_name: String = row.get(1)?;
            Ok(format!(
                "process template \"{process_template_name}\" stage \"{stage_name}\""
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let thread_stage_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(p.name, 'Unknown'),
                t.goal,
                COALESCE(s.name, s.kind, s.id)
             FROM thread_stage_assistants tsa
             INNER JOIN thread_stages ts ON ts.id = tsa.thread_stage_id
             INNER JOIN threads t ON t.id = ts.thread_id
             INNER JOIN stages s ON s.id = ts.stage_id
             LEFT JOIN projects p ON p.id = t.project_id
             WHERE tsa.assistant_id = ?
               AND ((? IS NULL AND t.project_id IS NULL) OR t.project_id = ?)
             ORDER BY p.name COLLATE NOCASE ASC, t.updated_at DESC, ts.sort_order ASC",
        )?;
        let rows = stmt.query_map(params![assistant_id, project_id, project_id], |row| {
            let project_name: String = row.get(0)?;
            let thread_goal: String = row.get(1)?;
            let stage_name: String = row.get(2)?;
            Ok(format!(
                "project \"{project_name}\" thread \"{thread_goal}\" stage \"{stage_name}\""
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let thread_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(p.name, 'Unknown'),
                t.goal
             FROM thread_assistants ta
             INNER JOIN threads t ON t.id = ta.thread_id
             LEFT JOIN projects p ON p.id = t.project_id
             WHERE ta.assistant_id = ?
               AND t.project_id = ?
             ORDER BY p.name COLLATE NOCASE ASC, t.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![assistant_id, project_id], |row| {
            let project_name: String = row.get(0)?;
            let thread_goal: String = row.get(1)?;
            Ok(format!(
                "project \"{project_name}\" thread \"{thread_goal}\""
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if !project_stage_usages.is_empty()
        || !process_template_stage_usages.is_empty()
        || !thread_stage_usages.is_empty()
        || !thread_usages.is_empty()
    {
        let mut parts = Vec::new();
        if !project_stage_usages.is_empty() {
            parts.push(format!(
                "{} project stage assistant binding(s): {}",
                project_stage_usages.len(),
                usage_list(&project_stage_usages)
            ));
        }
        if !process_template_stage_usages.is_empty() {
            parts.push(format!(
                "{} process template stage assistant binding(s): {}",
                process_template_stage_usages.len(),
                usage_list(&process_template_stage_usages)
            ));
        }
        if !thread_stage_usages.is_empty() {
            parts.push(format!(
                "{} thread stage assistant binding(s): {}",
                thread_stage_usages.len(),
                usage_list(&thread_stage_usages)
            ));
        }
        if !thread_usages.is_empty() {
            parts.push(format!(
                "{} thread assistant binding(s): {}",
                thread_usages.len(),
                usage_list(&thread_usages)
            ));
        }
        anyhow::bail!(
            "assistant is in use; remove these bindings before disabling: {}",
            parts.join(" | ")
        );
    }
    Ok(())
}

fn ensure_project_stage_can_be_disabled(conn: &Connection, stage_id: &str) -> Result<()> {
    let stage = load_project_stage_by_id(conn, stage_id)?;
    let project_id = stage.project_id.as_deref();
    let thread_stage_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(p.name, 'Unknown'),
                t.goal,
                COALESCE(s.name, s.kind, s.id)
             FROM thread_stages ts
             INNER JOIN threads t ON t.id = ts.thread_id
             INNER JOIN stages s ON s.id = ts.stage_id
             LEFT JOIN projects p ON p.id = t.project_id
             WHERE ts.stage_id = ?
               AND ((? IS NULL AND t.project_id IS NULL) OR t.project_id = ?)
             ORDER BY p.name COLLATE NOCASE ASC, t.updated_at DESC, ts.sort_order ASC",
        )?;
        let rows = stmt.query_map(params![stage_id, project_id, project_id], |row| {
            let project_name: String = row.get(0)?;
            let thread_goal: String = row.get(1)?;
            let stage_name: String = row.get(2)?;
            Ok(format!(
                "project \"{project_name}\" thread \"{thread_goal}\" stage \"{stage_name}\""
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    if !thread_stage_usages.is_empty() {
        anyhow::bail!(
            "stage is in use by {} thread stage(s); remove these stages from threads before disabling: {}",
            thread_stage_usages.len(),
            usage_list(&thread_stage_usages)
        );
    }
    Ok(())
}

pub(super) fn load_stage_assistants(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT tsa.assistant_id, a.name, a.color, tsa.agent_json, a.system_prompt, a.selected_skill_ids_json, a.selected_mcp_ids_json, tsa.sort_order
         FROM thread_stage_assistants tsa
         INNER JOIN assistants a ON a.id = tsa.assistant_id
         WHERE tsa.thread_stage_id = ?
         ORDER BY tsa.sort_order ASC, tsa.created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_stage_id], |row| {
        let agent_json: String = row.get(3)?;
        Ok(StageAssistantInfo {
            assistant_id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: String::new(),
                    mode: String::new(),
                    effort: String::new(),
                }
            }),
            system_prompt: row.get(4)?,
            selected_skill_ids: parse_string_array_json(&row.get::<_, String>(5)?),
            selected_mcp_ids: parse_string_array_json(&row.get::<_, String>(6)?),
            order: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_project_stage_assistants(
    conn: &Connection,
    stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT sa.assistant_id, a.name, a.color, a.agent_json, a.system_prompt, a.selected_skill_ids_json, a.selected_mcp_ids_json, sa.sort_order
         FROM stage_assistants sa
         INNER JOIN assistants a ON a.id = sa.assistant_id
         WHERE sa.stage_id = ?
         ORDER BY sa.sort_order ASC, sa.created_at ASC",
    )?;
    let rows = stmt.query_map(params![stage_id], |row| {
        let agent_json: String = row.get(3)?;
        Ok(StageAssistantInfo {
            assistant_id: row.get(0)?,
            name: row.get(1)?,
            color: row.get(2)?,
            agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: String::new(),
                    mode: String::new(),
                    effort: String::new(),
                }
            }),
            system_prompt: row.get(4)?,
            selected_skill_ids: parse_string_array_json(&row.get::<_, String>(5)?),
            selected_mcp_ids: parse_string_array_json(&row.get::<_, String>(6)?),
            order: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn stage_assistant_from_assistant(assistant: AssistantInfo, order: i64) -> StageAssistantInfo {
    StageAssistantInfo {
        assistant_id: assistant.id,
        name: assistant.name,
        color: assistant.color,
        agent: assistant.agent,
        system_prompt: assistant.system_prompt,
        selected_skill_ids: assistant.selected_skill_ids,
        selected_mcp_ids: assistant.selected_mcp_ids,
        order,
    }
}

fn normalize_assistant_agent(
    conn: &Connection,
    mut agent: AssistantAgentInfo,
) -> Result<AssistantAgentInfo> {
    agent.id = agent.id.trim().to_string();
    agent.name = agent.name.trim().to_string();
    agent.model = agent.model.trim().to_string();
    agent.mode = agent.mode.trim().to_string();
    agent.effort = agent.effort.trim().to_string();
    if agent.id.is_empty() {
        anyhow::bail!("assistant agent id cannot be empty");
    }
    if agent.model.is_empty() {
        anyhow::bail!("assistant model cannot be empty");
    }
    if agent.mode.is_empty() {
        anyhow::bail!("assistant permission mode cannot be empty");
    }
    if agent.effort.is_empty() {
        anyhow::bail!("assistant effort cannot be empty");
    }
    let db_agent = load_agent_by_id(conn, &agent.id)?;
    agent.name = db_agent.name;
    Ok(agent)
}

fn replace_thread_stage_assistants(
    conn: &Connection,
    thread_stage_id: &str,
    assistants: &[StageAssistantInfo],
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM thread_stage_assistants WHERE thread_stage_id = ?",
        params![thread_stage_id],
    )?;
    for (index, assistant) in assistants.iter().enumerate() {
        let agent_json = serde_json::to_string(&assistant.agent)?;
        conn.execute(
            "INSERT INTO thread_stage_assistants (thread_stage_id, assistant_id, agent_json, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                thread_stage_id,
                assistant.assistant_id,
                agent_json,
                index as i64,
                now,
                now
            ],
        )?;
    }
    Ok(())
}

fn replace_project_stage_assistants(
    conn: &Connection,
    stage_id: &str,
    assistants: &[AssistantInfo],
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM stage_assistants WHERE stage_id = ?",
        params![stage_id],
    )?;
    for (index, assistant) in assistants.iter().enumerate() {
        conn.execute(
            "INSERT INTO stage_assistants (stage_id, assistant_id, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
            params![stage_id, assistant.id, index as i64, now, now],
        )?;
    }
    Ok(())
}

fn next_thread_stage_id(conn: &Connection, thread_id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT id
         FROM thread_stages
         WHERE thread_id = ?
         ORDER BY sort_order ASC, created_at ASC
         LIMIT 1",
        params![thread_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn compact_stage_order(conn: &Connection, thread_id: &str) -> Result<()> {
    let ids: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT id
             FROM thread_stages
             WHERE thread_id = ?
             ORDER BY sort_order ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![thread_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE thread_stages SET sort_order = ? WHERE id = ?",
            params![index as i64, id],
        )?;
    }
    Ok(())
}

fn reorder_project_stage_scope(
    conn: &Connection,
    stage_id: &str,
    process_template_id: &str,
    project_id: Option<&str>,
    target_order: i64,
) -> Result<i64> {
    let rows: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT id, sort_order
             FROM stages
             WHERE process_template_id = ?
               AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)
             ORDER BY sort_order ASC, type ASC, project_id IS NOT NULL ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(
            params![process_template_id, project_id, project_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let Some(current_index) = rows.iter().position(|(id, _)| id == stage_id) else {
        anyhow::bail!("project stage not found in reorder scope: {stage_id}");
    };
    let Some(target_index) = rows
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != current_index)
        .find(|(_, (_, order))| *order == target_order)
        .map(|(index, _)| index)
    else {
        return Ok(rows[current_index].1);
    };
    if current_index == target_index {
        return Ok(rows[current_index].1);
    }

    let mut ids = rows.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let id = ids.remove(current_index);
    ids.insert(target_index, id);

    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE stages SET sort_order = ? WHERE id = ?",
            params![-((index as i64) + 1), id],
        )?;
    }
    let mut next_order = 0;
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE stages SET sort_order = ? WHERE id = ?",
            params![index as i64, id],
        )?;
        if id == stage_id {
            next_order = index as i64;
        }
    }
    Ok(next_order)
}

fn load_kanban_item_by_id(conn: &Connection, item_id: &str) -> Result<KanbanItem> {
    let mut item = conn
        .query_row(
            "SELECT id, project_id, title, description, status, sort_order, created_at, updated_at
         FROM kanban_items
         WHERE id = ?",
            params![item_id],
            kanban_item_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("kanban item not found: {item_id}"))?;
    item.sessions = load_kanban_item_sessions(conn, &item.id)?;
    Ok(item)
}

pub(super) fn load_stage_issues(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageIssueInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, thread_stage_id, title, description, status, severity, created_at, updated_at
         FROM thread_stage_issues
         WHERE thread_stage_id = ?
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_stage_id], issue_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_stage_issue_by_id(conn: &Connection, issue_id: &str) -> Result<StageIssueInfo> {
    conn.query_row(
        "SELECT id, thread_stage_id, title, description, status, severity, created_at, updated_at
         FROM thread_stage_issues
         WHERE id = ?",
        params![issue_id],
        issue_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("issue not found: {issue_id}"))
}

fn load_kanban_item_sessions(conn: &Connection, item_id: &str) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let sql = format!(
        "SELECT {SESSION_INFO_COLUMNS_S}
         FROM kanban_item_sessions kis
         INNER JOIN sessions s ON s.agent = kis.agent AND s.session_id = kis.session_id
         WHERE kis.item_id = ? AND s.available = 1
         ORDER BY s.updated_at DESC, s.started_at DESC",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![item_id], session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    for session in sessions.iter_mut() {
        session.subagents = subs_by_parent
            .remove(&(session.agent, session.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

fn attach_kanban_item_sessions(conn: &Connection, items: &mut [KanbanItem]) -> Result<()> {
    for item in items {
        item.sessions = load_kanban_item_sessions(conn, &item.id)?;
    }
    Ok(())
}

pub(super) fn dedupe_sessions(sessions: &mut Vec<SessionInfo>) {
    let mut selected: HashMap<(Agent, String), usize> = HashMap::new();
    let mut keep = vec![true; sessions.len()];

    for index in 0..sessions.len() {
        let key = (sessions[index].agent, sessions[index].id.clone());
        if let Some(previous) = selected.get(&key).copied() {
            if better_session_candidate(&sessions[index], &sessions[previous]) {
                keep[previous] = false;
                selected.insert(key, index);
            } else {
                keep[index] = false;
            }
        } else {
            selected.insert(key, index);
        }
    }

    let mut index = 0;
    sessions.retain(|_| {
        let retain = keep[index];
        index += 1;
        retain
    });
}

fn session_project_path(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT project_path
         FROM sessions
         WHERE agent = ? AND session_id = ? AND available = 1
         ORDER BY updated_at DESC, started_at DESC
         LIMIT 1",
        params![agent.as_str(), session_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn ensure_session_not_linked_to_thread_process(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<()> {
    let linked_count: i64 = conn.query_row(
        "SELECT
            (SELECT count(*) FROM thread_sessions WHERE agent = ? AND session_id = ?) +
            (SELECT count(*) FROM stage_sessions WHERE agent = ? AND session_id = ?)",
        params![agent.as_str(), session_id, agent.as_str(), session_id],
        |row| row.get(0),
    )?;
    if linked_count > 0 {
        anyhow::bail!("session is already linked to a thread or stage");
    }
    Ok(())
}

fn read_record_kind(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<MemoryRecordKind> {
    let raw: String = row.get(idx)?;
    MemoryRecordKind::from_db_str(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::other(e.to_string())),
        )
    })
}

fn upsert_subagent_inner(
    conn: &Connection,
    parent_agent: Agent,
    parent_session_id: &str,
    sub: &SubagentInfo,
) -> Result<()> {
    let (message_count, partial) =
        existing_subagent_count_state(conn, parent_agent, parent_session_id, &sub.id)?
            .unwrap_or((sub.message_count as i64, sub.partial as i64));
    conn.execute(
        "INSERT OR REPLACE INTO subagents (
            parent_agent, parent_session_id, subagent_id, file_path,
            agent_type, description,
            started_at, updated_at,
            message_count, first_user_message,
            file_size, file_mtime, partial, available
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?, ?,?,?,?)",
        params![
            parent_agent.as_str(),
            parent_session_id,
            sub.id,
            sub.file_path,
            sub.agent_type,
            sub.description,
            sub.started_at,
            sub.updated_at,
            message_count,
            sub.first_user_message,
            sub.file_size as i64,
            file_mtime_for(&sub.file_path),
            partial,
            sub.available as i64,
        ],
    )?;
    Ok(())
}

pub(super) fn load_all_subagents_grouped(
    conn: &Connection,
) -> Result<HashMap<(Agent, String), Vec<SubagentInfo>>> {
    let mut stmt = conn.prepare(
        "SELECT parent_agent, parent_session_id,
                subagent_id, file_path, agent_type, description,
                started_at, updated_at, message_count, first_user_message,
                file_size, partial, available
         FROM subagents
         ORDER BY started_at ASC",
    )?;
    let mut grouped: HashMap<(Agent, String), Vec<SubagentInfo>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        let agent_str: String = row.get(0)?;
        let parent_session_id: String = row.get(1)?;
        let sub = SubagentInfo {
            id: row.get(2)?,
            file_path: row.get(3)?,
            agent_type: row.get(4)?,
            description: row.get(5)?,
            started_at: row.get(6)?,
            updated_at: row.get(7)?,
            message_count: row.get::<_, i64>(8)? as usize,
            first_user_message: row.get(9)?,
            file_size: row.get::<_, i64>(10)? as u64,
            partial: row.get::<_, i64>(11)? != 0,
            available: row.get::<_, i64>(12)? != 0,
        };
        Ok((agent_str, parent_session_id, sub))
    })?;
    for r in rows {
        let (agent_str, parent_session_id, sub) = r?;
        let Some(agent) = Agent::from_db_str(&agent_str) else {
            continue;
        };
        grouped
            .entry((agent, parent_session_id))
            .or_default()
            .push(sub);
    }
    Ok(grouped)
}

fn load_subagents_for_refs(
    conn: &Connection,
    refs: &[(Agent, String)],
) -> Result<HashMap<(Agent, String), Vec<SubagentInfo>>> {
    if refs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut sql = String::from(
        "SELECT parent_agent, parent_session_id,
                subagent_id, file_path, agent_type, description,
                started_at, updated_at, message_count, first_user_message,
                file_size, partial, available
         FROM subagents
         WHERE (parent_agent, parent_session_id) IN (",
    );
    let mut values = Vec::<SqlValue>::with_capacity(refs.len() * 2);
    for (index, (agent, session_id)) in refs.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str("(?, ?)");
        values.push(SqlValue::from(agent.as_str().to_string()));
        values.push(SqlValue::from(session_id.clone()));
    }
    sql.push_str(") ORDER BY started_at ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), |row| {
        let agent_str: String = row.get(0)?;
        let parent_session_id: String = row.get(1)?;
        let sub = SubagentInfo {
            id: row.get(2)?,
            file_path: row.get(3)?,
            agent_type: row.get(4)?,
            description: row.get(5)?,
            started_at: row.get(6)?,
            updated_at: row.get(7)?,
            message_count: row.get::<_, i64>(8)? as usize,
            first_user_message: row.get(9)?,
            file_size: row.get::<_, i64>(10)? as u64,
            partial: row.get::<_, i64>(11)? != 0,
            available: row.get::<_, i64>(12)? != 0,
        };
        Ok((agent_str, parent_session_id, sub))
    })?;
    let mut grouped = HashMap::<(Agent, String), Vec<SubagentInfo>>::new();
    for row in rows {
        let (agent_str, parent_session_id, sub) = row?;
        let Some(agent) = Agent::from_db_str(&agent_str) else {
            continue;
        };
        grouped
            .entry((agent, parent_session_id))
            .or_default()
            .push(sub);
    }
    Ok(grouped)
}

fn load_all_indexed_subagents_grouped(
    conn: &Connection,
) -> Result<HashMap<(Agent, String), Vec<IndexedSubagentRecord>>> {
    let mut stmt = conn.prepare(
        "SELECT parent_agent, parent_session_id,
                subagent_id, file_path, file_size, file_mtime, available
         FROM subagents",
    )?;
    let mut grouped: HashMap<(Agent, String), Vec<IndexedSubagentRecord>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        let agent_str: String = row.get(0)?;
        let parent_session_id: String = row.get(1)?;
        let subagent_id: String = row.get(2)?;
        let file_path: String = row.get(3)?;
        let file_size = row.get::<_, i64>(4)? as u64;
        let file_mtime: Option<i64> = row.get(5)?;
        let available: bool = row.get::<_, i64>(6)? != 0;
        Ok((
            agent_str,
            parent_session_id,
            subagent_id,
            file_path,
            file_size,
            file_mtime,
            available,
        ))
    })?;
    for r in rows {
        let (
            agent_str,
            parent_session_id,
            subagent_id,
            file_path,
            file_size,
            file_mtime,
            available,
        ) = r?;
        let Some(agent) = Agent::from_db_str(&agent_str) else {
            continue;
        };
        let rec = IndexedSubagentRecord {
            parent_agent: agent,
            parent_session_id: parent_session_id.clone(),
            parent_scope: String::new(), // filled in by list_indexed_sessions
            subagent_id,
            file_path,
            file_size,
            file_mtime,
            available,
        };
        grouped
            .entry((agent, parent_session_id))
            .or_default()
            .push(rec);
    }
    Ok(grouped)
}

/// Column list for any SELECT that hydrates a [`SessionInfo`] via
/// [`session_info_from_row`]. Every reader must use the same list — the row
/// mapper reads by positional index. The `s.` prefix lets this be reused as
/// either an unaliased or aliased projection (callers concat their own FROM).
pub(super) const SESSION_INFO_COLUMNS_S: &str =
    "s.agent, s.session_id, s.file_path, s.project_path, s.project_name,
        s.started_at, s.updated_at, s.message_count, s.rename_title, s.title, s.first_user_message,
        s.file_size, s.partial, s.available, s.archived, s.forked_from_agent, s.forked_from_id,
        s.origin, s.scheduled_task_id, s.is_auxiliary";

/// Same projection without the `s.` table alias, for queries that select
/// directly from `sessions`.
const SESSION_INFO_COLUMNS: &str = "agent, session_id, file_path, project_path, project_name,
        started_at, updated_at, message_count, rename_title, title, first_user_message,
        file_size, partial, available, archived, forked_from_agent, forked_from_id,
        origin, scheduled_task_id, is_auxiliary";

pub(super) fn session_info_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionInfo> {
    let agent_str: String = row.get(0)?;
    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
    let origin_raw: String = row.get(17)?;
    Ok(SessionInfo {
        id: row.get(1)?,
        agent,
        forked_from_agent: row
            .get::<_, Option<String>>(15)?
            .and_then(|value| Agent::from_db_str(&value)),
        forked_from_id: row.get(16)?,
        file_path: row.get(2)?,
        project_path: row.get(3)?,
        project_name: row.get(4)?,
        started_at: row.get(5)?,
        updated_at: row.get(6)?,
        message_count: row.get::<_, i64>(7)? as usize,
        rename_title: row.get(8)?,
        title: row.get(9)?,
        first_user_message: row.get(10)?,
        file_size: row.get::<_, i64>(11)? as u64,
        partial: row.get::<_, i64>(12)? != 0,
        available: row.get::<_, i64>(13)? != 0,
        archived: row.get::<_, i64>(14)? != 0,
        origin: SessionOrigin::from_db_str(&origin_raw).unwrap_or_default(),
        scheduled_task_id: row.get(18)?,
        is_auxiliary: row.get::<_, i64>(19)? != 0,
        subagents: Vec::new(),
    })
}

fn load_sessions_by_refs(conn: &Connection, refs: &[SessionRef<'_>]) -> Result<Vec<SessionInfo>> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }
    let unique_refs = refs
        .iter()
        .map(|session_ref| (session_ref.agent, session_ref.session_id.to_string()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut sql = format!(
        "SELECT {SESSION_INFO_COLUMNS}
         FROM sessions
         WHERE (agent, session_id) IN (",
    );
    let mut values = Vec::<SqlValue>::with_capacity(unique_refs.len() * 2);
    for (index, (agent, session_id)) in unique_refs.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push_str("(?, ?)");
        values.push(SqlValue::from(agent.as_str().to_string()));
        values.push(SqlValue::from(session_id.clone()));
    }
    sql.push_str(
        ")
         ORDER BY available DESC,
                  partial ASC,
                  CASE
                      WHEN trim(file_path) = '' OR file_path LIKE 'astra://%' THEN 0
                      ELSE 1
                  END DESC,
                  CASE WHEN trim(file_path) = '' THEN 0 ELSE 1 END DESC,
                  updated_at DESC,
                  started_at DESC",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params_from_iter(values.iter()), session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    let requested = sessions
        .iter()
        .map(|session| (session.agent, session.id.clone()))
        .collect::<Vec<_>>();
    let mut subs_by_parent = load_subagents_for_refs(conn, &requested)?;
    for session in &mut sessions {
        session.subagents = subs_by_parent
            .remove(&(session.agent, session.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

fn load_sessions(conn: &Connection, user_projects_only: bool) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    // Sidebar filter contract: only `chat` and `channel` origin sessions are
    // shown directly. `thread` origin sessions are represented by their parent
    // thread item, and auxiliary sessions (guardian, Astra delegated, pi fake,
    // scheduled-task summary push) are hidden regardless of origin.
    let sql = if user_projects_only {
        format!(
            "SELECT {SESSION_INFO_COLUMNS_S}
             FROM sessions s
             INNER JOIN projects p ON p.path = s.project_path AND p.archived = 0
             WHERE s.is_auxiliary = 0 AND s.origin IN ('chat', 'channel')
             ORDER BY s.updated_at DESC"
        )
    } else {
        format!(
            "SELECT {SESSION_INFO_COLUMNS}
             FROM sessions
             WHERE is_auxiliary = 0 AND origin IN ('chat', 'channel')
             ORDER BY updated_at DESC"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map([], session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    for s in sessions.iter_mut() {
        s.subagents = subs_by_parent
            .remove(&(s.agent, s.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

impl SessionStore for SqliteStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        bootstrap::initialize_schema(&conn)
    }

    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        load_sessions(&conn, true)
    }

    fn list_all_sessions(&self) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        load_sessions(&conn, false)
    }

    fn list_sessions_by_refs(&self, refs: &[SessionRef<'_>]) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        load_sessions_by_refs(&conn, refs)
    }

    fn list_channel_sessions(&self) -> Result<Vec<ChannelSessionInfo>> {
        let conn = self.conn.lock().unwrap();
        channel_sessions::list_channel_sessions(&conn)
    }

    fn get_active_channel_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<Option<ChannelSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        channel_sessions::get_active_channel_session(&conn, platform, channel_id)
    }

    fn upsert_channel_session(&self, record: &ChannelSessionRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        channel_sessions::upsert_channel_session(&conn, record)
    }

    fn update_channel_session_activity(
        &self,
        platform: &str,
        channel_id: &str,
        last_update_id: Option<i64>,
        last_activity_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        channel_sessions::update_channel_session_activity(
            &conn,
            platform,
            channel_id,
            last_update_id,
            last_activity_at,
        )
    }

    fn mark_channel_session_ended(
        &self,
        platform: &str,
        channel_id: &str,
        agent: Agent,
        agent_session_id: &str,
        ended_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        channel_sessions::mark_channel_session_ended(
            &conn,
            platform,
            channel_id,
            agent,
            agent_session_id,
            ended_at,
        )
    }

    fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskRecord>> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::list_scheduled_tasks(&conn)
    }

    fn list_scheduled_task_runs(&self) -> Result<Vec<ScheduledTaskRunRecord>> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::list_scheduled_task_runs(&conn)
    }

    fn list_scheduled_task_runs_requiring_update(&self) -> Result<Vec<ScheduledTaskRunRecord>> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::list_scheduled_task_runs_requiring_update(&conn)
    }

    fn replace_scheduled_tasks(&self, tasks: &[ScheduledTaskRecord]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        scheduled_tasks::replace_scheduled_tasks(&mut conn, tasks)
    }

    fn insert_scheduled_task_run(&self, run: &ScheduledTaskRunRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::insert_scheduled_task_run(&conn, run)
    }

    fn update_scheduled_task_run_status(
        &self,
        run_id: &str,
        status: &str,
        completed_at_ms: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::update_scheduled_task_run_status(
            &conn,
            run_id,
            status,
            completed_at_ms,
            error,
        )
    }

    fn update_scheduled_task_run_agent_session_id(
        &self,
        run_id: &str,
        agent_session_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::update_scheduled_task_run_agent_session_id(&conn, run_id, agent_session_id)
    }

    fn update_scheduled_task_run_push(
        &self,
        run_id: &str,
        push_status: &str,
        push_summary: Option<&str>,
        push_error: Option<&str>,
        push_sent_at_ms: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::update_scheduled_task_run_push(
            &conn,
            run_id,
            push_status,
            push_summary,
            push_error,
            push_sent_at_ms,
        )
    }

    fn update_scheduled_task_last_run(&self, task_id: &str, when_ms: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::update_scheduled_task_last_run(&conn, task_id, when_ms)
    }

    fn fail_interrupted_task_run_pushes(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        scheduled_tasks::fail_interrupted_task_run_pushes(&conn)
    }

    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut subs_by_parent = load_all_indexed_subagents_grouped(&conn)?;
        let mut stmt = conn.prepare(
            "SELECT agent, session_id, scope, file_path, file_size, file_mtime, last_indexed_at, available, archived,
                    forked_from_agent, forked_from_id
             FROM sessions",
        )?;
        let mut sessions: Vec<IndexedSessionRecord> = stmt
            .query_map([], |row| {
                let agent_str: String = row.get(0)?;
                let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                Ok(IndexedSessionRecord {
                    agent,
                    session_id: row.get(1)?,
                    scope: row.get(2)?,
                    file_path: row.get(3)?,
                    forked_from_agent: row
                        .get::<_, Option<String>>(9)?
                        .and_then(|value| Agent::from_db_str(&value)),
                    forked_from_id: row.get(10)?,
                    file_size: row.get::<_, i64>(4)? as u64,
                    file_mtime: row.get(5)?,
                    last_indexed_at: row.get(6)?,
                    available: row.get::<_, i64>(7)? != 0,
                    archived: row.get::<_, i64>(8)? != 0,
                    subagents: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for s in sessions.iter_mut() {
            let mut subs = subs_by_parent
                .remove(&(s.agent, s.session_id.clone()))
                .unwrap_or_default();
            // The grouped loader doesn't know the parent's scope (subagents
            // table doesn't store it); fill it in from the parent session row.
            for sub in subs.iter_mut() {
                sub.parent_scope = s.scope.clone();
            }
            s.subagents = subs;
        }
        Ok(sessions)
    }

    fn update_session_rename_title(
        &self,
        agent: Agent,
        session_id: &str,
        rename_title: Option<&str>,
    ) -> Result<()> {
        let rename_title = rename_title.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions
             SET rename_title = ?, last_indexed_at = ?
             WHERE agent = ? AND session_id = ?",
            params![rename_title, now_ms(), agent.as_str(), session_id],
        )?;
        Ok(())
    }

    fn list_process_templates(&self) -> Result<Vec<ProcessTemplateInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, type, created_at, updated_at
             FROM process_templates
             ORDER BY type ASC, name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], process_template_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn create_process_template(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<ProcessTemplateInfo> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("process template name cannot be empty");
        }
        let description = description.map(str::trim).filter(|value| !value.is_empty());
        let now = now_ms();
        let id = stable_process_template_id(name, now);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO process_templates (id, name, description, type, created_at, updated_at)
             VALUES (?, ?, ?, 'custom', ?, ?)",
            params![id, name, description, now, now],
        )?;
        load_process_template_by_id(&conn, &id)
    }

    fn update_process_template(
        &self,
        process_template_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<ProcessTemplateInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_process_template_by_id(&conn, process_template_id)?;
        if current.process_template_type == ProcessTemplateType::Builtin {
            anyhow::bail!("builtin process template cannot be updated");
        }
        let next_name = match name {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    anyhow::bail!("process template name cannot be empty");
                }
                value.to_string()
            }
            None => current.name,
        };
        let next_description = match description {
            Some(Some(value)) => {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.trim().to_string())
                }
            }
            Some(None) => None,
            None => current.description,
        };
        conn.execute(
            "UPDATE process_templates SET name = ?, description = ?, updated_at = ? WHERE id = ?",
            params![next_name, next_description, now_ms(), process_template_id],
        )?;
        load_process_template_by_id(&conn, process_template_id)
    }

    fn delete_process_template(&self, process_template_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let current = load_process_template_by_id(&conn, process_template_id)?;
        if current.process_template_type == ProcessTemplateType::Builtin {
            anyhow::bail!("builtin process template cannot be deleted");
        }
        let project_count: i64 = conn.query_row(
            "SELECT count(*) FROM projects WHERE process_template_id = ? AND archived = 0",
            params![process_template_id],
            |row| row.get(0),
        )?;
        if project_count > 0 {
            anyhow::bail!("process template is used by projects");
        }
        let assistant_count: i64 = conn.query_row(
            "SELECT count(*) FROM assistants WHERE process_template_id = ?",
            params![process_template_id],
            |row| row.get(0),
        )?;
        if assistant_count > 0 {
            anyhow::bail!("process template is used by assistants");
        }
        let stage_count: i64 = conn.query_row(
            "SELECT count(*) FROM stages WHERE process_template_id = ?",
            params![process_template_id],
            |row| row.get(0),
        )?;
        if stage_count > 0 {
            anyhow::bail!("process template is used by stages");
        }
        let changed = conn.execute(
            "DELETE FROM process_templates WHERE id = ?",
            params![process_template_id],
        )?;
        if changed == 0 {
            anyhow::bail!("process template not found: {process_template_id}");
        }
        Ok(())
    }

    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().unwrap();
        projects::list_projects(&conn)
    }

    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        let mut conn = self.conn.lock().unwrap();
        projects::add_project(
            &mut conn,
            path,
            name,
            process_template_id,
            enabled_stage_ids,
        )
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        let mut conn = self.conn.lock().unwrap();
        projects::create_project(
            &mut conn,
            parent_path,
            name,
            process_template_id,
            enabled_stage_ids,
        )
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo> {
        let mut conn = self.conn.lock().unwrap();
        projects::update_project(&mut conn, project_id, name, process_template_id)
    }

    fn archive_project(&self, project_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        projects::archive_project(&conn, project_id)
    }

    fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let conn = self.conn.lock().unwrap();
        load_agents(&conn)
    }

    fn get_astra_config(&self) -> Result<AstraConfig> {
        let conn = self.conn.lock().unwrap();
        astra::get_astra_config(&conn)
    }

    fn update_astra_config(&self, patch: AstraConfigPatch<'_>) -> Result<AstraConfig> {
        let conn = self.conn.lock().unwrap();
        astra::update_astra_config(&conn, patch)
    }

    fn update_agent_preferences_by_id(
        &self,
        agent_id: &str,
        patch: AgentPreferencesPatch<'_>,
    ) -> Result<AgentInfo> {
        let AgentPreferencesPatch {
            display_name,
            enabled,
            order,
            ai_provider,
            ai_providers,
            commands,
            model,
            effort,
            permission_mode,
            models,
            efforts,
            permission_modes,
        } = patch;
        let conn = self.conn.lock().unwrap();
        let id = agent_id.trim();
        if id.is_empty() {
            anyhow::bail!("agentId is required");
        }
        let current = load_agent_by_id(&conn, id)?;
        if current.agent_type != AgentType::Builtin {
            anyhow::bail!("agent is not builtin: {id}");
        }
        let next_display_name = display_name
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let normalized_ai_providers = ai_providers.map(normalize_ai_providers_for_save);
        let next_ai_provider = selected_ai_provider_id(
            normalized_ai_providers
                .as_deref()
                .unwrap_or(&current.ai_providers),
            ai_provider
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .or(current.ai_provider.as_deref()),
        );
        let next_ai_providers = match normalized_ai_providers.as_deref() {
            Some(values) => serde_json::to_string(values)?,
            None => serde_json::to_string(&current.ai_providers)?,
        };
        let next_commands = match commands {
            Some(values) => serde_json::to_string(values)?,
            None => serde_json::to_string(&current.commands)?,
        };
        let trimmed_model = model.map(str::trim);
        let next_model = trimmed_model.filter(|value| !value.is_empty());
        let next_effort = effort.map(str::trim).filter(|value| !value.is_empty());
        let next_permission_mode = permission_mode
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let next_models = match models {
            Some(values) => runtime_options_json(values)?,
            None => runtime_options_json(&current.models)?,
        };
        let next_efforts = match efforts {
            Some(values) => serde_json::to_string(values)?,
            None => serde_json::to_string(&current.efforts)?,
        };
        let next_permission_modes = match permission_modes {
            Some(values) => serde_json::to_string(values)?,
            None => serde_json::to_string(&current.permission_modes)?,
        };
        let now = now_ms();
        conn.execute(
            "UPDATE agents
             SET display_name = COALESCE(?, display_name),
                 ai_provider = COALESCE(?, ai_provider),
                 ai_providers_json = ?,
                 commands_json = ?,
                 model = COALESCE(?, model),
                 models_json = ?,
                 effort = COALESCE(?, effort),
                 efforts_json = ?,
                 permission_mode = COALESCE(?, permission_mode),
                 permission_modes_json = ?,
                 enabled = COALESCE(?, enabled),
                 sort_order = COALESCE(?, sort_order),
                 updated_at = ?
             WHERE id = ? AND type = 'builtin'",
            params![
                next_display_name,
                next_ai_provider.as_deref(),
                next_ai_providers,
                next_commands,
                next_model,
                next_models,
                next_effort,
                next_efforts,
                next_permission_mode,
                next_permission_modes,
                enabled.map(|value| if value { 1_i64 } else { 0_i64 }),
                order,
                now,
                id,
            ],
        )?;
        load_agent_by_id(&conn, id)
    }

    fn update_builtin_agent_preferences(
        &self,
        agent: Agent,
        patch: AgentPreferencesPatch<'_>,
    ) -> Result<AgentInfo> {
        let id = agent.as_str();
        self.update_agent_preferences_by_id(id, patch)
    }

    fn get_last_runtime_agent_selection(&self) -> Result<Option<RuntimeAgentSelection>> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::get_last_runtime_agent_selection(&conn)
    }

    fn set_last_runtime_agent_selection(
        &self,
        agent: Agent,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<RuntimeAgentSelection> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::set_last_runtime_agent_selection(
            &conn,
            agent,
            model,
            effort,
            permission_mode,
        )
    }

    fn list_assistants(&self, project_id: Option<&str>) -> Result<Vec<AssistantInfo>> {
        let conn = self.conn.lock().unwrap();
        assistants::list_assistants(&conn, project_id)
    }

    fn create_assistant(&self, assistant: NewAssistant<'_>) -> Result<AssistantInfo> {
        let conn = self.conn.lock().unwrap();
        assistants::create_assistant(&conn, assistant)
    }

    fn update_assistant(
        &self,
        assistant_id: &str,
        name: Option<&str>,
        agent: Option<AssistantAgentInfo>,
        system_prompt: Option<Option<&str>>,
        color: Option<Option<&str>>,
        selected_skill_ids: Option<Vec<String>>,
        selected_mcp_ids: Option<Vec<String>>,
        enabled: Option<bool>,
    ) -> Result<AssistantInfo> {
        let conn = self.conn.lock().unwrap();
        assistants::update_assistant(
            &conn,
            assistant_id,
            name,
            agent,
            system_prompt,
            color,
            selected_skill_ids,
            selected_mcp_ids,
            enabled,
        )
    }

    fn delete_assistant(&self, assistant_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        assistants::delete_assistant(&conn, assistant_id)
    }

    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>> {
        let conn = self.conn.lock().unwrap();
        threads::list_threads(&conn, project_id)
    }

    fn list_thread_index(&self, project_id: Option<&str>) -> Result<Vec<ThreadIndexItemInfo>> {
        let conn = self.conn.lock().unwrap();
        thread_index::list_thread_index(&conn, project_id)
    }

    fn get_thread_work_state(&self, thread_id: &str) -> Result<ThreadInfo> {
        let conn = self.conn.lock().unwrap();
        load_thread_by_id(&conn, thread_id)
    }

    fn create_thread(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        threads::create_thread(&mut conn, project_id, goal, description)
    }

    fn create_thread_with_options(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
        kind: ThreadKind,
        assistant_ids: &[String],
        agent_participants: &[ThreadAgentInfo],
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        threads::create_thread_with_options(
            &mut conn,
            project_id,
            goal,
            description,
            kind,
            assistant_ids,
            agent_participants,
        )
    }

    fn create_thread_with_origin(
        &self,
        project_id: &str,
        goal: &str,
        description: Option<&str>,
        kind: ThreadKind,
        assistant_ids: &[String],
        agent_participants: &[ThreadAgentInfo],
        origin: ThreadOrigin,
        scheduled_task_id: Option<&str>,
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        threads::create_thread_with_origin(
            &mut conn,
            project_id,
            goal,
            description,
            kind,
            assistant_ids,
            agent_participants,
            origin,
            scheduled_task_id,
        )
    }

    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        threads::update_thread(&mut conn, thread_id, goal, description, enabled)
    }

    fn update_thread_with_options(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
        kind: Option<ThreadKind>,
        assistant_ids: Option<&[String]>,
        agent_participants: Option<&[ThreadAgentInfo]>,
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        threads::update_thread_with_options(
            &mut conn,
            thread_id,
            goal,
            description,
            enabled,
            kind,
            assistant_ids,
            agent_participants,
        )
    }

    fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        threads::delete_thread(&mut conn, thread_id)
    }

    fn create_plan_round(&self, round: NewPlanRound<'_>) -> Result<PlanRoundInfo> {
        let mut conn = self.conn.lock().unwrap();
        plans::create_plan_round(&mut conn, round)
    }

    fn get_plan_round(&self, round_id: &str) -> Result<Option<PlanRoundInfo>> {
        let conn = self.conn.lock().unwrap();
        plans::get_plan_round(&conn, round_id)
    }

    fn list_plan_rounds(&self, thread_id: &str) -> Result<Vec<PlanRoundInfo>> {
        let conn = self.conn.lock().unwrap();
        plans::list_plan_rounds(&conn, thread_id)
    }

    fn get_plan_task_thread_id(&self, task_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        plans::get_plan_task_thread_id(&conn, task_id)
    }

    fn update_plan_task_status(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanTaskInfo> {
        let mut conn = self.conn.lock().unwrap();
        plans::update_plan_task_status(&mut conn, task_id, patch)
    }

    fn complete_plan_task_and_start_next(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanRoundInfo> {
        let mut conn = self.conn.lock().unwrap();
        plans::complete_plan_task_and_start_next(&mut conn, task_id, patch)
    }

    fn link_plan_task_session(
        &self,
        session: NewPlanTaskSession<'_>,
    ) -> Result<PlanTaskSessionInfo> {
        let conn = self.conn.lock().unwrap();
        plans::link_plan_task_session(&conn, session)
    }

    fn relink_plan_task_session(
        &self,
        from: NewPlanTaskSession<'_>,
        to_session_id: &str,
        to_role: PlanTaskSessionRole,
    ) -> Result<PlanTaskSessionInfo> {
        let mut conn = self.conn.lock().unwrap();
        plans::relink_plan_task_session(&mut conn, from, to_session_id, to_role)
    }

    fn list_plan_task_sessions(&self, task_id: &str) -> Result<Vec<PlanTaskSessionInfo>> {
        let conn = self.conn.lock().unwrap();
        plans::list_plan_task_sessions(&conn, task_id)
    }

    fn list_project_stages(&self, project_id: &str) -> Result<Vec<ProjectStageInfo>> {
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
             FROM stages
             WHERE project_id = ?
             ORDER BY sort_order ASC, type ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![project_id], project_stage_from_row)?;
        let mut stages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for stage in stages.iter_mut() {
            stage.assistants = load_project_stage_assistants(&conn, &stage.id)?;
        }
        Ok(stages)
    }

    fn list_process_template_stages(
        &self,
        process_template_id: &str,
    ) -> Result<Vec<ProjectStageInfo>> {
        let conn = self.conn.lock().unwrap();
        ensure_process_template_exists(&conn, process_template_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
             FROM stages
             WHERE project_id IS NULL AND process_template_id = ?
             ORDER BY sort_order ASC, type ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![process_template_id], project_stage_from_row)?;
        let mut stages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for stage in stages.iter_mut() {
            stage.assistants = load_project_stage_assistants(&conn, &stage.id)?;
        }
        Ok(stages)
    }

    fn create_project_stage(
        &self,
        project_id: &str,
        process_template_id: Option<String>,
        name: &str,
        description: Option<&str>,
        icon: Option<&str>,
    ) -> Result<ProjectStageInfo> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("project stage name cannot be empty");
        }
        let description = description.map(str::trim).filter(|value| !value.is_empty());
        let icon = icon.map(str::trim).filter(|value| !value.is_empty());
        let conn = self.conn.lock().unwrap();
        let requested_process_template_id = process_template_id;
        let project = if requested_process_template_id.is_none() {
            Some(load_project_by_id(&conn, project_id)?)
        } else if project_id.trim().is_empty() {
            None
        } else {
            Some(load_project_by_id(&conn, project_id)?)
        };
        let resolved_process_template_id = requested_process_template_id
            .as_deref()
            .or_else(|| {
                project
                    .as_ref()
                    .map(|project| project.process_template_id.as_str())
            })
            .ok_or_else(|| {
                anyhow::anyhow!("project stage requires a project or process template")
            })?;
        ensure_process_template_exists(&conn, resolved_process_template_id)?;
        let template_project_id = if requested_process_template_id.is_some() {
            None
        } else {
            Some(project_id)
        };
        let next_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM stages
                 WHERE process_template_id = ?
                   AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)",
                params![
                    resolved_process_template_id,
                    template_project_id,
                    template_project_id
                ],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let now = now_ms();
        let id = stable_project_stage_id(
            Some(resolved_process_template_id),
            template_project_id,
            name,
            next_order,
            now,
        );
        conn.execute(
            "INSERT INTO stages (id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at)
             VALUES (?, ?, 'custom', ?, NULL, ?, ?, ?, ?, 1, 0, ?, ?)",
            params![
                id,
                template_project_id,
                resolved_process_template_id,
                name,
                description,
                icon,
                next_order,
                now,
                now
            ],
        )?;
        load_project_stage_by_id(&conn, &id)
    }

    fn update_project_stage(
        &self,
        stage_id: &str,
        patch: ProjectStagePatch<'_>,
    ) -> Result<ProjectStageInfo> {
        let ProjectStagePatch {
            name,
            description,
            icon,
            order,
            enabled,
            allow_empty_assistants,
        } = patch;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_project_stage_by_id(&tx, stage_id)?;
        if current.stage_type != ProjectStageType::Custom
            && (name.is_some() || description.is_some())
        {
            anyhow::bail!("builtin project stage details cannot be updated");
        }
        let Some(scope_process_template_id) = current.process_template_id else {
            anyhow::bail!("project stage requires a process template");
        };
        let scope_project_id = current.project_id.as_deref();
        let next_name = match name {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    anyhow::bail!("project stage name cannot be empty");
                }
                value.to_string()
            }
            None => current.name.unwrap_or_default(),
        };
        let next_description = match description {
            Some(Some(value)) => {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.trim().to_string())
                }
            }
            Some(None) => None,
            None => current.description,
        };
        let next_icon = match icon {
            Some(Some(value)) => {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.trim().to_string())
                }
            }
            Some(None) => None,
            None => current.icon,
        };
        let next_order = match order {
            Some(target_order) if target_order != current.order => reorder_project_stage_scope(
                &tx,
                stage_id,
                scope_process_template_id.as_str(),
                scope_project_id,
                target_order,
            )?,
            _ => current.order,
        };
        let next_enabled = enabled.unwrap_or(current.enabled);
        if current.enabled && !next_enabled {
            ensure_project_stage_can_be_disabled(&tx, stage_id)?;
        }
        let next_allow_empty_assistants =
            allow_empty_assistants.unwrap_or(current.allow_empty_assistants);
        let now = now_ms();
        if current.stage_type == ProjectStageType::Custom {
            tx.execute(
                "UPDATE stages SET name = ?, description = ?, icon = ?, sort_order = ?, enabled = ?, allow_empty_assistants = ?, updated_at = ? WHERE id = ?",
                params![
                    next_name,
                    next_description,
                    next_icon,
                    next_order,
                    next_enabled as i64,
                    next_allow_empty_assistants as i64,
                    now,
                    stage_id
                ],
            )?;
        } else {
            tx.execute(
                "UPDATE stages SET icon = ?, sort_order = ?, enabled = ?, allow_empty_assistants = ?, updated_at = ? WHERE id = ?",
                params![
                    next_icon,
                    next_order,
                    next_enabled as i64,
                    next_allow_empty_assistants as i64,
                    now,
                    stage_id
                ],
            )?;
        }
        let stage = load_project_stage_by_id(&tx, stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn update_project_stage_assistants(
        &self,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<ProjectStageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let stage = load_project_stage_by_id(&tx, stage_id)?;
        let assistants = validate_assistants_for_stage(&tx, &stage, assistant_ids)?;
        let now = now_ms();
        replace_project_stage_assistants(&tx, stage_id, &assistants, now)?;
        tx.execute(
            "UPDATE stages SET updated_at = ? WHERE id = ?",
            params![now, stage_id],
        )?;
        let stage = load_project_stage_by_id(&tx, stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn delete_project_stage(&self, stage_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let current = load_project_stage_by_id(&conn, stage_id)?;
        if current.stage_type != ProjectStageType::Custom {
            anyhow::bail!("builtin project stage cannot be deleted");
        }
        let changed = conn.execute("DELETE FROM stages WHERE id = ?", params![stage_id])?;
        if changed == 0 {
            anyhow::bail!("project stage not found: {stage_id}");
        }
        Ok(())
    }

    fn add_thread_stage(
        &self,
        thread_id: &str,
        stage_id: &str,
        assistant_ids: &[String],
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let thread = load_thread_by_id(&tx, thread_id)?;
        if !thread.enabled {
            anyhow::bail!("thread is disabled");
        }
        let project = load_project_by_id(&tx, &thread.project_id)?;
        let project_stage = load_project_stage_by_id(&tx, stage_id)?;
        if !project_stage.enabled {
            anyhow::bail!("project stage is disabled");
        }
        if project_stage.project_id.as_deref() != Some(thread.project_id.as_str())
            || project_stage.process_template_id.as_deref()
                != Some(project.process_template_id.as_str())
        {
            anyhow::bail!("project stage does not belong to this thread's project");
        }
        let default_assistant_ids = if assistant_ids.is_empty() {
            project_stage
                .assistants
                .iter()
                .map(|assistant| assistant.assistant_id.clone())
                .collect::<Vec<_>>()
        } else {
            assistant_ids.to_vec()
        };
        let assistant_bindings =
            validate_assistants_for_project(&tx, &thread.project_id, &default_assistant_ids)?
                .into_iter()
                .enumerate()
                .map(|(index, assistant)| stage_assistant_from_assistant(assistant, index as i64))
                .collect::<Vec<_>>();
        let assistant_ids = assistant_bindings
            .iter()
            .map(|assistant| assistant.assistant_id.clone())
            .collect::<Vec<_>>();
        if assistant_ids.is_empty() && !project_stage.allow_empty_assistants {
            anyhow::bail!("stage does not allow empty assistants");
        }
        let next_order: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM thread_stages WHERE thread_id = ?",
                params![thread_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let now = now_ms();
        let id = stable_thread_stage_id(thread_id, stage_id, &assistant_ids.join(","), next_order);
        tx.execute(
            "INSERT INTO thread_stages (id, thread_id, stage_id, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![id, thread_id, stage_id, next_order, now, now],
        )?;
        replace_thread_stage_assistants(&tx, &id, &assistant_bindings, now)?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, thread_id],
        )?;
        let stage = load_thread_stage_by_id(&tx, &id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn update_thread_stage(
        &self,
        thread_stage_id: &str,
        assistant_ids: Option<&[String]>,
        order: Option<i64>,
        enabled: Option<bool>,
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
        let next_assistant_bindings = match assistant_ids {
            Some(ids) => {
                let bindings = validate_assistants_for_project(&tx, &current.project_id, ids)?
                    .into_iter()
                    .enumerate()
                    .map(|(index, assistant)| {
                        stage_assistant_from_assistant(assistant, index as i64)
                    })
                    .collect::<Vec<_>>();
                if bindings.is_empty() && !current.allow_empty_assistants {
                    anyhow::bail!("stage does not allow empty assistants");
                }
                Some(bindings)
            }
            None => None,
        };
        let max_order: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(sort_order), 0) FROM thread_stages WHERE thread_id = ?",
                params![current.thread_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let next_order = order.unwrap_or(current.order).clamp(0, max_order);
        if next_order != current.order {
            let mut ids = {
                let mut stmt = tx.prepare(
                    "SELECT id
                     FROM thread_stages
                     WHERE thread_id = ?
                     ORDER BY sort_order ASC, created_at ASC",
                )?;
                let rows = stmt.query_map(params![current.thread_id], |row| row.get(0))?;
                rows.collect::<rusqlite::Result<Vec<String>>>()?
            };
            let Some(current_index) = ids.iter().position(|id| id == thread_stage_id) else {
                anyhow::bail!("thread stage not found in reorder scope: {thread_stage_id}");
            };
            let id = ids.remove(current_index);
            ids.insert(next_order as usize, id);
            for (index, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE thread_stages SET sort_order = ? WHERE id = ?",
                    params![-((index as i64) + 1), id],
                )?;
            }
            for (index, id) in ids.iter().enumerate() {
                tx.execute(
                    "UPDATE thread_stages SET sort_order = ? WHERE id = ?",
                    params![index as i64, id],
                )?;
            }
        }
        let now = now_ms();
        tx.execute(
            "UPDATE thread_stages
             SET sort_order = ?, updated_at = ?
             WHERE id = ?",
            params![next_order, now, thread_stage_id],
        )?;
        if let Some(next_assistant_bindings) = next_assistant_bindings {
            replace_thread_stage_assistants(&tx, thread_stage_id, &next_assistant_bindings, now)?;
        }
        if let Some(enabled) = enabled {
            if current.enabled && !enabled {
                ensure_project_stage_can_be_disabled(&tx, &current.stage_id)?;
            }
            tx.execute(
                "UPDATE stages SET enabled = ?, updated_at = ? WHERE id = ?",
                params![enabled as i64, now, current.stage_id],
            )?;
        }
        compact_stage_order(&tx, &current.thread_id)?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, current.thread_id],
        )?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn update_thread_stage_state(
        &self,
        thread_stage_id: &str,
        status: Option<StageStatus>,
        summary: Option<Option<String>>,
        outcome: Option<Option<String>>,
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Resolve the current effective stage (also validates existence and
        // applies the order-relative lazy default when no row exists yet).
        let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
        let next_status = status.unwrap_or(current.status);
        let next_summary = match summary {
            Some(value) => value,
            None => current.summary.clone(),
        };
        let next_outcome = match outcome {
            Some(value) => value,
            None => current.outcome.clone(),
        };
        let now = now_ms();
        tx.execute(
            "INSERT INTO thread_stage_states
                (thread_stage_id, status, summary, outcome, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(thread_stage_id) DO UPDATE SET
                status = excluded.status,
                summary = excluded.summary,
                outcome = excluded.outcome,
                updated_at = excluded.updated_at",
            params![
                thread_stage_id,
                next_status.as_str(),
                next_summary,
                next_outcome,
                now,
                now
            ],
        )?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, current.thread_id],
        )?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn list_thread_stage_issues(&self, thread_stage_id: &str) -> Result<Vec<StageIssueInfo>> {
        let conn = self.conn.lock().unwrap();
        load_stage_issues(&conn, thread_stage_id)
    }

    fn create_thread_stage_issue(
        &self,
        thread_stage_id: &str,
        title: &str,
        description: Option<&str>,
        severity: IssueSeverity,
    ) -> Result<StageIssueInfo> {
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("issue title cannot be empty");
        }
        let description = description.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().unwrap();
        // Validate the parent stage exists without loading its nested sessions/issues.
        let exists = conn
            .query_row(
                "SELECT 1 FROM thread_stages WHERE id = ?",
                params![thread_stage_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            anyhow::bail!("thread stage not found: {thread_stage_id}");
        }
        let now = now_ms();
        let id = stable_issue_id(thread_stage_id, title, now, &unique_nonce());
        conn.execute(
            "INSERT INTO thread_stage_issues (
                id, thread_stage_id, title, description, status, severity, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                thread_stage_id,
                title,
                description,
                IssueStatus::Open.as_str(),
                severity.as_str(),
                now,
                now,
            ],
        )?;
        load_stage_issue_by_id(&conn, &id)
    }

    fn update_thread_stage_issue(
        &self,
        issue_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<IssueStatus>,
        severity: Option<IssueSeverity>,
    ) -> Result<StageIssueInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_stage_issue_by_id(&conn, issue_id)?;
        let next_title = match title {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("issue title cannot be empty");
                }
                trimmed.to_string()
            }
            None => current.title,
        };
        let next_description = match description {
            Some(Some(value)) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            }
            Some(None) => None,
            None => current.description,
        };
        let next_status = status.unwrap_or(current.status);
        let next_severity = severity.unwrap_or(current.severity);
        conn.execute(
            "UPDATE thread_stage_issues
             SET title = ?, description = ?, status = ?, severity = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_title,
                next_description,
                next_status.as_str(),
                next_severity.as_str(),
                now_ms(),
                issue_id,
            ],
        )?;
        load_stage_issue_by_id(&conn, issue_id)
    }

    fn delete_thread_stage_issue(&self, issue_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "DELETE FROM thread_stage_issues WHERE id = ?",
            params![issue_id],
        )?;
        if changed == 0 {
            anyhow::bail!("issue not found: {issue_id}");
        }
        Ok(())
    }

    fn update_thread_stage_assistant_agent(
        &self,
        thread_stage_id: &str,
        assistant_id: &str,
        agent: AssistantAgentInfo,
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
        validate_assistant_for_project(&tx, &current.project_id, assistant_id)?;
        let exists: i64 = tx.query_row(
            "SELECT count(*) FROM thread_stage_assistants WHERE thread_stage_id = ? AND assistant_id = ?",
            params![thread_stage_id, assistant_id],
            |row| row.get(0),
        )?;
        if exists == 0 {
            anyhow::bail!("assistant is not linked to this thread stage");
        }
        let agent = normalize_assistant_agent(&tx, agent)?;
        let agent_json = serde_json::to_string(&agent)?;
        let now = now_ms();
        tx.execute(
            "UPDATE thread_stage_assistants
             SET agent_json = ?, updated_at = ?
             WHERE thread_stage_id = ? AND assistant_id = ?",
            params![agent_json, now, thread_stage_id, assistant_id],
        )?;
        tx.execute(
            "UPDATE thread_stages SET updated_at = ? WHERE id = ?",
            params![now, thread_stage_id],
        )?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, current.thread_id],
        )?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn delete_thread_stage(&self, thread_stage_id: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        let session_refs = {
            let mut stmt = tx.prepare(
                "SELECT agent, session_id
                 FROM stage_sessions
                 WHERE thread_stage_id = ?",
            )?;
            let refs = stmt
                .query_map(params![thread_stage_id], |row| {
                    let agent_str: String = row.get(0)?;
                    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                    Ok((agent, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            refs
        };
        tx.execute(
            "DELETE FROM thread_stages WHERE id = ?",
            params![thread_stage_id],
        )?;
        compact_stage_order(&tx, &stage.thread_id)?;
        let current_stage_id: Option<String> = tx.query_row(
            "SELECT stage_id FROM threads WHERE id = ?",
            params![stage.thread_id],
            |row| row.get(0),
        )?;
        let next_stage_id = if current_stage_id.as_deref() == Some(thread_stage_id) {
            next_thread_stage_id(&tx, &stage.thread_id)?
        } else {
            current_stage_id
        };
        tx.execute(
            "UPDATE threads SET stage_id = ?, updated_at = ? WHERE id = ?",
            params![next_stage_id, now_ms(), stage.thread_id],
        )?;
        for (agent, session_id) in &session_refs {
            downgrade_session_origin_when_unlinked(&tx, *agent, session_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn set_thread_stage(&self, thread_id: &str, thread_stage_id: &str) -> Result<ThreadInfo> {
        let conn = self.conn.lock().unwrap();
        let thread = load_thread_by_id(&conn, thread_id)?;
        if !thread.enabled {
            anyhow::bail!("thread is disabled");
        }
        let stage = load_thread_stage_by_id(&conn, thread_stage_id)?;
        if stage.thread_id != thread_id {
            anyhow::bail!("stage does not belong to this thread");
        }
        if !stage.enabled {
            anyhow::bail!("thread stage is disabled");
        }
        conn.execute(
            "UPDATE threads SET stage_id = ?, updated_at = ? WHERE id = ?",
            params![thread_stage_id, now_ms(), thread_id],
        )?;
        load_thread_by_id(&conn, thread_id)
    }

    fn link_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let thread = load_thread_by_id(&tx, thread_id)?;
        if !thread.enabled {
            anyhow::bail!("thread is disabled");
        }
        let project = load_project_by_id(&tx, &thread.project_id)?;
        let session_project_path = session_project_path(&tx, agent, session_id)?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        if session_project_path != project.path {
            anyhow::bail!("session does not belong to this thread's project");
        }
        ensure_session_not_linked_to_thread_process(&tx, agent, session_id)?;
        let now = now_ms();
        tx.execute(
            "INSERT OR IGNORE INTO thread_sessions (thread_id, agent, session_id, created_at)
             VALUES (?, ?, ?, ?)",
            params![thread_id, agent.as_str(), session_id, now],
        )?;
        // Upgrade the session's origin to `thread` so the sidebar filter
        // hides it (thread item represents these sessions). Sticky: only
        // `chat`-origin rows get upgraded, channel sessions stay channel.
        upgrade_session_origin_to_thread(&tx, agent, session_id)?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, thread_id],
        )?;
        let thread = load_thread_by_id(&tx, thread_id)?;
        tx.commit()?;
        Ok(thread)
    }

    fn unlink_thread_session(
        &self,
        thread_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<ThreadInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        load_thread_by_id(&tx, thread_id)?;
        tx.execute(
            "DELETE FROM thread_sessions
             WHERE thread_id = ? AND agent = ? AND session_id = ?",
            params![thread_id, agent.as_str(), session_id],
        )?;
        // If this session no longer appears in any thread / stage / plan / astra
        // link table, drop its sticky `origin = 'thread'` back to `'chat'`
        // so it returns to the sidebar.
        downgrade_session_origin_when_unlinked(&tx, agent, session_id)?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now_ms(), thread_id],
        )?;
        let thread = load_thread_by_id(&tx, thread_id)?;
        tx.commit()?;
        Ok(thread)
    }

    fn link_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        if !stage.enabled {
            anyhow::bail!("thread stage is disabled");
        }
        let thread = load_thread_by_id(&tx, &stage.thread_id)?;
        if !thread.enabled {
            anyhow::bail!("thread is disabled");
        }
        let project = load_project_by_id(&tx, &stage.project_id)?;
        let session_project_path = session_project_path(&tx, agent, session_id)?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        if session_project_path != project.path {
            anyhow::bail!("session does not belong to this stage's project");
        }
        ensure_session_not_linked_to_thread_process(&tx, agent, session_id)?;
        let now = now_ms();
        tx.execute(
            "INSERT OR IGNORE INTO stage_sessions (thread_stage_id, agent, session_id, created_at)
             VALUES (?, ?, ?, ?)",
            params![thread_stage_id, agent.as_str(), session_id, now],
        )?;
        upgrade_session_origin_to_thread(&tx, agent, session_id)?;
        tx.execute(
            "UPDATE thread_stages SET updated_at = ? WHERE id = ?",
            params![now, thread_stage_id],
        )?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, stage.thread_id],
        )?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn unlink_stage_session(
        &self,
        thread_stage_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        tx.execute(
            "DELETE FROM stage_sessions
             WHERE thread_stage_id = ? AND agent = ? AND session_id = ?",
            params![thread_stage_id, agent.as_str(), session_id],
        )?;
        // See downgrade_session_origin_when_unlinked: keep sticky `thread`
        // origin only while at least one link survives.
        downgrade_session_origin_when_unlinked(&tx, agent, session_id)?;
        let now = now_ms();
        tx.execute(
            "UPDATE thread_stages SET updated_at = ? WHERE id = ?",
            params![now, thread_stage_id],
        )?;
        tx.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            params![now, stage.thread_id],
        )?;
        let stage = load_thread_stage_by_id(&tx, thread_stage_id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn list_kanban_items(&self, project_id: &str) -> Result<Vec<KanbanItem>> {
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, title, description, status, sort_order, created_at, updated_at
             FROM kanban_items
             WHERE project_id = ?
             ORDER BY status, sort_order ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![project_id], kanban_item_from_row)?;
        let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        attach_kanban_item_sessions(&conn, &mut items)?;
        Ok(items)
    }

    fn create_kanban_item(
        &self,
        project_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<KanbanItem> {
        let title = title.trim();
        if title.is_empty() {
            anyhow::bail!("todo title cannot be empty");
        }
        let description = description.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let next_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1
                 FROM kanban_items
                 WHERE project_id = ? AND status = ?",
                params![project_id, KanbanStatus::Todo.as_str()],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let now = now_ms();
        let id = stable_kanban_id(project_id, title, now);
        conn.execute(
            "INSERT INTO kanban_items (
                id, project_id, title, description, status, sort_order, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                project_id,
                title,
                description,
                KanbanStatus::Todo.as_str(),
                next_order,
                now,
                now,
            ],
        )?;
        load_kanban_item_by_id(&conn, &id)
    }

    fn update_kanban_item(
        &self,
        item_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<KanbanStatus>,
    ) -> Result<KanbanItem> {
        let conn = self.conn.lock().unwrap();
        let current = load_kanban_item_by_id(&conn, item_id)?;
        let next_title = match title {
            Some(value) => {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    anyhow::bail!("todo title cannot be empty");
                }
                trimmed.to_string()
            }
            None => current.title,
        };
        let next_description = match description {
            Some(Some(value)) => {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.trim().to_string())
                }
            }
            Some(None) => None,
            None => current.description,
        };
        let next_status = status.unwrap_or(current.status);
        let next_order = if next_status != current.status {
            conn.query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1
                 FROM kanban_items
                 WHERE project_id = ? AND status = ?",
                params![current.project_id, next_status.as_str()],
                |row| row.get(0),
            )
            .unwrap_or(0)
        } else {
            current.sort_order
        };
        conn.execute(
            "UPDATE kanban_items
             SET title = ?, description = ?, status = ?, sort_order = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_title,
                next_description,
                next_status.as_str(),
                next_order,
                now_ms(),
                item_id,
            ],
        )?;
        load_kanban_item_by_id(&conn, item_id)
    }

    fn delete_kanban_item(&self, item_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute("DELETE FROM kanban_items WHERE id = ?", params![item_id])?;
        if changed == 0 {
            anyhow::bail!("kanban item not found: {item_id}");
        }
        Ok(())
    }

    fn link_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem> {
        let conn = self.conn.lock().unwrap();
        let item = load_kanban_item_by_id(&conn, item_id)?;
        let project = load_project_by_id(&conn, &item.project_id)?;
        let session_project_path = session_project_path(&conn, agent, session_id)?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        if session_project_path != project.path {
            anyhow::bail!("session does not belong to this project");
        }
        conn.execute(
            "INSERT OR IGNORE INTO kanban_item_sessions (item_id, agent, session_id, created_at)
             VALUES (?, ?, ?, ?)",
            params![item_id, agent.as_str(), session_id, now_ms()],
        )?;
        load_kanban_item_by_id(&conn, item_id)
    }

    fn unlink_kanban_item_session(
        &self,
        item_id: &str,
        agent: Agent,
        session_id: &str,
    ) -> Result<KanbanItem> {
        let conn = self.conn.lock().unwrap();
        load_kanban_item_by_id(&conn, item_id)?;
        conn.execute(
            "DELETE FROM kanban_item_sessions
             WHERE item_id = ? AND agent = ? AND session_id = ?",
            params![item_id, agent.as_str(), session_id],
        )?;
        load_kanban_item_by_id(&conn, item_id)
    }

    fn get_runtime_agent_capability(
        &self,
        agent: Agent,
    ) -> Result<Option<RuntimeAgentCapabilityRecord>> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::get_runtime_agent_capability(&conn, agent)
    }

    fn upsert_runtime_agent_capability(&self, record: &RuntimeAgentCapabilityRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::upsert_runtime_agent_capability(&conn, record)
    }

    fn get_runtime_agent_session_config(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<Option<RuntimeAgentSessionConfigRecord>> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::get_runtime_agent_session_config(&conn, agent, adapter_version)
    }

    fn list_runtime_agent_session_configs(
        &self,
        agent: Agent,
    ) -> Result<Vec<RuntimeAgentSessionConfigRecord>> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::list_runtime_agent_session_configs(&conn, agent)
    }

    fn mark_runtime_agent_session_config_needs_refresh(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::mark_runtime_agent_session_config_needs_refresh(
            &conn,
            agent,
            adapter_version,
        )
    }

    fn upsert_runtime_agent_session_config(
        &self,
        record: &RuntimeAgentSessionConfigRecord,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        runtime_agents::upsert_runtime_agent_session_config(&conn, record)
    }

    fn get_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Vec<SessionHistorySnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        snapshots::load_session_history_snapshots(&conn, child_agent, child_session_id)
    }

    fn replace_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
        snapshots: &[SessionHistorySnapshotRecord],
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        snapshots::replace_session_history_snapshots(
            &tx,
            child_agent,
            child_session_id,
            snapshots,
        )?;
        tx.commit()?;
        Ok(())
    }

    fn save_thread_work_snapshot(&self, snapshot: &ThreadWorkSnapshotRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        snapshots::save_thread_work_snapshot(&conn, snapshot)
    }

    fn get_thread_work_snapshot(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Option<ThreadWorkSnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        snapshots::get_thread_work_snapshot(&conn, child_agent, child_session_id)
    }

    fn replace_astra_run_sessions(
        &self,
        run_id: &str,
        sessions: &[AstraRunSessionRecord],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        astra::replace_astra_run_sessions(&conn, run_id, sessions)
    }

    fn list_astra_run_sessions(&self, run_id: &str) -> Result<Vec<AstraRunSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        astra::list_astra_run_sessions(&conn, run_id)
    }

    fn list_astra_run_sessions_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<Vec<AstraRunSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        astra::list_astra_run_sessions_for_thread(&conn, thread_id)
    }

    fn upsert_astra_run(&self, run: &AstraRunRecord) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        astra::upsert_astra_run(&mut conn, run)
    }

    fn get_astra_run(&self, run_id: &str) -> Result<Option<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        astra::get_astra_run(&conn, run_id)
    }

    fn get_active_astra_run(&self, thread_id: &str) -> Result<Option<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        astra::get_active_astra_run(&conn, thread_id)
    }

    fn list_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        astra::list_astra_runs(&conn, thread_id)
    }

    fn interrupt_active_astra_runs(&self) -> Result<Vec<AstraRunRecord>> {
        let mut conn = self.conn.lock().unwrap();
        astra::interrupt_active_astra_runs(&mut conn)
    }

    fn cleanup_partial_astra_sessions(&self, session_ids: &[String]) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        astra::cleanup_partial_astra_sessions(&mut conn, session_ids)
    }

    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        insert_session(&conn, scope, session)
    }

    fn mark_session_scheduled_task(
        &self,
        agent: Agent,
        session_id: &str,
        scheduled_task_id: &str,
        is_auxiliary: bool,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        identity::mark_session_scheduled_task(
            &conn,
            agent,
            session_id,
            scheduled_task_id,
            is_auxiliary,
        )
    }

    fn mark_session_origin(
        &self,
        agent: Agent,
        session_id: &str,
        origin: SessionOrigin,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        identity::mark_session_origin(&conn, agent, session_id, origin)
    }

    fn replace_by_scope(&self, scope: &str, agent: Agent, sessions: &[SessionInfo]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        identity::replace_by_scope(&mut conn, scope, agent, sessions)
    }

    fn mark_file_path_unavailable(&self, file_path: &str) -> Result<()> {
        if is_virtual_session_ref(file_path) {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions SET available = 0 WHERE file_path = ?",
            params![file_path],
        )?;
        Ok(())
    }

    fn upsert_subagent(
        &self,
        parent_agent: Agent,
        _parent_scope: &str,
        parent_session_id: &str,
        subagent: &SubagentInfo,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        upsert_subagent_inner(&conn, parent_agent, parent_session_id, subagent)
    }

    fn update_message_count(
        &self,
        agent: Agent,
        session_id: Option<&str>,
        file_path: &str,
        message_count: usize,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = if let Some(parent_session_id) = session_id {
            conn.execute(
                "UPDATE subagents
                 SET message_count = ?
                 WHERE parent_agent = ? AND parent_session_id = ? AND file_path = ?",
                params![
                    message_count as i64,
                    agent.as_str(),
                    parent_session_id,
                    file_path,
                ],
            )?
        } else {
            0
        };
        if changed == 0 {
            if let Some(session_id) = session_id {
                conn.execute(
                    "UPDATE sessions
                     SET message_count = ?
                     WHERE agent = ? AND session_id = ? AND file_path = ?",
                    params![message_count as i64, agent.as_str(), session_id, file_path],
                )?;
            } else {
                conn.execute(
                    "UPDATE sessions
                     SET message_count = ?
                     WHERE agent = ? AND file_path = ?",
                    params![message_count as i64, agent.as_str(), file_path],
                )?;
            }
        }
        Ok(())
    }

    fn mark_subagent_file_unavailable(&self, file_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE subagents SET available = 0 WHERE file_path = ?",
            params![file_path],
        )?;
        Ok(())
    }

    fn get_or_create_canvas_document(
        &self,
        session_id: &str,
        title: Option<&str>,
    ) -> Result<CanvasDocumentInfo> {
        let conn = self.conn.lock().unwrap();
        canvas::upsert_canvas_document_title(&conn, session_id, title)
    }

    fn get_canvas_document_state(&self, session_id: &str) -> Result<CanvasDocumentState> {
        let conn = self.conn.lock().unwrap();
        canvas::load_canvas_document_state(&conn, session_id)
    }

    fn save_canvas_draft(
        &self,
        session_id: &str,
        title: Option<&str>,
        draft_snapshot_path: &str,
        draft_snapshot_hash: &str,
    ) -> Result<CanvasDocumentInfo> {
        let conn = self.conn.lock().unwrap();
        canvas::save_canvas_draft(
            &conn,
            session_id,
            title,
            draft_snapshot_path,
            draft_snapshot_hash,
        )
    }

    fn save_canvas_revision(
        &self,
        session_id: &str,
        title: Option<&str>,
        snapshot_path: &str,
        snapshot_hash: &str,
        snapshot_size_bytes: i64,
        source: &str,
    ) -> Result<(CanvasDocumentInfo, CanvasRevisionInfo)> {
        let mut conn = self.conn.lock().unwrap();
        canvas::save_canvas_revision(
            &mut conn,
            session_id,
            title,
            snapshot_path,
            snapshot_hash,
            snapshot_size_bytes,
            source,
        )
    }

    fn prune_canvas_revisions(&self, session_id: &str, keep_latest: usize) -> Result<Vec<String>> {
        let mut conn = self.conn.lock().unwrap();
        canvas::prune_canvas_revisions(&mut conn, session_id, keep_latest)
    }

    fn replace_canvas_blocks(
        &self,
        session_id: &str,
        blocks: &[UpsertCanvasBlockRecord],
    ) -> Result<Vec<CanvasBlockRecord>> {
        let mut conn = self.conn.lock().unwrap();
        canvas::replace_canvas_blocks(&mut conn, session_id, blocks)
    }

    fn create_canvas_context_anchor(
        &self,
        session_id: &str,
        anchor_block_id: Option<&str>,
        selection_block_ids_json: &str,
        selection_element_ids_json: &str,
        turn_id: &str,
        summary: Option<&str>,
    ) -> Result<CanvasContextAnchor> {
        let conn = self.conn.lock().unwrap();
        canvas::create_canvas_context_anchor(
            &conn,
            session_id,
            anchor_block_id,
            selection_block_ids_json,
            selection_element_ids_json,
            turn_id,
            summary,
        )
    }

    fn mark_file_path_unindexable(&self, agent: Agent, file_path: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM subagents WHERE file_path = ?",
            params![file_path],
        )?;
        let file_size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
        let file_mtime = file_mtime_for(file_path);
        let session_id = format!("__unindexable__:{}", file_path);
        tx.execute(
            "INSERT OR REPLACE INTO sessions (
                agent, session_id, scope, file_path,
                project_path, project_name,
                started_at, updated_at,
                message_count, rename_title, title, first_user_message,
            file_size, file_mtime,
            partial, available, archived,
            last_indexed_at, forked_from_agent, forked_from_id
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?,?,?, ?,?, ?,?,?, ?,?,?)",
            params![
                agent.as_str(),
                session_id,
                file_path,
                file_path,
                Option::<String>::None,
                Option::<String>::None,
                Option::<i64>::None,
                file_mtime,
                0i64,
                Option::<String>::None,
                Option::<String>::None,
                Option::<String>::None,
                file_size as i64,
                file_mtime,
                0i64,
                0i64,
                0i64,
                now_ms(),
                Option::<String>::None,
                Option::<String>::None,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn mark_missing_scopes_unavailable(
        &self,
        agent: Agent,
        present: &HashSet<String>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let all_scopes: Vec<String> = {
            let mut stmt = tx.prepare("SELECT DISTINCT scope FROM sessions WHERE agent = ?")?;
            let rs = stmt.query_map(params![agent.as_str()], |r| r.get::<_, String>(0))?;
            let mut v = Vec::new();
            for r in rs {
                v.push(r?);
            }
            v
        };
        for scope in &all_scopes {
            if present.contains(scope) {
                continue;
            }
            tx.execute(
                "UPDATE sessions
                 SET available = 0
                 WHERE scope = ? AND agent = ?
                   AND NOT (file_size = 0 AND partial = 1)
                   AND NOT (scope LIKE 'astra://%' OR file_path LIKE 'astra://%')",
                params![scope, agent.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

impl MemoryStore for SqliteStore {
    fn upsert_record(&self, record: &MemoryRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO memory_records (
                record_id, project_key, canonical_hash, simhash,
                title, summary, body, kind, available, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                record.record_id,
                record.project_key,
                record.canonical_hash,
                record.simhash,
                record.title,
                record.summary,
                record.body,
                record.kind.as_db_str(),
                record.available as i64,
                record.updated_at,
            ],
        )?;
        Ok(())
    }

    fn upsert_memory_artifact(&self, artifact: &MemoryArtifact) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO memory_artifacts (
                record_id, backend, artifact_uri, content_hash, updated_at
            ) VALUES (?, ?, ?, ?, ?)",
            params![
                artifact.record_id,
                artifact.backend,
                artifact.artifact_uri,
                artifact.content_hash,
                artifact.updated_at,
            ],
        )?;
        Ok(())
    }

    fn artifact_for_record(
        &self,
        record_id: &str,
        backend: &str,
    ) -> Result<Option<MemoryArtifact>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT record_id, backend, artifact_uri, content_hash, updated_at
             FROM memory_artifacts
             WHERE record_id = ? AND backend = ?",
            params![record_id, backend],
            |row| {
                Ok(MemoryArtifact {
                    record_id: row.get(0)?,
                    backend: row.get(1)?,
                    artifact_uri: row.get(2)?,
                    content_hash: row.get(3)?,
                    updated_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    fn remove_memory_artifact(&self, record_id: &str, backend: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM memory_artifacts WHERE record_id = ? AND backend = ?",
            params![record_id, backend],
        )?;
        Ok(())
    }

    fn replace_record_sources(&self, record_id: &str, sources: &[MemorySource]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM memory_sources WHERE record_id = ?",
            params![record_id],
        )?;
        for source in sources {
            tx.execute(
                "INSERT OR REPLACE INTO memory_sources (
                    record_id, agent, session_id, file_path,
                    line_start, line_end, byte_start, byte_end
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    source.record_id,
                    source.agent,
                    source.session_id,
                    source.file_path,
                    opt_u64_to_i64(source.location.line_start),
                    opt_u64_to_i64(source.location.line_end),
                    opt_u64_to_i64(source.location.byte_start),
                    opt_u64_to_i64(source.location.byte_end),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn replace_record_continuation(
        &self,
        record_id: &str,
        continuation: Option<&RecordContinuation>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM record_continuations WHERE record_id = ?",
            params![record_id],
        )?;
        if let Some(continuation) = continuation {
            tx.execute(
                "INSERT INTO record_continuations (
                    record_id, project_key,
                    candidate_agent, candidate_session_id, candidate_file_path,
                    base_agent, base_session_id, base_file_path,
                    base_start_turn_index, base_start_line_start, base_start_byte_start,
                    base_end_turn_index, base_end_line_end, base_end_byte_end,
                    candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                    updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    continuation.record_id,
                    continuation.project_key,
                    continuation.candidate_agent,
                    continuation.candidate_session_id,
                    continuation.candidate_file_path,
                    continuation.base_agent,
                    continuation.base_session_id,
                    continuation.base_file_path,
                    continuation.base_start_turn_index as i64,
                    opt_u64_to_i64(continuation.base_start_line_start),
                    opt_u64_to_i64(continuation.base_start_byte_start),
                    continuation.base_end_turn_index as i64,
                    opt_u64_to_i64(continuation.base_end_line_end),
                    opt_u64_to_i64(continuation.base_end_byte_end),
                    continuation.candidate_trim_turn_start as i64,
                    opt_u64_to_i64(continuation.candidate_trim_line_start),
                    opt_u64_to_i64(continuation.candidate_trim_byte_start),
                    continuation.updated_at,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn list_records_for_source(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<Vec<MemoryRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.record_id, c.project_key, c.canonical_hash, c.simhash,
                    c.title, c.summary, c.body, c.kind, c.available, c.updated_at
             FROM memory_records c
             JOIN memory_sources s ON s.record_id = c.record_id
             WHERE s.agent = ? AND s.session_id = ? AND s.file_path = ?
             ORDER BY c.updated_at DESC",
        )?;
        let records = stmt
            .query_map(params![agent, session_id, file_path], |row| {
                Ok(MemoryRecord {
                    record_id: row.get(0)?,
                    project_key: row.get(1)?,
                    canonical_hash: row.get(2)?,
                    simhash: row.get(3)?,
                    title: row.get(4)?,
                    summary: row.get(5)?,
                    body: row.get(6)?,
                    kind: read_record_kind(row, 7)?,
                    available: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    fn mark_record_unavailable(&self, record_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memory_records SET available = 0 WHERE record_id = ?",
            params![record_id],
        )?;
        Ok(())
    }

    fn mark_source_records_unavailable(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memory_records
             SET available = 0
             WHERE record_id IN (
                SELECT record_id
                FROM memory_sources
                WHERE agent = ? AND session_id = ? AND file_path = ?
             )",
            params![agent, session_id, file_path],
        )?;
        Ok(())
    }

    fn list_project_records(&self, project_key: &str) -> Result<Vec<MemoryRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_id, project_key, canonical_hash, simhash,
                    title, summary, body, kind, available, updated_at
             FROM memory_records
             WHERE project_key = ?
             ORDER BY updated_at DESC",
        )?;
        let records = stmt
            .query_map(params![project_key], |row| {
                Ok(MemoryRecord {
                    record_id: row.get(0)?,
                    project_key: row.get(1)?,
                    canonical_hash: row.get(2)?,
                    simhash: row.get(3)?,
                    title: row.get(4)?,
                    summary: row.get(5)?,
                    body: row.get(6)?,
                    kind: read_record_kind(row, 7)?,
                    available: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    fn record_by_id(&self, record_id: &str) -> Result<Option<MemoryRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_id, project_key, canonical_hash, simhash,
                    title, summary, body, kind, available, updated_at
             FROM memory_records
             WHERE record_id = ?",
        )?;
        let record = stmt
            .query_row(params![record_id], |row| {
                Ok(MemoryRecord {
                    record_id: row.get(0)?,
                    project_key: row.get(1)?,
                    canonical_hash: row.get(2)?,
                    simhash: row.get(3)?,
                    title: row.get(4)?,
                    summary: row.get(5)?,
                    body: row.get(6)?,
                    kind: read_record_kind(row, 7)?,
                    available: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            })
            .optional()?;
        Ok(record)
    }

    fn sources_for_record(&self, record_id: &str) -> Result<Vec<MemorySource>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_id, agent, session_id, file_path,
                    line_start, line_end, byte_start, byte_end
             FROM memory_sources
             WHERE record_id = ?
             ORDER BY agent ASC, session_id ASC, line_start ASC",
        )?;
        let sources = stmt
            .query_map(params![record_id], |row| {
                let file_path: String = row.get(3)?;
                Ok(MemorySource {
                    record_id: row.get(0)?,
                    agent: row.get(1)?,
                    session_id: row.get(2)?,
                    file_path: file_path.clone(),
                    location: SourceLocation {
                        file_path,
                        line_start: opt_i64_to_u64(row.get(4)?),
                        line_end: opt_i64_to_u64(row.get(5)?),
                        byte_start: opt_i64_to_u64(row.get(6)?),
                        byte_end: opt_i64_to_u64(row.get(7)?),
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(sources)
    }

    fn continuation_for_record(&self, record_id: &str) -> Result<Option<RecordContinuation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_id, project_key,
                    candidate_agent, candidate_session_id, candidate_file_path,
                    base_agent, base_session_id, base_file_path,
                    base_start_turn_index, base_start_line_start, base_start_byte_start,
                    base_end_turn_index, base_end_line_end, base_end_byte_end,
                    candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                    updated_at
             FROM record_continuations
             WHERE record_id = ?",
        )?;
        let continuation = stmt
            .query_row(params![record_id], record_continuation_from_row)
            .optional()?;
        Ok(continuation)
    }

    fn continuations_for_base(
        &self,
        base_agent: &str,
        base_session_id: &str,
    ) -> Result<Vec<RecordContinuation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT record_id, project_key,
                    candidate_agent, candidate_session_id, candidate_file_path,
                    base_agent, base_session_id, base_file_path,
                    base_start_turn_index, base_start_line_start, base_start_byte_start,
                    base_end_turn_index, base_end_line_end, base_end_byte_end,
                    candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                    updated_at
             FROM record_continuations
             WHERE base_agent = ? AND base_session_id = ?
             ORDER BY updated_at DESC, candidate_session_id ASC, record_id ASC",
        )?;
        let rows = stmt.query_map(params![base_agent, base_session_id], |row| {
            record_continuation_from_row(row)
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn invalidate_continuations_referencing_base(
        &self,
        base_agent: &str,
        base_session_id: &str,
    ) -> Result<Vec<RecordContinuation>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let affected: Vec<RecordContinuation> = {
            let mut stmt = tx.prepare(
                "SELECT record_id, project_key,
                        candidate_agent, candidate_session_id, candidate_file_path,
                        base_agent, base_session_id, base_file_path,
                        base_start_turn_index, base_start_line_start, base_start_byte_start,
                        base_end_turn_index, base_end_line_end, base_end_byte_end,
                        candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                        updated_at
                 FROM record_continuations
                 WHERE base_agent = ? AND base_session_id = ?",
            )?;
            let rows = stmt.query_map(params![base_agent, base_session_id], |row| {
                record_continuation_from_row(row)
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.execute(
            "DELETE FROM record_continuations
             WHERE base_agent = ? AND base_session_id = ?",
            params![base_agent, base_session_id],
        )?;
        tx.commit()?;
        Ok(affected)
    }

    fn replace_turn_fingerprints(
        &self,
        project_key: &str,
        agent: &str,
        session_id: &str,
        fingerprints: &[TurnFingerprint],
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM turn_fingerprints
             WHERE project_key = ? AND agent = ? AND session_id = ?",
            params![project_key, agent, session_id],
        )?;
        for fp in fingerprints {
            tx.execute(
                "INSERT OR REPLACE INTO turn_fingerprints (
                    project_key, agent, session_id, turn_index, role,
                    canonical_hash, file_path,
                    text_len, line_start, line_end, byte_start, byte_end
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    fp.project_key,
                    fp.agent,
                    fp.session_id,
                    fp.turn_index as i64,
                    fp.role,
                    fp.canonical_hash,
                    fp.location.file_path,
                    fp.text_len as i64,
                    opt_u64_to_i64(fp.location.line_start),
                    opt_u64_to_i64(fp.location.line_end),
                    opt_u64_to_i64(fp.location.byte_start),
                    opt_u64_to_i64(fp.location.byte_end),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn list_turn_fingerprints(
        &self,
        project_key: &str,
        agent: &str,
        session_id: &str,
    ) -> Result<Vec<TurnFingerprint>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT project_key, agent, session_id, turn_index, role,
                    canonical_hash, file_path, text_len,
                    line_start, line_end, byte_start, byte_end
             FROM turn_fingerprints
             WHERE project_key = ? AND agent = ? AND session_id = ?
             ORDER BY turn_index ASC",
        )?;
        let rows = stmt.query_map(params![project_key, agent, session_id], |row| {
            let file_path: String = row.get(6)?;
            Ok(TurnFingerprint {
                project_key: row.get(0)?,
                agent: row.get(1)?,
                session_id: row.get(2)?,
                turn_index: row.get::<_, i64>(3)? as usize,
                role: row.get(4)?,
                canonical_hash: row.get(5)?,
                text_len: row.get::<_, i64>(7)? as usize,
                location: SourceLocation {
                    file_path,
                    line_start: opt_i64_to_u64(row.get(8)?),
                    line_end: opt_i64_to_u64(row.get(9)?),
                    byte_start: opt_i64_to_u64(row.get(10)?),
                    byte_end: opt_i64_to_u64(row.get(11)?),
                },
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn find_turn_fingerprint_candidates(
        &self,
        project_key: &str,
        exclude_agent: &str,
        exclude_session_id: &str,
        canonical_hashes: &[&str],
        limit: usize,
    ) -> Result<Vec<TurnFingerprintCandidate>> {
        if canonical_hashes.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn.lock().unwrap();
        let placeholders = canonical_hashes
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT agent, session_id, file_path, COUNT(*) AS shared_hashes
             FROM turn_fingerprints
             WHERE project_key = ?
               AND NOT (agent = ? AND session_id = ?)
               AND canonical_hash IN ({})
             GROUP BY agent, session_id, file_path
             ORDER BY shared_hashes DESC, session_id ASC
             LIMIT {}",
            placeholders, limit
        );
        let mut params: Vec<&dyn ToSql> = Vec::with_capacity(3 + canonical_hashes.len());
        params.push(&project_key);
        params.push(&exclude_agent);
        params.push(&exclude_session_id);
        for hash in canonical_hashes {
            params.push(hash);
        }
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params), |row| {
            Ok(TurnFingerprintCandidate {
                agent: row.get(0)?,
                session_id: row.get(1)?,
                file_path: row.get(2)?,
                shared_hashes: row.get::<_, i64>(3)? as usize,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn session_time_info(&self, agent: &str, session_id: &str) -> Result<Option<SessionTimeInfo>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT started_at, updated_at
             FROM sessions
             WHERE agent = ? AND session_id = ?
             LIMIT 1",
            params![agent, session_id],
            |row| {
                Ok(SessionTimeInfo {
                    started_at: row.get(0)?,
                    updated_at: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    fn record_memory_job(
        &self,
        project_key: &str,
        backend: &str,
        scope: &str,
        kind: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_jobs (
                project_key, backend, scope, kind, status, error, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![project_key, backend, scope, kind, status, error, now, now],
        )?;
        Ok(())
    }

    fn list_memory_jobs(&self, project_key: &str, status: Option<&str>) -> Result<Vec<MemoryJob>> {
        let conn = self.conn.lock().unwrap();
        let mut jobs = Vec::new();
        if let Some(status) = status {
            let mut stmt = conn.prepare(
                "SELECT id, project_key, backend, scope, kind, status, error, created_at, updated_at
                 FROM memory_jobs
                 WHERE project_key = ? AND backend = ? AND status = ?
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![project_key, "qmd", status], memory_job_from_row)?;
            for row in rows {
                jobs.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, project_key, backend, scope, kind, status, error, created_at, updated_at
                 FROM memory_jobs
                 WHERE project_key = ? AND backend = ?
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![project_key, "qmd"], memory_job_from_row)?;
            for row in rows {
                jobs.push(row?);
            }
        }
        Ok(jobs)
    }
}

fn memory_job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryJob> {
    Ok(MemoryJob {
        id: row.get(0)?,
        project_key: row.get(1)?,
        backend: row.get(2)?,
        scope: row.get(3)?,
        kind: row.get(4)?,
        status: row.get(5)?,
        error: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn record_continuation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RecordContinuation> {
    Ok(RecordContinuation {
        record_id: row.get(0)?,
        project_key: row.get(1)?,
        candidate_agent: row.get(2)?,
        candidate_session_id: row.get(3)?,
        candidate_file_path: row.get(4)?,
        base_agent: row.get(5)?,
        base_session_id: row.get(6)?,
        base_file_path: row.get(7)?,
        base_start_turn_index: row.get::<_, i64>(8)? as usize,
        base_start_line_start: opt_i64_to_u64(row.get(9)?),
        base_start_byte_start: opt_i64_to_u64(row.get(10)?),
        base_end_turn_index: row.get::<_, i64>(11)? as usize,
        base_end_line_end: opt_i64_to_u64(row.get(12)?),
        base_end_byte_end: opt_i64_to_u64(row.get(13)?),
        candidate_trim_turn_start: row.get::<_, i64>(14)? as usize,
        candidate_trim_line_start: opt_i64_to_u64(row.get(15)?),
        candidate_trim_byte_start: opt_i64_to_u64(row.get(16)?),
        updated_at: row.get(17)?,
    })
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use crate::models::{PlanRoundSource, PlanTaskRisk, ThreadReplaySessionSourceKind};

    fn unique_db(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}.db", unique_suffix()))
    }

    fn test_session(project: &ProjectInfo, id: &str, title: &str) -> SessionInfo {
        SessionInfo {
            id: id.to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 1,
            rename_title: None,
            title: Some(title.to_string()),
            first_user_message: Some(format!("{title} prompt")),
            file_path: Path::new(&project.path)
                .join(format!("{id}.jsonl"))
                .to_string_lossy()
                .to_string(),
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

    fn visible_session_ids(store: &SqliteStore) -> Vec<String> {
        let mut ids = store
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn assert_current_astra_run_columns(conn: &Connection) {
        let astra_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(astra_runs)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        for column in [
            "run_id",
            "thread_id",
            "project_id",
            "project_path",
            "status",
            "mode",
            "planner_backend",
            "round_index",
            "round_limit",
            "terminal_reason",
            "last_error_code",
            "last_error_message",
            "run_diagnostics_json",
            "error",
            "created_at",
            "updated_at",
        ] {
            assert!(
                astra_columns.contains(&column.to_string()),
                "{column} should exist"
            );
        }
        assert!(
            !astra_columns.contains(&"internal_planner_session_ids_json".to_string()),
            "internal_planner_session_ids_json should not exist"
        );
        for legacy_column in [
            "proposed_tasks_json",
            "approved_task_ids_json",
            "delegated_session_ids_json",
            "task_results_json",
            "current_stage_id",
            "completed_task_ids_json",
            "stage_attempt_counts_json",
            "retry_limit",
            "decision_backend",
            "internal_decision_session_ids_json",
        ] {
            assert!(
                !astra_columns.contains(&legacy_column.to_string()),
                "{legacy_column} should not exist"
            );
        }
    }

    fn assert_current_plan_task_session_columns(conn: &Connection) {
        let columns: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(thread_plan_task_sessions)")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        for column in [
            "task_id",
            "agent",
            "session_id",
            "role",
            "attempt_id",
            "attempt_count",
            "superseded_at",
            "created_at",
            "updated_at",
        ] {
            assert!(
                columns.contains(&column.to_string()),
                "{column} should exist"
            );
        }
    }

    fn assert_current_scheduled_task_schema(conn: &Connection) {
        for table in ["scheduled_tasks", "scheduled_task_runs"] {
            let exists: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "{table} table should exist");
        }

        let task_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(scheduled_tasks)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        for column in [
            "id",
            "name",
            "status",
            "schedule_json",
            "target_json",
            "project_id",
            "mode",
            "sort_order",
            "created_at_ms",
            "updated_at_ms",
            "last_run_at_ms",
        ] {
            assert!(
                task_columns.contains(&column.to_string()),
                "scheduled_tasks.{column} should exist"
            );
        }

        let run_columns: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(scheduled_task_runs)")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        for column in [
            "id",
            "task_id",
            "mode",
            "status",
            "started_at_ms",
            "completed_at_ms",
            "task_name",
            "target_json",
            "session_agent",
            "session_id",
            "thread_id",
            "astra_run_id",
            "push_platform",
            "push_chat_id",
            "push_status",
            "push_summary",
            "push_error",
            "push_sent_at_ms",
            "error",
        ] {
            assert!(
                run_columns.contains(&column.to_string()),
                "scheduled_task_runs.{column} should exist"
            );
        }

        for index in [
            "idx_scheduled_tasks_status_order",
            "idx_scheduled_tasks_project",
            "idx_scheduled_task_runs_task_started",
            "idx_scheduled_task_runs_session",
            "idx_scheduled_task_runs_thread",
            "idx_scheduled_task_runs_status_push",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?",
                    params![index],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "{index} index should exist");
        }
    }

    fn assert_current_process_template_schema(conn: &Connection) {
        assert_current_scheduled_task_schema(conn);

        let process_templates: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='process_templates'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(process_templates, 1);

        let workflows: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='workflows'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(workflows, 0);

        for (table, expected_column, legacy_column) in [
            ("projects", "process_template_id", "workflow_id"),
            ("assistants", "process_template_id", "workflow_id"),
            ("stages", "process_template_id", "workflow_id"),
        ] {
            let columns: Vec<String> = {
                let mut stmt = conn
                    .prepare(&format!("PRAGMA table_info({table})"))
                    .unwrap();
                let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
                rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
            };
            assert!(
                columns.contains(&expected_column.to_string()),
                "{table}.{expected_column} should exist"
            );
            assert!(
                !columns.contains(&legacy_column.to_string()),
                "{table}.{legacy_column} should not exist"
            );
        }

        let schema_sql: String = {
            let mut stmt = conn
                .prepare(
                    "SELECT COALESCE(group_concat(sql, '\n'), '')
                     FROM sqlite_master
                     WHERE sql IS NOT NULL",
                )
                .unwrap();
            stmt.query_row([], |row| row.get(0)).unwrap()
        };
        assert!(
            !schema_sql.contains("workflows"),
            "schema should not contain legacy workflows table references"
        );
        assert!(
            !schema_sql.contains("workflow_id"),
            "schema should not contain legacy workflow_id columns"
        );
        assert!(
            !schema_sql.contains("'workflow'"),
            "schema should not contain legacy workflow thread kind values"
        );
        assert!(
            schema_sql.contains("process_templates"),
            "schema should contain process_templates"
        );
        assert!(
            schema_sql.contains("process_template_id"),
            "schema should contain process_template_id"
        );
        assert!(
            schema_sql.contains("'process'"),
            "schema should contain process thread kind values"
        );
    }

    #[test]
    fn fresh_install_creates_current_schema() {
        let path = unique_db("sessio-schema-fresh");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let conn = store.conn.lock().unwrap();
        assert_current_process_template_schema(&conn);

        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(memory_records)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(columns.contains(&"record_id".to_string()));
        assert!(columns.contains(&"kind".to_string()));
        assert!(!columns.contains(&"qmd_path".to_string()));

        let session_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(sessions)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(session_columns.contains(&"forked_from_agent".to_string()));
        assert!(session_columns.contains(&"forked_from_id".to_string()));
        assert!(session_columns.contains(&"rename_title".to_string()));
        assert!(session_columns.contains(&"title".to_string()));

        let thread_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(threads)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(thread_columns.contains(&"kind".to_string()));

        let artifact_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_artifacts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artifact_table, 1);

        let job_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(memory_jobs)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(job_columns.contains(&"backend".to_string()));

        for removed_table in ["session_history", "session_history_turns"] {
            let exists: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                    params![removed_table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 0, "{removed_table} should not exist");
        }

        let snapshot_columns: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(session_history_snapshots)")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(snapshot_columns.contains(&"history_cache_version".to_string()));

        assert_current_astra_run_columns(&conn);
        assert_current_plan_task_session_columns(&conn);

        for table in [
            "agents",
            "assistants",
            "threads",
            "stages",
            "thread_stages",
            "thread_assistants",
            "thread_plan_rounds",
            "thread_plan_tasks",
            "thread_plan_task_sessions",
            "astra_run_sessions",
            "stage_sessions",
            "thread_stage_issues",
            "astra_runs",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?",
                    params![table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "{table} table should exist");
        }

        for index in [
            "idx_thread_plan_rounds_thread_index",
            "idx_thread_plan_rounds_thread_status",
            "idx_thread_plan_rounds_astra_run",
            "idx_thread_plan_tasks_round_order",
            "idx_thread_plan_tasks_round_status",
            "idx_thread_plan_tasks_stage",
            "idx_thread_plan_tasks_assistant",
            "idx_thread_plan_task_sessions_task",
            "idx_thread_plan_task_sessions_session",
            "idx_thread_plan_task_sessions_attempt",
            "idx_astra_run_sessions_run",
            "idx_astra_run_sessions_session",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='index' AND name=?",
                    params![index],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "{index} index should exist");
        }

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn deleting_thread_stage_restores_sidebar_session_visibility() {
        let path = unique_db("sessio-stage-delete-origin");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-stage-delete-origin-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "stage-delete-origin",
                "code".to_string(),
                None,
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Stage delete origin", None)
            .unwrap();
        let stage_template = store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .find(|stage| stage.allow_empty_assistants)
            .unwrap();
        let stage = store
            .add_thread_stage(&thread.id, &stage_template.id, &[])
            .unwrap();
        let session = test_session(&project, "stage-delete-session", "Stage delete");
        store.upsert_session(&session.file_path, &session).unwrap();
        assert_eq!(store.list_sessions().unwrap().len(), 1);

        store
            .link_stage_session(&stage.id, Agent::Codex, &session.id)
            .unwrap();
        assert!(
            store.list_sessions().unwrap().is_empty(),
            "stage-linked sessions should be hidden from the ordinary sidebar"
        );

        store.delete_thread_stage(&stage.id).unwrap();
        let visible = store.list_sessions().unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, session.id);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn astra_run_persistence_and_recovery() {
        let path = unique_db("astra-runs");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let project_path = std::env::temp_dir().join(format!("astra-project-{}", unique_suffix()));
        std::fs::create_dir_all(&project_path).unwrap();
        let project = store
            .add_project(
                &project_path.to_string_lossy(),
                Some("Astra Project"),
                "code".to_string(),
                None,
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Coordinate Astra", None)
            .unwrap();

        let run = AstraRunRecord {
            run_id: "astra-run-1".to_string(),
            thread_id: thread.id.clone(),
            project_id: project.id.clone(),
            project_path: project.path.clone(),
            status: "running".to_string(),
            mode: "auto".to_string(),
            planner_backend: Some("deterministic".to_string()),
            round_index: Some(0),
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_sessions: vec![AstraRunSessionRecord {
                run_id: "astra-run-1".to_string(),
                agent: Agent::Pi,
                session_id: "planner-session-1".to_string(),
                role: PlanTaskSessionRole::Planner,
                sort_order: 0,
                created_at: 10,
                updated_at: 20,
            }],
            run_diagnostics_json: r#"[{"kind":"planner_failure","code":"timeout"}]"#.to_string(),
            error: None,
            created_at: 10,
            updated_at: 20,
        };
        store.upsert_astra_run(&run).unwrap();
        let active = store.get_active_astra_run(&thread.id).unwrap().unwrap();
        assert_eq!(active.run_id, "astra-run-1");
        assert_eq!(active.status, "running");
        assert_eq!(
            active
                .internal_planner_sessions
                .iter()
                .map(|session| session.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["planner-session-1"]
        );
        assert_eq!(
            active.run_diagnostics_json,
            r#"[{"kind":"planner_failure","code":"timeout"}]"#
        );

        let interrupted_rows = store.interrupt_active_astra_runs().unwrap();
        assert_eq!(interrupted_rows.len(), 1);
        assert_eq!(interrupted_rows[0].run_id, "astra-run-1");
        assert_eq!(interrupted_rows[0].status, "interrupted");
        assert_eq!(
            interrupted_rows[0].terminal_reason.as_deref(),
            Some("process_recovered_active_run")
        );
        assert_eq!(
            interrupted_rows[0].last_error_code.as_deref(),
            Some("worker_interrupted")
        );
        assert!(store.get_active_astra_run(&thread.id).unwrap().is_none());
        let interrupted = store.get_astra_run("astra-run-1").unwrap().unwrap();
        assert_eq!(interrupted.status, "interrupted");

        let pending_review = AstraRunRecord {
            run_id: "astra-run-pending-review".to_string(),
            status: "completed".to_string(),
            terminal_reason: Some("pending_human_review".to_string()),
            error: None,
            created_at: 30,
            updated_at: 40,
            ..run.clone()
        };
        store.upsert_astra_run(&pending_review).unwrap();
        let persisted_review = store
            .get_astra_run("astra-run-pending-review")
            .unwrap()
            .unwrap();
        assert_eq!(persisted_review.status, "completed");
        assert_eq!(
            persisted_review.terminal_reason.as_deref(),
            Some("pending_human_review")
        );
        let runs = store.list_astra_runs(&thread.id).unwrap();
        assert!(runs.iter().any(|run| {
            run.run_id == "astra-run-pending-review"
                && run.terminal_reason.as_deref() == Some("pending_human_review")
        }));

        // A second pass has nothing active left to interrupt.
        assert!(store.interrupt_active_astra_runs().unwrap().is_empty());

        let runs = store.list_astra_runs(&thread.id).unwrap();
        assert_eq!(runs.len(), 2);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_path);
    }

    #[test]
    fn startup_recovery_closes_thinking_run_tasks_and_placeholders() {
        let path = unique_db("astra-recovery-task-sessions");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let project_path = std::env::temp_dir().join(format!(
            "astra-recovery-task-sessions-project-{}",
            unique_suffix()
        ));
        std::fs::create_dir_all(&project_path).unwrap();
        let project = store
            .add_project(
                &project_path.to_string_lossy(),
                Some("Astra Recovery Project"),
                "code".to_string(),
                None,
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Recover Astra thinking run", None)
            .unwrap();
        let run = AstraRunRecord {
            run_id: "astra-run-thinking".to_string(),
            thread_id: thread.id.clone(),
            project_id: project.id.clone(),
            project_path: project.path.clone(),
            status: "thinking".to_string(),
            mode: "auto".to_string(),
            planner_backend: Some("deterministic".to_string()),
            round_index: Some(0),
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_sessions: vec![AstraRunSessionRecord {
                run_id: "astra-run-thinking".to_string(),
                agent: Agent::Pi,
                session_id: "planner-placeholder".to_string(),
                role: PlanTaskSessionRole::Planner,
                sort_order: 0,
                created_at: 10,
                updated_at: 20,
            }],
            run_diagnostics_json: "[]".to_string(),
            error: None,
            created_at: 10,
            updated_at: 20,
        };
        store.upsert_astra_run(&run).unwrap();
        assert_eq!(
            store
                .get_active_astra_run(&thread.id)
                .unwrap()
                .unwrap()
                .status,
            "thinking"
        );
        let round = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: Some(&run.run_id),
                round_index: Some(0),
                summary: Some("recovery round"),
                mode: PlanRoundMode::Parallel,
                source: PlanRoundSource::Astra,
                status: PlanRoundStatus::Running,
                tasks: vec![NewPlanTask {
                    thread_stage_id: None,
                    assistant_id: None,
                    agent_participant_id: None,
                    target_agent: Agent::Codex,
                    stage_snapshot_json: None,
                    assistant_snapshot_json: None,
                    agent_snapshot_json: r#"{"agent":"codex"}"#,
                    title: "Recover running task",
                    prompt: "This task was running when the app quit.",
                    expected_output: None,
                    risk: PlanTaskRisk::Low,
                    sort_order: 0,
                    status: PlanTaskStatus::Running,
                }],
            })
            .unwrap();
        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: "delegated-placeholder",
                role: PlanTaskSessionRole::Delegated,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();

        let placeholder = SessionInfo {
            agent: Agent::Codex,
            id: "delegated-placeholder".to_string(),
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(1),
            updated_at: Some(1),
            message_count: 0,
            rename_title: Some("Astra delegated placeholder".to_string()),
            title: None,
            first_user_message: Some("placeholder".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            forked_from_agent: None,
            forked_from_id: None,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        let planner_placeholder = SessionInfo {
            id: "planner-placeholder".to_string(),
            agent: Agent::Pi,
            rename_title: Some("Astra planner placeholder".to_string()),
            ..placeholder.clone()
        };
        let real = SessionInfo {
            id: "real-session".to_string(),
            file_path: project_path
                .join("real-session.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 42,
            partial: false,
            ..placeholder.clone()
        };
        store.upsert_session("", &placeholder).unwrap();
        store.upsert_session("", &planner_placeholder).unwrap();
        store.upsert_session(&real.file_path, &real).unwrap();

        let interrupted_rows = store.interrupt_active_astra_runs().unwrap();
        assert_eq!(interrupted_rows.len(), 1);
        assert_eq!(interrupted_rows[0].status, "interrupted");
        assert!(store.get_active_astra_run(&thread.id).unwrap().is_none());

        let recovered_round = store.get_plan_round(&round.id).unwrap().unwrap();
        assert_eq!(recovered_round.status, PlanRoundStatus::Errored);
        assert_eq!(recovered_round.tasks[0].status, PlanTaskStatus::Errored);
        assert_eq!(
            recovered_round.tasks[0].error.as_deref(),
            Some("Astra task was active during startup recovery")
        );

        let sessions = store.list_all_sessions().unwrap();
        let delegated = sessions
            .iter()
            .find(|session| session.id == "delegated-placeholder")
            .unwrap();
        let planner = sessions
            .iter()
            .find(|session| session.id == "planner-placeholder")
            .unwrap();
        let real = sessions
            .iter()
            .find(|session| session.id == "real-session")
            .unwrap();
        assert!(!delegated.available);
        assert!(delegated.archived);
        assert!(!planner.available);
        assert!(planner.archived);
        assert!(real.available);
        assert!(!real.archived);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_path);
    }

    #[test]
    fn cleanup_partial_astra_sessions_only_archives_placeholders() {
        let path = unique_db("astra-partial-cleanup");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let project_path =
            std::env::temp_dir().join(format!("astra-partial-cleanup-project-{}", unique_suffix()));
        std::fs::create_dir_all(&project_path).unwrap();

        let placeholder = SessionInfo {
            agent: Agent::Codex,
            id: "placeholder-session".to_string(),
            project_path: Some(project_path.to_string_lossy().to_string()),
            project_name: Some("Astra Partial Cleanup".to_string()),
            started_at: Some(1),
            updated_at: Some(1),
            message_count: 0,
            rename_title: Some("Astra placeholder".to_string()),
            title: None,
            first_user_message: Some("placeholder".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            forked_from_agent: None,
            forked_from_id: None,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        let mut real = placeholder.clone();
        real.id = "real-session".to_string();
        real.file_size = 42;
        real.partial = false;
        store.upsert_session("", &placeholder).unwrap();
        store.upsert_session("", &real).unwrap();

        let changed = store
            .cleanup_partial_astra_sessions(&[
                "placeholder-session".to_string(),
                "real-session".to_string(),
            ])
            .unwrap();

        assert_eq!(changed, 1);
        let sessions = store.list_all_sessions().unwrap();
        let placeholder = sessions
            .iter()
            .find(|session| session.id == "placeholder-session")
            .unwrap();
        let real = sessions
            .iter()
            .find(|session| session.id == "real-session")
            .unwrap();
        assert!(!placeholder.available);
        assert!(placeholder.archived);
        assert!(real.available);
        assert!(!real.archived);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_path);
    }

    #[test]
    fn session_history_snapshot_roundtrip_stores_versioned_turns() {
        let path = unique_db("sessio-history-snapshot-roundtrip");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let turn = SessionHistoryTurn {
            turn_id: "turn-parent".to_string(),
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
        let snapshot = SessionHistorySnapshotRecord {
            child_agent: Agent::Pi,
            child_session_id: "child".to_string(),
            ancestor_agent: Agent::Codex,
            ancestor_session_id: "parent".to_string(),
            ancestor_index: 0,
            history_cache_version: 12,
            created_at: 30,
            turns: vec![turn.clone()],
        };

        store
            .replace_session_history_snapshots(Agent::Pi, "child", &[snapshot])
            .unwrap();
        let loaded = store
            .get_session_history_snapshots(Agent::Pi, "child")
            .unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].child_agent, Agent::Pi);
        assert_eq!(loaded[0].child_session_id, "child");
        assert_eq!(loaded[0].ancestor_agent, Agent::Codex);
        assert_eq!(loaded[0].ancestor_session_id, "parent");
        assert_eq!(loaded[0].history_cache_version, 12);
        assert_eq!(loaded[0].turns.len(), 1);
        assert_eq!(loaded[0].turns[0].turn_id, turn.turn_id);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upserting_indexed_session_preserves_placeholder_lineage() {
        let path = unique_db("sessio-lineage-preserve");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let pending = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Pi,
            forked_from_agent: Some(Agent::Claude),
            forked_from_id: Some("parent".to_string()),
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 0,
            rename_title: None,
            title: Some("pending".to_string()),
            first_user_message: Some("pending".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session("", &pending).unwrap();

        let indexed = SessionInfo {
            file_path: "/tmp/project/pi-child.jsonl".to_string(),
            file_size: 256,
            partial: false,
            title: Some("indexed".to_string()),
            first_user_message: Some("indexed".to_string()),
            forked_from_agent: None,
            forked_from_id: None,
            ..pending
        };
        store
            .replace_by_scope("/tmp/project/pi-child.jsonl", Agent::Pi, &[indexed])
            .unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Pi && session.id == "child")
            .unwrap();
        assert_eq!(row.forked_from_agent, Some(Agent::Claude));
        assert_eq!(row.forked_from_id.as_deref(), Some("parent"));
        assert_eq!(row.file_path, "/tmp/project/pi-child.jsonl");
        assert_eq!(row.title.as_deref(), Some("indexed"));
        assert_eq!(row.first_user_message.as_deref(), Some("indexed"));
        assert!(!row.partial);
        let row_count: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE agent = ? AND session_id = ?",
                params![Agent::Pi.as_str(), "child"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(row_count, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn indexed_session_preserves_rename_title_and_updates_parser_title() {
        let path = unique_db("sessio-manual-title-preserve");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let pending = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 0,
            rename_title: Some("Manual pending title".to_string()),
            title: Some("Manual pending title".to_string()),
            first_user_message: Some("# Sessio stage task".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session("", &pending).unwrap();

        let indexed = SessionInfo {
            file_path: "/tmp/project/codex-child.jsonl".to_string(),
            file_size: 256,
            partial: false,
            rename_title: None,
            title: Some("# Sessio stage task".to_string()),
            first_user_message: Some("# Sessio stage task".to_string()),
            message_count: 4,
            ..pending
        };
        store.upsert_session(&indexed.file_path, &indexed).unwrap();

        let rows = store.list_all_sessions().unwrap();
        let matching = rows
            .iter()
            .filter(|session| session.agent == Agent::Codex && session.id == "child")
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].file_path, "/tmp/project/codex-child.jsonl");
        assert!(!matching[0].partial);
        assert_eq!(
            matching[0].rename_title.as_deref(),
            Some("Manual pending title")
        );
        assert_eq!(matching[0].title.as_deref(), Some("# Sessio stage task"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upserting_existing_scope_preserves_channel_origin() {
        let path = unique_db("sessio-channel-origin-preserve");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let channel = SessionInfo {
            id: "channel-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 0,
            rename_title: None,
            title: Some("channel placeholder".to_string()),
            first_user_message: Some("channel".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Channel,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session("", &channel).unwrap();

        let indexed = SessionInfo {
            file_path: "/tmp/project/channel-session.jsonl".to_string(),
            file_size: 128,
            partial: false,
            title: Some("indexed".to_string()),
            first_user_message: Some("indexed prompt".to_string()),
            origin: crate::models::SessionOrigin::Chat,
            ..channel
        };
        store.upsert_session("", &indexed).unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "channel-session")
            .unwrap();
        assert_eq!(row.origin, crate::models::SessionOrigin::Channel);
        assert_eq!(row.file_path, "/tmp/project/channel-session.jsonl");
        assert!(!row.partial);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upserting_existing_scope_preserves_scheduled_task_and_auxiliary_flags() {
        let path = unique_db("sessio-scheduled-flags-preserve");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let session = SessionInfo {
            id: "summary-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 0,
            rename_title: None,
            title: Some("summary".to_string()),
            first_user_message: Some("summary".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: Some("task-1".to_string()),
            is_auxiliary: true,
            subagents: Vec::new(),
        };
        store.upsert_session("", &session).unwrap();

        let reindexed = SessionInfo {
            file_path: "/tmp/project/summary-session.jsonl".to_string(),
            file_size: 512,
            partial: false,
            scheduled_task_id: None,
            is_auxiliary: false,
            ..session
        };
        store.upsert_session("", &reindexed).unwrap();

        assert!(store.list_all_sessions().unwrap().is_empty());
        let row = store
            .list_sessions_by_refs(&[SessionRef {
                agent: Agent::Codex,
                session_id: "summary-session",
            }])
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(row.scheduled_task_id.as_deref(), Some("task-1"));
        assert!(row.is_auxiliary);
        assert_eq!(row.file_path, "/tmp/project/summary-session.jsonl");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn chat_task_placeholder_does_not_replace_indexed_session_fields() {
        let path = unique_db("sessio-chat-task-placeholder-indexed");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let placeholder = SessionInfo {
            id: "chat-task-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(100),
            updated_at: Some(100),
            message_count: 0,
            rename_title: None,
            title: Some("Auto task placeholder".to_string()),
            first_user_message: Some("placeholder prompt".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: Some("task-chat".to_string()),
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session("", &placeholder).unwrap();

        let indexed = SessionInfo {
            file_path: "/tmp/project/chat-task-session.jsonl".to_string(),
            file_size: 1024,
            started_at: Some(10),
            updated_at: Some(500),
            message_count: 7,
            title: Some("Real indexed title".to_string()),
            first_user_message: Some("real first prompt".to_string()),
            partial: false,
            scheduled_task_id: None,
            ..placeholder
        };
        store.upsert_session(&indexed.file_path, &indexed).unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "chat-task-session")
            .unwrap();
        assert_eq!(row.file_path, "/tmp/project/chat-task-session.jsonl");
        assert_eq!(row.started_at, Some(10));
        assert_eq!(row.updated_at, Some(500));
        assert_eq!(row.message_count, 7);
        assert_eq!(row.title.as_deref(), Some("Real indexed title"));
        assert_eq!(row.first_user_message.as_deref(), Some("real first prompt"));
        assert_eq!(row.scheduled_task_id.as_deref(), Some("task-chat"));
        assert!(!row.partial);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn placeholder_merge_into_existing_real_scope_preserves_provenance() {
        let path = unique_db("sessio-placeholder-provenance-merge");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let base = SessionInfo {
            id: "task-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 1,
            rename_title: None,
            title: Some("task".to_string()),
            first_user_message: Some("task".to_string()),
            file_path: "/tmp/project/task-session.jsonl".to_string(),
            file_size: 128,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session(&base.file_path, &base).unwrap();

        let placeholder = SessionInfo {
            file_path: String::new(),
            file_size: 0,
            partial: true,
            started_at: Some(100),
            updated_at: Some(100),
            message_count: 0,
            title: Some("placeholder title".to_string()),
            first_user_message: Some("placeholder prompt".to_string()),
            scheduled_task_id: Some("task-2".to_string()),
            is_auxiliary: true,
            ..base.clone()
        };
        store.upsert_session("", &placeholder).unwrap();
        assert!(store.list_all_sessions().unwrap().is_empty());

        let row_count: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE agent = ? AND session_id = ?",
                params![Agent::Codex.as_str(), "task-session"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(row_count, 1);
        let row = store
            .list_sessions_by_refs(&[SessionRef {
                agent: Agent::Codex,
                session_id: "task-session",
            }])
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(row.file_path, "/tmp/project/task-session.jsonl");
        assert_eq!(row.started_at, Some(10));
        assert_eq!(row.updated_at, Some(10));
        assert_eq!(row.message_count, 1);
        assert_eq!(row.title.as_deref(), Some("task"));
        assert_eq!(row.first_user_message.as_deref(), Some("task"));
        assert_eq!(row.scheduled_task_id.as_deref(), Some("task-2"));
        assert!(row.is_auxiliary);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn pending_session_after_indexed_real_row_does_not_downgrade_file_path() {
        let path = unique_db("sessio-real-row-wins");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let indexed = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 2,
            rename_title: None,
            title: Some("# Sessio stage task".to_string()),
            first_user_message: Some("# Sessio stage task".to_string()),
            file_path: "/tmp/project/codex-child.jsonl".to_string(),
            file_size: 256,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session(&indexed.file_path, &indexed).unwrap();

        let pending = SessionInfo {
            file_path: String::new(),
            file_size: 0,
            partial: true,
            message_count: 0,
            rename_title: Some("Manual pending title".to_string()),
            title: Some("Manual pending title".to_string()),
            first_user_message: Some("# Sessio stage task".to_string()),
            forked_from_agent: Some(Agent::Pi),
            forked_from_id: Some("parent".to_string()),
            ..indexed
        };
        store.upsert_session("", &pending).unwrap();

        let rows = store.list_all_sessions().unwrap();
        let matching = rows
            .iter()
            .filter(|session| session.agent == Agent::Codex && session.id == "child")
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        let row = matching[0];
        assert_eq!(row.file_path, "/tmp/project/codex-child.jsonl");
        assert!(!row.partial);
        assert_eq!(row.message_count, 2);
        assert_eq!(row.rename_title.as_deref(), Some("Manual pending title"));
        assert_eq!(row.title.as_deref(), Some("# Sessio stage task"));
        assert_eq!(row.forked_from_agent, Some(Agent::Pi));
        assert_eq!(row.forked_from_id.as_deref(), Some("parent"));

        let db_row_count: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE agent = ? AND session_id = ?",
                params![Agent::Codex.as_str(), "child"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(db_row_count, 1);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sidebar_hidden_placeholder_stays_hidden_after_real_path_update() {
        let path = unique_db("sessio-sidebar-hidden-placeholder-real-path");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let project_dir =
            temp_child_path(&std::env::temp_dir(), "sessio-sidebar-hidden-placeholder");
        std::fs::create_dir(&project_dir).unwrap();
        let project_path = std::fs::canonicalize(&project_dir)
            .unwrap()
            .to_string_lossy()
            .to_string();
        store
            .add_project(
                &project_path,
                Some("Sidebar Hidden Placeholder"),
                "research".to_string(),
                None,
            )
            .unwrap();

        let placeholder = SessionInfo {
            id: "pi-live-session".to_string(),
            agent: Agent::Pi,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project_path.clone()),
            project_name: Some("Sidebar Hidden Placeholder".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 2,
            rename_title: None,
            title: Some("live title".to_string()),
            first_user_message: Some("live prompt".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: true,
            subagents: Vec::new(),
        };
        store.upsert_session("", &placeholder).unwrap();
        assert!(store.list_sessions().unwrap().is_empty());
        assert!(store.list_all_sessions().unwrap().is_empty());
        let placeholder_ref = store
            .list_sessions_by_refs(&[SessionRef {
                agent: Agent::Pi,
                session_id: "pi-live-session",
            }])
            .unwrap()
            .pop()
            .unwrap();
        assert!(placeholder_ref.partial);

        let real_path = project_dir
            .join("pi-live-session.jsonl")
            .to_string_lossy()
            .to_string();
        let real = SessionInfo {
            file_path: real_path.clone(),
            file_size: 128,
            partial: false,
            message_count: 4,
            updated_at: Some(30),
            ..placeholder
        };
        store.upsert_session(&project_path, &real).unwrap();

        assert!(store.list_sessions().unwrap().is_empty());
        assert!(store.list_all_sessions().unwrap().is_empty());
        let visible_by_ref = store
            .list_sessions_by_refs(&[SessionRef {
                agent: Agent::Pi,
                session_id: "pi-live-session",
            }])
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(visible_by_ref.file_path, real_path.as_str());
        assert!(!visible_by_ref.partial);
        assert_eq!(visible_by_ref.message_count, 4);

        let is_auxiliary: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT is_auxiliary FROM sessions WHERE agent = ? AND session_id = ?",
                params![Agent::Pi.as_str(), "pi-live-session"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(is_auxiliary, 1);
        let stored = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT file_path, partial, message_count FROM sessions
                 WHERE agent = ? AND session_id = ?",
                params![Agent::Pi.as_str(), "pi-live-session"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(stored.0, real_path);
        assert_eq!(stored.1, 0);
        assert_eq!(stored.2, 4);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn astra_virtual_session_refs_stay_available_when_scopes_disappear() {
        let path = unique_db("astra-virtual-scope-guard");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let virtual_session = SessionInfo {
            id: "astra-child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 1,
            rename_title: Some("Astra delegated task".to_string()),
            title: None,
            first_user_message: Some("# Sessio stage task".to_string()),
            file_path: "astra://run-1/session/astra-child".to_string(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store
            .upsert_session(&virtual_session.file_path, &virtual_session)
            .unwrap();

        store
            .mark_missing_scopes_unavailable(Agent::Codex, &HashSet::new())
            .unwrap();
        store
            .replace_by_scope(&virtual_session.file_path, Agent::Codex, &[])
            .unwrap();
        store
            .mark_file_path_unavailable(&virtual_session.file_path)
            .unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "astra-child")
            .unwrap();
        assert!(row.available);
        assert_eq!(row.file_path, virtual_session.file_path);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reindex_preserves_rename_title_and_updates_parser_title() {
        let path = unique_db("sessio-reindex-title-preserve");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let existing = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 2,
            rename_title: Some("Manual pending title".to_string()),
            title: Some("Manual pending title".to_string()),
            first_user_message: Some("# Sessio stage task".to_string()),
            file_path: "/tmp/project/codex-child.jsonl".to_string(),
            file_size: 256,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store
            .upsert_session(&existing.file_path, &existing)
            .unwrap();

        let reindexed = SessionInfo {
            rename_title: None,
            title: Some("# Sessio stage task".to_string()),
            message_count: 4,
            updated_at: Some(30),
            ..existing
        };
        store
            .upsert_session(&reindexed.file_path, &reindexed)
            .unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "child")
            .unwrap();
        assert_eq!(row.rename_title.as_deref(), Some("Manual pending title"));
        assert_eq!(row.title.as_deref(), Some("# Sessio stage task"));
        assert_eq!(row.message_count, 4);
        assert!(!row.partial);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn update_session_rename_title_is_explicit_and_clearable() {
        let path = unique_db("sessio-rename-title-explicit");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let session = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 2,
            rename_title: None,
            title: Some("Indexed title".to_string()),
            first_user_message: Some("First prompt".to_string()),
            file_path: "/tmp/project/codex-child.jsonl".to_string(),
            file_size: 256,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session(&session.file_path, &session).unwrap();

        store
            .update_session_rename_title(Agent::Codex, "child", Some("Renamed"))
            .unwrap();
        let renamed = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "child")
            .unwrap();
        assert_eq!(renamed.rename_title.as_deref(), Some("Renamed"));
        assert_eq!(renamed.title.as_deref(), Some("Indexed title"));

        store
            .update_session_rename_title(Agent::Codex, "child", Some("  "))
            .unwrap();
        let cleared = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "child")
            .unwrap();
        assert_eq!(cleared.rename_title, None);
        assert_eq!(cleared.title.as_deref(), Some("Indexed title"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upserting_session_keeps_existing_db_lineage_over_parsed_fallback() {
        let path = unique_db("sessio-lineage-db-wins");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let existing = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: Some(Agent::Pi),
            forked_from_id: Some("db-parent".to_string()),
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 0,
            rename_title: None,
            title: Some("existing".to_string()),
            first_user_message: Some("existing".to_string()),
            file_path: "/tmp/project/child.jsonl".to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store
            .upsert_session("/tmp/project/child.jsonl", &existing)
            .unwrap();

        let parsed = SessionInfo {
            forked_from_agent: Some(Agent::Claude),
            forked_from_id: Some("cross-context-parent".to_string()),
            title: Some("parsed".to_string()),
            ..existing
        };
        store
            .upsert_session("/tmp/project/child.jsonl", &parsed)
            .unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "child")
            .unwrap();
        assert_eq!(row.forked_from_agent, Some(Agent::Pi));
        assert_eq!(row.forked_from_id.as_deref(), Some("db-parent"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn upserting_session_does_not_mix_existing_id_with_parsed_agent() {
        let path = unique_db("sessio-lineage-no-mix");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let existing = SessionInfo {
            id: "child".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: Some("db-parent".to_string()),
            project_path: Some("/tmp/project".to_string()),
            project_name: Some("project".to_string()),
            started_at: Some(10),
            updated_at: Some(10),
            message_count: 0,
            rename_title: None,
            title: Some("existing".to_string()),
            first_user_message: Some("existing".to_string()),
            file_path: "/tmp/project/child.jsonl".to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store
            .upsert_session("/tmp/project/child.jsonl", &existing)
            .unwrap();

        let parsed = SessionInfo {
            forked_from_agent: Some(Agent::Claude),
            forked_from_id: Some("cross-context-parent".to_string()),
            ..existing
        };
        store
            .upsert_session("/tmp/project/child.jsonl", &parsed)
            .unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Codex && session.id == "child")
            .unwrap();
        assert_eq!(row.forked_from_agent, None);
        assert_eq!(row.forked_from_id.as_deref(), Some("db-parent"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn projects_start_empty_and_gate_visible_sessions() {
        let path = unique_db("sessio-project-gates-sessions");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let project_dir = temp_child_path(&std::env::temp_dir(), "sessio-visible-project");
        std::fs::create_dir(&project_dir).unwrap();
        let project_path = std::fs::canonicalize(&project_dir)
            .unwrap()
            .to_string_lossy()
            .to_string();

        let session = SessionInfo {
            id: "visible".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project_path.clone()),
            project_name: Some("Visible".to_string()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 1,
            rename_title: None,
            title: Some("hello".to_string()),
            first_user_message: Some("hello".to_string()),
            file_path: project_dir
                .join("session.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session(&session.file_path, &session).unwrap();

        assert!(store.list_projects().unwrap().is_empty());
        assert!(store.list_sessions().unwrap().is_empty());
        assert_eq!(store.list_all_sessions().unwrap().len(), 1);

        let project = store
            .add_project(&project_path, Some("Visible"), "research".to_string(), None)
            .unwrap();
        assert_eq!(project.process_template_id, "research");
        assert_eq!(store.list_sessions().unwrap().len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn project_and_kanban_crud_roundtrip() {
        let path = unique_db("sessio-project-kanban");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-project-parent");
        std::fs::create_dir(&parent).unwrap();

        let created = store
            .create_project(
                &parent.to_string_lossy(),
                "video-plan",
                "video_production".to_string(),
                None,
            )
            .unwrap();
        assert_eq!(created.name, "video-plan");
        assert_eq!(created.process_template_id, "video_production");
        assert!(Path::new(&created.path).exists());

        let updated = store
            .update_project(&created.id, Some("Video Plan"), Some("general".to_string()))
            .unwrap();
        assert_eq!(updated.name, "Video Plan");
        assert_eq!(updated.process_template_id, "general");

        let item = store
            .create_kanban_item(&created.id, "Draft outline", Some("scene beats"))
            .unwrap();
        assert_eq!(item.status, KanbanStatus::Todo);

        let moved = store
            .update_kanban_item(&item.id, None, None, Some(KanbanStatus::AgentReview))
            .unwrap();
        assert_eq!(moved.status, KanbanStatus::AgentReview);

        let edited = store
            .update_kanban_item(
                &item.id,
                Some("Draft cold open"),
                Some(Some("")),
                Some(KanbanStatus::Done),
            )
            .unwrap();
        assert_eq!(edited.title, "Draft cold open");
        assert_eq!(edited.description, None);
        assert_eq!(edited.status, KanbanStatus::Done);

        assert_eq!(store.list_kanban_items(&created.id).unwrap().len(), 1);
        store.delete_kanban_item(&item.id).unwrap();
        assert!(store.list_kanban_items(&created.id).unwrap().is_empty());

        store.archive_project(&created.id).unwrap();
        assert!(store.list_projects().unwrap().is_empty());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn builtin_process_template_stage_kinds_are_process_template_specific() {
        let path = unique_db("sessio-process-template-stage-kinds");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(
            &std::env::temp_dir(),
            "sessio-process-template-stage-parent",
        );
        std::fs::create_dir(&parent).unwrap();

        let code = store
            .create_project(
                &parent.to_string_lossy(),
                "code-flow",
                "code".to_string(),
                None,
            )
            .unwrap();
        let writing = store
            .create_project(
                &parent.to_string_lossy(),
                "writing-flow",
                "writing".to_string(),
                None,
            )
            .unwrap();
        let video = store
            .create_project(
                &parent.to_string_lossy(),
                "video-flow",
                "video_production".to_string(),
                None,
            )
            .unwrap();
        let general = store
            .create_project(
                &parent.to_string_lossy(),
                "general-flow",
                "general".to_string(),
                None,
            )
            .unwrap();

        let code_kinds = stage_kinds(store.list_project_stages(&code.id).unwrap());
        assert_eq!(
            code_kinds,
            vec![
                StageType::Research,
                StageType::Plan,
                StageType::Develop,
                StageType::Review,
                StageType::Human,
                StageType::Done,
            ]
        );
        let code_assistants = store.list_assistants(Some(&code.id)).unwrap();
        assert_eq!(code_assistants.len(), 4);
        assert!(code_assistants
            .iter()
            .all(|item| item.project_id.as_deref() == Some(code.id.as_str())));
        assert!(code_assistants
            .iter()
            .all(|item| item.process_template_id.as_deref()
                == Some(code.process_template_id.as_str())));

        let writing_kinds = stage_kinds(store.list_project_stages(&writing.id).unwrap());
        assert_eq!(
            writing_kinds,
            vec![
                StageType::Research,
                StageType::Plan,
                StageType::Writing,
                StageType::Editing,
                StageType::Proofreading,
                StageType::Human,
                StageType::Done,
            ]
        );

        let video_kinds = stage_kinds(store.list_project_stages(&video.id).unwrap());
        assert_eq!(
            video_kinds,
            vec![
                StageType::Research,
                StageType::Plan,
                StageType::Screenplay,
                StageType::Storyboard,
                StageType::Design,
                StageType::Production,
                StageType::Review,
                StageType::Human,
                StageType::Done,
            ]
        );

        let general_kinds = stage_kinds(store.list_project_stages(&general.id).unwrap());
        assert_eq!(
            general_kinds,
            vec![
                StageType::Research,
                StageType::Plan,
                StageType::Build,
                StageType::Review,
                StageType::Human,
                StageType::Done,
            ]
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    fn stage_kinds(stages: Vec<ProjectStageInfo>) -> Vec<StageType> {
        stages
            .into_iter()
            .filter(|stage| stage.stage_type == ProjectStageType::Builtin)
            .filter_map(|stage| stage.kind)
            .collect()
    }

    #[test]
    fn project_builtin_stage_order_can_be_reordered_and_survives_seed() {
        let path = unique_db("sessio-builtin-stage-order");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-builtin-stage-order-parent");
        std::fs::create_dir(&parent).unwrap();
        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "code-flow",
                "code".to_string(),
                None,
            )
            .unwrap();

        let stages = store.list_project_stages(&project.id).unwrap();
        let research = stages
            .iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap()
            .clone();
        let plan = stages
            .iter()
            .find(|stage| stage.kind == Some(StageType::Plan))
            .unwrap()
            .clone();

        let moved = store
            .update_project_stage(
                &research.id,
                ProjectStagePatch {
                    order: Some(plan.order),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(moved.order, 1);
        assert_eq!(
            stage_kinds(store.list_project_stages(&project.id).unwrap()),
            vec![
                StageType::Plan,
                StageType::Research,
                StageType::Develop,
                StageType::Review,
                StageType::Human,
                StageType::Done,
            ]
        );

        store.init().unwrap();
        assert_eq!(
            stage_kinds(store.list_project_stages(&project.id).unwrap()),
            vec![
                StageType::Plan,
                StageType::Research,
                StageType::Develop,
                StageType::Review,
                StageType::Human,
                StageType::Done,
            ]
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn project_instantiates_selected_builtin_stage_templates() {
        let path = unique_db("sessio-project-stage-selection");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-stage-selection-parent");
        std::fs::create_dir(&parent).unwrap();

        let templates = store.list_process_template_stages("code").unwrap();
        let research_template = templates
            .iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert_eq!(research_template.assistants.len(), 1);
        assert_eq!(
            research_template.assistants[0].assistant_id,
            stable_process_template_builtin_assistant_id("code", "assistant-builtin-research")
        );
        let selected_template_ids = templates
            .iter()
            .filter(|stage| matches!(stage.kind, Some(StageType::Research | StageType::Done)))
            .map(|stage| stage.id.clone())
            .collect::<Vec<_>>();
        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "selected-stages",
                "code".to_string(),
                Some(&selected_template_ids),
            )
            .unwrap();

        let stages = store.list_project_stages(&project.id).unwrap();
        assert_eq!(
            stage_kinds(stages.clone()),
            vec![StageType::Research, StageType::Done]
        );
        assert!(stages
            .iter()
            .all(|stage| stage.project_id.as_deref() == Some(project.id.as_str())));
        assert!(stages
            .iter()
            .all(|stage| stage.stage_type == ProjectStageType::Builtin));
        assert!(stages
            .iter()
            .all(|stage| !selected_template_ids.contains(&stage.id)));
        let project_research = stages
            .iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert_eq!(project_research.assistants.len(), 1);
        let project_assistants = store.list_assistants(Some(&project.id)).unwrap();
        assert_eq!(project_assistants.len(), 4);
        assert_eq!(
            project_research.assistants[0].assistant_id,
            stable_project_builtin_assistant_id(
                &project.id,
                &stable_process_template_builtin_assistant_id("code", "assistant-builtin-research")
            )
        );

        let thread = store
            .create_thread(&project.id, "Use stages", None)
            .unwrap();
        let assistant = store
            .create_assistant(NewAssistant {
                name: "Researcher",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        assert!(store
            .add_thread_stage(
                &thread.id,
                &research_template.id,
                std::slice::from_ref(&assistant.id),
            )
            .is_err());
        assert!(store
            .add_thread_stage(&thread.id, &project_research.id, &[assistant.id])
            .is_ok());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn thread_stage_state_defaults_and_updates() {
        let path = unique_db("sessio-thread-stage-state");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-stage-state-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "stage-state",
                "code".to_string(),
                None,
            )
            .unwrap();
        let templates = store.list_project_stages(&project.id).unwrap();
        let first_id = templates[0].id.clone();
        let second_id = templates[1].id.clone();
        let thread = store
            .create_thread(&project.id, "State thread", None)
            .unwrap();
        let stage_a = store.add_thread_stage(&thread.id, &first_id, &[]).unwrap();
        let stage_b = store.add_thread_stage(&thread.id, &second_id, &[]).unwrap();

        // Lazy default: with no active stage and no stored state, all stages
        // read as not_started.
        let stages = load_thread_stages(&store.conn.lock().unwrap(), &thread.id).unwrap();
        assert!(stages
            .iter()
            .all(|stage| stage.status == StageStatus::NotStarted));

        // Setting the active stage derives completed/in_progress for rows that
        // still have no explicit state.
        store.set_thread_stage(&thread.id, &stage_b.id).unwrap();
        let stages = load_thread_stages(&store.conn.lock().unwrap(), &thread.id).unwrap();
        let status_of = |id: &str| stages.iter().find(|s| s.id == id).unwrap().status;
        assert_eq!(status_of(&stage_a.id), StageStatus::Completed);
        assert_eq!(status_of(&stage_b.id), StageStatus::InProgress);

        // An explicit write persists and overrides the derived default.
        let updated = store
            .update_thread_stage_state(
                &stage_a.id,
                Some(StageStatus::Blocked),
                Some(Some("hit an API limit".to_string())),
                None,
            )
            .unwrap();
        assert_eq!(updated.status, StageStatus::Blocked);
        assert_eq!(updated.summary.as_deref(), Some("hit an API limit"));
        let stages = load_thread_stages(&store.conn.lock().unwrap(), &thread.id).unwrap();
        assert_eq!(
            stages.iter().find(|s| s.id == stage_a.id).unwrap().status,
            StageStatus::Blocked
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn thread_stage_issues_crud_and_cascade() {
        let path = unique_db("sessio-thread-stage-issues");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-stage-issues-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "issues",
                "code".to_string(),
                None,
            )
            .unwrap();
        let templates = store.list_project_stages(&project.id).unwrap();
        let first_id = templates[0].id.clone();
        let thread = store
            .create_thread(&project.id, "Issue thread", None)
            .unwrap();
        let stage = store.add_thread_stage(&thread.id, &first_id, &[]).unwrap();

        // create defaults to open and round-trips the provided fields.
        let issue = store
            .create_thread_stage_issue(
                &stage.id,
                "API rate limit",
                Some("429s"),
                IssueSeverity::High,
            )
            .unwrap();
        assert_eq!(issue.status, IssueStatus::Open);
        assert_eq!(issue.severity, IssueSeverity::High);
        assert_eq!(issue.title, "API rate limit");
        assert_eq!(issue.description.as_deref(), Some("429s"));

        // list + load_thread_stages both surface the issue under its stage.
        assert_eq!(store.list_thread_stage_issues(&stage.id).unwrap().len(), 1);
        let stages = load_thread_stages(&store.conn.lock().unwrap(), &thread.id).unwrap();
        let stage_row = stages.iter().find(|s| s.id == stage.id).unwrap();
        assert_eq!(stage_row.issues.len(), 1);
        assert_eq!(stage_row.issues[0].id, issue.id);

        // update merges fields: empty description clears, omitted title stays.
        let updated = store
            .update_thread_stage_issue(
                &issue.id,
                None,
                Some(None),
                Some(IssueStatus::Resolved),
                Some(IssueSeverity::Low),
            )
            .unwrap();
        assert_eq!(updated.status, IssueStatus::Resolved);
        assert_eq!(updated.severity, IssueSeverity::Low);
        assert_eq!(updated.description, None);
        assert_eq!(updated.title, "API rate limit");

        // deleting the parent thread stage cascades to its issues.
        store.delete_thread_stage(&stage.id).unwrap();
        assert!(store
            .list_thread_stage_issues(&stage.id)
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn builtin_assistant_seeds_have_colors() {
        let path = unique_db("sessio-builtin-assistant-colors");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let global_research = store
            .list_assistants(None)
            .unwrap()
            .into_iter()
            .find(|assistant| assistant.id == "assistant-builtin-research")
            .unwrap();
        assert_eq!(global_research.color.as_deref(), Some("#0ea5e9"));

        let process_template_research = store
            .list_assistants(None)
            .unwrap()
            .into_iter()
            .find(|assistant| {
                assistant.id
                    == stable_process_template_builtin_assistant_id(
                        "code",
                        "assistant-builtin-research",
                    )
            })
            .unwrap();
        assert_eq!(process_template_research.color.as_deref(), Some("#0ea5e9"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn project_instantiated_builtin_assistants_keep_colors() {
        let path = unique_db("sessio-project-assistant-colors");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-project-assistant-colors");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "assistant-colors",
                "code".to_string(),
                None,
            )
            .unwrap();
        let project_research_id = stable_project_assistant_id(
            &project.id,
            &stable_process_template_builtin_assistant_id("code", "assistant-builtin-research"),
        );
        let project_research = store
            .list_assistants(Some(&project.id))
            .unwrap()
            .into_iter()
            .find(|assistant| assistant.id == project_research_id)
            .unwrap();
        assert_eq!(project_research.color.as_deref(), Some("#0ea5e9"));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn seed_does_not_recreate_deleted_process_template_stage_assistant_bindings() {
        let path = unique_db("sessio-process-template-stage-assistant-seed");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let research = store
            .list_process_template_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert_eq!(research.assistants.len(), 1);
        store
            .update_project_stage_assistants(&research.id, &[])
            .unwrap();

        store.init().unwrap();

        let research = store
            .list_process_template_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert!(research.assistants.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn process_template_stage_templates_accept_only_same_process_template_assistants() {
        let path = unique_db("sessio-process-template-stage-assistant-scope");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let research = store
            .list_process_template_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        let code_assistant = store
            .create_assistant(NewAssistant {
                name: "Code reviewer",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: Some("code".to_string()),
                project_id: None,
            })
            .unwrap();
        let writing_assistant = store
            .create_assistant(NewAssistant {
                name: "Writing reviewer",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: Some("writing".to_string()),
                project_id: None,
            })
            .unwrap();
        let shared_assistant = store
            .create_assistant(NewAssistant {
                name: "Shared reviewer",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: None,
            })
            .unwrap();

        let updated = store
            .update_project_stage_assistants(
                &research.id,
                &[code_assistant.id.clone(), shared_assistant.id.clone()],
            )
            .unwrap();
        assert_eq!(updated.assistants.len(), 2);
        assert_eq!(updated.assistants[0].assistant_id, code_assistant.id);
        assert_eq!(updated.assistants[1].assistant_id, shared_assistant.id);

        let wrong_process_template_error = store
            .update_project_stage_assistants(&research.id, &[writing_assistant.id])
            .unwrap_err()
            .to_string();
        assert!(wrong_process_template_error.contains("assistant is not available for this stage"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn custom_assistants_can_be_global_shared() {
        let path = unique_db("sessio-global-custom-assistant");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let assistant = store
            .create_assistant(NewAssistant {
                name: "Shared reviewer",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: vec![
                    "skill-a".to_string(),
                    "skill-a".to_string(),
                    " skill-b ".to_string(),
                ],
                selected_mcp_ids: vec!["mcp-a".to_string(), String::new(), "mcp-b".to_string()],
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: None,
            })
            .unwrap();
        assert_eq!(assistant.process_template_id, None);
        assert_eq!(assistant.project_id, None);
        assert_eq!(assistant.assistant_type, AssistantType::Custom);
        assert_eq!(assistant.selected_skill_ids, vec!["skill-a", "skill-b"]);
        assert_eq!(assistant.selected_mcp_ids, vec!["mcp-a", "mcp-b"]);

        let updated = store
            .update_assistant(
                &assistant.id,
                None,
                None,
                None,
                None,
                Some(vec!["skill-c".to_string()]),
                Some(vec!["mcp-c".to_string(), "mcp-c".to_string()]),
                None,
            )
            .unwrap();
        assert_eq!(updated.selected_skill_ids, vec!["skill-c"]);
        assert_eq!(updated.selected_mcp_ids, vec!["mcp-c"]);
        let listed = store
            .list_assistants(None)
            .unwrap()
            .into_iter()
            .find(|item| item.id == assistant.id)
            .unwrap();
        assert_eq!(listed.selected_skill_ids, vec!["skill-c"]);
        assert_eq!(listed.selected_mcp_ids, vec!["mcp-c"]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn project_creation_instantiates_global_custom_stage_assistants() {
        let path = unique_db("sessio-project-global-stage-assistant");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(
            &std::env::temp_dir(),
            "sessio-project-global-assistant-parent",
        );
        std::fs::create_dir(&parent).unwrap();

        let shared_assistant = store
            .create_assistant(NewAssistant {
                name: "Shared reviewer",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Review from global context"),
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: None,
            })
            .unwrap();
        let research_template = store
            .list_process_template_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        store
            .update_project_stage_assistants(
                &research_template.id,
                std::slice::from_ref(&shared_assistant.id),
            )
            .unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "global-stage-assistant",
                "code".to_string(),
                None,
            )
            .unwrap();
        let project_assistant_id = stable_project_assistant_id(&project.id, &shared_assistant.id);
        let project_assistants = store.list_assistants(Some(&project.id)).unwrap();
        let project_shared_assistant = project_assistants
            .iter()
            .find(|assistant| assistant.id == project_assistant_id)
            .unwrap();
        assert_eq!(project_shared_assistant.name, shared_assistant.name);
        assert_eq!(
            project_shared_assistant.assistant_type,
            AssistantType::Custom
        );
        assert_eq!(
            project_shared_assistant.project_id.as_deref(),
            Some(project.id.as_str())
        );

        let project_research = store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert_eq!(project_research.assistants.len(), 1);
        assert_eq!(
            project_research.assistants[0].assistant_id,
            project_assistant_id
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn kanban_items_link_and_aggregate_sessions() {
        let path = unique_db("sessio-kanban-session-links");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-kanban-link-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "linked",
                "code".to_string(),
                None,
            )
            .unwrap();
        let session = SessionInfo {
            id: "session-a".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 3,
            rename_title: None,
            title: Some("Implement feature".to_string()),
            first_user_message: Some("Please implement feature".to_string()),
            file_path: Path::new(&project.path)
                .join("session-a.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session(&session.file_path, &session).unwrap();
        let item = store
            .create_kanban_item(&project.id, "Build feature", None)
            .unwrap();

        let linked = store
            .link_kanban_item_session(&item.id, Agent::Codex, &session.id)
            .unwrap();
        assert_eq!(linked.sessions.len(), 1);
        assert_eq!(linked.sessions[0].id, session.id);

        let listed = store.list_kanban_items(&project.id).unwrap();
        assert_eq!(listed[0].sessions.len(), 1);
        assert_eq!(
            listed[0].sessions[0].title.as_deref(),
            Some("Implement feature")
        );

        let unlinked = store
            .unlink_kanban_item_session(&item.id, Agent::Codex, &session.id)
            .unwrap();
        assert!(unlinked.sessions.is_empty());

        let other_parent = temp_child_path(&std::env::temp_dir(), "sessio-kanban-other-parent");
        std::fs::create_dir(&other_parent).unwrap();
        let other_project = store
            .create_project(
                &other_parent.to_string_lossy(),
                "other",
                "code".to_string(),
                None,
            )
            .unwrap();
        let other_session = SessionInfo {
            id: "session-b".to_string(),
            project_path: Some(other_project.path.clone()),
            project_name: Some(other_project.name.clone()),
            file_path: Path::new(&other_project.path)
                .join("session-b.jsonl")
                .to_string_lossy()
                .to_string(),
            title: Some("Other project".to_string()),
            ..session
        };
        store
            .upsert_session(&other_session.file_path, &other_session)
            .unwrap();
        assert!(store
            .link_kanban_item_session(&item.id, Agent::Codex, &other_session.id)
            .is_err());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
        let _ = std::fs::remove_dir_all(&other_parent);
    }

    #[test]
    fn plan_task_supersede_and_relink_update_sidebar_visibility() {
        let path = unique_db("sessio-plan-task-origin");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-plan-task-origin-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "plan-task-origin",
                "code".to_string(),
                None,
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Plan task origin", None)
            .unwrap();
        let round = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("origin round"),
                mode: PlanRoundMode::Parallel,
                source: PlanRoundSource::Manual,
                status: PlanRoundStatus::Planned,
                tasks: vec![NewPlanTask {
                    thread_stage_id: None,
                    assistant_id: None,
                    agent_participant_id: None,
                    target_agent: Agent::Codex,
                    stage_snapshot_json: None,
                    assistant_snapshot_json: None,
                    agent_snapshot_json: r#"{"agent":"codex"}"#,
                    title: "Origin task",
                    prompt: "Track session origin",
                    expected_output: None,
                    risk: PlanTaskRisk::Low,
                    sort_order: 0,
                    status: PlanTaskStatus::Running,
                }],
            })
            .unwrap();

        let first = test_session(&project, "runtime-first", "Runtime first");
        let second = test_session(&project, "runtime-second", "Runtime second");
        let real = test_session(&project, "agent-real", "Agent real");
        for session in [&first, &second, &real] {
            store.upsert_session(&session.file_path, session).unwrap();
        }
        assert_eq!(store.list_sessions().unwrap().len(), 3);

        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: &first.id,
                role: PlanTaskSessionRole::Runtime,
                attempt_id: Some("attempt-1"),
                attempt_count: 1,
            })
            .unwrap();
        assert_eq!(
            visible_session_ids(&store),
            vec![real.id.clone(), second.id.clone()]
        );

        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: &second.id,
                role: PlanTaskSessionRole::Runtime,
                attempt_id: Some("attempt-1"),
                attempt_count: 1,
            })
            .unwrap();
        let visible_after_supersede = visible_session_ids(&store);
        assert_eq!(
            visible_after_supersede,
            vec![real.id.clone(), first.id.clone()]
        );

        store
            .relink_plan_task_session(
                NewPlanTaskSession {
                    task_id: &round.tasks[0].id,
                    agent: Agent::Codex,
                    session_id: &second.id,
                    role: PlanTaskSessionRole::Runtime,
                    attempt_id: Some("attempt-1"),
                    attempt_count: 1,
                },
                &real.id,
                PlanTaskSessionRole::Delegated,
            )
            .unwrap();
        let visible_after_relink = visible_session_ids(&store);
        assert_eq!(
            visible_after_relink,
            vec![first.id.clone(), second.id.clone()]
        );

        let real_ref = store
            .list_sessions_by_refs(&[SessionRef {
                agent: Agent::Codex,
                session_id: &real.id,
            }])
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(real_ref.origin, crate::models::SessionOrigin::Thread);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn plan_round_tasks_sessions_and_sequential_invariants_roundtrip() {
        let path = unique_db("sessio-plan-rounds");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-plan-round-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "plan-rounds",
                "code".to_string(),
                None,
            )
            .unwrap();
        let assistant = store
            .create_assistant(NewAssistant {
                name: "Planner",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Plan carefully"),
                color: Some("#22c55e"),
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        let thread = store
            .create_thread_with_options(
                &project.id,
                "Coordinate plan tasks",
                None,
                ThreadKind::Teamwork,
                std::slice::from_ref(&assistant.id),
                &[],
            )
            .unwrap();
        let stage_template = store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        let thread_stage = store
            .add_thread_stage(
                &thread.id,
                &stage_template.id,
                std::slice::from_ref(&assistant.id),
            )
            .unwrap();

        let parallel = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("parallel summary"),
                mode: PlanRoundMode::Parallel,
                source: PlanRoundSource::Manual,
                status: PlanRoundStatus::Planned,
                tasks: vec![
                    NewPlanTask {
                        thread_stage_id: Some(&thread_stage.id),
                        assistant_id: Some(&assistant.id),
                        agent_participant_id: None,
                        target_agent: Agent::Codex,
                        stage_snapshot_json: Some(r#"{"stage":"research-v1"}"#),
                        assistant_snapshot_json: Some(r#"{"assistant":"planner-v1"}"#),
                        agent_snapshot_json: r#"{"agent":"codex-v1"}"#,
                        title: "Research",
                        prompt: "Research prompt",
                        expected_output: Some("Notes"),
                        risk: PlanTaskRisk::Medium,
                        sort_order: 0,
                        status: PlanTaskStatus::Running,
                    },
                    NewPlanTask {
                        thread_stage_id: None,
                        assistant_id: Some(&assistant.id),
                        agent_participant_id: None,
                        target_agent: Agent::Claude,
                        stage_snapshot_json: None,
                        assistant_snapshot_json: Some(r#"{"assistant":"planner-v1"}"#),
                        agent_snapshot_json: r#"{"agent":"claude-v1"}"#,
                        title: "Review",
                        prompt: "Review prompt",
                        expected_output: Some("Review notes"),
                        risk: PlanTaskRisk::High,
                        sort_order: 1,
                        status: PlanTaskStatus::Running,
                    },
                ],
            })
            .unwrap();
        assert_eq!(parallel.round_index, 0);
        assert_eq!(parallel.status, PlanRoundStatus::Running);
        assert_eq!(parallel.tasks.len(), 2);
        assert!(parallel
            .tasks
            .iter()
            .all(|task| task.status == PlanTaskStatus::Running));
        assert_eq!(
            parallel.tasks[0].stage_snapshot_json.as_deref(),
            Some(r#"{"stage":"research-v1"}"#)
        );

        let session_ref = store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &parallel.tasks[0].id,
                agent: Agent::Codex,
                session_id: "runtime-session-1",
                role: PlanTaskSessionRole::Runtime,
                attempt_id: Some("attempt-1"),
                attempt_count: 1,
            })
            .unwrap();
        assert_eq!(session_ref.role, PlanTaskSessionRole::Runtime);
        assert_eq!(session_ref.attempt_id.as_deref(), Some("attempt-1"));
        assert_eq!(session_ref.attempt_count, 1);
        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &parallel.tasks[0].id,
                agent: Agent::Codex,
                session_id: "runtime-session-stale",
                role: PlanTaskSessionRole::Runtime,
                attempt_id: Some("attempt-1"),
                attempt_count: 1,
            })
            .unwrap();
        let parallel = store.get_plan_round(&parallel.id).unwrap().unwrap();
        assert_eq!(parallel.tasks[0].sessions.len(), 2);
        let first_runtime = parallel.tasks[0]
            .sessions
            .iter()
            .find(|session| session.session_id == "runtime-session-1")
            .unwrap();
        assert!(first_runtime.superseded_at.is_some());
        assert!(parallel.tasks[0]
            .sessions
            .iter()
            .any(|session| session.session_id == "runtime-session-stale"
                && session.superseded_at.is_none()));
        let relinked = store
            .relink_plan_task_session(
                NewPlanTaskSession {
                    task_id: &parallel.tasks[0].id,
                    agent: Agent::Codex,
                    session_id: "runtime-session-1",
                    role: PlanTaskSessionRole::Runtime,
                    attempt_id: Some("attempt-1"),
                    attempt_count: 1,
                },
                "agent-session-1",
                PlanTaskSessionRole::Delegated,
            )
            .unwrap();
        assert_eq!(relinked.session_id, "agent-session-1");
        assert_eq!(relinked.role, PlanTaskSessionRole::Delegated);
        assert_eq!(relinked.attempt_id.as_deref(), Some("attempt-1"));
        let parallel = store.get_plan_round(&parallel.id).unwrap().unwrap();
        assert_eq!(parallel.tasks[0].sessions.len(), 3);
        assert!(parallel.tasks[0].sessions.iter().any(|session| {
            session.session_id == "agent-session-1"
                && session.role == PlanTaskSessionRole::Delegated
                && session.superseded_at.is_none()
        }));
        assert!(parallel.tasks[0].sessions.iter().any(|session| {
            session.session_id == "runtime-session-1"
                && session.role == PlanTaskSessionRole::Runtime
                && session.superseded_at.is_some()
        }));
        let late_runtime = store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &parallel.tasks[0].id,
                agent: Agent::Codex,
                session_id: "runtime-session-late",
                role: PlanTaskSessionRole::Runtime,
                attempt_id: Some("attempt-1"),
                attempt_count: 1,
            })
            .unwrap();
        assert!(late_runtime.superseded_at.is_some());
        let parallel = store.get_plan_round(&parallel.id).unwrap().unwrap();
        assert!(parallel.tasks[0].sessions.iter().any(|session| {
            session.session_id == "runtime-session-late"
                && session.role == PlanTaskSessionRole::Runtime
                && session.superseded_at.is_some()
        }));

        store
            .update_assistant(
                &assistant.id,
                Some("Planner Renamed"),
                None,
                Some(Some("New prompt")),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let reloaded = store.get_plan_round(&parallel.id).unwrap().unwrap();
        assert_eq!(
            reloaded.tasks[0].assistant_snapshot_json.as_deref(),
            Some(r#"{"assistant":"planner-v1"}"#)
        );

        let lower_task_skip_error = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("bad sequential"),
                mode: PlanRoundMode::Sequential,
                source: PlanRoundSource::Manual,
                status: PlanRoundStatus::Planned,
                tasks: vec![
                    NewPlanTask {
                        thread_stage_id: None,
                        assistant_id: None,
                        agent_participant_id: None,
                        target_agent: Agent::Codex,
                        stage_snapshot_json: None,
                        assistant_snapshot_json: None,
                        agent_snapshot_json: r#"{"agent":"codex"}"#,
                        title: "First planned",
                        prompt: "First prompt",
                        expected_output: None,
                        risk: PlanTaskRisk::Low,
                        sort_order: 0,
                        status: PlanTaskStatus::Planned,
                    },
                    NewPlanTask {
                        thread_stage_id: None,
                        assistant_id: None,
                        agent_participant_id: None,
                        target_agent: Agent::Codex,
                        stage_snapshot_json: None,
                        assistant_snapshot_json: None,
                        agent_snapshot_json: r#"{"agent":"codex"}"#,
                        title: "Second running",
                        prompt: "Second prompt",
                        expected_output: None,
                        risk: PlanTaskRisk::Low,
                        sort_order: 1,
                        status: PlanTaskStatus::Running,
                    },
                ],
            })
            .unwrap_err()
            .to_string();
        assert!(lower_task_skip_error.contains("lowest-order planned task"));

        let sequential = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("sequential summary"),
                mode: PlanRoundMode::Sequential,
                source: PlanRoundSource::Agent,
                status: PlanRoundStatus::Planned,
                tasks: vec![
                    NewPlanTask {
                        thread_stage_id: None,
                        assistant_id: Some(&assistant.id),
                        agent_participant_id: None,
                        target_agent: Agent::Codex,
                        stage_snapshot_json: None,
                        assistant_snapshot_json: Some(r#"{"assistant":"planner-v2"}"#),
                        agent_snapshot_json: r#"{"agent":"codex-v2"}"#,
                        title: "Step 1",
                        prompt: "Step one",
                        expected_output: None,
                        risk: PlanTaskRisk::Low,
                        sort_order: 0,
                        status: PlanTaskStatus::Running,
                    },
                    NewPlanTask {
                        thread_stage_id: None,
                        assistant_id: Some(&assistant.id),
                        agent_participant_id: None,
                        target_agent: Agent::Codex,
                        stage_snapshot_json: None,
                        assistant_snapshot_json: Some(r#"{"assistant":"planner-v2"}"#),
                        agent_snapshot_json: r#"{"agent":"codex-v2"}"#,
                        title: "Step 2",
                        prompt: "Step two",
                        expected_output: None,
                        risk: PlanTaskRisk::Low,
                        sort_order: 1,
                        status: PlanTaskStatus::Planned,
                    },
                ],
            })
            .unwrap();
        assert_eq!(sequential.round_index, 1);
        assert_eq!(sequential.mode, PlanRoundMode::Sequential);
        assert_eq!(sequential.status, PlanRoundStatus::Running);
        assert_eq!(sequential.tasks[0].status, PlanTaskStatus::Running);
        assert_eq!(sequential.tasks[1].status, PlanTaskStatus::Planned);

        let concurrent_start_error = store
            .update_plan_task_status(
                &sequential.tasks[1].id,
                PlanTaskStatusPatch {
                    status: PlanTaskStatus::Running,
                    result_summary: None,
                    error: None,
                },
            )
            .unwrap_err()
            .to_string();
        assert!(concurrent_start_error.contains("running task"));

        let sequential = store
            .complete_plan_task_and_start_next(
                &sequential.tasks[0].id,
                PlanTaskStatusPatch {
                    status: PlanTaskStatus::Completed,
                    result_summary: Some(Some("step one done")),
                    error: Some(None),
                },
            )
            .unwrap();
        assert_eq!(sequential.status, PlanRoundStatus::Running);
        assert_eq!(sequential.tasks[0].status, PlanTaskStatus::Completed);
        assert_eq!(
            sequential.tasks[0].result_summary.as_deref(),
            Some("step one done")
        );
        assert_eq!(sequential.tasks[1].status, PlanTaskStatus::Running);

        let sequential = store
            .complete_plan_task_and_start_next(
                &sequential.tasks[1].id,
                PlanTaskStatusPatch {
                    status: PlanTaskStatus::Completed,
                    result_summary: Some(Some("step two done")),
                    error: Some(None),
                },
            )
            .unwrap();
        assert_eq!(sequential.status, PlanRoundStatus::Completed);
        assert!(sequential
            .tasks
            .iter()
            .all(|task| task.status == PlanTaskStatus::Completed));

        let listed = store.list_plan_rounds(&thread.id).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|round| round.round_index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );

        store.delete_thread(&thread.id).unwrap();
        let conn = store.conn.lock().unwrap();
        for (table, id) in [
            ("thread_plan_rounds", parallel.id.as_str()),
            ("thread_plan_tasks", parallel.tasks[0].id.as_str()),
            ("thread_plan_task_sessions", parallel.tasks[0].id.as_str()),
        ] {
            let count: i64 = if table == "thread_plan_task_sessions" {
                conn.query_row(
                    "SELECT count(*) FROM thread_plan_task_sessions WHERE task_id = ?",
                    params![id],
                    |row| row.get(0),
                )
                .unwrap()
            } else {
                conn.query_row(
                    &format!("SELECT count(*) FROM {table} WHERE id = ?"),
                    params![id],
                    |row| row.get(0),
                )
                .unwrap()
            };
            assert_eq!(count, 0, "{table} should cascade");
        }
        drop(conn);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn thread_replay_aggregates_and_dedupes_session_sources() {
        let path = unique_db("sessio-thread-replay");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-replay-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "thread-replay",
                "code".to_string(),
                None,
            )
            .unwrap();
        let assistant = store
            .create_assistant(NewAssistant {
                name: "Replay Assistant",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Keep traceable notes"),
                color: Some("#22c55e"),
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        let thread = store
            .create_thread_with_options(
                &project.id,
                "Replay everything",
                None,
                ThreadKind::Teamwork,
                std::slice::from_ref(&assistant.id),
                &[],
            )
            .unwrap();
        let stage_template = store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        let thread_stage = store
            .add_thread_stage(
                &thread.id,
                &stage_template.id,
                std::slice::from_ref(&assistant.id),
            )
            .unwrap();

        let direct_session = SessionInfo {
            id: "direct-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 2,
            rename_title: None,
            title: Some("Direct thread chat".to_string()),
            first_user_message: Some("Thread-level note".to_string()),
            file_path: Path::new(&project.path)
                .join("direct-session.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        let stage_task_session = SessionInfo {
            id: "stage-task-session".to_string(),
            started_at: Some(30),
            updated_at: Some(40),
            message_count: 4,
            title: Some("Stage and task work".to_string()),
            first_user_message: Some("Do stage work".to_string()),
            file_path: Path::new(&project.path)
                .join("stage-task-session.jsonl")
                .to_string_lossy()
                .to_string(),
            ..direct_session.clone()
        };
        let internal_session = SessionInfo {
            id: "planner-session".to_string(),
            agent: Agent::Pi,
            started_at: Some(50),
            updated_at: Some(60),
            message_count: 1,
            title: Some("Planner trace".to_string()),
            first_user_message: Some("Plan next round".to_string()),
            file_path: Path::new(&project.path)
                .join("planner-session.jsonl")
                .to_string_lossy()
                .to_string(),
            ..direct_session.clone()
        };
        for session in [&direct_session, &stage_task_session, &internal_session] {
            store.upsert_session(&session.file_path, session).unwrap();
        }
        store
            .link_thread_session(&thread.id, Agent::Codex, &direct_session.id)
            .unwrap();
        store
            .link_stage_session(&thread_stage.id, Agent::Codex, &stage_task_session.id)
            .unwrap();

        let round = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("Replay round"),
                mode: PlanRoundMode::Parallel,
                source: PlanRoundSource::Astra,
                status: PlanRoundStatus::Planned,
                tasks: vec![NewPlanTask {
                    thread_stage_id: Some(&thread_stage.id),
                    assistant_id: Some(&assistant.id),
                    agent_participant_id: None,
                    target_agent: Agent::Codex,
                    stage_snapshot_json: Some(r#"{"stage":"research"}"#),
                    assistant_snapshot_json: Some(r#"{"assistant":"replay"}"#),
                    agent_snapshot_json: r#"{"agent":"codex"}"#,
                    title: "Replay task",
                    prompt: "Do traceable work",
                    expected_output: Some("Trace"),
                    risk: PlanTaskRisk::Low,
                    sort_order: 0,
                    status: PlanTaskStatus::Running,
                }],
            })
            .unwrap();
        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: &stage_task_session.id,
                role: PlanTaskSessionRole::Runtime,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();

        store
            .upsert_astra_run(&AstraRunRecord {
                run_id: "replay-run".to_string(),
                thread_id: thread.id.clone(),
                project_id: project.id.clone(),
                project_path: project.path.clone(),
                status: "completed".to_string(),
                mode: "auto".to_string(),
                planner_backend: Some("runtime_agent_pi".to_string()),
                round_index: Some(0),
                round_limit: 3,
                terminal_reason: None,
                last_error_code: None,
                last_error_message: None,
                internal_planner_sessions: vec![AstraRunSessionRecord {
                    run_id: "replay-run".to_string(),
                    agent: Agent::Pi,
                    session_id: "planner-session".to_string(),
                    role: PlanTaskSessionRole::Planner,
                    sort_order: 0,
                    created_at: 70,
                    updated_at: 80,
                }],
                run_diagnostics_json: "[]".to_string(),
                error: None,
                created_at: 70,
                updated_at: 80,
            })
            .unwrap();

        let replay = store.get_thread_replay(&thread.id).unwrap();
        assert_eq!(replay.thread_id, thread.id);
        assert_eq!(replay.kind, ThreadKind::Teamwork);
        assert_eq!(replay.sessions.len(), 3);

        let direct = replay
            .sessions
            .iter()
            .find(|session| session.session_id == direct_session.id)
            .unwrap();
        assert_eq!(direct.agent, Agent::Codex);
        assert_eq!(direct.sources.len(), 1);
        assert_eq!(
            direct.sources[0].kind,
            ThreadReplaySessionSourceKind::Thread
        );
        assert_eq!(
            direct.session.as_ref().unwrap().title.as_deref(),
            Some("Direct thread chat")
        );

        let stage_task = replay
            .sessions
            .iter()
            .find(|session| session.session_id == stage_task_session.id)
            .unwrap();
        assert_eq!(stage_task.agent, Agent::Codex);
        assert_eq!(stage_task.sources.len(), 2);
        assert!(stage_task
            .sources
            .iter()
            .any(|source| source.kind == ThreadReplaySessionSourceKind::Stage
                && source.stage_id.as_deref() == Some(thread_stage.id.as_str())));
        assert!(stage_task.sources.iter().any(|source| source.kind
            == ThreadReplaySessionSourceKind::PlanTask
            && source.plan_task_id.as_deref() == Some(round.tasks[0].id.as_str())
            && source.role == Some(PlanTaskSessionRole::Runtime)
            && source.stage_snapshot_json.as_deref() == Some(r#"{"stage":"research"}"#)
            && source.assistant_snapshot_json.as_deref() == Some(r#"{"assistant":"replay"}"#)
            && source.agent_snapshot_json.as_deref() == Some(r#"{"agent":"codex"}"#)));

        let internal = replay
            .sessions
            .iter()
            .find(|session| session.session_id == internal_session.id)
            .unwrap();
        assert_eq!(internal.agent, Agent::Pi);
        assert_eq!(internal.sources.len(), 1);
        assert_eq!(
            internal.sources[0].kind,
            ThreadReplaySessionSourceKind::AstraInternal
        );
        assert_eq!(
            internal.sources[0].astra_run_id.as_deref(),
            Some("replay-run")
        );
        assert_eq!(internal.sources[0].role, Some(PlanTaskSessionRole::Planner));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn thread_index_lists_thread_without_sessions() {
        let path = unique_db("sessio-thread-index-empty");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-index-empty-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "thread-index-empty",
                "code".to_string(),
                None,
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Show thread entry", None)
            .unwrap();

        let items = store.list_thread_index(Some(&project.id)).unwrap();
        let item = items
            .iter()
            .find(|item| item.thread_id == thread.id)
            .unwrap();
        assert_eq!(item.goal, thread.goal);
        assert_eq!(item.project_id, project.id);
        assert_eq!(item.kind, thread.kind);
        assert!(item.session_keys.is_empty());
        assert!(item.time >= thread.updated_at.max(thread.created_at));
        assert!(store
            .list_thread_index(None)
            .unwrap()
            .iter()
            .any(|item| item.thread_id == thread.id));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn thread_index_aggregates_session_keys_and_activity_time() {
        let path = unique_db("sessio-thread-index-sources");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-index-sources-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "thread-index-sources",
                "code".to_string(),
                None,
            )
            .unwrap();
        let assistant = store
            .create_assistant(NewAssistant {
                name: "Index Assistant",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Aggregate every source", None)
            .unwrap();
        let stage_template = store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let thread_stage = store
            .add_thread_stage(
                &thread.id,
                &stage_template.id,
                std::slice::from_ref(&assistant.id),
            )
            .unwrap();

        // A linked session whose own updated_at is far ahead of every
        // thread/link timestamp: the index time must follow live session
        // activity, not just link-table timestamps.
        let session_activity_time = thread.updated_at + 250_000;
        let direct_session = SessionInfo {
            id: "direct-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(10),
            updated_at: Some(session_activity_time),
            message_count: 2,
            rename_title: None,
            title: Some("Direct thread chat".to_string()),
            first_user_message: Some("Thread note".to_string()),
            file_path: Path::new(&project.path)
                .join("direct.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        let stage_runtime_session = SessionInfo {
            id: "stage-runtime-session".to_string(),
            started_at: Some(30),
            updated_at: Some(40),
            message_count: 4,
            title: Some("Stage runtime".to_string()),
            first_user_message: Some("Stage note".to_string()),
            file_path: Path::new(&project.path)
                .join("stage-runtime.jsonl")
                .to_string_lossy()
                .to_string(),
            ..direct_session.clone()
        };
        let planner_session = SessionInfo {
            id: "planner-session".to_string(),
            agent: Agent::Pi,
            started_at: Some(50),
            updated_at: Some(60),
            message_count: 1,
            title: Some("Planner trace".to_string()),
            first_user_message: Some("Plan note".to_string()),
            file_path: Path::new(&project.path)
                .join("planner.jsonl")
                .to_string_lossy()
                .to_string(),
            ..direct_session.clone()
        };
        for session in [&direct_session, &stage_runtime_session, &planner_session] {
            store.upsert_session(&session.file_path, session).unwrap();
        }
        store
            .link_thread_session(&thread.id, Agent::Codex, &direct_session.id)
            .unwrap();
        store
            .link_stage_session(&thread_stage.id, Agent::Codex, &stage_runtime_session.id)
            .unwrap();

        let round = store
            .create_plan_round(NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("Index round"),
                mode: PlanRoundMode::Parallel,
                source: PlanRoundSource::Astra,
                status: PlanRoundStatus::Running,
                tasks: vec![NewPlanTask {
                    thread_stage_id: Some(&thread_stage.id),
                    assistant_id: Some(&assistant.id),
                    agent_participant_id: None,
                    target_agent: Agent::Codex,
                    stage_snapshot_json: None,
                    assistant_snapshot_json: None,
                    agent_snapshot_json: r#"{"agent":"codex"}"#,
                    title: "Runtime task",
                    prompt: "Do runtime work",
                    expected_output: None,
                    risk: PlanTaskRisk::Low,
                    sort_order: 0,
                    status: PlanTaskStatus::Running,
                }],
            })
            .unwrap();
        // Linking a second Codex runtime session supersedes the first one.
        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: "superseded-runtime-session",
                role: PlanTaskSessionRole::Runtime,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();
        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: &stage_runtime_session.id,
                role: PlanTaskSessionRole::Runtime,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();
        store
            .link_plan_task_session(NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Pi,
                session_id: "missing-runtime-session",
                role: PlanTaskSessionRole::Runtime,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();

        store
            .upsert_astra_run(&AstraRunRecord {
                run_id: "index-run".to_string(),
                thread_id: thread.id.clone(),
                project_id: project.id.clone(),
                project_path: project.path.clone(),
                status: "completed".to_string(),
                mode: "auto".to_string(),
                planner_backend: Some("runtime_agent_pi".to_string()),
                round_index: Some(0),
                round_limit: 3,
                terminal_reason: None,
                last_error_code: None,
                last_error_message: None,
                internal_planner_sessions: vec![
                    AstraRunSessionRecord {
                        run_id: "index-run".to_string(),
                        agent: Agent::Pi,
                        session_id: planner_session.id.clone(),
                        role: PlanTaskSessionRole::Planner,
                        sort_order: 0,
                        created_at: 70,
                        updated_at: 80,
                    },
                    AstraRunSessionRecord {
                        run_id: "index-run".to_string(),
                        agent: Agent::Pi,
                        session_id: "missing-planner-session".to_string(),
                        role: PlanTaskSessionRole::Planner,
                        sort_order: 1,
                        created_at: 81,
                        updated_at: 82,
                    },
                ],
                run_diagnostics_json: "[]".to_string(),
                error: None,
                created_at: 70,
                updated_at: 82,
            })
            .unwrap();

        let items = store.list_thread_index(Some(&project.id)).unwrap();
        let item = items
            .iter()
            .find(|item| item.thread_id == thread.id)
            .unwrap();
        assert_eq!(item.goal, thread.goal);
        assert_eq!(
            item.session_keys.iter().cloned().collect::<HashSet<_>>(),
            HashSet::from([
                format!("{}:direct-session", Agent::Codex.as_str()),
                format!("{}:stage-runtime-session", Agent::Codex.as_str()),
                format!("{}:missing-runtime-session", Agent::Pi.as_str()),
                format!("{}:planner-session", Agent::Pi.as_str()),
                format!("{}:missing-planner-session", Agent::Pi.as_str()),
            ])
        );
        assert_eq!(item.time, session_activity_time);

        // Archiving the project drops its threads from the index without
        // erroring, for both the scoped and the global listing.
        store.archive_project(&project.id).unwrap();
        assert!(store
            .list_thread_index(Some(&project.id))
            .unwrap()
            .is_empty());
        assert!(store
            .list_thread_index(None)
            .unwrap()
            .iter()
            .all(|item| item.thread_id != thread.id));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn thread_kind_and_thread_assistants_roundtrip() {
        let path = unique_db("sessio-thread-kind-assistants");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-kind-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "thread-kinds",
                "code".to_string(),
                None,
            )
            .unwrap();
        let builder = store
            .create_assistant(NewAssistant {
                name: "Builder",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Build carefully"),
                color: Some("#22c55e"),
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        let reviewer = store
            .create_assistant(NewAssistant {
                name: "Reviewer",
                agent: AssistantAgentInfo {
                    id: "claude".to_string(),
                    name: "Claude".to_string(),
                    model: "claude-sonnet-4-5".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "high".to_string(),
                },
                system_prompt: None,
                color: Some("#60a5fa"),
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();

        let legacy = store
            .create_thread(&project.id, "Legacy process", None)
            .unwrap();
        assert_eq!(legacy.kind, ThreadKind::Process);
        assert!(legacy.assistants.is_empty());
        let persisted_kind: String = {
            let conn = store.conn.lock().unwrap();
            conn.query_row(
                "SELECT kind FROM threads WHERE id = ?",
                params![legacy.id.as_str()],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(persisted_kind, "process");

        let assistant_ids = vec![builder.id.clone(), reviewer.id.clone()];
        let teamwork = store
            .create_thread_with_options(
                &project.id,
                "Teamwork lane",
                Some("shared context"),
                ThreadKind::Teamwork,
                &assistant_ids,
                &[],
            )
            .unwrap();
        assert_eq!(teamwork.kind, ThreadKind::Teamwork);
        assert_eq!(
            teamwork
                .assistants
                .iter()
                .map(|assistant| assistant.assistant_id.as_str())
                .collect::<Vec<_>>(),
            vec![builder.id.as_str(), reviewer.id.as_str()]
        );
        assert_eq!(teamwork.assistants[0].name, "Builder");
        assert_eq!(teamwork.assistants[0].agent.id, "codex");
        assert_eq!(
            teamwork.assistants[0].system_prompt.as_deref(),
            Some("Build carefully")
        );
        assert_eq!(teamwork.assistants[1].agent.id, "claude");

        let brainstorm_participants = vec![
            ThreadAgentInfo {
                participant_id: String::new(),
                agent: Agent::Codex,
                model: "gpt-5.3-codex".to_string(),
                effort: "medium".to_string(),
                permission_mode: "read-only".to_string(),
                order: 0,
                created_at: 0,
                updated_at: 0,
            },
            ThreadAgentInfo {
                participant_id: "custom-participant".to_string(),
                agent: Agent::Claude,
                model: "claude-sonnet-4-5".to_string(),
                effort: "high".to_string(),
                permission_mode: "workspace-write".to_string(),
                order: 1,
                created_at: 0,
                updated_at: 0,
            },
        ];
        let brainstorm = store
            .create_thread_with_options(
                &project.id,
                "Brainstorm lane",
                None,
                ThreadKind::Brainstorm,
                &[],
                &brainstorm_participants,
            )
            .unwrap();
        assert_eq!(brainstorm.kind, ThreadKind::Brainstorm);
        assert!(brainstorm.assistants.is_empty());
        assert_eq!(brainstorm.agent_participants.len(), 2);
        assert_eq!(brainstorm.agent_participants[0].agent, Agent::Codex);
        assert_eq!(brainstorm.agent_participants[0].model, "gpt-5.3-codex");
        assert_eq!(
            brainstorm.agent_participants[1].participant_id,
            "custom-participant"
        );

        let debate = store
            .create_thread_with_options(
                &project.id,
                "Debate lane",
                None,
                ThreadKind::Debate,
                &[],
                &brainstorm_participants[0..1],
            )
            .unwrap();
        assert_eq!(debate.kind, ThreadKind::Debate);
        assert!(debate.assistants.is_empty());
        assert_eq!(debate.agent_participants.len(), 1);
        assert_eq!(debate.agent_participants[0].agent, Agent::Codex);

        let empty_runtime_option_participants = vec![ThreadAgentInfo {
            participant_id: String::new(),
            agent: Agent::Codex,
            model: "gpt-5.3-codex".to_string(),
            effort: String::new(),
            permission_mode: String::new(),
            order: 0,
            created_at: 0,
            updated_at: 0,
        }];
        let empty_runtime_option_thread = store
            .create_thread_with_options(
                &project.id,
                "Agent participant with defaults",
                None,
                ThreadKind::Brainstorm,
                &[],
                &empty_runtime_option_participants,
            )
            .unwrap();
        assert_eq!(empty_runtime_option_thread.agent_participants.len(), 1);
        assert_eq!(empty_runtime_option_thread.agent_participants[0].effort, "");
        assert_eq!(
            empty_runtime_option_thread.agent_participants[0].permission_mode,
            ""
        );

        let listed = store.list_threads(&project.id).unwrap();
        assert!(listed.iter().any(|thread| thread.id == legacy.id
            && thread.kind == ThreadKind::Process
            && thread.assistants.is_empty()));
        assert!(listed.iter().any(|thread| thread.id == teamwork.id
            && thread.kind == ThreadKind::Teamwork
            && thread.assistants.len() == 2));
        assert!(listed.iter().any(|thread| thread.id == brainstorm.id
            && thread.kind == ThreadKind::Brainstorm
            && thread.assistants.is_empty()
            && thread.agent_participants.len() == 2));
        assert!(listed.iter().any(|thread| thread.id == debate.id
            && thread.kind == ThreadKind::Debate
            && thread.assistants.is_empty()
            && thread.agent_participants.len() == 1));

        let reordered = vec![reviewer.id.clone(), builder.id.clone()];
        let updated = store
            .update_thread_with_options(
                &teamwork.id,
                Some("Teamwork lane updated"),
                Some(Some("reordered")),
                None,
                Some(ThreadKind::Teamwork),
                Some(&reordered),
                None,
            )
            .unwrap();
        assert_eq!(updated.goal, "Teamwork lane updated");
        assert_eq!(updated.description.as_deref(), Some("reordered"));
        assert_eq!(
            updated
                .assistants
                .iter()
                .map(|assistant| assistant.assistant_id.as_str())
                .collect::<Vec<_>>(),
            vec![reviewer.id.as_str(), builder.id.as_str()]
        );
        assert_eq!(updated.assistants[0].order, 0);
        assert_eq!(updated.assistants[1].order, 1);

        let updated_participants = vec![ThreadAgentInfo {
            participant_id: "updated-participant".to_string(),
            agent: Agent::Claude,
            model: "claude-opus-4-8".to_string(),
            effort: "high".to_string(),
            permission_mode: "default".to_string(),
            order: 0,
            created_at: 0,
            updated_at: 0,
        }];
        let updated_brainstorm = store
            .update_thread_with_options(
                &brainstorm.id,
                None,
                None,
                None,
                Some(ThreadKind::Brainstorm),
                None,
                Some(&updated_participants),
            )
            .unwrap();
        assert!(updated_brainstorm.assistants.is_empty());
        assert_eq!(updated_brainstorm.agent_participants.len(), 1);
        assert_eq!(
            updated_brainstorm.agent_participants[0].participant_id,
            "updated-participant"
        );

        let disable_error = store
            .update_assistant(&builder.id, None, None, None, None, None, None, Some(false))
            .unwrap_err()
            .to_string();
        assert!(disable_error.contains("thread assistant binding(s)"));
        assert!(disable_error.contains("thread \"Teamwork lane updated\""));
        assert!(store
            .delete_assistant(&builder.id)
            .unwrap_err()
            .to_string()
            .contains("stages or threads"));

        store.delete_thread(&updated.id).unwrap();
        let binding_count: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM thread_assistants WHERE thread_id = ?",
                params![updated.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(binding_count, 0);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn list_sessions_by_refs_returns_best_requested_rows_only() {
        let path = unique_db("sessio-list-sessions-by-refs");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-list-sessions-by-refs-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "list-sessions-by-refs",
                "code".to_string(),
                None,
            )
            .unwrap();
        let placeholder = SessionInfo {
            id: "shared-session".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 0,
            rename_title: Some("Placeholder".to_string()),
            title: None,
            first_user_message: Some("placeholder".to_string()),
            file_path: String::new(),
            file_size: 0,
            partial: true,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session("", &placeholder).unwrap();

        let real_path = parent.join("shared-session.jsonl");
        std::fs::write(&real_path, "{}").unwrap();
        let indexed = SessionInfo {
            file_path: real_path.to_string_lossy().to_string(),
            file_size: 42,
            partial: false,
            message_count: 5,
            title: Some("Indexed".to_string()),
            updated_at: Some(30),
            ..placeholder.clone()
        };
        store.upsert_session(&indexed.file_path, &indexed).unwrap();

        let other_path = parent.join("other-session.jsonl");
        std::fs::write(&other_path, "{}").unwrap();
        let other = SessionInfo {
            id: "other-session".to_string(),
            file_path: other_path.to_string_lossy().to_string(),
            title: Some("Other".to_string()),
            ..indexed.clone()
        };
        store.upsert_session(&other.file_path, &other).unwrap();

        let subagent = SubagentInfo {
            id: "sub-1".to_string(),
            agent_type: Some("reviewer".to_string()),
            description: Some("Attached subagent".to_string()),
            started_at: Some(31),
            updated_at: Some(32),
            message_count: 2,
            first_user_message: Some("subagent".to_string()),
            file_path: parent.join("sub-1.jsonl").to_string_lossy().to_string(),
            file_size: 9,
            partial: false,
            available: true,
        };
        store
            .upsert_subagent(Agent::Codex, &indexed.file_path, &indexed.id, &subagent)
            .unwrap();

        let sessions = store
            .list_sessions_by_refs(&[
                SessionRef {
                    agent: Agent::Codex,
                    session_id: "shared-session",
                },
                SessionRef {
                    agent: Agent::Pi,
                    session_id: "missing-session",
                },
            ])
            .unwrap();

        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.id, "shared-session");
        assert_eq!(session.file_path, indexed.file_path);
        assert!(!session.partial);
        assert_eq!(session.title.as_deref(), Some("Indexed"));
        assert_eq!(session.subagents.len(), 1);
        assert_eq!(session.subagents[0].id, "sub-1");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn project_threads_assistants_stages_and_session_links_roundtrip() {
        let path = unique_db("sessio-thread-stage-links");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let agents = store.list_agents().unwrap();
        assert_eq!(
            agents
                .iter()
                .map(|agent| agent.id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex", "claude", "opencode", "pi"]
        );
        let pi_agent = agents.iter().find(|agent| agent.id == "pi").unwrap();
        assert!(!pi_agent.enabled);
        assert_eq!(pi_agent.transport, RuntimeTransportKind::PiRpc);
        assert_eq!(pi_agent.commands.session, vec!["pi --mode rpc".to_string()]);
        let codex_agent = agents.iter().find(|agent| agent.id == "codex").unwrap();
        assert_eq!(codex_agent.icon.as_deref(), Some("codex"));
        assert_eq!(codex_agent.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(codex_agent.commands.session.len(), 1);
        assert_eq!(
            codex_agent.commands.version.first().map(String::as_str),
            Some("npm view @agentclientprotocol/codex-acp version")
        );
        assert_eq!(codex_agent.effort.as_deref(), Some("high"));
        assert!(codex_agent
            .efforts
            .iter()
            .any(|option| option.value == "xhigh"));
        let claude_agent = agents.iter().find(|agent| agent.id == "claude").unwrap();
        assert_eq!(
            claude_agent.commands.version.first().map(String::as_str),
            Some("npm view @agentclientprotocol/claude-agent-acp version")
        );
        assert_eq!(claude_agent.effort.as_deref(), Some("high"));
        assert!(claude_agent
            .efforts
            .iter()
            .any(|option| option.value == "max"));
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(
                &parent.to_string_lossy(),
                "threaded",
                "code".to_string(),
                None,
            )
            .unwrap();
        let assistant = store
            .create_assistant(NewAssistant {
                name: "Builder",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: Some("Build carefully"),
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        assert_eq!(assistant.project_id.as_deref(), Some(project.id.as_str()));
        assert_eq!(assistant.agent.id, "codex");
        assert_eq!(assistant.agent.name, "Codex");
        assert_eq!(assistant.agent.model, "gpt-5.3-codex");
        assert_eq!(assistant.agent.mode, "read-only");
        assert_eq!(assistant.agent.effort, "medium");
        let project_assistants = store.list_assistants(Some(&project.id)).unwrap();
        assert_eq!(project_assistants.len(), 5);
        assert!(project_assistants
            .iter()
            .all(|item| item.project_id.as_deref() == Some(project.id.as_str())));
        assert_eq!(
            project_assistants
                .iter()
                .filter(|item| item.assistant_type == AssistantType::Builtin)
                .count(),
            4
        );
        assert!(project_assistants
            .iter()
            .any(|item| item.id == assistant.id));
        let reviewer = store
            .create_assistant(NewAssistant {
                name: "Reviewer",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();

        let thread = store
            .create_thread(&project.id, "Ship thread process", Some("first pass"))
            .unwrap();
        assert_eq!(thread.project_id, project.id);
        assert!(thread.stage_id.is_none());

        let builtin_stages = store.list_project_stages(&project.id).unwrap();
        assert_eq!(builtin_stages.len(), 6);
        let research_option = builtin_stages
            .iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap()
            .clone();
        assert!(research_option
            .description
            .as_deref()
            .unwrap()
            .contains("technical context"));
        assert!(builtin_stages
            .iter()
            .any(|stage| stage.kind == Some(StageType::Develop)));
        assert!(builtin_stages
            .iter()
            .all(|stage| stage.kind != Some(StageType::Build)));
        assert_eq!(research_option.assistants.len(), 1);
        let builtin_research_assistant_id = research_option.assistants[0].assistant_id.clone();
        assert_eq!(
            builtin_research_assistant_id,
            stable_project_builtin_assistant_id(
                &project.id,
                &stable_process_template_builtin_assistant_id("code", "assistant-builtin-research")
            )
        );
        let default_stage = store
            .update_project_stage_assistants(
                &research_option.id,
                std::slice::from_ref(&assistant.id),
            )
            .unwrap();
        assert_eq!(default_stage.assistants.len(), 1);
        assert_eq!(default_stage.assistants[0].assistant_id, assistant.id);
        let assistant_stage_binding_error = store
            .update_assistant(
                &assistant.id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
            )
            .unwrap_err()
            .to_string();
        assert!(assistant_stage_binding_error.contains("project stage assistant binding(s)"));
        assert!(assistant_stage_binding_error.contains("stage \"research\""));
        assert!(!assistant_stage_binding_error.contains("process template \""));
        let default_thread = store
            .create_thread(&project.id, "Default stage assistants", None)
            .unwrap();
        let default_research = store
            .add_thread_stage(&default_thread.id, &research_option.id, &[])
            .unwrap();
        assert_eq!(default_research.assistant_ids, vec![assistant.id.clone()]);
        assert_eq!(default_research.assistants[0].agent.id, "codex");
        assert!(store
            .list_threads(&project.id)
            .unwrap()
            .into_iter()
            .find(|item| item.id == default_thread.id)
            .unwrap()
            .stage_id
            .is_none());
        let assistant_disable_error = store
            .update_assistant(
                &assistant.id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
            )
            .unwrap_err()
            .to_string();
        assert!(assistant_disable_error.contains("project stage assistant binding(s)"));
        assert!(assistant_disable_error.contains("thread stage assistant binding(s)"));
        assert!(!assistant_disable_error.contains("process template \""));
        assert!(assistant_disable_error.contains("thread \"Default stage assistants\""));
        let custom_process_template = store
            .create_process_template("Custom Flow", Some("Custom process template description"))
            .unwrap();
        assert_eq!(
            custom_process_template.process_template_type,
            ProcessTemplateType::Custom
        );
        assert_eq!(
            custom_process_template.description.as_deref(),
            Some("Custom process template description")
        );
        let renamed_process_template = store
            .update_process_template(
                &custom_process_template.id,
                Some("Custom Flow Prime"),
                Some(Some("Updated custom process template description")),
            )
            .unwrap();
        assert_eq!(renamed_process_template.name, "Custom Flow Prime");
        assert_eq!(
            renamed_process_template.description.as_deref(),
            Some("Updated custom process template description")
        );
        let process_template_stage = store
            .create_project_stage(
                "",
                Some(custom_process_template.id.clone()),
                "Process Custom Stage",
                Some("Template stage"),
                None,
            )
            .unwrap();
        assert_eq!(
            process_template_stage.process_template_id.as_deref(),
            Some(custom_process_template.id.as_str())
        );
        assert_eq!(process_template_stage.project_id, None);
        let process_template_stages = store
            .list_process_template_stages(&custom_process_template.id)
            .unwrap();
        assert_eq!(process_template_stages.len(), 1);
        assert_eq!(process_template_stages[0].id, process_template_stage.id);
        store
            .delete_project_stage(&process_template_stage.id)
            .unwrap();
        store
            .delete_process_template(&custom_process_template.id)
            .unwrap();
        let build_option = store
            .create_project_stage(
                &project.id,
                None,
                "Implementation",
                Some("Implementation notes"),
                None,
            )
            .unwrap();
        assert_eq!(build_option.stage_type, ProjectStageType::Custom);
        assert_eq!(build_option.name.as_deref(), Some("Implementation"));
        assert_eq!(
            build_option.description.as_deref(),
            Some("Implementation notes")
        );
        assert_eq!(store.list_project_stages(&project.id).unwrap().len(), 7);
        let human_option = builtin_stages
            .iter()
            .find(|stage| stage.kind == Some(StageType::Human))
            .unwrap()
            .clone();
        assert!(human_option.allow_empty_assistants);
        let human = store
            .add_thread_stage(&thread.id, &human_option.id, &[])
            .unwrap();
        assert!(human.assistant_ids.is_empty());
        assert!(human.allow_empty_assistants);
        assert!(store
            .list_threads(&project.id)
            .unwrap()
            .into_iter()
            .find(|item| item.id == thread.id)
            .unwrap()
            .stage_id
            .is_none());
        assert!(!build_option.allow_empty_assistants);
        assert!(store
            .add_thread_stage(&thread.id, &build_option.id, &[])
            .unwrap_err()
            .to_string()
            .contains("stage does not allow empty assistants"));
        let manual_build = store
            .update_project_stage(
                &build_option.id,
                ProjectStagePatch {
                    allow_empty_assistants: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(manual_build.allow_empty_assistants);
        let empty_build = store
            .add_thread_stage(&thread.id, &build_option.id, &[])
            .unwrap();
        assert!(empty_build.assistant_ids.is_empty());
        store.delete_thread_stage(&empty_build.id).unwrap();
        let assistant_ids = vec![assistant.id.clone(), reviewer.id.clone()];
        let research = store
            .add_thread_stage(&thread.id, &research_option.id, &assistant_ids)
            .unwrap();
        assert_eq!(research.order, 1);
        assert_eq!(research.stage_id, research_option.id);
        assert_eq!(research.assistant_ids, assistant_ids);
        let builder_only_ids = vec![assistant.id.clone()];
        let build = store
            .add_thread_stage(&thread.id, &build_option.id, &builder_only_ids)
            .unwrap();
        assert_eq!(build.order, 2);
        assert_eq!(build.assistant_ids, builder_only_ids);
        assert_eq!(build.assistants[0].agent.id, "codex");
        let build = store
            .update_thread_stage_assistant_agent(
                &build.id,
                &assistant.id,
                AssistantAgentInfo {
                    id: "claude".to_string(),
                    name: "Claude".to_string(),
                    model: "claude-sonnet-4-5".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "high".to_string(),
                },
            )
            .unwrap();
        assert_eq!(build.assistants[0].assistant_id, assistant.id);
        assert_eq!(build.assistants[0].agent.id, "claude");
        assert_eq!(build.assistants[0].agent.name, "Claude");
        assert_eq!(build.assistants[0].agent.model, "claude-sonnet-4-5");
        let unchanged_assistant = store
            .list_assistants(Some(&project.id))
            .unwrap()
            .into_iter()
            .find(|item| item.id == assistant.id)
            .unwrap();
        assert_eq!(unchanged_assistant.agent.id, "codex");
        let unchanged_research = store
            .list_threads(&project.id)
            .unwrap()
            .into_iter()
            .flat_map(|thread| thread.stages)
            .find(|stage| stage.id == research.id)
            .unwrap();
        assert_eq!(unchanged_research.assistants[0].agent.id, "codex");
        let reviewed = store
            .update_project_stage(
                &build_option.id,
                ProjectStagePatch {
                    name: Some("Review Pass"),
                    description: Some(None),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(reviewed.name.as_deref(), Some("Review Pass"));
        assert_eq!(reviewed.description, None);
        let review_option = builtin_stages
            .iter()
            .find(|stage| stage.kind == Some(StageType::Review))
            .unwrap()
            .clone();
        let review_thread = store
            .create_thread(&project.id, "Review thread process", Some("second lane"))
            .unwrap();
        let review_stage_assistant_ids = vec![assistant.id.clone(), reviewer.id.clone()];
        let review_stage = store
            .add_thread_stage(
                &review_thread.id,
                &review_option.id,
                &review_stage_assistant_ids,
            )
            .unwrap();
        assert_eq!(review_stage.thread_id, review_thread.id);
        assert_eq!(review_stage.stage_id, review_option.id);
        assert_eq!(review_stage.assistant_ids, review_stage_assistant_ids);
        assert_eq!(review_stage.assistants.len(), 2);
        assert_eq!(review_stage.assistants[0].agent.id, "codex");
        let review_stage = store
            .update_thread_stage_assistant_agent(
                &review_stage.id,
                &assistant.id,
                AssistantAgentInfo {
                    id: "pi".to_string(),
                    name: "Pi".to_string(),
                    model: "default".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "medium".to_string(),
                },
            )
            .unwrap();
        assert_eq!(review_stage.assistants[0].assistant_id, assistant.id);
        assert_eq!(review_stage.assistants[0].agent.id, "pi");
        assert_eq!(review_stage.assistants[0].agent.name, "Pi");
        assert_eq!(review_stage.assistants[1].assistant_id, reviewer.id);
        assert_eq!(review_stage.assistants[1].agent.id, "codex");
        let thread_lanes = store.list_threads(&project.id).unwrap();
        assert_eq!(thread_lanes.len(), 3);
        let build_lane = thread_lanes
            .iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        let review_lane = thread_lanes
            .iter()
            .find(|item| item.id == review_thread.id)
            .unwrap();
        assert_eq!(build_lane.stages.len(), 3);
        assert_eq!(review_lane.stages.len(), 1);
        assert_eq!(
            build_lane
                .stages
                .iter()
                .find(|stage| stage.id == build.id)
                .unwrap()
                .assistants[0]
                .agent
                .id,
            "claude"
        );
        assert_eq!(review_lane.stages[0].assistants[0].agent.id, "pi");
        assert_eq!(
            store
                .list_assistants(Some(&project.id))
                .unwrap()
                .into_iter()
                .find(|item| item.id == assistant.id)
                .unwrap()
                .agent
                .id,
            "codex"
        );
        let build = store
            .update_thread_stage(&build.id, None, Some(0), None)
            .unwrap();
        assert_eq!(build.name.as_deref(), Some("Review Pass"));
        assert_eq!(build.order, 0);
        assert_eq!(build.assistants[0].agent.id, "claude");

        let listed_threads = store.list_threads(&project.id).unwrap();
        assert_eq!(listed_threads.len(), 3);
        let listed_build_lane = listed_threads
            .iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        assert!(listed_build_lane.stage_id.is_none());
        assert_eq!(listed_build_lane.stages.len(), 3);
        assert_eq!(listed_build_lane.stages[0].id, build.id);
        assert_eq!(listed_build_lane.stages[1].id, human.id);
        assert!(listed_build_lane.stages[1].assistant_ids.is_empty());
        assert_eq!(listed_build_lane.stages[2].assistant_ids, assistant_ids);
        let reordered_ids = vec![reviewer.id.clone(), assistant.id.clone()];
        let research = store
            .update_thread_stage(&research.id, Some(&reordered_ids), None, None)
            .unwrap();
        assert_eq!(research.assistant_ids, reordered_ids);

        let stage_disable_error = store
            .update_thread_stage(&research.id, None, None, Some(false))
            .unwrap_err()
            .to_string();
        assert!(stage_disable_error.contains("thread \"Ship thread process\""));
        assert!(stage_disable_error.contains("thread \"Default stage assistants\""));
        store.delete_thread_stage(&default_research.id).unwrap();
        store.delete_thread_stage(&research.id).unwrap();
        let disabled_research = store
            .update_project_stage(
                &research.stage_id,
                ProjectStagePatch {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!disabled_research.enabled);
        assert!(store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .any(|stage| stage.id == research.stage_id && !stage.enabled));
        assert!(store
            .list_process_template_stages(&project.process_template_id)
            .unwrap()
            .into_iter()
            .any(|stage| stage.kind == Some(StageType::Research) && stage.project_id.is_none()));
        assert!(store
            .add_thread_stage(&thread.id, &research.stage_id, &assistant_ids)
            .is_err());
        let enabled_research = store
            .update_project_stage(
                &research.stage_id,
                ProjectStagePatch {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(enabled_research.enabled);

        let switched = store.set_thread_stage(&thread.id, &build.id).unwrap();
        assert_eq!(switched.stage_id.as_deref(), Some(build.id.as_str()));
        let research = store
            .add_thread_stage(&thread.id, &research.stage_id, &reordered_ids)
            .unwrap();
        let switched = store.set_thread_stage(&thread.id, &research.id).unwrap();
        assert_eq!(switched.stage_id.as_deref(), Some(research.id.as_str()));

        let session = SessionInfo {
            id: "session-a".to_string(),
            agent: Agent::Codex,
            forked_from_agent: None,
            forked_from_id: None,
            project_path: Some(project.path.clone()),
            project_name: Some(project.name.clone()),
            started_at: Some(10),
            updated_at: Some(20),
            message_count: 3,
            rename_title: None,
            title: Some("Build stage".to_string()),
            first_user_message: Some("Please build".to_string()),
            file_path: Path::new(&project.path)
                .join("session-a.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            origin: crate::models::SessionOrigin::Chat,
            scheduled_task_id: None,
            is_auxiliary: false,
            subagents: Vec::new(),
        };
        store.upsert_session(&session.file_path, &session).unwrap();

        let linked = store
            .link_stage_session(&build.id, Agent::Codex, &session.id)
            .unwrap();
        assert_eq!(linked.sessions.len(), 1);
        assert_eq!(linked.sessions[0].id, session.id);
        assert!(store
            .link_thread_session(&thread.id, Agent::Codex, &session.id)
            .is_err());
        assert_eq!(
            store
                .list_threads(&project.id)
                .unwrap()
                .into_iter()
                .find(|item| item.id == thread.id)
                .unwrap()
                .stage_id
                .as_deref(),
            Some(research.id.as_str())
        );

        let listed_threads = store.list_threads(&project.id).unwrap();
        let current_thread = listed_threads
            .iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        assert!(current_thread.sessions.is_empty());
        let build_stage = current_thread
            .stages
            .iter()
            .find(|stage| stage.id == build.id)
            .unwrap();
        assert_eq!(build_stage.sessions.len(), 1);

        assert_eq!(
            current_thread.stage_id.as_deref(),
            Some(research.id.as_str())
        );

        let unlinked = store
            .unlink_stage_session(&build.id, Agent::Codex, &session.id)
            .unwrap();
        assert!(unlinked.sessions.is_empty());

        let thread_linked = store
            .link_thread_session(&thread.id, Agent::Codex, &session.id)
            .unwrap();
        assert_eq!(thread_linked.sessions.len(), 1);
        assert_eq!(thread_linked.sessions[0].id, session.id);
        assert!(thread_linked
            .stages
            .iter()
            .all(|stage| stage.sessions.is_empty()));
        assert!(store
            .link_stage_session(&build.id, Agent::Codex, &session.id)
            .is_err());
        let listed_thread = store
            .list_threads(&project.id)
            .unwrap()
            .into_iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        assert_eq!(listed_thread.sessions.len(), 1);
        assert_eq!(listed_thread.sessions[0].id, session.id);
        let thread_unlinked = store
            .unlink_thread_session(&thread.id, Agent::Codex, &session.id)
            .unwrap();
        assert!(thread_unlinked.sessions.is_empty());

        let edited_thread = store
            .update_thread(
                &thread.id,
                Some("Ship edited process"),
                Some(Some("")),
                None,
            )
            .unwrap();
        assert_eq!(edited_thread.goal, "Ship edited process");
        assert_eq!(edited_thread.description, None);
        let edited_assistant = store
            .update_assistant(
                &assistant.id,
                Some("Builder Prime"),
                Some(AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Ignored".to_string(),
                    model: "gpt-5".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                }),
                Some(None),
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(edited_assistant.name, "Builder Prime");
        assert_eq!(edited_assistant.agent.name, "Codex");
        assert_eq!(edited_assistant.agent.model, "gpt-5");
        assert_eq!(edited_assistant.agent.mode, "read-only");
        assert_eq!(edited_assistant.agent.effort, "medium");
        assert_eq!(edited_assistant.system_prompt, None);

        let other_parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-other-parent");
        std::fs::create_dir(&other_parent).unwrap();
        let other_project = store
            .create_project(
                &other_parent.to_string_lossy(),
                "other",
                "code".to_string(),
                None,
            )
            .unwrap();
        let other_session = SessionInfo {
            id: "session-b".to_string(),
            project_path: Some(other_project.path.clone()),
            project_name: Some(other_project.name.clone()),
            file_path: Path::new(&other_project.path)
                .join("session-b.jsonl")
                .to_string_lossy()
                .to_string(),
            title: Some("Other project".to_string()),
            ..session
        };
        store
            .upsert_session(&other_session.file_path, &other_session)
            .unwrap();
        assert!(store
            .link_stage_session(&build.id, Agent::Codex, &other_session.id)
            .is_err());

        let other_assistant = store
            .create_assistant(NewAssistant {
                name: "Other Builder",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&other_project.id),
            })
            .unwrap();
        let other_thread = store
            .create_thread(&other_project.id, "Other thread", None)
            .unwrap();
        let other_stage_option = store
            .create_project_stage(&other_project.id, None, "Other Plan", None, None)
            .unwrap();
        let other_assistant_ids = vec![other_assistant.id.clone()];
        let other_stage = store
            .add_thread_stage(
                &other_thread.id,
                &other_stage_option.id,
                &other_assistant_ids,
            )
            .unwrap();
        assert!(store.set_thread_stage(&thread.id, &other_stage.id).is_err());
        store.delete_thread_stage(&research.id).unwrap();
        store.delete_thread_stage(&human.id).unwrap();
        let after_delete_stage = store.list_threads(&project.id).unwrap();
        let after_delete_build_lane = after_delete_stage
            .iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        assert_eq!(after_delete_build_lane.stages.len(), 1);
        assert_eq!(after_delete_build_lane.stages[0].order, 0);
        assert!(store.delete_assistant(&assistant.id).is_err());
        store.delete_thread(&review_thread.id).unwrap();
        store.delete_thread(&thread.id).unwrap();
        store.delete_thread(&default_thread.id).unwrap();
        assert!(store.list_threads(&project.id).unwrap().is_empty());
        assert!(store.delete_assistant(&assistant.id).is_err());
        let cleared_default_stage = store
            .update_project_stage_assistants(&research_option.id, &[])
            .unwrap();
        assert!(cleared_default_stage.assistants.is_empty());
        store.delete_assistant(&assistant.id).unwrap();
        store.delete_assistant(&reviewer.id).unwrap();
        let remaining_assistants = store.list_assistants(Some(&project.id)).unwrap();
        assert_eq!(remaining_assistants.len(), 4);
        assert!(remaining_assistants
            .iter()
            .any(|item| item.id == builtin_research_assistant_id));

        assert!(store
            .create_assistant(NewAssistant {
                name: "Invalid",
                agent: AssistantAgentInfo {
                    id: "missing".to_string(),
                    name: "Missing".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .is_err());
        assert!(store
            .create_assistant(NewAssistant {
                name: "Invalid builtin",
                agent: AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                selected_skill_ids: Vec::new(),
                selected_mcp_ids: Vec::new(),
                assistant_type: AssistantType::Builtin,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .is_err());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
        let _ = std::fs::remove_dir_all(&other_parent);
    }

    #[test]
    fn runtime_agent_selection_roundtrip() {
        let path = unique_db("sessio-runtime-selection");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let saved = store
            .set_last_runtime_agent_selection(
                Agent::Codex,
                Some("gpt-5.5"),
                Some("high"),
                Some("read-only"),
            )
            .unwrap();
        assert_eq!(saved.agent, Agent::Codex);
        assert_eq!(saved.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(saved.effort.as_deref(), Some("high"));
        assert_eq!(saved.permission_mode.as_deref(), Some("read-only"));

        let loaded = store.get_last_runtime_agent_selection().unwrap().unwrap();
        assert_eq!(loaded.agent, Agent::Codex);
        assert_eq!(loaded.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(loaded.effort.as_deref(), Some("high"));
        assert_eq!(loaded.permission_mode.as_deref(), Some("read-only"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn runtime_agent_session_config_roundtrip() {
        let path = unique_db("sessio-runtime-session-config");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        store
            .mark_runtime_agent_session_config_needs_refresh(Agent::Codex, "codex-acp@1.2.3")
            .unwrap();
        assert!(store
            .get_runtime_agent_session_config(Agent::Codex, "codex-acp@1.2.3")
            .unwrap()
            .is_none());

        store
            .upsert_runtime_agent_session_config(&RuntimeAgentSessionConfigRecord {
                agent: Agent::Codex,
                adapter_version: "codex-acp@1.2.3".to_string(),
                available_commands_json: r#"[{"name":"plan"}]"#.to_string(),
                config_options_json: r#"[{"id":"model"}]"#.to_string(),
                created_at: 10,
                updated_at: 11,
            })
            .unwrap();

        let loaded = store
            .get_runtime_agent_session_config(Agent::Codex, "codex-acp@1.2.3")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.available_commands_json, r#"[{"name":"plan"}]"#);
        assert_eq!(loaded.config_options_json, r#"[{"id":"model"}]"#);

        let rows = store
            .list_runtime_agent_session_configs(Agent::Codex)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].adapter_version, "codex-acp@1.2.3");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn astra_config_update_returns_after_write() {
        let path = unique_db("sessio-astra-config-update");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        assert_eq!(
            store.get_astra_config().unwrap().agent.as_deref(),
            Some("codex")
        );

        let cleanup_path = path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = store.update_astra_config(AstraConfigPatch {
                agent: Some(Some("codex")),
                model: Some(Some("gpt-5.5")),
                ..Default::default()
            });
            let _ = tx.send(result);
        });

        let updated = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("update_astra_config should not deadlock")
            .unwrap();
        assert_eq!(updated.agent.as_deref(), Some("codex"));
        assert_eq!(updated.model.as_deref(), Some("gpt-5.5"));

        let _ = std::fs::remove_file(&cleanup_path);
    }

    #[test]
    fn runtime_agent_empty_model_patch_keeps_current_model() {
        let path = unique_db("runtime-empty-model");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let pi = store
            .update_agent_preferences_by_id(
                Agent::Pi.as_str(),
                AgentPreferencesPatch {
                    model: Some(""),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(pi.model.as_deref(), None);

        let codex = store
            .update_builtin_agent_preferences(
                Agent::Codex,
                AgentPreferencesPatch {
                    model: Some(""),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(codex.model.as_deref(), Some("gpt-5.5"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn astra_provider_ids_are_assigned_by_store() {
        let path = unique_db("astra-provider-id");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let mut providers = store
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|agent| agent.id == Agent::Pi.as_str())
            .unwrap()
            .ai_providers;
        providers.push(AgentAiProviderInfo {
            id: "".to_string(),
            display_name: "Local OpenAI".to_string(),
            provider: "openai".to_string(),
            api: Some("openai-responses".to_string()),
            base_url: Some("http://127.0.0.1:15721/v1".to_string()),
            api_key: Some("ccw".to_string()),
            model: Some("gpt-5.5".to_string()),
            models: runtime_options(vec![runtime_option("gpt-5.5", "GPT 5.5")]),
            enabled: true,
            order: 1,
        });

        let astra = store
            .update_agent_preferences_by_id(
                Agent::Pi.as_str(),
                AgentPreferencesPatch {
                    ai_provider: Some(""),
                    ai_providers: Some(&providers),
                    model: Some("gpt-5.5"),
                    ..Default::default()
                },
            )
            .unwrap();
        let generated = astra
            .ai_providers
            .iter()
            .find(|provider| provider.display_name == "Local OpenAI")
            .unwrap();
        assert!(generated.id.starts_with("custom-provider-"));
        assert!(!generated.id.trim().is_empty());
        assert_eq!(astra.ai_provider.as_deref(), Some(generated.id.as_str()));
        assert!(astra
            .ai_providers
            .iter()
            .any(|provider| Some(provider.id.as_str()) == astra.ai_provider.as_deref()));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn builtin_agent_seed_does_not_overwrite_existing_preferences_on_reinit() {
        let path = unique_db("sessio-agent-seed-preserve");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        store
            .update_builtin_agent_preferences(
                Agent::Codex,
                AgentPreferencesPatch {
                    display_name: Some("Custom Codex"),
                    enabled: Some(false),
                    order: Some(99),
                    model: Some("custom-model"),
                    effort: Some("medium"),
                    permission_mode: Some("auto"),
                    ..Default::default()
                },
            )
            .unwrap();

        store.init().unwrap();

        let codex = store
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|agent| agent.id == "codex")
            .unwrap();
        assert_eq!(codex.display_name, "Custom Codex");
        assert_eq!(codex.model.as_deref(), Some("custom-model"));
        assert_eq!(codex.effort.as_deref(), Some("medium"));
        assert_eq!(codex.permission_mode.as_deref(), Some("auto"));
        assert!(!codex.enabled);
        assert_eq!(codex.order, 99);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn builtin_agent_commands_can_be_updated() {
        let path = unique_db("sessio-agent-commands-update");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let updated = store
            .update_builtin_agent_preferences(
                Agent::Codex,
                AgentPreferencesPatch {
                    commands: Some(&AgentCommandsInfo {
                        session: vec!["custom-codex --acp".to_string()],
                        version: vec!["custom-codex --version".to_string()],
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            updated.commands.session,
            vec!["custom-codex --acp".to_string()]
        );
        assert_eq!(
            updated.commands.version,
            vec!["custom-codex --version".to_string()]
        );

        let loaded = store
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|agent| agent.id == Agent::Codex.as_str())
            .unwrap();
        assert_eq!(
            loaded.commands.session,
            vec!["custom-codex --acp".to_string()]
        );
        assert_eq!(
            loaded.commands.version,
            vec!["custom-codex --version".to_string()]
        );

        let _ = std::fs::remove_file(&path);
    }
}
