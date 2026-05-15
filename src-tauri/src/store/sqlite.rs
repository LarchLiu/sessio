use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use crate::models::{Agent, SessionInfo, SubagentInfo};
use crate::store::SessionStore;

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open sqlite at {}", path.display()))?;
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
    PRIMARY KEY (parent_agent, parent_session_id, subagent_id)
);

CREATE INDEX IF NOT EXISTS idx_subagents_parent ON subagents(parent_agent, parent_session_id);
"#;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn run_migrations(conn: &Connection) -> Result<()> {
    let current: Option<i64> = conn
        .query_row(
            "SELECT MAX(version) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if current.unwrap_or(0) < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.execute("INSERT OR IGNORE INTO schema_migrations(version) VALUES (1)", [])?;
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
            last_indexed_at
        ) VALUES (?,?,?,?, ?,?, ?,?, ?,?, ?,?, ?,?,?, ?)",
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
        ],
    )?;
    replace_subagents_inner(conn, s.agent, &s.id, &s.subagents)?;
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

fn replace_subagents_inner(
    conn: &Connection,
    parent_agent: Agent,
    parent_session_id: &str,
    items: &[SubagentInfo],
) -> Result<()> {
    conn.execute(
        "DELETE FROM subagents WHERE parent_agent = ? AND parent_session_id = ?",
        params![parent_agent.as_str(), parent_session_id],
    )?;
    for sub in items {
        conn.execute(
            "INSERT OR REPLACE INTO subagents (
                parent_agent, parent_session_id, subagent_id, file_path,
                agent_type, description,
                started_at, updated_at,
                message_count, first_user_message,
                file_size, file_mtime, partial
            ) VALUES (?,?,?,?, ?,?, ?,?, ?,?, ?,?,?)",
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
            ],
        )?;
    }
    Ok(())
}

fn load_subagents_for(
    conn: &Connection,
    parent_agent: Agent,
    parent_session_id: &str,
) -> Result<Vec<SubagentInfo>> {
    let mut stmt = conn.prepare(
        "SELECT subagent_id, file_path, agent_type, description,
                started_at, updated_at, message_count, first_user_message,
                file_size, partial
         FROM subagents
         WHERE parent_agent = ? AND parent_session_id = ?
         ORDER BY started_at ASC",
    )?;
    let rows = stmt
        .query_map(
            params![parent_agent.as_str(), parent_session_id],
            |row| {
                Ok(SubagentInfo {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    agent_type: row.get(2)?,
                    description: row.get(3)?,
                    started_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    message_count: row.get::<_, i64>(6)? as usize,
                    first_user_message: row.get(7)?,
                    file_size: row.get::<_, i64>(8)? as u64,
                    partial: row.get::<_, i64>(9)? != 0,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

impl SessionStore for SqliteStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        run_migrations(&conn)
    }

    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent, session_id, file_path, project_path, project_name,
                    started_at, updated_at, message_count, first_user_message,
                    file_size, partial, available, archived
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
            s.subagents = load_subagents_for(&conn, s.agent, &s.id)?;
        }
        Ok(sessions)
    }

    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        insert_session(&conn, scope, session)
    }

    fn replace_by_scope(
        &self,
        scope: &str,
        agent: Agent,
        sessions: &[SessionInfo],
    ) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let stale_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT session_id FROM sessions WHERE scope = ? AND agent = ?",
            )?;
            let rows = stmt.query_map(params![scope, agent.as_str()], |r| r.get::<_, String>(0))?;
            let mut v = Vec::new();
            for r in rows {
                v.push(r?);
            }
            v
        };
        tx.execute(
            "DELETE FROM sessions WHERE scope = ? AND agent = ?",
            params![scope, agent.as_str()],
        )?;
        let new_ids: HashSet<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        for sid in &stale_ids {
            if !new_ids.contains(sid.as_str()) {
                tx.execute(
                    "DELETE FROM subagents WHERE parent_agent = ? AND parent_session_id = ?",
                    params![agent.as_str(), sid],
                )?;
            }
        }
        for s in sessions {
            insert_session(&tx, scope, s)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn delete_by_file_path(&self, file_path: &str) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let rows: Vec<(String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT agent, session_id FROM sessions WHERE file_path = ?",
            )?;
            let rs = stmt.query_map(params![file_path], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for r in rs {
                out.push(r?);
            }
            out
        };
        for (agent_str, sid) in &rows {
            tx.execute(
                "DELETE FROM subagents WHERE parent_agent = ? AND parent_session_id = ?",
                params![agent_str, sid],
            )?;
        }
        tx.execute("DELETE FROM sessions WHERE file_path = ?", params![file_path])?;
        tx.commit()?;
        Ok(())
    }

    fn purge_missing_scopes(&self, agent: Agent, present: &HashSet<String>) -> Result<()> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let all_scopes: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT scope FROM sessions WHERE agent = ?",
            )?;
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
            let stale_ids: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT session_id FROM sessions WHERE scope = ? AND agent = ?",
                )?;
                let rs = stmt.query_map(params![scope, agent.as_str()], |r| r.get::<_, String>(0))?;
                let mut v = Vec::new();
                for r in rs {
                    v.push(r?);
                }
                v
            };
            for sid in &stale_ids {
                tx.execute(
                    "DELETE FROM subagents WHERE parent_agent = ? AND parent_session_id = ?",
                    params![agent.as_str(), sid],
                )?;
            }
            tx.execute(
                "DELETE FROM sessions WHERE scope = ? AND agent = ?",
                params![scope, agent.as_str()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
