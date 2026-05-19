use anyhow::{Context, Result};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, ToSql};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;

use crate::memory::{
    CardContinuation, MemoryCard, MemoryJob, MemorySource, MemoryStore, SessionTimeInfo,
    TurnFingerprint, TurnFingerprintCandidate,
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

const SCHEMA_V3: &str = r#"
CREATE TABLE IF NOT EXISTS memory_cards (
    card_id        TEXT PRIMARY KEY,
    project_key    TEXT NOT NULL,
    canonical_hash TEXT NOT NULL,
    simhash        TEXT,
    qmd_path       TEXT NOT NULL,
    title          TEXT NOT NULL,
    summary        TEXT,
    body           TEXT NOT NULL,
    available      INTEGER NOT NULL DEFAULT 1,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_cards_project ON memory_cards(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_memory_cards_hash ON memory_cards(canonical_hash);
CREATE INDEX IF NOT EXISTS idx_memory_cards_qmd_path ON memory_cards(qmd_path);

CREATE TABLE IF NOT EXISTS memory_sources (
    card_id     TEXT NOT NULL,
    agent       TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    file_path   TEXT NOT NULL,
    line_start  INTEGER,
    line_end    INTEGER,
    byte_start  INTEGER,
    byte_end    INTEGER,
    PRIMARY KEY(card_id, agent, session_id, file_path, line_start, line_end),
    FOREIGN KEY(card_id) REFERENCES memory_cards(card_id) ON DELETE CASCADE
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

CREATE TABLE IF NOT EXISTS memory_jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_key TEXT NOT NULL,
    scope       TEXT NOT NULL,
    kind        TEXT NOT NULL,
    status      TEXT NOT NULL,
    error       TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_jobs_project_status ON memory_jobs(project_key, status);
"#;

const SCHEMA_V4: &str = r#"
ALTER TABLE turn_fingerprints ADD COLUMN text_len INTEGER NOT NULL DEFAULT 0;
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

const SCHEMA_V5: &str = r#"
CREATE TABLE IF NOT EXISTS card_continuations (
    card_id                     TEXT PRIMARY KEY,
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
    FOREIGN KEY(card_id) REFERENCES memory_cards(card_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_card_continuations_project ON card_continuations(project_key, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_card_continuations_candidate ON card_continuations(candidate_agent, candidate_session_id);
CREATE INDEX IF NOT EXISTS idx_card_continuations_base ON card_continuations(base_agent, base_session_id);
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
    Ok(())
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
    fn upsert_card(&self, card: &MemoryCard) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO memory_cards (
                card_id, project_key, canonical_hash, simhash, qmd_path,
                title, summary, body, available, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                card.card_id,
                card.project_key,
                card.canonical_hash,
                card.simhash,
                card.qmd_path,
                card.title,
                card.summary,
                card.body,
                card.available as i64,
                card.updated_at,
            ],
        )?;
        Ok(())
    }

    fn replace_card_sources(&self, card_id: &str, sources: &[MemorySource]) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM memory_sources WHERE card_id = ?",
            params![card_id],
        )?;
        for source in sources {
            tx.execute(
                "INSERT OR REPLACE INTO memory_sources (
                    card_id, agent, session_id, file_path,
                    line_start, line_end, byte_start, byte_end
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    source.card_id,
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

    fn replace_card_continuation(
        &self,
        card_id: &str,
        continuation: Option<&CardContinuation>,
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM card_continuations WHERE card_id = ?",
            params![card_id],
        )?;
        if let Some(continuation) = continuation {
            tx.execute(
                "INSERT INTO card_continuations (
                    card_id, project_key,
                    candidate_agent, candidate_session_id, candidate_file_path,
                    base_agent, base_session_id, base_file_path,
                    base_start_turn_index, base_start_line_start, base_start_byte_start,
                    base_end_turn_index, base_end_line_end, base_end_byte_end,
                    candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                    updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    continuation.card_id,
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

    fn list_cards_for_source(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<Vec<MemoryCard>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT c.card_id, c.project_key, c.canonical_hash, c.simhash, c.qmd_path,
                    c.title, c.summary, c.body, c.available, c.updated_at
             FROM memory_cards c
             JOIN memory_sources s ON s.card_id = c.card_id
             WHERE s.agent = ? AND s.session_id = ? AND s.file_path = ?
             ORDER BY c.updated_at DESC",
        )?;
        let cards = stmt
            .query_map(params![agent, session_id, file_path], |row| {
                Ok(MemoryCard {
                    card_id: row.get(0)?,
                    project_key: row.get(1)?,
                    canonical_hash: row.get(2)?,
                    simhash: row.get(3)?,
                    qmd_path: row.get(4)?,
                    title: row.get(5)?,
                    summary: row.get(6)?,
                    body: row.get(7)?,
                    available: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(cards)
    }

    fn mark_card_unavailable(&self, card_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memory_cards SET available = 0 WHERE card_id = ?",
            params![card_id],
        )?;
        Ok(())
    }

    fn mark_source_cards_unavailable(
        &self,
        agent: &str,
        session_id: &str,
        file_path: &str,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memory_cards
             SET available = 0
             WHERE card_id IN (
                SELECT card_id
                FROM memory_sources
                WHERE agent = ? AND session_id = ? AND file_path = ?
             )",
            params![agent, session_id, file_path],
        )?;
        Ok(())
    }

    fn list_project_cards(&self, project_key: &str) -> Result<Vec<MemoryCard>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT card_id, project_key, canonical_hash, simhash, qmd_path,
                    title, summary, body, available, updated_at
             FROM memory_cards
             WHERE project_key = ?
             ORDER BY updated_at DESC",
        )?;
        let cards = stmt
            .query_map(params![project_key], |row| {
                Ok(MemoryCard {
                    card_id: row.get(0)?,
                    project_key: row.get(1)?,
                    canonical_hash: row.get(2)?,
                    simhash: row.get(3)?,
                    qmd_path: row.get(4)?,
                    title: row.get(5)?,
                    summary: row.get(6)?,
                    body: row.get(7)?,
                    available: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(cards)
    }

    fn card_by_id(&self, card_id: &str) -> Result<Option<MemoryCard>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT card_id, project_key, canonical_hash, simhash, qmd_path,
                    title, summary, body, available, updated_at
             FROM memory_cards
             WHERE card_id = ?",
        )?;
        let card = stmt
            .query_row(params![card_id], |row| {
                Ok(MemoryCard {
                    card_id: row.get(0)?,
                    project_key: row.get(1)?,
                    canonical_hash: row.get(2)?,
                    simhash: row.get(3)?,
                    qmd_path: row.get(4)?,
                    title: row.get(5)?,
                    summary: row.get(6)?,
                    body: row.get(7)?,
                    available: row.get::<_, i64>(8)? != 0,
                    updated_at: row.get(9)?,
                })
            })
            .optional()?;
        Ok(card)
    }

    fn sources_for_card(&self, card_id: &str) -> Result<Vec<MemorySource>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT card_id, agent, session_id, file_path,
                    line_start, line_end, byte_start, byte_end
             FROM memory_sources
             WHERE card_id = ?
             ORDER BY agent ASC, session_id ASC, line_start ASC",
        )?;
        let sources = stmt
            .query_map(params![card_id], |row| {
                let file_path: String = row.get(3)?;
                Ok(MemorySource {
                    card_id: row.get(0)?,
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

    fn continuation_for_card(&self, card_id: &str) -> Result<Option<CardContinuation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT card_id, project_key,
                    candidate_agent, candidate_session_id, candidate_file_path,
                    base_agent, base_session_id, base_file_path,
                    base_start_turn_index, base_start_line_start, base_start_byte_start,
                    base_end_turn_index, base_end_line_end, base_end_byte_end,
                    candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                    updated_at
             FROM card_continuations
             WHERE card_id = ?",
        )?;
        let continuation = stmt
            .query_row(params![card_id], |row| {
                Ok(CardContinuation {
                    card_id: row.get(0)?,
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
            })
            .optional()?;
        Ok(continuation)
    }

    fn continuations_for_base(
        &self,
        base_agent: &str,
        base_session_id: &str,
    ) -> Result<Vec<CardContinuation>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT card_id, project_key,
                    candidate_agent, candidate_session_id, candidate_file_path,
                    base_agent, base_session_id, base_file_path,
                    base_start_turn_index, base_start_line_start, base_start_byte_start,
                    base_end_turn_index, base_end_line_end, base_end_byte_end,
                    candidate_trim_turn_start, candidate_trim_line_start, candidate_trim_byte_start,
                    updated_at
             FROM card_continuations
             WHERE base_agent = ? AND base_session_id = ?
             ORDER BY updated_at DESC, candidate_session_id ASC, card_id ASC",
        )?;
        let rows = stmt.query_map(params![base_agent, base_session_id], |row| {
            Ok(CardContinuation {
                card_id: row.get(0)?,
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
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn invalidate_continuations_referencing_base(
        &self,
        base_agent: &str,
        base_session_id: &str,
    ) -> Result<Vec<String>> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let affected: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT card_id FROM card_continuations
                 WHERE base_agent = ? AND base_session_id = ?",
            )?;
            let rows = stmt.query_map(params![base_agent, base_session_id], |row| row.get(0))?;
            rows.collect::<rusqlite::Result<Vec<String>>>()?
        };
        tx.execute(
            "DELETE FROM card_continuations
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
        scope: &str,
        kind: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memory_jobs (
                project_key, scope, kind, status, error, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![project_key, scope, kind, status, error, now, now],
        )?;
        Ok(())
    }

    fn list_memory_jobs(&self, project_key: &str, status: Option<&str>) -> Result<Vec<MemoryJob>> {
        let conn = self.conn.lock().unwrap();
        let mut jobs = Vec::new();
        if let Some(status) = status {
            let mut stmt = conn.prepare(
                "SELECT id, project_key, scope, kind, status, error, created_at, updated_at
                 FROM memory_jobs
                 WHERE project_key = ? AND status = ?
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![project_key, status], memory_job_from_row)?;
            for row in rows {
                jobs.push(row?);
            }
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, project_key, scope, kind, status, error, created_at, updated_at
                 FROM memory_jobs
                 WHERE project_key = ?
                 ORDER BY updated_at DESC",
            )?;
            let rows = stmt.query_map(params![project_key], memory_job_from_row)?;
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
        scope: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        error: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}
