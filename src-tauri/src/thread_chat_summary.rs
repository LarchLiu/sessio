use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;

use crate::models::{Agent, PlanRoundInfo, SessionInfo, ThreadChatSummaryInfo, ThreadInfo};
use crate::store::{AstraRunRecord, SessionStore};

#[derive(Clone)]
pub struct ThreadChatSummaryCache {
    inner: Arc<ThreadChatSummaryCacheInner>,
}

struct ThreadChatSummaryCacheInner {
    store: Arc<dyn SessionStore>,
    state: RwLock<ThreadChatSummaryState>,
    refresh_lock: Mutex<()>,
}

#[derive(Default)]
struct ThreadChatSummaryState {
    all_loaded: bool,
    by_thread: HashMap<String, ThreadChatSummaryInfo>,
    by_project: HashMap<String, Vec<String>>,
}

impl ThreadChatSummaryCache {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self {
            inner: Arc::new(ThreadChatSummaryCacheInner {
                store,
                state: RwLock::new(ThreadChatSummaryState::default()),
                refresh_lock: Mutex::new(()),
            }),
        }
    }

    pub fn warm(&self) -> Result<()> {
        self.ensure_loaded()
    }

    pub fn refresh_all(&self) -> Result<()> {
        let _refresh = self.inner.refresh_lock.lock().unwrap();
        self.refresh_all_inner()
    }

    fn refresh_all_inner(&self) -> Result<()> {
        let summaries = build_all_summaries(self.inner.store.as_ref())?;
        let mut by_thread = HashMap::new();
        let mut by_project: HashMap<String, Vec<String>> = HashMap::new();
        for summary in summaries {
            by_project
                .entry(summary.project_id.clone())
                .or_default()
                .push(summary.thread_id.clone());
            by_thread.insert(summary.thread_id.clone(), summary);
        }
        sort_project_threads(&mut by_project, &by_thread);
        let mut state = self.inner.state.write().unwrap();
        *state = ThreadChatSummaryState {
            all_loaded: true,
            by_thread,
            by_project,
        };
        Ok(())
    }

    pub fn refresh_project(&self, project_id: &str) -> Result<()> {
        let _refresh = self.inner.refresh_lock.lock().unwrap();
        self.refresh_project_inner(project_id)
    }

    fn refresh_project_inner(&self, project_id: &str) -> Result<()> {
        let summaries = match build_project_summaries(self.inner.store.as_ref(), project_id) {
            Ok(summaries) => summaries,
            Err(error) if project_missing_error(&error) => Vec::new(),
            Err(error) => return Err(error),
        };
        let mut state = self.inner.state.write().unwrap();
        for thread_id in state.by_project.remove(project_id).unwrap_or_default() {
            state.by_thread.remove(&thread_id);
        }
        let mut ids = Vec::new();
        for summary in summaries {
            ids.push(summary.thread_id.clone());
            state.by_thread.insert(summary.thread_id.clone(), summary);
        }
        ids.sort_by(|a, b| {
            summary_sort_time(&state.by_thread, b).cmp(&summary_sort_time(&state.by_thread, a))
        });
        state.by_project.insert(project_id.to_string(), ids);
        Ok(())
    }

    pub fn list_project(&self, project_id: &str) -> Result<Vec<ThreadChatSummaryInfo>> {
        self.ensure_project_loaded(project_id)?;
        let state = self.inner.state.read().unwrap();
        let Some(ids) = state.by_project.get(project_id) else {
            return Ok(Vec::new());
        };
        Ok(ids
            .iter()
            .filter_map(|id| state.by_thread.get(id).cloned())
            .collect())
    }

    pub fn list_all(&self) -> Result<Vec<ThreadChatSummaryInfo>> {
        self.ensure_loaded()?;
        let state = self.inner.state.read().unwrap();
        let mut summaries = state.by_thread.values().cloned().collect::<Vec<_>>();
        summaries.sort_by(|a, b| b.time.cmp(&a.time));
        Ok(summaries)
    }

    fn ensure_loaded(&self) -> Result<()> {
        if self.inner.state.read().unwrap().all_loaded {
            return Ok(());
        }
        let _refresh = self.inner.refresh_lock.lock().unwrap();
        if self.inner.state.read().unwrap().all_loaded {
            return Ok(());
        }
        self.refresh_all_inner()
    }

    fn ensure_project_loaded(&self, project_id: &str) -> Result<()> {
        let state = self.inner.state.read().unwrap();
        if state.all_loaded || state.by_project.contains_key(project_id) {
            return Ok(());
        }
        drop(state);
        let _refresh = self.inner.refresh_lock.lock().unwrap();
        let state = self.inner.state.read().unwrap();
        if state.all_loaded || state.by_project.contains_key(project_id) {
            return Ok(());
        }
        drop(state);
        self.refresh_project_inner(project_id)
    }
}

fn build_all_summaries(store: &dyn SessionStore) -> Result<Vec<ThreadChatSummaryInfo>> {
    let session_lookup = session_lookup(store)?;
    let mut summaries = Vec::new();
    for project in store.list_projects()? {
        summaries.extend(build_project_summaries_with_lookup(
            store,
            &project.id,
            &session_lookup,
        )?);
    }
    Ok(summaries)
}

fn build_project_summaries(
    store: &dyn SessionStore,
    project_id: &str,
) -> Result<Vec<ThreadChatSummaryInfo>> {
    let session_lookup = session_lookup(store)?;
    build_project_summaries_with_lookup(store, project_id, &session_lookup)
}

fn project_missing_error(error: &anyhow::Error) -> bool {
    error.to_string().starts_with("project not found:")
}

fn build_project_summaries_with_lookup(
    store: &dyn SessionStore,
    project_id: &str,
    session_lookup: &HashMap<(Agent, String), SessionInfo>,
) -> Result<Vec<ThreadChatSummaryInfo>> {
    let mut summaries = Vec::new();
    for thread in store.list_threads(project_id)? {
        let plan_rounds = store.list_plan_rounds(&thread.id)?;
        let astra_runs = store.list_astra_runs(&thread.id)?;
        if let Some(summary) = build_thread_summary(thread, plan_rounds, astra_runs, session_lookup)
        {
            summaries.push(summary);
        }
    }
    summaries.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(summaries)
}

fn session_lookup(store: &dyn SessionStore) -> Result<HashMap<(Agent, String), SessionInfo>> {
    Ok(store
        .list_all_sessions()?
        .into_iter()
        .map(|session| ((session.agent, session.id.clone()), session))
        .collect())
}

fn build_thread_summary(
    thread: ThreadInfo,
    plan_rounds: Vec<PlanRoundInfo>,
    astra_runs: Vec<AstraRunRecord>,
    session_lookup: &HashMap<(Agent, String), SessionInfo>,
) -> Option<ThreadChatSummaryInfo> {
    let mut sessions_by_key = HashMap::<String, SessionInfo>::new();
    let mut session_keys = HashSet::<String>::new();
    let mut latest = thread.updated_at.max(thread.created_at);

    for session in &thread.sessions {
        latest = latest.max(session_time(session));
        add_session(&mut sessions_by_key, &mut session_keys, session.clone());
    }
    for stage in &thread.stages {
        latest = latest.max(stage.updated_at.max(stage.created_at));
        for session in &stage.sessions {
            latest = latest.max(session_time(session));
            add_session(&mut sessions_by_key, &mut session_keys, session.clone());
        }
    }

    for round in plan_rounds {
        latest = latest.max(round.updated_at.max(round.created_at));
        for task in round.tasks {
            latest = latest.max(task.updated_at.max(task.created_at));
            for task_session in task.sessions {
                latest = latest.max(task_session.updated_at.max(task_session.created_at));
                add_session_ref(
                    &mut sessions_by_key,
                    &mut session_keys,
                    session_lookup,
                    task_session.agent,
                    &task_session.session_id,
                );
            }
        }
    }

    for run in astra_runs {
        latest = latest.max(run.updated_at.max(run.created_at));
        let planner_agent =
            replay_agent_for_backend(run.planner_backend.as_deref()).unwrap_or(Agent::AstraPi);
        for session_id in parse_session_id_vec(&run.internal_planner_session_ids_json) {
            add_session_ref(
                &mut sessions_by_key,
                &mut session_keys,
                session_lookup,
                planner_agent,
                &session_id,
            );
        }
    }

    let mut sessions = sessions_by_key.into_values().collect::<Vec<_>>();
    sessions.sort_by(|a, b| session_time(b).cmp(&session_time(a)));
    let mut session_keys = session_keys.into_iter().collect::<Vec<_>>();
    session_keys.sort();

    Some(ThreadChatSummaryInfo {
        thread_id: thread.id,
        project_id: thread.project_id,
        goal: thread.goal,
        created_at: thread.created_at,
        updated_at: thread.updated_at,
        time: latest,
        sessions,
        session_keys,
    })
}

fn add_session_ref(
    sessions_by_key: &mut HashMap<String, SessionInfo>,
    session_keys: &mut HashSet<String>,
    session_lookup: &HashMap<(Agent, String), SessionInfo>,
    agent: Agent,
    session_id: &str,
) {
    session_keys.insert(session_identity(agent, session_id));
    if let Some(session) = session_lookup.get(&(agent, session_id.to_string())) {
        add_session(sessions_by_key, session_keys, session.clone());
    }
}

fn add_session(
    sessions_by_key: &mut HashMap<String, SessionInfo>,
    session_keys: &mut HashSet<String>,
    session: SessionInfo,
) {
    let key = session_identity(session.agent, &session.id);
    session_keys.insert(key.clone());
    let replace = sessions_by_key
        .get(&key)
        .map(|current| better_session_candidate(&session, current))
        .unwrap_or(true);
    if replace {
        sessions_by_key.insert(key, session);
    }
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
    session_time(candidate) > session_time(current)
}

fn is_real_session_file_path(file_path: &str) -> bool {
    let trimmed = file_path.trim();
    !trimmed.is_empty() && !trimmed.starts_with("astra://")
}

fn session_time(session: &SessionInfo) -> i64 {
    session.updated_at.or(session.started_at).unwrap_or(0)
}

fn session_identity(agent: Agent, session_id: &str) -> String {
    format!("{}:{session_id}", agent.as_str())
}

fn parse_session_id_vec(value: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(value).unwrap_or_default()
}

fn replay_agent_for_backend(backend: Option<&str>) -> Option<Agent> {
    let backend = backend?.trim();
    if backend.is_empty() {
        return None;
    }
    if backend == "astra_pi_acp" || backend == Agent::AstraPi.as_str() {
        return Some(Agent::AstraPi);
    }
    if let Some(agent) = backend.strip_prefix("runtime_agent_") {
        return Agent::from_db_str(agent);
    }
    Agent::from_db_str(backend)
}

fn sort_project_threads(
    by_project: &mut HashMap<String, Vec<String>>,
    by_thread: &HashMap<String, ThreadChatSummaryInfo>,
) {
    for ids in by_project.values_mut() {
        ids.sort_by(|a, b| summary_sort_time(by_thread, b).cmp(&summary_sort_time(by_thread, a)));
    }
}

fn summary_sort_time(by_thread: &HashMap<String, ThreadChatSummaryInfo>, thread_id: &str) -> i64 {
    by_thread
        .get(thread_id)
        .map(|summary| summary.time)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::store::sqlite::SqliteStore;

    #[test]
    fn summary_keeps_thread_chat_entry_without_sessions() {
        let path = temp_db_path("sessio-thread-chat-summary-empty");
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        store.init().unwrap();
        let project_dir = temp_project_path("sessio-thread-chat-summary-empty-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project = store
            .create_project(
                &project_dir.to_string_lossy(),
                "thread-chat-summary-empty",
                "code".to_string(),
                None,
            )
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Show thread chat entry", None)
            .unwrap();

        let cache = ThreadChatSummaryCache::new(store.clone());
        let summaries = cache.list_project(&project.id).unwrap();
        let summary = summaries
            .iter()
            .find(|summary| summary.thread_id == thread.id)
            .unwrap();
        assert_eq!(summary.goal, thread.goal);
        assert!(summary.sessions.is_empty());
        assert!(summary.session_keys.is_empty());

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    fn temp_db_path(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}.db", unique_suffix()))
    }

    fn temp_project_path(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", unique_suffix()))
    }

    fn unique_suffix() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{}-{nanos}", std::process::id())
    }
}
