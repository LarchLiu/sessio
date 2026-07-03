use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::models::{
    Agent, PlanTaskSessionRole, SessionInfo, ThreadReplayInfo, ThreadReplaySessionInfo,
    ThreadReplaySessionSourceInfo, ThreadReplaySessionSourceKind,
};

use super::{insert_best_session, AstraRunRecord, PlanRoundInfo, SessionRef, SessionStore};

pub(crate) fn get_thread_replay<S: SessionStore + ?Sized>(
    store: &S,
    thread_id: &str,
) -> Result<ThreadReplayInfo> {
    let thread = store.get_thread_work_state(thread_id)?;
    let plan_rounds = store.list_plan_rounds(thread_id)?;
    let astra_runs = store.list_astra_runs(thread_id)?;
    let mut session_lookup = HashMap::<(Agent, String), SessionInfo>::new();
    let mut sessions = HashMap::<(Agent, String), ThreadReplaySessionInfo>::new();

    for session in &thread.sessions {
        insert_best_session(&mut session_lookup, session.clone());
        add_replay_session_source(
            &mut sessions,
            session.agent,
            &session.id,
            Some(session.clone()),
            ThreadReplaySessionSourceInfo {
                kind: ThreadReplaySessionSourceKind::Thread,
                thread_id: Some(thread.id.clone()),
                stage_id: None,
                plan_round_id: None,
                plan_task_id: None,
                astra_run_id: None,
                role: None,
                label: Some("thread".to_string()),
                stage_snapshot_json: None,
                assistant_snapshot_json: None,
                agent_snapshot_json: None,
                created_at: session.started_at.or(session.updated_at),
            },
        );
    }

    for stage in &thread.stages {
        for session in &stage.sessions {
            insert_best_session(&mut session_lookup, session.clone());
            add_replay_session_source(
                &mut sessions,
                session.agent,
                &session.id,
                Some(session.clone()),
                ThreadReplaySessionSourceInfo {
                    kind: ThreadReplaySessionSourceKind::Stage,
                    thread_id: Some(thread.id.clone()),
                    stage_id: Some(stage.id.clone()),
                    plan_round_id: None,
                    plan_task_id: None,
                    astra_run_id: None,
                    role: None,
                    label: stage.name.clone().or_else(|| Some(stage.stage_id.clone())),
                    stage_snapshot_json: None,
                    assistant_snapshot_json: None,
                    agent_snapshot_json: None,
                    created_at: session.started_at.or(session.updated_at),
                },
            );
        }
    }

    let referenced_keys =
        collect_referenced_session_keys(&plan_rounds, &astra_runs, &session_lookup);
    if !referenced_keys.is_empty() {
        let refs = referenced_keys
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

    for round in &plan_rounds {
        for task in &round.tasks {
            for task_session in &task.sessions {
                if task_session.superseded_at.is_some() {
                    continue;
                }
                let session = session_lookup
                    .get(&(task_session.agent, task_session.session_id.clone()))
                    .cloned();
                add_replay_session_source(
                    &mut sessions,
                    task_session.agent,
                    &task_session.session_id,
                    session,
                    ThreadReplaySessionSourceInfo {
                        kind: ThreadReplaySessionSourceKind::PlanTask,
                        thread_id: Some(thread.id.clone()),
                        stage_id: task.thread_stage_id.clone(),
                        plan_round_id: Some(round.id.clone()),
                        plan_task_id: Some(task.id.clone()),
                        astra_run_id: round.astra_run_id.clone(),
                        role: Some(task_session.role),
                        label: Some(task.title.clone()),
                        stage_snapshot_json: task.stage_snapshot_json.clone(),
                        assistant_snapshot_json: task.assistant_snapshot_json.clone(),
                        agent_snapshot_json: Some(task.agent_snapshot_json.clone()),
                        created_at: Some(task_session.created_at),
                    },
                );
            }
        }
    }

    for run in &astra_runs {
        for session_ref in &run.internal_planner_sessions {
            let session = session_lookup
                .get(&(session_ref.agent, session_ref.session_id.clone()))
                .cloned();
            add_replay_session_source(
                &mut sessions,
                session_ref.agent,
                &session_ref.session_id,
                session,
                ThreadReplaySessionSourceInfo {
                    kind: ThreadReplaySessionSourceKind::AstraInternal,
                    thread_id: Some(thread.id.clone()),
                    stage_id: None,
                    plan_round_id: None,
                    plan_task_id: None,
                    astra_run_id: Some(run.run_id.clone()),
                    role: Some(PlanTaskSessionRole::Planner),
                    label: run
                        .planner_backend
                        .as_ref()
                        .map(|backend| format!("Astra planner: {backend}"))
                        .or_else(|| Some("Astra planner".to_string())),
                    stage_snapshot_json: None,
                    assistant_snapshot_json: None,
                    agent_snapshot_json: None,
                    created_at: Some(run.updated_at),
                },
            );
        }
    }

    let mut sessions = sessions.into_values().collect::<Vec<_>>();
    sessions.sort_by(|a, b| {
        a.first_seen_at
            .unwrap_or(i64::MAX)
            .cmp(&b.first_seen_at.unwrap_or(i64::MAX))
            .then_with(|| a.agent.as_str().cmp(b.agent.as_str()))
            .then_with(|| a.session_id.cmp(&b.session_id))
    });

    Ok(ThreadReplayInfo {
        thread_id: thread.id,
        kind: thread.kind,
        sessions,
    })
}

fn collect_referenced_session_keys(
    plan_rounds: &[PlanRoundInfo],
    astra_runs: &[AstraRunRecord],
    existing_sessions: &HashMap<(Agent, String), SessionInfo>,
) -> Vec<(Agent, String)> {
    let mut refs = HashSet::<(Agent, String)>::new();
    for round in plan_rounds {
        for task in &round.tasks {
            for task_session in &task.sessions {
                if task_session.superseded_at.is_some() {
                    continue;
                }
                let key = (task_session.agent, task_session.session_id.clone());
                if !existing_sessions.contains_key(&key) {
                    refs.insert(key);
                }
            }
        }
    }
    for run in astra_runs {
        for session in &run.internal_planner_sessions {
            if is_virtual_orchestrator_session_id(&session.session_id) {
                continue;
            }
            let key = (session.agent, session.session_id.clone());
            if !existing_sessions.contains_key(&key) {
                refs.insert(key);
            }
        }
    }
    refs.into_iter().collect()
}

fn is_virtual_orchestrator_session_id(session_id: &str) -> bool {
    session_id.trim().starts_with("deterministic-orchestrator-")
}

fn add_replay_session_source(
    sessions: &mut HashMap<(Agent, String), ThreadReplaySessionInfo>,
    agent: Agent,
    session_id: &str,
    session: Option<SessionInfo>,
    source: ThreadReplaySessionSourceInfo,
) {
    let key = (agent, session_id.to_string());
    let source_time = source.created_at;
    let entry = sessions
        .entry(key)
        .or_insert_with(|| ThreadReplaySessionInfo {
            agent,
            session_id: session_id.to_string(),
            session: None,
            sources: Vec::new(),
            first_seen_at: source_time,
            last_seen_at: source_time,
        });

    if entry.session.is_none() {
        entry.session = session;
    }
    if let Some(source_time) = source_time {
        entry.first_seen_at = Some(
            entry
                .first_seen_at
                .map(|value| value.min(source_time))
                .unwrap_or(source_time),
        );
        entry.last_seen_at = Some(
            entry
                .last_seen_at
                .map(|value| value.max(source_time))
                .unwrap_or(source_time),
        );
    }
    if !entry.sources.iter().any(|existing| {
        existing.kind == source.kind
            && existing.stage_id == source.stage_id
            && existing.plan_round_id == source.plan_round_id
            && existing.plan_task_id == source.plan_task_id
            && existing.astra_run_id == source.astra_run_id
            && existing.role == source.role
    }) {
        entry.sources.push(source);
    }
}
