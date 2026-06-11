use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use anyhow::Result;

use crate::models::{Agent, PlanRoundInfo, SessionInfo, ThreadChatSummaryInfo, ThreadInfo};
use crate::store::{
    better_session_candidate, collect_referenced_session_keys, insert_best_session,
    session_identity, session_time, AstraRunRecord, SessionRef, SessionStore,
};

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
    let mut summaries = Vec::new();
    for project in store.list_projects()? {
        summaries.extend(build_project_summaries(store, &project.id)?);
    }
    Ok(summaries)
}

fn build_project_summaries(
    store: &dyn SessionStore,
    project_id: &str,
) -> Result<Vec<ThreadChatSummaryInfo>> {
    let mut session_lookup = HashMap::<(Agent, String), SessionInfo>::new();
    let mut inputs = Vec::new();
    let mut unresolved = HashSet::<(Agent, String)>::new();
    for thread in store.list_threads(project_id)? {
        let plan_rounds = store.list_plan_rounds(&thread.id)?;
        let astra_runs = store.list_astra_runs(&thread.id)?;
        for session in &thread.sessions {
            insert_best_session(&mut session_lookup, session.clone());
        }
        for stage in &thread.stages {
            for session in &stage.sessions {
                insert_best_session(&mut session_lookup, session.clone());
            }
        }
        unresolved.extend(collect_referenced_session_keys(
            &plan_rounds,
            &astra_runs,
            &session_lookup,
        ));
        inputs.push((thread, plan_rounds, astra_runs));
    }
    if !unresolved.is_empty() {
        let refs = unresolved
            .iter()
            .map(|(agent, session_id)| SessionRef {
                agent: *agent,
                session_id: session_id.as_str(),
            })
            .collect::<Vec<_>>();
        for session in store.list_sessions_by_refs(&refs)? {
            insert_best_session(&mut session_lookup, session);
        }
    }
    let mut summaries = Vec::new();
    for (thread, plan_rounds, astra_runs) in inputs {
        if let Some(summary) =
            build_thread_summary(thread, plan_rounds, astra_runs, &session_lookup)
        {
            summaries.push(summary);
        }
    }
    summaries.sort_by(|a, b| b.time.cmp(&a.time));
    Ok(summaries)
}

fn project_missing_error(error: &anyhow::Error) -> bool {
    error.to_string().starts_with("project not found:")
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
                if task_session.superseded_at.is_some() {
                    continue;
                }
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
        for session_ref in run.internal_planner_sessions {
            add_session_ref(
                &mut sessions_by_key,
                &mut session_keys,
                session_lookup,
                session_ref.agent,
                &session_ref.session_id,
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
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::models::{
        PlanRoundMode, PlanRoundSource, PlanRoundStatus, PlanTaskInfo, PlanTaskRisk,
        PlanTaskSessionInfo, PlanTaskSessionRole, PlanTaskStatus,
    };
    use crate::store::sqlite::SqliteStore;
    use crate::store::{collect_referenced_session_keys, AstraRunRecord, AstraRunSessionRecord};

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

    #[test]
    fn summary_resolves_direct_stage_plan_and_astra_sessions() {
        let path = temp_db_path("sessio-thread-chat-summary-sources");
        let store = Arc::new(SqliteStore::open(&path).unwrap());
        store.init().unwrap();
        let project_dir = temp_project_path("sessio-thread-chat-summary-sources-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project = store
            .create_project(
                &project_dir.to_string_lossy(),
                "thread-chat-summary-sources",
                "code".to_string(),
                None,
            )
            .unwrap();
        let assistant = store
            .create_assistant(crate::store::NewAssistant {
                name: "Summary Assistant",
                agent: crate::models::AssistantAgentInfo {
                    id: "codex".to_string(),
                    name: "Codex".to_string(),
                    model: "gpt-5.3-codex".to_string(),
                    mode: "workspace-write".to_string(),
                    effort: "medium".to_string(),
                },
                system_prompt: None,
                color: None,
                assistant_type: crate::models::AssistantType::Custom,
                process_template_id: None,
                project_id: Some(&project.id),
            })
            .unwrap();
        let thread = store
            .create_thread(&project.id, "Summarize every source", None)
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
            first_user_message: Some("Thread note".to_string()),
            file_path: project_dir
                .join("direct.jsonl")
                .to_string_lossy()
                .to_string(),
            file_size: 1,
            partial: false,
            available: true,
            archived: false,
            subagents: Vec::new(),
        };
        let stage_runtime_session = SessionInfo {
            id: "stage-runtime-session".to_string(),
            started_at: Some(30),
            updated_at: Some(40),
            message_count: 4,
            title: Some("Stage runtime".to_string()),
            first_user_message: Some("Stage note".to_string()),
            file_path: project_dir
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
            file_path: project_dir
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
            .create_plan_round(crate::store::NewPlanRound {
                thread_id: &thread.id,
                astra_run_id: None,
                round_index: None,
                summary: Some("Summary round"),
                mode: PlanRoundMode::Parallel,
                source: PlanRoundSource::Astra,
                status: PlanRoundStatus::Running,
                tasks: vec![crate::store::NewPlanTask {
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
        store
            .link_plan_task_session(crate::store::NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: "superseded-runtime-session",
                role: PlanTaskSessionRole::Runtime,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();
        store
            .link_plan_task_session(crate::store::NewPlanTaskSession {
                task_id: &round.tasks[0].id,
                agent: Agent::Codex,
                session_id: &stage_runtime_session.id,
                role: PlanTaskSessionRole::Runtime,
                attempt_id: None,
                attempt_count: 1,
            })
            .unwrap();
        store
            .link_plan_task_session(crate::store::NewPlanTaskSession {
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
                run_id: "summary-run".to_string(),
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
                        run_id: "summary-run".to_string(),
                        agent: Agent::AstraPi,
                        session_id: planner_session.id.clone(),
                        role: PlanTaskSessionRole::Planner,
                        sort_order: 0,
                        created_at: 70,
                        updated_at: 80,
                    },
                    AstraRunSessionRecord {
                        run_id: "summary-run".to_string(),
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

        let cache = ThreadChatSummaryCache::new(store.clone());
        let summaries = cache.list_project(&project.id).unwrap();
        let summary = summaries
            .iter()
            .find(|summary| summary.thread_id == thread.id)
            .unwrap();

        assert_eq!(summary.goal, thread.goal);
        assert!(summary.time >= 82);
        assert!(summary.time >= summary.updated_at);
        assert_eq!(summary.sessions.len(), 3);
        assert_eq!(
            summary
                .sessions
                .iter()
                .map(|session| (session.agent, session.id.as_str()))
                .collect::<std::collections::HashSet<_>>(),
            std::collections::HashSet::from([
                (Agent::Codex, "direct-session"),
                (Agent::Codex, "stage-runtime-session"),
                (Agent::AstraPi, "planner-session"),
            ])
        );
        assert_eq!(
            summary.session_keys.iter().cloned().collect::<HashSet<_>>(),
            HashSet::from([
                session_identity(Agent::Codex, "direct-session"),
                session_identity(Agent::Codex, "stage-runtime-session"),
                session_identity(Agent::Gemini, "missing-runtime-session"),
                session_identity(Agent::AstraPi, "planner-session"),
                session_identity(Agent::AstraPi, "missing-planner-session"),
            ])
        );
        assert!(!summary.session_keys.contains(&session_identity(
            Agent::Codex,
            "superseded-runtime-session"
        )));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir_all(&project_dir);
    }

    #[test]
    fn collect_referenced_session_keys_skips_loaded_and_superseded_sessions() {
        let loaded = HashMap::from([(
            (Agent::Claude, "loaded-session".to_string()),
            SessionInfo {
                id: "loaded-session".to_string(),
                agent: Agent::Claude,
                forked_from_agent: None,
                forked_from_id: None,
                project_path: None,
                project_name: None,
                started_at: None,
                updated_at: None,
                message_count: 0,
                rename_title: None,
                title: None,
                first_user_message: None,
                file_path: "/tmp/loaded.jsonl".to_string(),
                file_size: 1,
                partial: false,
                available: true,
                archived: false,
                subagents: Vec::new(),
            },
        )]);
        let plan_rounds = vec![PlanRoundInfo {
            id: "round-1".to_string(),
            thread_id: "thread-1".to_string(),
            astra_run_id: Some("run-1".to_string()),
            round_index: 0,
            summary: None,
            mode: PlanRoundMode::Parallel,
            source: PlanRoundSource::Astra,
            status: PlanRoundStatus::Running,
            tasks: vec![PlanTaskInfo {
                id: "task-1".to_string(),
                round_id: "round-1".to_string(),
                thread_stage_id: None,
                assistant_id: None,
                agent_participant_id: None,
                target_agent: Agent::Claude,
                stage_snapshot_json: None,
                assistant_snapshot_json: None,
                agent_snapshot_json: "{}".to_string(),
                title: "Task".to_string(),
                prompt: "Prompt".to_string(),
                expected_output: None,
                risk: PlanTaskRisk::Low,
                sort_order: 0,
                status: PlanTaskStatus::Running,
                result_summary: None,
                error: None,
                started_at: None,
                completed_at: None,
                created_at: 10,
                updated_at: 10,
                sessions: vec![
                    PlanTaskSessionInfo {
                        task_id: "task-1".to_string(),
                        agent: Agent::Claude,
                        session_id: "loaded-session".to_string(),
                        role: PlanTaskSessionRole::Runtime,
                        attempt_id: None,
                        attempt_count: 1,
                        superseded_at: None,
                        created_at: 11,
                        updated_at: 11,
                    },
                    PlanTaskSessionInfo {
                        task_id: "task-1".to_string(),
                        agent: Agent::Gemini,
                        session_id: "missing-session".to_string(),
                        role: PlanTaskSessionRole::Runtime,
                        attempt_id: None,
                        attempt_count: 1,
                        superseded_at: None,
                        created_at: 12,
                        updated_at: 12,
                    },
                    PlanTaskSessionInfo {
                        task_id: "task-1".to_string(),
                        agent: Agent::Codex,
                        session_id: "superseded-session".to_string(),
                        role: PlanTaskSessionRole::Runtime,
                        attempt_id: None,
                        attempt_count: 1,
                        superseded_at: Some(13),
                        created_at: 13,
                        updated_at: 13,
                    },
                ],
            }],
            created_at: 9,
            updated_at: 14,
        }];
        let astra_runs = vec![AstraRunRecord {
            run_id: "run-1".to_string(),
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            project_path: "/tmp/project".to_string(),
            status: "running".to_string(),
            mode: "auto".to_string(),
            planner_backend: Some("astra_pi_acp".to_string()),
            round_index: Some(0),
            round_limit: 3,
            terminal_reason: None,
            last_error_code: None,
            last_error_message: None,
            internal_planner_sessions: vec![AstraRunSessionRecord {
                run_id: "run-1".to_string(),
                agent: Agent::AstraPi,
                session_id: "planner-session".to_string(),
                role: PlanTaskSessionRole::Planner,
                sort_order: 0,
                created_at: 15,
                updated_at: 16,
            }],
            run_diagnostics_json: "[]".to_string(),
            error: None,
            created_at: 15,
            updated_at: 16,
        }];

        let refs = collect_referenced_session_keys(&plan_rounds, &astra_runs, &loaded)
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        assert!(refs.contains(&(Agent::Gemini, "missing-session".to_string())));
        assert!(refs.contains(&(Agent::AstraPi, "planner-session".to_string())));
        assert!(!refs.contains(&(Agent::Claude, "loaded-session".to_string())));
        assert!(!refs.contains(&(Agent::Codex, "superseded-session".to_string())));
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
