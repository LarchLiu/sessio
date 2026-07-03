use anyhow::Result;
use rusqlite::Connection;

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
    selected_skill_ids_json TEXT NOT NULL DEFAULT '[]',
    selected_mcp_ids_json   TEXT NOT NULL DEFAULT '[]',
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

pub(crate) fn initialize_base_schema(conn: &Connection) -> Result<()> {
    conn.execute("DROP TABLE IF EXISTS canvas_shape_refs", [])?;
    conn.execute("DROP TABLE IF EXISTS canvas_context_anchors", [])?;
    conn.execute_batch(SCHEMA_SESSIONS)?;
    conn.execute_batch(SCHEMA_MEMORY)?;
    conn.execute_batch(SCHEMA_APP)?;
    ensure_column(
        conn,
        "assistants",
        "selected_skill_ids_json",
        "ALTER TABLE assistants ADD COLUMN selected_skill_ids_json TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(
        conn,
        "assistants",
        "selected_mcp_ids_json",
        "ALTER TABLE assistants ADD COLUMN selected_mcp_ids_json TEXT NOT NULL DEFAULT '[]'",
    )?;
    Ok(())
}

pub(crate) fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    conn.execute(alter_sql, [])?;
    Ok(())
}
