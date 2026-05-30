use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::agents::runtime::types::RuntimeTransportKind;
use crate::agents::sources::types::SourceLocation;
use crate::memory::{
    MemoryArtifact, MemoryJob, MemoryRecord, MemoryRecordKind, MemorySource, MemoryStore,
    RecordContinuation, SessionTimeInfo, TurnFingerprint, TurnFingerprintCandidate,
};
use crate::models::{
    Agent, AgentCommandsInfo, AgentInfo, AgentType, AssistantAgentInfo, AssistantInfo,
    AssistantType, KanbanItem, KanbanStatus, ProjectInfo, ProjectStageInfo, ProjectStageType,
    RuntimeAgentOptionMetadata, SessionHistoryTurn, SessionInfo, StageAssistantInfo, StageInfo,
    StageType, SubagentInfo, ThreadInfo, WorkflowInfo, WorkflowType,
};
use crate::store::{
    IndexedSessionRecord, IndexedSubagentRecord, RuntimeAgentCapabilityRecord,
    SessionHistoryRecord, SessionHistorySnapshotRecord, SessionStore,
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
             PRAGMA foreign_keys = ON;",
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
CREATE TABLE IF NOT EXISTS runtime_agent_capabilities (
    agent                TEXT PRIMARY KEY,
    transport_kind       TEXT NOT NULL,
    detected_version     TEXT,
    protocol_version     TEXT,
    raw_initialize_response_json TEXT NOT NULL,
    raw_capabilities_json TEXT NOT NULL,
    updated_at           INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS projects (
    id         TEXT PRIMARY KEY,
    path       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    workflow_id TEXT NOT NULL DEFAULT 'code',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived   INTEGER NOT NULL DEFAULT 0
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

CREATE TABLE IF NOT EXISTS workflows (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
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

CREATE TABLE IF NOT EXISTS assistants (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    agent_json      TEXT NOT NULL,
    system_prompt   TEXT,
    type            TEXT NOT NULL,
    workflow_id    TEXT,
    project_id      TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom')),
    CHECK((type = 'builtin' AND workflow_id IS NOT NULL AND project_id IS NULL)
       OR (type = 'custom' AND (workflow_id IS NOT NULL OR project_id IS NOT NULL))),
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
    "order"      INTEGER NOT NULL,
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
    CHECK((type = 'builtin' AND workflow_id IS NOT NULL AND project_id IS NULL AND kind IS NOT NULL AND name IS NULL)
       OR (type = 'custom' AND (workflow_id IS NOT NULL OR project_id IS NOT NULL) AND kind IS NULL AND name IS NOT NULL)),
    UNIQUE(workflow_id, project_id, "order"),
    FOREIGN KEY(workflow_id) REFERENCES workflows(id) ON DELETE CASCADE,
    FOREIGN KEY(project_id) REFERENCES projects(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_stages_project
    ON stages(workflow_id, project_id, type, "order", kind, name);

CREATE TABLE IF NOT EXISTS thread_stages (
    id           TEXT PRIMARY KEY,
    thread_id    TEXT NOT NULL,
    stage_id     TEXT NOT NULL,
    "order"      INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE(thread_id, stage_id),
    UNIQUE(thread_id, "order"),
    FOREIGN KEY(thread_id) REFERENCES threads(id) ON DELETE CASCADE,
    FOREIGN KEY(stage_id) REFERENCES stages(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_thread_stages_stage
    ON thread_stages(stage_id);

CREATE TABLE IF NOT EXISTS thread_stage_assistants (
    thread_stage_id TEXT NOT NULL,
    assistant_id    TEXT NOT NULL,
    agent_json      TEXT NOT NULL,
    "order"         INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    PRIMARY KEY(thread_stage_id, assistant_id),
    FOREIGN KEY(thread_stage_id) REFERENCES thread_stages(id) ON DELETE CASCADE,
    FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_thread_stage_assistants_assistant
    ON thread_stage_assistants(assistant_id);

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
    icon             TEXT,
    model            TEXT,
    models_json      TEXT NOT NULL DEFAULT '[]',
    effort           TEXT,
    efforts_json     TEXT NOT NULL DEFAULT '[]',
    permission_mode  TEXT,
    permission_modes_json TEXT NOT NULL DEFAULT '[]',
    type             TEXT NOT NULL,
    enabled          INTEGER NOT NULL DEFAULT 1,
    transport        TEXT NOT NULL DEFAULT 'acp',
    commands_json    TEXT NOT NULL DEFAULT '{"session":[],"version":[]}',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL,
    CHECK(type IN ('builtin', 'custom'))
);

CREATE INDEX IF NOT EXISTS idx_agents_type_enabled
    ON agents(type, enabled, name COLLATE NOCASE);
"#;

const SCHEMA_V5_PATCH: &str = r#"
ALTER TABLE sessions ADD COLUMN forked_from_agent TEXT;
ALTER TABLE projects ADD COLUMN workflow_id TEXT NOT NULL DEFAULT 'code';
ALTER TABLE assistants ADD COLUMN workflow_id TEXT;
ALTER TABLE stages ADD COLUMN workflow_id TEXT;
"#;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{count}")
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
        // V1 dbs already have the index; v2 only adds the column. Both are
        // idempotent on a fresh v1 schema since execute_batch tolerates the
        // CREATE INDEX IF NOT EXISTS, and the ALTER is guarded by version.
        // Catch & ignore errors from ALTER when the column already exists
        // (e.g. an old dev db that pre-baked it).
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
    conn.execute_batch(SCHEMA_V5)?;
    apply_v5_patch(conn);
    conn.execute("DELETE FROM schema_migrations WHERE version > 5", [])?;
    seed_builtin_workflows(conn)?;
    seed_builtin_workflow_stages(conn)?;
    seed_builtin_agents(conn)?;
    Ok(())
}

fn apply_v5_patch(conn: &Connection) {
    for statement in SCHEMA_V5_PATCH
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        let _ = conn.execute(statement, []);
    }
}

fn seed_builtin_workflows(conn: &Connection) -> Result<()> {
    let now = now_ms();
    for (id, name) in BUILTIN_WORKFLOW_SEEDS {
        conn.execute(
            "INSERT INTO workflows (id, name, type, created_at, updated_at)
             VALUES (?, ?, 'builtin', ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                type = excluded.type,
                updated_at = excluded.updated_at",
            params![id, name, now, now],
        )?;
    }
    Ok(())
}

fn seed_builtin_workflow_stages(conn: &Connection) -> Result<()> {
    let now = now_ms();
    delete_legacy_builtin_workflow_stages(conn)?;
    for (workflow_id, _) in BUILTIN_WORKFLOW_SEEDS {
        for (index, (kind, description)) in builtin_workflow_stage_seeds(workflow_id)
            .iter()
            .copied()
            .enumerate()
        {
            let id = format!("stage-builtin-{}-{}", workflow_id, kind.as_str());
            conn.execute(
                "INSERT INTO stages (id, project_id, type, workflow_id, kind, name, description, \"order\", created_at, updated_at)
                 VALUES (?, NULL, 'builtin', ?, ?, NULL, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                    type = excluded.type,
                    workflow_id = excluded.workflow_id,
                    kind = excluded.kind,
                    name = excluded.name,
                    description = excluded.description,
                    \"order\" = excluded.\"order\",
                    updated_at = excluded.updated_at",
                params![
                    id,
                    workflow_id,
                    kind.as_str(),
                    description,
                    index as i64,
                    now,
                    now
                ],
            )?;
        }
    }
    Ok(())
}

const BUILTIN_WORKFLOW_SEEDS: [(&str, &str); 5] = [
    ("code", "Code"),
    ("writing", "Writing"),
    ("research", "Research"),
    ("general", "General"),
    ("video_production", "Video production"),
];

fn delete_legacy_builtin_workflow_stages(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM stages
         WHERE type = 'builtin'
           AND workflow_id IS NULL",
        [],
    )?;
    Ok(())
}

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
    }
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
        "INSERT INTO agents (
            id, name, icon, model, models_json, effort, efforts_json,
            permission_mode, permission_modes_json, type, enabled, transport,
            commands_json, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            icon = COALESCE(agents.icon, excluded.icon),
            model = COALESCE(agents.model, excluded.model),
            models_json = excluded.models_json,
            effort = COALESCE(agents.effort, excluded.effort),
            efforts_json = excluded.efforts_json,
            permission_mode = COALESCE(agents.permission_mode, excluded.permission_mode),
            permission_modes_json = excluded.permission_modes_json,
            type = excluded.type,
            enabled = agents.enabled,
            transport = COALESCE(agents.transport, excluded.transport),
            commands_json = excluded.commands_json,
            updated_at = excluded.updated_at",
        params![
            id,
            runtime_agent_name(agent),
            id,
            model,
            serde_json::to_string(&models)?,
            effort,
            serde_json::to_string(&efforts)?,
            permission_mode,
            serde_json::to_string(&permission_modes)?,
            AgentType::Builtin.as_str(),
            enabled as i64,
            transport_kind_to_db(transport),
            serde_json::to_string(&commands)?,
            now,
            now,
        ],
    )?;
    Ok(())
}

fn seed_builtin_agents(conn: &Connection) -> Result<()> {
    let now = now_ms();
    seed_builtin_agent(
        conn,
        Agent::Codex,
        Some("gpt-5.3-codex"),
        vec![
            runtime_option("gpt-5.5", "5.5"),
            runtime_option("gpt-5.4", "5.4"),
            runtime_option("gpt-5.3-codex", "5.3 Codex"),
        ],
        Some("medium"),
        vec![
            runtime_option("minimal", "Minimal"),
            runtime_option("low", "Low"),
            runtime_option("medium", "Medium"),
            runtime_option("high", "High"),
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
        vec![
            runtime_option("claude-opus-4-7", "Opus 4.7"),
            runtime_option("claude-opus-4-6", "Opus 4.6"),
        ],
        Some("medium"),
        vec![
            runtime_option("low", "Low"),
            runtime_option("medium", "Medium"),
            runtime_option("high", "High"),
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
        Some("medium"),
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

fn existing_session_count_state(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    scope: &str,
) -> Result<Option<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT message_count, partial FROM sessions
         WHERE agent = ? AND session_id = ? AND scope = ?",
    )?;
    let state = stmt
        .query_row(params![agent.as_str(), session_id, scope], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    Ok(state)
}

fn existing_session_lineage(
    conn: &Connection,
    agent: Agent,
    session_id: &str,
    scope: &str,
) -> Result<(Option<Agent>, Option<String>)> {
    let mut stmt = conn.prepare(
        "SELECT forked_from_agent, forked_from_id FROM sessions
         WHERE agent = ? AND session_id = ? AND scope = ?",
    )?;
    let lineage = stmt
        .query_row(params![agent.as_str(), session_id, scope], |r| {
            let agent = r
                .get::<_, Option<String>>(0)?
                .and_then(|value| Agent::from_db_str(&value));
            Ok((agent, r.get(1)?))
        })
        .optional()?
        .unwrap_or((None, None));
    Ok(lineage)
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
    if let Some(existing) = existing_placeholder(conn, s.agent, &s.id, scope, s)? {
        let (existing_forked_from_agent, existing_forked_from_id) =
            existing_session_lineage(conn, s.agent, &existing.session_id, &existing.scope)?;
        let (forked_from_agent, forked_from_id) = merge_session_lineage(
            existing_forked_from_agent,
            existing_forked_from_id,
            s.forked_from_agent,
            s.forked_from_id.clone(),
        );
        conn.execute(
            "UPDATE sessions
             SET session_id = ?, scope = ?, file_path = ?, project_path = ?, project_name = ?,
                 started_at = ?, updated_at = ?, title = ?, first_user_message = ?,
                 file_size = ?, file_mtime = ?, available = ?, archived = ?,
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
                s.title,
                s.first_user_message,
                s.file_size as i64,
                file_mtime_for(&s.file_path),
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
        return Ok(());
    }
    let (message_count, partial) = existing_session_count_state(conn, s.agent, &s.id, scope)?
        .unwrap_or((s.message_count as i64, s.partial as i64));
    let (existing_forked_from_agent, existing_forked_from_id) =
        existing_session_lineage(conn, s.agent, &s.id, scope)?;
    let (forked_from_agent, forked_from_id) = merge_session_lineage(
        existing_forked_from_agent,
        existing_forked_from_id,
        s.forked_from_agent,
        s.forked_from_id.clone(),
    );
    conn.execute(
        "INSERT OR REPLACE INTO sessions (
            agent, session_id, scope, file_path,
            project_path, project_name,
            started_at, updated_at,
            message_count, title, first_user_message,
            file_size, file_mtime,
            partial, available, archived,
            last_indexed_at, forked_from_agent, forked_from_id
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?,?, ?,?, ?,?,?, ?,?,?)",
        params![
            s.agent.as_str(),
            s.id,
            scope,
            s.file_path,
            s.project_path,
            s.project_name,
            s.started_at,
            s.updated_at,
            message_count,
            s.title,
            s.first_user_message,
            s.file_size as i64,
            file_mtime_for(&s.file_path),
            partial,
            s.available as i64,
            s.archived as i64,
            now_ms(),
            forked_from_agent.map(|agent| agent.as_str()),
            forked_from_id,
        ],
    )?;
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
    let workflow_type_raw: String = row.get(2)?;
    Ok(WorkflowInfo {
        id: row.get(0)?,
        name: row.get(1)?,
        workflow_type: WorkflowType::from_db_str(&workflow_type_raw)
            .unwrap_or(WorkflowType::Custom),
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
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

fn agent_info_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentInfo> {
    let models_json: String = row.get(4)?;
    let efforts_json: String = row.get(6)?;
    let permission_modes_json: String = row.get(8)?;
    let agent_type_raw: String = row.get(9)?;
    let transport_raw: String = row.get(11)?;
    let commands_json: String = row.get(12)?;
    let models =
        serde_json::from_str::<Vec<RuntimeAgentOptionMetadata>>(&models_json).unwrap_or_default();
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
        icon: row.get(2)?,
        model: row.get(3)?,
        models,
        effort: row.get(5)?,
        efforts,
        permission_mode: row.get(7)?,
        permission_modes,
        agent_type: AgentType::from_db_str(&agent_type_raw).unwrap_or(AgentType::Custom),
        enabled: row.get::<_, i64>(10)? != 0,
        transport: transport_kind_from_db(&transport_raw),
        commands,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn assistant_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantInfo> {
    let agent_json: String = row.get(2)?;
    let assistant_type_raw: String = row.get(4)?;
    let workflow_id_raw: Option<String> = row.get(5)?;
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
        assistant_type: AssistantType::from_db_str(&assistant_type_raw)
            .unwrap_or(AssistantType::Custom),
        workflow_id: workflow_id_raw,
        project_id: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
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
        order: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn thread_stage_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StageInfo> {
    let stage_type_raw: String = row.get(4)?;
    let workflow_id_raw: Option<String> = row.get(5)?;
    let stage_kind_raw: Option<String> = row.get(6)?;
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
        order: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        sessions: Vec::new(),
    })
}

fn thread_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadInfo> {
    Ok(ThreadInfo {
        id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        description: row.get(3)?,
        stage_id: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        stages: Vec::new(),
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

fn load_agent_by_id(conn: &Connection, agent_id: &str) -> Result<AgentInfo> {
    conn.query_row(
        "SELECT id, name, icon, model, models_json, effort, efforts_json,
                permission_mode, permission_modes_json, type, enabled, transport,
                commands_json, created_at, updated_at
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
        "SELECT id, name, icon, model, models_json, effort, efforts_json,
                permission_mode, permission_modes_json, type, enabled, transport,
                commands_json, created_at, updated_at
         FROM agents
         ORDER BY type ASC, name COLLATE NOCASE ASC",
    )?;
    let rows = stmt.query_map([], agent_info_from_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_assistant_by_id(conn: &Connection, assistant_id: &str) -> Result<AssistantInfo> {
    conn.query_row(
        "SELECT id, name, agent_json, system_prompt, type, workflow_id, project_id, created_at, updated_at
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
            "SELECT id, project_id, goal, description, stage_id, created_at, updated_at
             FROM threads
             WHERE id = ?",
            params![thread_id],
            thread_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
    thread.stages = load_thread_stages(conn, &thread.id)?;
    Ok(thread)
}

fn load_project_stage_by_id(conn: &Connection, stage_id: &str) -> Result<ProjectStageInfo> {
    conn.query_row(
        "SELECT id, project_id, type, workflow_id, kind, name, description, \"order\", created_at, updated_at
         FROM stages
         WHERE id = ?",
        params![stage_id],
        project_stage_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow::anyhow!("project stage not found: {stage_id}"))
}

fn load_thread_stage_by_id(conn: &Connection, thread_stage_id: &str) -> Result<StageInfo> {
    let mut stage = conn
        .query_row(
            "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.workflow_id, s.kind, s.name, s.description,
                    ts.\"order\", ts.created_at, ts.updated_at
             FROM thread_stages ts
             INNER JOIN threads t ON t.id = ts.thread_id
             INNER JOIN stages s ON s.id = ts.stage_id
             WHERE ts.id = ?",
            params![thread_stage_id],
            thread_stage_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("thread stage not found: {thread_stage_id}"))?;
    stage.assistants = load_stage_assistants(conn, &stage.id)?;
    stage.assistant_ids = stage
        .assistants
        .iter()
        .map(|assistant| assistant.assistant_id.clone())
        .collect();
    stage.sessions = load_stage_sessions(conn, &stage.id)?;
    Ok(stage)
}

fn validate_assistant_for_project(
    conn: &Connection,
    project_id: &str,
    assistant_id: &str,
) -> Result<AssistantInfo> {
    let project = load_project_by_id(conn, project_id)?;
    let assistant = load_assistant_by_id(conn, assistant_id)?;
    if assistant.project_id.as_deref() == Some(project_id) {
        return Ok(assistant);
    }
    if assistant.project_id.is_none() && assistant.workflow_id == Some(project.workflow_id) {
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
    if assistants.is_empty() {
        anyhow::bail!("thread stage requires at least one assistant");
    }
    Ok(assistants)
}

fn load_stage_assistants(
    conn: &Connection,
    thread_stage_id: &str,
) -> Result<Vec<StageAssistantInfo>> {
    let mut stmt = conn.prepare(
        "SELECT tsa.assistant_id, a.name, tsa.agent_json, tsa.\"order\"
         FROM thread_stage_assistants tsa
         INNER JOIN assistants a ON a.id = tsa.assistant_id
         WHERE tsa.thread_stage_id = ?
         ORDER BY tsa.\"order\" ASC, tsa.created_at ASC",
    )?;
    let rows = stmt.query_map(params![thread_stage_id], |row| {
        let agent_json: String = row.get(2)?;
        Ok(StageAssistantInfo {
            assistant_id: row.get(0)?,
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
            order: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn stage_assistant_from_assistant(assistant: AssistantInfo, order: i64) -> StageAssistantInfo {
    StageAssistantInfo {
        assistant_id: assistant.id,
        name: assistant.name,
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
            "INSERT INTO thread_stage_assistants (thread_stage_id, assistant_id, agent_json, \"order\", created_at, updated_at)
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
    delete_legacy_agent_builtin_assistants(conn)?;
    let codex_agent = load_agent_by_id(conn, Agent::Codex.as_str())?;
    let Some(assistant_agent) = assistant_agent_from_db_agent(&codex_agent) else {
        return Ok(());
    };
    for (workflow_id, _) in BUILTIN_WORKFLOW_SEEDS {
        for seed in builtin_assistant_seeds() {
            upsert_builtin_assistant(conn, workflow_id, seed, &assistant_agent, now)?;
        }
    }
    Ok(())
}

fn delete_legacy_agent_builtin_assistants(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM assistants
         WHERE type = 'builtin'
           AND id IN (
             'assistant-builtin-codex',
             'assistant-builtin-claude',
             'assistant-builtin-gemini',
             'assistant-builtin-researcher',
             'assistant-builtin-planner',
             'assistant-builtin-builder',
             'assistant-builtin-reviewer'
           )
           OR (type = 'builtin' AND workflow_id IS NULL)",
        [],
    )?;
    Ok(())
}

struct BuiltinAssistantSeed {
    id: &'static str,
    name: &'static str,
    system_prompt: &'static str,
}

fn builtin_assistant_seeds() -> [BuiltinAssistantSeed; 4] {
    [
        BuiltinAssistantSeed {
            id: "assistant-builtin-researcher",
            name: "Researcher",
            system_prompt: "Research the problem space before implementation. Gather relevant context, inspect existing project behavior, identify constraints and unknowns, and report concise findings with sources or file references when available.",
        },
        BuiltinAssistantSeed {
            id: "assistant-builtin-planner",
            name: "Planner",
            system_prompt: "Create a clear execution plan from the thread goal. Break the work into ordered steps, call out dependencies and risks, and keep the plan focused on decisions that unblock implementation.",
        },
        BuiltinAssistantSeed {
            id: "assistant-builtin-builder",
            name: "Builder",
            system_prompt: "Implement the selected plan. Make scoped changes, follow the existing project patterns, keep behavior coherent across the stack, and verify the result with the most relevant checks.",
        },
        BuiltinAssistantSeed {
            id: "assistant-builtin-reviewer",
            name: "Reviewer",
            system_prompt: "Review the completed work for correctness, regressions, data model consistency, edge cases, and missing tests. Prioritize actionable findings and confirm when no blocking issues remain.",
        },
    ]
}

fn upsert_builtin_assistant(
    conn: &Connection,
    workflow_id: &str,
    seed: BuiltinAssistantSeed,
    assistant_agent: &AssistantAgentInfo,
    now: i64,
) -> Result<()> {
    let id = format!("{}-{}", seed.id, workflow_id);
    let agent_json = serde_json::to_string(&assistant_agent)?;
    conn.execute(
        "INSERT INTO assistants (
            id, name, agent_json, system_prompt, type, workflow_id, project_id, created_at, updated_at
         ) VALUES (?, ?, ?, ?, 'builtin', ?, NULL, ?, ?)
         ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            agent_json = excluded.agent_json,
            system_prompt = excluded.system_prompt,
            type = excluded.type,
            workflow_id = excluded.workflow_id,
            project_id = excluded.project_id,
            updated_at = excluded.updated_at",
        params![
            id,
            seed.name,
            agent_json,
            seed.system_prompt,
            workflow_id,
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
         ORDER BY \"order\" ASC, created_at ASC
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
             ORDER BY \"order\" ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(params![thread_id], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (index, id) in ids.iter().enumerate() {
        conn.execute(
            "UPDATE thread_stages SET \"order\" = ? WHERE id = ?",
            params![index as i64, id],
        )?;
    }
    Ok(())
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

fn load_kanban_item_sessions(conn: &Connection, item_id: &str) -> Result<Vec<SessionInfo>> {
    let mut subs_by_parent = load_all_subagents_grouped(conn)?;
    let mut stmt = conn.prepare(
        "SELECT s.agent, s.session_id, s.file_path, s.project_path, s.project_name,
                s.started_at, s.updated_at, s.message_count, s.title, s.first_user_message,
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
                    .get::<_, Option<String>>(14)?
                    .and_then(|value| Agent::from_db_str(&value)),
                forked_from_id: row.get(15)?,
                file_path: row.get(2)?,
                project_path: row.get(3)?,
                project_name: row.get(4)?,
                started_at: row.get(5)?,
                updated_at: row.get(6)?,
                message_count: row.get::<_, i64>(7)? as usize,
                title: row.get(8)?,
                first_user_message: row.get(9)?,
                file_size: row.get::<_, i64>(10)? as u64,
                partial: row.get::<_, i64>(11)? != 0,
                available: row.get::<_, i64>(12)? != 0,
                archived: row.get::<_, i64>(13)? != 0,
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
                s.started_at, s.updated_at, s.message_count, s.title, s.first_user_message,
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
                    .get::<_, Option<String>>(14)?
                    .and_then(|value| Agent::from_db_str(&value)),
                forked_from_id: row.get(15)?,
                file_path: row.get(2)?,
                project_path: row.get(3)?,
                project_name: row.get(4)?,
                started_at: row.get(5)?,
                updated_at: row.get(6)?,
                message_count: row.get::<_, i64>(7)? as usize,
                title: row.get(8)?,
                first_user_message: row.get(9)?,
                file_size: row.get::<_, i64>(10)? as u64,
                partial: row.get::<_, i64>(11)? != 0,
                available: row.get::<_, i64>(12)? != 0,
                archived: row.get::<_, i64>(13)? != 0,
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
        "SELECT ts.id, ts.thread_id, ts.stage_id, t.project_id, s.type, s.workflow_id, s.kind, s.name, s.description,
                ts.\"order\", ts.created_at, ts.updated_at
         FROM thread_stages ts
         INNER JOIN threads t ON t.id = ts.thread_id
         INNER JOIN stages s ON s.id = ts.stage_id
         WHERE ts.thread_id = ?
         ORDER BY ts.\"order\" ASC, ts.created_at ASC",
    )?;
    let mut stages = stmt
        .query_map(params![thread_id], thread_stage_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for stage in stages.iter_mut() {
        stage.assistants = load_stage_assistants(conn, &stage.id)?;
        stage.assistant_ids = stage
            .assistants
            .iter()
            .map(|assistant| assistant.assistant_id.clone())
            .collect();
        stage.sessions = load_stage_sessions(conn, &stage.id)?;
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
    let mut seen = HashSet::new();
    sessions.retain(|session| seen.insert((session.agent, session.id.clone())));
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
                s.started_at, s.updated_at, s.message_count, s.title, s.first_user_message,
                s.file_size, s.partial, s.available, s.archived, s.forked_from_agent, s.forked_from_id
         FROM sessions s
         INNER JOIN projects p ON p.path = s.project_path AND p.archived = 0
         ORDER BY s.updated_at DESC"
    } else {
        "SELECT agent, session_id, file_path, project_path, project_name,
                started_at, updated_at, message_count, title, first_user_message,
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
                    .get::<_, Option<String>>(14)?
                    .and_then(|value| Agent::from_db_str(&value)),
                forked_from_id: row.get(15)?,
                file_path: row.get(2)?,
                project_path: row.get(3)?,
                project_name: row.get(4)?,
                started_at: row.get(5)?,
                updated_at: row.get(6)?,
                message_count: row.get::<_, i64>(7)? as usize,
                title: row.get(8)?,
                first_user_message: row.get(9)?,
                file_size: row.get::<_, i64>(10)? as u64,
                partial: row.get::<_, i64>(11)? != 0,
                available: row.get::<_, i64>(12)? != 0,
                archived: row.get::<_, i64>(13)? != 0,
                subagents: Vec::new(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    sessions.retain(|s| !is_codex_guardian_index_row(s));
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

    fn list_workflows(&self) -> Result<Vec<WorkflowInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, type, created_at, updated_at
             FROM workflows
             ORDER BY type ASC, name COLLATE NOCASE ASC",
        )?;
        let rows = stmt.query_map([], workflow_from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
    ) -> Result<ProjectInfo> {
        let canonical = canonical_project_path(path)?;
        let name = clean_project_name(name, &canonical)?;
        let id = stable_project_id(&canonical);
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        ensure_workflow_exists(&conn, &workflow_id)?;
        conn.execute(
            "INSERT INTO projects (id, path, name, workflow_id, created_at, updated_at, archived)
             VALUES (?, ?, ?, ?, ?, ?, 0)",
            params![id, canonical, name, workflow_id.as_str(), now, now],
        )
        .with_context(|| "add project")?;
        load_project_by_id(&conn, &id)
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        workflow_id: String,
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
        self.add_project(&path, Some(&clean_name), workflow_id)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        workflow_id: Option<String>,
    ) -> Result<ProjectInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_project_by_id(&conn, project_id)?;
        let next_name = match name {
            Some(value) => clean_project_name(Some(value), &current.path)?,
            None => current.name,
        };
        let next_workflow_id = workflow_id.unwrap_or(current.workflow_id);
        ensure_workflow_exists(&conn, &next_workflow_id)?;
        conn.execute(
            "UPDATE projects
             SET name = ?, workflow_id = ?, updated_at = ?
             WHERE id = ? AND archived = 0",
            params![next_name, next_workflow_id.as_str(), now_ms(), project_id],
        )?;
        load_project_by_id(&conn, project_id)
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

    fn update_builtin_agent_preferences(
        &self,
        agent: Agent,
        model: Option<&str>,
        effort: Option<&str>,
        permission_mode: Option<&str>,
        models: Option<&[RuntimeAgentOptionMetadata]>,
        efforts: Option<&[RuntimeAgentOptionMetadata]>,
        permission_modes: Option<&[RuntimeAgentOptionMetadata]>,
    ) -> Result<AgentInfo> {
        let conn = self.conn.lock().unwrap();
        let id = agent.as_str();
        let current = load_agent_by_id(&conn, id)?;
        if current.agent_type != AgentType::Builtin {
            anyhow::bail!("agent is not builtin: {id}");
        }
        let next_model = model.map(str::trim).filter(|value| !value.is_empty());
        let next_effort = effort.map(str::trim).filter(|value| !value.is_empty());
        let next_permission_mode = permission_mode
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let next_models = match models {
            Some(values) => serde_json::to_string(values)?,
            None => serde_json::to_string(&current.models)?,
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
             SET model = COALESCE(?, model),
                 models_json = ?,
                 effort = COALESCE(?, effort),
                 efforts_json = ?,
                 permission_mode = COALESCE(?, permission_mode),
                 permission_modes_json = ?,
                 updated_at = ?
             WHERE id = ? AND type = 'builtin'",
            params![
                next_model,
                next_models,
                next_effort,
                next_efforts,
                next_permission_mode,
                next_permission_modes,
                now,
                id,
            ],
        )?;
        seed_builtin_assistants(&conn, now)?;
        load_agent_by_id(&conn, id)
    }

    fn list_assistants(&self, project_id: Option<&str>) -> Result<Vec<AssistantInfo>> {
        let conn = self.conn.lock().unwrap();
        let assistants = if let Some(project_id) = project_id {
            let project = load_project_by_id(&conn, project_id)?;
            let mut stmt = conn.prepare(
                "SELECT id, name, agent_json, system_prompt, type, workflow_id, project_id, created_at, updated_at
                 FROM assistants
                 WHERE (project_id IS NULL AND workflow_id = ?) OR project_id = ?
                 ORDER BY type ASC, project_id IS NOT NULL ASC, updated_at DESC, name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map(
                params![project.workflow_id.as_str(), project_id],
                assistant_from_row,
            )?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, name, agent_json, system_prompt, type, workflow_id, project_id, created_at, updated_at
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
                if resolved_workflow_id.is_none() {
                    anyhow::bail!("builtin assistant requires a workflow");
                }
            }
            AssistantType::Custom => {
                if project_id.is_none() && resolved_workflow_id.is_none() {
                    anyhow::bail!("custom assistant requires a project or workflow");
                }
            }
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
                id, name, agent_json, system_prompt, type, workflow_id, project_id, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                id,
                name,
                agent_json,
                system_prompt,
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
        let next_agent_json = serde_json::to_string(&next_agent)?;
        conn.execute(
            "UPDATE assistants
             SET name = ?, agent_json = ?, system_prompt = ?, updated_at = ?
             WHERE id = ?",
            params![
                next_name,
                next_agent_json,
                next_system_prompt,
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
            "SELECT count(*) FROM thread_stage_assistants WHERE assistant_id = ?",
            params![assistant_id],
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
            "SELECT id, project_id, goal, description, stage_id, created_at, updated_at
             FROM threads
             WHERE project_id = ?
             ORDER BY updated_at DESC, created_at DESC",
        )?;
        let mut threads = stmt
            .query_map(params![project_id], thread_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for thread in threads.iter_mut() {
            thread.stages = load_thread_stages(&conn, &thread.id)?;
        }
        Ok(threads)
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
            "INSERT INTO threads (id, project_id, goal, description, stage_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, NULL, ?, ?)",
            params![id, project_id, goal, description, now, now],
        )?;
        load_thread_by_id(&conn, &id)
    }

    fn update_thread(
        &self,
        thread_id: &str,
        goal: Option<&str>,
        description: Option<Option<&str>>,
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
        conn.execute(
            "UPDATE threads
             SET goal = ?, description = ?, updated_at = ?
             WHERE id = ?",
            params![next_goal, next_description, now_ms(), thread_id],
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
        let project = load_project_by_id(&conn, project_id)?;
        let mut stmt = conn.prepare(
            "SELECT id, project_id, type, workflow_id, kind, name, description, \"order\", created_at, updated_at
             FROM stages
             WHERE (project_id IS NULL AND workflow_id = ?) OR project_id = ?
             ORDER BY type ASC, project_id IS NOT NULL ASC, \"order\" ASC, created_at ASC",
        )?;
        let rows = stmt.query_map(
            params![project.workflow_id.as_str(), project_id],
            project_stage_from_row,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn create_project_stage(
        &self,
        project_id: &str,
        workflow_id: Option<String>,
        name: &str,
        description: Option<&str>,
    ) -> Result<ProjectStageInfo> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("project stage name cannot be empty");
        }
        let description = description.map(str::trim).filter(|value| !value.is_empty());
        let conn = self.conn.lock().unwrap();
        let project = load_project_by_id(&conn, project_id)?;
        let requested_workflow_id = workflow_id;
        let resolved_workflow_id = requested_workflow_id
            .as_deref()
            .unwrap_or(project.workflow_id.as_str());
        ensure_workflow_exists(&conn, resolved_workflow_id)?;
        let template_project_id = if requested_workflow_id.is_some() {
            None
        } else {
            Some(project_id)
        };
        let next_order: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(\"order\"), -1) + 1 FROM stages
                 WHERE type = 'custom'
                   AND workflow_id = ?
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
            "INSERT INTO stages (id, project_id, type, workflow_id, kind, name, description, \"order\", created_at, updated_at)
             VALUES (?, ?, 'custom', ?, NULL, ?, ?, ?, ?, ?)",
            params![
                id,
                template_project_id,
                resolved_workflow_id,
                name,
                description,
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
        order: Option<i64>,
    ) -> Result<ProjectStageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_project_stage_by_id(&tx, stage_id)?;
        if current.stage_type != ProjectStageType::Custom {
            anyhow::bail!("builtin project stage cannot be updated");
        }
        let Some(scope_workflow_id) = current.workflow_id else {
            anyhow::bail!("custom project stage requires a workflow");
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
        let max_order: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(\"order\"), 0) FROM stages
                 WHERE type = 'custom'
                   AND workflow_id = ?
                   AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)",
                params![
                    scope_workflow_id.as_str(),
                    scope_project_id,
                    scope_project_id
                ],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let next_order = order.unwrap_or(current.order).clamp(0, max_order);
        if next_order != current.order {
            tx.execute(
                "UPDATE stages SET \"order\" = -1 WHERE id = ?",
                params![stage_id],
            )?;
            if next_order < current.order {
                tx.execute(
                    "UPDATE stages
                     SET \"order\" = \"order\" + 1
                     WHERE type = 'custom'
                       AND workflow_id = ?
                       AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)
                       AND \"order\" >= ? AND \"order\" < ? AND id != ?",
                    params![
                        scope_workflow_id.as_str(),
                        scope_project_id,
                        scope_project_id,
                        next_order,
                        current.order,
                        stage_id
                    ],
                )?;
            } else {
                tx.execute(
                    "UPDATE stages
                     SET \"order\" = \"order\" - 1
                     WHERE type = 'custom'
                       AND workflow_id = ?
                       AND ((project_id IS NULL AND ? IS NULL) OR project_id = ?)
                       AND \"order\" <= ? AND \"order\" > ? AND id != ?",
                    params![
                        scope_workflow_id.as_str(),
                        scope_project_id,
                        scope_project_id,
                        next_order,
                        current.order,
                        stage_id
                    ],
                )?;
            }
        }
        let now = now_ms();
        tx.execute(
            "UPDATE stages SET name = ?, description = ?, \"order\" = ?, updated_at = ? WHERE id = ?",
            params![next_name, next_description, next_order, now, stage_id],
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
        let project = load_project_by_id(&tx, &thread.project_id)?;
        let project_stage = load_project_stage_by_id(&tx, stage_id)?;
        let stage_available = project_stage.project_id.as_deref()
            == Some(thread.project_id.as_str())
            || (project_stage.project_id.is_none()
                && project_stage.workflow_id == Some(project.workflow_id));
        if !stage_available {
            anyhow::bail!("project stage does not belong to this thread's project");
        }
        let assistant_bindings =
            validate_assistants_for_project(&tx, &thread.project_id, assistant_ids)?
                .into_iter()
                .enumerate()
                .map(|(index, assistant)| stage_assistant_from_assistant(assistant, index as i64))
                .collect::<Vec<_>>();
        let assistant_ids = assistant_bindings
            .iter()
            .map(|assistant| assistant.assistant_id.clone())
            .collect::<Vec<_>>();
        let next_order: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(\"order\"), -1) + 1 FROM thread_stages WHERE thread_id = ?",
                params![thread_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let now = now_ms();
        let id = stable_thread_stage_id(thread_id, stage_id, &assistant_ids.join(","), next_order);
        tx.execute(
            "INSERT INTO thread_stages (id, thread_id, stage_id, \"order\", created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![id, thread_id, stage_id, next_order, now, now],
        )?;
        replace_thread_stage_assistants(&tx, &id, &assistant_bindings, now)?;
        if thread.stage_id.is_none() {
            tx.execute(
                "UPDATE threads SET stage_id = ?, updated_at = ? WHERE id = ?",
                params![id, now, thread_id],
            )?;
        } else {
            tx.execute(
                "UPDATE threads SET updated_at = ? WHERE id = ?",
                params![now, thread_id],
            )?;
        }
        let stage = load_thread_stage_by_id(&tx, &id)?;
        tx.commit()?;
        Ok(stage)
    }

    fn update_thread_stage(
        &self,
        thread_stage_id: &str,
        assistant_ids: Option<&[String]>,
        order: Option<i64>,
    ) -> Result<StageInfo> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let current = load_thread_stage_by_id(&tx, thread_stage_id)?;
        let next_assistant_bindings = match assistant_ids {
            Some(ids) => Some(
                validate_assistants_for_project(&tx, &current.project_id, ids)?
                    .into_iter()
                    .enumerate()
                    .map(|(index, assistant)| {
                        stage_assistant_from_assistant(assistant, index as i64)
                    })
                    .collect::<Vec<_>>(),
            ),
            None => None,
        };
        let max_order: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(\"order\"), 0) FROM thread_stages WHERE thread_id = ?",
                params![current.thread_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let next_order = order.unwrap_or(current.order).clamp(0, max_order);
        if next_order != current.order {
            tx.execute(
                "UPDATE thread_stages SET \"order\" = -1 WHERE id = ?",
                params![thread_stage_id],
            )?;
            if next_order < current.order {
                tx.execute(
                    "UPDATE thread_stages
                     SET \"order\" = \"order\" + 1
                     WHERE thread_id = ? AND \"order\" >= ? AND \"order\" < ? AND id != ?",
                    params![
                        current.thread_id,
                        next_order,
                        current.order,
                        thread_stage_id
                    ],
                )?;
            } else {
                tx.execute(
                    "UPDATE thread_stages
                     SET \"order\" = \"order\" - 1
                     WHERE thread_id = ? AND \"order\" <= ? AND \"order\" > ? AND id != ?",
                    params![
                        current.thread_id,
                        next_order,
                        current.order,
                        thread_stage_id
                    ],
                )?;
            }
        }
        let now = now_ms();
        tx.execute(
            "UPDATE thread_stages
             SET \"order\" = ?, updated_at = ?
             WHERE id = ?",
            params![next_order, now, thread_stage_id],
        )?;
        if let Some(next_assistant_bindings) = next_assistant_bindings {
            replace_thread_stage_assistants(&tx, thread_stage_id, &next_assistant_bindings, now)?;
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
        load_thread_by_id(&conn, thread_id)?;
        let stage = load_thread_stage_by_id(&conn, thread_stage_id)?;
        if stage.thread_id != thread_id {
            anyhow::bail!("stage does not belong to this thread");
        }
        conn.execute(
            "UPDATE threads SET stage_id = ?, updated_at = ? WHERE id = ?",
            params![thread_stage_id, now_ms(), thread_id],
        )?;
        load_thread_by_id(&conn, thread_id)
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
        let project = load_project_by_id(&tx, &stage.project_id)?;
        let session_project_path = session_project_path(&tx, agent, session_id)?
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        if session_project_path != project.path {
            anyhow::bail!("session does not belong to this stage's project");
        }
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
                     WHERE scope = ? AND agent = ? AND session_id = ?",
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
                message_count, title, first_user_message,
            file_size, file_mtime,
            partial, available, archived,
            last_indexed_at, forked_from_agent, forked_from_id
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?,?, ?,?, ?,?,?, ?,?,?)",
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
                   AND NOT (file_size = 0 AND partial = 1)",
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
        assert_eq!(latest_schema_version, 5);

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

        for table in [
            "agents",
            "assistants",
            "threads",
            "stages",
            "thread_stages",
            "stage_sessions",
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
            .add_project(&project_path, Some("Visible"), "research".to_string())
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
            .create_project(&parent.to_string_lossy(), "code-flow", "code".to_string())
            .unwrap();
        let writing = store
            .create_project(
                &parent.to_string_lossy(),
                "writing-flow",
                "writing".to_string(),
            )
            .unwrap();
        let video = store
            .create_project(
                &parent.to_string_lossy(),
                "video-flow",
                "video_production".to_string(),
            )
            .unwrap();
        let general = store
            .create_project(
                &parent.to_string_lossy(),
                "general-flow",
                "general".to_string(),
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
    fn kanban_items_link_and_aggregate_sessions() {
        let path = unique_db("sessio-kanban-session-links");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-kanban-link-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(&parent.to_string_lossy(), "linked", "code".to_string())
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
            .create_project(&other_parent.to_string_lossy(), "other", "code".to_string())
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
        let codex_agent = agents.iter().find(|agent| agent.id == "codex").unwrap();
        assert_eq!(codex_agent.icon.as_deref(), Some("codex"));
        assert_eq!(codex_agent.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(codex_agent.commands.session.len(), 1);
        assert_eq!(
            codex_agent.commands.version.first().map(String::as_str),
            Some("codex --version")
        );
        assert_eq!(codex_agent.effort.as_deref(), Some("medium"));
        assert!(codex_agent
            .efforts
            .iter()
            .any(|option| option.value == "high"));
        let claude_agent = agents.iter().find(|agent| agent.id == "claude").unwrap();
        assert_eq!(
            claude_agent.commands.version.first().map(String::as_str),
            Some("claude --version")
        );
        assert_eq!(claude_agent.effort.as_deref(), Some("medium"));
        assert!(claude_agent
            .efforts
            .iter()
            .any(|option| option.value == "high"));
        let gemini_agent = agents.iter().find(|agent| agent.id == "gemini").unwrap();
        assert_eq!(
            gemini_agent.commands.version.first().map(String::as_str),
            Some("gemini --version")
        );
        assert_eq!(gemini_agent.model, None);
        assert_eq!(gemini_agent.effort.as_deref(), Some("medium"));
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-thread-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(&parent.to_string_lossy(), "threaded", "code".to_string())
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
        assert_eq!(
            project_assistants
                .iter()
                .filter(|item| item.assistant_type == AssistantType::Builtin)
                .count(),
            4
        );
        let builtin_builder = project_assistants
            .iter()
            .find(|item| item.id == "assistant-builtin-builder-code")
            .unwrap();
        assert_eq!(builtin_builder.name, "Builder");
        assert_eq!(builtin_builder.agent.id, "codex");
        assert!(builtin_builder
            .system_prompt
            .as_deref()
            .unwrap()
            .contains("Implement the selected plan"));
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
        let build_option = store
            .create_project_stage(
                &project.id,
                None,
                "Implementation",
                Some("Implementation notes"),
            )
            .unwrap();
        assert_eq!(build_option.stage_type, ProjectStageType::Custom);
        assert_eq!(build_option.name.as_deref(), Some("Implementation"));
        assert_eq!(
            build_option.description.as_deref(),
            Some("Implementation notes")
        );
        assert_eq!(store.list_project_stages(&project.id).unwrap().len(), 7);
        let assistant_ids = vec![assistant.id.clone(), reviewer.id.clone()];
        let research = store
            .add_thread_stage(&thread.id, &research_option.id, &assistant_ids)
            .unwrap();
        assert_eq!(research.order, 0);
        assert_eq!(research.stage_id, research_option.id);
        assert_eq!(research.assistant_ids, assistant_ids);
        let builder_only_ids = vec![assistant.id.clone()];
        let build = store
            .add_thread_stage(&thread.id, &build_option.id, &builder_only_ids)
            .unwrap();
        assert_eq!(build.order, 1);
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
            .update_project_stage(&build_option.id, Some("Review Pass"), Some(None), None)
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
        assert_eq!(thread_lanes.len(), 2);
        let build_lane = thread_lanes
            .iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        let review_lane = thread_lanes
            .iter()
            .find(|item| item.id == review_thread.id)
            .unwrap();
        assert_eq!(build_lane.stages.len(), 2);
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
        let build = store.update_thread_stage(&build.id, None, Some(0)).unwrap();
        assert_eq!(build.name.as_deref(), Some("Review Pass"));
        assert_eq!(build.order, 0);
        assert_eq!(build.assistants[0].agent.id, "claude");

        let listed_threads = store.list_threads(&project.id).unwrap();
        assert_eq!(listed_threads.len(), 2);
        let listed_build_lane = listed_threads
            .iter()
            .find(|item| item.id == thread.id)
            .unwrap();
        assert_eq!(
            listed_build_lane.stage_id.as_deref(),
            Some(research.id.as_str())
        );
        assert_eq!(listed_build_lane.stages.len(), 2);
        assert_eq!(listed_build_lane.stages[0].id, build.id);
        assert_eq!(listed_build_lane.stages[1].assistant_ids, assistant_ids);
        let reordered_ids = vec![reviewer.id.clone(), assistant.id.clone()];
        let research = store
            .update_thread_stage(&research.id, Some(&reordered_ids), None)
            .unwrap();
        assert_eq!(research.assistant_ids, reordered_ids);

        let switched = store.set_thread_stage(&thread.id, &build.id).unwrap();
        assert_eq!(switched.stage_id.as_deref(), Some(build.id.as_str()));
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

        let edited_thread = store
            .update_thread(&thread.id, Some("Ship edited workflow"), Some(Some("")))
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
            .create_project(&other_parent.to_string_lossy(), "other", "code".to_string())
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
                AssistantType::Custom,
                None,
                Some(&other_project.id),
            )
            .unwrap();
        let other_thread = store
            .create_thread(&other_project.id, "Other thread", None)
            .unwrap();
        let other_stage_option = store
            .create_project_stage(&other_project.id, None, "Other Plan", None)
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
        assert!(store.list_threads(&project.id).unwrap().is_empty());
        store.delete_assistant(&assistant.id).unwrap();
        store.delete_assistant(&reviewer.id).unwrap();
        let remaining_assistants = store.list_assistants(Some(&project.id)).unwrap();
        assert_eq!(remaining_assistants.len(), 4);
        assert!(remaining_assistants
            .iter()
            .all(|item| item.assistant_type == AssistantType::Builtin));

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
                AssistantType::Builtin,
                None,
                Some(&project.id),
            )
            .is_err());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&parent);
        let _ = std::fs::remove_dir_all(&other_parent);
    }
}
