use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
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
    Agent, AgentAiProviderInfo, AgentCommandsInfo, AgentInfo, AgentType, AssistantAgentInfo, AssistantInfo,
    AssistantType, IssueSeverity, IssueStatus, KanbanItem, KanbanStatus, ProjectInfo,
    ProjectStageInfo, ProjectStageType, RuntimeAgentOptionMetadata, SessionHistoryTurn,
    SessionInfo, StageAssistantInfo, StageInfo, StageIssueInfo, StageStatus, StageType,
    SubagentInfo, ThreadInfo, WorkflowInfo, WorkflowType,
};
use crate::store::{
    AstraRunRecord, IndexedSessionRecord, IndexedSubagentRecord, RuntimeAgentCapabilityRecord,
    RuntimeAgentSelection, SessionHistoryRecord, SessionHistorySnapshotRecord, SessionStore,
    ThreadWorkSnapshotRecord,
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

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
    version INTEGER PRIMARY KEY
);

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

// v2: subagents.available column (subagents can now be soft-deleted
// independently of their parent session row).
const SCHEMA_V2: &str = r#"
ALTER TABLE subagents ADD COLUMN available INTEGER NOT NULL DEFAULT 1;
CREATE INDEX IF NOT EXISTS idx_subagents_file_path ON subagents(file_path);
"#;

// V3 is the single post-v0.3.2 upgrade. It adds current memory tables and the
// initial Codex fork id column introduced after the v0.3.2 release.
const SCHEMA_V3: &str = r#"
ALTER TABLE sessions ADD COLUMN forked_from_id TEXT;

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

const SCHEMA_V4: &str = r#"
ALTER TABLE sessions ADD COLUMN title TEXT;
"#;

const SCHEMA_V5: &str = r#"
ALTER TABLE sessions ADD COLUMN forked_from_agent TEXT;

CREATE TABLE IF NOT EXISTS runtime_agent_capabilities (
    agent                TEXT PRIMARY KEY,
    transport_kind       TEXT NOT NULL,
    detected_version     TEXT,
    protocol_version     TEXT,
    raw_initialize_response_json TEXT NOT NULL,
    raw_capabilities_json TEXT NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS runtime_agent_selections (
    key             TEXT PRIMARY KEY,
    agent           TEXT NOT NULL,
    model           TEXT,
    effort          TEXT,
    permission_mode TEXT,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workflows (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    description TEXT,
    type       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom'))
);

CREATE INDEX IF NOT EXISTS idx_workflows_type_name
    ON workflows(type, name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS projects (
    id         TEXT PRIMARY KEY,
    path       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    workflow_id TEXT NOT NULL DEFAULT 'code',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived   INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(workflow_id) REFERENCES workflows(id)
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

CREATE TABLE IF NOT EXISTS session_history (
    agent           TEXT NOT NULL,
    session_id      TEXT NOT NULL,
    file_path       TEXT NOT NULL,
    file_size       INTEGER NOT NULL DEFAULT 0,
    file_mtime      INTEGER,
    history_cache_version INTEGER NOT NULL DEFAULT 0,
    message_count   INTEGER NOT NULL DEFAULT 0,
    indexed_through INTEGER,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY(agent, session_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_session_history_file_path
    ON session_history(file_path);

CREATE TABLE IF NOT EXISTS session_history_turns (
    agent      TEXT NOT NULL,
    session_id TEXT NOT NULL,
    file_path  TEXT NOT NULL,
    turn_index INTEGER NOT NULL,
    turn_id    TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    turn_json  TEXT NOT NULL,
    PRIMARY KEY(agent, session_id, file_path, turn_index),
    FOREIGN KEY(agent, session_id, file_path)
        REFERENCES session_history(agent, session_id, file_path)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_session_history_turns_turn_id
    ON session_history_turns(agent, session_id, turn_id);

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
    workflow_id    TEXT,
    project_id      TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom')),
    FOREIGN KEY(workflow_id) REFERENCES workflows(id) ON DELETE CASCADE,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_assistants_project
    ON assistants(workflow_id, project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS threads (
    id          TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    goal        TEXT NOT NULL,
    description TEXT,
    stage_id    TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE,
    FOREIGN KEY(stage_id) REFERENCES thread_stages(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_threads_project_updated
    ON threads(project_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_threads_stage
    ON threads(stage_id);

CREATE TABLE IF NOT EXISTS stages (
    id           TEXT PRIMARY KEY,
    project_id   TEXT,
    type         TEXT NOT NULL,
    workflow_id  TEXT,
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
    CHECK((type = 'builtin' AND workflow_id IS NOT NULL AND kind IS NOT NULL AND name IS NULL)
       OR (type = 'custom' AND (workflow_id IS NOT NULL OR project_id IS NOT NULL) AND kind IS NULL AND name IS NOT NULL)),
    UNIQUE(workflow_id, project_id, sort_order),
    FOREIGN KEY(workflow_id) REFERENCES workflows(id) ON DELETE CASCADE,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_stages_project
    ON stages(workflow_id, project_id, type, sort_order, kind, name);

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
"#;

const SCHEMA_V6: &str = r#"
CREATE TABLE IF NOT EXISTS thread_stage_states (
    thread_stage_id TEXT PRIMARY KEY,
    status          TEXT NOT NULL DEFAULT 'not_started',
    summary         TEXT,
    outcome         TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V7: &str = r#"
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
"#;

const SCHEMA_V8: &str = r#"
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
"#;

const SCHEMA_V9: &str = r#"
CREATE TABLE IF NOT EXISTS astra_runs (
    run_id                     TEXT PRIMARY KEY,
    thread_id                  TEXT NOT NULL,
    project_id                 TEXT NOT NULL,
    project_path               TEXT NOT NULL,
    status                     TEXT NOT NULL,
    proposed_tasks_json        TEXT NOT NULL DEFAULT '[]',
    approved_task_ids_json     TEXT NOT NULL DEFAULT '[]',
    delegated_session_ids_json TEXT NOT NULL DEFAULT '[]',
    error                      TEXT,
    created_at                 INTEGER NOT NULL,
    updated_at                 INTEGER NOT NULL,
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_astra_runs_thread_updated
    ON astra_runs(thread_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_astra_runs_thread_active
    ON astra_runs(thread_id, status);
"#;

const SCHEMA_V10: &str = r#"
ALTER TABLE astra_runs ADD COLUMN task_results_json TEXT NOT NULL DEFAULT '[]';
"#;

const SCHEMA_V11: &str = r#"
ALTER TABLE astra_runs ADD COLUMN mode TEXT NOT NULL DEFAULT 'auto';
ALTER TABLE astra_runs ADD COLUMN current_stage_id TEXT;
ALTER TABLE astra_runs ADD COLUMN completed_task_ids_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE astra_runs ADD COLUMN stage_attempt_counts_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE astra_runs ADD COLUMN retry_limit INTEGER NOT NULL DEFAULT 3;
"#;

const SCHEMA_CURRENT_SESSION_RENAME_TITLE: &str = r#"
ALTER TABLE sessions ADD COLUMN rename_title TEXT;
"#;

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

#[cfg(test)]
fn unique_suffix() -> String {
    unique_nonce()
}

fn run_migrations(conn: &Connection) -> Result<()> {
    let current: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .optional()
        .ok()
        .flatten();
    let current = current.unwrap_or(0);
    if current < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (1)",
            [],
        )?;
    }
    if current < 2 {
        // The current v1 bootstrap schema already includes this column, so
        // fresh installs can ignore the duplicate ALTER.
        let _ = conn.execute_batch(SCHEMA_V2);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (2)",
            [],
        )?;
    }
    if current < 3 {
        conn.execute_batch(SCHEMA_V3)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (3)",
            [],
        )?;
    }
    if current < 4 {
        let _ = conn.execute_batch(SCHEMA_V4);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (4)",
            [],
        )?;
    }
    if current < 5 {
        conn.execute_batch(SCHEMA_V5)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (5)",
            [],
        )?;
    }
    if current < 6 {
        // The v5 bootstrap schema already includes thread_stage_states, so
        // fresh installs can ignore the duplicate CREATE.
        let _ = conn.execute_batch(SCHEMA_V6);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (6)",
            [],
        )?;
    }
    if current < 7 {
        conn.execute_batch(SCHEMA_V7)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (7)",
            [],
        )?;
    }
    if current < 8 {
        conn.execute_batch(SCHEMA_V8)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (8)",
            [],
        )?;
    }
    if current < 9 {
        conn.execute_batch(SCHEMA_V9)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (9)",
            [],
        )?;
    }
    if current < 10 {
        let _ = conn.execute_batch(SCHEMA_V10);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (10)",
            [],
        )?;
    }
    if current < 11 {
        let _ = conn.execute_batch(SCHEMA_V11);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (11)",
            [],
        )?;
    }
    // Current unreleased schema shape: explicit user/Astra session titles live
    // outside indexed parser titles. Keep this unversioned until release.
    let _ = conn.execute_batch(SCHEMA_CURRENT_SESSION_RENAME_TITLE);
    for statement in [
        "ALTER TABLE agents ADD COLUMN ai_provider TEXT",
        "ALTER TABLE agents ADD COLUMN ai_providers_json TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE agents ADD COLUMN ai_api TEXT",
        "ALTER TABLE agents ADD COLUMN api_base_url TEXT",
        "ALTER TABLE agents ADD COLUMN api_key TEXT",
    ] {
        let _ = conn.execute(statement, []);
    }
    if current < 5 {
        seed_builtin_workflows(conn)?;
        seed_builtin_workflow_stages(conn)?;
        seed_builtin_agents(conn)?;
        seed_builtin_workflow_stage_assistants(conn, now_ms())?;
    }
    Ok(())
}

fn seed_builtin_workflows(conn: &Connection) -> Result<()> {
    let now = now_ms();
    for (id, name, description) in BUILTIN_WORKFLOW_SEEDS {
        conn.execute(
            "INSERT OR IGNORE INTO workflows (id, name, description, type, created_at, updated_at)
             VALUES (?, ?, ?, 'builtin', ?, ?)",
            params![id, name, description, now, now],
        )?;
    }
    Ok(())
}

fn seed_builtin_workflow_stages(conn: &Connection) -> Result<()> {
    let now = now_ms();
    for (workflow_id, _, _) in BUILTIN_WORKFLOW_SEEDS {
        for (index, (kind, description)) in builtin_workflow_stage_seeds(workflow_id)
            .iter()
            .copied()
            .enumerate()
        {
            let id = format!("stage-builtin-{}-{}", workflow_id, kind.as_str());
            let allow_empty_assistants = matches!(kind, StageType::Human | StageType::Done);
            conn.execute(
                "INSERT OR IGNORE INTO stages (id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at)
                 VALUES (?, NULL, 'builtin', ?, ?, NULL, ?, NULL, ?, 1, ?, ?, ?)",
                params![
                    id,
                    workflow_id,
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

fn seed_builtin_workflow_stage_assistants(conn: &Connection, now: i64) -> Result<()> {
    for (workflow_id, _, _) in BUILTIN_WORKFLOW_SEEDS {
        for (kind, _) in builtin_workflow_stage_seeds(workflow_id) {
            if matches!(kind, StageType::Human | StageType::Done) {
                continue;
            }
            let stage_id = format!("stage-builtin-{}-{}", workflow_id, kind.as_str());
            if stage_has_assistants(conn, &stage_id)? {
                continue;
            }
            let assistant_seed = builtin_assistant_seed_for_kind(kind);
            let assistant_id = stable_workflow_builtin_assistant_id(workflow_id, assistant_seed.id);
            seed_workflow_builtin_assistant(conn, workflow_id, assistant_seed.id, now)?;
            conn.execute(
                "INSERT OR IGNORE INTO stage_assistants (stage_id, assistant_id, sort_order, created_at, updated_at)
                 VALUES (?, ?, 0, ?, ?)",
                params![stage_id, assistant_id, now, now],
            )?;
        }
    }
    Ok(())
}

const BUILTIN_WORKFLOW_SEEDS: [(&str, &str, &str); 5] = [
    ("code", "Code", "workflow.description.code"),
    ("writing", "Writing", "workflow.description.writing"),
    ("research", "Research", "workflow.description.research"),
    ("general", "General", "workflow.description.general"),
    (
        "video_production",
        "Video production",
        "workflow.description.video_production",
    ),
];

fn builtin_workflow_stage_seeds(workflow_id: &str) -> Vec<(StageType, &'static str)> {
    match workflow_id {
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

fn astra_default_ai_providers() -> Vec<AgentAiProviderInfo> {
    vec![AgentAiProviderInfo {
        id: "openai".to_string(),
        display_name: "OpenAI".to_string(),
        provider: "openai".to_string(),
        api: Some("openai-responses".to_string()),
        base_url: None,
        api_key: None,
        models: runtime_options(vec![
            runtime_option("gpt-5-mini", "GPT-5 mini"),
            runtime_option("gpt-5", "GPT-5"),
        ]),
        enabled: true,
        order: 0,
    }]
}

fn seed_builtin_agent(
    conn: &Connection,
    agent: Agent,
    model: Option<&str>,
    models: Vec<RuntimeAgentOptionMetadata>,
    effort: Option<&str>,
    efforts: Vec<RuntimeAgentOptionMetadata>,
    permission_mode: Option<&str>,
    permission_modes: Vec<RuntimeAgentOptionMetadata>,
    enabled: bool,
    transport: RuntimeTransportKind,
    commands: AgentCommandsInfo,
    now: i64,
) -> Result<()> {
    let id = agent.as_str();
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
            Option::<&str>::None,
            "[]",
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

fn seed_astra_agent(conn: &Connection, now: i64) -> Result<()> {
    let ai_providers = astra_default_ai_providers();
    conn.execute(
        "INSERT OR IGNORE INTO agents (
            id, name, display_name, icon, ai_provider, ai_providers_json, ai_api, api_base_url, api_key,
            model, models_json, effort, efforts_json,
            permission_mode, permission_modes_json, type, enabled, transport,
            commands_json, sort_order, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            "astra",
            "Astra",
            "Astra",
            "astra",
            "openai",
            serde_json::to_string(&ai_providers)?,
            "openai-responses",
            Option::<&str>::None,
            Option::<&str>::None,
            "gpt-5-mini",
            runtime_options_json(&ai_providers[0].models)?,
            "off",
            serde_json::to_string(&vec![
                runtime_option("off", "Off"),
                runtime_option("minimal", "Minimal"),
                runtime_option("low", "Low"),
                runtime_option("medium", "Medium"),
                runtime_option("high", "High"),
                runtime_option("xhigh", "Extra High"),
            ])?,
            Option::<&str>::None,
            "[]",
            AgentType::Builtin.as_str(),
            1_i64,
            transport_kind_to_db(RuntimeTransportKind::PlainCli),
            serde_json::to_string(&AgentCommandsInfo::default())?,
            0_i64,
            now,
            now,
        ],
    )?;
    Ok(())
}

fn seed_builtin_agents(conn: &Connection) -> Result<()> {
    let now = now_ms();
    seed_astra_agent(conn, now)?;
    seed_builtin_agent(
        conn,
        Agent::Codex,
        Some("gpt-5.5"),
        runtime_options(vec![
            runtime_option("gpt-5.5", "5.5"),
            runtime_option("gpt-5.4", "5.4"),
            runtime_option("gpt-5.3-codex", "5.3 Codex"),
        ]),
        Some("high"),
        vec![
            runtime_option("low", "Low"),
            runtime_option("medium", "Medium"),
            runtime_option("high", "High"),
            runtime_option("xhigh", "Extra High"),
        ],
        Some("read-only"),
        vec![
            runtime_option("read-only", "Default permissions"),
            runtime_option("auto", "Auto-review"),
            runtime_option("full-access", "Full access"),
        ],
        true,
        RuntimeTransportKind::Acp,
        AgentCommandsInfo {
            session: vec!["npx -y @zed-industries/codex-acp@latest".to_string()],
            version: vec!["codex --version".to_string()],
        },
        now,
    )?;
    seed_builtin_agent(
        conn,
        Agent::Claude,
        Some("claude-opus-4-7"),
        runtime_options(vec![
            runtime_option("claude-opus-4-8", "Opus 4.8"),
            runtime_option("claude-opus-4-7", "Opus 4.7"),
            runtime_option("claude-opus-4-6", "Opus 4.6"),
        ]),
        Some("high"),
        vec![
            runtime_option("low", "Low"),
            runtime_option("medium", "Medium"),
            runtime_option("high", "High"),
            runtime_option("xhigh", "Extra High"),
            runtime_option("max", "Max"),
        ],
        Some("default"),
        vec![
            runtime_option("default", "Ask before edits"),
            runtime_option("acceptEdits", "Edit automatically"),
            runtime_option("plan", "Plan mode"),
            runtime_option("dontAsk", "Don't Ask"),
        ],
        true,
        RuntimeTransportKind::Acp,
        AgentCommandsInfo {
            session: vec!["npx -y @agentclientprotocol/claude-agent-acp@latest".to_string()],
            version: vec!["claude --version".to_string()],
        },
        now,
    )?;
    seed_builtin_agent(
        conn,
        Agent::Gemini,
        None,
        Vec::new(),
        Some("high"),
        vec![
            runtime_option("low", "Low"),
            runtime_option("medium", "Medium"),
            runtime_option("high", "High"),
        ],
        None,
        Vec::new(),
        false,
        RuntimeTransportKind::Acp,
        AgentCommandsInfo {
            session: vec!["npx -y -- @google/gemini-cli@latest --experimental-acp".to_string()],
            version: vec!["gemini --version".to_string()],
        },
        now,
    )?;
    seed_builtin_assistants(conn, now)?;
    Ok(())
}

fn transport_kind_to_db(transport: RuntimeTransportKind) -> &'static str {
    match transport {
        RuntimeTransportKind::Acp => "acp",
        RuntimeTransportKind::CliStreamJson => "cliStreamJson",
        RuntimeTransportKind::PlainCli => "plainCli",
        RuntimeTransportKind::Fake => "fake",
    }
}

fn transport_kind_from_db(value: &str) -> RuntimeTransportKind {
    match value {
        "cliStreamJson" => RuntimeTransportKind::CliStreamJson,
        "plainCli" => RuntimeTransportKind::PlainCli,
        "fake" => RuntimeTransportKind::Fake,
        _ => RuntimeTransportKind::Acp,
    }
}

fn runtime_agent_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Codex => "Codex",
        Agent::Claude => "Claude",
        Agent::Gemini => "Gemini",
    }
}

fn runtime_agent_display_name(agent: Agent) -> &'static str {
    match agent {
        Agent::Codex => "Codex CLI",
        Agent::Claude => "Claude Code",
        Agent::Gemini => "Gemini CLI",
    }
}

fn runtime_agent_order(agent: Agent) -> i64 {
    match agent {
        Agent::Codex => 1,
        Agent::Claude => 2,
        Agent::Gemini => 3,
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
}

fn load_identity_session_rows(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
) -> Result<Vec<ExistingSessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT scope, file_path, partial, available, archived,
                message_count, rename_title, title, first_user_message, forked_from_agent, forked_from_id
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
            conn.execute(
                "UPDATE sessions
                 SET project_path = COALESCE(project_path, ?),
                     project_name = COALESCE(project_name, ?),
                     started_at = COALESCE(started_at, ?),
                     updated_at = COALESCE(?, updated_at),
                     rename_title = ?,
                     title = ?,
                     first_user_message = ?,
                     message_count = ?,
                     partial = ?,
                     available = ?,
                     archived = ?,
                     last_indexed_at = ?,
                     forked_from_agent = ?,
                     forked_from_id = ?
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
                    now_ms(),
                    forked_from_agent.map(|agent| agent.as_str()),
                    forked_from_id,
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
            conn.execute(
                "UPDATE sessions
                 SET file_path = ?, project_path = ?, project_name = ?,
                     started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                     message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                     last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?
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
            conn.execute(
                "UPDATE sessions
                 SET scope = ?, file_path = ?, project_path = ?, project_name = ?,
                     started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                     message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                     last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?
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
            if conn.query_row(
                "SELECT 1 FROM sessions WHERE agent = ? AND session_id = ? AND scope = ? LIMIT 1",
                params![s.agent.as_str(), s.id, scope],
                |_| Ok(()),
            ).optional()?.is_some() {
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
                         last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?
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
                        s.agent.as_str(),
                        existing.session_id,
                        existing.scope,
                    ],
                )?;
                delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
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
        conn.execute(
            "UPDATE sessions
             SET session_id = ?, scope = ?, file_path = ?, project_path = ?, project_name = ?,
                 started_at = ?, updated_at = ?, rename_title = ?, title = ?, first_user_message = ?,
                 message_count = ?, file_size = ?, file_mtime = ?, partial = ?, available = ?, archived = ?,
                 last_indexed_at = ?, forked_from_agent = ?, forked_from_id = ?
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
                s.agent.as_str(),
                s.id,
                existing_same_scope.scope,
            ],
        )?;
        delete_duplicate_session_rows(conn, s.agent, &s.id, scope)?;
        return Ok(());
    }
    let (forked_from_agent, forked_from_id) = merge_identity_lineage(&identity_rows, s);
    conn.execute(
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
    format!(
        "project-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
}

fn stable_kanban_id(project_id: &str, title: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!(
        "kanban-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
}

fn stable_issue_id(thread_stage_id: &str, title: &str, now: i64, nonce: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(thread_stage_id.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(now.to_string().as_bytes());
    hasher.update(nonce.as_bytes());
    format!("issue-{}", hex::encode(hasher.finalize())[..16].to_string())
}

fn stable_workflow_id(name: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!(
        "workflow-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
}

fn stable_assistant_id(
    assistant_type: AssistantType,
    workflow_id: Option<&str>,
    project_id: Option<&str>,
    name: &str,
    model: &str,
    now: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(assistant_type.as_str().as_bytes());
    hasher.update(workflow_id.unwrap_or("").as_bytes());
    hasher.update(project_id.unwrap_or("").as_bytes());
    hasher.update(name.as_bytes());
    hasher.update(model.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!(
        "assistant-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
}

fn stable_project_builtin_assistant_id(project_id: &str, template_assistant_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(template_assistant_id.as_bytes());
    format!(
        "assistant-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
}

fn stable_project_assistant_id(project_id: &str, template_assistant_id: &str) -> String {
    stable_project_builtin_assistant_id(project_id, template_assistant_id)
}

fn stable_workflow_builtin_assistant_id(workflow_id: &str, source_assistant_id: &str) -> String {
    format!("assistant-workflow-{workflow_id}-{source_assistant_id}")
}

fn stable_thread_id(project_id: &str, goal: &str, now: i64) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(goal.as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!(
        "thread-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
}

fn stable_project_stage_id(
    workflow_id: Option<&str>,
    project_id: Option<&str>,
    stage_name: &str,
    order: i64,
    now: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(workflow_id.unwrap_or("").as_bytes());
    hasher.update(project_id.unwrap_or("").as_bytes());
    hasher.update(stage_name.as_bytes());
    hasher.update(order.to_string().as_bytes());
    hasher.update(now.to_string().as_bytes());
    format!("stage-{}", hex::encode(hasher.finalize())[..16].to_string())
}

fn stable_project_builtin_stage_id(project_id: &str, template_stage_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(template_stage_id.as_bytes());
    format!("stage-{}", hex::encode(hasher.finalize())[..16].to_string())
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
    format!(
        "thread-stage-{}",
        hex::encode(hasher.finalize())[..16].to_string()
    )
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
        workflow_id: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        session_count: row.get::<_, i64>(6)? as usize,
    })
}

fn workflow_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkflowInfo> {
    let workflow_type_raw: String = row.get(3)?;
    Ok(WorkflowInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        workflow_type: WorkflowType::from_db_str(&workflow_type_raw)
            .unwrap_or(WorkflowType::Custom),
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

fn assistant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantInfo> {
    let agent_json: String = row.get(2)?;
    let assistant_type_raw: String = row.get(5)?;
    let workflow_id_raw: Option<String> = row.get(6)?;
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
        workflow_id: workflow_id_raw,
        project_id: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

fn project_stage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectStageInfo> {
    let stage_type_raw: String = row.get(2)?;
    let workflow_id_raw: Option<String> = row.get(3)?;
    let stage_kind_raw: Option<String> = row.get(4)?;
    Ok(ProjectStageInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        stage_type: ProjectStageType::from_db_str(&stage_type_raw)
            .unwrap_or(ProjectStageType::Custom),
        workflow_id: workflow_id_raw,
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
    let workflow_id_raw: Option<String> = row.get(5)?;
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
        workflow_id: workflow_id_raw,
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
    Ok(ThreadInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        description: row.get(3)?,
        stage_id: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        stages: Vec::new(),
        sessions: Vec::new(),
    })
}

fn load_project_by_id(conn: &Connection, project_id: &str) -> Result<ProjectInfo> {
    conn.query_row(
        "SELECT p.id, p.path, p.name, p.workflow_id, p.created_at, p.updated_at,
                COUNT(s.session_id) AS session_count
         FROM projects p
         LEFT JOIN sessions s ON s.project_path = p.path AND s.available = 1
         WHERE p.id = ? AND p.archived = 0
         GROUP BY p.id",
        params![project_id],
        project_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("project not found: {project_id}"))
}

fn ensure_workflow_exists(conn: &Connection, workflow_id: &str) -> Result<()> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM workflows WHERE id = ? LIMIT 1",
            params![workflow_id],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        anyhow::bail!("workflow not found: {workflow_id}");
    }
    Ok(())
}

fn load_workflow_by_id(conn: &Connection, workflow_id: &str) -> Result<WorkflowInfo> {
    conn.query_row(
        "SELECT id, name, description, type, created_at, updated_at
         FROM workflows
         WHERE id = ?",
        params![workflow_id],
        workflow_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("workflow not found: {workflow_id}"))
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

fn astra_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AstraRunRecord> {
    Ok(AstraRunRecord {
        run_id: row.get(0)?,
        thread_id: row.get(1)?,
        project_id: row.get(2)?,
        project_path: row.get(3)?,
        status: row.get(4)?,
        mode: row.get(5)?,
        proposed_tasks_json: row.get(6)?,
        approved_task_ids_json: row.get(7)?,
        delegated_session_ids_json: row.get(8)?,
        task_results_json: row.get(9)?,
        current_stage_id: row.get(10)?,
        completed_task_ids_json: row.get(11)?,
        stage_attempt_counts_json: row.get(12)?,
        retry_limit: row.get(13)?,
        error: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
    })
}

fn load_assistant_by_id(conn: &Connection, assistant_id: &str) -> Result<AssistantInfo> {
    conn.query_row(
        "SELECT id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
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
            "SELECT id, project_id, goal, description, stage_id, enabled, created_at, updated_at
             FROM threads
             WHERE id = ?",
            params![thread_id],
            thread_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
    thread.stages = load_thread_stages(conn, &thread.id)?;
    thread.sessions = load_thread_sessions(conn, &thread.id)?;
    Ok(thread)
}

fn load_project_stage_by_id(conn: &Connection, stage_id: &str) -> Result<ProjectStageInfo> {
    let mut stage = conn.query_row(
        "SELECT id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
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
    workflow_id: &str,
    now: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
         FROM assistants
         WHERE project_id IS NULL
           AND enabled = 1
           AND (workflow_id = ? OR (workflow_id IS NULL AND type = 'custom'))
         ORDER BY CASE WHEN workflow_id = ? THEN 0 ELSE 1 END, type ASC, updated_at DESC, name COLLATE NOCASE ASC",
    )?;
    let templates = stmt
        .query_map(params![workflow_id, workflow_id], assistant_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for template in templates {
        let id = stable_project_assistant_id(project_id, &template.id);
        conn.execute(
            "INSERT OR IGNORE INTO assistants (
                id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
             )
             SELECT ?, name, agent_json, system_prompt, color, type, workflow_id, ?, 1, ?, ?
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
    workflow_id: &str,
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
        "SELECT id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
         FROM stages
         WHERE project_id IS NULL AND workflow_id = ? AND type = 'builtin'
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let templates = stmt
        .query_map(params![workflow_id], project_stage_from_row)?
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
                id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
             ) VALUES (?, ?, 'builtin', ?, ?, NULL, ?, ?, ?, 1, ?, ?, ?)",
            params![
                id,
                project_id,
                workflow_id,
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
    workflow_id: &str,
    now: i64,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id
         FROM stages
         WHERE project_id IS NULL AND workflow_id = ? AND type = 'builtin'
         ORDER BY sort_order ASC, created_at ASC",
    )?;
    let template_stage_ids = stmt
        .query_map(params![workflow_id], |row| row.get::<_, String>(0))?
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
            "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.workflow_id, s.kind, s.name, s.description, s.icon,
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
        && stage.workflow_id.is_some()
        && assistant.project_id.is_none()
        && assistant.workflow_id == stage.workflow_id
    {
        return Ok(assistant);
    }
    if stage.project_id.is_none()
        && stage.workflow_id.is_some()
        && assistant.project_id.is_none()
        && assistant.workflow_id.is_none()
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
    let workflow_stage_usages = {
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(w.name, 'Unknown'),
                COALESCE(s.name, s.kind, s.id)
             FROM stage_assistants sa
             INNER JOIN stages s ON s.id = sa.stage_id
             LEFT JOIN workflows w ON w.id = s.workflow_id
             WHERE sa.assistant_id = ?
               AND s.project_id IS NULL
               AND s.workflow_id IS NOT NULL
             ORDER BY w.name COLLATE NOCASE ASC, s.sort_order ASC",
        )?;
        let rows = stmt.query_map(params![assistant_id], |row| {
            let workflow_name: String = row.get(0)?;
            let stage_name: String = row.get(1)?;
            Ok(format!(
                "workflow \"{workflow_name}\" stage \"{stage_name}\""
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
    if !project_stage_usages.is_empty()
        || !workflow_stage_usages.is_empty()
        || !thread_stage_usages.is_empty()
    {
        let mut parts = Vec::new();
        if !project_stage_usages.is_empty() {
            parts.push(format!(
                "{} project stage assistant binding(s): {}",
                project_stage_usages.len(),
                usage_list(&project_stage_usages)
            ));
        }
        if !workflow_stage_usages.is_empty() {
            parts.push(format!(
                "{} workflow stage assistant binding(s): {}",
                workflow_stage_usages.len(),
                usage_list(&workflow_stage_usages)
            ));
        }
        if !thread_stage_usages.is_empty() {
            parts.push(format!(
                "{} thread stage assistant binding(s): {}",
                thread_stage_usages.len(),
                usage_list(&thread_stage_usages)
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
        "SELECT tsa.assistant_id, a.name, a.color, tsa.agent_json, tsa.sort_order
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
            order: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_project_stage_assistants(
    conn: &Connection,
    stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT sa.assistant_id, a.name, a.color, a.agent_json, sa.sort_order
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
            order: row.get(4)?,
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

fn assistant_agent_from_db_agent(agent: &AgentInfo) -> Option<AssistantAgentInfo> {
    let Some(model) = agent
        .model
        .clone()
        .or_else(|| agent.models.first().map(|option| option.value.clone()))
    else {
        return None;
    };
    let Some(mode) = agent.permission_mode.clone().or_else(|| {
        agent
            .permission_modes
            .first()
            .map(|option| option.value.clone())
    }) else {
        return None;
    };
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

fn seed_workflow_builtin_assistant(
    conn: &Connection,
    workflow_id: &str,
    source_assistant_id: &str,
    now: i64,
) -> Result<()> {
    let workflow_assistant_id =
        stable_workflow_builtin_assistant_id(workflow_id, source_assistant_id);
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
         )
         SELECT ?, name, agent_json, system_prompt, color, type, ?, NULL, enabled, ?, ?
         FROM assistants
         WHERE id = ?
         ON CONFLICT(id) DO NOTHING",
        params![
            workflow_assistant_id,
            workflow_id,
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
    BUILTIN_WORKFLOW_SEEDS
        .iter()
        .flat_map(|(workflow_id, _, _)| builtin_workflow_stage_seeds(workflow_id))
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
            system_prompt: "Draft the requested content in the selected voice, structure, and level of detail while preserving the goal, audience, and constraints.",
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
            id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
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
    workflow_id: &str,
    project_id: Option<&str>,
    target_order: i64,
) -> Result<i64> {
    let rows: Vec<(String, i64)> = {
        let mut stmt = conn.prepare(
            "SELECT id, sort_order
             FROM stages
             WHERE workflow_id = ?
               AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)
             ORDER BY sort_order ASC, type ASC, project_id IS NOT NULL ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![workflow_id, project_id, project_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
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
    let insert_index = if current_index < target_index {
        target_index
    } else {
        target_index
    };
    ids.insert(insert_index, id);

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
    let mut stmt = conn.prepare(
        "SELECT s.agent, s.session_id, s.file_path, s.project_path, s.project_name,
                s.started_at, s.updated_at, s.message_count, s.rename_title, s.title, s.first_user_message,
                s.file_size, s.partial, s.available, s.archived, s.forked_from_agent, s.forked_from_id
         FROM kanban_item_sessions kis
         INNER JOIN sessions s ON s.agent = kis.agent AND s.session_id = kis.session_id
         WHERE kis.item_id = ? AND s.available = 1
         ORDER BY s.updated_at DESC, s.started_at DESC",
    )?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![item_id], |row| {
            let agent_str: String = row.get(0)?;
            let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
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
                subagents: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    sessions.retain(|s| !is_codex_guardian_index_row(s));
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
    let mut stmt = conn.prepare(
        "SELECT s.agent, s.session_id, s.file_path, s.project_path, s.project_name,
                s.started_at, s.updated_at, s.message_count, s.rename_title, s.title, s.first_user_message,
                s.file_size, s.partial, s.available, s.archived, s.forked_from_agent, s.forked_from_id
         FROM thread_sessions ts
         INNER JOIN sessions s ON s.agent = ts.agent AND s.session_id = ts.session_id
         WHERE ts.thread_id = ? AND s.available = 1
         ORDER BY ts.created_at ASC, s.updated_at DESC, s.started_at DESC",
    )?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![thread_id], |row| {
            let agent_str: String = row.get(0)?;
            let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
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
                subagents: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    sessions.retain(|s| !is_codex_guardian_index_row(s));
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
    let mut stmt = conn.prepare(
        "SELECT s.agent, s.session_id, s.file_path, s.project_path, s.project_name,
                s.started_at, s.updated_at, s.message_count, s.rename_title, s.title, s.first_user_message,
                s.file_size, s.partial, s.available, s.archived, s.forked_from_agent, s.forked_from_id
         FROM stage_sessions ss
         INNER JOIN sessions s ON s.agent = ss.agent AND s.session_id = ss.session_id
         WHERE ss.thread_stage_id = ? AND s.available = 1
         ORDER BY ss.created_at ASC, s.updated_at DESC, s.started_at DESC",
    )?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map(params![thread_stage_id], |row| {
            let agent_str: String = row.get(0)?;
            let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
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
                subagents: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    sessions.retain(|s| !is_codex_guardian_index_row(s));
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
        "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.workflow_id, s.kind, s.name, s.description, s.icon,
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

fn ensure_session_not_linked_to_thread_workflow(
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

fn load_session_history(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    file_path: &str,
) -> Result<Option<SessionHistoryRecord>> {
    let header = conn
        .query_row(
            "SELECT file_size, file_mtime, history_cache_version, message_count, indexed_through, updated_at
             FROM session_history
             WHERE agent = ? AND session_id = ? AND file_path = ?",
            params![agent.as_str(), session_id, file_path],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((
        file_size,
        file_mtime,
        history_cache_version,
        message_count,
        indexed_through,
        updated_at,
    )) = header
    else {
        return Ok(None);
    };

    let mut stmt = conn.prepare(
        "SELECT turn_json
         FROM session_history_turns
         WHERE agent = ? AND session_id = ? AND file_path = ?
         ORDER BY turn_index ASC",
    )?;
    let rows = stmt.query_map(params![agent.as_str(), session_id, file_path], |row| {
        row.get::<_, String>(0)
    })?;
    let mut turns = Vec::new();
    for row in rows {
        let json = row?;
        turns.push(serde_json::from_str::<SessionHistoryTurn>(&json)?);
    }

    Ok(Some(SessionHistoryRecord {
        agent,
        session_id: session_id.to_string(),
        file_path: file_path.to_string(),
        file_size: file_size as u64,
        file_mtime,
        history_cache_version,
        message_count: message_count as usize,
        indexed_through,
        updated_at,
        turns,
    }))
}

fn replace_session_history_inner(conn: &Connection, record: &SessionHistoryRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO session_history (
            agent, session_id, file_path, file_size, file_mtime, history_cache_version,
            message_count, indexed_through, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(agent, session_id, file_path) DO UPDATE SET
            file_size = excluded.file_size,
            file_mtime = excluded.file_mtime,
            history_cache_version = excluded.history_cache_version,
            message_count = excluded.message_count,
            indexed_through = excluded.indexed_through,
            updated_at = excluded.updated_at",
        params![
            record.agent.as_str(),
            record.session_id.as_str(),
            record.file_path.as_str(),
            record.file_size as i64,
            record.file_mtime,
            record.history_cache_version,
            record.message_count as i64,
            record.indexed_through,
            record.updated_at,
        ],
    )?;
    conn.execute(
        "DELETE FROM session_history_turns
         WHERE agent = ? AND session_id = ? AND file_path = ?",
        params![record.agent.as_str(), record.session_id, record.file_path],
    )?;
    {
        let mut stmt = conn.prepare(
            "INSERT INTO session_history_turns (
                agent, session_id, file_path, turn_index, turn_id,
                started_at, updated_at, turn_json
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        for (index, turn) in record.turns.iter().enumerate() {
            let turn_json = serde_json::to_string(turn)?;
            stmt.execute(params![
                record.agent.as_str(),
                record.session_id.as_str(),
                record.file_path.as_str(),
                index as i64,
                turn.turn_id.as_str(),
                turn.started_at,
                turn.updated_at,
                turn_json,
            ])?;
        }
    }
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

fn load_sessions(conn: &Connection, user_projects_only: bool) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let sql = if user_projects_only {
        "SELECT s.agent, s.session_id, s.file_path, s.project_path, s.project_name,
                s.started_at, s.updated_at, s.message_count, s.rename_title, s.title, s.first_user_message,
                s.file_size, s.partial, s.available, s.archived, s.forked_from_agent, s.forked_from_id
         FROM sessions s
         INNER JOIN projects p ON p.path = s.project_path AND p.archived = 0
         ORDER BY s.updated_at DESC"
    } else {
        "SELECT agent, session_id, file_path, project_path, project_name,
                started_at, updated_at, message_count, rename_title, title, first_user_message,
                file_size, partial, available, archived, forked_from_agent, forked_from_id
         FROM sessions
         ORDER BY updated_at DESC"
    };
    let mut stmt = conn.prepare(sql)?;
    let mut sessions: Vec<SessionInfo> = stmt
        .query_map([], |row| {
            let agent_str: String = row.get(0)?;
            let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
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
                subagents: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    sessions.retain(|s| !is_codex_guardian_index_row(s));
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
        run_migrations(&conn)
    }

    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        load_sessions(&conn, true)
    }

    fn list_all_sessions(&self) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        load_sessions(&conn, false)
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

    fn list_workflows(&self) -> Result<Vec<WorkflowInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, type, created_at, updated_at
             FROM workflows
             ORDER BY type ASC, name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], workflow_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn create_workflow(&self, name: &str, description: Option<&str>) -> Result<WorkflowInfo> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("workflow name cannot be empty");
        }
        let description = description.map(str::trim).filter(|value| !value.is_empty());
        let now = now_ms();
        let id = stable_workflow_id(name, now);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO workflows (id, name, description, type, created_at, updated_at)
             VALUES (?, ?, ?, 'custom', ?, ?)",
            params![id, name, description, now, now],
        )?;
        load_workflow_by_id(&conn, &id)
    }

    fn update_workflow(
        &self,
        workflow_id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
    ) -> Result<WorkflowInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_workflow_by_id(&conn, workflow_id)?;
        if current.workflow_type == WorkflowType::Builtin {
            anyhow::bail!("builtin workflow cannot be updated");
        }
        let next_name = match name {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    anyhow::bail!("workflow name cannot be empty");
                }
                value.to_string()
            }
            None => current.name,
        };
        let next_description = match description {
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
            Some(None) => None,
            None => current.description,
        };
        conn.execute(
            "UPDATE workflows SET name = ?, description = ?, updated_at = ? WHERE id = ?",
            params![next_name, next_description, now_ms(), workflow_id],
        )?;
        load_workflow_by_id(&conn, workflow_id)
    }

    fn delete_workflow(&self, workflow_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let current = load_workflow_by_id(&conn, workflow_id)?;
        if current.workflow_type == WorkflowType::Builtin {
            anyhow::bail!("builtin workflow cannot be deleted");
        }
        let project_count: i64 = conn.query_row(
            "SELECT count(*) FROM projects WHERE workflow_id = ? AND archived = 0",
            params![workflow_id],
            |row| row.get(0),
        )?;
        if project_count > 0 {
            anyhow::bail!("workflow is used by projects");
        }
        let assistant_count: i64 = conn.query_row(
            "SELECT count(*) FROM assistants WHERE workflow_id = ?",
            params![workflow_id],
            |row| row.get(0),
        )?;
        if assistant_count > 0 {
            anyhow::bail!("workflow is used by assistants");
        }
        let stage_count: i64 = conn.query_row(
            "SELECT count(*) FROM stages WHERE workflow_id = ?",
            params![workflow_id],
            |row| row.get(0),
        )?;
        if stage_count > 0 {
            anyhow::bail!("workflow is used by stages");
        }
        let changed = conn.execute("DELETE FROM workflows WHERE id = ?", params![workflow_id])?;
        if changed == 0 {
            anyhow::bail!("workflow not found: {workflow_id}");
        }
        Ok(())
    }

    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.path, p.name, p.workflow_id, p.created_at, p.updated_at,
                    COUNT(s.session_id) AS session_count
             FROM projects p
             LEFT JOIN sessions s ON s.project_path = p.path AND s.available = 1
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
        workflow_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        let canonical = canonical_project_path(path)?;
        let name = clean_project_name(name, &canonical)?;
        let id = stable_project_id(&canonical);
        let now = now_ms();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        ensure_workflow_exists(&tx, &workflow_id)?;
        tx.execute(
            "INSERT INTO projects (id, path, name, workflow_id, created_at, updated_at, archived)
             VALUES (?, ?, ?, ?, ?, ?, 0)",
            params![id, canonical, name, workflow_id.as_str(), now, now],
        )
        .with_context(|| "add project")?;
        instantiate_project_builtin_stages(&tx, &id, &workflow_id, enabled_stage_ids, now)?;
        instantiate_project_assistants(&tx, &id, &workflow_id, now)?;
        link_project_stage_assistants(&tx, &id, &workflow_id, now)?;
        let project = load_project_by_id(&tx, &id)?;
        tx.commit()?;
        Ok(project)
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        workflow_id: String,
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
        self.add_project(&path, Some(&clean_name), workflow_id, enabled_stage_ids)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        workflow_id: Option<String>,
    ) -> Result<ProjectInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_project_by_id(&tx, project_id)?;
        let next_name = match name {
            Some(value) => clean_project_name(Some(value), &current.path)?,
            None => current.name,
        };
        let current_workflow_id = current.workflow_id.clone();
        let next_workflow_id = workflow_id.unwrap_or_else(|| current_workflow_id.clone());
        ensure_workflow_exists(&tx, &next_workflow_id)?;
        let workflow_changed = next_workflow_id != current_workflow_id;
        tx.execute(
            "UPDATE projects
             SET name = ?, workflow_id = ?, updated_at = ?
             WHERE id = ? AND archived = 0",
            params![next_name, next_workflow_id.as_str(), now_ms(), project_id],
        )?;
        if workflow_changed {
            tx.execute(
                "DELETE FROM stages WHERE project_id = ? AND type = 'builtin'",
                params![project_id],
            )?;
            instantiate_project_builtin_stages(&tx, project_id, &next_workflow_id, None, now_ms())?;
            instantiate_project_assistants(&tx, project_id, &next_workflow_id, now_ms())?;
            link_project_stage_assistants(&tx, project_id, &next_workflow_id, now_ms())?;
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

    fn update_agent_preferences_by_id(
        &self,
        agent_id: &str,
        display_name: Option<&str>,
        enabled: Option<bool>,
        order: Option<i64>,
        ai_provider: Option<&str>,
        ai_providers: Option<&[AgentAiProviderInfo]>,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        models: Option<&[RuntimeAgentOptionMetadata]>,
        efforts: Option<&[RuntimeAgentOptionMetadata]>,
        permission_modes: Option<&[RuntimeAgentOptionMetadata]>,
    ) -> Result<AgentInfo> {
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
        let next_ai_provider = ai_provider
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let next_ai_providers = match ai_providers {
            Some(values) => serde_json::to_string(values)?,
            None => serde_json::to_string(&current.ai_providers)?,
        };
        let trimmed_model = model.map(str::trim);
        let clear_model = id == "astra" && matches!(trimmed_model, Some(""));
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
        let next_enabled = if id == "astra" { Some(true) } else { enabled };
        let now = now_ms();
        conn.execute(
            "UPDATE agents
             SET display_name = COALESCE(?, display_name),
                 ai_provider = COALESCE(?, ai_provider),
                 ai_providers_json = ?,
                 model = CASE WHEN ? = 1 THEN NULL ELSE COALESCE(?, model) END,
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
                next_ai_provider,
                next_ai_providers,
                if clear_model { 1_i64 } else { 0_i64 },
                next_model,
                next_models,
                next_effort,
                next_efforts,
                next_permission_mode,
                next_permission_modes,
                next_enabled.map(|value| if value { 1_i64 } else { 0_i64 }),
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
        display_name: Option<&str>,
        enabled: Option<bool>,
        order: Option<i64>,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        models: Option<&[RuntimeAgentOptionMetadata]>,
        efforts: Option<&[RuntimeAgentOptionMetadata]>,
        permission_modes: Option<&[RuntimeAgentOptionMetadata]>,
    ) -> Result<AgentInfo> {
        let id = agent.as_str();
        self.update_agent_preferences_by_id(
            id,
            display_name,
            enabled,
            order,
            None,
            None,
            model,
            effort,
            permission_mode,
            models,
            efforts,
            permission_modes,
        )
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
                "SELECT id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
                 FROM assistants
                 WHERE project_id = ?
                 ORDER BY type ASC, updated_at DESC, name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map(params![project_id], assistant_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
                 FROM assistants
                 ORDER BY type ASC, updated_at DESC, name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map([], assistant_from_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(assistants)
    }

    fn create_assistant(
        &self,
        name: &str,
        agent: AssistantAgentInfo,
        system_prompt: Option<&str>,
        color: Option<&str>,
        assistant_type: AssistantType,
        workflow_id: Option<String>,
        project_id: Option<&str>,
    ) -> Result<AssistantInfo> {
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
        let resolved_workflow_id =
            workflow_id.or_else(|| project.as_ref().map(|project| project.workflow_id.clone()));
        match assistant_type {
            AssistantType::Builtin => {
                if project_id.is_some() {
                    anyhow::bail!("builtin assistant cannot be linked to a project");
                }
            }
            AssistantType::Custom => {}
        }
        if let Some(workflow_id) = resolved_workflow_id.as_deref() {
            ensure_workflow_exists(&conn, workflow_id)?;
        }
        let now = now_ms();
        let id = stable_assistant_id(
            assistant_type,
            resolved_workflow_id.as_deref(),
            project_id,
            name,
            &agent.model,
            now,
        );
        let agent_json = serde_json::to_string(&agent)?;
        conn.execute(
            "INSERT INTO assistants (
                id, name, agent_json, system_prompt, color, type, workflow_id, project_id, enabled, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
            params![
                id,
                name,
                agent_json,
                system_prompt,
                color,
                assistant_type.as_str(),
                resolved_workflow_id.as_deref(),
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
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
            Some(None) => None,
            None => current.system_prompt,
        };
        let next_color = match color {
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
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
                (SELECT count(*) FROM stage_assistants WHERE assistant_id = ?)",
            params![assistant_id, assistant_id],
            |row| row.get(0),
        )?;
        if stage_count > 0 {
            anyhow::bail!("assistant is used by stages");
        }
        conn.execute("DELETE FROM assistants WHERE id = ?", params![assistant_id])?;
        Ok(())
    }

    fn list_threads(&self, project_id: &str) -> Result<Vec<ThreadInfo>> {
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, goal, description, stage_id, enabled, created_at, updated_at
             FROM threads
             WHERE project_id = ?
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let mut threads = stmt
            .query_map(params![project_id], thread_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for thread in threads.iter_mut() {
            thread.stages = load_thread_stages(&conn, &thread.id)?;
            thread.sessions = load_thread_sessions(&conn, &thread.id)?;
        }
        Ok(threads)
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
        let goal = goal.trim();
        if goal.is_empty() {
            anyhow::bail!("thread goal cannot be empty");
        }
        let description = description.map(str::trim).filter(|s| !s.is_empty());
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let now = now_ms();
        let id = stable_thread_id(project_id, goal, now);
        conn.execute(
            "INSERT INTO threads (id, project_id, goal, description, stage_id, enabled, created_at, updated_at)
             VALUES (?, ?, ?, ?, NULL, 1, ?, ?)",
            params![id, project_id, goal, description, now, now],
        )?;
        load_thread_by_id(&conn, &id)
    }

    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
        enabled: Option<bool>,
    ) -> Result<ThreadInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_thread_by_id(&conn, thread_id)?;
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
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
            Some(None) => None,
            None => current.description,
        };
        let next_enabled = enabled.unwrap_or(current.enabled);
        conn.execute(
            "UPDATE threads
             SET goal = ?, description = ?, enabled = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_goal,
                next_description,
                next_enabled as i64,
                now_ms(),
                thread_id
            ],
        )?;
        load_thread_by_id(&conn, thread_id)
    }

    fn delete_thread(&self, thread_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute("DELETE FROM threads WHERE id = ?", params![thread_id])?;
        if changed == 0 {
            anyhow::bail!("thread not found: {thread_id}");
        }
        Ok(())
    }

    fn list_project_stages(&self, project_id: &str) -> Result<Vec<ProjectStageInfo>> {
        let conn = self.conn.lock().unwrap();
        load_project_by_id(&conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
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

    fn list_workflow_stages(&self, workflow_id: &str) -> Result<Vec<ProjectStageInfo>> {
        let conn = self.conn.lock().unwrap();
        ensure_workflow_exists(&conn, workflow_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at
             FROM stages
             WHERE project_id IS NULL AND workflow_id = ?
             ORDER BY sort_order ASC, type ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![workflow_id], project_stage_from_row)?;
        let mut stages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        for stage in stages.iter_mut() {
            stage.assistants = load_project_stage_assistants(&conn, &stage.id)?;
        }
        Ok(stages)
    }

    fn create_project_stage(
        &self,
        project_id: &str,
        workflow_id: Option<String>,
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
        let requested_workflow_id = workflow_id;
        let project = if requested_workflow_id.is_none() {
            Some(load_project_by_id(&conn, project_id)?)
        } else if project_id.trim().is_empty() {
            None
        } else {
            Some(load_project_by_id(&conn, project_id)?)
        };
        let resolved_workflow_id = requested_workflow_id
            .as_deref()
            .or_else(|| project.as_ref().map(|project| project.workflow_id.as_str()))
            .ok_or_else(|| anyhow::anyhow!("project stage requires a project or workflow"))?;
        ensure_workflow_exists(&conn, resolved_workflow_id)?;
        let template_project_id = if requested_workflow_id.is_some() {
            None
        } else {
            Some(project_id)
        };
        let next_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(sort_order), -1) + 1 FROM stages
                 WHERE workflow_id = ?
                   AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)",
                params![
                    resolved_workflow_id,
                    template_project_id,
                    template_project_id
                ],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let now = now_ms();
        let id = stable_project_stage_id(
            Some(resolved_workflow_id),
            template_project_id,
            name,
            next_order,
            now,
        );
        conn.execute(
            "INSERT INTO stages (id, project_id, type, workflow_id, kind, name, description, icon, sort_order, enabled, allow_empty_assistants, created_at, updated_at)
             VALUES (?, ?, 'custom', ?, NULL, ?, ?, ?, ?, 1, 0, ?, ?)",
            params![
                id,
                template_project_id,
                resolved_workflow_id,
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
        name: Option<&str>,
        description: Option<Option<&str>>,
        icon: Option<Option<&str>>,
        order: Option<i64>,
        enabled: Option<bool>,
        allow_empty_assistants: Option<bool>,
    ) -> Result<ProjectStageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_project_stage_by_id(&tx, stage_id)?;
        if current.stage_type != ProjectStageType::Custom
            && (name.is_some() || description.is_some())
        {
            anyhow::bail!("builtin project stage details cannot be updated");
        }
        let Some(scope_workflow_id) = current.workflow_id else {
            anyhow::bail!("project stage requires a workflow");
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
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
            Some(None) => None,
            None => current.description,
        };
        let next_icon = match icon {
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
            Some(None) => None,
            None => current.icon,
        };
        let next_order = match order {
            Some(target_order) if target_order != current.order => reorder_project_stage_scope(
                &tx,
                stage_id,
                scope_workflow_id.as_str(),
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
            || project_stage.workflow_id.as_deref() != Some(project.workflow_id.as_str())
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
        ensure_session_not_linked_to_thread_workflow(&tx, agent, session_id)?;
        let now = now_ms();
        tx.execute(
            "INSERT OR IGNORE INTO thread_sessions (thread_id, agent, session_id, created_at)
             VALUES (?, ?, ?, ?)",
            params![thread_id, agent.as_str(), session_id, now],
        )?;
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
        ensure_session_not_linked_to_thread_workflow(&tx, agent, session_id)?;
        let now = now_ms();
        tx.execute(
            "INSERT OR IGNORE INTO stage_sessions (thread_stage_id, agent, session_id, created_at)
             VALUES (?, ?, ?, ?)",
            params![thread_stage_id, agent.as_str(), session_id, now],
        )?;
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
            Some(Some(value)) => value
                .trim()
                .is_empty()
                .then(|| None)
                .unwrap_or_else(|| Some(value.trim().to_string())),
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

    fn get_session_history(
        &self,
        agent: Agent,
        session_id: &str,
        file_path: &str,
    ) -> Result<Option<SessionHistoryRecord>> {
        let conn = self.conn.lock().unwrap();
        load_session_history(&conn, agent, session_id, file_path)
    }

    fn replace_session_history(&self, record: &SessionHistoryRecord) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        replace_session_history_inner(&tx, record)?;
        tx.commit()?;
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

    fn upsert_astra_run(&self, run: &AstraRunRecord) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO astra_runs (
                run_id, thread_id, project_id, project_path, status, mode,
                proposed_tasks_json, approved_task_ids_json, delegated_session_ids_json, task_results_json,
                current_stage_id, completed_task_ids_json, stage_attempt_counts_json, retry_limit,
                error, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(run_id) DO UPDATE SET
                thread_id = excluded.thread_id,
                project_id = excluded.project_id,
                project_path = excluded.project_path,
                status = excluded.status,
                mode = excluded.mode,
                proposed_tasks_json = excluded.proposed_tasks_json,
                approved_task_ids_json = excluded.approved_task_ids_json,
                delegated_session_ids_json = excluded.delegated_session_ids_json,
                task_results_json = excluded.task_results_json,
                current_stage_id = excluded.current_stage_id,
                completed_task_ids_json = excluded.completed_task_ids_json,
                stage_attempt_counts_json = excluded.stage_attempt_counts_json,
                retry_limit = excluded.retry_limit,
                error = excluded.error,
                updated_at = excluded.updated_at",
            params![
                run.run_id,
                run.thread_id,
                run.project_id,
                run.project_path,
                run.status,
                run.mode,
                run.proposed_tasks_json,
                run.approved_task_ids_json,
                run.delegated_session_ids_json,
                run.task_results_json,
                run.current_stage_id,
                run.completed_task_ids_json,
                run.stage_attempt_counts_json,
                run.retry_limit,
                run.error,
                run.created_at,
                run.updated_at,
            ],
        )?;
        Ok(())
    }

    fn get_astra_run(&self, run_id: &str) -> Result<Option<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT run_id, thread_id, project_id, project_path, status, mode,
                    proposed_tasks_json, approved_task_ids_json, delegated_session_ids_json, task_results_json,
                    current_stage_id, completed_task_ids_json, stage_attempt_counts_json, retry_limit,
                    error, created_at, updated_at
             FROM astra_runs
             WHERE run_id = ?",
            params![run_id],
            astra_run_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn get_active_astra_run(&self, thread_id: &str) -> Result<Option<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT run_id, thread_id, project_id, project_path, status, mode,
                    proposed_tasks_json, approved_task_ids_json, delegated_session_ids_json, task_results_json,
                    current_stage_id, completed_task_ids_json, stage_attempt_counts_json, retry_limit,
                    error, created_at, updated_at
             FROM astra_runs
             WHERE thread_id = ?
               AND status IN ('planning', 'awaiting_approval', 'dispatching', 'running')
             ORDER BY updated_at DESC
             LIMIT 1",
            params![thread_id],
            astra_run_from_row,
        )
        .optional()
        .map_err(Into::into)
    }

    fn list_astra_runs(&self, thread_id: &str) -> Result<Vec<AstraRunRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT run_id, thread_id, project_id, project_path, status, mode,
                    proposed_tasks_json, approved_task_ids_json, delegated_session_ids_json, task_results_json,
                    current_stage_id, completed_task_ids_json, stage_attempt_counts_json, retry_limit,
                    error, created_at, updated_at
             FROM astra_runs
             WHERE thread_id = ?
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let rows = stmt.query_map(params![thread_id], astra_run_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn interrupt_active_astra_runs(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE astra_runs
             SET status = 'interrupted', updated_at = ?
             WHERE status IN ('planning', 'awaiting_approval', 'dispatching', 'running')",
            params![now_ms()],
        )?;
        Ok(())
    }

    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        insert_session(&conn, scope, session)
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

fn is_codex_guardian_index_row(session: &SessionInfo) -> bool {
    if session.agent != Agent::Codex {
        return false;
    }
    let Ok(file) = std::fs::File::open(&session.file_path) else {
        return false;
    };
    let reader = std::io::BufReader::new(file);
    for line in std::io::BufRead::lines(reader).map_while(|line| line.ok()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("type").and_then(|x| x.as_str()) != Some("session_meta") {
            continue;
        }
        return v
            .get("payload")
            .and_then(|x| x.get("source"))
            .and_then(|x| x.get("subagent"))
            .and_then(|x| x.get("other"))
            .and_then(|x| x.as_str())
            == Some("guardian");
    }
    false
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
mod migration_tests {
    use super::*;

    fn unique_db(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}.db", unique_suffix()))
    }

    // Verify a synthetic v0.3.2-era schema migrates cleanly into the current
    // shape.
    #[test]
    fn migrates_v032_database_to_current_schema() {
        let path = unique_db("sessio-mig-prev8");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY);
                INSERT INTO schema_migrations(version) VALUES (1),(2);
                CREATE TABLE sessions (
                    agent TEXT NOT NULL, session_id TEXT NOT NULL, scope TEXT NOT NULL,
                    file_path TEXT NOT NULL, project_path TEXT, project_name TEXT,
                    started_at INTEGER, updated_at INTEGER,
                    message_count INTEGER NOT NULL DEFAULT 0, first_user_message TEXT,
                    file_size INTEGER NOT NULL DEFAULT 0, file_mtime INTEGER,
                    partial INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1,
                    archived INTEGER NOT NULL DEFAULT 0,
                    last_indexed_at INTEGER NOT NULL,
                    PRIMARY KEY (agent, session_id, scope)
                );
                CREATE TABLE subagents (
                    parent_agent TEXT NOT NULL, parent_session_id TEXT NOT NULL,
                    subagent_id TEXT NOT NULL, file_path TEXT NOT NULL,
                    agent_type TEXT, description TEXT,
                    started_at INTEGER, updated_at INTEGER,
                    message_count INTEGER NOT NULL DEFAULT 0, first_user_message TEXT,
                    file_size INTEGER NOT NULL DEFAULT 0, file_mtime INTEGER,
                    partial INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1,
                    PRIMARY KEY (parent_agent, parent_session_id, subagent_id)
                );
                "#,
            )
            .unwrap();
        }

        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let conn = store.conn.lock().unwrap();
        let latest_schema_version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(latest_schema_version, 11);

        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(memory_records)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(columns.contains(&"record_id".to_string()));
        assert!(columns.contains(&"kind".to_string()));

        let session_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(sessions)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(session_columns.contains(&"forked_from_agent".to_string()));
        assert!(session_columns.contains(&"forked_from_id".to_string()));
        assert!(session_columns.contains(&"rename_title".to_string()));
        assert!(session_columns.contains(&"title".to_string()));

        let projects_count: i64 = conn
            .query_row("SELECT count(*) FROM projects", [], |row| row.get(0))
            .unwrap();
        assert_eq!(projects_count, 0);

        let artifact_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_artifacts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artifact_table, 1);

        let continuations_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='record_continuations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(continuations_table, 1);

        let history_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='session_history'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(history_table, 1);

        let snapshot_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='session_history_snapshots'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshot_table, 1);

        let astra_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='astra_runs'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(astra_table, 1);

        let astra_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(astra_runs)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(astra_columns.contains(&"task_results_json".to_string()));
        assert!(astra_columns.contains(&"mode".to_string()));
        assert!(astra_columns.contains(&"current_stage_id".to_string()));
        assert!(astra_columns.contains(&"completed_task_ids_json".to_string()));
        assert!(astra_columns.contains(&"stage_attempt_counts_json".to_string()));
        assert!(astra_columns.contains(&"retry_limit".to_string()));

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    // Verify a fresh install reaches the current shape.
    #[test]
    fn fresh_install_reaches_current_schema() {
        let path = unique_db("sessio-mig-fresh");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let conn = store.conn.lock().unwrap();
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

        // memory_artifacts table exists from V3 already.
        let artifact_table: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='memory_artifacts'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artifact_table, 1);

        // memory_jobs.backend column is present from V3.
        let job_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(memory_jobs)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(job_columns.contains(&"backend".to_string()));

        let history_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(session_history)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(history_columns.contains(&"indexed_through".to_string()));
        assert!(history_columns.contains(&"message_count".to_string()));
        assert!(history_columns.contains(&"history_cache_version".to_string()));

        let snapshot_columns: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(session_history_snapshots)")
                .unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(snapshot_columns.contains(&"history_cache_version".to_string()));

        let astra_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(astra_runs)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(astra_columns.contains(&"task_results_json".to_string()));
        assert!(astra_columns.contains(&"mode".to_string()));
        assert!(astra_columns.contains(&"current_stage_id".to_string()));
        assert!(astra_columns.contains(&"completed_task_ids_json".to_string()));
        assert!(astra_columns.contains(&"stage_attempt_counts_json".to_string()));
        assert!(astra_columns.contains(&"retry_limit".to_string()));

        for table in [
            "agents",
            "assistants",
            "threads",
            "stages",
            "thread_stages",
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

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn astra_run_persistence_and_recovery() {
        let path = unique_db("sessio-astra-runs");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let project_path =
            std::env::temp_dir().join(format!("sessio-astra-project-{}", unique_suffix()));
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
            proposed_tasks_json: r#"[{"id":"task-1"}]"#.to_string(),
            approved_task_ids_json: r#"["task-1"]"#.to_string(),
            delegated_session_ids_json: r#"["runtime-1"]"#.to_string(),
            task_results_json: r#"[{"taskId":"task-1","status":"completed"}]"#.to_string(),
            current_stage_id: Some("stage-1".to_string()),
            completed_task_ids_json: r#"["task-1"]"#.to_string(),
            stage_attempt_counts_json: r#"{"stage-1":1}"#.to_string(),
            retry_limit: 3,
            error: None,
            created_at: 10,
            updated_at: 20,
        };
        store.upsert_astra_run(&run).unwrap();
        let active = store.get_active_astra_run(&thread.id).unwrap().unwrap();
        assert_eq!(active.run_id, "astra-run-1");
        assert_eq!(active.status, "running");

        store.interrupt_active_astra_runs().unwrap();
        assert!(store.get_active_astra_run(&thread.id).unwrap().is_none());
        let interrupted = store.get_astra_run("astra-run-1").unwrap().unwrap();
        assert_eq!(interrupted.status, "interrupted");

        let runs = store.list_astra_runs(&thread.id).unwrap();
        assert_eq!(runs.len(), 1);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_path);
    }

    #[test]
    fn session_history_roundtrip_stores_acp_turn_json() {
        let path = unique_db("sessio-history-roundtrip");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let turn = SessionHistoryTurn {
            turn_id: "turn-1".to_string(),
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
        let record = SessionHistoryRecord {
            agent: Agent::Codex,
            session_id: "session-a".to_string(),
            file_path: "/tmp/session-a.jsonl".to_string(),
            file_size: 42,
            file_mtime: Some(100),
            history_cache_version: 12,
            message_count: 3,
            indexed_through: Some(20),
            updated_at: 30,
            turns: vec![turn.clone()],
        };

        store.replace_session_history(&record).unwrap();
        let loaded = store
            .get_session_history(Agent::Codex, "session-a", "/tmp/session-a.jsonl")
            .unwrap()
            .unwrap();

        assert_eq!(loaded.file_size, 42);
        assert_eq!(loaded.history_cache_version, 12);
        assert_eq!(loaded.message_count, 3);
        assert_eq!(loaded.indexed_through, Some(20));
        assert_eq!(loaded.turns.len(), 1);
        assert_eq!(loaded.turns[0].turn_id, turn.turn_id);

        let _ = std::fs::remove_file(&path);
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
    fn astra_virtual_session_refs_stay_available_when_scopes_disappear() {
        let path = unique_db("sessio-astra-virtual-scope-guard");
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
            subagents: Vec::new(),
        };
        store.upsert_session(&session.file_path, &session).unwrap();

        assert!(store.list_projects().unwrap().is_empty());
        assert!(store.list_sessions().unwrap().is_empty());
        assert_eq!(store.list_all_sessions().unwrap().len(), 1);

        let project = store
            .add_project(&project_path, Some("Visible"), "research".to_string(), None)
            .unwrap();
        assert_eq!(project.workflow_id, "research");
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
        assert_eq!(created.workflow_id, "video_production");
        assert!(Path::new(&created.path).exists());

        let updated = store
            .update_project(&created.id, Some("Video Plan"), Some("general".to_string()))
            .unwrap();
        assert_eq!(updated.name, "Video Plan");
        assert_eq!(updated.workflow_id, "general");

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
    fn builtin_workflow_stage_kinds_are_workflow_specific() {
        let path = unique_db("sessio-workflow-stage-kinds");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-workflow-stage-parent");
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
            .all(|item| item.workflow_id.as_deref() == Some(code.workflow_id.as_str())));

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
            .update_project_stage(&research.id, None, None, None, Some(plan.order), None, None)
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

        let templates = store.list_workflow_stages("code").unwrap();
        let research_template = templates
            .iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert_eq!(research_template.assistants.len(), 1);
        assert_eq!(
            research_template.assistants[0].assistant_id,
            stable_workflow_builtin_assistant_id("code", "assistant-builtin-research")
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
                &stable_workflow_builtin_assistant_id("code", "assistant-builtin-research")
            )
        );

        let thread = store
            .create_thread(&project.id, "Use stages", None)
            .unwrap();
        let assistant = store
            .create_assistant(
                "Researcher",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                None,
                Some(&project.id),
            )
            .unwrap();
        assert!(store
            .add_thread_stage(&thread.id, &research_template.id, &[assistant.id.clone()])
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

        let workflow_research = store
            .list_assistants(None)
            .unwrap()
            .into_iter()
            .find(|assistant| {
                assistant.id
                    == stable_workflow_builtin_assistant_id("code", "assistant-builtin-research")
            })
            .unwrap();
        assert_eq!(workflow_research.color.as_deref(), Some("#0ea5e9"));

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
            &stable_workflow_builtin_assistant_id("code", "assistant-builtin-research"),
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
    fn seed_does_not_recreate_deleted_workflow_stage_assistant_bindings() {
        let path = unique_db("sessio-workflow-stage-assistant-seed");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let research = store
            .list_workflow_stages("code")
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
            .list_workflow_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        assert!(research.assistants.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn workflow_stage_templates_accept_only_same_workflow_assistants() {
        let path = unique_db("sessio-workflow-stage-assistant-scope");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let research = store
            .list_workflow_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        let code_assistant = store
            .create_assistant(
                "Code reviewer",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                Some("code".to_string()),
                None,
            )
            .unwrap();
        let writing_assistant = store
            .create_assistant(
                "Writing reviewer",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                Some("writing".to_string()),
                None,
            )
            .unwrap();
        let shared_assistant = store
            .create_assistant(
                "Shared reviewer",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                None,
                None,
            )
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

        let wrong_workflow_error = store
            .update_project_stage_assistants(&research.id, &[writing_assistant.id])
            .unwrap_err()
            .to_string();
        assert!(wrong_workflow_error.contains("assistant is not available for this stage"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn custom_assistants_can_be_global_shared() {
        let path = unique_db("sessio-global-custom-assistant");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let assistant = store
            .create_assistant(
                "Shared reviewer",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                None,
                None,
            )
            .unwrap();
        assert_eq!(assistant.workflow_id, None);
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
            .create_assistant(
                "Shared reviewer",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                Some("Review from global context"),
                None,
                AssistantType::Custom,
                None,
                None,
            )
            .unwrap();
        let research_template = store
            .list_workflow_stages("code")
            .unwrap()
            .into_iter()
            .find(|stage| stage.kind == Some(StageType::Research))
            .unwrap();
        store
            .update_project_stage_assistants(&research_template.id, &[shared_assistant.id.clone()])
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
            vec!["astra", "codex", "claude", "gemini"]
        );
        let codex_agent = agents.iter().find(|agent| agent.id == "codex").unwrap();
        assert_eq!(codex_agent.icon.as_deref(), Some("codex"));
        assert_eq!(codex_agent.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(codex_agent.commands.session.len(), 1);
        assert_eq!(
            codex_agent.commands.version.first().map(String::as_str),
            Some("codex --version")
        );
        assert_eq!(codex_agent.effort.as_deref(), Some("high"));
        assert!(codex_agent
            .efforts
            .iter()
            .any(|option| option.value == "xhigh"));
        let claude_agent = agents.iter().find(|agent| agent.id == "claude").unwrap();
        assert_eq!(
            claude_agent.commands.version.first().map(String::as_str),
            Some("claude --version")
        );
        assert_eq!(claude_agent.effort.as_deref(), Some("high"));
        assert!(claude_agent
            .efforts
            .iter()
            .any(|option| option.value == "max"));
        let gemini_agent = agents.iter().find(|agent| agent.id == "gemini").unwrap();
        assert_eq!(
            gemini_agent.commands.version.first().map(String::as_str),
            Some("gemini --version")
        );
        assert_eq!(gemini_agent.model, None);
        assert_eq!(gemini_agent.effort.as_deref(), Some("high"));
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
            .create_assistant(
                "Builder",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                Some("Build carefully"),
                None,
                AssistantType::Custom,
                None,
                Some(&project.id),
            )
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
            .create_assistant(
                "Reviewer",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                None,
                Some(&project.id),
            )
            .unwrap();

        let thread = store
            .create_thread(&project.id, "Ship thread workflow", Some("first pass"))
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
                &stable_workflow_builtin_assistant_id("code", "assistant-builtin-research")
            )
        );
        let default_stage = store
            .update_project_stage_assistants(&research_option.id, &[assistant.id.clone()])
            .unwrap();
        assert_eq!(default_stage.assistants.len(), 1);
        assert_eq!(default_stage.assistants[0].assistant_id, assistant.id);
        let assistant_stage_binding_error = store
            .update_assistant(&assistant.id, None, None, None, None, Some(false))
            .unwrap_err()
            .to_string();
        assert!(assistant_stage_binding_error.contains("project stage assistant binding(s)"));
        assert!(assistant_stage_binding_error.contains("stage \"research\""));
        assert!(!assistant_stage_binding_error.contains("workflow \""));
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
            .update_assistant(&assistant.id, None, None, None, None, Some(false))
            .unwrap_err()
            .to_string();
        assert!(assistant_disable_error.contains("project stage assistant binding(s)"));
        assert!(assistant_disable_error.contains("thread stage assistant binding(s)"));
        assert!(!assistant_disable_error.contains("workflow \""));
        assert!(assistant_disable_error.contains("thread \"Default stage assistants\""));
        let custom_workflow = store
            .create_workflow("Custom Flow", Some("Custom workflow description"))
            .unwrap();
        assert_eq!(custom_workflow.workflow_type, WorkflowType::Custom);
        assert_eq!(
            custom_workflow.description.as_deref(),
            Some("Custom workflow description")
        );
        let renamed_workflow = store
            .update_workflow(
                &custom_workflow.id,
                Some("Custom Flow Prime"),
                Some(Some("Updated custom workflow description")),
            )
            .unwrap();
        assert_eq!(renamed_workflow.name, "Custom Flow Prime");
        assert_eq!(
            renamed_workflow.description.as_deref(),
            Some("Updated custom workflow description")
        );
        let workflow_stage = store
            .create_project_stage(
                "",
                Some(custom_workflow.id.clone()),
                "Workflow Custom Stage",
                Some("Template stage"),
                None,
            )
            .unwrap();
        assert_eq!(
            workflow_stage.workflow_id.as_deref(),
            Some(custom_workflow.id.as_str())
        );
        assert_eq!(workflow_stage.project_id, None);
        let workflow_stages = store.list_workflow_stages(&custom_workflow.id).unwrap();
        assert_eq!(workflow_stages.len(), 1);
        assert_eq!(workflow_stages[0].id, workflow_stage.id);
        store.delete_project_stage(&workflow_stage.id).unwrap();
        store.delete_workflow(&custom_workflow.id).unwrap();
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
            .update_project_stage(&build_option.id, None, None, None, None, None, Some(true))
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
                Some("Review Pass"),
                Some(None),
                None,
                None,
                None,
                None,
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
            .create_thread(&project.id, "Review thread workflow", Some("second lane"))
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
                    id: "gemini".to_string(),
                    name: "Gemini".to_string(),
                    model: "gemini-3-pro".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "medium".to_string(),
                },
            )
            .unwrap();
        assert_eq!(review_stage.assistants[0].assistant_id, assistant.id);
        assert_eq!(review_stage.assistants[0].agent.id, "gemini");
        assert_eq!(review_stage.assistants[0].agent.name, "Gemini");
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
        assert_eq!(review_lane.stages[0].assistants[0].agent.id, "gemini");
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
        assert!(stage_disable_error.contains("thread \"Ship thread workflow\""));
        assert!(stage_disable_error.contains("thread \"Default stage assistants\""));
        store.delete_thread_stage(&default_research.id).unwrap();
        store.delete_thread_stage(&research.id).unwrap();
        let disabled_research = store
            .update_project_stage(
                &research.stage_id,
                None,
                None,
                None,
                None,
                Some(false),
                None,
            )
            .unwrap();
        assert!(!disabled_research.enabled);
        assert!(store
            .list_project_stages(&project.id)
            .unwrap()
            .into_iter()
            .any(|stage| stage.id == research.stage_id && !stage.enabled));
        assert!(store
            .list_workflow_stages(&project.workflow_id)
            .unwrap()
            .into_iter()
            .any(|stage| stage.kind == Some(StageType::Research) && stage.project_id.is_none()));
        assert!(store
            .add_thread_stage(&thread.id, &research.stage_id, &assistant_ids)
            .is_err());
        let enabled_research = store
            .update_project_stage(&research.stage_id, None, None, None, None, Some(true), None)
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
                Some("Ship edited workflow"),
                Some(Some("")),
                None,
            )
            .unwrap();
        assert_eq!(edited_thread.goal, "Ship edited workflow");
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
        assert!(store
            .link_stage_session(&build.id, Agent::Codex, &other_session.id)
            .is_err());

        let other_assistant = store
            .create_assistant(
                "Other Builder",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                None,
                Some(&other_project.id),
            )
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
            .create_assistant(
                "Invalid",
                AssistantAgentInfo {
                    id: "missing".to_string(),
                    name: "Missing".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Custom,
                None,
                Some(&project.id),
            )
            .is_err());
        assert!(store
            .create_assistant(
                "Invalid builtin",
                AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "read-only".to_string(),
                    effort: "medium".to_string(),
                },
                None,
                None,
                AssistantType::Builtin,
                None,
                Some(&project.id),
            )
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
    fn astra_preferences_can_clear_model_without_affecting_runtime_agents() {
        let path = unique_db("sessio-astra-clear-model");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let astra = store
            .update_agent_preferences_by_id(
                "astra",
                None,
                None,
                None,
                None,
                None,
                Some(""),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(astra.model, None);

        let codex = store
            .update_builtin_agent_preferences(
                Agent::Codex,
                None,
                None,
                None,
                Some(""),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(codex.model.as_deref(), Some("gpt-5.5"));

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
                Some("Custom Codex"),
                Some(false),
                Some(99),
                Some("custom-model"),
                Some("medium"),
                Some("auto"),
                None,
                None,
                None,
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
}
