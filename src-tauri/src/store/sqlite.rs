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
    Agent, KanbanItem, KanbanStatus, ProjectInfo, ProjectType, SessionInfo, SubagentInfo,
};
use crate::store::{
    IndexedSessionRecord, IndexedSubagentRecord, RuntimeAgentCapabilityRecord, SessionStore,
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
"#;

const SCHEMA_V6: &str = r#"
ALTER TABLE sessions ADD COLUMN forked_from_agent TEXT;
"#;

const SCHEMA_V7: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id         TEXT PRIMARY KEY,
    path       TEXT NOT NULL UNIQUE,
    name       TEXT NOT NULL,
    type       TEXT NOT NULL DEFAULT 'code',
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
"#;

const SCHEMA_V8: &str = r#"
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
    if current < 6 {
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
    conn.execute_batch(SCHEMA_V5)?;
    conn.execute_batch(SCHEMA_V7)?;
    conn.execute_batch(SCHEMA_V8)?;
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

#[cfg(test)]
fn temp_child_path(parent: &Path, name: &str) -> std::path::PathBuf {
    parent.join(format!("{name}-{}", unique_suffix()))
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectInfo> {
    let project_type_raw: String = row.get(3)?;
    Ok(ProjectInfo {
        id: row.get(0)?,
        path: row.get(1)?,
        name: row.get(2)?,
        project_type: ProjectType::from_db_str(&project_type_raw).unwrap_or(ProjectType::Code),
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        session_count: row.get::<_, i64>(6)? as usize,
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

fn load_project_by_id(conn: &Connection, project_id: &str) -> Result<ProjectInfo> {
    conn.query_row(
        "SELECT p.id, p.path, p.name, p.type, p.created_at, p.updated_at,
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

    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT p.id, p.path, p.name, p.type, p.created_at, p.updated_at,
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
        project_type: ProjectType,
    ) -> Result<ProjectInfo> {
        let canonical = canonical_project_path(path)?;
        let name = clean_project_name(name, &canonical)?;
        let id = stable_project_id(&canonical);
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, path, name, type, created_at, updated_at, archived)
             VALUES (?, ?, ?, ?, ?, ?, 0)",
            params![id, canonical, name, project_type.as_str(), now, now],
        )
        .with_context(|| "add project")?;
        load_project_by_id(&conn, &id)
    }

    fn create_project(
        &self,
        parent_path: &str,
        name: &str,
        project_type: ProjectType,
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
        self.add_project(&path, Some(&clean_name), project_type)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        project_type: Option<ProjectType>,
    ) -> Result<ProjectInfo> {
        let conn = self.conn.lock().unwrap();
        let current = load_project_by_id(&conn, project_id)?;
        let next_name = match name {
            Some(value) => clean_project_name(Some(value), &current.path)?,
            None => current.name,
        };
        let next_type = project_type.unwrap_or(current.project_type);
        conn.execute(
            "UPDATE projects
             SET name = ?, type = ?, updated_at = ?
             WHERE id = ? AND archived = 0",
            params![next_name, next_type.as_str(), now_ms(), project_id],
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

        drop(conn);
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
            .add_project(&project_path, Some("Visible"), ProjectType::Research)
            .unwrap();
        assert_eq!(project.project_type, ProjectType::Research);
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
                ProjectType::VideoProduction,
            )
            .unwrap();
        assert_eq!(created.name, "video-plan");
        assert_eq!(created.project_type, ProjectType::VideoProduction);
        assert!(Path::new(&created.path).exists());

        let updated = store
            .update_project(&created.id, Some("Video Plan"), Some(ProjectType::General))
            .unwrap();
        assert_eq!(updated.name, "Video Plan");
        assert_eq!(updated.project_type, ProjectType::General);

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
    fn kanban_items_link_and_aggregate_sessions() {
        let path = unique_db("sessio-kanban-session-links");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();
        let parent = temp_child_path(&std::env::temp_dir(), "sessio-kanban-link-parent");
        std::fs::create_dir(&parent).unwrap();

        let project = store
            .create_project(&parent.to_string_lossy(), "linked", ProjectType::Code)
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
            .create_project(&other_parent.to_string_lossy(), "other", ProjectType::Code)
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
}
