use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::memory::{
    MemoryArtifact, MemoryJob, MemoryRecord, MemoryRecordKind, MemorySource, MemoryStore,
    RecordContinuation, SessionTimeInfo, TurnFingerprint, TurnFingerprintCandidate,
};
use crate::models::{Agent, SessionInfo, SubagentInfo};
use crate::providers::types::SourceLocation;
use crate::store::{IndexedSessionRecord, IndexedSubagentRecord, SessionStore};

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

// V3 was rewritten to the post-V8 shape (`record_id` instead of `card_id`,
// `memory_artifacts` table, no `qmd_path` column) so a fresh install reaches
// the same end state without running V8. V8 still ships verbatim for upgrades
// from pre-V8 databases — it's guarded by `memory_cards_has_qmd_path` so it
// does nothing on installs that already came in via the new V3.
const SCHEMA_V3: &str = r#"
CREATE TABLE IF NOT EXISTS memory_cards (
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

CREATE INDEX IF NOT EXISTS idx_memory_cards_project ON memory_cards(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_cards_hash ON memory_cards(canonical_hash);

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
    FOREIGN KEY(record_id) REFERENCES memory_cards(record_id) ON DELETE CASCADE
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
    line_start     INTEGER,
    line_end       INTEGER,
    byte_start     INTEGER,
    byte_end       INTEGER,
    PRIMARY KEY(project_key, agent, session_id, turn_index)
);

CREATE INDEX IF NOT EXISTS idx_turn_fingerprints_hash ON turn_fingerprints(canonical_hash);

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
ALTER TABLE turn_fingerprints ADD COLUMN text_len INTEGER NOT NULL DEFAULT 0;
"#;

const SCHEMA_V5: &str = r#"
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
    FOREIGN KEY(record_id) REFERENCES memory_cards(record_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_record_continuations_project ON record_continuations(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_record_continuations_candidate ON record_continuations(candidate_agent, candidate_session_id);
CREATE INDEX IF NOT EXISTS idx_record_continuations_base ON record_continuations(base_agent, base_session_id);
"#;

// text_len=0 means the row predates the V4 migration and would silently
// underweight in dedupe scoring. Drop those rows so they get rebuilt with
// real lengths on the next build pass. Idempotent: new rows always carry
// a non-zero text_len, so re-running this is a no-op.
const SCHEMA_V6: &str = r#"
DELETE FROM turn_fingerprints WHERE text_len = 0;
"#;

const SCHEMA_V7: &str = r#"
ALTER TABLE sessions ADD COLUMN forked_from_id TEXT;
"#;

// V8 only runs on pre-V8 databases (guarded by `memory_cards_has_qmd_path`).
// On fresh installs V3 already provisions the post-V8 shape, so the guard
// returns false and this block is a no-op. The SQL deliberately uses the
// old `card_id` / `qmd_path` names — those columns only exist on the
// pre-V8 schema this migration is meant to upgrade.
const SCHEMA_V8: &str = r#"
CREATE TABLE IF NOT EXISTS memory_artifacts (
    record_id    TEXT NOT NULL,
    backend      TEXT NOT NULL,
    artifact_uri TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY(record_id, backend)
);

CREATE INDEX IF NOT EXISTS idx_memory_artifacts_backend ON memory_artifacts(backend, artifact_uri);

INSERT INTO memory_artifacts (
    record_id, backend, artifact_uri, content_hash, updated_at
) SELECT
    card_id, 'qmd', qmd_path, canonical_hash, updated_at
FROM memory_cards;

DROP INDEX IF EXISTS idx_memory_cards_qmd_path;
ALTER TABLE memory_cards DROP COLUMN qmd_path;
ALTER TABLE memory_cards RENAME COLUMN card_id TO record_id;
ALTER TABLE memory_sources RENAME COLUMN card_id TO record_id;
ALTER TABLE memory_cards ADD COLUMN kind TEXT NOT NULL DEFAULT 'session';
"#;

// V9 only runs on pre-V9 databases where memory_jobs lacks a backend column.
// On fresh installs V3 already declares `backend TEXT NOT NULL DEFAULT 'qmd'`,
// so the ALTER fails (column exists) and we swallow the error. The index
// re-create is idempotent.
const SCHEMA_V9: &str = r#"
ALTER TABLE memory_jobs ADD COLUMN backend TEXT NOT NULL DEFAULT 'qmd';
CREATE INDEX IF NOT EXISTS idx_memory_jobs_project_status ON memory_jobs(project_key, backend, status);
"#;

// V10 renames the legacy `card_continuations` table (created by pre-V10 V5)
// to `record_continuations` and its `card_id` column to `record_id`. Only
// runs when `card_continuations` exists; on fresh installs V5 already
// provisions the new name so the guard returns false and V10 is a no-op.
const SCHEMA_V10: &str = r#"
ALTER TABLE card_continuations RENAME TO record_continuations;
ALTER TABLE record_continuations RENAME COLUMN card_id TO record_id;
DROP INDEX IF EXISTS idx_card_continuations_project;
DROP INDEX IF EXISTS idx_card_continuations_candidate;
DROP INDEX IF EXISTS idx_card_continuations_base;
CREATE INDEX IF NOT EXISTS idx_record_continuations_project ON record_continuations(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_record_continuations_candidate ON record_continuations(candidate_agent, candidate_session_id);
CREATE INDEX IF NOT EXISTS idx_record_continuations_base ON record_continuations(base_agent, base_session_id);
"#;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
        conn.execute_batch(SCHEMA_V6)?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (6)",
            [],
        )?;
    }
    if current < 7 {
        let _ = conn.execute_batch(SCHEMA_V7);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (7)",
            [],
        )?;
    }
    if current < 8 {
        if memory_cards_has_qmd_path(conn)? {
            conn.execute_batch(SCHEMA_V8)?;
        }
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (8)",
            [],
        )?;
    }
    if current < 9 {
        let _ = conn.execute_batch(SCHEMA_V9);
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (9)",
            [],
        )?;
    }
    if current < 10 {
        if table_exists(conn, "card_continuations")? {
            conn.execute_batch(SCHEMA_V10)?;
        }
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version) VALUES (10)",
            [],
        )?;
    }
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let mut stmt =
        conn.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name = ? LIMIT 1")?;
    let exists = stmt.exists(params![table])?;
    Ok(exists)
}

fn memory_cards_has_qmd_path(conn: &Connection) -> Result<bool> {
    let mut stmt = conn.prepare("PRAGMA table_info(memory_cards)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == "qmd_path" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn insert_session(conn: &Connection, scope: &str, s: &SessionInfo) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sessions (
            agent, session_id, scope, file_path,
            project_path, project_name,
            started_at, updated_at,
            message_count, first_user_message,
            file_size, file_mtime,
            partial, available, archived,
            last_indexed_at, forked_from_id
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?, ?,?, ?,?,?, ?,?)",
        params![
            s.agent.as_str(),
            s.id,
            scope,
            s.file_path,
            s.project_path,
            s.project_name,
            s.started_at,
            s.updated_at,
            s.message_count as i64,
            s.first_user_message,
            s.file_size as i64,
            file_mtime_for(&s.file_path),
            s.partial as i64,
            s.available as i64,
            s.archived as i64,
            now_ms(),
            s.forked_from_id,
        ],
    )?;
    // Subagent rows are written through upsert_subagent so their lifecycle
    // is independent from the parent session's reindex.
    Ok(())
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

fn read_record_kind(
    row: &rusqlite::Row<'_>,
    idx: usize,
) -> rusqlite::Result<MemoryRecordKind> {
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
            sub.message_count as i64,
            sub.first_user_message,
            sub.file_size as i64,
            file_mtime_for(&sub.file_path),
            sub.partial as i64,
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

impl SessionStore for SqliteStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        run_migrations(&conn)
    }

    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut subs_by_parent = load_all_subagents_grouped(&conn)?;
        let mut stmt = conn.prepare(
            "SELECT agent, session_id, file_path, project_path, project_name,
                    started_at, updated_at, message_count, first_user_message,
                    file_size, partial, available, archived, forked_from_id
             FROM sessions
             ORDER BY updated_at DESC",
        )?;
        let mut sessions: Vec<SessionInfo> = stmt
            .query_map([], |row| {
                let agent_str: String = row.get(0)?;
                let agent = Agent::from_db_str(&agent_str).unwrap_or(Agent::Codex);
                Ok(SessionInfo {
                    id: row.get(1)?,
                    agent,
                    forked_from_id: row.get(13)?,
                    file_path: row.get(2)?,
                    project_path: row.get(3)?,
                    project_name: row.get(4)?,
                    started_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    message_count: row.get::<_, i64>(7)? as usize,
                    first_user_message: row.get(8)?,
                    file_size: row.get::<_, i64>(9)? as u64,
                    partial: row.get::<_, i64>(10)? != 0,
                    available: row.get::<_, i64>(11)? != 0,
                    archived: row.get::<_, i64>(12)? != 0,
                    subagents: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for s in sessions.iter_mut() {
            s.subagents = subs_by_parent
                .remove(&(s.agent, s.id.clone()))
                .unwrap_or_default();
        }
        Ok(sessions)
    }

    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>> {
        let conn = self.conn.lock().unwrap();
        let mut subs_by_parent = load_all_indexed_subagents_grouped(&conn)?;
        let mut stmt = conn.prepare(
            "SELECT agent, session_id, scope, file_path, file_size, file_mtime, last_indexed_at, available, archived
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

    fn mark_subagent_file_unavailable(&self, file_path: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE subagents SET available = 0 WHERE file_path = ?",
            params![file_path],
        )?;
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
                "UPDATE sessions SET available = 0 WHERE scope = ? AND agent = ?",
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
            "INSERT OR REPLACE INTO memory_cards (
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
             FROM memory_cards c
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
            "UPDATE memory_cards SET available = 0 WHERE record_id = ?",
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
            "UPDATE memory_cards
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
             FROM memory_cards
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
             FROM memory_cards
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

fn record_continuation_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RecordContinuation> {
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_db(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.db"))
    }

    // Verify a synthetic pre-V8 schema (memory_cards uses `card_id` + `qmd_path`
    // and there's no `memory_artifacts` table or `memory_jobs.backend` column)
    // migrates cleanly into the current shape and existing rows survive the
    // rename / artifact extraction.
    #[test]
    fn migrates_pre_v8_database_to_current_schema() {
        let path = unique_db("sessio-mig-prev8");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY);
                INSERT INTO schema_migrations(version) VALUES (1),(2),(3),(4),(5),(6),(7);
                CREATE TABLE sessions (
                    agent TEXT NOT NULL, session_id TEXT NOT NULL, scope TEXT NOT NULL,
                    file_path TEXT NOT NULL, project_path TEXT, project_name TEXT,
                    started_at INTEGER, updated_at INTEGER,
                    message_count INTEGER NOT NULL DEFAULT 0, first_user_message TEXT,
                    file_size INTEGER NOT NULL DEFAULT 0, file_mtime INTEGER,
                    partial INTEGER NOT NULL DEFAULT 0, available INTEGER NOT NULL DEFAULT 1,
                    archived INTEGER NOT NULL DEFAULT 0,
                    last_indexed_at INTEGER NOT NULL, forked_from_id TEXT,
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
                CREATE TABLE memory_cards (
                    card_id TEXT PRIMARY KEY, project_key TEXT NOT NULL,
                    canonical_hash TEXT NOT NULL, simhash TEXT,
                    qmd_path TEXT NOT NULL,
                    title TEXT NOT NULL, summary TEXT, body TEXT NOT NULL,
                    available INTEGER NOT NULL DEFAULT 1,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE memory_sources (
                    card_id TEXT NOT NULL, agent TEXT NOT NULL,
                    session_id TEXT NOT NULL, file_path TEXT NOT NULL,
                    line_start INTEGER, line_end INTEGER,
                    byte_start INTEGER, byte_end INTEGER,
                    PRIMARY KEY(card_id, agent, session_id, file_path, line_start, line_end)
                );
                CREATE TABLE turn_fingerprints (
                    project_key TEXT NOT NULL, agent TEXT NOT NULL,
                    session_id TEXT NOT NULL, turn_index INTEGER NOT NULL,
                    role TEXT NOT NULL, canonical_hash TEXT NOT NULL,
                    file_path TEXT NOT NULL,
                    line_start INTEGER, line_end INTEGER,
                    byte_start INTEGER, byte_end INTEGER,
                    text_len INTEGER NOT NULL DEFAULT 5,
                    PRIMARY KEY(project_key, agent, session_id, turn_index)
                );
                CREATE TABLE memory_jobs (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    project_key TEXT NOT NULL,
                    scope TEXT NOT NULL, kind TEXT NOT NULL, status TEXT NOT NULL,
                    error TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
                );
                CREATE TABLE card_continuations (
                    card_id TEXT PRIMARY KEY,
                    project_key TEXT NOT NULL,
                    candidate_agent TEXT NOT NULL,
                    candidate_session_id TEXT NOT NULL,
                    candidate_file_path TEXT NOT NULL,
                    base_agent TEXT NOT NULL,
                    base_session_id TEXT NOT NULL,
                    base_file_path TEXT NOT NULL,
                    base_start_turn_index INTEGER NOT NULL,
                    base_start_line_start INTEGER,
                    base_start_byte_start INTEGER,
                    base_end_turn_index INTEGER NOT NULL,
                    base_end_line_end INTEGER,
                    base_end_byte_end INTEGER,
                    candidate_trim_turn_start INTEGER NOT NULL,
                    candidate_trim_line_start INTEGER,
                    candidate_trim_byte_start INTEGER,
                    updated_at INTEGER NOT NULL
                );
                INSERT INTO memory_cards(card_id, project_key, canonical_hash, qmd_path, title, body, updated_at)
                    VALUES ('sessio-codex-x', 'proj', 'h1', '/legacy/path/sessio-codex-x.md', 'T', 'B', 100);
                INSERT INTO memory_sources(card_id, agent, session_id, file_path)
                    VALUES ('sessio-codex-x', 'codex', 'x', '/tmp/x.jsonl');
                INSERT INTO memory_jobs(project_key, scope, kind, status, created_at, updated_at)
                    VALUES ('proj', '/tmp/proj', 'memory_build', 'succeeded', 1, 2);
                "#,
            )
            .unwrap();
        }

        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let conn = store.conn.lock().unwrap();
        // Card row survived rename: card_id → record_id and got default kind.
        let (record_id, kind, available): (String, String, i64) = conn
            .query_row(
                "SELECT record_id, kind, available FROM memory_cards WHERE record_id = ?",
                params!["sessio-codex-x"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(record_id, "sessio-codex-x");
        assert_eq!(kind, "session");
        assert_eq!(available, 1);

        // memory_artifacts row was populated from the legacy qmd_path.
        let (artifact_record_id, backend, uri, hash): (String, String, String, String) = conn
            .query_row(
                "SELECT record_id, backend, artifact_uri, content_hash FROM memory_artifacts WHERE record_id = ?",
                params!["sessio-codex-x"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(artifact_record_id, "sessio-codex-x");
        assert_eq!(backend, "qmd");
        assert_eq!(uri, "/legacy/path/sessio-codex-x.md");
        assert_eq!(hash, "h1");

        // memory_sources row was renamed: card_id → record_id.
        let source_record_id: String = conn
            .query_row(
                "SELECT record_id FROM memory_sources WHERE session_id = ?",
                params!["x"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source_record_id, "sessio-codex-x");

        // memory_jobs.backend column was added with default 'qmd'.
        let job_backend: String = conn
            .query_row(
                "SELECT backend FROM memory_jobs WHERE project_key = ?",
                params!["proj"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(job_backend, "qmd");

        let qmd_path_present = {
            let mut stmt = conn.prepare("PRAGMA table_info(memory_cards)").unwrap();
            let mut rows = stmt.query([]).unwrap();
            let mut found = false;
            while let Some(row) = rows.next().unwrap() {
                let name: String = row.get(1).unwrap();
                if name == "qmd_path" {
                    found = true;
                    break;
                }
            }
            found
        };
        assert!(!qmd_path_present, "qmd_path column must be dropped");

        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    // Verify a fresh install reaches the post-V8/V9 shape without ever
    // running the V8 column rewrite (qmd_path never exists).
    #[test]
    fn fresh_install_skips_v8_column_rewrite() {
        let path = unique_db("sessio-mig-fresh");
        let store = SqliteStore::open(&path).unwrap();
        store.init().unwrap();

        let conn = store.conn.lock().unwrap();
        let columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(memory_cards)").unwrap();
            let rows = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
            rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(columns.contains(&"record_id".to_string()));
        assert!(columns.contains(&"kind".to_string()));
        assert!(!columns.contains(&"card_id".to_string()));
        assert!(!columns.contains(&"qmd_path".to_string()));

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
}
