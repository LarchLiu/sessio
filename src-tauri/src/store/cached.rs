use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::models::{Agent, SessionInfo};
use crate::store::{IndexedSessionRecord, IndexedSubagentRecord, SessionStore};

// In-memory snapshot of the indexed-session view. polling reads this on every
// tick instead of hitting the underlying store; writes go through inner first
// and the snapshot is patched on success.
struct Snapshot {
    by_pk: HashMap<(Agent, String, String), IndexedSessionRecord>,
}

impl Snapshot {
    fn load_from(records: Vec<IndexedSessionRecord>) -> Self {
        let mut by_pk = HashMap::with_capacity(records.len());
        for r in records {
            by_pk.insert((r.agent, r.session_id.clone(), r.scope.clone()), r);
        }
        Self { by_pk }
    }

    fn to_vec(&self) -> Vec<IndexedSessionRecord> {
        self.by_pk.values().cloned().collect()
    }
}

pub struct CachedStore {
    inner: Arc<dyn SessionStore>,
    snapshot: RwLock<Snapshot>,
}

impl CachedStore {
    pub fn new(inner: Arc<dyn SessionStore>) -> Result<Self> {
        let records = inner.list_indexed_sessions()?;
        Ok(Self {
            inner,
            snapshot: RwLock::new(Snapshot::load_from(records)),
        })
    }

    fn refresh_from_inner(&self) -> Result<()> {
        let records = self.inner.list_indexed_sessions()?;
        *self.snapshot.write().unwrap() = Snapshot::load_from(records);
        Ok(())
    }

    // Build the IndexedSubagentRecord list from the canonical SubagentInfo.
    // file_mtime isn't carried on SubagentInfo at this layer, so we reach into
    // the filesystem the same way insert_session does (file_mtime_for in
    // sqlite). This keeps the snapshot consistent with what the inner row holds.
    fn subagent_records(session: &SessionInfo) -> Vec<IndexedSubagentRecord> {
        session
            .subagents
            .iter()
            .map(|s| IndexedSubagentRecord {
                file_path: s.file_path.clone(),
                file_size: s.file_size,
                file_mtime: file_mtime_for(&s.file_path),
            })
            .collect()
    }

    fn to_indexed(scope: &str, s: &SessionInfo) -> IndexedSessionRecord {
        IndexedSessionRecord {
            agent: s.agent,
            session_id: s.id.clone(),
            scope: scope.to_string(),
            file_path: s.file_path.clone(),
            file_size: s.file_size,
            file_mtime: file_mtime_for(&s.file_path),
            last_indexed_at: now_ms(),
            available: s.available,
            archived: s.archived,
            subagents: Self::subagent_records(s),
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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

impl SessionStore for CachedStore {
    fn init(&self) -> Result<()> {
        self.inner.init()
    }

    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        self.inner.list_sessions()
    }

    fn list_indexed_sessions(&self) -> Result<Vec<IndexedSessionRecord>> {
        Ok(self.snapshot.read().unwrap().to_vec())
    }

    fn upsert_session(&self, scope: &str, session: &SessionInfo) -> Result<()> {
        self.inner.upsert_session(scope, session)?;
        let rec = Self::to_indexed(scope, session);
        let mut snap = self.snapshot.write().unwrap();
        snap.by_pk
            .insert((rec.agent, rec.session_id.clone(), rec.scope.clone()), rec);
        Ok(())
    }

    fn replace_by_scope(
        &self,
        scope: &str,
        agent: Agent,
        sessions: &[SessionInfo],
    ) -> Result<()> {
        self.inner.replace_by_scope(scope, agent, sessions)?;
        let new_ids: HashSet<String> = sessions.iter().map(|s| s.id.clone()).collect();
        let mut snap = self.snapshot.write().unwrap();
        // Mirror inner's semantics: rows whose session_id isn't in the new set
        // get marked unavailable; rows that match are replaced wholesale.
        for ((rec_agent, sid, rec_scope), rec) in snap.by_pk.iter_mut() {
            if *rec_agent == agent && rec_scope == scope && !new_ids.contains(sid) {
                rec.available = false;
            }
        }
        for s in sessions {
            let rec = Self::to_indexed(scope, s);
            snap.by_pk
                .insert((agent, s.id.clone(), scope.to_string()), rec);
        }
        Ok(())
    }

    fn mark_file_path_unavailable(&self, file_path: &str) -> Result<()> {
        self.inner.mark_file_path_unavailable(file_path)?;
        let mut snap = self.snapshot.write().unwrap();
        for rec in snap.by_pk.values_mut() {
            if rec.file_path == file_path {
                rec.available = false;
            }
        }
        Ok(())
    }

    fn mark_missing_scopes_unavailable(
        &self,
        agent: Agent,
        present: &HashSet<String>,
    ) -> Result<()> {
        self.inner.mark_missing_scopes_unavailable(agent, present)?;
        let mut snap = self.snapshot.write().unwrap();
        for rec in snap.by_pk.values_mut() {
            if rec.agent == agent && !present.contains(&rec.scope) {
                rec.available = false;
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
impl CachedStore {
    // Exposed for tests / future maintenance commands that need to drop the
    // cache and rebuild it from the source of truth.
    pub fn rebuild_snapshot(&self) -> Result<()> {
        self.refresh_from_inner()
    }
}
