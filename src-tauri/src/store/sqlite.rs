use anyhow::{Context, Result};
use rusqlite::{
    Connection, OptionalExtension, ToSql, params, params_from_iter, types::Value as SqlValue,
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
    AssistantInfo, AssistantType, AstraConfig, CanvasBlockKind, CanvasBlockRecord,
    CanvasBlockSourceType, CanvasContextAnchor, CanvasDocumentInfo, CanvasDocumentState,
    CanvasRevisionInfo, ChannelSessionInfo, IssueSeverity, IssueStatus, KanbanItem, KanbanStatus,
    PlanRoundInfo, PlanRoundMode, PlanRoundSource, PlanRoundStatus, PlanTaskInfo, PlanTaskRisk,
    PlanTaskSessionInfo, PlanTaskSessionRole, PlanTaskStatus, ProcessTemplateInfo,
    ProcessTemplateType, ProjectInfo, ProjectStageInfo, ProjectStageType,
    RuntimeAgentOptionMetadata, SessionHistoryTurn, SessionInfo, SessionOrigin, StageAssistantInfo,
    StageInfo, StageIssueInfo, StageStatus, StageType, SubagentInfo, ThreadAgentInfo,
    ThreadAssistantInfo, ThreadIndexItemInfo, ThreadInfo, ThreadKind, ThreadOrigin,
};
use crate::store::{
    AgentPreferencesPatch, AstraConfigPatch, AstraRunRecord, AstraRunSessionRecord,
    ChannelSessionRecord, IndexedSessionRecord, IndexedSubagentRecord, NewAssistant, NewPlanRound,
    NewPlanTask, NewPlanTaskSession, PlanTaskStatusPatch, ProjectStagePatch,
    RuntimeAgentCapabilityRecord, RuntimeAgentSelection, RuntimeAgentSessionConfigRecord,
    SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK, ScheduledTaskRecord, ScheduledTaskRunRecord,
    SessionHistorySnapshotRecord, SessionRef, SessionStore, ThreadWorkSnapshotRecord,
    UpsertCanvasBlockRecord,
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

const SCHEMA_SESSIONS: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    agent              TEXT NOT NULL,
    session_id         TEXT NOT NULL,
    scope              TEXT NOT NULL,
    file_path          TEXT NOT NULL,
    project_path       TEXT,
    project_name       TEXT,
    started_at         INTEGER,
    updated_at         INTEGER,
    message_count      INTEGER NOT NULL DEFAULT 0,
    rename_title       TEXT,
    title              TEXT,
    first_user_message TEXT,
    file_size          INTEGER NOT NULL DEFAULT 0,
    file_mtime         INTEGER,
    partial            INTEGER NOT NULL DEFAULT 0,
    available          INTEGER NOT NULL DEFAULT 1,
    archived           INTEGER NOT NULL DEFAULT 0,
    forked_from_agent  TEXT,
    forked_from_id     TEXT,
    origin             TEXT NOT NULL DEFAULT 'chat',
    scheduled_task_id  TEXT,
    is_auxiliary       INTEGER NOT NULL DEFAULT 0,
    last_indexed_at    INTEGER NOT NULL,
    PRIMARY KEY (agent, session_id, scope)
);

CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_agent_updated ON sessions(agent, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_project_updated ON sessions(project_path, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_scope ON sessions(scope);
CREATE INDEX IF NOT EXISTS idx_sessions_file_path ON sessions(file_path);

CREATE TABLE IF NOT EXISTS subagents (
    parent_agent       TEXT NOT NULL,
    parent_session_id  TEXT NOT NULL,
    subagent_id        TEXT NOT NULL,
    file_path          TEXT NOT NULL,
    agent_type         TEXT,
    description        TEXT,
    started_at         INTEGER,
    updated_at         INTEGER,
    message_count      INTEGER NOT NULL DEFAULT 0,
    first_user_message TEXT,
    file_size          INTEGER NOT NULL DEFAULT 0,
    file_mtime         INTEGER,
    partial            INTEGER NOT NULL DEFAULT 0,
    available          INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (parent_agent, parent_session_id, subagent_id)
);

CREATE INDEX IF NOT EXISTS idx_subagents_parent ON subagents(parent_agent, parent_session_id);
CREATE INDEX IF NOT EXISTS idx_subagents_file_path ON subagents(file_path);
"#;

const RUNTIME_SELECTION_KEY: &str = "last";

const SCHEMA_MEMORY: &str = r#"
CREATE TABLE IF NOT EXISTS memory_records (
    record_id      TEXT PRIMARY KEY,
    project_key    TEXT NOT NULL,
    canonical_hash TEXT NOT NULL,
    simhash        TEXT,
    title          TEXT NOT NULL,
    summary        TEXT,
    body           TEXT NOT NULL,
    kind           TEXT NOT NULL DEFAULT 'session',
    available      INTEGER NOT NULL DEFAULT 1,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_records_project ON memory_records(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_records_hash ON memory_records(canonical_hash);

CREATE TABLE IF NOT EXISTS memory_artifacts (
    record_id    TEXT NOT NULL,
    backend      TEXT NOT NULL,
    artifact_uri TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY(record_id, backend)
);

CREATE INDEX IF NOT EXISTS idx_memory_artifacts_backend ON memory_artifacts(backend, artifact_uri);

CREATE TABLE IF NOT EXISTS memory_sources (
    record_id    TEXT NOT NULL,
    agent       TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    file_path   TEXT NOT NULL,
    line_start  INTEGER,
    line_end    INTEGER,
    byte_start  INTEGER,
    byte_end    INTEGER,
    PRIMARY KEY(record_id, agent, session_id, file_path, line_start, line_end),
    FOREIGN KEY(record_id) REFERENCES memory_records(record_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_memory_sources_session ON memory_sources(agent, session_id);
CREATE INDEX IF NOT EXISTS idx_memory_sources_file_path ON memory_sources(file_path);

CREATE TABLE IF NOT EXISTS turn_fingerprints (
    project_key    TEXT NOT NULL,
    agent          TEXT NOT NULL,
    session_id     TEXT NOT NULL,
    turn_index     INTEGER NOT NULL,
    role           TEXT NOT NULL,
    canonical_hash TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    text_len       INTEGER NOT NULL,
    line_start     INTEGER,
    line_end       INTEGER,
    byte_start     INTEGER,
    byte_end       INTEGER,
    PRIMARY KEY(project_key, agent, session_id, turn_index)
);

CREATE INDEX IF NOT EXISTS idx_turn_fingerprints_hash ON turn_fingerprints(canonical_hash);

CREATE TABLE IF NOT EXISTS record_continuations (
    record_id                   TEXT PRIMARY KEY,
    project_key                 TEXT NOT NULL,
    candidate_agent             TEXT NOT NULL,
    candidate_session_id        TEXT NOT NULL,
    candidate_file_path         TEXT NOT NULL,
    base_agent                  TEXT NOT NULL,
    base_session_id             TEXT NOT NULL,
    base_file_path              TEXT NOT NULL,
    base_start_turn_index       INTEGER NOT NULL,
    base_start_line_start       INTEGER,
    base_start_byte_start       INTEGER,
    base_end_turn_index         INTEGER NOT NULL,
    base_end_line_end           INTEGER,
    base_end_byte_end           INTEGER,
    candidate_trim_turn_start   INTEGER NOT NULL,
    candidate_trim_line_start   INTEGER,
    candidate_trim_byte_start   INTEGER,
    updated_at                  INTEGER NOT NULL,
    FOREIGN KEY(record_id) REFERENCES memory_records(record_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_record_continuations_project ON record_continuations(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_record_continuations_candidate ON record_continuations(candidate_agent, candidate_session_id);
CREATE INDEX IF NOT EXISTS idx_record_continuations_base ON record_continuations(base_agent, base_session_id);

-- memory_jobs records per-project memory pipeline steps for diagnostics.
-- `kind` tells you which step ran: `project_build` (full project rebuild,
-- scope = project_path), `source_build` (single-source rebuild, scope =
-- source file_path), or `backend_sync` (push the project's records to the
-- backend index, scope = project_path). `scope` is interpreted via `kind`.
CREATE TABLE IF NOT EXISTS memory_jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_key TEXT NOT NULL,
    backend     TEXT NOT NULL DEFAULT 'qmd',
    scope       TEXT NOT NULL,
    kind        TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_jobs_project_status ON memory_jobs(project_key, backend, status);
"#;

const SCHEMA_APP: &str = r#"
CREATE TABLE IF NOT EXISTS runtime_agent_capabilities (
    agent                TEXT PRIMARY KEY,
    transport_kind       TEXT NOT NULL,
    detected_version     TEXT,
    protocol_version     TEXT,
    raw_initialize_response_json TEXT NOT NULL,
    raw_capabilities_json TEXT NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_agent_session_configs (
    agent                TEXT NOT NULL,
    adapter_version      TEXT NOT NULL,
    available_commands_json TEXT NOT NULL DEFAULT '[]',
    config_options_json  TEXT NOT NULL DEFAULT '[]',
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL,
    PRIMARY KEY (agent, adapter_version)
);

CREATE INDEX IF NOT EXISTS idx_runtime_agent_session_configs_agent_updated
    ON runtime_agent_session_configs(agent, updated_at DESC);

CREATE TABLE IF NOT EXISTS runtime_agent_selections (
    key             TEXT PRIMARY KEY,
    agent           TEXT NOT NULL,
    model           TEXT,
    effort          TEXT,
    permission_mode TEXT,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS process_templates (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    description TEXT,
    type       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom'))
);

CREATE INDEX IF NOT EXISTS idx_process_templates_type_name
    ON process_templates(type, name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS projects (
    id         TEXT PRIMARY KEY,
    path       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    process_template_id TEXT NOT NULL DEFAULT 'code',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived   INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(process_template_id) REFERENCES process_templates(id)
);

CREATE INDEX IF NOT EXISTS idx_projects_archived_updated ON projects(archived, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_projects_path ON projects(path);

CREATE TABLE IF NOT EXISTS kanban_items (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    title       TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'todo',
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_kanban_items_project_status_order
    ON kanban_items(project_id, status, sort_order, created_at);

CREATE TABLE IF NOT EXISTS kanban_item_sessions (
    item_id    TEXT NOT NULL,
    agent      TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(item_id, agent, session_id),
    FOREIGN KEY(item_id) REFERENCES kanban_items(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_kanban_item_sessions_item
    ON kanban_item_sessions(item_id, created_at);
CREATE INDEX IF NOT EXISTS idx_kanban_item_sessions_session
    ON kanban_item_sessions(agent, session_id);

CREATE TABLE IF NOT EXISTS session_history_snapshots (
    child_agent           TEXT NOT NULL,
    child_session_id      TEXT NOT NULL,
    ancestor_index        INTEGER NOT NULL,
    ancestor_agent        TEXT NOT NULL,
    ancestor_session_id   TEXT NOT NULL,
    history_cache_version INTEGER NOT NULL,
    created_at            INTEGER NOT NULL,
    PRIMARY KEY(child_agent, child_session_id, ancestor_index)
);

CREATE INDEX IF NOT EXISTS idx_session_history_snapshots_child
    ON session_history_snapshots(child_agent, child_session_id);

CREATE TABLE IF NOT EXISTS session_history_snapshot_turns (
    child_agent      TEXT NOT NULL,
    child_session_id TEXT NOT NULL,
    ancestor_index   INTEGER NOT NULL,
    turn_index       INTEGER NOT NULL,
    turn_id          TEXT NOT NULL,
    started_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    turn_json        TEXT NOT NULL,
    PRIMARY KEY(child_agent, child_session_id, ancestor_index, turn_index),
    FOREIGN KEY(child_agent, child_session_id, ancestor_index)
        REFERENCES session_history_snapshots(child_agent, child_session_id, ancestor_index)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS assistants (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    agent_json      TEXT NOT NULL,
    system_prompt   TEXT,
    color           TEXT,
    type            TEXT NOT NULL,
    process_template_id    TEXT,
    project_id      TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom')),
    FOREIGN KEY(process_template_id) REFERENCES process_templates(id) ON DELETE CASCADE,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_assistants_project
    ON assistants(process_template_id, project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS threads (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    goal        TEXT NOT NULL,
    description TEXT,
    stage_id    TEXT,
    kind        TEXT NOT NULL DEFAULT 'process' CHECK(kind IN ('process', 'teamwork', 'brainstorm', 'debate')),
    enabled     INTEGER NOT NULL DEFAULT 1,
    origin      TEXT NOT NULL DEFAULT 'manual' CHECK(origin IN ('manual', 'scheduled_task')),
    scheduled_task_id TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY(stage_id) REFERENCES thread_stages(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_threads_project_updated
    ON threads(project_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_threads_stage
    ON threads(stage_id);

CREATE TABLE IF NOT EXISTS thread_assistants (
    thread_id    TEXT NOT NULL,
    assistant_id TEXT NOT NULL,
    sort_order   INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY(thread_id, assistant_id),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
    FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_thread_assistants_assistant
    ON thread_assistants(assistant_id);

CREATE TABLE IF NOT EXISTS thread_agents (
    thread_id       TEXT NOT NULL,
    participant_id  TEXT NOT NULL,
    agent           TEXT NOT NULL,
    model           TEXT NOT NULL,
    effort          TEXT NOT NULL,
    permission_mode TEXT NOT NULL,
    sort_order      INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY(thread_id, participant_id),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_agents_thread_order
    ON thread_agents(thread_id, sort_order);

CREATE TABLE IF NOT EXISTS stages (
    id           TEXT PRIMARY KEY,
    project_id   TEXT,
    type         TEXT NOT NULL,
    process_template_id  TEXT,
    kind         TEXT,
    name         TEXT,
    description  TEXT,
    icon         TEXT,
    sort_order      INTEGER NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    allow_empty_assistants INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom')),
    CHECK(kind IS NULL OR kind IN (
        'research',
        'plan',
        'develop',
        'build',
        'writing',
        'editing',
        'review',
        'proofreading',
        'screenplay',
        'storyboard',
        'design',
        'production',
        'human',
        'done'
    )),
    CHECK((type = 'builtin' AND process_template_id IS NOT NULL AND kind IS NOT NULL AND name IS NULL)
       OR (type = 'custom' AND (process_template_id IS NOT NULL OR project_id IS NOT NULL) AND kind IS NULL AND name IS NOT NULL)),
    UNIQUE(process_template_id, project_id, sort_order),
    FOREIGN KEY(process_template_id) REFERENCES process_templates(id) ON DELETE CASCADE,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_stages_project
    ON stages(process_template_id, project_id, type, sort_order, kind, name);

CREATE TABLE IF NOT EXISTS stage_assistants (
    stage_id     TEXT NOT NULL,
    assistant_id TEXT NOT NULL,
    sort_order      INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY(stage_id, assistant_id),
    FOREIGN KEY(stage_id) REFERENCES stages(id) ON DELETE CASCADE,
    FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_stage_assistants_assistant
    ON stage_assistants(assistant_id);

CREATE TABLE IF NOT EXISTS thread_stages (
    id           TEXT PRIMARY KEY,
    thread_id    TEXT NOT NULL,
    stage_id     TEXT NOT NULL,
    sort_order      INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE(thread_id, stage_id),
    UNIQUE(thread_id, sort_order),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
    FOREIGN KEY(stage_id) REFERENCES stages(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS thread_stage_states (
    thread_stage_id TEXT PRIMARY KEY,
    status          TEXT NOT NULL DEFAULT 'not_started',
    summary         TEXT,
    outcome         TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_stages_stage
    ON thread_stages(stage_id);

CREATE TABLE IF NOT EXISTS thread_stage_assistants (
    thread_stage_id TEXT NOT NULL,
    assistant_id    TEXT NOT NULL,
    agent_json      TEXT NOT NULL,
    sort_order         INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY(thread_stage_id, assistant_id),
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE,
    FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_thread_stage_assistants_assistant
    ON thread_stage_assistants(assistant_id);

CREATE TABLE IF NOT EXISTS thread_sessions (
    thread_id  TEXT NOT NULL,
    agent      TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(thread_id, agent, session_id),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_sessions_thread
    ON thread_sessions(thread_id, created_at);
CREATE INDEX IF NOT EXISTS idx_thread_sessions_session
    ON thread_sessions(agent, session_id);

CREATE TABLE IF NOT EXISTS stage_sessions (
    thread_stage_id TEXT NOT NULL,
    agent      TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(thread_stage_id, agent, session_id),
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_stage_sessions_stage
    ON stage_sessions(thread_stage_id, created_at);
CREATE INDEX IF NOT EXISTS idx_stage_sessions_session
    ON stage_sessions(agent, session_id);

CREATE TABLE IF NOT EXISTS astra_config (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    agent            TEXT,
    model            TEXT,
    effort           TEXT,
    permission_mode  TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS agents (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    display_name     TEXT NOT NULL,
    icon             TEXT,
    ai_provider      TEXT,
    ai_providers_json TEXT NOT NULL DEFAULT '[]',
    ai_api           TEXT,
    api_base_url     TEXT,
    api_key          TEXT,
    model            TEXT,
    models_json      TEXT NOT NULL DEFAULT '{}',
    effort           TEXT,
    efforts_json     TEXT NOT NULL DEFAULT '[]',
    permission_mode  TEXT,
    permission_modes_json TEXT NOT NULL DEFAULT '[]',
    type             TEXT NOT NULL,
    enabled          INTEGER NOT NULL DEFAULT 1,
    transport        TEXT NOT NULL DEFAULT 'acp',
    commands_json    TEXT NOT NULL DEFAULT '{"session":[],"version":[]}',
    sort_order          INTEGER NOT NULL DEFAULT 0,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom'))
);

CREATE INDEX IF NOT EXISTS idx_agents_type_enabled
    ON agents(type, enabled, sort_order, display_name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS thread_work_snapshots (
    child_agent      TEXT NOT NULL,
    child_session_id TEXT NOT NULL,
    thread_id        TEXT NOT NULL,
    stage_id         TEXT,
    snapshot_json    TEXT NOT NULL,
    version          INTEGER NOT NULL,
    created_at       INTEGER NOT NULL,
    PRIMARY KEY(child_agent, child_session_id)
);

CREATE INDEX IF NOT EXISTS idx_thread_work_snapshots_thread
    ON thread_work_snapshots(thread_id);

CREATE TABLE IF NOT EXISTS thread_stage_issues (
    id               TEXT PRIMARY KEY,
    thread_stage_id  TEXT NOT NULL,
    title            TEXT NOT NULL,
    description      TEXT,
    status           TEXT NOT NULL DEFAULT 'open',
    severity         TEXT NOT NULL DEFAULT 'medium',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_stage_issues_stage
    ON thread_stage_issues(thread_stage_id);

CREATE TABLE IF NOT EXISTS astra_runs (
    run_id                     TEXT PRIMARY KEY,
    thread_id                  TEXT NOT NULL,
    project_id                 TEXT NOT NULL,
    project_path               TEXT NOT NULL,
    status                     TEXT NOT NULL,
    mode                       TEXT NOT NULL DEFAULT 'auto',
    planner_backend            TEXT,
    round_index                INTEGER,
    round_limit                INTEGER NOT NULL DEFAULT 3,
    terminal_reason            TEXT,
    last_error_code            TEXT,
    last_error_message         TEXT,
    run_diagnostics_json               TEXT NOT NULL DEFAULT '[]',
    error                      TEXT,
    created_at                 INTEGER NOT NULL,
    updated_at                 INTEGER NOT NULL,
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_astra_runs_thread_updated
    ON astra_runs(thread_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_astra_runs_thread_active
    ON astra_runs(thread_id, status);

CREATE TABLE IF NOT EXISTS astra_run_sessions (
    run_id        TEXT NOT NULL,
    agent         TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    role          TEXT NOT NULL DEFAULT 'planner',
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY(run_id, agent, session_id, role),
    CHECK(role IN ('planner')),
    FOREIGN KEY(run_id) REFERENCES astra_runs(run_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_astra_run_sessions_run
    ON astra_run_sessions(run_id, sort_order, created_at);

CREATE INDEX IF NOT EXISTS idx_astra_run_sessions_session
    ON astra_run_sessions(agent, session_id);

CREATE TABLE IF NOT EXISTS thread_plan_rounds (
    id           TEXT PRIMARY KEY,
    thread_id    TEXT NOT NULL,
    astra_run_id TEXT,
    round_index  INTEGER NOT NULL,
    summary      TEXT,
    mode         TEXT NOT NULL,
    source       TEXT NOT NULL,
    status       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE(thread_id, round_index),
    CHECK(mode IN ('parallel', 'sequential')),
    CHECK(source IN ('astra', 'manual', 'agent')),
    CHECK(status IN ('planned', 'running', 'completed', 'cancelled', 'errored')),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
    FOREIGN KEY(astra_run_id) REFERENCES astra_runs(run_id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_thread_plan_rounds_thread_index
    ON thread_plan_rounds(thread_id, round_index);
CREATE INDEX IF NOT EXISTS idx_thread_plan_rounds_thread_status
    ON thread_plan_rounds(thread_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_thread_plan_rounds_astra_run
    ON thread_plan_rounds(astra_run_id);

CREATE TABLE IF NOT EXISTS thread_plan_tasks (
    id                      TEXT PRIMARY KEY,
    round_id                TEXT NOT NULL,
    thread_stage_id         TEXT,
    assistant_id            TEXT,
    agent_participant_id    TEXT,
    target_agent            TEXT NOT NULL,
    stage_snapshot_json     TEXT,
    assistant_snapshot_json TEXT,
    agent_snapshot_json     TEXT NOT NULL,
    title                   TEXT NOT NULL,
    prompt                  TEXT NOT NULL,
    expected_output         TEXT,
    risk                    TEXT NOT NULL,
    sort_order              INTEGER NOT NULL,
    status                  TEXT NOT NULL,
    result_summary          TEXT,
    error                   TEXT,
    started_at              INTEGER,
    completed_at            INTEGER,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    CHECK(risk IN ('low', 'medium', 'high')),
    CHECK(status IN ('planned', 'running', 'completed', 'failed', 'errored', 'cancelled')),
    FOREIGN KEY(round_id) REFERENCES thread_plan_rounds(id) ON DELETE CASCADE,
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE SET NULL,
    FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_round_order
    ON thread_plan_tasks(round_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_round_status
    ON thread_plan_tasks(round_id, status, sort_order);
CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_stage
    ON thread_plan_tasks(thread_stage_id);
CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_assistant
    ON thread_plan_tasks(assistant_id);
CREATE INDEX IF NOT EXISTS idx_thread_plan_tasks_agent_participant
    ON thread_plan_tasks(agent_participant_id);

CREATE TABLE IF NOT EXISTS thread_plan_task_sessions (
    task_id       TEXT NOT NULL,
    agent         TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    role          TEXT NOT NULL,
    attempt_id    TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 1,
    superseded_at INTEGER,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY(task_id, agent, session_id, role),
    CHECK(role IN ('primary', 'delegated', 'runtime', 'planner', 'synthesis', 'cross_check', 'diagnostic')),
    FOREIGN KEY(task_id) REFERENCES thread_plan_tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_plan_task_sessions_task
    ON thread_plan_task_sessions(task_id, created_at);
CREATE INDEX IF NOT EXISTS idx_thread_plan_task_sessions_session
    ON thread_plan_task_sessions(agent, session_id);
CREATE INDEX IF NOT EXISTS idx_thread_plan_task_sessions_attempt
    ON thread_plan_task_sessions(task_id, attempt_count, created_at);

CREATE TABLE IF NOT EXISTS channel_sessions (
    platform                  TEXT NOT NULL,
    channel_id                TEXT NOT NULL,
    channel_type              TEXT,
    user_id                   TEXT,
    team_id                   TEXT,
    thread_id                 TEXT,
    display_name              TEXT,
    agent                     TEXT NOT NULL,
    agent_session_id          TEXT NOT NULL,
    sessio_runtime_session_id TEXT NOT NULL,
    workspace_path            TEXT NOT NULL,
    metadata_json             TEXT NOT NULL DEFAULT '{}',
    last_update_id            INTEGER,
    created_at                INTEGER NOT NULL,
    updated_at                INTEGER NOT NULL,
    last_activity_at          INTEGER NOT NULL,
    ended_at                  INTEGER,
    PRIMARY KEY(platform, channel_id, agent, agent_session_id)
);

CREATE INDEX IF NOT EXISTS idx_channel_sessions_channel
    ON channel_sessions(platform, channel_id, ended_at, last_activity_at DESC);

CREATE INDEX IF NOT EXISTS idx_channel_sessions_session
    ON channel_sessions(agent, agent_session_id);

CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'paused')),
    schedule_json  TEXT NOT NULL,
    target_json    TEXT NOT NULL,
    project_id     TEXT NOT NULL,
    mode           TEXT NOT NULL CHECK(mode IN ('chat', 'process', 'teamwork', 'brainstorm', 'debate')),
    sort_order     INTEGER NOT NULL DEFAULT 0,
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL,
    last_run_at_ms INTEGER,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_status_order
    ON scheduled_tasks(status, sort_order, created_at_ms);

CREATE INDEX IF NOT EXISTS idx_scheduled_tasks_project
    ON scheduled_tasks(project_id, sort_order);

CREATE TABLE IF NOT EXISTS scheduled_task_runs (
    id             TEXT PRIMARY KEY,
    task_id        TEXT NOT NULL,
    mode           TEXT NOT NULL CHECK(mode IN ('chat', 'process', 'teamwork', 'brainstorm', 'debate')),
    trigger        TEXT NOT NULL DEFAULT 'scheduled' CHECK(trigger IN ('scheduled', 'manual')),
    status         TEXT NOT NULL DEFAULT 'completed' CHECK(status IN ('running', 'completed', 'failed', 'cancelled')),
    started_at_ms  INTEGER NOT NULL,
    scheduled_for_ms INTEGER,
    completed_at_ms INTEGER,
    task_name      TEXT,
    target_json    TEXT,
    session_agent  TEXT,
    session_id     TEXT,
    agent_session_id TEXT,
    thread_id      TEXT,
    astra_run_id   TEXT,
    push_platform  TEXT,
    push_chat_id   TEXT,
    push_status    TEXT CHECK(push_status IS NULL OR push_status IN ('pending', 'summarizing', 'sent', 'failed')),
    push_summary   TEXT,
    push_error     TEXT,
    push_sent_at_ms INTEGER,
    error          TEXT,
    FOREIGN KEY(task_id) REFERENCES scheduled_tasks(id) ON DELETE CASCADE,
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_scheduled_task_runs_task_started
    ON scheduled_task_runs(task_id, started_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_scheduled_task_runs_session
    ON scheduled_task_runs(session_agent, session_id);

CREATE INDEX IF NOT EXISTS idx_scheduled_task_runs_agent_session
    ON scheduled_task_runs(session_agent, agent_session_id);

CREATE INDEX IF NOT EXISTS idx_scheduled_task_runs_thread
    ON scheduled_task_runs(thread_id);

CREATE INDEX IF NOT EXISTS idx_scheduled_task_runs_status_push
    ON scheduled_task_runs(status, push_status, started_at_ms);

CREATE TABLE IF NOT EXISTS canvases (
    id                     TEXT PRIMARY KEY,
    session_id             TEXT NOT NULL,
    title                  TEXT NOT NULL,
    current_saved_revision INTEGER,
    draft_snapshot_path    TEXT,
    draft_snapshot_hash    TEXT,
    draft_updated_at       INTEGER,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    UNIQUE(session_id)
);

CREATE TABLE IF NOT EXISTS canvas_revisions (
    id                  TEXT PRIMARY KEY,
    canvas_id           TEXT NOT NULL,
    revision            INTEGER NOT NULL,
    snapshot_path       TEXT NOT NULL,
    snapshot_hash       TEXT NOT NULL,
    snapshot_size_bytes INTEGER NOT NULL,
    source              TEXT NOT NULL,
    created_at          INTEGER NOT NULL,
    UNIQUE(canvas_id, revision),
    FOREIGN KEY(canvas_id) REFERENCES canvases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_canvas_revisions_canvas_created
    ON canvas_revisions(canvas_id, created_at DESC);

CREATE TABLE IF NOT EXISTS canvas_blocks (
    id            TEXT PRIMARY KEY,
    canvas_id     TEXT NOT NULL,
    block_id      TEXT NOT NULL,
    block_kind    TEXT NOT NULL,
    source_type   TEXT NOT NULL,
    source_key    TEXT,
    source_path   TEXT,
    metadata_json TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    UNIQUE(canvas_id, block_id),
    FOREIGN KEY(canvas_id) REFERENCES canvases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_canvas_blocks_canvas
    ON canvas_blocks(canvas_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS canvas_context_anchors (
    id                         TEXT PRIMARY KEY,
    canvas_id                  TEXT NOT NULL,
    anchor_block_id            TEXT,
    selection_block_ids_json   TEXT NOT NULL,
    selection_element_ids_json TEXT NOT NULL DEFAULT '[]',
    turn_id                    TEXT NOT NULL,
    summary                    TEXT,
    created_at                 INTEGER NOT NULL,
    FOREIGN KEY(canvas_id) REFERENCES canvases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_canvas_context_anchors_canvas
    ON canvas_context_anchors(canvas_id, created_at DESC);
"#;

const ASTRA_RUN_SELECT: &str = "run_id, thread_id, project_id, project_path, status, mode,
    planner_backend, round_index, round_limit, terminal_reason,
    last_error_code, last_error_message, run_diagnostics_json, error, created_at, updated_at";
const ACTIVE_ASTRA_RUN_STATUS_SQL: &str =
    "'planning', 'thinking', 'awaiting_approval', 'dispatching', 'running'";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

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

fn initialize_schema(conn: &Connection) -> Result<()> {
    conn.execute("DROP TABLE IF EXISTS canvas_shape_refs", [])?;
    conn.execute("DROP TABLE IF EXISTS canvas_context_anchors", [])?;
    conn.execute_batch(SCHEMA_SESSIONS)?;
    conn.execute_batch(SCHEMA_MEMORY)?;
    conn.execute_batch(SCHEMA_APP)?;
    seed_builtins(conn)?;
    sync_astra_pi_builtin_agent_defaults(conn, now_ms())?;
    seed_opencode_builtin_agent(conn, now_ms())?;
    Ok(())
}

/// Seed all builtin data in dependency order: process templates, their stages,
/// runtime agents and assistants, then the process-template stage assistant
/// bindings. Idempotent -- every insert uses INSERT OR IGNORE / ON CONFLICT
/// DO NOTHING, so re-running never clobbers user edits.
fn seed_builtins(conn: &Connection) -> Result<()> {
    let now = now_ms();
    seed_builtin_process_templates(conn, now)?;
    seed_builtin_process_template_stages(conn, now)?;
    seed_builtin_agents(conn, now)?;
    seed_astra_config(conn, now)?;
    seed_builtin_process_template_stage_assistants(conn, now)?;
    Ok(())
}

fn seed_builtin_process_templates(conn: &Connection, now: i64) -> Result<()> {
    for (id, name, description) in BUILTIN_PROCESS_TEMPLATE_SEEDS {
        conn.execute(
            "INSERT OR IGNORE INTO process_templates (id, name, description, type, created_at, updated_at)
             VALUES (?, ?, ?, 'builtin', ?, ?)",
            params![id, name, description, now, now],
        )?;
    }
    Ok(())
}

fn seed_builtin_process_template_stages(conn: &Connection, now: i64) -> Result<()> {
    for (process_template_id, _, _) in BUILTIN_PROCESS_TEMPLATE_SEEDS {
        for (index, (kind, description)) in
            builtin_process_template_stage_seeds(process_template_id)
                .iter()
                .copied()
                .enumerate()
        {
            let id = format!("stage-builtin-{}-{}", process_template_id, kind.as_str());
            let allow_empty_assistants = matches!(kind, StageType::Human | StageType::Done);
            conn.execute(
                "INSERT OR IGNORE INTO stages (id, project_id, type, process_template_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at)
                 VALUES (?, NULL, 'builtin', ?, ?, NULL, ?, NULL, ?, 1, ?, ?, ?)",
                params![
                    id,
                    process_template_id,
                    kind.as_str(),
                    description,
                    (index as i64 + 1) * 1000,
                    allow_empty_assistants as i64,
                    now,
                    now
                ],
            )?;
        }
    }
    Ok(())
}

fn seed_builtin_process_template_stage_assistants(conn: &Connection, now: i64) -> Result<()> {
    let existing_bindings: i64 =
        conn.query_row("SELECT count(*) FROM stage_assistants", [], |row| {
            row.get(0)
        })?;
    if existing_bindings > 0 {
        return Ok(());
    }
    for (process_template_id, _, _) in BUILTIN_PROCESS_TEMPLATE_SEEDS {
        for (kind, _) in builtin_process_template_stage_seeds(process_template_id) {
            if matches!(kind, StageType::Human | StageType::Done) {
                continue;
            }
            let stage_id = format!("stage-builtin-{}-{}", process_template_id, kind.as_str());
            if stage_has_assistants(conn, &stage_id)? {
                continue;
            }
            let assistant_seed = builtin_assistant_seed_for_kind(kind);
            let assistant_id = stable_process_template_builtin_assistant_id(
                process_template_id,
                assistant_seed.id,
            );
            seed_process_template_builtin_assistant(
                conn,
                process_template_id,
                assistant_seed.id,
                now,
            )?;
            conn.execute(
                "INSERT OR IGNORE INTO stage_assistants (stage_id, assistant_id, sort_order, created_at, updated_at)
                 VALUES (?, ?, 0, ?, ?)",
                params![stage_id, assistant_id, now, now],
            )?;
        }
    }
    Ok(())
}

const BUILTIN_PROCESS_TEMPLATE_SEEDS: [(&str, &str, &str); 5] = [
    ("code", "Code", "process_template.description.code"),
    ("writing", "Writing", "process_template.description.writing"),
    (
        "research",
        "Research",
        "process_template.description.research",
    ),
    ("general", "General", "process_template.description.general"),
    (
        "video_production",
        "Video production",
        "process_template.description.video_production",
    ),
];

fn builtin_process_template_stage_seeds(
    process_template_id: &str,
) -> Vec<(StageType, &'static str)> {
    match process_template_id {
        "code" => vec![
            (
                StageType::Research,
                "Gather technical context, codebase constraints, dependencies, and open questions before implementation.",
            ),
            (
                StageType::Plan,
                "Turn the engineering goal into a concrete implementation plan with scope, sequencing, and validation.",
            ),
            (
                StageType::Develop,
                "Implement the planned code changes and keep the thread moving toward a working result.",
            ),
            (
                StageType::Review,
                "Inspect the code for correctness, regressions, edge cases, and missing validation.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, product judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the goal has been completed and verified.",
            ),
        ],
        "writing" => vec![
            (
                StageType::Research,
                "Gather references, audience context, constraints, and source material before drafting.",
            ),
            (
                StageType::Plan,
                "Shape the writing brief into structure, angle, outline, and acceptance criteria.",
            ),
            (
                StageType::Writing,
                "Draft the content in the selected voice, structure, and level of detail.",
            ),
            (
                StageType::Editing,
                "Revise the draft for clarity, flow, accuracy, and fit to the intended audience.",
            ),
            (
                StageType::Proofreading,
                "Check grammar, spelling, formatting, terminology, and final polish before delivery.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, editorial judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the writing goal has been completed and verified.",
            ),
        ],
        "video_production" => vec![
            (
                StageType::Research,
                "Gather references, audience context, production constraints, and creative direction.",
            ),
            (
                StageType::Plan,
                "Turn the video goal into production scope, sequencing, responsibilities, and success criteria.",
            ),
            (
                StageType::Screenplay,
                "Write or refine the script, scenes, narration, dialogue, and beats.",
            ),
            (
                StageType::Storyboard,
                "Map scenes into shots, visual flow, timing, framing, and transitions.",
            ),
            (
                StageType::Design,
                "Define visual style, assets, graphics, motion language, and production look.",
            ),
            (
                StageType::Production,
                "Produce the video assets and assemble the planned shots into the working result.",
            ),
            (
                StageType::Review,
                "Review the cut for story, pacing, accuracy, visual quality, and delivery requirements.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, creative judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the video production goal has been completed and verified.",
            ),
        ],
        _ => vec![
            (
                StageType::Research,
                "Gather context, constraints, references, and open questions before committing to an approach.",
            ),
            (
                StageType::Plan,
                "Turn the goal into a concrete execution plan with scope, sequencing, and success criteria.",
            ),
            (
                StageType::Build,
                "Implement the planned work and keep the thread moving toward a working result.",
            ),
            (
                StageType::Review,
                "Inspect the result for correctness, regressions, edge cases, and missing validation.",
            ),
            (
                StageType::Human,
                "Pause for human input, approval, product judgment, or external information.",
            ),
            (
                StageType::Done,
                "Close the thread after the goal has been completed and verified.",
            ),
        ],
    }
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
        if self.is_empty() { fallback() } else { self }
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

struct BuiltinAgentSeed {
    model: Option<&'static str>,
    models: Vec<RuntimeAgentOptionMetadata>,
    effort: Option<&'static str>,
    efforts: Vec<RuntimeAgentOptionMetadata>,
    permission_mode: Option<&'static str>,
    permission_modes: Vec<RuntimeAgentOptionMetadata>,
    enabled: bool,
    transport: RuntimeTransportKind,
    commands: AgentCommandsInfo,
    ai_providers: Vec<AgentAiProviderInfo>,
}

fn seed_builtin_agent(
    conn: &Connection,
    agent: Agent,
    seed: BuiltinAgentSeed,
    now: i64,
) -> Result<()> {
    let BuiltinAgentSeed {
        model,
        models,
        effort,
        efforts,
        permission_mode,
        permission_modes,
        enabled,
        transport,
        commands,
        ai_providers,
    } = seed;
    let id = agent.as_str();
    let ai_provider = selected_ai_provider_id(&ai_providers, None);
    let ai_providers_json = serde_json::to_string(&ai_providers)?;
    conn.execute(
        "INSERT OR IGNORE INTO agents (
            id, name, display_name, icon, ai_provider, ai_providers_json, ai_api, api_base_url, api_key,
            model, models_json, effort, efforts_json,
            permission_mode, permission_modes_json, type, enabled, transport,
            commands_json, sort_order, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            id,
            runtime_agent_name(agent),
            runtime_agent_display_name(agent),
            id,
            ai_provider.as_deref(),
            ai_providers_json,
            Option::<&str>::None,
            Option::<&str>::None,
            Option::<&str>::None,
            model,
            runtime_options_json(&models)?,
            effort,
            serde_json::to_string(&efforts)?,
            permission_mode,
            serde_json::to_string(&permission_modes)?,
            AgentType::Builtin.as_str(),
            enabled as i64,
            transport_kind_to_db(transport),
            serde_json::to_string(&commands)?,
            runtime_agent_order(agent),
            now,
            now,
        ],
    )?;
    Ok(())
}

fn seed_astra_config(conn: &Connection, now: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO astra_config (
            id, agent, model, effort, permission_mode, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        params![
            1,
            Some(Agent::AstraPi.as_str()),
            Option::<&str>::None, // model
            Option::<&str>::None, // effort
            Option::<&str>::None, // permission_mode
            now,
            now,
        ],
    )?;
    Ok(())
}

fn seed_builtin_agents(conn: &Connection, now: i64) -> Result<()> {
    // Astra Pi agent - for Astra orchestration or direct chat
    seed_builtin_agent(
        conn,
        Agent::AstraPi,
        BuiltinAgentSeed {
            model: Some("gpt-5.5"),
            models: runtime_options(vec![
                runtime_option("gpt-5.5", "GPT 5.5"),
                runtime_option("gpt-5.4", "GPT 5.4"),
            ]),
            effort: Some("off"),
            efforts: vec![
                runtime_option("off", "Off"),
                runtime_option("minimal", "Minimal"),
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
            ],
            permission_mode: None,
            permission_modes: [].to_vec(),
            enabled: true,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: Vec::new(),
                version: Vec::new(),
            },
            ai_providers: vec![AgentAiProviderInfo {
                id: "cc-switch".to_string(),
                display_name: "CC Switch".to_string(),
                provider: "openai".to_string(),
                api: Some("openai-responses".to_string()),
                base_url: Some("http://127.0.0.1:15721/v1".to_string()),
                api_key: Some("ccs".to_string()),
                model: Some("gpt-5.5".to_string()),
                models: vec![
                    runtime_option("gpt-5.5", "GPT-5.5"),
                    runtime_option("gpt-5.4", "GPT 5.4"),
                ],
                enabled: true,
                order: 0,
            }],
        },
        now,
    )?;
    sync_astra_pi_builtin_agent_defaults(conn, now)?;
    seed_builtin_agent(
        conn,
        Agent::Codex,
        BuiltinAgentSeed {
            model: Some("gpt-5.5"),
            models: runtime_options(vec![
                runtime_option("gpt-5.5", "5.5"),
                runtime_option("gpt-5.4", "5.4"),
                runtime_option("gpt-5.3-codex", "5.3 Codex"),
            ]),
            effort: Some("high"),
            efforts: vec![
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
            ],
            permission_mode: Some("read-only"),
            permission_modes: vec![
                runtime_option("read-only", "Default permissions"),
                runtime_option("auto", "Auto-review"),
                runtime_option("full-access", "Full access"),
            ],
            enabled: true,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: vec!["npx -y @agentclientprotocol/codex-acp@latest".to_string()],
                version: vec!["npm view @agentclientprotocol/codex-acp version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )?;
    seed_builtin_agent(
        conn,
        Agent::Pi,
        BuiltinAgentSeed {
            model: None,
            models: Vec::new(),
            effort: Some("medium"),
            efforts: vec![
                runtime_option("off", "Off"),
                runtime_option("minimal", "Minimal"),
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
            ],
            permission_mode: None,
            permission_modes: Vec::new(),
            enabled: true,
            transport: RuntimeTransportKind::PiRpc,
            commands: AgentCommandsInfo {
                session: vec!["pi --mode rpc".to_string()],
                version: vec!["pi --version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )?;
    sync_pi_builtin_agent_defaults(conn, now)?;
    seed_builtin_agent(
        conn,
        Agent::Claude,
        BuiltinAgentSeed {
            model: Some("claude-opus-4-8"),
            models: runtime_options(vec![
                runtime_option("claude-opus-4-8", "Opus 4.8"),
                runtime_option("claude-opus-4-7", "Opus 4.7"),
                runtime_option("claude-opus-4-6", "Opus 4.6"),
            ]),
            effort: Some("high"),
            efforts: vec![
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
                runtime_option("max", "Max"),
            ],
            permission_mode: Some("default"),
            permission_modes: vec![
                runtime_option("default", "Ask before edits"),
                runtime_option("acceptEdits", "Edit automatically"),
                runtime_option("plan", "Plan mode"),
                runtime_option("dontAsk", "Don't Ask"),
            ],
            enabled: true,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: vec!["npx -y @agentclientprotocol/claude-agent-acp@latest".to_string()],
                version: vec!["npm view @agentclientprotocol/claude-agent-acp version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )?;
    seed_opencode_builtin_agent(conn, now)?;
    seed_builtin_assistants(conn, now)?;
    Ok(())
}

/// Seed (`INSERT OR IGNORE`) on every init so the builtin OpenCode row is
/// always present without clobbering user edits.
fn seed_opencode_builtin_agent(conn: &Connection, now: i64) -> Result<()> {
    seed_builtin_agent(
        conn,
        Agent::Opencode,
        BuiltinAgentSeed {
            model: None,
            models: Vec::new(),
            effort: Some("high"),
            efforts: vec![
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
            ],
            permission_mode: None,
            permission_modes: Vec::new(),
            enabled: false,
            transport: RuntimeTransportKind::Acp,
            commands: AgentCommandsInfo {
                session: vec!["opencode acp".to_string()],
                version: vec!["opencode --version".to_string()],
            },
            ai_providers: vec![],
        },
        now,
    )
}

fn sync_astra_pi_builtin_agent_defaults(conn: &Connection, now: i64) -> Result<()> {
    let commands_json = serde_json::to_string(&AgentCommandsInfo {
        session: Vec::new(),
        version: Vec::new(),
    })?;
    conn.execute(
        "UPDATE agents SET display_name = ?, updated_at = ? WHERE id = ? AND display_name = ?",
        params!["Astra Pi", now, Agent::AstraPi.as_str(), "Pi"],
    )?;
    conn.execute(
        "UPDATE agents SET name = ?, updated_at = ? WHERE id = ? AND name = ?",
        params!["Astra Pi", now, Agent::AstraPi.as_str(), "Pi"],
    )?;
    conn.execute(
        "UPDATE agents SET ai_provider = ?, updated_at = ?
         WHERE id = ? AND (ai_provider IS NULL OR trim(ai_provider) = '')",
        params!["cc-switch", now, Agent::AstraPi.as_str()],
    )?;
    conn.execute(
        "UPDATE agents
         SET commands_json = ?, updated_at = ?
         WHERE id = ? AND commands_json <> ?",
        params![commands_json, now, Agent::AstraPi.as_str(), commands_json],
    )?;
    Ok(())
}

fn sync_pi_builtin_agent_defaults(conn: &Connection, now: i64) -> Result<()> {
    let commands_json = serde_json::to_string(&AgentCommandsInfo {
        session: vec!["pi --mode rpc".to_string()],
        version: vec!["pi --version".to_string()],
    })?;
    conn.execute(
        "UPDATE agents
         SET transport = ?, commands_json = ?, enabled = 1, updated_at = ?
         WHERE id = ? AND (transport <> ? OR commands_json <> ? OR enabled = 0)",
        params![
            transport_kind_to_db(RuntimeTransportKind::PiRpc),
            commands_json,
            now,
            Agent::Pi.as_str(),
            transport_kind_to_db(RuntimeTransportKind::PiRpc),
            commands_json,
        ],
    )?;
    Ok(())
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
        Agent::AstraPi => "Astra Pi",
        Agent::Pi => "Pi",
        Agent::Codex => "Codex",
        Agent::Claude => "Claude",
        Agent::Gemini => "Gemini",
        Agent::Opencode => "OpenCode",
    }
}

fn runtime_agent_display_name(agent: Agent) -> &'static str {
    match agent {
        Agent::AstraPi => "Astra Pi",
        Agent::Pi => "Pi",
        Agent::Codex => "Codex CLI",
        Agent::Claude => "Claude Code",
        Agent::Gemini => "Gemini CLI",
        Agent::Opencode => "OpenCode",
    }
}

fn runtime_agent_order(agent: Agent) -> i64 {
    match agent {
        Agent::AstraPi => 0,
        Agent::Pi => 1,
        Agent::Codex => 2,
        Agent::Claude => 3,
        Agent::Gemini => 4,
        Agent::Opencode => 5,
    }
}

#[derive(Debug, Clone)]
struct ExistingSessionRow {
    scope: String,
    file_path: String,
    partial: i64,
    available: i64,
    archived: i64,
    message_count: i64,
    rename_title: Option<String>,
    title: Option<String>,
    first_user_message: Option<String>,
    forked_from_agent: Option<Agent>,
    forked_from_id: Option<String>,
    /// `origin` is provenance plus sidebar routing. Link/unlink paths may
    /// upgrade/downgrade `chat <-> thread`, and merge logic carries the
    /// existing non-chat value forward so a later parser pass can't downgrade
    /// `thread`/`channel` back to `chat`.
    origin: SessionOrigin,
    /// Sticky for the same reason: auto task placeholder rows write this and
    /// we want it preserved when the indexer later replaces the row.
    scheduled_task_id: Option<String>,
    /// Sticky-OR: once any row in the identity set is auxiliary, the merged
    /// write keeps it set. Auxiliary rows never appear in the sidebar.
    is_auxiliary: i64,
}

struct MergedSessionProvenance {
    origin: SessionOrigin,
    scheduled_task_id: Option<String>,
    is_auxiliary: i64,
}

fn load_identity_session_rows(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<Vec<ExistingSessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT scope, file_path, partial, available, archived,
                message_count, rename_title, title, first_user_message, forked_from_agent, forked_from_id,
                origin, scheduled_task_id, is_auxiliary
         FROM sessions
         WHERE agent = ? AND session_id = ?
         ORDER BY
           CASE WHEN file_path != '' AND file_path NOT LIKE 'astra://%' THEN 0 ELSE 1 END,
           partial ASC,
           updated_at DESC,
           last_indexed_at DESC",
    )?;
    let rows = stmt
        .query_map(params![agent.as_str(), session_id], |row| {
            let forked_agent = row
                .get::<_, Option<String>>(9)?
                .and_then(|value| Agent::from_db_str(&value));
            let origin_raw: String = row.get(11)?;
            Ok(ExistingSessionRow {
                scope: row.get(0)?,
                file_path: row.get(1)?,
                partial: row.get(2)?,
                available: row.get(3)?,
                archived: row.get(4)?,
                message_count: row.get(5)?,
                rename_title: row.get(6)?,
                title: row.get(7)?,
                first_user_message: row.get(8)?,
                forked_from_agent: forked_agent,
                forked_from_id: row.get(10)?,
                origin: SessionOrigin::from_db_str(&origin_raw).unwrap_or_default(),
                scheduled_task_id: row.get(12)?,
                is_auxiliary: row.get(13)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn is_real_session_file_path(file_path: &str) -> bool {
    !file_path.trim().is_empty() && !is_virtual_session_ref(file_path)
}

fn choose_identity_title(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
    prefer_incoming_parsed: bool,
) -> Option<String> {
    let existing = rows.iter().find_map(|row| {
        row.title
            .as_ref()
            .map(|title| title.trim())
            .filter(|title| !title.is_empty())
            .map(ToString::to_string)
    });
    if prefer_incoming_parsed {
        incoming.title.clone().or(existing)
    } else {
        existing.or_else(|| incoming.title.clone())
    }
}

fn choose_identity_rename_title(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
) -> Option<String> {
    rows.iter()
        .find_map(|row| row.rename_title.clone())
        .or_else(|| incoming.rename_title.clone())
}

fn choose_identity_first_user(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
    prefer_incoming: bool,
) -> Option<String> {
    let existing = rows.iter().find_map(|row| row.first_user_message.clone());
    if prefer_incoming {
        incoming.first_user_message.clone().or(existing)
    } else {
        existing.or_else(|| incoming.first_user_message.clone())
    }
}

fn merge_identity_lineage(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
) -> (Option<Agent>, Option<String>) {
    let mut forked_from_agent = None;
    let mut forked_from_id = None;
    for row in rows {
        let merged = merge_session_lineage(
            forked_from_agent,
            forked_from_id,
            row.forked_from_agent,
            row.forked_from_id.clone(),
        );
        forked_from_agent = merged.0;
        forked_from_id = merged.1;
    }
    merge_session_lineage(
        forked_from_agent,
        forked_from_id,
        incoming.forked_from_agent,
        incoming.forked_from_id.clone(),
    )
}

fn merged_message_count(rows: &[ExistingSessionRow], incoming: &SessionInfo) -> i64 {
    rows.iter()
        .map(|row| row.message_count)
        .max()
        .unwrap_or_default()
        .max(incoming.message_count as i64)
}

/// Upgrade a session's origin from the default `chat` to `thread` for every
/// row sharing the `(agent, session_id)` identity. Channel-origin rows are
/// left intact (a channel-originated message that lands in a thread keeps its
/// `channel` provenance). Used by `link_thread_session`,
/// `link_stage_session`, and `link_plan_task_session` so the sidebar filter
/// hides any session attached to a thread workflow.
fn upgrade_session_origin_to_thread(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE sessions
            SET origin = 'thread'
          WHERE agent = ? AND session_id = ? AND origin = 'chat'",
        params![agent.as_str(), session_id],
    )?;
    Ok(())
}

/// Symmetric counterpart to `upgrade_session_origin_to_thread`. Called from
/// every `unlink_*` / supersede path: if the `(agent, session_id)` identity has no
/// remaining thread / stage / plan-task / astra-run reference, downgrade
/// `origin = 'thread'` rows back to `'chat'` so the session reappears in the
/// sidebar. Channel-origin rows are not touched; auxiliary rows (Astra
/// delegated etc.) stay hidden via `is_auxiliary` independently of origin.
///
/// The pre-link reverse-join model recomputed visibility on every render,
/// so unlinking automatically restored sidebar presence. The new sticky
/// model needs this explicit downgrade to preserve that behaviour.
fn downgrade_session_origin_when_unlinked(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<()> {
    let still_linked: i64 = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM thread_sessions
            WHERE agent = ?1 AND session_id = ?2
            UNION ALL
            SELECT 1 FROM stage_sessions
            WHERE agent = ?1 AND session_id = ?2
            UNION ALL
            SELECT 1 FROM thread_plan_task_sessions
            WHERE agent = ?1 AND session_id = ?2 AND superseded_at IS NULL
            UNION ALL
            SELECT 1 FROM astra_run_sessions
            WHERE agent = ?1 AND session_id = ?2
         )",
        params![agent.as_str(), session_id],
        |row| row.get(0),
    )?;
    if still_linked == 0 {
        conn.execute(
            "UPDATE sessions
                SET origin = 'chat'
              WHERE agent = ? AND session_id = ? AND origin = 'thread'",
            params![agent.as_str(), session_id],
        )?;
    }
    Ok(())
}

/// Preserve any already-recorded non-chat provenance. A `thread`/`channel`
/// incoming row may only upgrade an identity whose stored rows are still the
/// default `chat`, matching `mark_session_origin` / `upgrade_*` semantics.
fn merged_origin(rows: &[ExistingSessionRow], incoming: SessionOrigin) -> SessionOrigin {
    rows.iter()
        .map(|row| row.origin)
        .find(|origin| *origin != SessionOrigin::Chat)
        .unwrap_or(incoming)
}

/// Sticky scheduled_task_id merge: prefer the incoming value when set, else
/// preserve any existing value. Once a session is attached to a scheduled
/// task that link stays for its lifetime.
fn merged_scheduled_task_id(rows: &[ExistingSessionRow], incoming: Option<&str>) -> Option<String> {
    if let Some(value) = incoming {
        return Some(value.to_string());
    }
    rows.iter().find_map(|row| row.scheduled_task_id.clone())
}

/// Sticky-OR for auxiliary: incoming OR any existing row sets it. Once any
/// row in the identity set is auxiliary, the merged write keeps it set.
fn merged_is_auxiliary(rows: &[ExistingSessionRow], incoming: bool) -> i64 {
    (incoming || rows.iter().any(|row| row.is_auxiliary != 0)) as i64
}

fn merge_session_provenance(
    rows: &[ExistingSessionRow],
    incoming: &SessionInfo,
) -> MergedSessionProvenance {
    MergedSessionProvenance {
        origin: merged_origin(rows, incoming.origin),
        scheduled_task_id: merged_scheduled_task_id(rows, incoming.scheduled_task_id.as_deref()),
        is_auxiliary: merged_is_auxiliary(rows, incoming.is_auxiliary),
    }
}

fn delete_duplicate_session_rows(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    keep_scope: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM sessions
         WHERE agent = ? AND session_id = ? AND scope != ?",
        params![agent.as_str(), session_id, keep_scope],
    )?;
    Ok(())
}

fn merge_session_lineage(
    existing_agent: Option<Agent>,
    existing_id: Option<String>,
    parsed_agent: Option<Agent>,
    parsed_id: Option<String>,
) -> (Option<Agent>, Option<String>) {
    match (existing_agent, existing_id) {
        (Some(agent), Some(id)) => (Some(agent), Some(id)),
        (None, Some(id)) => {
            let agent = if parsed_id.as_deref() == Some(id.as_str()) {
                parsed_agent
            } else {
                None
            };
            (agent, Some(id))
        }
        (Some(agent), None) => (Some(agent), parsed_id),
        (None, None) => (parsed_agent, parsed_id),
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

struct ExistingPlaceholder {
    session_id: String,
    scope: String,
}

fn insert_session(conn: &Connection, scope: &str, s: &SessionInfo) -> Result<()> {
    let identity_rows = load_identity_session_rows(conn, s.agent, &s.id)?;
    let incoming_real = is_real_session_file_path(&s.file_path);
    let existing_real = identity_rows
        .iter()
        .find(|row| is_real_session_file_path(&row.file_path))
        .cloned();
    let existing_same_scope = identity_rows.iter().find(|row| row.scope == scope).cloned();

    if !incoming_real {
        if let Some(existing) = existing_real.clone() {
            let message_count = merged_message_count(&identity_rows, s);
            let partial = existing.partial;
            let available = (existing.available != 0 || s.available) as i64;
            let archived = existing.archived;
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, false);
            let first_user_message = choose_identity_first_user(&identity_rows, s, false);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            conn.execute(
                "UPDATE sessions
                 SET project_path = COALESCE(project_path, ?),
                     project_name = COALESCE(project_name, ?),
                     started_at = COALESCE(started_at, ?),
                     updated_at = COALESCE(updated_at, ?),
                     rename_title = ?,
                     title = ?,
                     first_user_message = ?,
                     message_count = ?,
                     partial = ?,
                     available = ?,
                     archived = ?,
                     forked_from_agent = ?,
                     forked_from_id = ?,
                     origin = ?,
                     scheduled_task_id = ?,
                     is_auxiliary = ?
                 WHERE agent = ? AND session_id = ? AND scope = ?",
                params![
                    s.project_path,
                    s.project_name,
                    s.started_at,
                    s.updated_at,
                    rename_title,
                    title,
                    first_user_message,
                    message_count,
                    partial,
                    available,
                    archived,
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
                    provenance.origin.as_str(),
                    provenance.scheduled_task_id,
                    provenance.is_auxiliary,
                    s.agent.as_str(),
                    s.id,
                    existing.scope,
                ],
            )?;
            delete_duplicate_session_rows(conn, s.agent, &s.id, &existing.scope)?;
            return Ok(());
        }
    }

    if incoming_real {
        if let Some(existing) = existing_same_scope.clone() {
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, true);
            let first_user_message = choose_identity_first_user(&identity_rows, s, true);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            conn.execute(
                "UPDATE sessions
                 SET file_path = ?, project_path = ?, project_name = ?,
                     started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                     message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                     last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                     origin = ?, scheduled_task_id = ?, is_auxiliary = ?
                 WHERE agent = ? AND session_id = ? AND scope = ?",
                params![
                    s.file_path,
                    s.project_path,
                    s.project_name,
                    s.started_at,
                    s.updated_at,
                    rename_title,
                    title,
                    first_user_message,
                    merged_message_count(&identity_rows, s),
                    s.file_size as i64,
                    file_mtime_for(&s.file_path),
                    0,
                    s.available as i64,
                    s.archived as i64,
                    now_ms(),
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
                    provenance.origin.as_str(),
                    provenance.scheduled_task_id,
                    provenance.is_auxiliary,
                    s.agent.as_str(),
                    s.id,
                    existing.scope,
                ],
            )?;
            delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
            return Ok(());
        }
        if let Some(existing) = existing_real.clone().filter(|row| row.scope != scope) {
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, true);
            let first_user_message = choose_identity_first_user(&identity_rows, s, true);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            conn.execute(
                "UPDATE sessions
                 SET scope = ?, file_path = ?, project_path = ?, project_name = ?,
                     started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                     message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                     last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                     origin = ?, scheduled_task_id = ?, is_auxiliary = ?
                 WHERE agent = ? AND session_id = ? AND scope = ?",
                params![
                    scope,
                    s.file_path,
                    s.project_path,
                    s.project_name,
                    s.started_at,
                    s.updated_at,
                    rename_title,
                    title,
                    first_user_message,
                    merged_message_count(&identity_rows, s),
                    s.file_size as i64,
                    file_mtime_for(&s.file_path),
                    0,
                    s.available as i64,
                    s.archived as i64,
                    now_ms(),
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
                    provenance.origin.as_str(),
                    provenance.scheduled_task_id,
                    provenance.is_auxiliary,
                    s.agent.as_str(),
                    s.id,
                    existing.scope,
                ],
            )?;
            delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
            return Ok(());
        }
        if let Some(existing) = existing_placeholder(conn, s.agent, &s.id, scope, s)? {
            let rename_title = choose_identity_rename_title(&identity_rows, s);
            let title = choose_identity_title(&identity_rows, s, true);
            let first_user_message = choose_identity_first_user(&identity_rows, s, true);
            let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
            let provenance = merge_session_provenance(&identity_rows, s);
            if conn.query_row(
                "SELECT 1 FROM sessions WHERE agent = ? AND session_id = ? AND scope = ? LIMIT 1",
                params![s.agent.as_str(), s.id, scope],
                |_| Ok(()),
            ).optional()?.is_some() {
                conn.execute(
                    "UPDATE sessions
                     SET origin = ?,
                         scheduled_task_id = ?,
                         is_auxiliary = ?
                     WHERE agent = ? AND session_id = ? AND scope = ?",
                    params![
                        provenance.origin.as_str(),
                        provenance.scheduled_task_id,
                        provenance.is_auxiliary,
                        s.agent.as_str(),
                        s.id,
                        scope,
                    ],
                )?;
                conn.execute(
                    "DELETE FROM sessions
                     WHERE agent = ? AND session_id = ? AND scope = ?",
                    params![s.agent.as_str(), s.id, existing.scope],
                )?;
            } else {
                conn.execute(
                    "UPDATE sessions
                     SET session_id = ?, scope = ?, file_path = ?, project_path = ?, project_name = ?,
                         started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                         message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                         last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                         origin = ?, scheduled_task_id = ?, is_auxiliary = ?
                     WHERE agent = ? AND session_id = ? AND scope = ?",
                    params![
                        s.id,
                        scope,
                        s.file_path,
                        s.project_path,
                        s.project_name,
                        s.started_at,
                        s.updated_at,
                        rename_title,
                        title,
                        first_user_message,
                        merged_message_count(&identity_rows, s),
                        s.file_size as i64,
                        file_mtime_for(&s.file_path),
                        0,
                        s.available as i64,
                        s.archived as i64,
                        now_ms(),
                        forked_from_agent.map(|agent| agent.as_str()),
                        forked_from_id,
                        provenance.origin.as_str(),
                        provenance.scheduled_task_id,
                        provenance.is_auxiliary,
                        s.agent.as_str(),
                        existing.session_id,
                        existing.scope,
                    ],
                )?;
                delete_duplicate_session_rows(
                    conn,
                    s.agent,
                    &s.id,
                    scope,
                )?;
                return Ok(());
            }
        }
    }

    let identity_rows = load_identity_session_rows(conn, s.agent, &s.id)?;
    let prefer_incoming_parsed = incoming_real;
    let rename_title = choose_identity_rename_title(&identity_rows, s);
    let title = choose_identity_title(&identity_rows, s, prefer_incoming_parsed);
    let first_user_message = choose_identity_first_user(&identity_rows, s, prefer_incoming_parsed);
    if let Some(existing_same_scope) = identity_rows.iter().find(|row| row.scope == scope) {
        let message_count = merged_message_count(&identity_rows, s);
        let partial = if s.partial {
            existing_same_scope.partial
        } else {
            0
        };
        let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
        let provenance = merge_session_provenance(&identity_rows, s);
        conn.execute(
            "UPDATE sessions
             SET session_id = ?, scope = ?, file_path = ?, project_path = ?, project_name = ?,
                 started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                 message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                 last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?,
                 origin = ?, scheduled_task_id = ?, is_auxiliary = ?
             WHERE agent = ? AND session_id = ? AND scope = ?",
            params![
                s.id,
                scope,
                s.file_path,
                s.project_path,
                s.project_name,
                s.started_at,
                s.updated_at,
                rename_title,
                title,
                first_user_message,
                message_count,
                s.file_size as i64,
                file_mtime_for(&s.file_path),
                partial,
                s.available as i64,
                s.archived as i64,
                now_ms(),
                forked_from_agent.map(|agent| agent.as_str()),
                forked_from_id,
                provenance.origin.as_str(),
                provenance.scheduled_task_id,
                provenance.is_auxiliary,
                s.agent.as_str(),
                s.id,
                existing_same_scope.scope,
            ],
        )?;
        delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
        return Ok(());
    }
    let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
    let provenance = merge_session_provenance(&identity_rows, s);
    conn.execute(
        "INSERT OR REPLACE INTO sessions (
            agent, session_id, scope, file_path,
            project_path, project_name,
            started_at, updated_at,
            message_count, rename_title, title, first_user_message,
            file_size, file_mtime,
            partial, available, archived,
            last_indexed_at, forked_from_agent, forked_from_id,
            origin, scheduled_task_id, is_auxiliary
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?,?,?, ?,?, ?,?,?, ?,?,?, ?,?,?)",
        params![
            s.agent.as_str(),
            s.id,
            scope,
            s.file_path,
            s.project_path,
            s.project_name,
            s.started_at,
            s.updated_at,
            merged_message_count(&identity_rows, s),
            rename_title,
            title,
            first_user_message,
            s.file_size as i64,
            file_mtime_for(&s.file_path),
            s.partial as i64,
            s.available as i64,
            s.archived as i64,
            now_ms(),
            forked_from_agent.map(|agent| agent.as_str()),
            forked_from_id,
            provenance.origin.as_str(),
            provenance.scheduled_task_id,
            provenance.is_auxiliary,
        ],
    )?;
    delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
    // Subagent rows are written through upsert_subagent so their lifecycle
    // is independent from the parent session's reindex.
    Ok(())
}

fn existing_placeholder(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    next_scope: &str,
    _next: &SessionInfo,
) -> Result<Option<ExistingPlaceholder>> {
    if let Some(scope) = existing_placeholder_scope(conn, agent, session_id, next_scope)? {
        return Ok(Some(ExistingPlaceholder {
            session_id: session_id.to_string(),
            scope,
        }));
    }
    Ok(None)
}

fn existing_placeholder_scope(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    next_scope: &str,
) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT scope FROM sessions
         WHERE agent = ? AND session_id = ? AND scope != ?
           AND file_size = 0 AND partial = 1
           AND (file_path = '' OR file_path LIKE 'astra://%')
         ORDER BY last_indexed_at DESC
         LIMIT 1",
    )?;
    let scope = stmt
        .query_row(params![agent.as_str(), session_id, next_scope], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(scope)
}

fn file_mtime_for(file_path: &str) -> Option<i64> {
    if file_path.is_empty() {
        return None;
    }
    std::fs::metadata(file_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_millis() as i64)
        })
}

fn opt_u64_to_i64(v: Option<u64>) -> Option<i64> {
    v.map(|n| n as i64)
}

fn opt_i64_to_u64(v: Option<i64>) -> Option<u64> {
    v.map(|n| n as u64)
}

fn canonical_project_path(path: &str) -> Result<String> {
    let path = Path::new(path);
    let meta = std::fs::metadata(path)
        .with_context(|| format!("project directory does not exist: {}", path.display()))?;
    if !meta.is_dir() {
        anyhow::bail!("project path is not a directory: {}", path.display());
    }
    Ok(std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize project path {}", path.display()))?
        .to_string_lossy()
        .to_string())
}

fn clean_project_name(name: Option<&str>, path: &str) -> Result<String> {
    let from_name = name.map(str::trim).filter(|s| !s.is_empty());
    let value = from_name
        .map(str::to_string)
        .or_else(|| {
            Path::new(path)
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| path.to_string());
    if value.trim().is_empty() {
        anyhow::bail!("project name cannot be empty");
    }
    Ok(value)
}

fn clean_child_project_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("project name cannot be empty");
    }
    if trimmed.contains(std::path::MAIN_SEPARATOR) || trimmed == "." || trimmed == ".." {
        anyhow::bail!("project name must be a single directory name");
    }
    if cfg!(windows) && (trimmed.contains('/') || trimmed.contains('\\')) {
        anyhow::bail!("project name must be a single directory name");
    }
    Ok(trimmed.to_string())
}

fn stable_project_id(path: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    format!("project-{}", &hex::encode(hasher.finalize())[..16])
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

fn stable_assistant_id(
    assistant_type: AssistantType,
    process_template_id: Option<&str>,
    project_id: Option<&str>,
    name: &str,
    model: &str,
    now: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(assistant_type.as_str().as_bytes());
    hasher.update(process_template_id.unwrap_or("").as_bytes());
    hasher.update(project_id.unwrap_or("").as_bytes());
    hasher.update(name.as_bytes());
    hasher.update(model.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("assistant-{}", &hex::encode(hasher.finalize())[..16])
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

fn stable_thread_id(project_id: &str, goal: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(goal.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("thread-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_thread_agent_participant_id(
    thread_id: &str,
    agent: Agent,
    model: &str,
    order: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_id.as_bytes());
    hasher.update(agent.as_str().as_bytes());
    hasher.update(model.as_bytes());
    hasher.update(order.to_string().as_bytes());
    format!("thread-agent-{}", &hex::encode(hasher.finalize())[..16])
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

fn stable_plan_round_id(thread_id: &str, round_index: i64, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_id.as_bytes());
    hasher.update(round_index.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("plan-round-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_plan_task_id(
    round_id: &str,
    title: &str,
    sort_order: i64,
    now: i64,
    nonce: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(round_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(sort_order.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("plan-task-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_id(session_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    format!("canvas-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_revision_id(canvas_id: &str, revision: i64, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(canvas_id.as_bytes());
    hasher.update(revision.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("canvas-revision-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_block_record_id(canvas_id: &str, block_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(canvas_id.as_bytes());
    hasher.update(block_id.as_bytes());
    format!("canvas-block-{}", &hex::encode(hasher.finalize())[..16])
}

fn stable_canvas_anchor_id(
    canvas_id: &str,
    selection_block_ids_json: &str,
    selection_element_ids_json: &str,
    turn_id: &str,
    now: i64,
    nonce: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(canvas_id.as_bytes());
    hasher.update(selection_block_ids_json.as_bytes());
    hasher.update(selection_element_ids_json.as_bytes());
    hasher.update(turn_id.as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("canvas-anchor-{}", &hex::encode(hasher.finalize())[..16])
}

#[cfg(test)]
fn temp_child_path(parent: &Path, name: &str) -> std::path::PathBuf {
    parent.join(format!("{name}-{}", unique_suffix()))
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectInfo> {
    Ok(ProjectInfo {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        process_template_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        session_count: row.get::<_, i64>(6)? as usize,
    })
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

fn canvas_document_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasDocumentInfo> {
    Ok(CanvasDocumentInfo {
        id: row.get(0)?,
        session_id: row.get(1)?,
        title: row.get(2)?,
        current_saved_revision: row.get(3)?,
        draft_snapshot_path: row.get(4)?,
        draft_snapshot_hash: row.get(5)?,
        draft_updated_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn canvas_revision_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasRevisionInfo> {
    Ok(CanvasRevisionInfo {
        id: row.get(0)?,
        canvas_id: row.get(1)?,
        revision: row.get(2)?,
        snapshot_path: row.get(3)?,
        snapshot_hash: row.get(4)?,
        snapshot_size_bytes: row.get(5)?,
        source: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn canvas_block_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasBlockRecord> {
    let kind_raw: String = row.get(3)?;
    let source_type_raw: String = row.get(4)?;
    Ok(CanvasBlockRecord {
        id: row.get(0)?,
        canvas_id: row.get(1)?,
        block_id: row.get(2)?,
        block_kind: CanvasBlockKind::from_db_str(&kind_raw).unwrap_or(CanvasBlockKind::Note),
        source_type: CanvasBlockSourceType::from_db_str(&source_type_raw)
            .unwrap_or(CanvasBlockSourceType::Note),
        source_key: row.get(5)?,
        source_path: row.get(6)?,
        metadata_json: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn canvas_anchor_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CanvasContextAnchor> {
    Ok(CanvasContextAnchor {
        id: row.get(0)?,
        canvas_id: row.get(1)?,
        anchor_block_id: row.get(2)?,
        selection_block_ids_json: row.get(3)?,
        selection_element_ids_json: row.get(4)?,
        turn_id: row.get(5)?,
        summary: row.get(6)?,
        created_at: row.get(7)?,
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

fn runtime_agent_session_config_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RuntimeAgentSessionConfigRecord> {
    let agent_raw: String = row.get(0)?;
    Ok(RuntimeAgentSessionConfigRecord {
        agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Codex),
        adapter_version: row.get(1)?,
        available_commands_json: row.get(2)?,
        config_options_json: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn assistant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantInfo> {
    let agent_json: String = row.get(2)?;
    let assistant_type_raw: String = row.get(5)?;
    let process_template_id_raw: Option<String> = row.get(6)?;
    Ok(AssistantInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        agent: serde_json::from_str::<AssistantAgentInfo>(&agent_json).unwrap_or_else(|_| {
            AssistantAgentInfo {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                model: String::new(),
                mode: String::new(),
                effort: String::new(),
            }
        }),
        system_prompt: row.get(3)?,
        color: row.get(4)?,
        assistant_type: AssistantType::from_db_str(&assistant_type_raw)
            .unwrap_or(AssistantType::Custom),
        process_template_id: process_template_id_raw,
        project_id: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
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

fn thread_stage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StageInfo> {
    let stage_type_raw: String = row.get(4)?;
    let process_template_id_raw: Option<String> = row.get(5)?;
    let stage_kind_raw: Option<String> = row.get(6)?;
    let status_raw: Option<String> = row.get(15)?;
    Ok(StageInfo {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        stage_id: row.get(2)?,
        project_id: row.get(3)?,
        assistant_ids: Vec::new(),
        assistants: Vec::new(),
        stage_type: ProjectStageType::from_db_str(&stage_type_raw)
            .unwrap_or(ProjectStageType::Custom),
        process_template_id: process_template_id_raw,
        kind: stage_kind_raw.and_then(|value| StageType::from_db_str(&value)),
        name: row.get(7)?,
        description: row.get(8)?,
        icon: row.get(9)?,
        order: row.get(10)?,
        status: status_raw
            .as_deref()
            .and_then(StageStatus::from_db_str)
            .unwrap_or(StageStatus::NotStarted),
        summary: row.get(16)?,
        outcome: row.get(17)?,
        enabled: row.get::<_, i64>(11)? != 0,
        allow_empty_assistants: row.get::<_, i64>(12)? != 0,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        sessions: Vec::new(),
        issues: Vec::new(),
    })
}

fn thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadInfo> {
    let kind_raw: String = row.get(5)?;
    let origin_raw: String = row.get(9)?;
    Ok(ThreadInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        description: row.get(3)?,
        stage_id: row.get(4)?,
        kind: ThreadKind::from_db_str(&kind_raw).unwrap_or_default(),
        enabled: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        origin: ThreadOrigin::from_db_str(&origin_raw).unwrap_or_default(),
        scheduled_task_id: row.get(10)?,
        assistants: Vec::new(),
        agent_participants: Vec::new(),
        stages: Vec::new(),
        sessions: Vec::new(),
    })
}

fn thread_index_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadIndexItemInfo> {
    let kind_raw: String = row.get(3)?;
    let origin_raw: String = row.get(7)?;
    Ok(ThreadIndexItemInfo {
        thread_id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        kind: ThreadKind::from_db_str(&kind_raw).unwrap_or_default(),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        time: row.get(6)?,
        origin: ThreadOrigin::from_db_str(&origin_raw).unwrap_or_default(),
        scheduled_task_id: row.get(8)?,
        session_keys: Vec::new(),
    })
}

fn default_canvas_title(session_id: &str, requested_title: Option<&str>) -> String {
    requested_title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Canvas {session_id}"))
}

fn get_canvas_document_by_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<CanvasDocumentInfo>> {
    conn.query_row(
        "SELECT id, session_id, title, current_saved_revision, draft_snapshot_path,
                draft_snapshot_hash, draft_updated_at, created_at, updated_at
         FROM canvases
         WHERE session_id = ?",
        params![session_id],
        canvas_document_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn upsert_canvas_document_title(
    conn: &Connection,
    session_id: &str,
    title: Option<&str>,
) -> Result<CanvasDocumentInfo> {
    let now = now_ms();
    let canvas_id = stable_canvas_id(session_id);
    let title_value = default_canvas_title(session_id, title);
    conn.execute(
        "INSERT INTO canvases (
            id, session_id, title, current_saved_revision, draft_snapshot_path,
            draft_snapshot_hash, draft_updated_at, created_at, updated_at
         ) VALUES (?, ?, ?, NULL, NULL, NULL, NULL, ?, ?)
         ON CONFLICT(session_id) DO UPDATE SET
            title = CASE
                WHEN excluded.title <> '' THEN excluded.title
                ELSE canvases.title
            END,
            updated_at = excluded.updated_at",
        params![canvas_id, session_id, title_value, now, now],
    )?;
    get_canvas_document_by_session(conn, session_id)?
        .ok_or_else(|| anyhow::anyhow!("canvas document missing after upsert for {session_id}"))
}

fn latest_canvas_revision(
    conn: &Connection,
    canvas_id: &str,
) -> Result<Option<CanvasRevisionInfo>> {
    conn.query_row(
        "SELECT id, canvas_id, revision, snapshot_path, snapshot_hash, snapshot_size_bytes, source, created_at
         FROM canvas_revisions
         WHERE canvas_id = ?
         ORDER BY revision DESC, created_at DESC
         LIMIT 1",
        params![canvas_id],
        canvas_revision_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn stale_canvas_revision_paths(
    conn: &Connection,
    canvas_id: &str,
    keep_latest: usize,
) -> Result<Vec<String>> {
    let keep_latest = i64::try_from(keep_latest).unwrap_or(i64::MAX);
    let mut stmt = conn.prepare(
        "SELECT snapshot_path
         FROM canvas_revisions
         WHERE canvas_id = ?
         ORDER BY revision DESC, created_at DESC
         LIMIT -1 OFFSET ?",
    )?;
    let rows = stmt.query_map(params![canvas_id, keep_latest], |row| {
        row.get::<_, String>(0)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_canvas_block_records(conn: &Connection, canvas_id: &str) -> Result<Vec<CanvasBlockRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, canvas_id, block_id, block_kind, source_type, source_key, source_path,
                metadata_json, created_at, updated_at
         FROM canvas_blocks
         WHERE canvas_id = ?
         ORDER BY updated_at DESC, created_at DESC, block_id ASC",
    )?;
    let rows = stmt.query_map(params![canvas_id], canvas_block_record_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_canvas_anchors(conn: &Connection, canvas_id: &str) -> Result<Vec<CanvasContextAnchor>> {
    let mut stmt = conn.prepare(
        "SELECT id, canvas_id, anchor_block_id, selection_block_ids_json, selection_element_ids_json, turn_id, summary, created_at
         FROM canvas_context_anchors
         WHERE canvas_id = ?
         ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt.query_map(params![canvas_id], canvas_anchor_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_canvas_document_state(conn: &Connection, session_id: &str) -> Result<CanvasDocumentState> {
    let document = upsert_canvas_document_title(conn, session_id, None)?;
    let saved_revision = latest_canvas_revision(conn, &document.id)?;
    let block_records = load_canvas_block_records(conn, &document.id)?;
    let anchors = load_canvas_anchors(conn, &document.id)?;
    Ok(CanvasDocumentState {
        document,
        draft_snapshot: None,
        saved_revision,
        saved_snapshot: None,
        block_records,
        anchors,
    })
}

fn load_thread_index_session_keys(
    conn: &Connection,
    project_id: Option<&str>,
) -> Result<HashMap<String, HashSet<String>>> {
    let mut keys = HashMap::<String, HashSet<String>>::new();
    for sql in [
        "SELECT s.thread_id, s.agent, s.session_id
         FROM thread_sessions s
         INNER JOIN threads t ON t.id = s.thread_id
         WHERE (?1 IS NULL OR t.project_id = ?1)",
        "SELECT ts.thread_id, ss.agent, ss.session_id
         FROM stage_sessions ss
         INNER JOIN thread_stages ts ON ts.id = ss.thread_stage_id
         INNER JOIN threads t ON t.id = ts.thread_id
         WHERE (?1 IS NULL OR t.project_id = ?1)",
        "SELECT r.thread_id, s.agent, s.session_id
         FROM thread_plan_task_sessions s
         INNER JOIN thread_plan_tasks tk ON tk.id = s.task_id
         INNER JOIN thread_plan_rounds r ON r.id = tk.round_id
         INNER JOIN threads t ON t.id = r.thread_id
         WHERE s.superseded_at IS NULL AND (?1 IS NULL OR t.project_id = ?1)",
        "SELECT r.thread_id, s.agent, s.session_id
         FROM astra_run_sessions s
         INNER JOIN astra_runs r ON r.run_id = s.run_id
         INNER JOIN threads t ON t.id = r.thread_id
         WHERE (?1 IS NULL OR t.project_id = ?1)",
    ] {
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![project_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (thread_id, agent, session_id) = row?;
            keys.entry(thread_id)
                .or_default()
                .insert(format!("{agent}:{session_id}"));
        }
    }
    Ok(keys)
}

fn plan_round_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanRoundInfo> {
    let mode_raw: String = row.get(5)?;
    let source_raw: String = row.get(6)?;
    let status_raw: String = row.get(7)?;
    Ok(PlanRoundInfo {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        astra_run_id: row.get(2)?,
        round_index: row.get(3)?,
        summary: row.get(4)?,
        mode: PlanRoundMode::from_db_str(&mode_raw).unwrap_or(PlanRoundMode::Parallel),
        source: PlanRoundSource::from_db_str(&source_raw).unwrap_or(PlanRoundSource::Manual),
        status: PlanRoundStatus::from_db_str(&status_raw).unwrap_or(PlanRoundStatus::Planned),
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        tasks: Vec::new(),
    })
}

fn plan_task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanTaskInfo> {
    let target_agent_raw: String = row.get(5)?;
    let risk_raw: String = row.get(12)?;
    let status_raw: String = row.get(14)?;
    Ok(PlanTaskInfo {
        id: row.get(0)?,
        round_id: row.get(1)?,
        thread_stage_id: row.get(2)?,
        assistant_id: row.get(3)?,
        agent_participant_id: row.get(4)?,
        target_agent: Agent::from_db_str(&target_agent_raw).unwrap_or(Agent::Codex),
        stage_snapshot_json: row.get(6)?,
        assistant_snapshot_json: row.get(7)?,
        agent_snapshot_json: row.get(8)?,
        title: row.get(9)?,
        prompt: row.get(10)?,
        expected_output: row.get(11)?,
        risk: PlanTaskRisk::from_db_str(&risk_raw).unwrap_or(PlanTaskRisk::Medium),
        sort_order: row.get(13)?,
        status: PlanTaskStatus::from_db_str(&status_raw).unwrap_or(PlanTaskStatus::Planned),
        result_summary: row.get(15)?,
        error: row.get(16)?,
        started_at: row.get(17)?,
        completed_at: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
        sessions: Vec::new(),
    })
}

fn plan_task_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanTaskSessionInfo> {
    let agent_raw: String = row.get(1)?;
    let role_raw: String = row.get(3)?;
    Ok(PlanTaskSessionInfo {
        task_id: row.get(0)?,
        agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Codex),
        session_id: row.get(2)?,
        role: PlanTaskSessionRole::from_db_str(&role_raw).unwrap_or(PlanTaskSessionRole::Runtime),
        attempt_id: row.get(4)?,
        attempt_count: row.get(5)?,
        superseded_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

fn load_project_by_id(conn: &Connection, project_id: &str) -> Result<ProjectInfo> {
    conn.query_row(
        "SELECT p.id, p.path, p.name, p.process_template_id, p.created_at, p.updated_at,
                COUNT(s.session_id) AS session_count
         FROM projects p
         LEFT JOIN sessions s ON s.project_path = p.path AND s.available = 1
                              AND s.is_auxiliary = 0 AND s.origin IN ('chat', 'channel')
         WHERE p.id = ? AND p.archived = 0
         GROUP BY p.id",
        params![project_id],
        project_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("project not found: {project_id}"))
}

fn ensure_process_template_exists(conn: &Connection, process_template_id: &str) -> Result<()> {
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

fn load_agent_by_id(conn: &Connection, agent_id: &str) -> Result<AgentInfo> {
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

fn runtime_agent_selection_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RuntimeAgentSelection> {
    let agent_str: String = row.get(0)?;
    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
    Ok(RuntimeAgentSelection {
        agent,
        model: row.get(1)?,
        effort: row.get(2)?,
        permission_mode: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn astra_run_from_row_without_sessions(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AstraRunRecord> {
    Ok(AstraRunRecord {
        run_id: row.get(0)?,
        thread_id: row.get(1)?,
        project_id: row.get(2)?,
        project_path: row.get(3)?,
        status: row.get(4)?,
        mode: row.get(5)?,
        planner_backend: row.get(6)?,
        round_index: row.get(7)?,
        round_limit: row.get(8)?,
        terminal_reason: row.get(9)?,
        last_error_code: row.get(10)?,
        last_error_message: row.get(11)?,
        internal_planner_sessions: Vec::new(),
        run_diagnostics_json: row.get(12)?,
        error: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn astra_run_session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AstraRunSessionRecord> {
    let agent_raw: String = row.get(1)?;
    let role_raw: String = row.get(3)?;
    Ok(AstraRunSessionRecord {
        run_id: row.get(0)?,
        agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::AstraPi),
        session_id: row.get(2)?,
        role: PlanTaskSessionRole::from_db_str(&role_raw).unwrap_or(PlanTaskSessionRole::Planner),
        sort_order: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn list_astra_run_sessions(conn: &Connection, run_id: &str) -> Result<Vec<AstraRunSessionRecord>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, agent, session_id, role, sort_order, created_at, updated_at
         FROM astra_run_sessions
         WHERE run_id = ?
         ORDER BY sort_order ASC, created_at ASC, session_id ASC",
    )?;
    let rows = stmt.query_map(params![run_id], astra_run_session_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn hydrate_astra_run_sessions(conn: &Connection, runs: &mut [AstraRunRecord]) -> Result<()> {
    if runs.is_empty() {
        return Ok(());
    }
    let run_ids = runs
        .iter()
        .map(|run| run.run_id.clone())
        .collect::<Vec<_>>();
    let mut sql = String::from(
        "SELECT run_id, agent, session_id, role, sort_order, created_at, updated_at
         FROM astra_run_sessions
         WHERE run_id IN (",
    );
    let mut values = Vec::<SqlValue>::with_capacity(run_ids.len());
    for (index, run_id) in run_ids.iter().enumerate() {
        if index > 0 {
            sql.push_str(", ");
        }
        sql.push('?');
        values.push(SqlValue::from(run_id.clone()));
    }
    sql.push_str(") ORDER BY run_id ASC, sort_order ASC, created_at ASC, session_id ASC");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(values.iter()), astra_run_session_from_row)?;
    let mut grouped = HashMap::<String, Vec<AstraRunSessionRecord>>::new();
    for row in rows {
        let record = row?;
        grouped
            .entry(record.run_id.clone())
            .or_default()
            .push(record);
    }
    for run in runs {
        run.internal_planner_sessions = grouped.remove(&run.run_id).unwrap_or_default();
    }
    Ok(())
}

fn replace_astra_run_sessions(
    conn: &Connection,
    run_id: &str,
    sessions: &[AstraRunSessionRecord],
) -> Result<()> {
    // Capture the prior (agent, session_id) set before the DELETE. We only
    // need to downgrade entries that don't reappear in `sessions`; the new
    // INSERTs below re-upgrade everything that's still listed. Computing the
    // set difference up front avoids redundant `still_linked` queries when
    // prior and sessions overlap heavily.
    let prior: HashSet<(Agent, String)> = {
        let mut stmt =
            conn.prepare("SELECT agent, session_id FROM astra_run_sessions WHERE run_id = ?")?;
        let rows = stmt
            .query_map(params![run_id], |row| {
                let agent_str: String = row.get(0)?;
                let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                Ok((agent, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<HashSet<_>>>()?;
        rows
    };
    let next: HashSet<(Agent, String)> = sessions
        .iter()
        .map(|session| (session.agent, session.session_id.clone()))
        .collect();
    conn.execute(
        "DELETE FROM astra_run_sessions WHERE run_id = ?",
        params![run_id],
    )?;
    if !sessions.is_empty() {
        let mut stmt = conn.prepare(
            "INSERT INTO astra_run_sessions (
                run_id, agent, session_id, role, sort_order, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )?;
        for session in sessions {
            stmt.execute(params![
                run_id,
                session.agent.as_str(),
                session.session_id,
                session.role.as_str(),
                session.sort_order,
                session.created_at,
                session.updated_at,
            ])?;
            upgrade_session_origin_to_thread(conn, session.agent, &session.session_id)?;
        }
    }
    for (agent, session_id) in prior.difference(&next) {
        downgrade_session_origin_when_unlinked(conn, *agent, session_id)?;
    }
    Ok(())
}

fn load_assistant_by_id(conn: &Connection, assistant_id: &str) -> Result<AssistantInfo> {
    conn.query_row(
        "SELECT id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
         FROM assistants
         WHERE id = ?",
        params![assistant_id],
        assistant_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("assistant not found: {assistant_id}"))
}

fn load_thread_by_id(conn: &Connection, thread_id: &str) -> Result<ThreadInfo> {
    let mut thread = conn
        .query_row(
            "SELECT id, project_id, goal, description, stage_id, kind, enabled, created_at, updated_at,
                    origin, scheduled_task_id
             FROM threads
             WHERE id = ?",
            params![thread_id],
            thread_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
    thread.assistants = load_thread_assistants(conn, &thread.id)?;
    thread.agent_participants = load_thread_agents(conn, &thread.id)?;
    thread.stages = load_thread_stages(conn, &thread.id)?;
    thread.sessions = load_thread_sessions(conn, &thread.id)?;
    Ok(thread)
}

fn load_plan_round_by_id(conn: &Connection, round_id: &str) -> Result<PlanRoundInfo> {
    let mut round = conn
        .query_row(
            "SELECT id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
             FROM thread_plan_rounds
             WHERE id = ?",
            params![round_id],
            plan_round_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("plan round not found: {round_id}"))?;
    round.tasks = load_plan_tasks(conn, &round.id)?;
    Ok(round)
}

fn load_plan_task_by_id(conn: &Connection, task_id: &str) -> Result<PlanTaskInfo> {
    let mut task = conn
        .query_row(
            "SELECT id, round_id, thread_stage_id, assistant_id, agent_participant_id, target_agent,
                    stage_snapshot_json, assistant_snapshot_json, agent_snapshot_json,
                    title, prompt, expected_output, risk, sort_order, status,
                    result_summary, error, started_at, completed_at, created_at, updated_at
             FROM thread_plan_tasks
             WHERE id = ?",
            params![task_id],
            plan_task_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("plan task not found: {task_id}"))?;
    task.sessions = load_plan_task_sessions(conn, &task.id)?;
    Ok(task)
}

fn load_plan_tasks(conn: &Connection, round_id: &str) -> Result<Vec<PlanTaskInfo>> {
    let mut stmt = conn.prepare(
        "SELECT id, round_id, thread_stage_id, assistant_id, agent_participant_id, target_agent,
                stage_snapshot_json, assistant_snapshot_json, agent_snapshot_json,
                title, prompt, expected_output, risk, sort_order, status,
                result_summary, error, started_at, completed_at, created_at, updated_at
         FROM thread_plan_tasks
         WHERE round_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let mut tasks = stmt
        .query_map(params![round_id], plan_task_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for task in tasks.iter_mut() {
        task.sessions = load_plan_task_sessions(conn, &task.id)?;
    }
    Ok(tasks)
}

fn load_plan_task_sessions(conn: &Connection, task_id: &str) -> Result<Vec<PlanTaskSessionInfo>> {
    let mut stmt = conn.prepare(
        "SELECT task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
         FROM thread_plan_task_sessions
         WHERE task_id = ?
         ORDER BY attempt_count ASC, created_at ASC, role ASC, agent ASC, session_id ASC",
    )?;
    let rows = stmt.query_map(params![task_id], plan_task_session_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn clean_required(value: &str, field: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{field} cannot be empty");
    }
    Ok(value.to_string())
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn aggregate_round_status(statuses: &[PlanTaskStatus]) -> PlanRoundStatus {
    if statuses.contains(&PlanTaskStatus::Running) {
        return PlanRoundStatus::Running;
    }
    if statuses
        .iter()
        .any(|status| matches!(status, PlanTaskStatus::Failed | PlanTaskStatus::Errored))
    {
        return PlanRoundStatus::Errored;
    }
    if statuses
        .iter()
        .all(|status| *status == PlanTaskStatus::Cancelled)
    {
        return PlanRoundStatus::Cancelled;
    }
    if statuses.contains(&PlanTaskStatus::Planned) {
        return PlanRoundStatus::Planned;
    }
    PlanRoundStatus::Completed
}

fn validate_new_plan_round_invariants(round: &NewPlanRound<'_>) -> Result<()> {
    if round.mode != PlanRoundMode::Sequential {
        return Ok(());
    }
    let running_tasks = round
        .tasks
        .iter()
        .filter(|task| task.status == PlanTaskStatus::Running)
        .collect::<Vec<_>>();
    if running_tasks.len() > 1 {
        anyhow::bail!("sequential plan round cannot start multiple running tasks");
    }
    if let Some(running_task) = running_tasks.first() {
        let lower_planned_count = round
            .tasks
            .iter()
            .filter(|task| task.status == PlanTaskStatus::Planned)
            .filter(|task| task.sort_order < running_task.sort_order)
            .count();
        if lower_planned_count > 0 {
            anyhow::bail!("sequential plan round must start the lowest-order planned task");
        }
    }
    Ok(())
}

fn plan_task_statuses(conn: &Connection, round_id: &str) -> Result<Vec<PlanTaskStatus>> {
    let mut stmt = conn.prepare(
        "SELECT status
         FROM thread_plan_tasks
         WHERE round_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![round_id], |row| {
        let status_raw: String = row.get(0)?;
        Ok(PlanTaskStatus::from_db_str(&status_raw).unwrap_or(PlanTaskStatus::Planned))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn update_plan_round_status_from_tasks(
    conn: &Connection,
    round_id: &str,
    now: i64,
) -> Result<PlanRoundStatus> {
    let statuses = plan_task_statuses(conn, round_id)?;
    let status = aggregate_round_status(&statuses);
    conn.execute(
        "UPDATE thread_plan_rounds
         SET status = ?, updated_at = ?
         WHERE id = ?",
        params![status.as_str(), now, round_id],
    )?;
    Ok(status)
}

fn ensure_no_other_running_task(
    conn: &Connection,
    round_id: &str,
    task_id: Option<&str>,
) -> Result<()> {
    let count: i64 = match task_id {
        Some(task_id) => conn.query_row(
            "SELECT count(*)
             FROM thread_plan_tasks
             WHERE round_id = ? AND status = 'running' AND id != ?",
            params![round_id, task_id],
            |row| row.get(0),
        )?,
        None => conn.query_row(
            "SELECT count(*)
             FROM thread_plan_tasks
             WHERE round_id = ? AND status = 'running'",
            params![round_id],
            |row| row.get(0),
        )?,
    };
    if count > 0 {
        anyhow::bail!("sequential plan round already has a running task");
    }
    Ok(())
}

fn ensure_sequential_running_candidate(
    conn: &Connection,
    round_id: &str,
    task_id: &str,
) -> Result<()> {
    ensure_no_other_running_task(conn, round_id, Some(task_id))?;
    let candidate_order: i64 = conn.query_row(
        "SELECT sort_order
         FROM thread_plan_tasks
         WHERE id = ? AND round_id = ?",
        params![task_id, round_id],
        |row| row.get(0),
    )?;
    let lower_planned_count: i64 = conn.query_row(
        "SELECT count(*)
         FROM thread_plan_tasks
         WHERE round_id = ? AND status = 'planned' AND sort_order < ?",
        params![round_id, candidate_order],
        |row| row.get(0),
    )?;
    if lower_planned_count > 0 {
        anyhow::bail!("sequential plan round must start the lowest-order planned task");
    }
    Ok(())
}

fn ensure_plan_task_refs(conn: &Connection, thread_id: &str, task: &NewPlanTask<'_>) -> Result<()> {
    if let Some(thread_stage_id) = task.thread_stage_id {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM thread_stages WHERE id = ? AND thread_id = ? LIMIT 1",
                params![thread_stage_id, thread_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            anyhow::bail!("thread stage does not belong to thread: {thread_stage_id}");
        }
    }
    if let Some(assistant_id) = task.assistant_id {
        load_assistant_by_id(conn, assistant_id)?;
    }
    if let Some(participant_id) = task.agent_participant_id {
        let exists: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM thread_agents WHERE thread_id = ? AND participant_id = ? LIMIT 1",
                params![thread_id, participant_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            anyhow::bail!("thread agent participant does not belong to thread: {participant_id}");
        }
    }
    Ok(())
}

fn insert_plan_task(
    conn: &Connection,
    round_id: &str,
    thread_id: &str,
    task: &NewPlanTask<'_>,
    now: i64,
    nonce: &str,
) -> Result<()> {
    ensure_plan_task_refs(conn, thread_id, task)?;
    let title = clean_required(task.title, "plan task title")?;
    let prompt = clean_required(task.prompt, "plan task prompt")?;
    let agent_snapshot_json =
        clean_required(task.agent_snapshot_json, "plan task agent snapshot json")?;
    let expected_output = clean_optional(task.expected_output);
    let stage_snapshot_json = clean_optional(task.stage_snapshot_json);
    let assistant_snapshot_json = clean_optional(task.assistant_snapshot_json);
    let started_at = if matches!(task.status, PlanTaskStatus::Running) || task.status.is_terminal()
    {
        Some(now)
    } else {
        None
    };
    let completed_at = if task.status.is_terminal() {
        Some(now)
    } else {
        None
    };
    let id = stable_plan_task_id(round_id, &title, task.sort_order, now, nonce);
    conn.execute(
        "INSERT INTO thread_plan_tasks (
            id, round_id, thread_stage_id, assistant_id, agent_participant_id, target_agent,
            stage_snapshot_json, assistant_snapshot_json, agent_snapshot_json,
            title, prompt, expected_output, risk, sort_order, status,
            result_summary, error, started_at, completed_at, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?, ?, ?)",
        params![
            id,
            round_id,
            task.thread_stage_id,
            task.assistant_id,
            task.agent_participant_id,
            task.target_agent.as_str(),
            stage_snapshot_json,
            assistant_snapshot_json,
            agent_snapshot_json,
            title,
            prompt,
            expected_output,
            task.risk.as_str(),
            task.sort_order,
            task.status.as_str(),
            started_at,
            completed_at,
            now,
            now,
        ],
    )?;
    Ok(())
}

fn apply_plan_task_status_patch(
    conn: &Connection,
    task_id: &str,
    patch: PlanTaskStatusPatch<'_>,
    now: i64,
) -> Result<PlanTaskInfo> {
    let current = load_plan_task_by_id(conn, task_id)?;
    let round = load_plan_round_by_id(conn, &current.round_id)?;
    if round.mode == PlanRoundMode::Sequential && patch.status == PlanTaskStatus::Running {
        ensure_sequential_running_candidate(conn, &current.round_id, task_id)?;
    }
    let result_summary = match patch.result_summary {
        Some(value) => clean_optional(value),
        None => current.result_summary,
    };
    let error = match patch.error {
        Some(value) => clean_optional(value),
        None => current.error,
    };
    let started_at = match patch.status {
        PlanTaskStatus::Planned => None,
        PlanTaskStatus::Running => current.started_at.or(Some(now)),
        status if status.is_terminal() => current.started_at.or(Some(now)),
        _ => current.started_at,
    };
    let completed_at = if patch.status.is_terminal() {
        current.completed_at.or(Some(now))
    } else {
        None
    };
    conn.execute(
        "UPDATE thread_plan_tasks
         SET status = ?,
             result_summary = ?,
             error = ?,
             started_at = ?,
             completed_at = ?,
             updated_at = ?
         WHERE id = ?",
        params![
            patch.status.as_str(),
            result_summary,
            error,
            started_at,
            completed_at,
            now,
            task_id,
        ],
    )?;
    update_plan_round_status_from_tasks(conn, &current.round_id, now)?;
    load_plan_task_by_id(conn, task_id)
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

fn instantiate_project_assistants(
    conn: &Connection,
    project_id: &str,
    process_template_id: &str,
    now: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
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
                id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
             )
             SELECT ?, name, agent_json, system_prompt, color, type, process_template_id, ?, 1, ?, ?
             FROM assistants
             WHERE id = ?",
            params![id, project_id, now, now, template.id],
        )?;
    }
    Ok(())
}

fn instantiate_project_builtin_stages(
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

fn link_project_stage_assistants(
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

fn validate_assistant_for_project(
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

fn validate_assistants_for_project(
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

fn ensure_assistant_can_be_disabled(conn: &Connection, assistant_id: &str) -> Result<()> {
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

fn load_stage_assistants(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT tsa.assistant_id, a.name, a.color, tsa.agent_json, a.system_prompt, tsa.sort_order
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
            order: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_project_stage_assistants(
    conn: &Connection,
    stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT sa.assistant_id, a.name, a.color, a.agent_json, a.system_prompt, sa.sort_order
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
            order: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_thread_assistants(conn: &Connection, thread_id: &str) -> Result<Vec<ThreadAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT ta.assistant_id, a.name, a.color, a.agent_json, a.system_prompt, ta.sort_order
         FROM thread_assistants ta
         INNER JOIN assistants a ON a.id = ta.assistant_id
         WHERE ta.thread_id = ?
         ORDER BY ta.sort_order ASC, ta.created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_id], |row| {
        let agent_json: String = row.get(3)?;
        Ok(ThreadAssistantInfo {
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
            order: row.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_thread_agents(conn: &Connection, thread_id: &str) -> Result<Vec<ThreadAgentInfo>> {
    let mut stmt = conn.prepare(
        "SELECT participant_id, agent, model, effort, permission_mode, sort_order, created_at, updated_at
         FROM thread_agents
         WHERE thread_id = ?
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_id], |row| {
        let agent_raw: String = row.get(1)?;
        Ok(ThreadAgentInfo {
            participant_id: row.get(0)?,
            agent: Agent::from_db_str(&agent_raw).unwrap_or(Agent::Codex),
            model: row.get(2)?,
            effort: row.get(3)?,
            permission_mode: row.get(4)?,
            order: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
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

fn replace_thread_assistants(
    conn: &Connection,
    thread_id: &str,
    assistants: &[AssistantInfo],
    now: i64,
) -> Result<()> {
    conn.execute(
        "DELETE FROM thread_assistants WHERE thread_id = ?",
        params![thread_id],
    )?;
    for (index, assistant) in assistants.iter().enumerate() {
        conn.execute(
            "INSERT INTO thread_assistants (thread_id, assistant_id, sort_order, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
            params![thread_id, assistant.id, index as i64, now, now],
        )?;
    }
    Ok(())
}

fn normalize_thread_agents(
    conn: &Connection,
    thread_id: &str,
    participants: &[ThreadAgentInfo],
) -> Result<Vec<ThreadAgentInfo>> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for (index, participant) in participants.iter().enumerate() {
        load_agent_by_id(conn, participant.agent.as_str())?;
        let model = participant.model.trim();
        if model.is_empty() {
            anyhow::bail!("thread agent model cannot be empty");
        }
        let effort = participant.effort.trim();
        let permission_mode = participant.permission_mode.trim();
        let order = participant.order;
        let participant_id = participant.participant_id.trim();
        let participant_id = if participant_id.is_empty() {
            stable_thread_agent_participant_id(thread_id, participant.agent, model, index as i64)
        } else {
            participant_id.to_string()
        };
        if !seen.insert(participant_id.clone()) {
            anyhow::bail!("duplicate thread agent participant id: {participant_id}");
        }
        normalized.push(ThreadAgentInfo {
            participant_id,
            agent: participant.agent,
            model: model.to_string(),
            effort: effort.to_string(),
            permission_mode: permission_mode.to_string(),
            order,
            created_at: participant.created_at,
            updated_at: participant.updated_at,
        });
    }
    Ok(normalized)
}

fn replace_thread_agents(
    conn: &Connection,
    thread_id: &str,
    participants: &[ThreadAgentInfo],
    now: i64,
) -> Result<()> {
    let participants = normalize_thread_agents(conn, thread_id, participants)?;
    conn.execute(
        "DELETE FROM thread_agents WHERE thread_id = ?",
        params![thread_id],
    )?;
    for (index, participant) in participants.iter().enumerate() {
        conn.execute(
            "INSERT INTO thread_agents (
                thread_id, participant_id, agent, model, effort, permission_mode,
                sort_order, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                thread_id,
                participant.participant_id,
                participant.agent.as_str(),
                participant.model,
                participant.effort,
                participant.permission_mode,
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

fn assistant_agent_from_db_agent(agent: &AgentInfo) -> Option<AssistantAgentInfo> {
    let model = agent
        .model
        .clone()
        .or_else(|| agent.models.first().map(|option| option.value.clone()))?;
    let mode = agent.permission_mode.clone().or_else(|| {
        agent
            .permission_modes
            .first()
            .map(|option| option.value.clone())
    })?;
    let effort = agent
        .effort
        .clone()
        .or_else(|| agent.efforts.first().map(|option| option.value.clone()))
        .unwrap_or_default();
    Some(AssistantAgentInfo {
        id: agent.id.clone(),
        name: agent.name.clone(),
        model,
        mode,
        effort,
    })
}

fn seed_builtin_assistants(conn: &Connection, now: i64) -> Result<()> {
    let codex_agent = load_agent_by_id(conn, Agent::Codex.as_str())?;
    let Some(assistant_agent) = assistant_agent_from_db_agent(&codex_agent) else {
        return Ok(());
    };
    for seed in builtin_assistant_seeds() {
        upsert_builtin_assistant(conn, seed, &assistant_agent, now)?;
    }
    Ok(())
}

fn seed_process_template_builtin_assistant(
    conn: &Connection,
    process_template_id: &str,
    source_assistant_id: &str,
    now: i64,
) -> Result<()> {
    let process_template_assistant_id =
        stable_process_template_builtin_assistant_id(process_template_id, source_assistant_id);
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
         )
         SELECT ?, name, agent_json, system_prompt, color, type, ?, NULL, enabled, ?, ?
         FROM assistants
         WHERE id = ?
         ON CONFLICT(id) DO NOTHING",
        params![
            process_template_assistant_id,
            process_template_id,
            now,
            now,
            source_assistant_id
        ],
    )?;
    Ok(())
}

struct BuiltinAssistantSeed {
    id: &'static str,
    name: &'static str,
    color: &'static str,
    system_prompt: &'static str,
}

fn builtin_assistant_seeds() -> Vec<BuiltinAssistantSeed> {
    builtin_assistant_kinds()
        .into_iter()
        .map(builtin_assistant_seed_for_kind)
        .collect()
}

fn builtin_assistant_kinds() -> Vec<StageType> {
    let mut seen = HashSet::new();
    BUILTIN_PROCESS_TEMPLATE_SEEDS
        .iter()
        .flat_map(|(process_template_id, _, _)| {
            builtin_process_template_stage_seeds(process_template_id)
        })
        .map(|(kind, _)| kind)
        .filter(|kind| !matches!(kind, StageType::Human | StageType::Done))
        .filter(|kind| seen.insert(*kind))
        .collect()
}

fn builtin_assistant_seed_for_kind(kind: StageType) -> BuiltinAssistantSeed {
    match kind {
        StageType::Research => BuiltinAssistantSeed {
            id: "assistant-builtin-research",
            name: "Researcher",
            color: "#0ea5e9",
            system_prompt: "Research the problem space before implementation. Gather relevant context, inspect existing project behavior, identify constraints and unknowns, and report concise findings with sources or file references when available.",
        },
        StageType::Plan => BuiltinAssistantSeed {
            id: "assistant-builtin-plan",
            name: "Planner",
            color: "#8b5cf6",
            system_prompt: "Create a clear execution plan from the thread goal. Break the work into ordered steps, call out dependencies and risks, and keep the plan focused on decisions that unblock implementation.",
        },
        StageType::Develop => BuiltinAssistantSeed {
            id: "assistant-builtin-develop",
            name: "Developer",
            color: "#22c55e",
            system_prompt: "Implement the planned code changes. Follow existing project patterns, keep behavior coherent across the stack, and verify the result with relevant checks.",
        },
        StageType::Build => BuiltinAssistantSeed {
            id: "assistant-builtin-build",
            name: "Builder",
            color: "#f59e0b",
            system_prompt: "Implement the planned work and keep the thread moving toward a working result. Make scoped changes and verify the result with the most relevant checks.",
        },
        StageType::Writing => BuiltinAssistantSeed {
            id: "assistant-builtin-writing",
            name: "Writer",
            color: "#ec4899",
            system_prompt: "Draft the requested content in the selected voice, structure, and level of detail while preserving the goal, audience, and constraints. Use the available file-editing tools to write the draft into the target file before finishing; do not only return the text in chat. If no target file is specified, inspect the project or thread context to find the appropriate document path, or create a clearly named draft file and report its path.",
        },
        StageType::Editing => BuiltinAssistantSeed {
            id: "assistant-builtin-editing",
            name: "Editor",
            color: "#f97316",
            system_prompt: "Revise the draft for clarity, flow, accuracy, structure, and fit to the intended audience while preserving the authorial intent.",
        },
        StageType::Review => BuiltinAssistantSeed {
            id: "assistant-builtin-review",
            name: "Reviewer",
            color: "#ef4444",
            system_prompt: "Review the completed work for correctness, regressions, data model consistency, edge cases, and missing tests. Prioritize actionable findings and confirm when no blocking issues remain.",
        },
        StageType::Proofreading => BuiltinAssistantSeed {
            id: "assistant-builtin-proofreading",
            name: "Proofreader",
            color: "#14b8a6",
            system_prompt: "Check grammar, spelling, formatting, terminology, consistency, and final polish before delivery.",
        },
        StageType::Screenplay => BuiltinAssistantSeed {
            id: "assistant-builtin-screenplay",
            name: "Screenwriter",
            color: "#6366f1",
            system_prompt: "Write or refine scripts, scenes, narration, dialogue, and beats for the video production goal.",
        },
        StageType::Storyboard => BuiltinAssistantSeed {
            id: "assistant-builtin-storyboard",
            name: "Storyboarder",
            color: "#a855f7",
            system_prompt: "Map scenes into shots, visual flow, timing, framing, transitions, and production notes.",
        },
        StageType::Design => BuiltinAssistantSeed {
            id: "assistant-builtin-design",
            name: "Designer",
            color: "#06b6d4",
            system_prompt: "Define visual style, assets, graphics, motion language, and production look for the project.",
        },
        StageType::Production => BuiltinAssistantSeed {
            id: "assistant-builtin-production",
            name: "Producer",
            color: "#84cc16",
            system_prompt: "Produce and assemble the planned assets or shots into a working result that matches the production plan.",
        },
        StageType::Human | StageType::Done => BuiltinAssistantSeed {
            id: "assistant-builtin-done",
            name: "Done",
            color: "#64748b",
            system_prompt: "Close the completed thread.",
        },
    }
}

fn upsert_builtin_assistant(
    conn: &Connection,
    seed: BuiltinAssistantSeed,
    assistant_agent: &AssistantAgentInfo,
    now: i64,
) -> Result<()> {
    let agent_json = serde_json::to_string(&assistant_agent)?;
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, 'builtin', NULL, NULL, 1, ?, ?)
         ON CONFLICT(id) DO NOTHING",
        params![
            seed.id,
            seed.name,
            agent_json,
            seed.system_prompt,
            seed.color,
            now,
            now
        ],
    )?;
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

fn load_stage_issues(conn: &Connection, thread_stage_id: &str) -> Result<Vec<StageIssueInfo>> {
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

fn load_thread_sessions(conn: &Connection, thread_id: &str) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let sql = format!(
        "SELECT {SESSION_INFO_COLUMNS_S}
         FROM thread_sessions ts
         INNER JOIN sessions s ON s.agent = ts.agent AND s.session_id = ts.session_id
         WHERE ts.thread_id = ? AND s.available = 1
         ORDER BY ts.created_at ASC, s.updated_at DESC, s.started_at DESC",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![thread_id], session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    for session in sessions.iter_mut() {
        session.subagents = subs_by_parent
            .remove(&(session.agent, session.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

fn load_stage_sessions(conn: &Connection, thread_stage_id: &str) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let sql = format!(
        "SELECT {SESSION_INFO_COLUMNS_S}
         FROM stage_sessions ss
         INNER JOIN sessions s ON s.agent = ss.agent AND s.session_id = ss.session_id
         WHERE ss.thread_stage_id = ? AND s.available = 1
         ORDER BY ss.created_at ASC, s.updated_at DESC, s.started_at DESC",
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![thread_stage_id], session_info_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    dedupe_sessions(&mut sessions);
    for session in sessions.iter_mut() {
        session.subagents = subs_by_parent
            .remove(&(session.agent, session.id.clone()))
            .unwrap_or_default();
    }
    Ok(sessions)
}

fn load_thread_stages(conn: &Connection, thread_id: &str) -> Result<Vec<StageInfo>> {
    let mut stmt = conn.prepare(
        "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.process_template_id, s.kind, s.name, s.description, s.icon,
                ts.sort_order, s.enabled, s.allow_empty_assistants, ts.created_at, ts.updated_at,
                tss.status, tss.summary, tss.outcome
         FROM thread_stages ts
         INNER JOIN threads t ON t.id = ts.thread_id
         INNER JOIN stages s ON s.id = ts.stage_id
         LEFT JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
         WHERE ts.thread_id = ?
         ORDER BY ts.sort_order ASC, ts.created_at ASC",
    )?;
    let mut stages = stmt
        .query_map(params![thread_id], thread_stage_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Lazy default: thread stages without an explicit thread_stage_states row
    // get a status derived from their order relative to the active stage
    // (before -> completed, active -> in_progress, after -> not_started). This
    // keeps pre-V6 threads coherent without materializing rows on read.
    let stored: HashSet<String> = {
        let mut stmt = conn.prepare(
            "SELECT tss.thread_stage_id
             FROM thread_stage_states tss
             INNER JOIN thread_stages ts ON ts.id = tss.thread_stage_id
             WHERE ts.thread_id = ?",
        )?;
        let ids = stmt
            .query_map(params![thread_id], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<HashSet<String>>>()?;
        ids
    };
    let active_stage_id: Option<String> = conn
        .query_row(
            "SELECT stage_id FROM threads WHERE id = ?",
            params![thread_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let active_index = active_stage_id.as_deref().and_then(|active| {
        stages
            .iter()
            .position(|stage| stage.id == active || stage.stage_id == active)
    });
    for (index, stage) in stages.iter_mut().enumerate() {
        if !stored.contains(&stage.id) {
            stage.status = match active_index {
                Some(active) if index < active => StageStatus::Completed,
                Some(active) if index == active => StageStatus::InProgress,
                _ => StageStatus::NotStarted,
            };
        }
        stage.assistants = load_stage_assistants(conn, &stage.id)?;
        stage.assistant_ids = stage
            .assistants
            .iter()
            .map(|assistant| assistant.assistant_id.clone())
            .collect();
        stage.sessions = load_stage_sessions(conn, &stage.id)?;
        stage.issues = load_stage_issues(conn, &stage.id)?;
    }
    Ok(stages)
}

fn attach_kanban_item_sessions(conn: &Connection, items: &mut [KanbanItem]) -> Result<()> {
    for item in items {
        item.sessions = load_kanban_item_sessions(conn, &item.id)?;
    }
    Ok(())
}

fn dedupe_sessions(sessions: &mut Vec<SessionInfo>) {
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

fn better_session_candidate(candidate: &SessionInfo, current: &SessionInfo) -> bool {
    if candidate.available != current.available {
        return candidate.available;
    }
    if candidate.partial != current.partial {
        return !candidate.partial;
    }
    let candidate_real_path = is_real_session_file_path(&candidate.file_path);
    let current_real_path = is_real_session_file_path(&current.file_path);
    if candidate_real_path != current_real_path {
        return candidate_real_path;
    }
    if candidate.file_path.is_empty() != current.file_path.is_empty() {
        return !candidate.file_path.is_empty();
    }
    candidate
        .updated_at
        .unwrap_or(candidate.started_at.unwrap_or_default())
        > current
            .updated_at
            .unwrap_or(current.started_at.unwrap_or_default())
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

fn load_session_history_snapshots(
    conn: &Connection,
    child_agent: Agent,
    child_session_id: &str,
) -> Result<Vec<SessionHistorySnapshotRecord>> {
    let mut stmt = conn.prepare(
        "SELECT ancestor_index, ancestor_agent, ancestor_session_id,
                history_cache_version, created_at
         FROM session_history_snapshots
         WHERE child_agent = ? AND child_session_id = ?
         ORDER BY ancestor_index ASC",
    )?;
    let rows = stmt.query_map(params![child_agent.as_str(), child_session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;

    let mut snapshots = Vec::new();
    for row in rows {
        let (
            ancestor_index,
            ancestor_agent,
            ancestor_session_id,
            history_cache_version,
            created_at,
        ) = row?;
        let Some(ancestor_agent) = Agent::from_db_str(&ancestor_agent) else {
            continue;
        };
        let mut turns_stmt = conn.prepare(
            "SELECT turn_json
             FROM session_history_snapshot_turns
             WHERE child_agent = ? AND child_session_id = ? AND ancestor_index = ?
             ORDER BY turn_index ASC",
        )?;
        let turn_rows = turns_stmt.query_map(
            params![child_agent.as_str(), child_session_id, ancestor_index],
            |row| row.get::<_, String>(0),
        )?;
        let mut turns = Vec::new();
        for turn_row in turn_rows {
            turns.push(serde_json::from_str::<SessionHistoryTurn>(&turn_row?)?);
        }
        snapshots.push(SessionHistorySnapshotRecord {
            child_agent,
            child_session_id: child_session_id.to_string(),
            ancestor_agent,
            ancestor_session_id,
            ancestor_index,
            history_cache_version,
            created_at,
            turns,
        });
    }

    Ok(snapshots)
}

fn replace_session_history_snapshots_inner(
    conn: &Connection,
    child_agent: Agent,
    child_session_id: &str,
    snapshots: &[SessionHistorySnapshotRecord],
) -> Result<()> {
    conn.execute(
        "DELETE FROM session_history_snapshots
         WHERE child_agent = ? AND child_session_id = ?",
        params![child_agent.as_str(), child_session_id],
    )?;
    {
        let mut header_stmt = conn.prepare(
            "INSERT INTO session_history_snapshots (
                child_agent, child_session_id, ancestor_index, ancestor_agent,
                ancestor_session_id, history_cache_version, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )?;
        let mut turn_stmt = conn.prepare(
            "INSERT INTO session_history_snapshot_turns (
                child_agent, child_session_id, ancestor_index, turn_index, turn_id,
                started_at, updated_at, turn_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for snapshot in snapshots {
            header_stmt.execute(params![
                child_agent.as_str(),
                child_session_id,
                snapshot.ancestor_index,
                snapshot.ancestor_agent.as_str(),
                snapshot.ancestor_session_id.as_str(),
                snapshot.history_cache_version,
                snapshot.created_at,
            ])?;
            for (index, turn) in snapshot.turns.iter().enumerate() {
                turn_stmt.execute(params![
                    child_agent.as_str(),
                    child_session_id,
                    snapshot.ancestor_index,
                    index as i64,
                    turn.turn_id.as_str(),
                    turn.started_at,
                    turn.updated_at,
                    serde_json::to_string(turn)?,
                ])?;
            }
        }
    }
    Ok(())
}

fn load_all_subagents_grouped(
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
const SESSION_INFO_COLUMNS_S: &str =
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

fn session_info_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionInfo> {
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

fn channel_session_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ChannelSessionRecord> {
    let agent_str: String = row.get(7)?;
    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
    Ok(ChannelSessionRecord {
        platform: row.get(0)?,
        channel_id: row.get(1)?,
        channel_type: row.get(2)?,
        user_id: row.get(3)?,
        team_id: row.get(4)?,
        thread_id: row.get(5)?,
        display_name: row.get(6)?,
        agent,
        agent_session_id: row.get(8)?,
        sessio_runtime_session_id: row.get(9)?,
        workspace_path: row.get(10)?,
        metadata_json: row.get(11)?,
        last_update_id: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        last_activity_at: row.get(15)?,
        ended_at: row.get(16)?,
    })
}

fn channel_session_info_from_record(record: ChannelSessionRecord) -> ChannelSessionInfo {
    let metadata = serde_json::from_str::<serde_json::Value>(&record.metadata_json)
        .unwrap_or_else(|_| serde_json::json!({}));
    ChannelSessionInfo {
        platform: record.platform,
        channel_id: record.channel_id,
        channel_type: record.channel_type,
        user_id: record.user_id,
        team_id: record.team_id,
        thread_id: record.thread_id,
        display_name: record.display_name,
        agent: record.agent,
        agent_session_id: record.agent_session_id,
        sessio_runtime_session_id: record.sessio_runtime_session_id,
        workspace_path: record.workspace_path,
        metadata,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_activity_at: record.last_activity_at,
        ended_at: record.ended_at,
    }
}

fn scheduled_task_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ScheduledTaskRecord> {
    Ok(ScheduledTaskRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        status: row.get(2)?,
        schedule_json: row.get(3)?,
        target_json: row.get(4)?,
        project_id: row.get(5)?,
        mode: row.get(6)?,
        sort_order: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
        last_run_at_ms: row.get(10)?,
    })
}

fn scheduled_task_run_record_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ScheduledTaskRunRecord> {
    let session_agent_raw: Option<String> = row.get(10)?;
    Ok(ScheduledTaskRunRecord {
        id: row.get(0)?,
        task_id: row.get(1)?,
        mode: row.get(2)?,
        trigger: row.get(3)?,
        status: row.get(4)?,
        started_at_ms: row.get(5)?,
        scheduled_for_ms: row.get(6)?,
        completed_at_ms: row.get(7)?,
        task_name: row.get(8)?,
        target_json: row.get(9)?,
        session_agent: session_agent_raw.as_deref().and_then(Agent::from_db_str),
        session_id: row.get(11)?,
        agent_session_id: row.get(12)?,
        thread_id: row.get(13)?,
        astra_run_id: row.get(14)?,
        push_platform: row.get(15)?,
        push_chat_id: row.get(16)?,
        push_status: row.get(17)?,
        push_summary: row.get(18)?,
        push_error: row.get(19)?,
        push_sent_at_ms: row.get(20)?,
        error: row.get(21)?,
    })
}

fn select_channel_session_columns() -> &'static str {
    "platform, channel_id, channel_type, user_id, team_id, thread_id, display_name,
     agent, agent_session_id, sessio_runtime_session_id, workspace_path, metadata_json,
     last_update_id, created_at, updated_at, last_activity_at, ended_at"
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
        initialize_schema(&conn)
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
        let sql = format!(
            "SELECT {} FROM channel_sessions ORDER BY updated_at DESC",
            select_channel_session_columns()
        );
        let mut stmt = conn.prepare(&sql)?;
        let records = stmt
            .query_map([], channel_session_record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records
            .into_iter()
            .map(channel_session_info_from_record)
            .collect())
    }

    fn get_active_channel_session(
        &self,
        platform: &str,
        channel_id: &str,
    ) -> Result<Option<ChannelSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {} FROM channel_sessions
             WHERE platform = ? AND channel_id = ? AND ended_at IS NULL
             ORDER BY last_activity_at DESC, updated_at DESC
             LIMIT 1",
            select_channel_session_columns()
        );
        conn.query_row(
            &sql,
            params![platform, channel_id],
            channel_session_record_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn upsert_channel_session(&self, record: &ChannelSessionRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        if record.ended_at.is_none() {
            conn.execute(
                "UPDATE channel_sessions
                 SET ended_at = ?, updated_at = ?
                 WHERE platform = ? AND channel_id = ? AND ended_at IS NULL
                   AND NOT (agent = ? AND agent_session_id = ?)",
                params![
                    record.updated_at,
                    record.updated_at,
                    record.platform.as_str(),
                    record.channel_id.as_str(),
                    record.agent.as_str(),
                    record.agent_session_id.as_str(),
                ],
            )?;
        }
        conn.execute(
            "INSERT INTO channel_sessions (
                platform, channel_id, channel_type, user_id, team_id, thread_id, display_name,
                agent, agent_session_id, sessio_runtime_session_id, workspace_path, metadata_json,
                last_update_id, created_at, updated_at, last_activity_at, ended_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(platform, channel_id, agent, agent_session_id) DO UPDATE SET
                channel_type = excluded.channel_type,
                user_id = excluded.user_id,
                team_id = excluded.team_id,
                thread_id = excluded.thread_id,
                display_name = excluded.display_name,
                sessio_runtime_session_id = excluded.sessio_runtime_session_id,
                workspace_path = excluded.workspace_path,
                metadata_json = excluded.metadata_json,
                last_update_id = CASE
                    WHEN excluded.last_update_id IS NULL THEN channel_sessions.last_update_id
                    WHEN channel_sessions.last_update_id IS NULL THEN excluded.last_update_id
                    WHEN excluded.last_update_id > channel_sessions.last_update_id THEN excluded.last_update_id
                    ELSE channel_sessions.last_update_id
                END,
                updated_at = excluded.updated_at,
                last_activity_at = excluded.last_activity_at,
                ended_at = excluded.ended_at",
            params![
                record.platform.as_str(),
                record.channel_id.as_str(),
                record.channel_type.as_deref(),
                record.user_id.as_deref(),
                record.team_id.as_deref(),
                record.thread_id.as_deref(),
                record.display_name.as_deref(),
                record.agent.as_str(),
                record.agent_session_id.as_str(),
                record.sessio_runtime_session_id.as_str(),
                record.workspace_path.as_str(),
                record.metadata_json.as_str(),
                record.last_update_id,
                record.created_at,
                record.updated_at,
                record.last_activity_at,
                record.ended_at,
            ],
        )?;
        Ok(())
    }

    fn update_channel_session_activity(
        &self,
        platform: &str,
        channel_id: &str,
        last_update_id: Option<i64>,
        last_activity_at: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE channel_sessions
             SET last_update_id = CASE
                    WHEN ? IS NULL THEN last_update_id
                    WHEN last_update_id IS NULL THEN ?
                    WHEN ? > last_update_id THEN ?
                    ELSE last_update_id
                 END,
                 last_activity_at = ?,
                 updated_at = ?
             WHERE platform = ? AND channel_id = ? AND ended_at IS NULL",
            params![
                last_update_id,
                last_update_id,
                last_update_id,
                last_update_id,
                last_activity_at,
                last_activity_at,
                platform,
                channel_id,
            ],
        )?;
        Ok(())
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
        conn.execute(
            "UPDATE channel_sessions
             SET ended_at = COALESCE(ended_at, ?), updated_at = ?
             WHERE platform = ? AND channel_id = ? AND agent = ? AND agent_session_id = ?",
            params![
                ended_at,
                ended_at,
                platform,
                channel_id,
                agent.as_str(),
                agent_session_id,
            ],
        )?;
        Ok(())
    }

    fn list_scheduled_tasks(&self) -> Result<Vec<ScheduledTaskRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, status, schedule_json, target_json, project_id, mode,
                    sort_order, created_at_ms, updated_at_ms, last_run_at_ms
             FROM scheduled_tasks
             ORDER BY sort_order ASC, created_at_ms ASC",
        )?;
        let records = stmt
            .query_map([], scheduled_task_record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    fn list_scheduled_task_runs(&self) -> Result<Vec<ScheduledTaskRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                    task_name, target_json, session_agent, session_id, agent_session_id,
                    thread_id, astra_run_id, push_platform, push_chat_id,
                    push_status, push_summary, push_error, push_sent_at_ms, error
             FROM (
                SELECT id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                       task_name, target_json, session_agent, session_id, agent_session_id,
                       thread_id, astra_run_id, push_platform, push_chat_id,
                       push_status, push_summary, push_error, push_sent_at_ms, error,
                       ROW_NUMBER() OVER (
                           PARTITION BY task_id
                           ORDER BY started_at_ms DESC, id DESC
                       ) AS run_rank
                FROM scheduled_task_runs
             )
             WHERE run_rank <= ?
                OR status = 'running'
                OR push_status IN ('pending', 'summarizing')
             ORDER BY started_at_ms DESC, id DESC",
        )?;
        let records = stmt
            .query_map(
                params![SCHEDULED_TASK_RUN_HISTORY_LIMIT_PER_TASK as i64],
                scheduled_task_run_record_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    fn list_scheduled_task_runs_requiring_update(&self) -> Result<Vec<ScheduledTaskRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                    task_name, target_json, session_agent, session_id, agent_session_id,
                    thread_id, astra_run_id, push_platform, push_chat_id,
                    push_status, push_summary, push_error, push_sent_at_ms, error
             FROM scheduled_task_runs
             WHERE status = 'running'
                OR push_status IN ('pending', 'summarizing')
             ORDER BY started_at_ms DESC, id DESC",
        )?;
        let records = stmt
            .query_map([], scheduled_task_run_record_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(records)
    }

    fn replace_scheduled_tasks(&self, tasks: &[ScheduledTaskRecord]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        if tasks.is_empty() {
            tx.execute("DELETE FROM scheduled_tasks", [])?;
        } else {
            let placeholders = vec!["?"; tasks.len()].join(", ");
            let sql = format!("DELETE FROM scheduled_tasks WHERE id NOT IN ({placeholders})");
            let ids = tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>();
            tx.execute(&sql, params_from_iter(ids))?;
        }
        {
            let mut stmt = tx.prepare(
                "INSERT INTO scheduled_tasks (
                    id, name, status, schedule_json, target_json, project_id, mode,
                    sort_order, created_at_ms, updated_at_ms, last_run_at_ms
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    status = excluded.status,
                    schedule_json = excluded.schedule_json,
                    target_json = excluded.target_json,
                    project_id = excluded.project_id,
                    mode = excluded.mode,
                    sort_order = excluded.sort_order,
                    updated_at_ms = excluded.updated_at_ms,
                    last_run_at_ms = excluded.last_run_at_ms",
            )?;
            for (index, task) in tasks.iter().enumerate() {
                stmt.execute(params![
                    task.id.as_str(),
                    task.name.as_str(),
                    task.status.as_str(),
                    task.schedule_json.as_str(),
                    task.target_json.as_str(),
                    task.project_id.as_str(),
                    task.mode.as_str(),
                    index as i64,
                    task.created_at_ms,
                    task.updated_at_ms,
                    task.last_run_at_ms,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn insert_scheduled_task_run(&self, run: &ScheduledTaskRunRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO scheduled_task_runs (
                id, task_id, mode, trigger, status, started_at_ms, scheduled_for_ms, completed_at_ms,
                task_name, target_json, session_agent, session_id, agent_session_id, thread_id, astra_run_id,
                push_platform, push_chat_id, push_status, push_summary, push_error, push_sent_at_ms, error
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO NOTHING",
            params![
                run.id.as_str(),
                run.task_id.as_str(),
                run.mode.as_str(),
                run.trigger.as_str(),
                run.status.as_str(),
                run.started_at_ms,
                run.scheduled_for_ms,
                run.completed_at_ms,
                run.task_name.as_deref(),
                run.target_json.as_deref(),
                run.session_agent.map(|agent| agent.as_str().to_string()),
                run.session_id.as_deref(),
                run.agent_session_id.as_deref(),
                run.thread_id.as_deref(),
                run.astra_run_id.as_deref(),
                run.push_platform.as_deref(),
                run.push_chat_id.as_deref(),
                run.push_status.as_deref(),
                run.push_summary.as_deref(),
                run.push_error.as_deref(),
                run.push_sent_at_ms,
                run.error.as_deref(),
            ],
        )?;
        Ok(())
    }

    fn update_scheduled_task_run_status(
        &self,
        run_id: &str,
        status: &str,
        completed_at_ms: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_task_runs
             SET status = ?,
                 completed_at_ms = CASE WHEN ? IS NULL THEN completed_at_ms ELSE ? END,
                 error = CASE WHEN ? IS NULL THEN error ELSE ? END
             WHERE id = ?",
            params![
                status,
                completed_at_ms,
                completed_at_ms,
                error,
                error,
                run_id
            ],
        )?;
        Ok(())
    }

    fn update_scheduled_task_run_agent_session_id(
        &self,
        run_id: &str,
        agent_session_id: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_task_runs
             SET agent_session_id = ?
             WHERE id = ?",
            params![agent_session_id, run_id],
        )?;
        Ok(())
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
        conn.execute(
            "UPDATE scheduled_task_runs
             SET push_status = ?,
                 push_summary = CASE WHEN ? IS NULL THEN push_summary ELSE ? END,
                 push_error = ?,
                 push_sent_at_ms = CASE WHEN ? IS NULL THEN push_sent_at_ms ELSE ? END
             WHERE id = ?",
            params![
                push_status,
                push_summary,
                push_summary,
                push_error,
                push_sent_at_ms,
                push_sent_at_ms,
                run_id,
            ],
        )?;
        Ok(())
    }

    fn update_scheduled_task_last_run(&self, task_id: &str, when_ms: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_tasks
             SET last_run_at_ms = ?
             WHERE id = ?",
            params![when_ms, task_id],
        )?;
        Ok(())
    }

    fn fail_interrupted_task_run_pushes(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE scheduled_task_runs
             SET push_status = 'failed',
                 push_error = COALESCE(push_error, 'push interrupted by app restart')
             WHERE push_status = 'summarizing'",
            [],
        )?;
        Ok(())
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
        let mut stmt = conn.prepare(
            "SELECT p.id, p.path, p.name, p.process_template_id, p.created_at, p.updated_at,
                    COUNT(s.session_id) AS session_count
             FROM projects p
             LEFT JOIN sessions s ON s.project_path = p.path AND s.available = 1
                                  AND s.is_auxiliary = 0 AND s.origin IN ('chat', 'channel')
             WHERE p.archived = 0
             GROUP BY p.id
             ORDER BY p.updated_at DESC, p.name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], project_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        let canonical = canonical_project_path(path)?;
        let name = clean_project_name(name, &canonical)?;
        let id = stable_project_id(&canonical);
        let now = now_ms();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        ensure_process_template_exists(&tx, &process_template_id)?;
        tx.execute(
            "INSERT INTO projects (id, path, name, process_template_id, created_at, updated_at, archived)
             VALUES (?, ?, ?, ?, ?, ?, 0)",
            params![id, canonical, name, process_template_id.as_str(), now, now],
        )
        .with_context(|| "add project")?;
        instantiate_project_builtin_stages(&tx, &id, &process_template_id, enabled_stage_ids, now)?;
        instantiate_project_assistants(&tx, &id, &process_template_id, now)?;
        link_project_stage_assistants(&tx, &id, &process_template_id, now)?;
        let project = load_project_by_id(&tx, &id)?;
        tx.commit()?;
        Ok(project)
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        let parent = canonical_project_path(parent_path)?;
        let clean_name = clean_child_project_name(name)?;
        let project_path = Path::new(&parent).join(&clean_name);
        if project_path.exists() {
            anyhow::bail!(
                "project directory already exists: {}",
                project_path.display()
            );
        }
        std::fs::create_dir(&project_path)
            .with_context(|| format!("create project directory {}", project_path.display()))?;
        let path = canonical_project_path(&project_path.to_string_lossy())?;
        self.add_project(
            &path,
            Some(&clean_name),
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
        let tx = conn.transaction()?;
        let current = load_project_by_id(&tx, project_id)?;
        let next_name = match name {
            Some(value) => clean_project_name(Some(value), &current.path)?,
            None => current.name,
        };
        let current_process_template_id = current.process_template_id.clone();
        let next_process_template_id =
            process_template_id.unwrap_or_else(|| current_process_template_id.clone());
        ensure_process_template_exists(&tx, &next_process_template_id)?;
        let process_template_changed = next_process_template_id != current_process_template_id;
        tx.execute(
            "UPDATE projects
             SET name = ?, process_template_id = ?, updated_at = ?
             WHERE id = ? AND archived = 0",
            params![
                next_name,
                next_process_template_id.as_str(),
                now_ms(),
                project_id
            ],
        )?;
        if process_template_changed {
            tx.execute(
                "DELETE FROM stages WHERE project_id = ? AND type = 'builtin'",
                params![project_id],
            )?;
            instantiate_project_builtin_stages(
                &tx,
                project_id,
                &next_process_template_id,
                None,
                now_ms(),
            )?;
            instantiate_project_assistants(&tx, project_id, &next_process_template_id, now_ms())?;
            link_project_stage_assistants(&tx, project_id, &next_process_template_id, now_ms())?;
        }
        let project = load_project_by_id(&tx, project_id)?;
        tx.commit()?;
        Ok(project)
    }

    fn archive_project(&self, project_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE projects SET archived = 1, updated_at = ? WHERE id = ? AND archived = 0",
            params![now_ms(), project_id],
        )?;
        if changed == 0 {
            anyhow::bail!("project not found: {project_id}");
        }
        Ok(())
    }

    fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        let conn = self.conn.lock().unwrap();
        load_agents(&conn)
    }

    fn get_astra_config(&self) -> Result<AstraConfig> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent, model, effort, permission_mode, created_at, updated_at
             FROM astra_config WHERE id = 1",
        )?;
        let config = stmt.query_row([], |row| {
            Ok(AstraConfig {
                agent: row.get(0)?,
                model: row.get(1)?,
                effort: row.get(2)?,
                permission_mode: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        Ok(config)
    }

    fn update_astra_config(&self, patch: AstraConfigPatch<'_>) -> Result<AstraConfig> {
        let now = now_ms();

        let mut updates = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(value) = patch.agent {
            updates.push("agent = ?");
            params.push(Box::new(value));
        }
        if let Some(value) = patch.model {
            updates.push("model = ?");
            params.push(Box::new(value));
        }
        if let Some(value) = patch.effort {
            updates.push("effort = ?");
            params.push(Box::new(value));
        }
        if let Some(value) = patch.permission_mode {
            updates.push("permission_mode = ?");
            params.push(Box::new(value));
        }

        if updates.is_empty() {
            return self.get_astra_config();
        }

        updates.push("updated_at = ?");
        params.push(Box::new(now));

        let sql = format!(
            "UPDATE astra_config SET {} WHERE id = 1",
            updates.join(", ")
        );
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let conn = self.conn.lock().unwrap();
        conn.execute(&sql, param_refs.as_slice())?;
        drop(conn);

        self.get_astra_config()
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
        conn.query_row(
            "SELECT agent, model, effort, permission_mode, updated_at
             FROM runtime_agent_selections
             WHERE key = ?",
            params![RUNTIME_SELECTION_KEY],
            runtime_agent_selection_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn set_last_runtime_agent_selection(
        &self,
        agent: Agent,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
    ) -> Result<RuntimeAgentSelection> {
        let conn = self.conn.lock().unwrap();
        let now = now_ms();
        let model = model.map(str::trim).filter(|value| !value.is_empty());
        let effort = effort.map(str::trim).filter(|value| !value.is_empty());
        let permission_mode = permission_mode
            .map(str::trim)
            .filter(|value| !value.is_empty());
        conn.execute(
            "INSERT INTO runtime_agent_selections (
                key, agent, model, effort, permission_mode, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET
                agent = excluded.agent,
                model = excluded.model,
                effort = excluded.effort,
                permission_mode = excluded.permission_mode,
                updated_at = excluded.updated_at",
            params![
                RUNTIME_SELECTION_KEY,
                agent.as_str(),
                model,
                effort,
                permission_mode,
                now,
            ],
        )?;
        Ok(RuntimeAgentSelection {
            agent,
            model: model.map(str::to_string),
            effort: effort.map(str::to_string),
            permission_mode: permission_mode.map(str::to_string),
            updated_at: now,
        })
    }

    fn list_assistants(&self, project_id: Option<&str>) -> Result<Vec<AssistantInfo>> {
        let conn = self.conn.lock().unwrap();
        let assistants = if let Some(project_id) = project_id {
            load_project_by_id(&conn, project_id)?;
            let mut stmt = conn.prepare(
                "SELECT id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
                 FROM assistants
                 WHERE project_id = ?
                 ORDER BY type ASC, updated_at DESC, name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map(params![project_id], assistant_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
                 FROM assistants
                 ORDER BY type ASC, updated_at DESC, name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map([], assistant_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(assistants)
    }

    fn create_assistant(&self, assistant: NewAssistant<'_>) -> Result<AssistantInfo> {
        let NewAssistant {
            name,
            agent,
            system_prompt,
            color,
            assistant_type,
            process_template_id,
            project_id,
        } = assistant;
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("assistant name cannot be empty");
        }
        let mut agent = agent;
        agent.id = agent.id.trim().to_string();
        agent.name = agent.name.trim().to_string();
        agent.model = agent.model.trim().to_string();
        agent.mode = agent.mode.trim().to_string();
        agent.effort = agent.effort.trim().to_string();
        if agent.id.is_empty() {
            anyhow::bail!("assistant agent id cannot be empty");
        }
        if agent.name.is_empty() {
            anyhow::bail!("assistant agent name cannot be empty");
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
        let system_prompt = system_prompt.map(str::trim).filter(|s| !s.is_empty());
        let color = color.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().unwrap();
        let db_agent = load_agent_by_id(&conn, &agent.id)?;
        agent.name = db_agent.name;
        let project = project_id
            .map(|project_id| load_project_by_id(&conn, project_id))
            .transpose()?;
        let resolved_process_template_id = process_template_id.or_else(|| {
            project
                .as_ref()
                .map(|project| project.process_template_id.clone())
        });
        match assistant_type {
            AssistantType::Builtin => {
                if project_id.is_some() {
                    anyhow::bail!("builtin assistant cannot be linked to a project");
                }
            }
            AssistantType::Custom => {}
        }
        if let Some(process_template_id) = resolved_process_template_id.as_deref() {
            ensure_process_template_exists(&conn, process_template_id)?;
        }
        let now = now_ms();
        let id = stable_assistant_id(
            assistant_type,
            resolved_process_template_id.as_deref(),
            project_id,
            name,
            &agent.model,
            now,
        );
        let agent_json = serde_json::to_string(&agent)?;
        conn.execute(
            "INSERT INTO assistants (
                id, name, agent_json, system_prompt, color, type, process_template_id, project_id, enabled, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
            params![
                id,
                name,
                agent_json,
                system_prompt,
                color,
                assistant_type.as_str(),
                resolved_process_template_id.as_deref(),
                project_id,
                now,
                now,
            ],
        )?;
        load_assistant_by_id(&conn, &id)
    }

    fn update_assistant(
        &self,
        assistant_id: &str,
        name: Option<&str>,
        agent: Option<AssistantAgentInfo>,
        system_prompt: Option<Option<&str>>,
        color: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<AssistantInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_assistant_by_id(&conn, assistant_id)?;
        let next_agent = match agent {
            Some(mut value) => {
                value.id = value.id.trim().to_string();
                value.name = value.name.trim().to_string();
                value.model = value.model.trim().to_string();
                value.mode = value.mode.trim().to_string();
                value.effort = value.effort.trim().to_string();
                if value.id.is_empty() {
                    anyhow::bail!("assistant agent id cannot be empty");
                }
                if value.model.is_empty() {
                    anyhow::bail!("assistant model cannot be empty");
                }
                if value.mode.is_empty() {
                    anyhow::bail!("assistant permission mode cannot be empty");
                }
                if value.effort.is_empty() {
                    anyhow::bail!("assistant effort cannot be empty");
                }
                let db_agent = load_agent_by_id(&conn, &value.id)?;
                value.name = db_agent.name;
                value
            }
            None => current.agent,
        };
        let next_name = match name {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    anyhow::bail!("assistant name cannot be empty");
                }
                value.to_string()
            }
            None => current.name,
        };
        let next_system_prompt = match system_prompt {
            Some(Some(value)) => {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.trim().to_string())
                }
            }
            Some(None) => None,
            None => current.system_prompt,
        };
        let next_color = match color {
            Some(Some(value)) => {
                if value.trim().is_empty() {
                    None
                } else {
                    Some(value.trim().to_string())
                }
            }
            Some(None) => None,
            None => current.color,
        };
        let next_enabled = enabled.unwrap_or(current.enabled);
        if current.enabled && !next_enabled {
            ensure_assistant_can_be_disabled(&conn, assistant_id)?;
        }
        let next_agent_json = serde_json::to_string(&next_agent)?;
        conn.execute(
            "UPDATE assistants
             SET name = ?, agent_json = ?, system_prompt = ?, color = ?, enabled = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_name,
                next_agent_json,
                next_system_prompt,
                next_color,
                next_enabled as i64,
                now_ms(),
                assistant_id,
            ],
        )?;
        load_assistant_by_id(&conn, assistant_id)
    }

    fn delete_assistant(&self, assistant_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        load_assistant_by_id(&conn, assistant_id)?;
        let stage_count: i64 = conn.query_row(
            "SELECT
                (SELECT count(*) FROM thread_stage_assistants WHERE assistant_id = ?) +
                (SELECT count(*) FROM stage_assistants WHERE assistant_id = ?) +
                (SELECT count(*) FROM thread_assistants WHERE assistant_id = ?)",
            params![assistant_id, assistant_id, assistant_id],
            |row| row.get(0),
        )?;
        if stage_count > 0 {
            anyhow::bail!("assistant is used by stages or threads");
        }
        conn.execute("DELETE FROM assistants WHERE id = ?", params![assistant_id])?;
        Ok(())
    }

    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>> {
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, goal, description, stage_id, kind, enabled, created_at, updated_at,
                    origin, scheduled_task_id
             FROM threads
             WHERE project_id = ?
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let mut threads = stmt
            .query_map(params![project_id], thread_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for thread in threads.iter_mut() {
            thread.assistants = load_thread_assistants(&conn, &thread.id)?;
            thread.agent_participants = load_thread_agents(&conn, &thread.id)?;
            thread.stages = load_thread_stages(&conn, &thread.id)?;
            thread.sessions = load_thread_sessions(&conn, &thread.id)?;
        }
        Ok(threads)
    }

    fn list_thread_index(&self, project_id: Option<&str>) -> Result<Vec<ThreadIndexItemInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "WITH base AS (
                SELECT t.id, t.project_id, t.goal, t.kind, t.created_at, t.updated_at,
                       t.origin, t.scheduled_task_id
                FROM threads t
                INNER JOIN projects p ON p.id = t.project_id AND p.archived = 0
                WHERE (?1 IS NULL OR t.project_id = ?1)
             ), thread_times AS (
                SELECT id AS thread_id, created_at AS time FROM base
                UNION ALL SELECT id, updated_at FROM base
                UNION ALL SELECT b.id, ts.created_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id
                UNION ALL SELECT b.id, ts.updated_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id
                UNION ALL SELECT b.id, tss.created_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
                UNION ALL SELECT b.id, tss.updated_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN thread_stage_states tss ON tss.thread_stage_id = ts.id
                UNION ALL SELECT b.id, s.created_at FROM base b INNER JOIN thread_sessions s ON s.thread_id = b.id
                UNION ALL SELECT b.id, COALESCE(sess.updated_at, sess.started_at) FROM base b INNER JOIN thread_sessions s ON s.thread_id = b.id INNER JOIN sessions sess ON sess.agent = s.agent AND sess.session_id = s.session_id
                UNION ALL SELECT b.id, ss.created_at FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN stage_sessions ss ON ss.thread_stage_id = ts.id
                UNION ALL SELECT b.id, COALESCE(sess.updated_at, sess.started_at) FROM base b INNER JOIN thread_stages ts ON ts.thread_id = b.id INNER JOIN stage_sessions ss ON ss.thread_stage_id = ts.id INNER JOIN sessions sess ON sess.agent = ss.agent AND sess.session_id = ss.session_id
                UNION ALL SELECT b.id, r.created_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id
                UNION ALL SELECT b.id, r.updated_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id
                UNION ALL SELECT b.id, t.created_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id
                UNION ALL SELECT b.id, t.updated_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id
                UNION ALL SELECT b.id, pts.created_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id INNER JOIN thread_plan_task_sessions pts ON pts.task_id = t.id AND pts.superseded_at IS NULL
                UNION ALL SELECT b.id, pts.updated_at FROM base b INNER JOIN thread_plan_rounds r ON r.thread_id = b.id INNER JOIN thread_plan_tasks t ON t.round_id = r.id INNER JOIN thread_plan_task_sessions pts ON pts.task_id = t.id AND pts.superseded_at IS NULL
                UNION ALL SELECT b.id, ar.created_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id
                UNION ALL SELECT b.id, ar.updated_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id
                UNION ALL SELECT b.id, ars.created_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id INNER JOIN astra_run_sessions ars ON ars.run_id = ar.run_id
                UNION ALL SELECT b.id, ars.updated_at FROM base b INNER JOIN astra_runs ar ON ar.thread_id = b.id INNER JOIN astra_run_sessions ars ON ars.run_id = ar.run_id
             )
             SELECT b.id, b.project_id, b.goal, b.kind, b.created_at, b.updated_at, MAX(tt.time) AS time,
                    b.origin, b.scheduled_task_id
             FROM base b
             INNER JOIN thread_times tt ON tt.thread_id = b.id
             GROUP BY b.id, b.project_id, b.goal, b.kind, b.created_at, b.updated_at, b.origin, b.scheduled_task_id
             ORDER BY time DESC, b.updated_at DESC, b.created_at DESC",
        )?;
        let rows = stmt.query_map(params![project_id], thread_index_from_row)?;
        let mut items = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut keys_by_thread = load_thread_index_session_keys(&conn, project_id)?;
        for item in items.iter_mut() {
            if let Some(keys) = keys_by_thread.remove(&item.thread_id) {
                let mut keys = keys.into_iter().collect::<Vec<_>>();
                keys.sort();
                item.session_keys = keys;
            }
        }
        Ok(items)
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
        self.create_thread_with_options(
            project_id,
            goal,
            description,
            ThreadKind::Process,
            &[],
            &[],
        )
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
        self.create_thread_with_origin(
            project_id,
            goal,
            description,
            kind,
            assistant_ids,
            agent_participants,
            ThreadOrigin::Manual,
            None,
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
        let goal = goal.trim();
        if goal.is_empty() {
            anyhow::bail!("thread goal cannot be empty");
        }
        let description = description.map(str::trim).filter(|s| !s.is_empty());
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        load_project_by_id(&tx, project_id)?;
        let assistants = validate_assistants_for_project(&tx, project_id, assistant_ids)?;
        let now = now_ms();
        let id = stable_thread_id(project_id, goal, now);
        tx.execute(
            "INSERT INTO threads (id, project_id, goal, description, stage_id, kind, enabled,
                                  origin, scheduled_task_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, NULL, ?, 1, ?, ?, ?, ?)",
            params![
                id,
                project_id,
                goal,
                description,
                kind.as_str(),
                origin.as_str(),
                scheduled_task_id,
                now,
                now
            ],
        )?;
        replace_thread_assistants(&tx, &id, &assistants, now)?;
        replace_thread_agents(&tx, &id, agent_participants, now)?;
        let thread = load_thread_by_id(&tx, &id)?;
        tx.commit()?;
        Ok(thread)
    }

    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<ThreadInfo> {
        self.update_thread_with_options(thread_id, goal, description, enabled, None, None, None)
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
        let tx = conn.transaction()?;
        let current = load_thread_by_id(&tx, thread_id)?;
        let next_goal = match goal {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    anyhow::bail!("thread goal cannot be empty");
                }
                value.to_string()
            }
            None => current.goal,
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
        let next_enabled = enabled.unwrap_or(current.enabled);
        let next_kind = kind.unwrap_or(current.kind);
        let assistant_bindings = assistant_ids
            .map(|ids| validate_assistants_for_project(&tx, &current.project_id, ids))
            .transpose()?;
        let now = now_ms();
        tx.execute(
            "UPDATE threads
             SET goal = ?, description = ?, kind = ?, enabled = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_goal,
                next_description,
                next_kind.as_str(),
                next_enabled as i64,
                now,
                thread_id
            ],
        )?;
        if let Some(assistants) = assistant_bindings.as_deref() {
            replace_thread_assistants(&tx, thread_id, assistants, now)?;
        }
        if let Some(participants) = agent_participants {
            replace_thread_agents(&tx, thread_id, participants, now)?;
        }
        let thread = load_thread_by_id(&tx, thread_id)?;
        tx.commit()?;
        Ok(thread)
    }

    fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        // Collect every session this thread references through any link
        // table. ON DELETE CASCADE on `threads` wipes those rows out for us;
        // we then call downgrade per identity to restore sidebar visibility
        // for any session that's no longer attached anywhere.
        let mut session_refs: HashSet<(Agent, String)> = HashSet::new();
        for sql in [
            "SELECT agent, session_id FROM thread_sessions WHERE thread_id = ?",
            "SELECT s.agent, s.session_id FROM stage_sessions s
               INNER JOIN thread_stages ts ON ts.id = s.thread_stage_id
               WHERE ts.thread_id = ?",
            "SELECT s.agent, s.session_id FROM thread_plan_task_sessions s
               INNER JOIN thread_plan_tasks t ON t.id = s.task_id
               INNER JOIN thread_plan_rounds r ON r.id = t.round_id
               WHERE r.thread_id = ?",
            "SELECT s.agent, s.session_id FROM astra_run_sessions s
               INNER JOIN astra_runs r ON r.run_id = s.run_id
               WHERE r.thread_id = ?",
        ] {
            let mut stmt = tx.prepare(sql)?;
            let rows = stmt
                .query_map(params![thread_id], |row| {
                    let agent_str: String = row.get(0)?;
                    let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                    Ok((agent, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for entry in rows {
                session_refs.insert(entry);
            }
        }
        let changed = tx.execute("DELETE FROM threads WHERE id = ?", params![thread_id])?;
        if changed == 0 {
            anyhow::bail!("thread not found: {thread_id}");
        }
        for (agent, session_id) in &session_refs {
            downgrade_session_origin_when_unlinked(&tx, *agent, session_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn create_plan_round(&self, round: NewPlanRound<'_>) -> Result<PlanRoundInfo> {
        validate_new_plan_round_invariants(&round)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        load_thread_by_id(&tx, round.thread_id)?;
        if let Some(astra_run_id) = round.astra_run_id {
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM astra_runs WHERE run_id = ? LIMIT 1",
                    params![astra_run_id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_none() {
                anyhow::bail!("Astra run not found: {astra_run_id}");
            }
        }
        let round_index = match round.round_index {
            Some(value) if value < 0 => anyhow::bail!("round index cannot be negative"),
            Some(value) => value,
            None => tx.query_row(
                "SELECT COALESCE(MAX(round_index), -1) + 1
                 FROM thread_plan_rounds
                 WHERE thread_id = ?",
                params![round.thread_id],
                |row| row.get(0),
            )?,
        };
        let now = now_ms();
        let id = stable_plan_round_id(round.thread_id, round_index, now, &unique_nonce());
        let summary = clean_optional(round.summary);
        tx.execute(
            "INSERT INTO thread_plan_rounds (
                id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                round.thread_id,
                round.astra_run_id,
                round_index,
                summary,
                round.mode.as_str(),
                round.source.as_str(),
                round.status.as_str(),
                now,
                now,
            ],
        )?;
        for task in &round.tasks {
            insert_plan_task(&tx, &id, round.thread_id, task, now, &unique_nonce())?;
        }
        if !round.tasks.is_empty() {
            update_plan_round_status_from_tasks(&tx, &id, now)?;
        }
        let loaded = load_plan_round_by_id(&tx, &id)?;
        tx.commit()?;
        Ok(loaded)
    }

    fn get_plan_round(&self, round_id: &str) -> Result<Option<PlanRoundInfo>> {
        let conn = self.conn.lock().unwrap();
        let round = conn
            .query_row(
                "SELECT id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
                 FROM thread_plan_rounds
                 WHERE id = ?",
                params![round_id],
                plan_round_from_row,
            )
            .optional()?;
        match round {
            Some(mut round) => {
                round.tasks = load_plan_tasks(&conn, &round.id)?;
                Ok(Some(round))
            }
            None => Ok(None),
        }
    }

    fn list_plan_rounds(&self, thread_id: &str) -> Result<Vec<PlanRoundInfo>> {
        let conn = self.conn.lock().unwrap();
        load_thread_by_id(&conn, thread_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, astra_run_id, round_index, summary, mode, source, status, created_at, updated_at
             FROM thread_plan_rounds
             WHERE thread_id = ?
             ORDER BY round_index ASC, created_at ASC",
        )?;
        let mut rounds = stmt
            .query_map(params![thread_id], plan_round_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for round in rounds.iter_mut() {
            round.tasks = load_plan_tasks(&conn, &round.id)?;
        }
        Ok(rounds)
    }

    fn get_plan_task_thread_id(&self, task_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT r.thread_id
             FROM thread_plan_tasks t
             INNER JOIN thread_plan_rounds r ON r.id = t.round_id
             WHERE t.id = ?",
            params![task_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(Into::into)
    }

    fn update_plan_task_status(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanTaskInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let task = apply_plan_task_status_patch(&tx, task_id, patch, now_ms())?;
        tx.commit()?;
        Ok(task)
    }

    fn complete_plan_task_and_start_next(
        &self,
        task_id: &str,
        patch: PlanTaskStatusPatch<'_>,
    ) -> Result<PlanRoundInfo> {
        if !patch.status.is_terminal() {
            anyhow::bail!("sequential transition requires a terminal task status");
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_plan_task_by_id(&tx, task_id)?;
        let round = load_plan_round_by_id(&tx, &current.round_id)?;
        if round.mode != PlanRoundMode::Sequential {
            anyhow::bail!("plan round is not sequential");
        }
        let now = now_ms();
        apply_plan_task_status_patch(&tx, task_id, patch, now)?;
        ensure_no_other_running_task(&tx, &current.round_id, None)?;
        let next_task_id: Option<String> = tx
            .query_row(
                "SELECT id
                 FROM thread_plan_tasks
                 WHERE round_id = ? AND status = 'planned'
                 ORDER BY sort_order ASC, created_at ASC
                 LIMIT 1",
                params![current.round_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(next_task_id) = next_task_id {
            apply_plan_task_status_patch(
                &tx,
                &next_task_id,
                PlanTaskStatusPatch {
                    status: PlanTaskStatus::Running,
                    result_summary: None,
                    error: None,
                },
                now,
            )?;
        } else {
            update_plan_round_status_from_tasks(&tx, &current.round_id, now)?;
        }
        let loaded = load_plan_round_by_id(&tx, &current.round_id)?;
        tx.commit()?;
        Ok(loaded)
    }

    fn link_plan_task_session(
        &self,
        session: NewPlanTaskSession<'_>,
    ) -> Result<PlanTaskSessionInfo> {
        let conn = self.conn.lock().unwrap();
        load_plan_task_by_id(&conn, session.task_id)?;
        let now = now_ms();
        let attempt_count = session.attempt_count.max(1);
        let attempt_id = session
            .attempt_id
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let superseded_at = if session.role == PlanTaskSessionRole::Runtime {
            let delegated_exists = conn
                .query_row(
                    "SELECT 1
                     FROM thread_plan_task_sessions
                     WHERE task_id = ? AND agent = ? AND role = 'delegated' AND attempt_count = ?
                       AND superseded_at IS NULL
                     LIMIT 1",
                    params![session.task_id, session.agent.as_str(), attempt_count],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            delegated_exists.then_some(now)
        } else {
            None
        };
        let superseded_session_refs = {
            let mut stmt = conn.prepare(
                "SELECT agent, session_id
                 FROM thread_plan_task_sessions
                 WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
                   AND session_id != ? AND superseded_at IS NULL",
            )?;
            let refs = stmt
                .query_map(
                    params![
                        session.task_id,
                        session.agent.as_str(),
                        session.role.as_str(),
                        attempt_count,
                        session.session_id,
                    ],
                    |row| {
                        let agent_str: String = row.get(0)?;
                        let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                        Ok((agent, row.get::<_, String>(1)?))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            refs
        };
        conn.execute(
            "UPDATE thread_plan_task_sessions
             SET superseded_at = COALESCE(superseded_at, ?), updated_at = ?
             WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
               AND session_id != ? AND superseded_at IS NULL",
            params![
                now,
                now,
                session.task_id,
                session.agent.as_str(),
                session.role.as_str(),
                attempt_count,
                session.session_id,
            ],
        )?;
        conn.execute(
            "INSERT INTO thread_plan_task_sessions (
                task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(task_id, agent, session_id, role) DO UPDATE SET
                attempt_id = excluded.attempt_id,
                attempt_count = excluded.attempt_count,
                superseded_at = excluded.superseded_at,
                updated_at = excluded.updated_at",
            params![
                session.task_id,
                session.agent.as_str(),
                session.session_id,
                session.role.as_str(),
                attempt_id,
                attempt_count,
                superseded_at,
                now,
                now,
            ],
        )?;
        if superseded_at.is_some() {
            downgrade_session_origin_when_unlinked(&conn, session.agent, session.session_id)?;
        } else {
            upgrade_session_origin_to_thread(&conn, session.agent, session.session_id)?;
        }
        for (agent, session_id) in &superseded_session_refs {
            downgrade_session_origin_when_unlinked(&conn, *agent, session_id)?;
        }
        conn.query_row(
            "SELECT task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
             FROM thread_plan_task_sessions
             WHERE task_id = ? AND agent = ? AND session_id = ? AND role = ?",
            params![
                session.task_id,
                session.agent.as_str(),
                session.session_id,
                session.role.as_str(),
            ],
            plan_task_session_from_row,
        )
        .map_err(Into::into)
    }

    fn relink_plan_task_session(
        &self,
        from: NewPlanTaskSession<'_>,
        to_session_id: &str,
        to_role: PlanTaskSessionRole,
    ) -> Result<PlanTaskSessionInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        load_plan_task_by_id(&tx, from.task_id)?;
        let now = now_ms();
        let existing_attempt = tx
            .query_row(
                "SELECT attempt_id, attempt_count, created_at
                 FROM thread_plan_task_sessions
                 WHERE task_id = ? AND agent = ? AND session_id = ? AND role = ?",
                params![
                    from.task_id,
                    from.agent.as_str(),
                    from.session_id,
                    from.role.as_str(),
                ],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let attempt_id = existing_attempt
            .as_ref()
            .and_then(|(attempt_id, _, _)| attempt_id.as_deref())
            .or_else(|| {
                from.attempt_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            });
        let attempt_count = existing_attempt
            .as_ref()
            .map(|(_, attempt_count, _)| *attempt_count)
            .unwrap_or_else(|| from.attempt_count.max(1));
        let existing_created_at = existing_attempt
            .as_ref()
            .map(|(_, _, created_at)| *created_at)
            .unwrap_or(now);
        let superseded_session_refs = {
            let mut stmt = tx.prepare(
                "SELECT agent, session_id
                 FROM thread_plan_task_sessions
                 WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
                   AND session_id != ? AND superseded_at IS NULL",
            )?;
            let refs = stmt
                .query_map(
                    params![
                        from.task_id,
                        from.agent.as_str(),
                        from.role.as_str(),
                        attempt_count,
                        to_session_id,
                    ],
                    |row| {
                        let agent_str: String = row.get(0)?;
                        let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                        Ok((agent, row.get::<_, String>(1)?))
                    },
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            refs
        };
        tx.execute(
            "UPDATE thread_plan_task_sessions
             SET superseded_at = COALESCE(superseded_at, ?), updated_at = ?
             WHERE task_id = ? AND agent = ? AND role = ? AND attempt_count = ?
               AND session_id != ? AND superseded_at IS NULL",
            params![
                now,
                now,
                from.task_id,
                from.agent.as_str(),
                from.role.as_str(),
                attempt_count,
                to_session_id,
            ],
        )?;
        tx.execute(
            "INSERT INTO thread_plan_task_sessions (
                task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
             )
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?, ?)
             ON CONFLICT(task_id, agent, session_id, role) DO UPDATE SET
                attempt_id = excluded.attempt_id,
                attempt_count = excluded.attempt_count,
                superseded_at = excluded.superseded_at,
                updated_at = excluded.updated_at",
            params![
                from.task_id,
                from.agent.as_str(),
                to_session_id,
                to_role.as_str(),
                attempt_id,
                attempt_count,
                existing_created_at,
                now,
            ],
        )?;
        upgrade_session_origin_to_thread(&tx, from.agent, to_session_id)?;
        for (agent, session_id) in &superseded_session_refs {
            downgrade_session_origin_when_unlinked(&tx, *agent, session_id)?;
        }
        let linked = tx.query_row(
            "SELECT task_id, agent, session_id, role, attempt_id, attempt_count, superseded_at, created_at, updated_at
             FROM thread_plan_task_sessions
             WHERE task_id = ? AND agent = ? AND session_id = ? AND role = ?",
            params![
                from.task_id,
                from.agent.as_str(),
                to_session_id,
                to_role.as_str(),
            ],
            plan_task_session_from_row,
        )?;
        tx.commit()?;
        Ok(linked)
    }

    fn list_plan_task_sessions(&self, task_id: &str) -> Result<Vec<PlanTaskSessionInfo>> {
        let conn = self.conn.lock().unwrap();
        load_plan_task_by_id(&conn, task_id)?;
        load_plan_task_sessions(&conn, task_id)
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
        let mut stmt = conn.prepare(
            "SELECT transport_kind, detected_version, protocol_version,
                    raw_initialize_response_json, raw_capabilities_json, updated_at
             FROM runtime_agent_capabilities
             WHERE agent = ?",
        )?;
        stmt.query_row(params![agent.as_str()], |row| {
            let transport_kind: String = row.get(0)?;
            Ok(RuntimeAgentCapabilityRecord {
                agent,
                transport: transport_kind_from_db(&transport_kind),
                version: row.get(1)?,
                protocol_version: row.get(2)?,
                raw_initialize_response_json: row.get(3)?,
                raw_capabilities_json: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })
        .optional()
        .map_err(Into::into)
    }

    fn upsert_runtime_agent_capability(&self, record: &RuntimeAgentCapabilityRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO runtime_agent_capabilities (
                agent, transport_kind, detected_version, protocol_version,
                raw_initialize_response_json, raw_capabilities_json, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(agent) DO UPDATE SET
                transport_kind = excluded.transport_kind,
                detected_version = excluded.detected_version,
                protocol_version = excluded.protocol_version,
                raw_initialize_response_json = excluded.raw_initialize_response_json,
                raw_capabilities_json = excluded.raw_capabilities_json,
                updated_at = excluded.updated_at",
            params![
                record.agent.as_str(),
                transport_kind_to_db(record.transport),
                record.version,
                record.protocol_version,
                record.raw_initialize_response_json,
                record.raw_capabilities_json,
                record.updated_at,
            ],
        )?;
        Ok(())
    }

    fn get_runtime_agent_session_config(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<Option<RuntimeAgentSessionConfigRecord>> {
        let Some(adapter_version) = normalize_adapter_version_key(adapter_version) else {
            return Ok(None);
        };
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT agent, adapter_version, available_commands_json,
                    config_options_json, created_at, updated_at
             FROM runtime_agent_session_configs
             WHERE agent = ? AND adapter_version = ?",
            params![agent.as_str(), adapter_version],
            runtime_agent_session_config_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn list_runtime_agent_session_configs(
        &self,
        agent: Agent,
    ) -> Result<Vec<RuntimeAgentSessionConfigRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent, adapter_version, available_commands_json,
                    config_options_json, created_at, updated_at
             FROM runtime_agent_session_configs
             WHERE agent = ?
             ORDER BY updated_at DESC, adapter_version ASC",
        )?;
        let rows = stmt.query_map(
            params![agent.as_str()],
            runtime_agent_session_config_from_row,
        )?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn mark_runtime_agent_session_config_needs_refresh(
        &self,
        agent: Agent,
        adapter_version: &str,
    ) -> Result<()> {
        let Some(adapter_version) = normalize_adapter_version_key(adapter_version) else {
            return Ok(());
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM runtime_agent_session_configs
             WHERE agent = ? AND adapter_version = ?",
            params![agent.as_str(), adapter_version],
        )?;
        Ok(())
    }

    fn upsert_runtime_agent_session_config(
        &self,
        record: &RuntimeAgentSessionConfigRecord,
    ) -> Result<()> {
        let Some(adapter_version) = normalize_adapter_version_key(&record.adapter_version) else {
            return Ok(());
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO runtime_agent_session_configs (
                agent, adapter_version, available_commands_json,
                config_options_json, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(agent, adapter_version) DO UPDATE SET
                available_commands_json = excluded.available_commands_json,
                config_options_json = excluded.config_options_json,
                updated_at = excluded.updated_at",
            params![
                record.agent.as_str(),
                adapter_version,
                record.available_commands_json,
                record.config_options_json,
                record.created_at,
                record.updated_at,
            ],
        )?;
        Ok(())
    }

    fn get_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Vec<SessionHistorySnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        load_session_history_snapshots(&conn, child_agent, child_session_id)
    }

    fn replace_session_history_snapshots(
        &self,
        child_agent: Agent,
        child_session_id: &str,
        snapshots: &[SessionHistorySnapshotRecord],
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        replace_session_history_snapshots_inner(&tx, child_agent, child_session_id, snapshots)?;
        tx.commit()?;
        Ok(())
    }

    fn save_thread_work_snapshot(&self, snapshot: &ThreadWorkSnapshotRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO thread_work_snapshots
                (child_agent, child_session_id, thread_id, stage_id, snapshot_json, version, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(child_agent, child_session_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                stage_id = excluded.stage_id,
                snapshot_json = excluded.snapshot_json,
                version = excluded.version,
                created_at = excluded.created_at",
            params![
                snapshot.child_agent.as_str(),
                snapshot.child_session_id,
                snapshot.thread_id,
                snapshot.stage_id,
                snapshot.snapshot_json,
                snapshot.version,
                snapshot.created_at,
            ],
        )?;
        Ok(())
    }

    fn get_thread_work_snapshot(
        &self,
        child_agent: Agent,
        child_session_id: &str,
    ) -> Result<Option<ThreadWorkSnapshotRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT child_agent, child_session_id, thread_id, stage_id, snapshot_json, version, created_at
             FROM thread_work_snapshots
             WHERE child_agent = ? AND child_session_id = ?",
            params![child_agent.as_str(), child_session_id],
            |row| {
                let agent_raw: String = row.get(0)?;
                Ok(ThreadWorkSnapshotRecord {
                    child_agent: Agent::from_db_str(&agent_raw).unwrap_or(child_agent),
                    child_session_id: row.get(1)?,
                    thread_id: row.get(2)?,
                    stage_id: row.get(3)?,
                    snapshot_json: row.get(4)?,
                    version: row.get(5)?,
                    created_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    fn replace_astra_run_sessions(
        &self,
        run_id: &str,
        sessions: &[AstraRunSessionRecord],
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        replace_astra_run_sessions(&conn, run_id, sessions)
    }

    fn list_astra_run_sessions(&self, run_id: &str) -> Result<Vec<AstraRunSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        list_astra_run_sessions(&conn, run_id)
    }

    fn list_astra_run_sessions_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<Vec<AstraRunSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.run_id, s.agent, s.session_id, s.role, s.sort_order, s.created_at, s.updated_at
             FROM astra_run_sessions s
             INNER JOIN astra_runs r ON r.run_id = s.run_id
             WHERE r.thread_id = ?
             ORDER BY r.updated_at DESC, r.created_at DESC, s.sort_order ASC, s.created_at ASC",
        )?;
        let rows = stmt.query_map(params![thread_id], astra_run_session_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn upsert_astra_run(&self, run: &AstraRunRecord) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO astra_runs (
                run_id, thread_id, project_id, project_path, status, mode,
                planner_backend, round_index, round_limit, terminal_reason,
                last_error_code, last_error_message, run_diagnostics_json,
                error, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(run_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                project_id = excluded.project_id,
                project_path = excluded.project_path,
                status = excluded.status,
                mode = excluded.mode,
                planner_backend = excluded.planner_backend,
                round_index = excluded.round_index,
                round_limit = excluded.round_limit,
                terminal_reason = excluded.terminal_reason,
                last_error_code = excluded.last_error_code,
                last_error_message = excluded.last_error_message,
                run_diagnostics_json = excluded.run_diagnostics_json,
                error = excluded.error,
                updated_at = excluded.updated_at",
            params![
                run.run_id,
                run.thread_id,
                run.project_id,
                run.project_path,
                run.status,
                run.mode,
                run.planner_backend,
                run.round_index,
                run.round_limit,
                run.terminal_reason,
                run.last_error_code,
                run.last_error_message,
                run.run_diagnostics_json,
                run.error,
                run.created_at,
                run.updated_at,
            ],
        )?;
        replace_astra_run_sessions(&tx, &run.run_id, &run.internal_planner_sessions)?;
        tx.commit()?;
        Ok(())
    }

    fn get_astra_run(&self, run_id: &str) -> Result<Option<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!("SELECT {ASTRA_RUN_SELECT} FROM astra_runs WHERE run_id = ?");
        let mut run = conn
            .query_row(&sql, params![run_id], astra_run_from_row_without_sessions)
            .optional()
            .map_err(anyhow::Error::from)?;
        if let Some(run) = run.as_mut() {
            run.internal_planner_sessions = list_astra_run_sessions(&conn, &run.run_id)?;
        }
        Ok(run)
    }

    fn get_active_astra_run(&self, thread_id: &str) -> Result<Option<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {ASTRA_RUN_SELECT}
             FROM astra_runs
             WHERE thread_id = ?
               AND status IN ({ACTIVE_ASTRA_RUN_STATUS_SQL})
             ORDER BY updated_at DESC
             LIMIT 1"
        );
        let mut run = conn
            .query_row(
                &sql,
                params![thread_id],
                astra_run_from_row_without_sessions,
            )
            .optional()
            .map_err(anyhow::Error::from)?;
        if let Some(run) = run.as_mut() {
            run.internal_planner_sessions = list_astra_run_sessions(&conn, &run.run_id)?;
        }
        Ok(run)
    }

    fn list_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let sql = format!(
            "SELECT {ASTRA_RUN_SELECT}
             FROM astra_runs
             WHERE thread_id = ?
             ORDER BY updated_at DESC, created_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![thread_id], astra_run_from_row_without_sessions)?;
        let mut runs = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        hydrate_astra_run_sessions(&conn, &mut runs)?;
        Ok(runs)
    }

    fn interrupt_active_astra_runs(&self) -> Result<Vec<AstraRunRecord>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let now = now_ms();
        let mut active: Vec<AstraRunRecord> = {
            let sql = format!(
                "SELECT {ASTRA_RUN_SELECT}
                 FROM astra_runs
                 WHERE status IN ({ACTIVE_ASTRA_RUN_STATUS_SQL})"
            );
            let mut stmt = tx.prepare(&sql)?;
            let rows = stmt.query_map([], astra_run_from_row_without_sessions)?;
            let mut runs = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            hydrate_astra_run_sessions(&tx, &mut runs)?;
            runs
        };
        let mut placeholder_session_ids = HashSet::new();
        for run in &active {
            for session_id in run
                .internal_planner_sessions
                .iter()
                .map(|session| session.session_id.as_str())
            {
                if !session_id.trim().is_empty() {
                    placeholder_session_ids.insert(session_id.to_string());
                }
            }
            let mut stmt = tx.prepare(
                "SELECT DISTINCT s.session_id
                 FROM thread_plan_task_sessions s
                 INNER JOIN thread_plan_tasks t ON t.id = s.task_id
                 INNER JOIN thread_plan_rounds r ON r.id = t.round_id
                 WHERE r.astra_run_id = ?",
            )?;
            let rows = stmt.query_map(params![run.run_id], |row| row.get::<_, String>(0))?;
            for session_id in rows.collect::<rusqlite::Result<Vec<_>>>()? {
                if !session_id.trim().is_empty() {
                    placeholder_session_ids.insert(session_id);
                }
            }
        }
        let update_active_runs_sql = format!(
            "UPDATE astra_runs
             SET status = 'interrupted',
                 terminal_reason = COALESCE(terminal_reason, 'process_recovered_active_run'),
                 last_error_code = COALESCE(last_error_code, 'worker_interrupted'),
                 last_error_message = COALESCE(last_error_message, 'Astra run was active during startup recovery'),
                 error = COALESCE(error, 'Astra run was active during startup recovery'),
                 updated_at = ?
             WHERE status IN ({ACTIVE_ASTRA_RUN_STATUS_SQL})"
        );
        tx.execute(&update_active_runs_sql, params![now])?;
        for run in &active {
            tx.execute(
                "UPDATE thread_plan_tasks
                 SET status = 'errored',
                     error = COALESCE(error, 'Astra task was active during startup recovery'),
                     result_summary = COALESCE(result_summary, 'Interrupted during startup recovery'),
                     completed_at = COALESCE(completed_at, ?),
                     updated_at = ?
                 WHERE status = 'running'
                   AND round_id IN (
                       SELECT id
                       FROM thread_plan_rounds
                       WHERE astra_run_id = ?
                   )",
                params![now, now, run.run_id],
            )?;
            tx.execute(
                "UPDATE thread_plan_rounds
                 SET status = 'errored',
                     updated_at = ?
                 WHERE astra_run_id = ?
                   AND status IN ('planned', 'running')",
                params![now, run.run_id],
            )?;
        }
        for session_id in &placeholder_session_ids {
            tx.execute(
                "UPDATE sessions
                 SET available = 0, archived = 1, last_indexed_at = ?
                 WHERE session_id = ?
                   AND partial = 1
                   AND file_size = 0
                   AND available = 1",
                params![now, session_id],
            )?;
        }
        tx.commit()?;
        for run in &mut active {
            run.status = "interrupted".to_string();
            if run.terminal_reason.is_none() {
                run.terminal_reason = Some("process_recovered_active_run".to_string());
            }
            if run.last_error_code.is_none() {
                run.last_error_code = Some("worker_interrupted".to_string());
            }
            if run.last_error_message.is_none() {
                run.last_error_message =
                    Some("Astra run was active during startup recovery".to_string());
            }
            if run.error.is_none() {
                run.error = Some("Astra run was active during startup recovery".to_string());
            }
            run.updated_at = now;
        }
        Ok(active)
    }

    fn cleanup_partial_astra_sessions(&self, session_ids: &[String]) -> Result<usize> {
        if session_ids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut changed = 0usize;
        for session_id in session_ids {
            changed += tx.execute(
                "UPDATE sessions
                 SET available = 0, archived = 1, last_indexed_at = ?
                 WHERE session_id = ?
                   AND partial = 1
                   AND file_size = 0
                   AND available = 1",
                params![now_ms(), session_id],
            )?;
        }
        tx.commit()?;
        Ok(changed)
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
        // OR-ing into is_auxiliary keeps the sticky semantics: marking a
        // session auxiliary later in its lifetime is allowed, but a chat-mode
        // task session that was created with is_auxiliary=false must not flip
        // to auxiliary just because a later mark call lands.
        let aux_value = if is_auxiliary { 1 } else { 0 };
        conn.execute(
            "UPDATE sessions
                SET scheduled_task_id = ?,
                    is_auxiliary = MAX(is_auxiliary, ?)
              WHERE agent = ? AND session_id = ?",
            params![scheduled_task_id, aux_value, agent.as_str(), session_id,],
        )?;
        Ok(())
    }

    fn mark_session_origin(
        &self,
        agent: Agent,
        session_id: &str,
        origin: SessionOrigin,
    ) -> Result<()> {
        // Sticky origin: only upgrade rows whose stored origin is still the
        // default `chat`. A `thread` or `channel` row stays put. Marking with
        // `Chat` is a no-op.
        if origin == SessionOrigin::Chat {
            return Ok(());
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE sessions
                SET origin = ?
              WHERE agent = ? AND session_id = ? AND origin = 'chat'",
            params![origin.as_str(), agent.as_str(), session_id],
        )?;
        Ok(())
    }

    fn replace_by_scope(&self, scope: &str, agent: Agent, sessions: &[SessionInfo]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let new_ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        let stale_ids: Vec<String> = {
            let mut stmt =
                tx.prepare("SELECT session_id FROM sessions WHERE scope = ? AND agent = ?")?;
            let rows = stmt.query_map(params![scope, agent.as_str()], |r| r.get::<_, String>(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        for sid in &stale_ids {
            if !new_ids.contains(sid.as_str()) {
                tx.execute(
                    "UPDATE sessions SET available = 0
                     WHERE scope = ? AND agent = ? AND session_id = ?
                       AND NOT (scope LIKE 'astra://%' OR file_path LIKE 'astra://%')",
                    params![scope, agent.as_str(), sid],
                )?;
            }
        }
        for s in sessions {
            insert_session(&tx, scope, s)?;
        }
        tx.commit()?;
        Ok(())
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
        upsert_canvas_document_title(&conn, session_id, title)
    }

    fn get_canvas_document_state(&self, session_id: &str) -> Result<CanvasDocumentState> {
        let conn = self.conn.lock().unwrap();
        load_canvas_document_state(&conn, session_id)
    }

    fn save_canvas_draft(
        &self,
        session_id: &str,
        title: Option<&str>,
        draft_snapshot_path: &str,
        draft_snapshot_hash: &str,
    ) -> Result<CanvasDocumentInfo> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        let document = upsert_canvas_document_title(&conn, session_id, title)?;
        conn.execute(
            "UPDATE canvases
             SET draft_snapshot_path = ?, draft_snapshot_hash = ?, draft_updated_at = ?, updated_at = ?
             WHERE id = ?",
            params![
                draft_snapshot_path,
                draft_snapshot_hash,
                now,
                now,
                document.id,
            ],
        )?;
        get_canvas_document_by_session(&conn, session_id)?.ok_or_else(|| {
            anyhow::anyhow!("canvas document missing after draft save for {session_id}")
        })
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
        let now = now_ms();
        let nonce = unique_nonce();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let document = upsert_canvas_document_title(&tx, session_id, title)?;
        let next_revision = tx.query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM canvas_revisions WHERE canvas_id = ?",
            params![document.id.as_str()],
            |row| row.get::<_, i64>(0),
        )?;
        let revision_id = stable_canvas_revision_id(&document.id, next_revision, now, &nonce);
        tx.execute(
            "INSERT INTO canvas_revisions (
                id, canvas_id, revision, snapshot_path, snapshot_hash, snapshot_size_bytes, source, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                revision_id,
                document.id.as_str(),
                next_revision,
                snapshot_path,
                snapshot_hash,
                snapshot_size_bytes,
                source,
                now,
            ],
        )?;
        tx.execute(
            "UPDATE canvases
             SET current_saved_revision = ?, draft_snapshot_path = ?, draft_snapshot_hash = ?, draft_updated_at = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_revision,
                snapshot_path,
                snapshot_hash,
                now,
                now,
                document.id.as_str(),
            ],
        )?;
        let updated_document =
            get_canvas_document_by_session(&tx, session_id)?.ok_or_else(|| {
                anyhow::anyhow!("canvas document missing after revision save for {session_id}")
            })?;
        let revision = tx.query_row(
            "SELECT id, canvas_id, revision, snapshot_path, snapshot_hash, snapshot_size_bytes, source, created_at
             FROM canvas_revisions
             WHERE id = ?",
            params![revision_id],
            canvas_revision_from_row,
        )?;
        tx.commit()?;
        Ok((updated_document, revision))
    }

    fn prune_canvas_revisions(&self, session_id: &str, keep_latest: usize) -> Result<Vec<String>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let document = upsert_canvas_document_title(&tx, session_id, None)?;
        let stale_paths = stale_canvas_revision_paths(&tx, &document.id, keep_latest)?;
        if !stale_paths.is_empty() {
            let keep_latest = i64::try_from(keep_latest).unwrap_or(i64::MAX);
            tx.execute(
                "DELETE FROM canvas_revisions
                 WHERE id IN (
                    SELECT id
                    FROM canvas_revisions
                    WHERE canvas_id = ?
                    ORDER BY revision DESC, created_at DESC
                    LIMIT -1 OFFSET ?
                 )",
                params![document.id.as_str(), keep_latest],
            )?;
        }
        tx.commit()?;
        Ok(stale_paths)
    }

    fn replace_canvas_blocks(
        &self,
        session_id: &str,
        blocks: &[UpsertCanvasBlockRecord],
    ) -> Result<Vec<CanvasBlockRecord>> {
        let now = now_ms();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let document = upsert_canvas_document_title(&tx, session_id, None)?;
        tx.execute(
            "DELETE FROM canvas_blocks WHERE canvas_id = ?",
            params![document.id.as_str()],
        )?;
        for item in blocks {
            let id = stable_canvas_block_record_id(&document.id, &item.block_id);
            tx.execute(
                "INSERT INTO canvas_blocks (
                    id, canvas_id, block_id, block_kind, source_type, source_key, source_path,
                    metadata_json, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    document.id.as_str(),
                    item.block_id.as_str(),
                    item.block_kind.as_str(),
                    item.source_type.as_str(),
                    item.source_key.as_deref(),
                    item.source_path.as_deref(),
                    item.metadata_json.as_str(),
                    now,
                    now,
                ],
            )?;
        }
        let loaded = load_canvas_block_records(&tx, &document.id)?;
        tx.commit()?;
        Ok(loaded)
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
        let now = now_ms();
        let nonce = unique_nonce();
        let conn = self.conn.lock().unwrap();
        let document = upsert_canvas_document_title(&conn, session_id, None)?;
        let id = stable_canvas_anchor_id(
            &document.id,
            selection_block_ids_json,
            selection_element_ids_json,
            turn_id,
            now,
            &nonce,
        );
        conn.execute(
            "INSERT INTO canvas_context_anchors (
                id, canvas_id, anchor_block_id, selection_block_ids_json, selection_element_ids_json, turn_id, summary, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                document.id.as_str(),
                anchor_block_id,
                selection_block_ids_json,
                selection_element_ids_json,
                turn_id,
                summary,
                now,
            ],
        )?;
        conn.query_row(
            "SELECT id, canvas_id, anchor_block_id, selection_block_ids_json, selection_element_ids_json, turn_id, summary, created_at
             FROM canvas_context_anchors
             WHERE id = ?",
            params![id],
            canvas_anchor_from_row,
        )
        .map_err(Into::into)
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

fn is_virtual_session_ref(value: &str) -> bool {
    value.trim_start().starts_with("astra://")
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
    use crate::models::ThreadReplaySessionSourceKind;

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
                agent: Agent::AstraPi,
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
                agent: Agent::AstraPi,
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
            agent: Agent::AstraPi,
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
            child_agent: Agent::Gemini,
            child_session_id: "child".to_string(),
            ancestor_agent: Agent::Codex,
            ancestor_session_id: "parent".to_string(),
            ancestor_index: 0,
            history_cache_version: 12,
            created_at: 30,
            turns: vec![turn.clone()],
        };

        store
            .replace_session_history_snapshots(Agent::Gemini, "child", &[snapshot])
            .unwrap();
        let loaded = store
            .get_session_history_snapshots(Agent::Gemini, "child")
            .unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].child_agent, Agent::Gemini);
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
            agent: Agent::Gemini,
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
            file_path: "/tmp/project/gemini-child.jsonl".to_string(),
            file_size: 256,
            partial: false,
            title: Some("indexed".to_string()),
            first_user_message: Some("indexed".to_string()),
            forked_from_agent: None,
            forked_from_id: None,
            ..pending
        };
        store
            .replace_by_scope("/tmp/project/gemini-child.jsonl", Agent::Gemini, &[indexed])
            .unwrap();

        let row = store
            .list_all_sessions()
            .unwrap()
            .into_iter()
            .find(|session| session.agent == Agent::Gemini && session.id == "child")
            .unwrap();
        assert_eq!(row.forked_from_agent, Some(Agent::Claude));
        assert_eq!(row.forked_from_id.as_deref(), Some("parent"));
        assert_eq!(row.file_path, "/tmp/project/gemini-child.jsonl");
        assert_eq!(row.title.as_deref(), Some("indexed"));
        assert_eq!(row.first_user_message.as_deref(), Some("indexed"));
        assert!(!row.partial);
        let row_count: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM sessions WHERE agent = ? AND session_id = ?",
                params![Agent::Gemini.as_str(), "child"],
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
            forked_from_agent: Some(Agent::Gemini),
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
        assert_eq!(row.forked_from_agent, Some(Agent::Gemini));
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
            agent: Agent::AstraPi,
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
                agent: Agent::AstraPi,
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
                agent: Agent::AstraPi,
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
                params![Agent::AstraPi.as_str(), "pi-live-session"],
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
                params![Agent::AstraPi.as_str(), "pi-live-session"],
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
            forked_from_agent: Some(Agent::Gemini),
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
        assert_eq!(row.forked_from_agent, Some(Agent::Gemini));
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
        assert!(
            code_assistants
                .iter()
                .all(|item| item.project_id.as_deref() == Some(code.id.as_str()))
        );
        assert!(
            code_assistants
                .iter()
                .all(|item| item.process_template_id.as_deref()
                    == Some(code.process_template_id.as_str()))
        );

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
        assert!(
            stages
                .iter()
                .all(|stage| stage.project_id.as_deref() == Some(project.id.as_str()))
        );
        assert!(
            stages
                .iter()
                .all(|stage| stage.stage_type == ProjectStageType::Builtin)
        );
        assert!(
            stages
                .iter()
                .all(|stage| !selected_template_ids.contains(&stage.id))
        );
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
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        assert!(
            store
                .add_thread_stage(
                    &thread.id,
                    &research_template.id,
                    std::slice::from_ref(&assistant.id),
                )
                .is_err()
        );
        assert!(
            store
                .add_thread_stage(&thread.id, &project_research.id, &[assistant.id])
                .is_ok()
        );

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
        assert!(
            stages
                .iter()
                .all(|stage| stage.status == StageStatus::NotStarted)
        );

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
        assert!(
            store
                .list_thread_stage_issues(&stage.id)
                .unwrap()
                .is_empty()
        );

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
                assistant_type: AssistantType::Custom,
                process_template_id: None,
                project_id: None,
            })
            .unwrap();
        assert_eq!(assistant.process_template_id, None);
        assert_eq!(assistant.project_id, None);
        assert_eq!(assistant.assistant_type, AssistantType::Custom);

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
        assert!(
            store
                .link_kanban_item_session(&item.id, Agent::Codex, &other_session.id)
                .is_err()
        );

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
        assert!(
            parallel
                .tasks
                .iter()
                .all(|task| task.status == PlanTaskStatus::Running)
        );
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
        assert!(
            parallel.tasks[0]
                .sessions
                .iter()
                .any(|session| session.session_id == "runtime-session-stale"
                    && session.superseded_at.is_none())
        );
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
        assert!(
            sequential
                .tasks
                .iter()
                .all(|task| task.status == PlanTaskStatus::Completed)
        );

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
            agent: Agent::AstraPi,
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
                planner_backend: Some("astra_pi_acp".to_string()),
                round_index: Some(0),
                round_limit: 3,
                terminal_reason: None,
                last_error_code: None,
                last_error_message: None,
                internal_planner_sessions: vec![AstraRunSessionRecord {
                    run_id: "replay-run".to_string(),
                    agent: Agent::AstraPi,
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
        assert!(
            stage_task
                .sources
                .iter()
                .any(|source| source.kind == ThreadReplaySessionSourceKind::Stage
                    && source.stage_id.as_deref() == Some(thread_stage.id.as_str()))
        );
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
        assert_eq!(internal.agent, Agent::AstraPi);
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
        assert!(
            store
                .list_thread_index(None)
                .unwrap()
                .iter()
                .any(|item| item.thread_id == thread.id)
        );

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
            agent: Agent::AstraPi,
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
                agent: Agent::Gemini,
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
                planner_backend: Some("astra_pi_acp".to_string()),
                round_index: Some(0),
                round_limit: 3,
                terminal_reason: None,
                last_error_code: None,
                last_error_message: None,
                internal_planner_sessions: vec![
                    AstraRunSessionRecord {
                        run_id: "index-run".to_string(),
                        agent: Agent::AstraPi,
                        session_id: planner_session.id.clone(),
                        role: PlanTaskSessionRole::Planner,
                        sort_order: 0,
                        created_at: 70,
                        updated_at: 80,
                    },
                    AstraRunSessionRecord {
                        run_id: "index-run".to_string(),
                        agent: Agent::AstraPi,
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
                format!("{}:missing-runtime-session", Agent::Gemini.as_str()),
                format!("{}:planner-session", Agent::AstraPi.as_str()),
                format!("{}:missing-planner-session", Agent::AstraPi.as_str()),
            ])
        );
        assert_eq!(item.time, session_activity_time);

        // Archiving the project drops its threads from the index without
        // erroring, for both the scoped and the global listing.
        store.archive_project(&project.id).unwrap();
        assert!(
            store
                .list_thread_index(Some(&project.id))
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_thread_index(None)
                .unwrap()
                .iter()
                .all(|item| item.thread_id != thread.id)
        );

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
            .update_assistant(&builder.id, None, None, None, None, Some(false))
            .unwrap_err()
            .to_string();
        assert!(disable_error.contains("thread assistant binding(s)"));
        assert!(disable_error.contains("thread \"Teamwork lane updated\""));
        assert!(
            store
                .delete_assistant(&builder.id)
                .unwrap_err()
                .to_string()
                .contains("stages or threads")
        );

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
                    agent: Agent::Gemini,
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
            vec!["astra-pi", "pi", "codex", "claude", "opencode"]
        );
        let pi_agent = agents.iter().find(|agent| agent.id == "pi").unwrap();
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
        assert!(
            codex_agent
                .efforts
                .iter()
                .any(|option| option.value == "xhigh")
        );
        let astra_agent = agents.iter().find(|agent| agent.id == "astra-pi").unwrap();
        assert_eq!(astra_agent.display_name, "Astra Pi");
        assert_eq!(astra_agent.transport, RuntimeTransportKind::Acp);
        assert!(astra_agent.commands.session.is_empty());
        assert!(astra_agent.commands.version.is_empty());
        assert_eq!(astra_agent.ai_provider.as_deref(), Some("cc-switch"));
        assert_eq!(astra_agent.model.as_deref(), Some("gpt-5.5"));
        let astra_provider = astra_agent
            .ai_providers
            .iter()
            .find(|provider| provider.id == "cc-switch")
            .unwrap();
        assert_eq!(astra_provider.display_name, "CC Switch");
        assert_eq!(
            astra_provider.base_url.as_deref(),
            Some("http://127.0.0.1:15721/v1")
        );
        assert_eq!(astra_provider.api_key.as_deref(), Some("ccs"));
        assert_eq!(astra_provider.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(
            astra_provider
                .models
                .iter()
                .map(|option| (option.value.as_str(), option.display_name.as_str()))
                .collect::<Vec<_>>(),
            vec![("gpt-5.5", "GPT-5.5"), ("gpt-5.4", "GPT 5.4"),]
        );
        let claude_agent = agents.iter().find(|agent| agent.id == "claude").unwrap();
        assert_eq!(
            claude_agent.commands.version.first().map(String::as_str),
            Some("npm view @agentclientprotocol/claude-agent-acp version")
        );
        assert_eq!(claude_agent.effort.as_deref(), Some("high"));
        assert!(
            claude_agent
                .efforts
                .iter()
                .any(|option| option.value == "max")
        );
        assert!(agents.iter().all(|agent| agent.id != "gemini"));
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
        assert!(
            project_assistants
                .iter()
                .all(|item| item.project_id.as_deref() == Some(project.id.as_str()))
        );
        assert_eq!(
            project_assistants
                .iter()
                .filter(|item| item.assistant_type == AssistantType::Builtin)
                .count(),
            4
        );
        assert!(
            project_assistants
                .iter()
                .any(|item| item.id == assistant.id)
        );
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
        assert!(
            research_option
                .description
                .as_deref()
                .unwrap()
                .contains("technical context")
        );
        assert!(
            builtin_stages
                .iter()
                .any(|stage| stage.kind == Some(StageType::Develop))
        );
        assert!(
            builtin_stages
                .iter()
                .all(|stage| stage.kind != Some(StageType::Build))
        );
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
            .update_assistant(&assistant.id, None, None, None, None, Some(false))
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
        assert!(
            store
                .list_threads(&project.id)
                .unwrap()
                .into_iter()
                .find(|item| item.id == default_thread.id)
                .unwrap()
                .stage_id
                .is_none()
        );
        let assistant_disable_error = store
            .update_assistant(&assistant.id, None, None, None, None, Some(false))
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
        assert!(
            store
                .list_threads(&project.id)
                .unwrap()
                .into_iter()
                .find(|item| item.id == thread.id)
                .unwrap()
                .stage_id
                .is_none()
        );
        assert!(!build_option.allow_empty_assistants);
        assert!(
            store
                .add_thread_stage(&thread.id, &build_option.id, &[])
                .unwrap_err()
                .to_string()
                .contains("stage does not allow empty assistants")
        );
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
        assert!(
            store
                .list_project_stages(&project.id)
                .unwrap()
                .into_iter()
                .any(|stage| stage.id == research.stage_id && !stage.enabled)
        );
        assert!(
            store
                .list_process_template_stages(&project.process_template_id)
                .unwrap()
                .into_iter()
                .any(|stage| stage.kind == Some(StageType::Research) && stage.project_id.is_none())
        );
        assert!(
            store
                .add_thread_stage(&thread.id, &research.stage_id, &assistant_ids)
                .is_err()
        );
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
        assert!(
            store
                .link_thread_session(&thread.id, Agent::Codex, &session.id)
                .is_err()
        );
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
        assert!(
            thread_linked
                .stages
                .iter()
                .all(|stage| stage.sessions.is_empty())
        );
        assert!(
            store
                .link_stage_session(&build.id, Agent::Codex, &session.id)
                .is_err()
        );
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
        assert!(
            store
                .link_stage_session(&build.id, Agent::Codex, &other_session.id)
                .is_err()
        );

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
        assert!(
            remaining_assistants
                .iter()
                .any(|item| item.id == builtin_research_assistant_id)
        );

        assert!(
            store
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
                    assistant_type: AssistantType::Custom,
                    process_template_id: None,
                    project_id: Some(&project.id),
                })
                .is_err()
        );
        assert!(
            store
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
                    assistant_type: AssistantType::Builtin,
                    process_template_id: None,
                    project_id: Some(&project.id),
                })
                .is_err()
        );

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
        assert!(
            store
                .get_runtime_agent_session_config(Agent::Codex, "codex-acp@1.2.3")
                .unwrap()
                .is_none()
        );

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
    fn astra_pi_agent_display_name_is_explicit() {
        let path = unique_db("sessio-astra-pi-display-name");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let astra_pi = store
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|agent| agent.id == Agent::AstraPi.as_str())
            .unwrap();
        assert_eq!(astra_pi.name, "Astra Pi");
        assert_eq!(astra_pi.display_name, "Astra Pi");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn astra_pi_builtin_commands_are_not_persisted() {
        let path = unique_db("sessio-astra-pi-command-cleanup");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let legacy_commands = AgentCommandsInfo {
            session: vec!["astra-pi --acp".to_string()],
            version: vec!["astra-pi --version".to_string()],
        };
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE agents SET commands_json = ? WHERE id = ?",
                params![
                    serde_json::to_string(&legacy_commands).unwrap(),
                    Agent::AstraPi.as_str()
                ],
            )
            .unwrap();
        }

        store.init().unwrap();

        let astra_pi = store
            .list_agents()
            .unwrap()
            .into_iter()
            .find(|agent| agent.id == Agent::AstraPi.as_str())
            .unwrap();
        assert!(astra_pi.commands.session.is_empty());
        assert!(astra_pi.commands.version.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn runtime_agent_empty_model_patch_keeps_current_model() {
        let path = unique_db("runtime-empty-model");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let astra_pi = store
            .update_agent_preferences_by_id(
                Agent::AstraPi.as_str(),
                AgentPreferencesPatch {
                    model: Some(""),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(astra_pi.model.as_deref(), Some("gpt-5.5"));

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
            .find(|agent| agent.id == Agent::AstraPi.as_str())
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
                Agent::AstraPi.as_str(),
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
        assert_eq!(astra.ai_provider.as_deref(), Some("cc-switch"));
        assert!(
            astra
                .ai_providers
                .iter()
                .any(|provider| Some(provider.id.as_str()) == astra.ai_provider.as_deref())
        );

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
